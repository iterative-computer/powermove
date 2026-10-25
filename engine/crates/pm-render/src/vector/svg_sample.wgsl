struct Params {
    x: vec4<f32>,
    y: vec4<f32>,
    size: vec4<f32>,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(0) @binding(2) var linear: sampler;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 2.0 - 1.0, 1.0 - f32(i >> 1u) * 2.0, 0.0, 1.0);
}

// The output pixel's square, mapped into the source raster, integrated by
// a stratified 4 × 4 quadrature of the bilinear reconstruction.
fn integrate(position: vec4<f32>) -> vec4<f32> {
    let pixel = floor(position.xy);
    let origin = vec2<f32>(dot(p.x.xyz, vec3<f32>(pixel, 1.0)), dot(p.y.xyz, vec3<f32>(pixel, 1.0)));
    let ex = vec2<f32>(p.x.x, p.y.x);
    let ey = vec2<f32>(p.x.y, p.y.y);
    let inv_size = 1.0 / p.size.xy;
    var color = vec4<f32>(0.0);
    for (var j = 0u; j < 4u; j++) {
        for (var i = 0u; i < 4u; i++) {
            let q = origin + ex * ((f32(i) + 0.5) * 0.25) + ey * ((f32(j) + 0.5) * 0.25);
            color += textureSampleLevel(source, linear, q * inv_size, 0.0);
        }
    }
    return color * (1.0 / 16.0);
}
@fragment fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return integrate(position);
}
// RGBA16F stores the nearest half (`crate::nearest_half`).
@fragment fn fs_half(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return pm_scene_half_color(integrate(position));
}
