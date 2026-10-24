// The 2D compositing passes (plan section 8; legacy `legacy/gl/shaders.ts`).
//
// Prepended with `pm_color::wgsl::COLOR`.
//
// One quad per draw, as today's `PM.VERT`: the unit quad (0,0) (1,0) (0,1)
// (1,1) through a 3×3 matrix to target pixels, projective (w = p.z), with
// the content UV window `uv.xy + a · uv.zw`.
//
// Targets keep GL's row order, as today's framebuffers do (row 0 is the
// bottom of the picture): clip y is negated against `PM.VERT`, so the
// rasterizer sees the same edges in the same orientation as ANGLE's Metal
// back end does, and a pixel center on the shared edge of two quads goes
// to the same quad as today. Sources sampled from targets flip v
// (`u_fromFbo`); images and video frames are stored top row first. The
// renderer flips the finished frame once into a top-down target.
//
// Blending is the fixed-function premultiplied source-over (ONE,
// ONE_MINUS_SRC_ALPHA) set on the pipeline, as today's `blendFunc`.

struct Quad {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // xy: the full frame's size; zw: the target's origin in it when it is a
    // region of interest (GL rows), else 0.
    res: vec4<f32>,
    // Content UV window: offset (xy), scale (zw).
    uv: vec4<f32>,
    // Solid: premultiplied color. Texture: x = alpha, y = 1 when the source
    // is in GL row order, z = 1 when it holds premultiplied E values to be
    // read into linear light.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> q: Quad;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var o: VsOut;
    o.uv = q.uv.xy + a * q.uv.zw;
    o.pos = vec4<f32>((p.x / q.res.x) * 2.0 - p.z, (p.y / q.res.y) * 2.0 - p.z, 0.0, p.z);
    return o;
}

// `FRAG_DRAW`: no mipmaps, bilinear, hard edges outside the UV window;
// `color.y` = 1 for sources in GL row order (`u_fromFbo`).
@fragment
fn fs_draw(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (q.color.y > 0.5) {
        uv = vec2<f32>(uv.x, 1.0 - uv.y);
    }
    var c = textureSampleLevel(tex, smp, uv, 0.0);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        c = vec4<f32>(0.0);
    }
    if (q.color.z > 0.5) {
        c = pm_premultiplied_e_to_linear(c);
    }
    return c * q.color.x;
}

// Solid layers: the premultiplied color times the exact area coverage of
// each pixel by the layer's box, `q.uv.xy` layer units with `q.uv.z`
// circular corners (0 for square ones).
//
// The quad grows by 1.5 device pixels on every side (the fringe a pixel can
// partly cover). Each fragment maps its pixel square, from its full-frame
// position, into layer units through the inverse of the draw's matrix (a
// parallelogram: exact for affine placement, first order on a 2.5D plane),
// clips it against the box, and subtracts, at each rounded corner it
// reaches, the part of the corner square outside the circle (polygon ∩
// disk, exact). Pixels well inside or outside take the early answers 1 and
// 0. Only uniforms and the pixel's position enter the arithmetic, so a
// region of interest draws the full frame's values.

@vertex
fn vs_solid(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var e = a;
    if (p.z > 1e-6) {
        // Device pixels per unit of the unit quad along x and y here.
        let dx = (m[0].xy * p.z - p.xy * m[0].z) / (p.z * p.z);
        let dy = (m[1].xy * p.z - p.xy * m[1].z) / (p.z * p.z);
        let grow = min(vec2<f32>(1.5) / max(vec2<f32>(length(dx), length(dy)), vec2<f32>(1e-4)), vec2<f32>(1e4));
        let grown = a + (a * 2.0 - 1.0) * grow;
        if ((m * vec3<f32>(grown, 1.0)).z > 1e-6) {
            e = grown;
        }
    }
    let pe = m * vec3<f32>(e, 1.0);
    var o: VsOut;
    o.pos = vec4<f32>((pe.x / q.res.x) * 2.0 - pe.z, (pe.y / q.res.y) * 2.0 - pe.z, 0.0, pe.z);
    o.uv = e;
    return o;
}

struct Poly {
    v: array<vec2<f32>, 12>,
    n: u32,
};

// The part of `p` where `s · (x[axis] − c) ≥ 0`.
fn clip_axis(p: Poly, axis: u32, c: f32, s: f32) -> Poly {
    var o: Poly;
    o.n = 0u;
    for (var i = 0u; i < p.n; i++) {
        let a = p.v[i];
        let b = p.v[(i + 1u) % p.n];
        let da = s * (a[axis] - c);
        let db = s * (b[axis] - c);
        if (da >= 0.0) {
            o.v[o.n] = a;
            o.n++;
        }
        if ((da >= 0.0) != (db >= 0.0)) {
            o.v[o.n] = mix(a, b, da / (da - db));
            o.n++;
        }
    }
    return o;
}

// `p` within the axis-aligned box `lo..hi`.
fn clip_box(p: Poly, lo: vec2<f32>, hi: vec2<f32>) -> Poly {
    return clip_axis(clip_axis(clip_axis(clip_axis(p, 0u, lo.x, 1.0), 0u, hi.x, -1.0), 1u, lo.y, 1.0), 1u, hi.y, -1.0);
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// Signed area (shoelace).
fn poly_area(p: Poly) -> f32 {
    var s = 0.0;
    for (var i = 0u; i < p.n; i++) {
        s += cross2(p.v[i], p.v[(i + 1u) % p.n]);
    }
    return 0.5 * s;
}

// Signed area of the sector of radius `r` between directions `a` and `b`.
fn sector(a: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let c = cross2(a, b);
    let d = dot(a, b);
    if (abs(c) + abs(d) == 0.0) {
        return 0.0;
    }
    return 0.5 * r * r * atan2(c, d);
}

// Signed area of the triangle (center, a, b) inside the circle of radius
// `r` about the center (points relative to it).
fn disk_triangle(a: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let d = b - a;
    let aa = dot(d, d);
    if (aa == 0.0) {
        return 0.0;
    }
    let bb = dot(a, d);
    let disc = bb * bb - aa * (dot(a, a) - r * r);
    var t0 = 0.0;
    var t1 = 0.0;
    if (disc > 0.0) {
        let s = sqrt(disc);
        t0 = clamp((-bb - s) / aa, 0.0, 1.0);
        t1 = clamp((-bb + s) / aa, 0.0, 1.0);
    }
    let p0 = a + d * t0;
    let p1 = a + d * t1;
    return sector(a, p0, r) + 0.5 * cross2(p0, p1) + sector(p1, b, r);
}

// Signed area of `p` inside the circle of radius `r` about `center`. A
// polygon wholly inside or outside the circle takes its area or 0 exactly:
// the fan from a distant center would cancel large terms.
fn disk_area(p: Poly, center: vec2<f32>, r: f32) -> f32 {
    var far = 0.0;
    var near = 1e30;
    var s = 0.0;
    for (var i = 0u; i < p.n; i++) {
        let a = p.v[i] - center;
        let b = p.v[(i + 1u) % p.n] - center;
        far = max(far, length(a));
        let d = b - a;
        let t = clamp(-dot(a, d) / max(dot(d, d), 1e-30), 0.0, 1.0);
        near = min(near, length(a + d * t));
        s += cross2(a, b);
    }
    if (far <= r) {
        return 0.5 * s;
    }
    // The center outside the polygon and every edge beyond the radius.
    if (near >= r && !contains(p, center)) {
        return 0.0;
    }
    var area = 0.0;
    for (var i = 0u; i < p.n; i++) {
        area += disk_triangle(p.v[i] - center, p.v[(i + 1u) % p.n] - center, r);
    }
    return area;
}

// Whether the convex polygon `p` contains `q`.
fn contains(p: Poly, q: vec2<f32>) -> bool {
    var pos = false;
    var neg = false;
    for (var i = 0u; i < p.n; i++) {
        let c = cross2(p.v[(i + 1u) % p.n] - p.v[i], q - p.v[i]);
        pos = pos || c > 0.0;
        neg = neg || c < 0.0;
    }
    return !(pos && neg);
}

// The rounded box's area inside the pixel `pixel` (coordinates relative to
// the fragment), signed like `pixel`: its area in the box, less each
// reached corner square's part outside that corner's circle.
fn rounded_area(pixel: Poly, lo: vec2<f32>, hi: vec2<f32>, r: f32, reach_lo: vec2<f32>, reach_hi: vec2<f32>) -> f32 {
    let inside = clip_box(pixel, lo, hi);
    var area = poly_area(inside);
    if (r <= 0.0) {
        return area;
    }
    for (var k = 0u; k < 4u; k++) {
        let right = (k & 1u) == 1u;
        let bottom = k >= 2u;
        let sq_lo = vec2<f32>(select(lo.x, hi.x - r, right), select(lo.y, hi.y - r, bottom));
        let sq_hi = sq_lo + vec2<f32>(r);
        if (any(reach_hi <= sq_lo) || any(reach_lo >= sq_hi)) {
            continue;
        }
        let corner = clip_box(inside, sq_lo, sq_hi);
        if (corner.n < 3u) {
            continue;
        }
        let center = vec2<f32>(select(sq_hi.x, sq_lo.x, right), select(sq_hi.y, sq_lo.y, bottom));
        area -= poly_area(corner) - disk_area(corner, center, r);
    }
    return area;
}

// The inverse of a 3×3 matrix (adjugate over determinant); zero when
// singular.
fn inverse3(m: mat3x3<f32>) -> mat3x3<f32> {
    let r0 = cross(m[1], m[2]);
    let r1 = cross(m[2], m[0]);
    let r2 = cross(m[0], m[1]);
    let det = dot(m[0], r0);
    if (det == 0.0) {
        return mat3x3<f32>(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0));
    }
    return transpose(mat3x3<f32>(r0, r1, r2)) * (1.0 / det);
}

@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4<f32> {
    // The pixel center in the matrix's output space (GL rows: y up).
    let at = in.pos.xy + q.res.zw;
    let inv = inverse3(mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz));
    let h = inv * vec3<f32>(at.x, q.res.y - at.y, 1.0);
    if (h.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    let size = q.uv.xy;
    let r = q.uv.z;
    let local = h.xy / h.z * size;
    // Layer units per device pixel along x and y (the sign of y is
    // immaterial: the pixel square is symmetric).
    let jx = (inv[0].xy * h.z - h.xy * inv[0].z) / (h.z * h.z) * size;
    let jy = (inv[1].xy * h.z - h.xy * inv[1].z) / (h.z * h.z) * size;
    let det = abs(cross2(jx, jy));
    // Relative to the pixel, for precision far from the origin.
    let lo = min(vec2<f32>(0.0), size) - local;
    let hi = max(vec2<f32>(0.0), size) - local;
    let ext = 0.5 * (abs(jx) + abs(jy));
    if (det == 0.0 || any(ext <= lo) || any(-ext >= hi)) {
        return vec4<f32>(0.0);
    }
    let inner_lo = lo + vec2<f32>(r);
    let inner_hi = hi - vec2<f32>(r);
    let inside = all(-ext >= lo) && all(ext <= hi);
    let clear_x = -ext.x >= inner_lo.x && ext.x <= inner_hi.x;
    let clear_y = -ext.y >= inner_lo.y && ext.y <= inner_hi.y;
    if (inside && (r <= 0.0 || clear_x || clear_y)) {
        return q.color;
    }
    var pixel: Poly;
    pixel.n = 4u;
    pixel.v[0] = -0.5 * (jx + jy);
    pixel.v[1] = 0.5 * (jx - jy);
    pixel.v[2] = 0.5 * (jx + jy);
    pixel.v[3] = 0.5 * (jy - jx);
    // f32 leaves coverage within ~1e-6 of exact: snap the ends.
    let coverage = clamp(abs(rounded_area(pixel, lo, hi, r, -ext, ext)) / det, 0.0, 1.0);
    if (coverage > 1.0 - 1e-5) {
        return q.color;
    }
    return q.color * select(coverage, 0.0, coverage < 1e-5);
}

// `FRAG_BACKGROUND_FILL` at full-frame coordinates (a region of interest
// samples the gradient where the full frame has it).
struct Fill {
    stops: array<vec4<f32>, 8>,
    // x: count, y: 1 linear / 2 radial, z: angle in degrees, w: region height.
    params: vec4<f32>,
    // Region origin (xy) and full output size (zw) in pixels.
    view: vec4<f32>,
    // x: 1 when the working encoding is linear light (the gradient is
    // interpolated in E, as today, then decoded).
    flags: vec4<f32>,
};

@group(1) @binding(2) var<uniform> fill: Fill;

fn step_amount(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 <= e0) {
        return select(0.0, 1.0, x >= e0);
    }
    return smoothstep(e0, e1, x);
}

@fragment
fn fs_background(in: VsOut) -> @location(0) vec4<f32> {
    // GL's v_st: x to the right, y up from the bottom edge. Row 0 of the
    // target is the bottom of the region (GL row order).
    let x = in.pos.x + fill.view.x;
    let y_from_bottom = in.pos.y + (fill.view.w - fill.view.y - fill.params.w);
    let st = vec2<f32>(x / fill.view.z, y_from_bottom / fill.view.w);
    let p = st - vec2<f32>(0.5);
    let a = radians(fill.params.z);
    var t: f32;
    if (fill.params.y > 1.5) {
        t = clamp(length(p) * 1.4142, 0.0, 1.0);
    } else {
        t = clamp(dot(p, vec2<f32>(cos(a), sin(a))) + 0.5, 0.0, 1.0);
    }
    var color = fill.stops[0].rgb;
    let count = i32(fill.params.x);
    for (var i = 0; i < 7; i++) {
        if (i >= count - 1) {
            break;
        }
        let left = fill.stops[i];
        let right = fill.stops[i + 1];
        let amount = step_amount(left.a, right.a, t);
        color = mix(color, right.rgb, amount);
    }
    if (fill.flags.x > 0.5) {
        color = pm_e_decode3(color);
    }
    return vec4<f32>(color, 1.0);
}

// The finished frame from GL row order to top-down rows (an exact copy).
@fragment
fn fs_flip(in: VsOut) -> @location(0) vec4<f32> {
    let h = i32(textureDimensions(tex).y);
    let at = vec2<i32>(i32(in.pos.x), h - 1 - i32(in.pos.y));
    return textureLoad(tex, at, 0);
}
