//! Bezier path masks: today's Canvas2D coverage route (`applyPathMasks` in
//! `legacy/gl/compositor.ts`; docs/engine/composite.md "Path masks").
//!
//! Today, when any enabled mask of a layer has a path, every enabled mask
//! (parametric ones too, with no count limit) is filled white on its own
//! canvas of the output size and drawn, in order, onto one coverage canvas
//! through `ctx.filter = blur(feather · |world x| / 2 px)` with
//! `source-over` (add) or `destination-out` (subtract); the canvas starts
//! white when no mask adds. The layer is multiplied by the canvas's alpha,
//! sampled at each output pixel (through the inverse plane for a 3D layer).
//!
//! Each item's fill is pm-vector's Canvas2D emulation on the CPU
//! (`pm_vector::canvas2d::mask::item`); its blur is Skia's GPU blur image
//! filter, planned by `pm_vector::canvas2d::filter` and run here on the GPU
//! with the same bilinear hardware; the composite is hardware blending into
//! an 8-bit canvas, as Graphite's. The SkSL effects declare `half` sums,
//! weights, and offsets, but Graphite evaluates them at full precision on
//! this GPU: `f32` sums match today's canvases bit for bit over the whole
//! `tools/mask-ref` sweep, and `f16` sums leave thousands of pixels 1 code
//! value off (measured 2026-10-18, Apple M4 Max).

use std::collections::HashMap;

use pm_vector::canvas2d::filter::{BlurPlan, Map, Pass, Rect};
use pm_vector::canvas2d::mask::{self, Composite};
use pm_vector::{PathMask, PathMasks};

use crate::pipelines::bytes_of;

/// A layer's masks on the path route, ready for the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct PathCoverage {
    /// The coverage canvas: the output size (`W × H`).
    pub width: u32,
    pub height: u32,
    /// The layer's `scaledWorld`.
    pub world: [f64; 6],
    pub masks: PathMasks,
}

/// `Pass` in `path_masks.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct PassUniform {
    dst_org: [f32; 2],
    src_org: [f32; 2],
    src_inv: [f32; 2],
    n: u32,
    linear: u32,
    map_s: [f32; 2],
    map_t: [f32; 2],
    dir: [f32; 2],
    mode: [u32; 2],
    k: [[f32; 4]; 7],
    o: [[f32; 4]; 14],
}

/// The coverage canvas format: one 8-bit channel (every premultiplied
/// channel of today's white canvas is its alpha).
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

pub(crate) struct PathMaskPipelines {
    format: wgpu::TextureFormat,
    /// Item rasters across frames.
    items: std::sync::Mutex<ItemCache>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    rescale: crate::pipelines::LazyRenderPipeline,
    blur1d: crate::pipelines::LazyRenderPipeline,
    blur2d: crate::pipelines::LazyRenderPipeline,
    over: crate::pipelines::LazyRenderPipeline,
    out: crate::pipelines::LazyRenderPipeline,
}

impl PathMaskPipelines {
    pub fn new(device: &wgpu::Device) -> PathMaskPipelines {
        Self::with_format(device, FORMAT)
    }

    fn with_format(device: &wgpu::Device, format: wgpu::TextureFormat) -> PathMaskPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render path masks"),
            source: wgpu::ShaderSource::Wgsl(include_str!("path_masks.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm path masks"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<PassUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm path masks"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &'static str, blend: Option<wgpu::BlendState>| {
            let device = device.clone();
            let module = module.clone();
            let pipeline_layout = pipeline_layout.clone();
            crate::pipelines::LazyRenderPipeline::new(move || {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
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
        let blend = |src: wgpu::BlendFactor| wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: src,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: src,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm path masks bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        PathMaskPipelines {
            format,
            rescale: pipeline("fs_rescale", None),
            blur1d: pipeline("fs_blur1d", None),
            blur2d: pipeline("fs_blur2d", None),
            over: pipeline("fs_draw", Some(blend(wgpu::BlendFactor::One))),
            out: pipeline("fs_draw", Some(blend(wgpu::BlendFactor::Zero))),
            items: Default::default(),
            layout,
            sampler,
        }
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        [
            &self.rescale,
            &self.blur1d,
            &self.blur2d,
            &self.over,
            &self.out,
        ]
        .into_iter()
        .filter(|pipeline| pipeline.is_built())
        .count()
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        [
            ("rescale", &self.rescale),
            ("blur1d", &self.blur1d),
            ("blur2d", &self.blur2d),
            ("over", &self.over),
            ("out", &self.out),
        ]
        .into_iter()
        .map(|(name, pipeline)| {
            let start = std::time::Instant::now();
            pipeline.get();
            (
                format!("path mask {name} {:?}", self.format),
                start.elapsed(),
            )
        })
        .collect()
    }
}

/// An image on the GPU: `rect` in its space, with a transparent 1-pixel
/// border (`ring`), or the bare canvas.
struct Image {
    texture: wgpu::Texture,
    rect: Rect,
    ring: i32,
}

impl Image {
    fn new(device: &wgpu::Device, rect: Rect, ring: i32, format: wgpu::TextureFormat) -> Image {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm path mask image"),
            size: wgpu::Extent3d {
                width: (rect.w + 2 * ring as u32).max(1),
                height: (rect.h + 2 * ring as u32).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Image {
            texture,
            rect,
            ring,
        }
    }
    /// The space coordinate of texel (0, 0)'s corner.
    fn origin(&self) -> [f32; 2] {
        [
            (self.rect.x0 - self.ring) as f32,
            (self.rect.y0 - self.ring) as f32,
        ]
    }
    fn inv_size(&self) -> [f32; 2] {
        let s = self.texture.size();
        [1.0 / s.width as f32, 1.0 / s.height as f32]
    }
}

/// Item rasters keyed by their mask, world, and canvas, kept across frames
/// (a static mask is filled once), least recently used dropped past
/// [`ITEM_CACHE_BYTES`].
#[derive(Default)]
pub(crate) struct ItemCache {
    items: HashMap<String, (std::sync::Arc<mask::Item>, u64)>,
    tick: u64,
    bytes: usize,
}

/// The item cache's budget.
const ITEM_CACHE_BYTES: usize = 192 << 20;

impl ItemCache {
    fn item(
        &mut self,
        m: &PathMask,
        world: [f64; 6],
        w: u32,
        h: u32,
    ) -> Result<std::sync::Arc<mask::Item>, String> {
        let key = format!("{w}x{h} {world:?} {:?} {:?} {:?}", m.ops, m.outline, m.path);
        self.tick += 1;
        if let Some((i, used)) = self.items.get_mut(&key) {
            *used = self.tick;
            return Ok(i.clone());
        }
        let item =
            std::sync::Arc::new(mask::item(m, world, w, h).map_err(|u| format!("path mask: {u}"))?);
        self.bytes += item.alpha.len();
        self.items.insert(key, (item.clone(), self.tick));
        while self.bytes > ITEM_CACHE_BYTES && self.items.len() > 1 {
            let oldest = self
                .items
                .iter()
                .min_by_key(|(_, (_, u))| *u)
                .map(|(k, _)| k.clone());
            let Some(k) = oldest else { break };
            if let Some((i, _)) = self.items.remove(&k) {
                self.bytes -= i.alpha.len();
            }
        }
        Ok(item)
    }
}

impl PathMaskPipelines {
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        u: &PassUniform,
        src: &Image,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        viewport: [f32; 4],
    ) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm path mask pass"),
            size: size_of::<PassUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut u = *u;
        u.mode[0] = u32::from(self.format != FORMAT);
        queue.write_buffer(&buffer, 0, bytes_of(&u));
        let view = src.texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm path mask pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm path mask pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [x, y, w, h] = viewport;
        if w > 0.0 && h > 0.0 {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// The coverage canvas of `cov`: an 8-bit single-channel texture of
    /// the output size, top-down rows. `Err` names a mask the Canvas2D
    /// emulation cannot fill (the layer fails).
    pub fn coverage(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cov: &PathCoverage,
    ) -> Result<wgpu::Texture, String> {
        let (w, h) = (cov.width.max(1), cov.height.max(1));
        let canvas = Image::new(device, Rect::ltrb(0, 0, w as i32, h as i32), 0, self.format);
        let canvas_view = canvas.texture.create_view(&Default::default());
        let start = if cov.masks.start_covered { 1.0 } else { 0.0 };
        // The canvas's starting state (`fillRect` in white when no mask adds).
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm path mask canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &canvas_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: start,
                        g: start,
                        b: start,
                        a: start,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for m in &cov.masks.masks {
            let item = self
                .items
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .item(m, cov.world, w, h)?;
            let mut img = Image::new(device, Rect::ltrb(0, 0, w as i32, h as i32), 1, self.format);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &img.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 1, y: 1, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &item.alpha,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            let plan = BlurPlan::new(w, h, mask::sigma(m, cov.world));
            let (draw, linear) = match &plan {
                Some(plan) => {
                    for pass in &plan.passes {
                        img = self.run_pass(device, queue, encoder, pass, &img);
                    }
                    (plan.draw, plan.linear)
                }
                None => (
                    Map {
                        scale: [1.0, 1.0],
                        translate: [0.0, 0.0],
                    },
                    false,
                ),
            };
            let u = PassUniform {
                src_org: img.origin(),
                src_inv: img.inv_size(),
                linear: u32::from(linear),
                map_s: draw.scale,
                map_t: draw.translate,
                ..Default::default()
            };
            let pipeline = match Composite::from(m.mode) {
                Composite::SourceOver => self.over.get(),
                Composite::DestinationOut => self.out.get(),
            };
            self.pass(
                device,
                queue,
                encoder,
                pipeline,
                &u,
                &img,
                &canvas_view,
                wgpu::LoadOp::Load,
                [0.0, 0.0, w as f32, h as f32],
            );
        }
        Ok(canvas.texture)
    }

    fn run_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        src: &Image,
    ) -> Image {
        let dst = pass.dst();
        let out = Image::new(device, dst, 1, self.format);
        let mut u = PassUniform {
            dst_org: out.origin(),
            src_org: src.origin(),
            src_inv: src.inv_size(),
            ..Default::default()
        };
        let pipeline = match pass {
            Pass::Rescale { sample, .. } => {
                u.map_s = sample.scale;
                u.map_t = sample.translate;
                self.rescale.get()
            }
            Pass::Blur1d { dir, taps, .. } => {
                u.dir = *dir;
                u.n = (taps.len() / 2) as u32;
                for (i, pair) in taps.chunks(2).enumerate() {
                    u.o[i] = [pair[0].0, pair[0].1, pair[1].0, pair[1].1];
                }
                self.blur1d.get()
            }
            Pass::Blur2d {
                weights, offsets, ..
            } => {
                u.n = (weights.len() / 4) as u32;
                for (i, k) in weights.chunks(4).enumerate() {
                    u.k[i] = [k[0], k[1], k[2], k[3]];
                }
                for (i, o) in offsets.chunks(2).enumerate() {
                    u.o[i] = [o[0][0], o[0][1], o[1][0], o[1][1]];
                }
                self.blur2d.get()
            }
        };
        let view = out.texture.create_view(&Default::default());
        self.pass(
            device,
            queue,
            encoder,
            pipeline,
            &u,
            src,
            &view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            [1.0, 1.0, dst.w as f32, dst.h as f32],
        );
        out
    }
}

/// The coverage canvas of `cov` read back (top-down, one byte per pixel):
/// for parity tests and diagnostics.
pub fn coverage_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cov: &PathCoverage,
) -> Result<Vec<u8>, String> {
    let pipelines = PathMaskPipelines::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let texture = pipelines.coverage(device, queue, &mut encoder, cov)?;
    let (w, h) = (texture.width(), texture.height());
    let row = w.div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm path mask readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range().map_err(|e| format!("{e:?}"))?;
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize]);
    }
    Ok(out)
}

#[cfg(all(test, target_os = "macos"))]
mod filter_tests {
    use super::*;

    /// Independently measures Metal's UNORM8 bilinear sampler across all
    /// 256 weight buckets, two axes and four different channel gradients.
    #[test]
    fn unorm_sampler_matches_the_cpu_model() {
        let gpu = pm_gpu::Gpu::new("Canvas2D filter sampler").unwrap();
        let (d, q) = (&gpu.device, &gpu.queue);
        let pipes = PathMaskPipelines::with_format(d, wgpu::TextureFormat::Rgba32Float);
        let src = Image::new(
            d,
            Rect::ltrb(0, 0, 2, 2),
            1,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let rgba = [
            0, 11, 234, 255, 255, 89, 124, 255, 67, 243, 3, 255, 184, 10, 128, 255,
        ];
        q.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 1, y: 1, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
        let mut e = d.create_command_encoder(&Default::default());
        let sample = Map {
            scale: [1.0 / 4096.0, 0.0],
            translate: [0.500123, 1.230123],
        };
        let out = pipes.run_pass(
            d,
            q,
            &mut e,
            &Pass::Rescale {
                dst: Rect::ltrb(0, 0, 4096, 1),
                sample,
            },
            &src,
        );
        q.submit([e.finish()]);
        let data = pm_gpu::readback::read_f32(d, q, &out.texture).unwrap();
        for c in 0..4 {
            let reference = pm_vector::canvas2d::filter::Image {
                rect: Rect::ltrb(0, 0, 2, 2),
                px: rgba.chunks_exact(4).map(|p| p[c]).collect(),
            };
            for x in 0..4094usize {
                let sx = (x as f32 + 0.5).mul_add(sample.scale[0], sample.translate[0]);
                let expected = reference.bilinear(sx, sample.translate[1]);
                let actual = data[((4098 + x + 1) * 4) + c];
                assert!(
                    (actual - expected).abs() < 1e-7,
                    "x={x} channel={c}: {actual} vs {expected}"
                );
            }
        }
    }
}
