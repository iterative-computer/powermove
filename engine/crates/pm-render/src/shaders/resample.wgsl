// Level 2 raster sampling (plan section 8, "Sampling and antialiasing";
// docs/engine/motion.md, "Level 2 sampling").
//
// One draw covers the pixel bounds of a layer's content polygon (its quad,
// or the part of it the UV window fills, clipped at the camera plane). Each
// fragment maps its pixel center back to the unit quad through the inverse
// homography, samples the source with the declared kernel, and multiplies
// by the exact area of the pixel square inside the polygon (analytic
// coverage edges). Targets keep GL's row order as every 2D pass does
// (composite.wgsl); `view` turns a fragment position into top-down
// full-frame output pixels.

struct Quad {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    res: vec4<f32>,
    uv: vec4<f32>,
    color: vec4<f32>,
};

struct Resample {
    // Output pixel (top-down, full frame) to unit quad, column-major.
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    inv2: vec4<f32>,
    // Content UV window: offset (xy), scale (zw).
    uvwin: vec4<f32>,
    // x: alpha, y: 1 when the source is in GL row order, z: 0 playback
    // (trilinear) or 1 final (Lanczos-3), w: number of edges.
    params: vec4<f32>,
    // xy: region origin in full-frame pixels, z: target height.
    view: vec4<f32>,
    // Source width and height at level 0, mip levels.
    tex: vec4<f32>,
    // xy: the origin the edges are relative to (output pixels), so edge
    // distances keep their precision far from the frame's origin.
    origin: vec4<f32>,
    // Content polygon edges, unit normals: a·x + b·y + c ≥ 0 inside, with
    // (x, y) relative to `origin`.
    edges: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> q: Quad;
@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var trilinear: sampler;
@group(1) @binding(2) var<uniform> r: Resample;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var o: VsOut;
    o.pos = vec4<f32>((p.x / q.res.x) * 2.0 - p.z, (p.y / q.res.y) * 2.0 - p.z, 0.0, p.z);
    return o;
}

// The area of the unit pixel square centered at `c` (relative to
// `origin`) inside the polygon: Sutherland–Hodgman against each edge in
// coordinates relative to the pixel center, then the shoelace formula.
fn coverage(c: vec2<f32>) -> f32 {
    var poly: array<vec2<f32>, 12>;
    var next: array<vec2<f32>, 12>;
    poly[0] = vec2<f32>(-0.5, -0.5);
    poly[1] = vec2<f32>(0.5, -0.5);
    poly[2] = vec2<f32>(0.5, 0.5);
    poly[3] = vec2<f32>(-0.5, 0.5);
    var n = 4u;
    let count = u32(r.params.w);
    for (var e = 0u; e < count; e++) {
        // The edge moved to the pixel center.
        let edge = vec3<f32>(r.edges[e].xy, r.edges[e].z + dot(r.edges[e].xy, c));
        var m = 0u;
        for (var i = 0u; i < n; i++) {
            let a = poly[i];
            let b = poly[(i + 1u) % n];
            let da = dot(edge.xy, a) + edge.z;
            let db = dot(edge.xy, b) + edge.z;
            if (da >= 0.0) {
                next[m] = a;
                m++;
            }
            if ((da >= 0.0) != (db >= 0.0)) {
                let t = da / (da - db);
                next[m] = a + (b - a) * t;
                m++;
            }
        }
        n = m;
        for (var i = 0u; i < n; i++) {
            poly[i] = next[i];
        }
        if (n == 0u) {
            return 0.0;
        }
    }
    var area = 0.0;
    for (var i = 0u; i < n; i++) {
        let a = poly[i];
        let b = poly[(i + 1u) % n];
        area += a.x * b.y - b.x * a.y;
    }
    return clamp(abs(area) * 0.5, 0.0, 1.0);
}

const PI: f32 = 3.14159265358979;

fn lanczos3(x: f32) -> f32 {
    let ax = abs(x);
    if (ax >= 3.0) {
        return 0.0;
    }
    if (ax < 1e-5) {
        return 1.0;
    }
    let px = PI * x;
    return 3.0 * sin(px) * sin(px / 3.0) / (px * px);
}

// The most source texels per kernel unit taken from one pyramid level; a
// stronger minification reads a coarser (box-filtered) level.
const MAX_KERNEL_SCALE: f32 = 4.0;

// Lanczos-3 in the principal axes of the mapping (`j` = texels per output
// pixel along x and y), with the anti-ringing clamp to the central texels.
fn lanczos(t0: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    // Principal axes: eigenvectors of J·Jᵀ (texel space).
    let m00 = dx.x * dx.x + dy.x * dy.x;
    let m01 = dx.x * dx.y + dy.x * dy.y;
    let m11 = dx.y * dx.y + dy.y * dy.y;
    let tr = m00 + m11;
    let det = m00 * m11 - m01 * m01;
    let disc = sqrt(max(tr * tr * 0.25 - det, 0.0));
    let l1 = max(tr * 0.5 + disc, 0.0);
    let l2 = max(tr * 0.5 - disc, 0.0);
    // Isotropic mappings (equal singular values within rounding) keep the
    // texel axes; otherwise the major axis's eigenvector.
    var u1 = vec2<f32>(1.0, 0.0);
    if (disc > 1e-4 * tr) {
        if (abs(m01) > 1e-6 * tr) {
            u1 = normalize(vec2<f32>(l1 - m11, m01));
        } else if (m11 > m00) {
            u1 = vec2<f32>(0.0, 1.0);
        }
    }
    let u2 = vec2<f32>(-u1.y, u1.x);
    let s1 = max(1.0, sqrt(l1));
    let s2 = max(1.0, sqrt(l2));
    let levels = r.tex.z;
    let level = clamp(ceil(log2(s1 / MAX_KERNEL_SCALE)), 0.0, levels - 1.0);
    let k = exp2(level);
    let size = vec2<f32>(textureDimensions(src, i32(level)));
    let a1 = max(1.0, s1 / k);
    let a2 = max(1.0, s2 / k);
    // The sample position at this level, in texel units (centers at +0.5).
    let t = t0 * size / r.tex.xy;
    let extent = 3.0 * (abs(u1) * a1 + abs(u2) * a2);
    let lo = vec2<i32>(floor(t - extent));
    let hi = vec2<i32>(ceil(t + extent));
    let last = vec2<i32>(size) - vec2<i32>(1, 1);
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    var mn = vec4<f32>(1e30);
    var mx = vec4<f32>(-1e30);
    for (var y = lo.y; y <= hi.y; y++) {
        for (var x = lo.x; x <= hi.x; x++) {
            let d = vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5) - t;
            let kx = dot(d, u1) / a1;
            let ky = dot(d, u2) / a2;
            if (abs(kx) >= 3.0 || abs(ky) >= 3.0) {
                continue;
            }
            let c = textureLoad(src, clamp(vec2<i32>(x, y), vec2<i32>(0), last), i32(level));
            let w = lanczos3(kx) * lanczos3(ky);
            sum += c * w;
            wsum += w;
            if (abs(kx) < 1.0 && abs(ky) < 1.0) {
                mn = min(mn, c);
                mx = max(mx, c);
            }
        }
    }
    var out = sum / wsum;
    if (mx.x >= mn.x) {
        out = clamp(out, mn, mx);
    }
    return out;
}

@fragment
fn fs_resample(in: VsOut) -> @location(0) vec4<f32> {
    // Top-down full-frame output pixel of this fragment.
    let px = vec2<f32>(in.pos.x + r.view.x, r.view.y + r.view.z - in.pos.y);
    let inv = mat3x3<f32>(r.inv0.xyz, r.inv1.xyz, r.inv2.xyz);
    let h = inv * vec3<f32>(px, 1.0);
    let quad = h.xy / h.z;
    var uv = r.uvwin.xy + quad * r.uvwin.zw;
    if (r.params.y > 0.5) {
        uv.y = 1.0 - uv.y;
    }
    // The exact Jacobian of the mapping: content UV per output pixel
    // (∂q/∂x = (inv₀ − q·inv₀.z) / w, likewise for y), then texels.
    var duvx = (r.inv0.xy - quad * r.inv0.z) / h.z * r.uvwin.zw;
    var duvy = (r.inv1.xy - quad * r.inv1.z) / h.z * r.uvwin.zw;
    if (r.params.y > 0.5) {
        duvx.y = -duvx.y;
        duvy.y = -duvy.y;
    }
    let dx = duvx * r.tex.xy;
    let dy = duvy * r.tex.xy;
    let cov = coverage(px - r.origin.xy);
    if (cov <= 0.0 || h.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    // Pixels at the polygon's edge sample the nearest content (clamp).
    let cuv = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    var c: vec4<f32>;
    if (r.params.z < 0.5) {
        c = textureSampleGrad(src, trilinear, cuv, duvx, duvy);
    } else {
        c = lanczos(cuv * r.tex.xy, dx, dy);
    }
    return c * (cov * r.params.x);
}
