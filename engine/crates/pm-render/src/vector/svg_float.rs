//! SVG paint in working float precision. Every path gets the same analytic
//! AreaCoverage as native shapes; paint and isolated groups stay RGBA32F
//! until the final working-format store.

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, BezPath, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::{FloatDraw, FloatGradient, FloatNode, FloatPaint, SvgDocument};
use wgpu::util::DeviceExt;

use crate::coverage::{self, AreaCoverage, Batch, Item};

const DRAW: &str = r#"
struct Params { v: array<vec4<f32>, 8> };
struct Stop { color: vec4<f32>, position: vec4<f32> };
@group(0) @binding(0) var<storage, read> coverage: array<f32>;
@group(0) @binding(1) var<storage, read> stops: array<Stop>;
@group(0) @binding(2) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 2.0 - 1.0;
    let y = 1.0 - f32(i >> 1u) * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}
fn extend(v: f32, method: u32) -> f32 {
    if (method == 1u) { return fract(v); }
    if (method == 2u) { return 1.0 - abs(fract(v * 0.5) * 2.0 - 1.0); }
    return clamp(v, 0.0, 1.0);
}
fn gradient_t(q: vec2<f32>) -> f32 {
    let a = p.v[3].xy;
    let b = p.v[3].zw;
    if (p.v[4].w < 1.5) {
        let d = b - a;
        return dot(q - a, d) / max(dot(d, d), 1e-30);
    }
    // Two-circle SVG radial gradient: |q - (F+t(C-F))| = fr+t(r-fr).
    let d = b - a;
    let e = q - a;
    let dr = p.v[4].x - p.v[4].y;
    let aa = dot(d, d) - dr * dr;
    let bb = -2.0 * (dot(e, d) + p.v[4].y * dr);
    let cc = dot(e, e) - p.v[4].y * p.v[4].y;
    if (abs(aa) < 1e-9) { return -cc / select(bb, 1e-9, abs(bb) < 1e-9); }
    let disc = max(bb * bb - 4.0 * aa * cc, 0.0);
    let root = sqrt(disc);
    let lo = (-bb - root) / (2.0 * aa);
    let hi = (-bb + root) / (2.0 * aa);
    // The outer intersection is the visible branch; a positive radius is
    // required when a focal radius is present.
    let v = max(lo, hi);
    return select(v, min(lo, hi), p.v[4].y + dr * v < 0.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let xy = vec2<i32>(floor(pos.xy));
    let box = vec4<i32>(p.v[5]);
    if (xy.x < box.x || xy.y < box.y || xy.x >= box.x + box.z || xy.y >= box.y + box.w) {
        discard;
    }
    let idx = bitcast<u32>(p.v[6].x) + u32((xy.y - box.y) * box.z + xy.x - box.x);
    let cov = clamp(coverage[idx], 0.0, 1.0);
    var rgba = p.v[0];
    if (p.v[4].w > 0.5) {
        let q0 = pos.xy + p.v[7].xy;
        let q = vec2<f32>(dot(p.v[1].xyz, vec3<f32>(q0, 1.0)), dot(p.v[2].xyz, vec3<f32>(q0, 1.0)));
        let t = extend(gradient_t(q), u32(p.v[6].w));
        let first = bitcast<u32>(p.v[6].y);
        let n = bitcast<u32>(p.v[6].z);
        if (n == 0u) { return vec4<f32>(0.0); }
        rgba = stops[first].color;
        for (var i = 1u; i < n; i = i + 1u) {
            let next = stops[first + i];
            let prev = stops[first + i - 1u];
            if (i == n - 1u && t >= next.position.x) { rgba = next.color; break; }
            if (t < next.position.x || i == n - 1u) {
                let u = clamp((t - prev.position.x) / max(next.position.x - prev.position.x, 1e-12), 0.0, 1.0);
                rgba = mix(prev.color, next.color, u);
                break;
            }
        }
        rgba.a *= p.v[4].z;
    }
    let alpha = rgba.a * cov;
    return vec4<f32>(rgba.rgb * alpha, alpha);
}
"#;

const COMPOSITE: &str = r#"
struct Params { v: vec4<f32> };
@group(0) @binding(0) var back: texture_2d<f32>;
@group(0) @binding(1) var front: texture_2d<f32>;
@group(0) @binding(2) var clip_tex: texture_2d<f32>;
@group(0) @binding(3) var mask_tex: texture_2d<f32>;
@group(0) @binding(4) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 2.0 - 1.0, 1.0 - f32(i >> 1u) * 2.0, 0.0, 1.0);
}
fn lum(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.3, 0.59, 0.11)); }
fn sat(c: vec3<f32>) -> f32 { return max(max(c.r,c.g),c.b) - min(min(c.r,c.g),c.b); }
fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(min(c.r,c.g),c.b);
    let x = max(max(c.r,c.g),c.b);
    var v = c;
    if (n < 0.0) { v = vec3<f32>(l) + (v - vec3<f32>(l)) * l / max(l - n, 1e-9); }
    if (x > 1.0) { v = vec3<f32>(l) + (v - vec3<f32>(l)) * (1.0 - l) / max(x - l, 1e-9); }
    return v;
}
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> { return clip_color(c + vec3<f32>(l - lum(c))); }
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    // W3C SetSat, sorted by component value.
    var o = vec3<f32>(0.0);
    var ids = vec3<u32>(0u,1u,2u);
    if (c[ids.x] > c[ids.y]) { let z=ids.x; ids.x=ids.y; ids.y=z; }
    if (c[ids.y] > c[ids.z]) { let z=ids.y; ids.y=ids.z; ids.z=z; }
    if (c[ids.x] > c[ids.y]) { let z=ids.x; ids.x=ids.y; ids.y=z; }
    if (c[ids.z] > c[ids.x]) { o[ids.y] = (c[ids.y]-c[ids.x])*s/(c[ids.z]-c[ids.x]); o[ids.z]=s; }
    return o;
}
fn blend(b: vec3<f32>, s: vec3<f32>, mode: u32) -> vec3<f32> {
    if (mode == 1u) { return b*s; }
    if (mode == 2u) { return b+s-b*s; }
    if (mode == 3u) { return select(2.0*b*s,1.0-2.0*(1.0-b)*(1.0-s),b>vec3<f32>(0.5)); }
    if (mode == 4u) { return min(b,s); }
    if (mode == 5u) { return max(b,s); }
    if (mode == 6u) { let v = select(min(b/max(1.0-s,vec3<f32>(1e-12)),vec3<f32>(1.0)),vec3<f32>(1.0),s>=vec3<f32>(1.0)); return select(v,vec3<f32>(0.0),b<=vec3<f32>(0.0)); }
    if (mode == 7u) { let v = 1.0-select(min((1.0-b)/max(s,vec3<f32>(1e-12)),vec3<f32>(1.0)),vec3<f32>(1.0),s<=vec3<f32>(0.0)); return select(v,vec3<f32>(1.0),b>=vec3<f32>(1.0)); }
    if (mode == 8u) { return select(2.0*b*s,1.0-2.0*(1.0-b)*(1.0-s),s>vec3<f32>(0.5)); }
    if (mode == 9u) {
        let d = select(((16.0*b-12.0)*b+4.0)*b,sqrt(b),b>vec3<f32>(0.25));
        return select(b-(1.0-2.0*s)*b*(1.0-b),b+(2.0*s-1.0)*(d-b),s>vec3<f32>(0.5));
    }
    if (mode == 10u) { return abs(b-s); }
    if (mode == 11u) { return b+s-2.0*b*s; }
    if (mode == 12u) { return set_lum(set_sat(s,sat(b)),lum(b)); }
    if (mode == 13u) { return set_lum(set_sat(b,sat(s)),lum(b)); }
    if (mode == 14u) { return set_lum(s,lum(b)); }
    if (mode == 15u) { return set_lum(b,lum(s)); }
    return s;
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let xy = vec2<i32>(floor(pos.xy));
    let b = textureLoad(back,xy,0);
    var s = textureLoad(front,xy,0);
    if (p.v.z > 0.5) { s *= textureLoad(clip_tex,xy,0).a; }
    if (p.v.w > 0.5) {
        let m = textureLoad(mask_tex,xy,0);
        // The sign of w selects alpha versus luminance mask.
        let factor = select(m.a, dot(m.rgb,vec3<f32>(0.2126,0.7152,0.0722)),p.v.w > 1.5);
        s *= factor;
    }
    s *= p.v.x;
    let cb = select(vec3<f32>(0.0), b.rgb/max(b.a,1e-12), b.a>0.0);
    let cs = select(vec3<f32>(0.0), s.rgb/max(s.a,1e-12), s.a>0.0);
    let mixed = blend(cb,cs,u32(p.v.y));
    let rgb = (1.0-s.a)*b.rgb + (1.0-b.a)*s.rgb + b.a*s.a*mixed;
    return vec4<f32>(rgb, s.a+b.a*(1.0-s.a));
}
"#;

const COPY: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u)*2.0-1.0, 1.0-f32(i >> 1u)*2.0,0.0,1.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(source,vec2<i32>(floor(pos.xy)),0);
}
"#;

struct DrawEntry {
    path: BezPath,
    bounds: PixelRect,
    offset: u64,
}

pub struct SvgPainter {
    coverage: AreaCoverage,
    draw: wgpu::RenderPipeline,
    draw_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    copy: [wgpu::RenderPipeline; 3],
    copy_layout: wgpu::BindGroupLayout,
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn buffer_entry(binding: u32, storage: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: if storage {
                wgpu::BufferBindingType::Storage { read_only: true }
            } else {
                wgpu::BufferBindingType::Uniform
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pm svg float"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pm svg float"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
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
            module,
            entry_point: Some("fs"),
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
}
fn float_target(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm svg float layer"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn bytes(x: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding; the byte view stays within this initialized slice.
    unsafe { std::slice::from_raw_parts(x.as_ptr().cast::<u8>(), std::mem::size_of_val(x)) }
}
fn uniform(device: &wgpu::Device, data: &[f32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pm svg float uniform"),
        contents: bytes(data),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    entries: &[wgpu::BindGroupEntry<'_>],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm svg float bind"),
        layout,
        entries,
    })
}
fn render_pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    clear: bool,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm svg float paint"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..4, 0..1);
}

impl SvgPainter {
    pub fn new(device: &wgpu::Device) -> Self {
        let dm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float draw"),
            source: wgpu::ShaderSource::Wgsl(DRAW.into()),
        });
        let cm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE.into()),
        });
        let xm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float copy"),
            source: wgpu::ShaderSource::Wgsl(COPY.into()),
        });
        let dl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float draw"),
            entries: &[
                buffer_entry(0, true),
                buffer_entry(1, true),
                buffer_entry(2, false),
            ],
        });
        let cl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float composite"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                buffer_entry(4, false),
            ],
        });
        let xl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float copy"),
            entries: &[tex_entry(0)],
        });
        Self {
            coverage: AreaCoverage::new(device),
            draw: pipeline(
                device,
                &dm,
                &dl,
                wgpu::TextureFormat::Rgba32Float,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            draw_layout: dl,
            composite: pipeline(device, &cm, &cl, wgpu::TextureFormat::Rgba32Float, None),
            composite_layout: cl,
            copy: [
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Rgba32Float,
            ]
            .map(|f| pipeline(device, &xm, &xl, f, None)),
            copy_layout: xl,
        }
    }

    /// Records a float SVG render. Unsupported documents return `None` for
    /// the caller's resvg fallback; no commands are recorded in that case.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &SvgDocument,
        transform: Affine,
        rect: PixelRect,
        clip: &[Point],
        precision: Precision,
    ) -> Result<Option<(wgpu::Texture, u64)>, String> {
        let Some(scene) = doc.float_scene(transform, clip) else {
            return Ok(None);
        };
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let mut paths = Vec::new();
        collect(&scene, rect, &mut paths);
        let covered_pixels = paths.iter().fold(0u64, |sum, e| {
            sum.saturating_add(u64::from(e.bounds.width) * u64::from(e.bounds.height))
        });
        if covered_pixels > device.limits().max_storage_buffer_binding_size / 4
            || covered_pixels > u64::from(u32::MAX)
        {
            return Ok(None);
        }
        let items: Vec<_> = paths
            .iter()
            .map(|e| Item {
                path: &e.path,
                bounds: e.bounds,
                at: [0, 0],
            })
            .collect();
        let batch = Batch::new(&items);
        let values = coverage::buffer(device, batch.bytes());
        self.coverage.encode(device, encoder, &batch, &values, None);
        for (i, e) in paths.iter_mut().enumerate() {
            e.offset = batch.offset(i);
        }
        let (stops, ranges) = collect_stops(&scene);
        let stop_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pm svg float stops"),
            contents: bytes(stops.as_flattened()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut cursor = 0;
        let mut stop_cursor = 0;
        let target = self.paint_nodes(
            device,
            encoder,
            &scene,
            rect,
            &paths,
            &values,
            &stop_buffer,
            &ranges,
            &mut cursor,
            &mut stop_cursor,
        )?;
        let format = pm_gpu::working_format(precision);
        let (output, bpp) = match format {
            wgpu::TextureFormat::Rgba8Unorm => (0, 4),
            wgpu::TextureFormat::Rgba16Float => (1, 8),
            wgpu::TextureFormat::Rgba32Float => (2, 16),
            _ => return Err("unsupported SVG working format".into()),
        };
        if output == 2 {
            return Ok(Some((target, u64::from(w) * u64::from(h) * bpp)));
        }
        let result = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm svg float output"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = bind(
            device,
            &self.copy_layout,
            &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        );
        render_pass(encoder, &result, true, &self.copy[output], &bg);
        let _ = queue;
        Ok(Some((result, u64::from(w) * u64::from(h) * bpp)))
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_nodes(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        nodes: &[FloatNode],
        rect: PixelRect,
        paths: &[DrawEntry],
        values: &wgpu::Buffer,
        stops: &wgpu::Buffer,
        ranges: &[(u32, u32)],
        cursor: &mut usize,
        stop_cursor: &mut usize,
    ) -> Result<wgpu::Texture, String> {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let mut target = float_target(device, w, h);
        // Explicit clear also covers empty groups.
        {
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm svg float clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        for node in nodes {
            match node {
                FloatNode::Draw(draw) => {
                    let entry = &paths[*cursor];
                    *cursor += 1;
                    let range = ranges[*stop_cursor];
                    *stop_cursor += 1;
                    if entry.bounds.width == 0 || entry.bounds.height == 0 {
                        continue;
                    }
                    let params = draw_params(draw, entry, rect, range);
                    let ub = uniform(device, &params);
                    let bg = bind(
                        device,
                        &self.draw_layout,
                        &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: values.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: stops.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: ub.as_entire_binding(),
                            },
                        ],
                    );
                    render_pass(encoder, &target, false, &self.draw, &bg);
                }
                FloatNode::Group(group) => {
                    let front = self.paint_nodes(
                        device,
                        encoder,
                        &group.children,
                        rect,
                        paths,
                        values,
                        stops,
                        ranges,
                        cursor,
                        stop_cursor,
                    )?;
                    let clip = if let Some(nodes) = &group.clip {
                        Some(self.paint_nodes(
                            device,
                            encoder,
                            nodes,
                            rect,
                            paths,
                            values,
                            stops,
                            ranges,
                            cursor,
                            stop_cursor,
                        )?)
                    } else {
                        None
                    };
                    let mask = if let Some(nodes) = &group.mask {
                        Some(self.paint_nodes(
                            device,
                            encoder,
                            nodes,
                            rect,
                            paths,
                            values,
                            stops,
                            ranges,
                            cursor,
                            stop_cursor,
                        )?)
                    } else {
                        None
                    };
                    let result = float_target(device, w, h);
                    let back_view = target.create_view(&wgpu::TextureViewDescriptor::default());
                    let front_view = front.create_view(&wgpu::TextureViewDescriptor::default());
                    let clip_view = clip
                        .as_ref()
                        .unwrap_or(&front)
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let mask_view = mask
                        .as_ref()
                        .unwrap_or(&front)
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let ub = uniform(
                        device,
                        &[
                            group.opacity,
                            blend_code(group.blend) as f32,
                            if clip.is_some() { 1.0 } else { 0.0 },
                            if mask.is_none() {
                                0.0
                            } else if group.mask_luminance {
                                2.0
                            } else {
                                1.0
                            },
                        ],
                    );
                    let bg = bind(
                        device,
                        &self.composite_layout,
                        &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&back_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&front_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&clip_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&mask_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: ub.as_entire_binding(),
                            },
                        ],
                    );
                    render_pass(encoder, &result, true, &self.composite, &bg);
                    target = result;
                }
            }
        }
        Ok(target)
    }
}

fn collect(nodes: &[FloatNode], rect: PixelRect, out: &mut Vec<DrawEntry>) {
    for node in nodes {
        match node {
            FloatNode::Draw(d) => {
                let path =
                    Affine::translate((-f64::from(rect.x), -f64::from(rect.y))) * d.outline.clone();
                let bounds = coverage::bounds(&path, rect.width.max(1), rect.height.max(1))
                    .unwrap_or(PixelRect::new(0, 0, 0, 0));
                out.push(DrawEntry {
                    path,
                    bounds,
                    offset: 0,
                });
            }
            FloatNode::Group(g) => {
                collect(&g.children, rect, out);
                if let Some(c) = &g.clip {
                    collect(c, rect, out);
                }
                if let Some(m) = &g.mask {
                    collect(m, rect, out);
                }
            }
        }
    }
}
fn collect_stops(nodes: &[FloatNode]) -> (Vec<[f32; 8]>, Vec<(u32, u32)>) {
    fn walk(nodes: &[FloatNode], stops: &mut Vec<[f32; 8]>, ranges: &mut Vec<(u32, u32)>) {
        for node in nodes {
            match node {
                FloatNode::Draw(d) => {
                    let start = stops.len() as u32;
                    if let FloatPaint::Gradient { stops: colors, .. } = &d.paint {
                        for s in colors {
                            stops.push([
                                s.rgba[0], s.rgba[1], s.rgba[2], s.rgba[3], s.offset, 0.0, 0.0, 0.0,
                            ]);
                        }
                    }
                    ranges.push((start, stops.len() as u32 - start));
                }
                FloatNode::Group(g) => {
                    walk(&g.children, stops, ranges);
                    if let Some(c) = &g.clip {
                        walk(c, stops, ranges);
                    }
                    if let Some(m) = &g.mask {
                        walk(m, stops, ranges);
                    }
                }
            }
        }
    }
    let mut stops = Vec::new();
    let mut ranges = Vec::new();
    walk(nodes, &mut stops, &mut ranges);
    if stops.is_empty() {
        stops.push([0.0; 8]);
    }
    (stops, ranges)
}
fn draw_params(d: &FloatDraw, e: &DrawEntry, rect: PixelRect, range: (u32, u32)) -> [f32; 32] {
    let mut v = [0.0f32; 32];
    v[20..24].copy_from_slice(&[
        e.bounds.x as f32,
        e.bounds.y as f32,
        e.bounds.width as f32,
        e.bounds.height as f32,
    ]);
    v[24..28].copy_from_slice(&[
        f32::from_bits(e.offset as u32),
        f32::from_bits(range.0),
        f32::from_bits(range.1),
        0.0,
    ]);
    v[28..30].copy_from_slice(&[rect.x as f32, rect.y as f32]);
    match &d.paint {
        FloatPaint::Solid(color) => v[..4].copy_from_slice(color),
        FloatPaint::Gradient {
            geometry,
            spread,
            opacity,
            ..
        } => {
            let (a, b, inv, kind, r, fr) = match geometry {
                FloatGradient::Linear { p0, p1, inverse } => (*p0, *p1, *inverse, 1.0, 0.0, 0.0),
                FloatGradient::Radial {
                    focus,
                    center,
                    radius,
                    focus_radius,
                    inverse,
                } => (*focus, *center, *inverse, 2.0, *radius, *focus_radius),
            };
            v[4..8].copy_from_slice(&[inv[0], inv[2], inv[4], 0.0]);
            v[8..12].copy_from_slice(&[inv[1], inv[3], inv[5], 0.0]);
            v[12..16].copy_from_slice(&[a[0], a[1], b[0], b[1]]);
            v[16..20].copy_from_slice(&[r, fr, *opacity, kind]);
            v[27] = match spread {
                pm_vector::render::svg::SpreadMethod::Pad => 0.0,
                pm_vector::render::svg::SpreadMethod::Repeat => 1.0,
                pm_vector::render::svg::SpreadMethod::Reflect => 2.0,
            };
        }
    }
    v
}
fn blend_code(mode: pm_vector::render::svg::BlendMode) -> u32 {
    use pm_vector::render::svg::BlendMode::*;
    match mode {
        Normal => 0,
        Multiply => 1,
        Screen => 2,
        Overlay => 3,
        Darken => 4,
        Lighten => 5,
        ColorDodge => 6,
        ColorBurn => 7,
        HardLight => 8,
        SoftLight => 9,
        Difference => 10,
        Exclusion => 11,
        Hue => 12,
        Saturation => 13,
        Color => 14,
        Luminosity => 15,
    }
}
