// Today's bezier path mask coverage (`applyPathMasks`; docs/engine/
// composite.md "Path masks"): Canvas2D's `blur(σ px)` filter as Chromium
// 152's Skia Graphite runs it (`pm_vector::canvas2d::filter`), and the
// item drawn onto the coverage canvas with `source-over` or
// `destination-out`.
//
// Every image is single-channel 8-bit (the canvases are white: each
// premultiplied channel is the coverage) with a transparent 1-pixel border,
// sampled with clamp to edge, so samples outside an image are transparent
// as Skia's decal tiling and pixel-outset images make them. The SkSL blur
// effects declare `half` sums, weights, and offsets, which Graphite runs at
// full precision on Metal: the passes sum in f32 (bit-exact with today's
// canvases; docs/engine/composite.md).

struct Pass {
    // Destination space coordinate of the target's texel (0, 0) corner.
    dst_org: vec2<f32>,
    // Source space coordinate of the source texture's (0, 0) corner.
    src_org: vec2<f32>,
    // 1 / source texture size.
    src_inv: vec2<f32>,
    // Taps (1D: pairs of taps; 2D: groups of four).
    n: u32,
    linear: u32,
    // Destination to source space (rescale and final draw).
    map_s: vec2<f32>,
    map_t: vec2<f32>,
    dir: vec2<f32>,
    pad: vec2<f32>,
    // 2D: weights, four per entry.
    k: array<vec4<f32>, 7>,
    // 1D: (offset, weight, offset, weight); 2D: offsets (x, y, x, y).
    o: array<vec4<f32>, 14>,
};

@group(0) @binding(0) var<uniform> p: Pass;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var bilinear: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    return vec4<f32>(a.x * 2.0 - 1.0, 1.0 - a.y * 2.0, 0.0, 1.0);
}

// Bilinear at a source space position.
fn sample_at(s: vec2<f32>) -> f32 {
    return textureSampleLevel(src, bilinear, (s - p.src_org) * p.src_inv, 0.0).r;
}

// The texel containing a source space position.
fn texel_at(s: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(src));
    let t = clamp(vec2<i32>(floor(s - p.src_org)), vec2<i32>(0), size - 1);
    return textureLoad(src, t, 0).r;
}

fn dst_center(pos: vec4<f32>) -> vec2<f32> {
    return pos.xy + p.dst_org;
}

// A rescale step (`FilterResult::rescale`): bilinear from the previous
// image through the step's inverse scale.
@fragment
fn fs_rescale(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = dst_center(pos);
    let v = sample_at(c * p.map_s + p.map_t);
    return vec4<f32>(v);
}

// `make_blur_1D_shader`.
@fragment
fn fs_blur1d(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = dst_center(pos);
    let dir = vec2<f32>(p.dir);
    var sum = f32(0.0);
    for (var i = 0u; i < p.n; i++) {
        let s = vec4<f32>(p.o[i]);
        sum += s.y * f32(sample_at(coord + vec2<f32>(s.x * dir)));
        sum += s.w * f32(sample_at(coord + vec2<f32>(s.z * dir)));
    }
    return vec4<f32>(f32(sum));
}

// `make_blur_2D_shader` (nearest sampling).
@fragment
fn fs_blur2d(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = dst_center(pos);
    var sum = f32(0.0);
    for (var i = 0u; i < p.n; i++) {
        let k = vec4<f32>(p.k[i]);
        var o = vec4<f32>(p.o[2u * i]);
        sum += k.x * f32(texel_at(coord + vec2<f32>(o.xy)));
        sum += k.y * f32(texel_at(coord + vec2<f32>(o.zw)));
        o = vec4<f32>(p.o[2u * i + 1u]);
        sum += k.z * f32(texel_at(coord + vec2<f32>(o.xy)));
        sum += k.w * f32(texel_at(coord + vec2<f32>(o.zw)));
    }
    return vec4<f32>(f32(sum));
}

// The item (or its blur) drawn onto the coverage canvas; the pipeline's
// blend state is the composite operation.
@fragment
fn fs_draw(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let s = pos.xy * p.map_s + p.map_t;
    var v: f32;
    if (p.linear == 1u) {
        v = sample_at(s);
    } else {
        v = texel_at(s);
    }
    return vec4<f32>(v);
}
