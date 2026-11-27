// A lit field of instanced cubes for the orbit and walk latency tests.

struct Camera {
    view_proj: mat4x4f,
    light_dir: vec4f,
};
@group(0) @binding(0) var<uniform> cam: Camera;

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) normal: vec3f,
    @location(1) tint: vec3f,
};

const GRID: u32 = 24u;

fn cube_corner(i: u32) -> vec3f {
    // 36 vertices, 6 faces of 2 triangles; face f, corner k.
    let f = i / 6u;
    let k = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u)[i % 6u];
    let a = f32(k & 1u) * 2.0 - 1.0;
    let b = f32((k >> 1u) & 1u) * 2.0 - 1.0;
    switch f {
        case 0u: { return vec3f(1.0, a, -b); }
        case 1u: { return vec3f(-1.0, a, b); }
        case 2u: { return vec3f(a, 1.0, b); }
        case 3u: { return vec3f(a, -1.0, -b); }
        case 4u: { return vec3f(-a, b, 1.0); }
        default: { return vec3f(a, b, -1.0); }
    }
}

fn face_normal(f: u32) -> vec3f {
    switch f {
        case 0u: { return vec3f(1.0, 0.0, 0.0); }
        case 1u: { return vec3f(-1.0, 0.0, 0.0); }
        case 2u: { return vec3f(0.0, 1.0, 0.0); }
        case 3u: { return vec3f(0.0, -1.0, 0.0); }
        case 4u: { return vec3f(0.0, 0.0, 1.0); }
        default: { return vec3f(0.0, 0.0, -1.0); }
    }
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let gx = f32(ii % GRID) - f32(GRID) * 0.5;
    let gz = f32((ii / GRID) % GRID) - f32(GRID) * 0.5;
    let gy = f32(ii / (GRID * GRID));
    let h = 0.3 + 0.7 * fract(sin(f32(ii) * 12.9898) * 43758.5453);
    var p = cube_corner(vi) * vec3f(0.35, 0.35 * h, 0.35);
    p += vec3f(gx * 1.2, gy * 1.2 + 0.35 * h, gz * 1.2);
    var o: VOut;
    o.pos = cam.view_proj * vec4f(p, 1.0);
    o.normal = face_normal(vi / 6u);
    o.tint = vec3f(0.35 + 0.5 * fract(f32(ii) * 0.137), 0.45, 0.6 + 0.3 * fract(f32(ii) * 0.071));
    return o;
}

@fragment
fn fs(v: VOut) -> @location(0) vec4f {
    let d = max(dot(normalize(v.normal), normalize(-cam.light_dir.xyz)), 0.0);
    return vec4f(v.tint * (0.25 + 0.75 * d), 1.0);
}
