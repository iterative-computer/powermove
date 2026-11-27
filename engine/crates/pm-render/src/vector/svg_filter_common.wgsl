// Shared by the SVG filter passes (`svg_filter.rs`): the primitive's
// parameters, its colour space, and exact region coverage.
//
// Every filter image holds, per grid pixel, the area integral of its
// premultiplied sRGB-encoded paint clipped to the image's region `Q` (a
// user-space rectangle; unbounded for the source). Coverage is exact for
// grids aligned with user space and a 4 × 4 point quadrature otherwise, so
// intersecting regions never multiplies a partial pixel twice.

struct Fe {
    // Grid pixel to filter user space: x = dot(inv_x.xy, g) + inv_x.z.
    // inv_x.w: 1 when the grid is aligned with user space.
    inv_x: vec4<f32>,
    inv_y: vec4<f32>,
    // The output image's origin in grid pixels (xy).
    origin: vec4<f32>,
    // The input images' origins (in1: xy, in2: zw).
    origins: vec4<f32>,
    // Regions (x0, y0, x1, y1, user space): the inputs' and the output's
    // (filter region ∩ primitive subregion).
    q1: vec4<f32>,
    q2: vec4<f32>,
    s: vec4<f32>,
    // Operator parameters.
    k: vec4<f32>,
    // x: operator, y: flags (1: in alpha only, 2: in2 alpha only,
    // 4: linearRGB), z: sub-operator or pass, w: kernel radius.
    mode: vec4<u32>,
    m: array<vec4<f32>, 8>,
};

fn fe_user(g: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(dot(fe.inv_x.xy, g) + fe.inv_x.z, dot(fe.inv_y.xy, g) + fe.inv_y.z);
}

fn fe_isect(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(max(a.xy, b.xy), min(a.zw, b.zw));
}

// The fraction of grid pixel `px` (its top-left corner) inside `r`.
fn fe_cover(r: vec4<f32>, px: vec2<f32>) -> f32 {
    if (fe.inv_x.w > 0.5) {
        let a = fe_user(px);
        let b = fe_user(px + vec2<f32>(1.0));
        let lo = min(a, b);
        let hi = max(a, b);
        let d = max(min(hi, r.zw) - max(lo, r.xy), vec2<f32>(0.0));
        return clamp(d.x * d.y / ((hi.x - lo.x) * (hi.y - lo.y)), 0.0, 1.0);
    }
    var n = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let u = fe_user(px + (vec2<f32>(f32(i), f32(j)) + vec2<f32>(0.5)) * 0.25);
            if (all(u >= r.xy) && all(u < r.zw)) { n += 1.0; }
        }
    }
    return n / 16.0;
}

fn fe_srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(c / 12.92, pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c > vec3<f32>(0.04045));
}

fn fe_linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(12.92 * c, 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055,
                  c > vec3<f32>(0.0031308));
}

fn fe_linear() -> bool {
    return (fe.mode.y & 4u) != 0u;
}

// Premultiplied colour into the primitive's space. Unpremultiplying makes
// it commute with coverage: c × (k·p) = k × c(p).
fn fe_in(p: vec4<f32>, alpha_only: bool) -> vec4<f32> {
    var c = p;
    if (alpha_only) { c = vec4<f32>(0.0, 0.0, 0.0, c.a); }
    if (fe_linear() && c.a > 0.0) {
        c = vec4<f32>(fe_srgb_to_linear(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}

// Back to sRGB-encoded storage.
fn fe_out(p: vec4<f32>) -> vec4<f32> {
    if (fe_linear() && p.a > 0.0) {
        return vec4<f32>(fe_linear_to_srgb(p.rgb / p.a) * p.a, p.a);
    }
    return p;
}
