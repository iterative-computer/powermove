// The depth and object-id surface (contract 2, `DepthIdSurface`; plan
// section 8). `rgba16float`, top-down, the frame's size:
//
//   r: view-space depth of the visible 3D surface in world units (positive
//      in front of the camera), +inf where the top layer is 2D or nothing;
//   g: (id − 1) & 0x7ff, b: (id − 1) >> 11, where id indexes the frame's
//      legend (the top-level layer ids, 1-based); exact in f16;
//   a: 1 where a layer covers the pixel, 0 elsewhere.
//
// A layer covers a pixel where its own drawn alpha is at least 0.5/255
// (visible at 8 bits). Draws run bottom to top without blending, so the
// topmost covering layer wins.

struct Quad {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    res: vec4<f32>,
    uv: vec4<f32>,
    // x: alpha (texture) or coverage alpha (solid); y: id − 1 low bits;
    // z: id − 1 high bits; w: 1 when the texture is in GL row order.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> q: Quad;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;
@group(2) @binding(0) var depth: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const COVERED: f32 = 0.5 / 255.0;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var o: VsOut;
    o.uv = q.uv.xy + a * q.uv.zw;
    // GL row order, as the color passes.
    o.pos = vec4<f32>((p.x / q.res.x) * 2.0 - p.z, (p.y / q.res.y) * 2.0 - p.z, 0.0, p.z);
    return o;
}

fn inf() -> f32 {
    return bitcast<f32>(0x7f800000u);
}

@fragment
fn fs_id_solid(in: VsOut) -> @location(0) vec4<f32> {
    if (q.color.x < COVERED) {
        discard;
    }
    return vec4<f32>(inf(), q.color.y, q.color.z, 1.0);
}

@fragment
fn fs_id_draw(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (q.color.w > 0.5) {
        uv = vec2<f32>(uv.x, 1.0 - uv.y);
    }
    var a = textureSampleLevel(tex, smp, uv, 0.0).a * q.color.x;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        a = 0.0;
    }
    if (a < COVERED) {
        discard;
    }
    return vec4<f32>(inf(), q.color.y, q.color.z, 1.0);
}

// A 3D layer: its own alpha decides coverage, its depth texture (linear
// view depth, +inf where it drew nothing) gives r.
@fragment
fn fs_id_3d(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let a = textureSampleLevel(tex, smp, uv, 0.0).a * q.color.x;
    let dims = vec2<f32>(textureDimensions(depth));
    let texel = vec2<i32>(clamp(uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let d = textureLoad(depth, texel, 0).r;
    if (a < COVERED) {
        discard;
    }
    return vec4<f32>(d, q.color.y, q.color.z, 1.0);
}
