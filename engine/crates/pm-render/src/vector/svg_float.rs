//! SVG paint in working float precision. Every path gets the same analytic
//! AreaCoverage as native shapes; paint and isolated groups stay RGBA32F
//! until the final working-format store.

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, BezPath, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::usvg_filter as filter;
use pm_vector::render::svg::{
    FloatDraw, FloatFilter, FloatGradient, FloatNode, FloatPaint, SvgDocument,
};
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
fn gradient_color(q0: vec2<f32>) -> vec4<f32> {
    let q = vec2<f32>(dot(p.v[1].xyz, vec3<f32>(q0, 1.0)), dot(p.v[2].xyz, vec3<f32>(q0, 1.0)));
    let t = extend(gradient_t(q), u32(p.v[6].w));
    let first = bitcast<u32>(p.v[6].y);
    let n = bitcast<u32>(p.v[6].z);
    if (n == 0u) { return vec4<f32>(0.0); }
    var rgba = stops[first].color;
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
    return rgba;
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let xy = vec2<i32>(floor(pos.xy));
    let box = vec4<i32>(p.v[5]);
    if (xy.x < box.x || xy.y < box.y || xy.x >= box.x + box.z || xy.y >= box.y + box.w) {
        discard;
    }
    let first = bitcast<u32>(p.v[6].x);
    let local = xy - box.xy;
    if (p.v[4].w < 0.5) {
        let cov = clamp(coverage[first + u32(local.y * box.z + local.x)], 0.0, 1.0);
        let alpha = p.v[0].a * cov;
        return vec4<f32>(p.v[0].rgb * alpha, alpha);
    }
    let rate = u32(p.v[7].z);
    let stride = u32(p.v[7].w);
    let origin = first + u32(local.y) * rate * stride + u32(local.x) * rate;
    var sum = vec4<f32>(0.0);
    for (var y = 0u; y < rate; y++) {
        for (var x = 0u; x < rate; x++) {
            let cov = clamp(coverage[origin + y * stride + x], 0.0, 1.0);
            let q0 = vec2<f32>(xy) + (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / f32(rate) + p.v[7].xy;
            let rgba = gradient_color(q0);
            let alpha = rgba.a * cov;
            sum += vec4<f32>(rgba.rgb * alpha, alpha);
        }
    }
    return sum / f32(rate * rate);
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
struct Clip { edges: vec4<f32> };
@group(0) @binding(1) var<uniform> clip: Clip;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u)*2.0-1.0, 1.0-f32(i >> 1u)*2.0,0.0,1.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = floor(pos.xy);
    let x = clamp(min(p.x + 1.0, clip.edges.z) - max(p.x, clip.edges.x), 0.0, 1.0);
    let y = clamp(min(p.y + 1.0, clip.edges.w) - max(p.y, clip.edges.y), 0.0, 1.0);
    return textureLoad(source,vec2<i32>(p),0) * (x * y);
}
"#;

/// A device-space image box can be clipped exactly by its pixel area.
/// Keeping this outside the SVG group tree avoids a 4x group grid for every
/// ordinary image layer. Rotated/sheared boxes still use the group path.
fn axis_clip(clip: &[Point], rect: PixelRect) -> Option<[f32; 4]> {
    let [a, b, c, d] = clip else { return None };
    let near = |x: f64, y: f64| (x - y).abs() <= 1e-7;
    if !near(a.y, b.y) || !near(b.x, c.x) || !near(c.y, d.y) || !near(d.x, a.x) {
        return None;
    }
    let x0 = a.x.min(c.x) - f64::from(rect.x);
    let y0 = a.y.min(c.y) - f64::from(rect.y);
    let x1 = a.x.max(c.x) - f64::from(rect.x);
    let y1 = a.y.max(c.y) - f64::from(rect.y);
    [x0, y0, x1, y1]
        .iter()
        .all(|v| v.is_finite() && v.abs() < 16_000_000.0)
        .then_some([x0 as f32, y0 as f32, x1 as f32, y1 as f32])
}

const RESOLVE: &str = r#"
struct Params { v: array<vec4<f32>, 2> };
@group(0) @binding(0) var fine: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u)*2.0-1.0, 1.0-f32(i >> 1u)*2.0,0.0,1.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let out_xy = vec2<i32>(floor(pos.xy));
    let xy = select(out_xy - vec2<i32>(p.v[0].xy), out_xy, p.v[1].y > 0.5);
    let size = vec2<i32>(p.v[0].zw);
    let rate = i32(p.v[1].x);
    if (xy.x < 0 || xy.y < 0 || xy.x >= size.x || xy.y >= size.y) {
        return vec4<f32>(0.0);
    }
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < rate; y++) {
        for (var x = 0; x < rate; x++) {
            let source_xy = xy + select(vec2<i32>(0), vec2<i32>(p.v[0].xy), p.v[1].y > 0.5);
            sum += textureLoad(fine, source_xy * rate + vec2<i32>(x, y), 0);
        }
    }
    return sum / f32(rate * rate);
}
"#;

const GROUP_RATE: u32 = 4;
const MAX_FINE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILTER_PIXEL_PASSES: u64 = 1_500_000_000;
// The tile-wide estimate includes shader fragments outside each filter's
// actual region. Keep the 640×360 ART native while rejecting canonical
// oversampling that multiplies its blur into hundreds of billions of taps.
const MAX_FILTER_KERNEL_SAMPLES: u64 = 64_000_000_000;

#[derive(Clone, Copy)]
struct FilterPaintGrid {
    rect: PixelRect,
    smooth_resolve: bool,
}

struct DrawEntry {
    path: BezPath,
    bounds: PixelRect,
    coverage_bounds: PixelRect,
    samples: u32,
    offset: u64,
}

/// Reuses the same GPU allocations across ordered tile commands in one
/// encoder. A target returns to the free list after its last encoded read;
/// ordered render passes make that reuse safe before submission.
#[derive(Default)]
struct ScratchPool {
    textures: std::collections::HashMap<(u32, u32), Vec<wgpu::Texture>>,
    busy: std::collections::HashSet<wgpu::Texture>,
    coverage: Vec<(u64, wgpu::Buffer)>,
}

impl ScratchPool {
    fn discard_unused(&mut self) {
        debug_assert!(self.busy.is_empty());
        self.textures.clear();
        self.coverage.clear();
    }

    fn reset(&mut self) {
        debug_assert!(
            self.busy.is_empty(),
            "previous SVG tile retained a live scratch target"
        );
        self.busy.clear();
    }

    fn target(&mut self, device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        // A larger cleared scratch target can serve a smaller edge tile.
        // This keeps the number of retained allocations independent of the
        // number of tiles and their varying edge dimensions.
        let key = self
            .textures
            .iter()
            .filter(|((tw, th), list)| {
                *tw >= w && *th >= h && list.iter().any(|texture| !self.busy.contains(texture))
            })
            .map(|(key, _)| *key)
            .min_by_key(|(tw, th)| (u64::from(*tw) * u64::from(*th), *tw, *th))
            .unwrap_or((w, h));
        let list = self.textures.entry(key).or_default();
        let out = if let Some(texture) = list.iter().find(|texture| !self.busy.contains(*texture)) {
            texture.clone()
        } else {
            let texture = float_target(device, key.0, key.1);
            list.push(texture.clone());
            texture
        };
        assert!(
            self.busy.insert(out.clone()),
            "scratch texture checked out twice"
        );
        out
    }

    fn release(&mut self, texture: &wgpu::Texture) {
        assert!(self.busy.remove(texture), "scratch texture released twice");
    }

    fn coverage(&mut self, device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
        if let Some((_, buffer)) = self
            .coverage
            .iter()
            .filter(|(size, _)| *size >= bytes.max(16))
            .min_by_key(|(size, _)| *size)
        {
            return buffer.clone();
        }
        let size = bytes.max(16);
        let buffer = coverage::buffer(device, size);
        self.coverage.push((size, buffer.clone()));
        buffer
    }

    fn allocated_bytes(&self) -> u64 {
        self.textures.iter().fold(
            self.coverage.iter().map(|(size, _)| *size).sum(),
            |sum, ((w, h), list)| {
                sum.saturating_add(
                    u64::from(*w)
                        .saturating_mul(u64::from(*h))
                        .saturating_mul(16)
                        .saturating_mul(list.len() as u64),
                )
            },
        )
    }

    fn projection(&self, coverage_bytes: Option<u64>) -> ScratchProjection {
        let mut slots = Vec::new();
        for (key, list) in &self.textures {
            slots.extend(std::iter::repeat_n(*key, list.len()));
        }
        let mut bytes = self.allocated_bytes();
        if let Some(size) = coverage_bytes.map(|size| size.max(16))
            && !self.coverage.iter().any(|(have, _)| *have >= size)
        {
            bytes = bytes.saturating_add(size);
        }
        ScratchProjection {
            slots,
            busy: Vec::new(),
            bytes,
        }
    }
}

struct ScratchProjection {
    slots: Vec<(u32, u32)>,
    busy: Vec<bool>,
    bytes: u64,
}

impl ScratchProjection {
    fn target(&mut self, (w, h): (u32, u32)) -> usize {
        let slot = self
            .slots
            .iter()
            .enumerate()
            .filter(|(i, (tw, th))| {
                *tw >= w && *th >= h && !self.busy.get(*i).copied().unwrap_or(false)
            })
            .min_by_key(|(i, (tw, th))| (u64::from(*tw) * u64::from(*th), *tw, *th, *i))
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                self.slots.push((w, h));
                self.bytes = self
                    .bytes
                    .saturating_add(u64::from(w).saturating_mul(u64::from(h)).saturating_mul(16));
                self.slots.len() - 1
            });
        if self.busy.len() < self.slots.len() {
            self.busy.resize(self.slots.len(), false);
        }
        assert!(!std::mem::replace(&mut self.busy[slot], true));
        slot
    }

    fn release(&mut self, slot: usize) {
        assert!(std::mem::replace(&mut self.busy[slot], false));
    }

    fn paint_filter_grid(
        &mut self,
        filter: &FloatFilter,
        graphic: usize,
        size: (u32, u32),
    ) -> usize {
        let mut created = Vec::new();
        for primitive in filter.filter.primitives() {
            let result = match primitive.kind() {
                filter::Kind::GaussianBlur(_) => {
                    let first = self.target(size);
                    let second = self.target(size);
                    self.release(first);
                    second
                }
                filter::Kind::Offset(_) => self.target(size),
                _ => continue, // SvgDocument rejects unsupported primitives
            };
            created.push(result);
        }
        let output = created.last().copied().unwrap_or(graphic);
        for slot in created {
            if slot != output {
                self.release(slot);
            }
        }
        output
    }

    fn paint_filter(&mut self, filter: &FloatFilter, graphic: usize, fine: (u32, u32)) -> usize {
        if matches!(filter.resolution, Some(None)) {
            return self.target(fine);
        }
        if let Some(Some([w, h])) = filter.resolution {
            let grid = (w, h);
            let source = self.target(grid);
            let filtered = self.paint_filter_grid(filter, source, grid);
            let output = self.target(fine);
            self.release(filtered);
            if filtered != source {
                self.release(source);
            }
            output
        } else {
            self.paint_filter_grid(filter, graphic, fine)
        }
    }

    fn paint_nodes(&mut self, nodes: &[FloatNode], fine: (u32, u32)) -> usize {
        let mut target = self.target(fine);
        for node in nodes {
            let FloatNode::Group(group) = node else {
                continue;
            };
            let mut front = self.paint_nodes(&group.children, fine);
            for filter in &group.filters {
                let filtered = self.paint_filter(filter, front, fine);
                if filtered != front {
                    self.release(front);
                }
                front = filtered;
            }
            let clip = group
                .clip
                .as_ref()
                .map(|nodes| self.paint_nodes(nodes, fine));
            let mask = group
                .mask
                .as_ref()
                .map(|nodes| self.paint_nodes(nodes, fine));
            let result = self.target(fine);
            self.release(target);
            self.release(front);
            if let Some(clip) = clip {
                self.release(clip);
            }
            if let Some(mask) = mask {
                self.release(mask);
            }
            target = result;
        }
        target
    }
}

pub struct SvgPainter {
    fallback_reason: Option<String>,
    coverage: AreaCoverage,
    draw: wgpu::RenderPipeline,
    draw_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    copy: [wgpu::RenderPipeline; 3],
    copy_layout: wgpu::BindGroupLayout,
    resolve: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    filter: wgpu::RenderPipeline,
    filter_layout: wgpu::BindGroupLayout,
    filter_resample: wgpu::RenderPipeline,
    filter_resample_layout: wgpu::BindGroupLayout,
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
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
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

fn clear_target(encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm svg clear"),
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

fn submit_tiled_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: wgpu::CommandEncoder,
) -> Result<(), String> {
    // Each tile creates coverage inputs, stop buffers, views and bind groups.
    // Waiting on this exact submission releases those transient resources
    // before the next batch and bounds sequential exports as well as one SVG.
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|error| format!("SVG tiled GPU submission failed: {error:?}"))?;
    Ok(())
}

impl SvgPainter {
    /// Float blending is optional on engine devices. Reject it before
    /// constructing pipelines so the caller can use the native fallback.
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !device
            .features()
            .contains(wgpu::Features::FLOAT32_BLENDABLE)
        {
            return None;
        }
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
        let rm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float resolve"),
            source: wgpu::ShaderSource::Wgsl(RESOLVE.into()),
        });
        let fm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svg_filters.wgsl").into()),
        });
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg filterRes resample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svg_filter_resample.wgsl").into()),
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
            entries: &[tex_entry(0), buffer_entry(1, false)],
        });
        let rl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float resolve"),
            entries: &[tex_entry(0), buffer_entry(1, false)],
        });
        let fl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float filter"),
            entries: &[tex_entry(0), buffer_entry(1, false), buffer_entry(2, true)],
        });
        let sl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg filterRes resample"),
            entries: &[tex_entry(0), buffer_entry(1, false)],
        });
        Some(Self {
            fallback_reason: None,
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
            resolve: pipeline(device, &rm, &rl, wgpu::TextureFormat::Rgba32Float, None),
            resolve_layout: rl,
            filter: pipeline(device, &fm, &fl, wgpu::TextureFormat::Rgba32Float, None),
            filter_layout: fl,
            filter_resample: pipeline(device, &sm, &sl, wgpu::TextureFormat::Rgba32Float, None),
            filter_resample_layout: sl,
        })
    }

    /// The runtime reason for the most recent resvg fallback, if any.
    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    /// Records a float SVG render. A scene beyond the native path's limits
    /// returns `None` so the caller can render every legacy-supported SVG.
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
        self.fallback_reason = None;
        match self.render_inner(
            device, queue, encoder, doc, transform, rect, clip, precision,
        ) {
            Ok(Some(image)) => Ok(Some(image)),
            Ok(None) => {
                self.fallback_reason = Some("SVG features require resvg".into());
                Ok(None)
            }
            Err(reason) => {
                self.fallback_reason = Some(reason);
                Ok(None)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_inner(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        doc: &SvgDocument,
        transform: Affine,
        rect: PixelRect,
        clip: &[Point],
        precision: Precision,
    ) -> Result<Option<(wgpu::Texture, u64)>, String> {
        let box_clip = axis_clip(clip, rect);
        let scene_clip = if box_clip.is_some() { &[][..] } else { clip };
        let Some(mut scene) = doc.float_scene(transform, scene_clip) else {
            return Ok(None);
        };
        if rect.width.max(1) > device.limits().max_texture_dimension_2d
            || rect.height.max(1) > device.limits().max_texture_dimension_2d
        {
            return Err("SVG output exceeds device texture dimensions".into());
        }
        if has_filters(&scene) {
            let target =
                self.render_joint_tiled(device, queue, doc, transform, rect, scene_clip)?;
            return self
                .finish_output(device, encoder, target, rect, precision, box_clip)
                .map(Some);
        }
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let mut paint_rect = rect;
        let mut resolve = None;
        let mut paths = Vec::new();
        if collect(&scene, rect, &mut paths).is_none() {
            return Err("SVG coverage coordinates overflow".into());
        }
        if needs_joint_samples(&scene) {
            let Some(bounds) = path_bounds(&paths) else {
                let target =
                    self.render_joint_tiled(device, queue, doc, transform, rect, scene_clip)?;
                return self
                    .finish_output(device, encoder, target, rect, precision, box_clip)
                    .map(Some);
            };
            let rate = GROUP_RATE;
            let Some(x) = rect
                .x
                .checked_add(bounds.x)
                .and_then(|v| v.checked_mul(rate as i32))
            else {
                return Err("SVG fine x coordinate overflow".into());
            };
            let Some(y) = rect
                .y
                .checked_add(bounds.y)
                .and_then(|v| v.checked_mul(rate as i32))
            else {
                return Err("SVG fine y coordinate overflow".into());
            };
            let Some(fw) = bounds.width.checked_mul(rate) else {
                return Err("SVG fine width overflow".into());
            };
            let Some(fh) = bounds.height.checked_mul(rate) else {
                return Err("SVG fine height overflow".into());
            };
            let max_dim = device.limits().max_texture_dimension_2d;
            let target_bytes = u64::from(fw)
                .saturating_mul(u64::from(fh))
                .saturating_mul(16)
                .saturating_mul(target_count(&scene));
            if fw > max_dim || fh > max_dim || target_bytes > MAX_FINE_BYTES {
                let target =
                    self.render_joint_tiled(device, queue, doc, transform, rect, scene_clip)?;
                return self
                    .finish_output(device, encoder, target, rect, precision, box_clip)
                    .map(Some);
            }
            let fine_clip: Vec<_> = scene_clip
                .iter()
                .map(|p| Point::new(p.x * f64::from(rate), p.y * f64::from(rate)))
                .collect();
            let Some(fine_scene) =
                doc.float_scene(Affine::scale(f64::from(rate)) * transform, &fine_clip)
            else {
                return Err("SVG fine scene changed during render".into());
            };
            let fine_rect = PixelRect::new(x, y, fw, fh);
            let mut fine_paths = Vec::new();
            if collect(&fine_scene, fine_rect, &mut fine_paths).is_none() {
                return Err("SVG fine coverage coordinates overflow".into());
            }
            let coverage_bytes = coverage_pixels(&fine_paths).saturating_mul(4);
            if target_bytes.saturating_add(coverage_bytes) > MAX_FINE_BYTES {
                let target =
                    self.render_joint_tiled(device, queue, doc, transform, rect, scene_clip)?;
                return self
                    .finish_output(device, encoder, target, rect, precision, box_clip)
                    .map(Some);
            }
            scene = fine_scene;
            paths = fine_paths;
            paint_rect = fine_rect;
            resolve = Some(bounds);
        }
        let limits = device.limits();
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size)
            .min(u64::from(u32::MAX) * 4)
            / 4;
        // Never lower the selected sampling rate to fit storage: that would
        // silently weaken the paint integration bound.
        if coverage_pixels(&paths) > limit {
            let target =
                self.render_joint_tiled(device, queue, doc, transform, rect, scene_clip)?;
            return self
                .finish_output(device, encoder, target, rect, precision, box_clip)
                .map(Some);
        }
        let items: Vec<_> = paths
            .iter()
            .map(|e| Item {
                path: &e.path,
                bounds: e.coverage_bounds,
                at: [0, 0],
            })
            .collect();
        let batch = Batch::new(&items);
        batch
            .fits_gpu_limits(&limits)
            .map_err(|reason| format!("SVG {reason}"))?;
        let (stops, ranges) = collect_stops(&scene);
        if (stops.len() as u64).saturating_mul(32)
            > limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size)
        {
            return Err("SVG gradient stop buffer exceeds GPU limit".into());
        }
        let values = coverage::buffer(device, batch.bytes());
        self.coverage.encode(device, encoder, &batch, &values, None);
        for (i, e) in paths.iter_mut().enumerate() {
            e.offset = batch.offset(i);
        }
        let stop_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pm svg float stops"),
            contents: bytes(stops.as_flattened()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut cursor = 0;
        let mut stop_cursor = 0;
        let mut scratch = ScratchPool::default();
        let target = self.paint_nodes(
            device,
            encoder,
            &scene,
            paint_rect,
            &paths,
            &values,
            &stop_buffer,
            &ranges,
            &mut cursor,
            &mut stop_cursor,
            &mut scratch,
            false,
        )?;
        let target = if let Some(bounds) = resolve {
            let output = float_target(device, w, h);
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let ub = uniform(
                device,
                &[
                    bounds.x as f32,
                    bounds.y as f32,
                    bounds.width as f32,
                    bounds.height as f32,
                    GROUP_RATE as f32,
                    0.0,
                    0.0,
                    0.0,
                ],
            );
            let bg = bind(
                device,
                &self.resolve_layout,
                &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: ub.as_entire_binding(),
                    },
                ],
            );
            render_pass(encoder, &output, true, &self.resolve, &bg);
            output
        } else {
            target
        };
        self.finish_output(device, encoder, target, rect, precision, box_clip)
            .map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_joint_tiled(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        doc: &SvgDocument,
        transform: Affine,
        rect: PixelRect,
        clip: &[Point],
    ) -> Result<wgpu::Texture, String> {
        if rect.width.max(1) > device.limits().max_texture_dimension_2d
            || rect.height.max(1) > device.limits().max_texture_dimension_2d
        {
            return Err("SVG output exceeds device texture dimensions".into());
        }
        let output = float_target(device, rect.width.max(1), rect.height.max(1));
        let mut tile_encoder = device.create_command_encoder(&Default::default());
        clear_target(&mut tile_encoder, &output);
        let scene = doc
            .float_scene(transform, clip)
            .ok_or_else(|| "SVG filter scene changed during tile render".to_owned())?;
        let visible = doc.user_bounds().and_then(|bounds| {
            PixelRect::round_out(transform.transform_rect_bbox(bounds)).intersect(rect)
        });
        let Some(visible) = visible else {
            submit_tiled_batch(device, queue, tile_encoder)?;
            return Ok(output);
        };
        let halo = filter_halo(&scene)?;
        let smooth_resolve = smooth_filter_resolve(&scene);
        let mut scratch = ScratchPool::default();
        const TILE: u32 = 32;
        const TILES_PER_SUBMIT: usize = 8;
        let mut pending = 0;
        let start_x = ((visible.x - rect.x) as u32 / TILE) * TILE;
        let start_y = ((visible.y - rect.y) as u32 / TILE) * TILE;
        let end_x = (visible.right() - rect.x) as u32;
        let end_y = (visible.bottom() - rect.y) as u32;
        let tiles_x = u64::from((end_x - start_x).div_ceil(TILE));
        let tiles_y = u64::from((end_y - start_y).div_ceil(TILE));
        let fine_w =
            u64::from(TILE.saturating_add(halo[0].saturating_mul(2))) * u64::from(GROUP_RATE);
        let fine_h =
            u64::from(TILE.saturating_add(halo[1].saturating_mul(2))) * u64::from(GROUP_RATE);
        let work = tiles_x
            .saturating_mul(tiles_y)
            .saturating_mul(fine_w)
            .saturating_mul(fine_h)
            .saturating_mul(target_count(&scene));
        if work > MAX_FILTER_PIXEL_PASSES {
            return Err(format!(
                "SVG filter work budget exceeded ({work} fine pixel passes)"
            ));
        }
        let kernel_samples = filter_kernel_work(&scene, fine_w.saturating_mul(fine_h))?
            .saturating_mul(tiles_x.saturating_mul(tiles_y));
        if kernel_samples > MAX_FILTER_KERNEL_SAMPLES {
            return Err(format!(
                "SVG filter convolution budget exceeded ({kernel_samples} kernel samples)"
            ));
        }
        for y in (start_y..end_y).step_by(TILE as usize) {
            for x in (start_x..end_x).step_by(TILE as usize) {
                let tile = PixelRect::new(
                    rect.x
                        .checked_add(i32::try_from(x).map_err(|_| "SVG tile x overflow")?)
                        .ok_or("SVG tile x overflow")?,
                    rect.y
                        .checked_add(i32::try_from(y).map_err(|_| "SVG tile y overflow")?)
                        .ok_or("SVG tile y overflow")?,
                    TILE.min(rect.width.max(1) - x),
                    TILE.min(rect.height.max(1) - y),
                );
                self.render_joint_tile(
                    device,
                    &mut tile_encoder,
                    doc,
                    transform,
                    rect,
                    clip,
                    tile,
                    halo,
                    smooth_resolve,
                    &output,
                    &mut scratch,
                )?;
                pending += 1;
                if pending == TILES_PER_SUBMIT {
                    submit_tiled_batch(device, queue, tile_encoder)?;
                    tile_encoder = device.create_command_encoder(&Default::default());
                    pending = 0;
                }
            }
        }
        if pending > 0 {
            submit_tiled_batch(device, queue, tile_encoder)?;
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_joint_tile(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        doc: &SvgDocument,
        transform: Affine,
        rect: PixelRect,
        clip: &[Point],
        tile: PixelRect,
        halo: [u32; 2],
        smooth_resolve: bool,
        output: &wgpu::Texture,
        scratch: &mut ScratchPool,
    ) -> Result<(), String> {
        let hx = i32::try_from(halo[0]).map_err(|_| "SVG halo x overflow")?;
        let hy = i32::try_from(halo[1]).map_err(|_| "SVG halo y overflow")?;
        let expanded = PixelRect::new(
            tile.x.checked_sub(hx).ok_or("SVG tile x overflow")?,
            tile.y.checked_sub(hy).ok_or("SVG tile y overflow")?,
            tile.width
                .checked_add(halo[0].checked_mul(2).ok_or("SVG halo width overflow")?)
                .ok_or("SVG tile width overflow")?,
            tile.height
                .checked_add(halo[1].checked_mul(2).ok_or("SVG halo height overflow")?)
                .ok_or("SVG tile height overflow")?,
        );
        let rate = GROUP_RATE;
        let fine_rect = PixelRect::new(
            expanded
                .x
                .checked_mul(rate as i32)
                .ok_or("SVG fine x overflow")?,
            expanded
                .y
                .checked_mul(rate as i32)
                .ok_or("SVG fine y overflow")?,
            expanded
                .width
                .checked_mul(rate)
                .ok_or("SVG fine width overflow")?,
            expanded
                .height
                .checked_mul(rate)
                .ok_or("SVG fine height overflow")?,
        );
        let fine_clip: Vec<_> = clip
            .iter()
            .map(|p| Point::new(p.x * f64::from(rate), p.y * f64::from(rate)))
            .collect();
        let scene = doc
            .float_scene(Affine::scale(f64::from(rate)) * transform, &fine_clip)
            .ok_or_else(|| "SVG filter scene changed during fine render".to_owned())?;
        let mut paths = Vec::new();
        collect(&scene, fine_rect, &mut paths)
            .ok_or_else(|| "SVG fine coverage coordinates overflow".to_owned())?;
        if paths
            .iter()
            .all(|path| path.bounds.width == 0 || path.bounds.height == 0)
        {
            return Ok(());
        }
        let coverage_bytes = coverage_pixels(&paths).saturating_mul(4);
        let mut projection = scratch.projection((!paths.is_empty()).then_some(coverage_bytes));
        let fine_size = (fine_rect.width, fine_rect.height);
        let painted = if paths.is_empty() {
            projection.target(fine_size)
        } else {
            projection.paint_nodes(&scene, fine_size)
        };
        let resolved = projection.target((tile.width, tile.height));
        projection.release(painted);
        projection.release(resolved);
        if projection.bytes > MAX_FINE_BYTES {
            // Prior edge/split tiles can leave incompatible free allocations.
            // Replan without them before treating the current tile as too big.
            scratch.discard_unused();
            projection = scratch.projection((!paths.is_empty()).then_some(coverage_bytes));
            let painted = projection.paint_nodes(&scene, fine_size);
            let resolved = projection.target((tile.width, tile.height));
            projection.release(painted);
            projection.release(resolved);
        }
        let projected = projection.bytes;
        let limits = device.limits();
        let limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let oversize = fine_rect.width > device.limits().max_texture_dimension_2d
            || fine_rect.height > device.limits().max_texture_dimension_2d
            || projected > MAX_FINE_BYTES
            || coverage_bytes > limit;
        if oversize {
            if tile.width <= 1 && tile.height <= 1 {
                return Err(format!(
                    "SVG joint tile exceeds device or 128 MiB fine-grid budget (scratch {projected} bytes, {} targets, fine {}×{}, halo {}×{})",
                    projection.slots.len(),
                    fine_rect.width,
                    fine_rect.height,
                    halo[0],
                    halo[1]
                ));
            }
            let (a, b) = if tile.width >= tile.height && tile.width > 1 {
                let half = tile.width / 2;
                (
                    PixelRect::new(tile.x, tile.y, half, tile.height),
                    PixelRect::new(tile.x + half as i32, tile.y, tile.width - half, tile.height),
                )
            } else {
                let half = tile.height / 2;
                (
                    PixelRect::new(tile.x, tile.y, tile.width, half),
                    PixelRect::new(tile.x, tile.y + half as i32, tile.width, tile.height - half),
                )
            };
            self.render_joint_tile(
                device,
                encoder,
                doc,
                transform,
                rect,
                clip,
                a,
                halo,
                smooth_resolve,
                output,
                scratch,
            )?;
            self.render_joint_tile(
                device,
                encoder,
                doc,
                transform,
                rect,
                clip,
                b,
                halo,
                smooth_resolve,
                output,
                scratch,
            )?;
            return Ok(());
        }
        scratch.reset();
        let tile_target = if paths.is_empty() {
            let blank = scratch.target(device, fine_rect.width, fine_rect.height);
            clear_target(encoder, &blank);
            blank
        } else {
            let items: Vec<_> = paths
                .iter()
                .map(|e| Item {
                    path: &e.path,
                    bounds: e.coverage_bounds,
                    at: [0, 0],
                })
                .collect();
            let batch = Batch::new(&items);
            batch
                .fits_gpu_limits(&limits)
                .map_err(|reason| format!("SVG tile {reason}"))?;
            let (stops, ranges) = collect_stops(&scene);
            if (stops.len() as u64).saturating_mul(32) > limit {
                return Err("SVG tile gradient stop buffer exceeds GPU limit".into());
            }
            let values = scratch.coverage(device, batch.bytes());
            self.coverage.encode(device, encoder, &batch, &values, None);
            for (i, e) in paths.iter_mut().enumerate() {
                e.offset = batch.offset(i);
            }
            let stop_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pm svg tile stops"),
                contents: bytes(stops.as_flattened()),
                usage: wgpu::BufferUsages::STORAGE,
            });
            self.paint_nodes(
                device,
                encoder,
                &scene,
                fine_rect,
                &paths,
                &values,
                &stop_buffer,
                &ranges,
                &mut 0,
                &mut 0,
                scratch,
                smooth_resolve,
            )?
        };
        let resolved = scratch.target(device, tile.width, tile.height);
        let view = tile_target.create_view(&wgpu::TextureViewDescriptor::default());
        let ub = uniform(
            device,
            &[
                (tile.x - expanded.x) as f32,
                (tile.y - expanded.y) as f32,
                tile.width as f32,
                tile.height as f32,
                rate as f32,
                1.0,
                0.0,
                0.0,
            ],
        );
        let bg = bind(
            device,
            &self.resolve_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
            ],
        );
        render_pass(encoder, &resolved, true, &self.resolve, &bg);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &resolved,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: output,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: (tile.x - rect.x) as u32,
                    y: (tile.y - rect.y) as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: tile.width,
                height: tile.height,
                depth_or_array_layers: 1,
            },
        );
        scratch.release(&tile_target);
        scratch.release(&resolved);
        if scratch.allocated_bytes() > MAX_FINE_BYTES {
            return Err("SVG scratch allocations exceed 128 MiB fine-grid budget".into());
        }
        Ok(())
    }

    fn finish_output(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: wgpu::Texture,
        rect: PixelRect,
        precision: Precision,
        box_clip: Option<[f32; 4]>,
    ) -> Result<(wgpu::Texture, u64), String> {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let format = pm_gpu::working_format(precision);
        let (output, bpp) = match format {
            wgpu::TextureFormat::Rgba8Unorm => (0, 4),
            wgpu::TextureFormat::Rgba16Float => (1, 8),
            wgpu::TextureFormat::Rgba32Float => (2, 16),
            _ => return Err("unsupported SVG working format".into()),
        };
        if output == 2 && box_clip.is_none() {
            return Ok((target, u64::from(w) * u64::from(h) * bpp));
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
        let clip = uniform(device, &box_clip.unwrap_or([0.0, 0.0, w as f32, h as f32]));
        let bg = bind(
            device,
            &self.copy_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: clip.as_entire_binding(),
                },
            ],
        );
        render_pass(encoder, &result, true, &self.copy[output], &bg);
        Ok((result, u64::from(w) * u64::from(h) * bpp))
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
        scratch: &mut ScratchPool,
        smooth_resolve: bool,
    ) -> Result<wgpu::Texture, String> {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let mut target = scratch.target(device, w, h);
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
                    let mut front = self.paint_nodes(
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
                        scratch,
                        smooth_resolve,
                    )?;
                    for filter in &group.filters {
                        let filtered = self.paint_filter(
                            device,
                            encoder,
                            filter,
                            &front,
                            FilterPaintGrid {
                                rect,
                                smooth_resolve,
                            },
                            scratch,
                        )?;
                        if filtered != front {
                            scratch.release(&front);
                        }
                        front = filtered;
                    }
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
                            scratch,
                            smooth_resolve,
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
                            scratch,
                            smooth_resolve,
                        )?)
                    } else {
                        None
                    };
                    let result = scratch.target(device, w, h);
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
                    scratch.release(&target);
                    scratch.release(&front);
                    if let Some(clip) = &clip {
                        scratch.release(clip);
                    }
                    if let Some(mask) = &mask {
                        scratch.release(mask);
                    }
                    target = result;
                }
            }
        }
        Ok(target)
    }

    fn paint_filter(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        filter: &FloatFilter,
        graphic: &wgpu::Texture,
        grid: FilterPaintGrid,
        scratch: &mut ScratchPool,
    ) -> Result<wgpu::Texture, String> {
        let FilterPaintGrid {
            rect,
            smooth_resolve,
        } = grid;
        if let Some(Some([w, h])) = filter.resolution {
            let r = filter.filter.rect();
            let region = [r.left(), r.top(), r.width(), r.height()];
            if w > device.limits().max_texture_dimension_2d
                || h > device.limits().max_texture_dimension_2d
                || u64::from(w) * u64::from(h) * 16 * 4 > MAX_FINE_BYTES
            {
                return Err("SVG filterRes grid exceeds device or 128 MiB budget".into());
            }
            let source = self.filter_resample_pass(
                device,
                encoder,
                graphic,
                [w, h],
                [w, h],
                region,
                filter.transform,
                [rect.x, rect.y],
                false,
                scratch,
            )?;
            let mut grid_filter = filter.clone();
            grid_filter.transform =
                Affine::scale_non_uniform(
                    f64::from(w) / f64::from(r.width()),
                    f64::from(h) / f64::from(r.height()),
                ) * Affine::translate((-f64::from(r.left()), -f64::from(r.top())));
            grid_filter.resolution = None;
            let grid_rect = PixelRect::new(0, 0, w, h);
            let filtered = self.paint_filter(
                device,
                encoder,
                &grid_filter,
                &source,
                FilterPaintGrid {
                    rect: grid_rect,
                    smooth_resolve: false,
                },
                scratch,
            )?;
            let output = self.filter_resample_pass(
                device,
                encoder,
                &filtered,
                [rect.width.max(1), rect.height.max(1)],
                [w, h],
                region,
                filter.transform.inverse(),
                [rect.x, rect.y],
                true,
                scratch,
            )?;
            scratch.release(&filtered);
            if filtered != source {
                scratch.release(&source);
            }
            return Ok(output);
        }
        let mut results = std::collections::HashMap::<String, wgpu::Texture>::new();
        let mut previous: Option<wgpu::Texture> = None;
        let mut created = Vec::new();
        let region = rect4(filter.filter.rect());
        let t = filter.transform;
        let inv = t.inverse().as_coeffs();
        if !inv.iter().all(|v| v.is_finite()) {
            return Err("singular SVG filter transform".into());
        }
        if matches!(filter.resolution, Some(None)) {
            let blank = scratch.target(device, rect.width.max(1), rect.height.max(1));
            clear_target(encoder, &blank);
            return Ok(blank);
        }
        let input = |source: &filter::Input,
                     results: &std::collections::HashMap<String, wgpu::Texture>|
         -> Result<(wgpu::Texture, bool), String> {
            match source {
                filter::Input::SourceGraphic => Ok((graphic.clone(), false)),
                filter::Input::SourceAlpha => Ok((graphic.clone(), true)),
                filter::Input::Reference(name) => results
                    .get(name)
                    .cloned()
                    .map(|v| (v, false))
                    .ok_or_else(|| format!("SVG filter input {name} is missing")),
            }
        };
        for primitive in filter.filter.primitives() {
            let bounds = rect4(primitive.rect());
            let linear = primitive.color_interpolation() == filter::ColorInterpolation::LinearRGB;
            let result = match primitive.kind() {
                filter::Kind::GaussianBlur(blur) => {
                    let (source, alpha) = input(blur.input(), &results)?;
                    let c = t.as_coeffs();
                    let axes = [
                        ([c[0], c[1]], f64::from(blur.std_dev_x().get())),
                        ([c[2], c[3]], f64::from(blur.std_dev_y().get())),
                    ];
                    let mut current = source;
                    for (stage, (v, sigma)) in axes.iter().enumerate() {
                        let length = v[0].hypot(v[1]);
                        let mut sigma = sigma * length;
                        if smooth_resolve && filter.resolution.is_none() && sigma > 0.0 {
                            // The final 4× resolve averages subpixel centers.
                            // Remove its variance so the reconstructed output
                            // has the authored Gaussian variance at pixel centers.
                            let subcell_variance = f64::from(GROUP_RATE * GROUP_RATE - 1) / 12.0;
                            if sigma * sigma > subcell_variance {
                                sigma = (sigma * sigma - subcell_variance).sqrt();
                            }
                        }
                        let axis = if length > 0.0 {
                            [v[0] / length, v[1] / length]
                        } else {
                            [1.0, 0.0]
                        };
                        let weights = gaussian_weights(sigma)?;
                        let next = self.filter_pass(
                            device,
                            encoder,
                            &current,
                            rect,
                            region,
                            bounds,
                            inv,
                            axis,
                            &weights,
                            stage as u32,
                            [0.0, 0.0],
                            alpha && stage == 0,
                            linear && stage == 0,
                            linear && stage == 1,
                            scratch,
                        );
                        if stage > 0 {
                            scratch.release(&current);
                        }
                        current = next;
                    }
                    current
                }
                filter::Kind::Offset(offset) => {
                    let (source, alpha) = input(offset.input(), &results)?;
                    let d = t.as_coeffs();
                    let shift = [
                        d[0] * f64::from(offset.dx()) + d[2] * f64::from(offset.dy()),
                        d[1] * f64::from(offset.dx()) + d[3] * f64::from(offset.dy()),
                    ];
                    self.filter_pass(
                        device,
                        encoder,
                        &source,
                        rect,
                        region,
                        bounds,
                        inv,
                        [1.0, 0.0],
                        &[1.0],
                        2,
                        shift,
                        alpha,
                        false,
                        false,
                        scratch,
                    )
                }
                kind => {
                    return Err(format!(
                        "unsupported SVG float filter primitive: {}",
                        filter_kind(kind)
                    ));
                }
            };
            results.insert(primitive.result().to_owned(), result.clone());
            created.push(result.clone());
            previous = Some(result);
        }
        let output = previous.unwrap_or_else(|| graphic.clone());
        for texture in created {
            if texture != output {
                scratch.release(&texture);
            }
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn filter_resample_pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        size: [u32; 2],
        grid: [u32; 2],
        region: [f32; 4],
        transform: Affine,
        origin: [i32; 2],
        project: bool,
        scratch: &mut ScratchPool,
    ) -> Result<wgpu::Texture, String> {
        let c = transform.as_coeffs();
        if !c.iter().all(|v| v.is_finite()) {
            return Err("singular SVG filterRes transform".into());
        }
        let p = [
            region[0],
            region[1],
            region[2],
            region[3],
            c[0] as f32,
            c[2] as f32,
            c[4] as f32,
            0.0,
            c[1] as f32,
            c[3] as f32,
            c[5] as f32,
            0.0,
            origin[0] as f32,
            origin[1] as f32,
            0.0,
            0.0,
            grid[0] as f32,
            grid[1] as f32,
            0.0,
            0.0,
            f32::from(project),
            GROUP_RATE as f32,
            0.0,
            0.0,
        ];
        let ub = uniform(device, &p);
        let view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = bind(
            device,
            &self.filter_resample_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
            ],
        );
        let out = scratch.target(device, size[0], size[1]);
        render_pass(encoder, &out, true, &self.filter_resample, &bg);
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn filter_pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        rect: PixelRect,
        region: [f32; 4],
        bounds: [f32; 4],
        inverse: [f64; 6],
        axis: [f64; 2],
        weights: &[f32],
        stage: u32,
        shift: [f64; 2],
        alpha: bool,
        to_linear: bool,
        to_srgb: bool,
        scratch: &mut ScratchPool,
    ) -> wgpu::Texture {
        let mut p = [0.0f32; 32];
        p[0..4].copy_from_slice(&[
            axis[0] as f32,
            axis[1] as f32,
            (weights.len() / 2) as f32,
            stage as f32,
        ]);
        p[4..8].copy_from_slice(&region);
        p[8..12].copy_from_slice(&bounds);
        p[12..16].copy_from_slice(&[
            inverse[0] as f32,
            inverse[2] as f32,
            inverse[1] as f32,
            inverse[3] as f32,
        ]);
        p[16..20].copy_from_slice(&[
            inverse[4] as f32,
            inverse[5] as f32,
            rect.x as f32,
            rect.y as f32,
        ]);
        p[20..24].copy_from_slice(&[
            shift[0] as f32,
            shift[1] as f32,
            f32::from(alpha),
            f32::from(to_linear),
        ]);
        p[24] = f32::from(to_srgb);
        let ub = uniform(device, &p);
        let wb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pm svg gaussian weights"),
            contents: bytes(weights),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = bind(
            device,
            &self.filter_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wb.as_entire_binding(),
                },
            ],
        );
        let out = scratch.target(device, rect.width.max(1), rect.height.max(1));
        render_pass(encoder, &out, true, &self.filter, &bg);
        out
    }
}

fn rect4(r: pm_vector::render::svg::SvgRect) -> [f32; 4] {
    [r.left(), r.top(), r.right(), r.bottom()]
}

fn gaussian_weights(sigma: f64) -> Result<Vec<f32>, String> {
    if !sigma.is_finite() || !(0.0..=4096.0).contains(&sigma) {
        return Err(format!("invalid SVG Gaussian standard deviation {sigma}"));
    }
    if sigma == 0.0 {
        return Ok(vec![1.0]);
    }
    let radius = (8.0 * sigma).ceil() as i32;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    Ok((-radius..=radius)
        .map(|k| {
            let k = f64::from(k);
            (phi((k + 0.5) / sigma) - phi((k - 0.5) / sigma)) as f32
        })
        .collect())
}

fn filter_kind(kind: &filter::Kind) -> &'static str {
    match kind {
        filter::Kind::Blend(_) => "feBlend",
        filter::Kind::ColorMatrix(_) => "feColorMatrix",
        filter::Kind::ComponentTransfer(_) => "feComponentTransfer",
        filter::Kind::Composite(_) => "feComposite",
        filter::Kind::ConvolveMatrix(_) => "feConvolveMatrix",
        filter::Kind::DiffuseLighting(_) => "feDiffuseLighting",
        filter::Kind::DisplacementMap(_) => "feDisplacementMap",
        filter::Kind::DropShadow(_) => "feDropShadow",
        filter::Kind::Flood(_) => "feFlood",
        filter::Kind::GaussianBlur(_) => "feGaussianBlur",
        filter::Kind::Image(_) => "feImage",
        filter::Kind::Merge(_) => "feMerge",
        filter::Kind::Morphology(_) => "feMorphology",
        filter::Kind::Offset(_) => "feOffset",
        filter::Kind::SpecularLighting(_) => "feSpecularLighting",
        filter::Kind::Tile(_) => "feTile",
        filter::Kind::Turbulence(_) => "feTurbulence",
    }
}

fn needs_joint_samples(nodes: &[FloatNode]) -> bool {
    nodes.iter().any(|node| match node {
        FloatNode::Draw(_) => false,
        FloatNode::Group(group) => {
            group.clip.is_some()
                || group.mask.is_some()
                || !group.filters.is_empty()
                || needs_joint_samples(&group.children)
        }
    })
}

fn target_count(nodes: &[FloatNode]) -> u64 {
    1 + nodes
        .iter()
        .filter_map(|node| match node {
            FloatNode::Draw(_) => None,
            FloatNode::Group(g) => Some(
                1 + 2 * g
                    .filters
                    .iter()
                    .map(|f| f.filter.primitives().len() as u64)
                    .sum::<u64>()
                    + target_count(&g.children)
                    + g.clip.as_deref().map(target_count).unwrap_or(0)
                    + g.mask.as_deref().map(target_count).unwrap_or(0),
            ),
        })
        .sum::<u64>()
}

fn has_filters(nodes: &[FloatNode]) -> bool {
    nodes.iter().any(|node| match node {
        FloatNode::Draw(_) => false,
        FloatNode::Group(g) => {
            !g.filters.is_empty()
                || has_filters(&g.children)
                || g.clip.as_deref().is_some_and(has_filters)
                || g.mask.as_deref().is_some_and(has_filters)
        }
    })
}

/// Allow the Gaussian variance correction for the final 4× box resolve only
/// when every hard group/filter boundary falls on whole output pixels.
fn smooth_filter_resolve(nodes: &[FloatNode]) -> bool {
    fn aligned(r: pm_vector::render::svg::SvgRect, t: Affine) -> bool {
        let c = t.as_coeffs();
        let edge = |v: f64| v.is_finite() && (v - v.round()).abs() < 1e-6;
        c[1].abs() < 1e-9
            && c[2].abs() < 1e-9
            && edge(c[0] * f64::from(r.left()) + c[4])
            && edge(c[0] * f64::from(r.right()) + c[4])
            && edge(c[3] * f64::from(r.top()) + c[5])
            && edge(c[3] * f64::from(r.bottom()) + c[5])
    }
    nodes.iter().all(|node| match node {
        FloatNode::Draw(_) => true,
        FloatNode::Group(g) => {
            g.clip.is_none()
                && g.mask.is_none()
                && g.filters.iter().all(|f| {
                    aligned(f.filter.rect(), f.transform)
                        && f.filter
                            .primitives()
                            .iter()
                            .all(|p| aligned(p.rect(), f.transform))
                })
                && smooth_filter_resolve(&g.children)
        }
    })
}

/// Upper bound on separable Gaussian texture loads for one output tile.
/// A canonical source at 8x can turn a modest authored blur into a
/// thousand-tap fine-grid convolution; count those taps before encoding.
fn filter_kernel_work(nodes: &[FloatNode], fine_pixels: u64) -> Result<u64, String> {
    let mut work = 0u64;
    for node in nodes {
        if let FloatNode::Group(group) = node {
            for filter in &group.filters {
                if matches!(filter.resolution, Some(None)) {
                    continue;
                }
                let (pixels, axes) = if let Some(Some([width, height])) = filter.resolution {
                    let region = filter.filter.rect();
                    let rw = f64::from(region.width());
                    let rh = f64::from(region.height());
                    if rw <= 0.0 || rh <= 0.0 {
                        return Err("SVG filterRes region has no area".into());
                    }
                    (
                        u64::from(width).saturating_mul(u64::from(height)),
                        [f64::from(width) / rw, f64::from(height) / rh],
                    )
                } else {
                    let c = filter.transform.as_coeffs();
                    (
                        fine_pixels,
                        [c[0].hypot(c[1]) * f64::from(GROUP_RATE),
                         c[2].hypot(c[3]) * f64::from(GROUP_RATE)],
                    )
                };
                for primitive in filter.filter.primitives() {
                    if let filter::Kind::GaussianBlur(blur) = primitive.kind() {
                        for (sigma, axis) in [
                            (f64::from(blur.std_dev_x().get()), axes[0]),
                            (f64::from(blur.std_dev_y().get()), axes[1]),
                        ] {
                            let scaled = sigma * axis;
                            if !scaled.is_finite() || scaled > 4096.0 {
                                return Err(format!(
                                    "SVG filter convolution exceeds device kernel limit (sigma {scaled})"
                                ));
                            }
                            let taps = (8.0 * scaled).ceil() as u64 * 2 + 1;
                            work = work.saturating_add(pixels.saturating_mul(taps));
                        }
                    }
                }
            }
            work = work.saturating_add(filter_kernel_work(&group.children, fine_pixels)?);
            if let Some(clip) = &group.clip {
                work = work.saturating_add(filter_kernel_work(clip, fine_pixels)?);
            }
            if let Some(mask) = &group.mask {
                work = work.saturating_add(filter_kernel_work(mask, fine_pixels)?);
            }
        }
    }
    Ok(work)
}

/// A conservative device-pixel support radius of every supported primitive
/// along every path through the scene. The 8σ tails are below f32 precision.
fn filter_halo(nodes: &[FloatNode]) -> Result<[u32; 2], String> {
    let mut halo = [2.0f64; 2];
    fn walk(nodes: &[FloatNode], halo: &mut [f64; 2]) -> Result<(), String> {
        for node in nodes {
            if let FloatNode::Group(g) = node {
                for f in &g.filters {
                    let c = f.transform.as_coeffs();
                    if let Some(Some([rw, rh])) = f.resolution {
                        let r = f.filter.rect();
                        let cell_x = f64::from(r.width()) / f64::from(rw);
                        let cell_y = f64::from(r.height()) / f64::from(rh);
                        for axis in 0..2 {
                            halo[axis] +=
                                2.0 * (c[axis].abs() * cell_x + c[axis + 2].abs() * cell_y);
                        }
                    }
                    for p in f.filter.primitives() {
                        match p.kind() {
                            filter::Kind::GaussianBlur(b) => {
                                for axis in 0..2 {
                                    halo[axis] += 8.0
                                        * (f64::from(b.std_dev_x().get()) * c[axis].abs()
                                            + f64::from(b.std_dev_y().get()) * c[axis + 2].abs());
                                }
                            }
                            filter::Kind::Offset(o) => {
                                for axis in 0..2 {
                                    halo[axis] += (f64::from(o.dx()) * c[axis]
                                        + f64::from(o.dy()) * c[axis + 2])
                                        .abs();
                                }
                            }
                            other => {
                                return Err(format!(
                                    "unsupported SVG float filter primitive: {}",
                                    filter_kind(other)
                                ));
                            }
                        }
                    }
                }
                walk(&g.children, halo)?;
                if let Some(c) = &g.clip {
                    walk(c, halo)?;
                }
                if let Some(m) = &g.mask {
                    walk(m, halo)?;
                }
            }
        }
        Ok(())
    }
    walk(nodes, &mut halo)?;
    if !halo
        .iter()
        .all(|v| v.is_finite() && *v < i32::MAX as f64 / 4.0)
    {
        return Err("SVG filter halo exceeds device coordinates".into());
    }
    Ok(halo.map(|v| v.ceil() as u32))
}

fn coverage_pixels(paths: &[DrawEntry]) -> u64 {
    paths.iter().fold(0u64, |sum, e| {
        sum.saturating_add(u64::from(e.coverage_bounds.width) * u64::from(e.coverage_bounds.height))
    })
}

fn path_bounds(paths: &[DrawEntry]) -> Option<PixelRect> {
    let mut x0 = i32::MAX;
    let mut y0 = i32::MAX;
    let mut x1 = 0i64;
    let mut y1 = 0i64;
    for e in paths {
        if e.bounds.width == 0 || e.bounds.height == 0 {
            continue;
        }
        x0 = x0.min(e.bounds.x);
        y0 = y0.min(e.bounds.y);
        x1 = x1.max(i64::from(e.bounds.x) + i64::from(e.bounds.width));
        y1 = y1.max(i64::from(e.bounds.y) + i64::from(e.bounds.height));
    }
    (x0 != i32::MAX).then(|| {
        PixelRect::new(
            x0,
            y0,
            (x1 - i64::from(x0)) as u32,
            (y1 - i64::from(y0)) as u32,
        )
    })
}

fn collect(nodes: &[FloatNode], rect: PixelRect, out: &mut Vec<DrawEntry>) -> Option<()> {
    for node in nodes {
        match node {
            FloatNode::Draw(d) => {
                let path =
                    Affine::translate((-f64::from(rect.x), -f64::from(rect.y))) * d.outline.clone();
                let bounds = coverage::bounds(&path, rect.width.max(1), rect.height.max(1))
                    .unwrap_or(PixelRect::new(0, 0, 0, 0));
                let samples = gradient_samples(&d.paint);
                if u64::from(bounds.width) * u64::from(samples) > u64::from(u32::MAX)
                    || u64::from(bounds.height) * u64::from(samples) > u64::from(u32::MAX)
                    || i64::from(bounds.x) * i64::from(samples) > i64::from(i32::MAX)
                    || i64::from(bounds.y) * i64::from(samples) > i64::from(i32::MAX)
                {
                    return None;
                }
                out.push(DrawEntry {
                    path: Affine::scale(f64::from(samples)) * path,
                    bounds,
                    coverage_bounds: sample_bounds(bounds, samples),
                    samples,
                    offset: 0,
                });
            }
            FloatNode::Group(g) => {
                collect(&g.children, rect, out)?;
                if let Some(c) = &g.clip {
                    collect(c, rect, out)?;
                }
                if let Some(m) = &g.mask {
                    collect(m, rect, out)?;
                }
            }
        }
    }
    Some(())
}

fn sample_bounds(bounds: PixelRect, rate: u32) -> PixelRect {
    PixelRect::new(
        bounds.x * rate as i32,
        bounds.y * rate as i32,
        bounds.width * rate,
        bounds.height * rate,
    )
}

/// Gradient change across one device pixel, in worst-case 8-bit code values.
/// Radials receive more samples near a focal circle tangent to the outer
/// circle; solids keep exact one-pixel coverage.
fn gradient_samples(paint: &FloatPaint) -> u32 {
    let FloatPaint::Gradient {
        geometry,
        stops,
        spread,
        ..
    } = paint
    else {
        return 1;
    };
    let per_pixel = match geometry {
        FloatGradient::Linear { p0, p1, inverse } => {
            let d = [f64::from(p1[0] - p0[0]), f64::from(p1[1] - p0[1])];
            let length2 = (d[0] * d[0] + d[1] * d[1]).max(1e-30);
            ((f64::from(inverse[0]) * d[0] + f64::from(inverse[1]) * d[1]).abs()
                + (f64::from(inverse[2]) * d[0] + f64::from(inverse[3]) * d[1]).abs())
                / length2
        }
        FloatGradient::Radial {
            focus,
            center,
            radius,
            focus_radius,
            inverse,
        } => {
            // On |q - F - t(C-F)| = fr + t(r-fr), implicit
            // differentiation gives |grad_q t| = 1 / |u·(C-F) + r-fr|.
            // Its worst angular direction has denominator (r-fr)-|C-F|.
            let dr = f64::from(*radius - *focus_radius);
            let separation = f64::from(center[0] - focus[0]).hypot(f64::from(center[1] - focus[1]));
            (f64::from(inverse[0]).hypot(f64::from(inverse[1]))
                + f64::from(inverse[2]).hypot(f64::from(inverse[3])))
                / (dr - separation).max(1e-12)
        }
    };
    let mut narrowest = 1.0f64;
    for pair in stops.windows(2) {
        let span = f64::from(pair[1].offset - pair[0].offset);
        if span <= 0.0 && pair[0].rgba != pair[1].rgba {
            return 16;
        }
        if span > 0.0 {
            narrowest = narrowest.min(span);
        }
    }
    if matches!(spread, pm_vector::render::svg::SpreadMethod::Repeat)
        && stops
            .first()
            .zip(stops.last())
            .is_some_and(|(a, b)| a.rgba != b.rgba)
    {
        return 16;
    }
    let cv = 255.0 * per_pixel / narrowest;
    if cv < 0.5 {
        1
    } else if cv < 2.0 {
        2
    } else if cv < 4.0 {
        4
    } else if cv < 8.0 {
        8
    } else {
        16
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
    v[28..32].copy_from_slice(&[
        rect.x as f32,
        rect.y as f32,
        e.samples as f32,
        e.coverage_bounds.width as f32,
    ]);
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
