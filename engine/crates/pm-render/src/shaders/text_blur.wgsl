// Separable pixel-integrated Gaussian in premultiplied working precision.
// Samples outside the source remain transparent; the kernel is not normalized.

struct Blur {
    width: u32,
    height: u32,
    radius: u32,
    axis: u32,
};

@group(0) @binding(0) var<uniform> blur: Blur;
@group(0) @binding(1) var blur_src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    return vec4<f32>(a.x * 2.0 - 1.0, 1.0 - a.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let dir = select(vec2<i32>(0, 1), vec2<i32>(1, 0), blur.axis == 0u);
    let radius = i32(blur.radius);
    var sum = vec4<f32>(0.0);
    for (var k = -radius; k <= radius; k++) {
        let q = p + dir * k;
        if (q.x >= 0 && q.y >= 0 && q.x < i32(blur.width) && q.y < i32(blur.height)) {
            sum += textureLoad(blur_src, q, 0) * weights[u32(k + radius)];
        }
    }
    return sum;
}
