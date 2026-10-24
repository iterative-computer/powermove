//! The level 2 resampler on the GPU (`shaders/resample.wgsl`,
//! `shaders/mips.wgsl`): mip pyramids of sources, the content polygon and
//! inverse mapping of a draw, and its pipelines.

use std::collections::VecDeque;

use crate::pipelines::{PassBlend, QuadUniform, bytes_of};

use super::Sampling;

/// `Resample` in `resample.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResampleUniform {
    pub inv: [[f32; 4]; 3],
    pub uvwin: [f32; 4],
    pub params: [f32; 4],
    pub view: [f32; 4],
    pub tex: [f32; 4],
    pub origin: [f32; 4],
    pub edges: [[f32; 4]; MAX_EDGES],
}

/// A quad clipped at the camera plane has at most 5 edges.
pub(crate) const MAX_EDGES: usize = 6;

/// A source with its mip pyramid.
#[derive(Clone)]
pub(crate) struct Pyramid {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
}

/// Pyramids of stills, kept across frames (their textures are the media
/// cache's, which never change content).
pub(crate) struct PyramidCache {
    entries: VecDeque<(wgpu::Texture, Pyramid)>,
}

/// Stills whose pyramids are kept.
const CACHED_PYRAMIDS: usize = 16;

impl PyramidCache {
    pub fn new() -> PyramidCache {
        PyramidCache {
            entries: VecDeque::new(),
        }
    }

    fn get(&mut self, source: &wgpu::Texture) -> Option<Pyramid> {
        let i = self.entries.iter().position(|(t, _)| t == source)?;
        let entry = self.entries.remove(i)?;
        let p = entry.1.clone();
        self.entries.push_front(entry);
        Some(p)
    }

    fn put(&mut self, source: wgpu::Texture, pyramid: Pyramid) {
        self.entries.push_front((source, pyramid));
        self.entries.truncate(CACHED_PYRAMIDS);
    }

    /// Drops every kept pyramid.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Bytes held (level 0 and the levels below it).
    pub fn bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|(_, p)| {
                let px = u64::from(p.width) * u64::from(p.height);
                let texel = match p.texture.format() {
                    wgpu::TextureFormat::Rgba32Float => 16,
                    _ => 8,
                };
                px * texel * 4 / 3
            })
            .sum()
    }
}

impl Default for PyramidCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The level 2 pipelines.
pub(crate) struct Resampler {
    layout: wgpu::BindGroupLayout,
    mip_layout: wgpu::BindGroupLayout,
    trilinear: wgpu::Sampler,
    /// `[format][blend]`: rgba16float, rgba32float, rgba8unorm × over, additive.
    draw: [[crate::pipelines::LazyRenderPipeline; 2]; 3],
    copy: [crate::pipelines::LazyRenderPipeline; 3],
    down: [crate::pipelines::LazyRenderPipeline; 3],
}

const FORMATS: [wgpu::TextureFormat; 3] = [
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureFormat::Rgba32Float,
    wgpu::TextureFormat::Rgba8Unorm,
];

fn format_index(format: wgpu::TextureFormat) -> usize {
    match format {
        wgpu::TextureFormat::Rgba16Float => 0,
        wgpu::TextureFormat::Rgba32Float => 1,
        wgpu::TextureFormat::Rgba8Unorm => 2,
        other => panic!("unsupported resampler format: {other:?}"),
    }
}

impl Resampler {
    pub fn new(device: &wgpu::Device, quad_layout: &wgpu::BindGroupLayout) -> Resampler {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render resample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/resample.wgsl").into()),
        });
        let mips = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render mips"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mips.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pm resample"),
                entries: &[
                    texture(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                size_of::<ResampleUniform>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm mips"),
            entries: &[texture(0)],
        });
        let trilinear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm trilinear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm resample"),
            bind_group_layouts: &[Some(quad_layout), Some(&layout)],
            immediate_size: 0,
        });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm mips"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let component = |dst| wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: dst,
            operation: wgpu::BlendOperation::Add,
        };
        let blend = |b: PassBlend| {
            let dst = match b {
                PassBlend::Over => wgpu::BlendFactor::OneMinusSrcAlpha,
                PassBlend::Additive => wgpu::BlendFactor::One,
            };
            wgpu::BlendState {
                color: component(dst),
                alpha: component(dst),
            }
        };
        let pipeline = |layout: &wgpu::PipelineLayout,
                        module: &wgpu::ShaderModule,
                        vs: &'static str,
                        fs: &'static str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>| {
            let device = device.clone();
            let module = module.clone();
            let layout = layout.clone();
            crate::pipelines::LazyRenderPipeline::new(move || {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(fs),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(vs),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
        };
        let draw = FORMATS.map(|f| {
            [PassBlend::Over, PassBlend::Additive].map(|b| {
                pipeline(
                    &draw_layout,
                    &module,
                    "vs",
                    "fs_resample",
                    f,
                    Some(blend(b)),
                )
            })
        });
        let copy =
            FORMATS.map(|f| pipeline(&mip_pipeline_layout, &mips, "vs_full", "fs_copy", f, None));
        let down =
            FORMATS.map(|f| pipeline(&mip_pipeline_layout, &mips, "vs_full", "fs_down", f, None));
        Resampler {
            layout,
            mip_layout,
            trilinear,
            draw,
            copy,
            down,
        }
    }

    /// The mip pyramid of `source` (a working-format texture): stills come
    /// from `cache` after their first frame.
    pub fn pyramid(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        still: bool,
        cache: &mut PyramidCache,
    ) -> Pyramid {
        if still && let Some(p) = cache.get(source) {
            return p;
        }
        let (width, height) = (source.width().max(1), source.height().max(1));
        let levels = 32 - width.max(height).leading_zeros();
        let format = source.format();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm level 2 pyramid"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level_view = |level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("pm pyramid level"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let i = format_index(format);
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let mut above = source_view;
        for level in 0..levels {
            let target = level_view(level);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pm pyramid"),
                layout: &self.mip_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&above),
                }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm pyramid level"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(if level == 0 {
                self.copy[i].get()
            } else {
                self.down[i].get()
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..4, 0..1);
            drop(pass);
            above = target;
        }
        let pyramid = Pyramid {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
            width,
            height,
            levels,
        };
        if still {
            cache.put(source.clone(), pyramid.clone());
        }
        pyramid
    }

    pub fn pipeline(&self, format: wgpu::TextureFormat, blend: PassBlend) -> &wgpu::RenderPipeline {
        self.draw[format_index(format)][match blend {
            PassBlend::Over => 0,
            PassBlend::Additive => 1,
        }]
        .get()
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        self.draw
            .iter()
            .flat_map(|variants| variants.iter())
            .chain(self.copy.iter())
            .chain(self.down.iter())
            .filter(|pipeline| pipeline.is_built())
            .count()
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        let mut times = Vec::new();
        for (i, format) in FORMATS.into_iter().enumerate() {
            for (name, pipeline) in [
                ("draw over", &self.draw[i][0]),
                ("draw additive", &self.draw[i][1]),
                ("copy", &self.copy[i]),
                ("down", &self.down[i]),
            ] {
                let start = std::time::Instant::now();
                pipeline.get();
                times.push((format!("resample {name} {format:?}"), start.elapsed()));
            }
        }
        times
    }

    /// The bind group of one draw (its pyramid and uniform).
    pub fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pyramid: &Pyramid,
        uniform: &ResampleUniform,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm resample"),
            size: size_of::<ResampleUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes_of(uniform));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm resample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&pyramid.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.trilinear),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        })
    }
}

/// A level 2 draw's geometry: the content polygon in output pixels
/// (top-down, full frame), its pixel bounds, and the inverse mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Unit quad to output pixels (column-major 3×3).
    pub matrix: [f64; 9],
    /// Output pixels to unit quad.
    pub inverse: [f64; 9],
    /// The polygon's edges `(a, b, c)`: `a·x + b·y + c ≥ 0` inside.
    pub edges: Vec<[f64; 3]>,
    /// The polygon's vertices.
    pub polygon: Vec<[f64; 2]>,
    /// Pixel bounds `[x0, y0, x1, y1]` grown by one pixel.
    pub bounds: [f64; 4],
}

/// A 2×3 affine matrix `[a, b, c, d, e, f]` as a column-major 3×3.
pub fn affine3(m: &[f64; 6]) -> [f64; 9] {
    [m[0], m[1], 0.0, m[2], m[3], 0.0, m[4], m[5], 1.0]
}

fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let [a, d, g, b, e, h, c, f, i] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    Some([
        (e * i - f * h) / det,
        (f * g - d * i) / det,
        (d * h - e * g) / det,
        (c * h - b * i) / det,
        (a * i - c * g) / det,
        (b * g - a * h) / det,
        (b * f - c * e) / det,
        (c * d - a * f) / det,
        (a * e - b * d) / det,
    ])
}

/// The smallest homogeneous w kept in front of the camera plane.
const NEAR_W: f64 = 1e-6;

/// The placement of a level 2 draw: `matrix` maps the unit quad to output
/// pixels; `uv` is the content UV window (offset, scale), whose part
/// outside 0..1 is transparent. `None` when nothing is in front of the
/// camera or the mapping is singular.
pub fn placement(matrix: [f64; 9], uv: [f64; 4]) -> Option<Placement> {
    let inverse = invert3(&matrix)?;
    // The part of the unit quad the UV window fills.
    let span = |offset: f64, scale: f64| -> Option<(f64, f64)> {
        if scale == 0.0 || !scale.is_finite() {
            return None;
        }
        let (a, b) = ((0.0 - offset) / scale, (1.0 - offset) / scale);
        let (lo, hi) = (a.min(b).max(0.0), a.max(b).min(1.0));
        (hi > lo).then_some((lo, hi))
    };
    let (u0, u1) = span(uv[0], uv[2])?;
    let (v0, v1) = span(uv[1], uv[3])?;
    let m = &matrix;
    let hom = |u: f64, v: f64| {
        [
            m[0] * u + m[3] * v + m[6],
            m[1] * u + m[4] * v + m[7],
            m[2] * u + m[5] * v + m[8],
        ]
    };
    // Clip the quad at w ≥ NEAR_W in homogeneous space.
    let quad = [hom(u0, v0), hom(u1, v0), hom(u1, v1), hom(u0, v1)];
    let mut clipped: Vec<[f64; 3]> = Vec::with_capacity(5);
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let (ia, ib) = (a[2] >= NEAR_W, b[2] >= NEAR_W);
        if ia {
            clipped.push(a);
        }
        if ia != ib {
            let t = (NEAR_W - a[2]) / (b[2] - a[2]);
            clipped.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR_W]);
        }
    }
    if clipped.len() < 3 {
        return None;
    }
    let polygon: Vec<[f64; 2]> = clipped.iter().map(|p| [p[0] / p[2], p[1] / p[2]]).collect();
    if polygon
        .iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite())
    {
        return None;
    }
    // Orientation: the signed area's sign makes every edge's inside ≥ 0.
    let area: f64 = (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area.abs() < 1e-12 {
        return None;
    }
    let sign = area.signum();
    let edges = (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            // Inside is to the left of a → b for counterclockwise polygons
            // (positive area in y-down coordinates means clockwise on screen;
            // the sign covers both).
            let (nx, ny) = (-(b[1] - a[1]) * sign, (b[0] - a[0]) * sign);
            [nx, ny, -(nx * a[0] + ny * a[1])]
        })
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &polygon {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    Some(Placement {
        matrix,
        inverse,
        edges,
        polygon,
        bounds: [
            x0.floor() - 1.0,
            y0.floor() - 1.0,
            x1.ceil() + 1.0,
            y1.ceil() + 1.0,
        ],
    })
}

impl Placement {
    /// The draw's bounding quad (unit quad to output pixels), clamped to
    /// the frame grown by a pixel; `None` when it misses the frame.
    pub fn bounds_matrix(&self, width: u32, height: u32) -> Option<[f64; 6]> {
        let x0 = self.bounds[0].max(-1.0);
        let y0 = self.bounds[1].max(-1.0);
        let x1 = self.bounds[2].min(f64::from(width) + 1.0);
        let y1 = self.bounds[3].min(f64::from(height) + 1.0);
        (x1 > x0 && y1 > y0).then_some([x1 - x0, 0.0, 0.0, y1 - y0, x0, y0])
    }

    /// The fragment uniform for a draw of this placement.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn uniform(
        &self,
        uv: [f64; 4],
        alpha: f64,
        from_fbo: bool,
        sampling: Sampling,
        region_origin: (u32, u32),
        target_height: u32,
        pyramid: &Pyramid,
    ) -> ResampleUniform {
        let i = &self.inverse;
        let mut u = ResampleUniform {
            inv: [
                [i[0] as f32, i[1] as f32, i[2] as f32, 0.0],
                [i[3] as f32, i[4] as f32, i[5] as f32, 0.0],
                [i[6] as f32, i[7] as f32, i[8] as f32, 0.0],
            ],
            uvwin: uv.map(|v| v as f32),
            params: [
                alpha as f32,
                if from_fbo { 1.0 } else { 0.0 },
                match sampling {
                    Sampling::Playback => 0.0,
                    Sampling::Final => 1.0,
                },
                self.edges.len().min(MAX_EDGES) as f32,
            ],
            view: [
                region_origin.0 as f32,
                region_origin.1 as f32,
                target_height as f32,
                0.0,
            ],
            tex: [
                pyramid.width as f32,
                pyramid.height as f32,
                pyramid.levels as f32,
                0.0,
            ],
            origin: [self.bounds[0] as f32, self.bounds[1] as f32, 0.0, 0.0],
            edges: [[0.0; 4]; MAX_EDGES],
        };
        // Unit normals, relative to the bounds' (integer) origin.
        let (ox, oy) = (self.bounds[0], self.bounds[1]);
        for (k, e) in self.edges.iter().take(MAX_EDGES).enumerate() {
            let len = e[0].hypot(e[1]);
            let (a, b) = (e[0] / len, e[1] / len);
            let c = (e[2] + e[0] * ox + e[1] * oy) / len;
            u.edges[k] = [a as f32, b as f32, c as f32, 0.0];
        }
        u
    }
}

/// The vertex uniform of a level 2 draw: the bounds quad.
pub(crate) fn bounds_quad(bounds: [f64; 6], width: u32, height: u32) -> QuadUniform {
    QuadUniform {
        m: [
            [bounds[0] as f32, bounds[1] as f32, 0.0, 0.0],
            [bounds[2] as f32, bounds[3] as f32, 0.0, 0.0],
            [bounds[4] as f32, bounds[5] as f32, 1.0, 0.0],
        ],
        res: [width as f32, height as f32, 0.0, 0.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [0.0; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_axis_aligned_quad_has_four_edges_inward() {
        let p = placement(
            affine3(&[100.0, 0.0, 0.0, 50.0, 10.0, 20.0]),
            [0.0, 0.0, 1.0, 1.0],
        )
        .unwrap();
        assert_eq!(p.polygon.len(), 4);
        let inside = |x: f64, y: f64| p.edges.iter().all(|e| e[0] * x + e[1] * y + e[2] >= 0.0);
        assert!(inside(60.0, 45.0));
        assert!(!inside(5.0, 45.0) && !inside(60.0, 75.0) && !inside(111.0, 21.0));
        assert_eq!(p.bounds, [9.0, 19.0, 111.0, 71.0]);
        // A mirrored quad keeps its inside.
        let m = placement(
            affine3(&[-100.0, 0.0, 0.0, 50.0, 110.0, 20.0]),
            [0.0, 0.0, 1.0, 1.0],
        )
        .unwrap();
        let inside = |x: f64, y: f64| m.edges.iter().all(|e| e[0] * x + e[1] * y + e[2] >= 0.0);
        assert!(inside(60.0, 45.0));
    }

    #[test]
    fn the_uv_window_limits_the_polygon() {
        // `contain` letterboxing: v from 0.25 to 0.75 of the quad.
        let p = placement(
            affine3(&[100.0, 0.0, 0.0, 100.0, 0.0, 0.0]),
            [0.0, -0.5, 1.0, 2.0],
        )
        .unwrap();
        let ys: Vec<f64> = p.polygon.iter().map(|q| q[1]).collect();
        assert_eq!(ys.iter().cloned().fold(f64::MAX, f64::min), 25.0);
        assert_eq!(ys.iter().cloned().fold(f64::MIN, f64::max), 75.0);
    }

    #[test]
    fn planes_are_clipped_at_the_camera() {
        // w = 1 − 2u: the u > 0.5 half is behind the camera.
        let m = [100.0, 0.0, -2.0, 0.0, 100.0, 0.0, 0.0, 0.0, 1.0];
        let p = placement(m, [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert_eq!(p.polygon.len(), 4);
        assert!(p.polygon.iter().all(|q| q[0].is_finite()));
        // Entirely behind: nothing.
        let m = [100.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, -1.0];
        assert!(placement(m, [0.0, 0.0, 1.0, 1.0]).is_none());
    }
}
