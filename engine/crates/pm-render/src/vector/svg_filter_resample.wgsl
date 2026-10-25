struct Params { v: array<vec4<f32>, 6> };
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u)*2.0-1.0, 1.0-f32(i >> 1u)*2.0, 0.0, 1.0);
}
fn fetch(q: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    if (any(q < vec2<i32>(0)) || any(q >= size)) { return vec4<f32>(0.0); }
    return textureLoad(source, q, 0);
}
fn sample(q: vec2<f32>) -> vec4<f32> {
    let base = floor(q - vec2<f32>(0.5));
    let f = q - vec2<f32>(0.5) - base;
    let a = vec2<i32>(base);
    return mix(mix(fetch(a), fetch(a + vec2<i32>(1, 0)), f.x),
               mix(fetch(a + vec2<i32>(0, 1)), fetch(a + vec2<i32>(1, 1)), f.x), f.y);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let region = p.v[0];
    var q = vec2<f32>(0.0);
    if (p.v[5].x < 0.5) {
        if (any(pos.xy >= p.v[4].xy)) { return vec4<f32>(0.0); }
        let local = region.xy + pos.xy / p.v[4].xy * region.zw;
        let device = vec2<f32>(dot(p.v[1].xyz, vec3<f32>(local, 1.0)),
                               dot(p.v[2].xyz, vec3<f32>(local, 1.0)));
        q = device - p.v[3].xy;
    } else {
        let fine_device = pos.xy + p.v[3].xy;
        let rate = p.v[5].y;
        let device = floor(fine_device / rate) * rate + vec2<f32>(0.5 * rate);
        let local = vec2<f32>(dot(p.v[1].xyz, vec3<f32>(device, 1.0)),
                               dot(p.v[2].xyz, vec3<f32>(device, 1.0)));
        let uv = (local - region.xy) / region.zw;
        if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
            return vec4<f32>(0.0);
        }
        q = uv * p.v[4].xy;
    }
    return sample(q);
}
