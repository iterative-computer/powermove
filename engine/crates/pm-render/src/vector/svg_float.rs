//! SVG paint in working float precision. Every path gets the same analytic
//! AreaCoverage as native shapes; paint and isolated groups stay RGBA32F
//! until the final working-format store.

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, BezPath, Point, Rect, Shape};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::{
    FloatDraw, FloatGradient, FloatGroup, FloatNode, FloatPaint, SvgDocument,
};
use wgpu::util::DeviceExt;

use super::svg_filter::{self, Filters};
use crate::coverage::{self, AreaCoverage, Batch, Item};

const DRAW: &str = r#"
struct Params { v: array<vec4<f32>, 8> };
struct Stop { color: vec4<f32>, position: vec4<f32> };
@group(0) @binding(0) var<storage, read> coverage: array<f32>;
@group(0) @binding(1) var<storage, read> stops: array<Stop>;
@group(0) @binding(2) var<uniform> p: Params;
@group(0) @binding(3) var image: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 2.0 - 1.0;
    let y = 1.0 - f32(i >> 1u) * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}
fn texel(t: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(image));
    if (any(t < vec2<i32>(0)) || any(t >= size)) { return vec4<f32>(0.0); }
    return textureLoad(image, t, 0);
}
// The monotonized central slope of differences `a` (left) and `b` (right).
fn limited(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let central = 0.5 * (a + b);
    let m = min(abs(central), 2.0 * min(abs(a), abs(b)));
    return select(vec4<f32>(0.0), sign(central) * m, a * b > vec4<f32>(0.0));
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
    // A filtered group's raster (`FloatPaint::Image`): its texel at the
    // grid pixel's device pixel. On a finer grid, a linear reconstruction
    // that keeps each texel's average (limited, so it makes no new extrema)
    // gives clips and masks the raster's slope within the pixel.
    if (p.v[4].w > 2.5) {
        let rate = p.v[1].z;
        let g = vec2<f32>(xy) + p.v[7].xy;
        let device = floor(g / rate);
        let t = vec2<i32>(device - p.v[1].xy);
        var c = texel(t);
        if (rate > 1.5) {
            let o = (g + vec2<f32>(0.5)) / rate - device - vec2<f32>(0.5);
            c += o.x * limited(c - texel(t - vec2<i32>(1, 0)), texel(t + vec2<i32>(1, 0)) - c);
            c += o.y * limited(c - texel(t - vec2<i32>(0, 1)), texel(t + vec2<i32>(0, 1)) - c);
        }
        return c * p.v[1].w;
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

/// The W3C separable and non-separable blend functions on straight colour
/// (`mode`: [`blend_code`]), shared with feBlend.
pub(super) const BLEND_WGSL: &str = r#"
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
fn clipped(pos: vec4<f32>) -> vec4<f32> {
    let p = floor(pos.xy);
    let x = clamp(min(p.x + 1.0, clip.edges.z) - max(p.x, clip.edges.x), 0.0, 1.0);
    let y = clamp(min(p.y + 1.0, clip.edges.w) - max(p.y, clip.edges.y), 0.0, 1.0);
    return textureLoad(source,vec2<i32>(p),0) * (x * y);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> { return clipped(pos); }
// RGBA16F stores the nearest half (`crate::nearest_half`).
@fragment fn fs_half(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return pm_scene_half_color(clipped(pos));
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
/// Live images of one filter evaluation (a filtered group's canvas). A
/// larger canvas splits its output into tiles, each with its own reach.
pub(super) const MAX_FILTER_BYTES: u64 = 256 * 1024 * 1024;
/// Fine-grid pixel passes (tiles × fine area × targets) of one tiled
/// joint render, preflighted before any command is encoded. Beyond them the
/// image uses the declared resvg fallback with the reason, never a long GPU
/// stall.
const MAX_TILED_PIXEL_PASSES: u64 = 1_500_000_000;
/// Device pixels a joint tile reads around itself: the subpixel grid's
/// coverage reach.
const JOINT_HALO: [u32; 2] = [2, 2];

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
pub(super) struct ScratchPool {
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

    pub(super) fn target(&mut self, device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
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

    pub(super) fn release(&mut self, texture: &wgpu::Texture) {
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

    fn paint_nodes(&mut self, nodes: &[FloatNode], fine: (u32, u32)) -> usize {
        let mut target = self.target(fine);
        for node in nodes {
            let FloatNode::Group(group) = node else {
                continue;
            };
            debug_assert!(
                group.filters.is_empty(),
                "filters are drawn before painting"
            );
            let front = self.paint_nodes(&group.children, fine);
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
    filters: Filters,
    /// Bound as the image of draws that have none.
    blank: wgpu::Texture,
}

pub(super) fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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
pub(super) fn buffer_entry(binding: u32, storage: bool) -> wgpu::BindGroupLayoutEntry {
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
pub(super) fn pipeline(
    device: &wgpu::Device,
    (module, entry): (&wgpu::ShaderModule, &str),
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
}
pub(super) fn float_target(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
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
pub(super) fn uniform(device: &wgpu::Device, data: &[f32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pm svg float uniform"),
        contents: bytes(data),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}
pub(super) fn bind(
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
pub(super) fn render_pass(
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

pub(super) fn clear_target(encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture) {
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

/// Tile batches in flight. Each batch is a short command buffer (a long one
/// can trip the GPU watchdog); waiting for the batch two back bounds the
/// transient coverage inputs, stop buffers and bind groups of a large SVG
/// without stalling small ones, whose one or two batches never wait.
#[derive(Default)]
struct TileSubmissions(std::collections::VecDeque<wgpu::SubmissionIndex>);

impl TileSubmissions {
    const IN_FLIGHT: usize = 2;

    fn submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
    ) -> Result<(), String> {
        self.0.push_back(queue.submit([encoder.finish()]));
        if self.0.len() > Self::IN_FLIGHT {
            let oldest = self.0.pop_front().expect("a submission is in flight");
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(oldest),
                    timeout: None,
                })
                .map_err(|error| format!("SVG tiled GPU submission failed: {error:?}"))?;
        }
        Ok(())
    }
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
            source: wgpu::ShaderSource::Wgsl(format!("{BLEND_WGSL}\n{COMPOSITE}").into()),
        });
        let xm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float copy"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{COPY}", crate::nearest_half::NEAREST_HALF_WGSL).into(),
            ),
        });
        let rm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg float resolve"),
            source: wgpu::ShaderSource::Wgsl(RESOLVE.into()),
        });
        let dl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg float draw"),
            entries: &[
                buffer_entry(0, true),
                buffer_entry(1, true),
                buffer_entry(2, false),
                tex_entry(3),
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
        Some(Self {
            fallback_reason: None,
            coverage: AreaCoverage::new(device),
            draw: pipeline(
                device,
                (&dm, "fs"),
                &dl,
                wgpu::TextureFormat::Rgba32Float,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            draw_layout: dl,
            composite: pipeline(
                device,
                (&cm, "fs"),
                &cl,
                wgpu::TextureFormat::Rgba32Float,
                None,
            ),
            composite_layout: cl,
            copy: [
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Rgba32Float,
            ]
            .map(|f| {
                let entry = if f == wgpu::TextureFormat::Rgba16Float {
                    "fs_half"
                } else {
                    "fs"
                };
                pipeline(device, (&xm, entry), &xl, f, None)
            }),
            copy_layout: xl,
            resolve: pipeline(
                device,
                (&rm, "fs"),
                &rl,
                wgpu::TextureFormat::Rgba32Float,
                None,
            ),
            resolve_layout: rl,
            filters: Filters::new(device),
            blank: float_target(device, 1, 1),
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
                self.fallback_reason = Some(match doc.unsupported_feature() {
                    Some(feature) => format!("SVG {feature} is drawn by resvg"),
                    None => "SVG transform is not finite".into(),
                });
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
        let rate = f64::from(GROUP_RATE);
        let fine_transform = Affine::scale(rate) * transform;
        let fine_clip: Vec<_> = scene_clip
            .iter()
            .map(|p| Point::new(p.x * rate, p.y * rate))
            .collect();
        let fine_scene = || {
            doc.float_scene(fine_transform, &fine_clip)
                .ok_or_else(|| "SVG fine scene changed during render".to_owned())
        };
        let visible = doc.user_bounds().and_then(|bounds| {
            PixelRect::round_out(transform.transform_rect_bbox(bounds)).intersect(rect)
        });
        let target = if has_filters(&scene) {
            // Filtered groups draw first, at device resolution; the scene
            // then paints their results like any other raster.
            let mut fine = fine_scene()?;
            let mut images = Vec::new();
            self.substitute(device, queue, &mut scene, &mut fine, rect, &mut images)?;
            self.paint_scene(
                device,
                queue,
                encoder,
                &scene,
                &|| Ok(fine.clone()),
                rect,
                visible,
                &images,
            )?
        } else {
            self.paint_scene(
                device,
                queue,
                encoder,
                &scene,
                &fine_scene,
                rect,
                visible,
                &[],
            )?
        };
        self.finish_output(device, encoder, target, rect, precision, box_clip)
            .map(Some)
    }

    /// Paints a scene without filters over `rect` (RGBA32F): at device
    /// resolution, on the joint 4× grid (`fine`, the same scene under a 4×
    /// transform) when clips or masks need it, or in tiles of that grid.
    /// `visible` bounds the tiles; `images` are the scene's rasters.
    #[allow(clippy::too_many_arguments)]
    fn paint_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &[FloatNode],
        fine: &dyn Fn() -> Result<Vec<FloatNode>, String>,
        rect: PixelRect,
        visible: Option<PixelRect>,
        images: &[wgpu::Texture],
    ) -> Result<wgpu::Texture, String> {
        let tiled = || self.render_joint_tiled(device, queue, &fine()?, rect, visible, images);
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let mut paint_rect = rect;
        let mut resolve = None;
        let mut paths = Vec::new();
        if collect(scene, rect, &mut paths).is_none() {
            return Err("SVG coverage coordinates overflow".into());
        }
        let fine_scene;
        let mut scene = scene;
        if needs_joint_samples(scene) {
            let Some(bounds) = path_bounds(&paths) else {
                return tiled();
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
                .saturating_mul(target_count(scene));
            if fw > max_dim || fh > max_dim || target_bytes > MAX_FINE_BYTES {
                return tiled();
            }
            fine_scene = fine()?;
            let fine_rect = PixelRect::new(x, y, fw, fh);
            let mut fine_paths = Vec::new();
            if collect(&fine_scene, fine_rect, &mut fine_paths).is_none() {
                return Err("SVG fine coverage coordinates overflow".into());
            }
            let coverage_bytes = coverage_pixels(&fine_paths).saturating_mul(4);
            if target_bytes.saturating_add(coverage_bytes) > MAX_FINE_BYTES {
                return tiled();
            }
            scene = &fine_scene;
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
            return tiled();
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
        let (stops, ranges) = collect_stops(scene);
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
            scene,
            paint_rect,
            &paths,
            &values,
            &stop_buffer,
            &ranges,
            &mut cursor,
            &mut stop_cursor,
            &mut scratch,
            images,
        )?;
        let Some(bounds) = resolve else {
            return Ok(target);
        };
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
        Ok(output)
    }

    /// Replaces each outermost filtered group of `scene` (and of `fine`,
    /// its 4× twin) with a draw of the group's filter result over `out`;
    /// the group keeps its clip, mask, opacity and blend.
    fn substitute(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &mut [FloatNode],
        fine: &mut [FloatNode],
        out: PixelRect,
        images: &mut Vec<wgpu::Texture>,
    ) -> Result<(), String> {
        if scene.len() != fine.len() {
            return Err("SVG fine scene differs from the device scene".into());
        }
        for i in 0..scene.len() {
            let (FloatNode::Group(dg), FloatNode::Group(fg)) = (&mut scene[i], &mut fine[i]) else {
                continue;
            };
            if let (Some(dm), Some(fm)) = (dg.mask.as_mut(), fg.mask.as_mut()) {
                self.substitute(device, queue, dm, fm, out, images)?;
            }
            if dg.filters.is_empty() {
                self.substitute(
                    device,
                    queue,
                    &mut dg.children,
                    &mut fg.children,
                    out,
                    images,
                )?;
                continue;
            }
            let result = self.filter_group(device, queue, dg, fg, out, images)?;
            let draw = |rate: u32, opacity: f32| {
                result.map(|(index, rect)| {
                    let r = f64::from(rate);
                    FloatNode::Draw(FloatDraw {
                        outline: Rect::new(
                            f64::from(rect.x) * r,
                            f64::from(rect.y) * r,
                            f64::from(rect.right()) * r,
                            f64::from(rect.bottom()) * r,
                        )
                        .to_path(0.1),
                        paint: FloatPaint::Image {
                            index,
                            origin: [rect.x, rect.y],
                            rate,
                            opacity,
                        },
                    })
                })
            };
            // A group that only carries opacity is its result's draw.
            if dg.clip.is_none()
                && dg.mask.is_none()
                && dg.blend == pm_vector::render::svg::BlendMode::Normal
            {
                let opacity = dg.opacity;
                let (d, f) = (draw(1, opacity), draw(GROUP_RATE, opacity));
                match (d, f) {
                    (Some(d), Some(f)) => {
                        scene[i] = d;
                        fine[i] = f;
                    }
                    _ => {
                        dg.children.clear();
                        dg.filters.clear();
                        fg.children.clear();
                        fg.filters.clear();
                    }
                }
                continue;
            }
            dg.filters.clear();
            fg.filters.clear();
            dg.children = draw(1, 1.0).into_iter().collect();
            fg.children = draw(GROUP_RATE, 1.0).into_iter().collect();
        }
        Ok(())
    }

    /// A filtered group's result over `out`: its image (pushed onto
    /// `images`, at device resolution) and the device rectangle it covers,
    /// or `None` where no filter region reaches `out`.
    fn filter_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        group: &FloatGroup,
        fine: &FloatGroup,
        out: PixelRect,
        images: &mut Vec<wgpu::Texture>,
    ) -> Result<Option<(usize, PixelRect)>, String> {
        let mut region: Option<PixelRect> = None;
        let mut reach = [0.0f64; 2];
        let mut rate = 1;
        for f in &group.filters {
            let r = f.filter.rect();
            let user = Rect::new(
                f64::from(r.left()),
                f64::from(r.top()),
                f64::from(r.right()),
                f64::from(r.bottom()),
            );
            let device_rect = PixelRect::round_out(f.transform.transform_rect_bbox(user));
            region = Some(region.map_or(device_rect, |a| a.union(device_rect)));
            let h = svg_filter::halo(f)?;
            reach = [reach[0] + h[0], reach[1] + h[1]];
            rate = rate.max(svg_filter::grid_rate(f));
        }
        let Some(need) = region.and_then(|r| r.intersect(out)) else {
            return Ok(None);
        };
        // The source: the group's children wherever the output can read
        // them, their own filtered groups drawn first.
        let reach = reach.map(|v| v.ceil() as u32 + 1);
        let area = svg_filter::grow(need, reach);
        let mut children = group.children.clone();
        let mut fine_children = fine.children.clone();
        self.substitute(
            device,
            queue,
            &mut children,
            &mut fine_children,
            area,
            images,
        )?;
        let source = FilterSource {
            bounds: scene_bounds(&children, area),
            children: &children,
            fine: &fine_children,
            rate,
        };
        let texture = self.filter_tiles(device, queue, &group.filters, &source, need, images)?;
        images.push(texture);
        Ok(Some((images.len() - 1, need)))
    }

    /// The filters of a group over `need` (device pixels), split into tiles
    /// while their planned images exceed the filter budget.
    fn filter_tiles(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        filters: &[pm_vector::render::svg::FloatFilter],
        source: &FilterSource<'_>,
        need: PixelRect,
        images: &[wgpu::Texture],
    ) -> Result<wgpu::Texture, String> {
        let rate = source.rate;
        let grid = |r: PixelRect| {
            PixelRect::new(
                r.x * rate as i32,
                r.y * rate as i32,
                r.width * rate,
                r.height * rate,
            )
        };
        let transforms: Vec<Affine> = filters
            .iter()
            .map(|f| Affine::scale(f64::from(rate)) * f.transform)
            .collect();
        let plans = |need: PixelRect| -> Vec<svg_filter::Plan> {
            // Forward: where each filter's output can have content.
            let everywhere = PixelRect::new(-(1 << 28), -(1 << 28), 1 << 29, 1 << 29);
            let mut supports = vec![source.bounds.map(grid)];
            for (f, t) in filters.iter().zip(&transforms) {
                let last = *supports.last().expect("the source");
                supports.push(svg_filter::Plan::new(f, *t, last, everywhere).output);
            }
            // Backward: where each is read.
            let mut plans = Vec::with_capacity(filters.len());
            let mut want = Some(grid(need));
            for (k, (f, t)) in filters.iter().zip(&transforms).enumerate().rev() {
                let plan = match want {
                    Some(w) => svg_filter::Plan::new(f, *t, supports[k], w),
                    None => svg_filter::Plan::new(f, *t, None, grid(need)),
                };
                want = plan.source;
                plans.push(plan);
            }
            plans.reverse();
            plans
        };
        let bytes = |plans: &[svg_filter::Plan]| plans.iter().map(|p| p.bytes).sum::<u64>();
        let planned = plans(need);
        if bytes(&planned) > MAX_FILTER_BYTES {
            let center = PixelRect::new(
                need.x + (need.width / 2) as i32,
                need.y + (need.height / 2) as i32,
                1,
                1,
            );
            if need.width <= 1 && need.height <= 1 || bytes(&plans(center)) > MAX_FILTER_BYTES {
                return Err(format!(
                    "SVG filter exceeds the {} MiB filter budget at every tile size",
                    MAX_FILTER_BYTES >> 20
                ));
            }
            let (a, b) = if need.width >= need.height {
                let half = need.width / 2;
                (
                    PixelRect::new(need.x, need.y, half, need.height),
                    PixelRect::new(need.x + half as i32, need.y, need.width - half, need.height),
                )
            } else {
                let half = need.height / 2;
                (
                    PixelRect::new(need.x, need.y, need.width, half),
                    PixelRect::new(need.x, need.y + half as i32, need.width, need.height - half),
                )
            };
            let output = float_target(device, need.width, need.height);
            for part in [a, b] {
                let tile = self.filter_tiles(device, queue, filters, source, part, images)?;
                let mut encoder = device.create_command_encoder(&Default::default());
                copy_rect(&mut encoder, &tile, [0, 0], &output, part, need);
                queue.submit([encoder.finish()]);
            }
            return Ok(output);
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pool = ScratchPool::default();
        // The source over the first filter's planned rectangle, on its grid.
        let mut current = match planned.first().and_then(|p| p.source) {
            None => None,
            // A 4× grid paints the 4× twin there, unless its clips or masks
            // need a finer grid still: then the device paint is repeated.
            Some(rect) if rate == GROUP_RATE && !needs_joint_samples(source.fine) => {
                let texture = self.paint_scene(
                    device,
                    queue,
                    &mut encoder,
                    source.fine,
                    &|| Err("SVG filter grid scene needs no finer grid".to_owned()),
                    rect,
                    scene_bounds(source.fine, rect),
                    images,
                )?;
                Some((svg_filter::Image::source(texture, rect), false))
            }
            Some(rect) => {
                let device_rect = PixelRect::round_out(Rect::new(
                    f64::from(rect.x) / f64::from(rate),
                    f64::from(rect.y) / f64::from(rate),
                    f64::from(rect.right()) / f64::from(rate),
                    f64::from(rect.bottom()) / f64::from(rate),
                ));
                let fine = source.fine.to_vec();
                let texture = self.paint_scene(
                    device,
                    queue,
                    &mut encoder,
                    source.children,
                    &|| Ok(fine.clone()),
                    device_rect,
                    scene_bounds(source.children, device_rect),
                    images,
                )?;
                let painted = svg_filter::Image::source(texture, device_rect);
                if rate > 1 {
                    let fine =
                        self.filters
                            .replicate(device, &mut encoder, &painted, rate, &mut pool);
                    Some((fine, true))
                } else {
                    Some((painted, false))
                }
            }
        };
        for ((f, t), plan) in filters.iter().zip(&transforms).zip(&planned) {
            let Some((input, pooled)) = current.take() else {
                break;
            };
            let output = self
                .filters
                .run(device, &mut encoder, f, plan, *t, &input, &mut pool)?;
            if pooled {
                input.release(&mut pool);
            }
            current = output.texture.is_some().then_some((output, true));
        }
        // The result over `need` at device resolution, transparent where
        // nothing was drawn.
        let output = float_target(device, need.width, need.height);
        if let Some((image, _)) = &current {
            let texture = image.texture.as_ref().expect("a drawn image");
            if rate == 1 {
                copy_rect(&mut encoder, texture, [0, 0], &output, image.rect, need);
            } else {
                let fine_need = grid(need);
                let fine = float_target(device, fine_need.width, fine_need.height);
                copy_rect(&mut encoder, texture, [0, 0], &fine, image.rect, fine_need);
                let view = fine.create_view(&wgpu::TextureViewDescriptor::default());
                let ub = uniform(
                    device,
                    &[
                        0.0,
                        0.0,
                        need.width as f32,
                        need.height as f32,
                        rate as f32,
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
                render_pass(&mut encoder, &output, true, &self.resolve, &bg);
            }
        }
        queue.submit([encoder.finish()]);
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_joint_tiled(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fine_scene: &[FloatNode],
        rect: PixelRect,
        visible: Option<PixelRect>,
        images: &[wgpu::Texture],
    ) -> Result<wgpu::Texture, String> {
        if rect.width.max(1) > device.limits().max_texture_dimension_2d
            || rect.height.max(1) > device.limits().max_texture_dimension_2d
        {
            return Err("SVG output exceeds device texture dimensions".into());
        }
        let output = float_target(device, rect.width.max(1), rect.height.max(1));
        let mut tile_encoder = device.create_command_encoder(&Default::default());
        clear_target(&mut tile_encoder, &output);
        let mut submissions = TileSubmissions::default();
        let Some(visible) = visible else {
            submissions.submit(device, queue, tile_encoder)?;
            return Ok(output);
        };
        let halo = JOINT_HALO;
        let targets = target_count(fine_scene);
        // The fine scene is resolved once; every tile culls it to its own
        // fine rectangle. Rebuilding outlines per tile multiplied the
        // polygon work by the tile count.
        let mut scratch = ScratchPool::default();
        let tile_size = joint_tile_size(halo, live_targets(fine_scene));
        let tiles_per_submit = 4;
        let mut pending = 0;
        let start_x = (visible.x - rect.x) as u32;
        let start_y = (visible.y - rect.y) as u32;
        let end_x = (visible.right() - rect.x) as u32;
        let end_y = (visible.bottom() - rect.y) as u32;
        let tile_w = tile_size.min(end_x - start_x);
        let tile_h = tile_size.min(end_y - start_y);
        let tiles_x = u64::from((end_x - start_x).div_ceil(tile_w));
        let tiles_y = u64::from((end_y - start_y).div_ceil(tile_h));
        let fine_w =
            u64::from(tile_w.saturating_add(halo[0].saturating_mul(2))) * u64::from(GROUP_RATE);
        let fine_h =
            u64::from(tile_h.saturating_add(halo[1].saturating_mul(2))) * u64::from(GROUP_RATE);
        let work = tiles_x
            .saturating_mul(tiles_y)
            .saturating_mul(fine_w)
            .saturating_mul(fine_h)
            .saturating_mul(targets);
        if work > MAX_TILED_PIXEL_PASSES {
            return Err(format!(
                "SVG tiled paint budget exceeded ({work} fine pixel passes)"
            ));
        }
        for y in (start_y..end_y).step_by(tile_h as usize) {
            for x in (start_x..end_x).step_by(tile_w as usize) {
                let tile = PixelRect::new(
                    rect.x
                        .checked_add(i32::try_from(x).map_err(|_| "SVG tile x overflow")?)
                        .ok_or("SVG tile x overflow")?,
                    rect.y
                        .checked_add(i32::try_from(y).map_err(|_| "SVG tile y overflow")?)
                        .ok_or("SVG tile y overflow")?,
                    tile_w.min(end_x - x),
                    tile_h.min(end_y - y),
                );
                self.render_joint_tile(
                    device,
                    &mut tile_encoder,
                    fine_scene,
                    rect,
                    tile,
                    halo,
                    &output,
                    &mut scratch,
                    images,
                )?;
                pending += 1;
                if pending == tiles_per_submit {
                    submissions.submit(device, queue, tile_encoder)?;
                    tile_encoder = device.create_command_encoder(&Default::default());
                    pending = 0;
                }
            }
        }
        if pending > 0 {
            submissions.submit(device, queue, tile_encoder)?;
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_joint_tile(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &[FloatNode],
        rect: PixelRect,
        tile: PixelRect,
        halo: [u32; 2],
        output: &wgpu::Texture,
        scratch: &mut ScratchPool,
        images: &[wgpu::Texture],
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
        let mut paths = Vec::new();
        collect(scene, fine_rect, &mut paths)
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
            projection.paint_nodes(scene, fine_size)
        };
        let resolved = projection.target((tile.width, tile.height));
        projection.release(painted);
        projection.release(resolved);
        if projection.bytes > MAX_FINE_BYTES {
            // Prior edge/split tiles can leave incompatible free allocations.
            // Replan without them before treating the current tile as too big.
            scratch.discard_unused();
            projection = scratch.projection((!paths.is_empty()).then_some(coverage_bytes));
            let painted = projection.paint_nodes(scene, fine_size);
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
                device, encoder, scene, rect, a, halo, output, scratch, images,
            )?;
            self.render_joint_tile(
                device, encoder, scene, rect, b, halo, output, scratch, images,
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
            let (stops, ranges) = collect_stops(scene);
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
                scene,
                fine_rect,
                &paths,
                &values,
                &stop_buffer,
                &ranges,
                &mut 0,
                &mut 0,
                scratch,
                images,
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
        images: &[wgpu::Texture],
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
                    let image = match draw.paint {
                        FloatPaint::Image { index, .. } => {
                            images.get(index).ok_or("SVG filter result is missing")?
                        }
                        _ => &self.blank,
                    };
                    let image = image.create_view(&wgpu::TextureViewDescriptor::default());
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
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&image),
                            },
                        ],
                    );
                    render_pass(encoder, &target, false, &self.draw, &bg);
                }
                FloatNode::Group(group) => {
                    debug_assert!(
                        group.filters.is_empty(),
                        "filters are drawn before painting"
                    );
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
                        scratch,
                        images,
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
                            scratch,
                            images,
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
                            images,
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
}

/// A filtered group's source: its children (filtered descendants already
/// drawn), their 4× twin, where they draw, and the filter grid's rate.
struct FilterSource<'a> {
    children: &'a [FloatNode],
    fine: &'a [FloatNode],
    bounds: Option<PixelRect>,
    rate: u32,
}

/// The device rectangle a scene draws in over `rect`.
fn scene_bounds(scene: &[FloatNode], rect: PixelRect) -> Option<PixelRect> {
    let mut paths = Vec::new();
    collect(scene, rect, &mut paths)?;
    path_bounds(&paths).map(|b| PixelRect::new(b.x + rect.x, b.y + rect.y, b.width, b.height))
}

/// Copies `rect` (device pixels) from `src`, whose texel `from` is the
/// rectangle's corner, into `dst`, which covers `dst_area`.
fn copy_rect(
    encoder: &mut wgpu::CommandEncoder,
    src: &wgpu::Texture,
    from: [i32; 2],
    dst: &wgpu::Texture,
    rect: PixelRect,
    dst_area: PixelRect,
) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: from[0] as u32,
                y: from[1] as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: (rect.x - dst_area.x) as u32,
                y: (rect.y - dst_area.y) as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: rect.width,
            height: rect.height,
            depth_or_array_layers: 1,
        },
    );
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
                1 + target_count(&g.children)
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

/// The most RGBA32F fine targets one tile holds at once (the scratch
/// projection of its paint order).
fn live_targets(scene: &[FloatNode]) -> u64 {
    let mut projection = ScratchProjection {
        slots: Vec::new(),
        busy: Vec::new(),
        bytes: 0,
    };
    projection.paint_nodes(scene, (1, 1));
    projection.slots.len() as u64
}

/// The largest square tile (32 … 1024 output pixels) whose live fine
/// targets, halo included, use at most three quarters of the scratch
/// budget; the rest is coverage. Larger tiles keep the halo overlap, and
/// with it the filter work, small; a tile that still overflows splits.
fn joint_tile_size(halo: [u32; 2], live_targets: u64) -> u32 {
    let per_target = MAX_FINE_BYTES / 4 * 3 / live_targets.max(1) / 16;
    let fine_side = per_target.isqrt() / u64::from(GROUP_RATE);
    let side = fine_side.saturating_sub(u64::from(2 * halo[0].max(halo[1])));
    side.clamp(32, 1024) as u32
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
            FloatNode::Draw(d) if matches!(d.paint, FloatPaint::Image { .. }) => {
                // A raster needs no coverage: its outline is its pixel
                // rectangle on this grid.
                let b = d.outline.bounding_box();
                let local = PixelRect::new(
                    b.x0 as i32 - rect.x,
                    b.y0 as i32 - rect.y,
                    b.width() as u32,
                    b.height() as u32,
                )
                .intersect(PixelRect::new(
                    0,
                    0,
                    rect.width.max(1),
                    rect.height.max(1),
                ));
                out.push(DrawEntry {
                    path: BezPath::new(),
                    bounds: local.unwrap_or(PixelRect::new(0, 0, 0, 0)),
                    coverage_bounds: PixelRect::new(0, 0, 0, 0),
                    samples: 1,
                    offset: 0,
                });
            }
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
        FloatPaint::Image {
            origin,
            rate,
            opacity,
            ..
        } => {
            v[4..8].copy_from_slice(&[origin[0] as f32, origin[1] as f32, *rate as f32, *opacity]);
            v[19] = 3.0;
        }
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
pub(super) fn blend_code(mode: pm_vector::render::svg::BlendMode) -> u32 {
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
