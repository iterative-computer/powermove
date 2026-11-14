// GPU scopes (plan section 1, Colour): one compute pass bins every pixel of a
// finished frame into storage counters. `pm_color::scopes` is the f64 reference.
//
// Counters, in order: waveform, red, green and blue parade (levels × columns
// each), vectorscope (bins²), luma, red, green and blue histograms, then the
// pixels under the range, over it, outside the vectorscope, and not finite.

struct Params {
    // columns, levels, vector bins, signal (0 display code values, 1 BT.2100 PQ).
    a: vec4<u32>,
    // Luma weights.
    weights: vec4<f32>,
    to_2020_0: vec4<f32>,
    to_2020_1: vec4<f32>,
    to_2020_2: vec4<f32>,
    // Output stage (x), its tone's source and target peaks (y, z), tone (w).
    output: vec4<f32>,
};

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var<uniform> P: Params;
@group(0) @binding(2) var<storage, read_write> counters: array<atomic<u32>>;

fn level(v: f32, n: u32) -> u32 {
    return u32(floor(clamp(v, 0.0, 1.0) * f32(n - 1u) + 0.5));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(frame);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let columns = P.a.x;
    let levels = P.a.y;
    let bins = P.a.z;
    var rgb = textureLoad(frame, vec2<i32>(id.xy), 0).rgb;
    let plane = levels * columns;
    let vector_start = plane * 4u;
    let histogram_start = vector_start + bins * bins;
    let totals = histogram_start + levels * 4u;
    // NaN fails its own comparison, infinity the bound.
    if (!all(abs(rgb) <= vec3<f32>(1e30))) {
        atomicAdd(&counters[totals + 3u], 1u);
        return;
    }
    // Output: the project's output tone, as the export applies it.
    if (P.output.x > 0.5 && P.output.w > 0.5) {
        rgb = pm_tone_map_output(vec4<f32>(rgb, 1.0), P.output.y, P.output.z).rgb;
    }
    if (P.a.w == 1u) {
        let to_2020 = mat3x3<f32>(P.to_2020_0.xyz, P.to_2020_1.xyz, P.to_2020_2.xyz);
        rgb = pm_pq_inverse_eotf(to_2020 * pm_e_decode3(rgb) * 203.0);
    }
    // Output: what a file of the signal holds.
    if (P.output.x > 0.5) {
        rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (any(rgb < vec3<f32>(0.0))) {
        atomicAdd(&counters[totals], 1u);
    }
    if (any(rgb > vec3<f32>(1.0))) {
        atomicAdd(&counters[totals + 1u], 1u);
    }
    let weights = P.weights.xyz;
    let luma = dot(rgb, weights);
    let cb = (rgb.b - luma) / (2.0 * (1.0 - weights.b));
    let cr = (rgb.r - luma) / (2.0 * (1.0 - weights.r));
    let x = id.x * columns / dims.x;
    if (abs(cb) > 0.5 || abs(cr) > 0.5) {
        atomicAdd(&counters[totals + 2u], 1u);
    }
    atomicAdd(&counters[level(luma, levels) * columns + x], 1u);
    atomicAdd(&counters[histogram_start + level(luma, levels)], 1u);
    for (var c = 0u; c < 3u; c++) {
        let at = level(rgb[c], levels);
        atomicAdd(&counters[plane * (c + 1u) + at * columns + x], 1u);
        atomicAdd(&counters[histogram_start + levels * (c + 1u) + at], 1u);
    }
    atomicAdd(&counters[vector_start + level(cr + 0.5, bins) * bins + level(cb + 0.5, bins)], 1u);
}
