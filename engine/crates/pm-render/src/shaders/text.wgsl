// The float paint pass of text rasters (docs/engine/text-render.md).
//
// Vello renders 8-bit coverage (white, area coverage) into pages laid out
// 1:1 with device pixels; this pass paints each canvas draw in order with
// premultiplied source-over (fixed-function ONE, ONE_MINUS_SRC_ALPHA) at
// working precision:
//
// - kind 0, coverage: `color × curve(coverage)`, the coverage curve
//   `1 − (1 − c)^gamma` of the stem-darkening profile;
// - kind 1, image: a color bitmap glyph, bilinear (the image carries a
//   one-pixel transparent border, so edges fade as the CPU reference
//   samples them), times the draw's alpha;
// - kind 2, layer: a pre-painted layer (a blurred draw) copied 1:1.
//
// Targets are top-down (row 0 at the top), like images.

struct Item {
    // The quad in target pixels: x0, y0, x1, y1.
    rect: vec4<f32>,
    // Page (or layer) texel = floor(fragment) + map.xy.
    map: vec4<f32>,
    // Coverage: premultiplied paint. Image: alpha multiplier in w.
    color: vec4<f32>,
    // x: kind, y: gamma, zw: target size.
    params: vec4<f32>,
    // Image: target pixel to padded image pixel, rows (a, c, e) and (b, d, f);
    // inv1.w unused, inv0.w unused; size in `isize`.
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    isize: vec4<f32>,
};

@group(0) @binding(0) var<uniform> it: Item;
@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let p = mix(it.rect.xy, it.rect.zw, a);
    return vec4<f32>(p.x / it.params.z * 2.0 - 1.0, 1.0 - p.y / it.params.w * 2.0, 0.0, 1.0);
}

@fragment
fn fs_paint(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let kind = it.params.x;
    if (kind < 0.5) {
        let t = vec2<i32>(floor(pos.xy) + it.map.xy);
        let c = textureLoad(src, t, 0).a;
        var k = c;
        if (it.params.y != 1.0) {
            k = 1.0 - pow(1.0 - c, it.params.y);
        }
        return it.color * k;
    }
    if (kind < 1.5) {
        let p = vec3<f32>(pos.xy, 1.0);
        let u = vec2<f32>(dot(it.inv0.xyz, p), dot(it.inv1.xyz, p));
        let s = textureSampleLevel(src, smp, u / it.isize.xy, 0.0);
        return s * it.color.w;
    }
    let t = vec2<i32>(floor(pos.xy) + it.map.xy);
    return textureLoad(src, t, 0);
}

// Separable Gaussian (CSS `blur(σ)`, radius 3σ): one direction per pass,
// zero outside the source.
struct Blur {
    // xy: direction (1, 0) or (0, 1); z: sigma; w: radius.
    dir: vec4<f32>,
    // xy: source size; zw: target size.
    size: vec4<f32>,
    // x: 1 / sum of the weights.
    norm: vec4<f32>,
};

@group(0) @binding(1) var<uniform> bl: Blur;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    return vec4<f32>(a.x * 2.0 - 1.0, 1.0 - a.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let r = i32(bl.dir.w);
    let d = vec2<i32>(bl.dir.xy);
    let n = vec2<i32>(bl.size.xy);
    let s2 = 2.0 * bl.dir.z * bl.dir.z;
    var acc = vec4<f32>(0.0);
    for (var i = -r; i <= r; i = i + 1) {
        let q = p + d * i;
        if (q.x >= 0 && q.y >= 0 && q.x < n.x && q.y < n.y) {
            let w = exp(-f32(i * i) / s2);
            acc = acc + w * textureLoad(src, q, 0);
        }
    }
    return acc * bl.norm.x;
}
