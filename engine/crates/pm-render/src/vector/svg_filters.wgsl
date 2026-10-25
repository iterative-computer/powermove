struct Params { v: array<vec4<f32>, 8> };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(0) @binding(2) var<storage, read> weights: array<f32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u)*2.0-1.0, 1.0-f32(i >> 1u)*2.0, 0.0, 1.0);
}
fn local(q: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(dot(p.v[3].xy, q) + p.v[4].x,
                     dot(p.v[3].zw, q) + p.v[4].y);
}
fn in_rect(q: vec2<f32>, r: vec4<f32>) -> bool {
    return all(q >= r.xy) && all(q < r.zw);
}
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(c / 12.92, pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c > vec3<f32>(0.04045));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(12.92 * c, 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055,
                  c > vec3<f32>(0.0031308));
}
fn fetch(q: vec2<i32>) -> vec4<f32> {
    let dim = vec2<i32>(textureDimensions(source));
    if (any(q < vec2<i32>(0)) || any(q >= dim)) { return vec4<f32>(0.0); }
    var c = textureLoad(source, q, 0);
    if (p.v[5].z > 0.5) { c = vec4<f32>(0.0, 0.0, 0.0, c.a); }
    if (p.v[5].w > 0.5 && c.a > 0.0) { c = vec4<f32>(srgb_to_linear(c.rgb / c.a) * c.a, c.a); }
    return c;
}
fn sample(q: vec2<f32>) -> vec4<f32> {
    let base = floor(q - vec2<f32>(0.5));
    let f = q - vec2<f32>(0.5) - base;
    let a = vec2<i32>(base);
    return mix(mix(fetch(a), fetch(a + vec2<i32>(1, 0)), f.x),
               mix(fetch(a + vec2<i32>(0, 1)), fetch(a + vec2<i32>(1, 1)), f.x), f.y);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let global = pos.xy + p.v[4].zw;
    let uv = local(global);
    let mode = u32(p.v[0].w);
    if (mode != 0u && (!in_rect(uv, p.v[1]) || !in_rect(uv, p.v[2]))) {
        return vec4<f32>(0.0);
    }
    var result = vec4<f32>(0.0);
    if (mode == 2u) {
        // A source scale below 1 reconstructs a device-grid image at the
        // fine grid's pixel centers.
        let scale = select(1.0, p.v[6].y, p.v[6].y > 0.0);
        result = sample((pos.xy - p.v[5].xy) * scale);
    } else {
        let radius = i32(p.v[0].z);
        for (var k = -radius; k <= radius; k++) {
            result += sample(pos.xy + f32(k) * p.v[0].xy) * weights[u32(k + radius)];
        }
    }
    if (mode != 0u && p.v[6].x > 0.5 && result.a > 0.0) {
        result = vec4<f32>(linear_to_srgb(result.rgb / result.a) * result.a, result.a);
    }
    return result;
}
