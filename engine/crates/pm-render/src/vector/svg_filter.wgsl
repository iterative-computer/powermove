// SVG filter primitives on a filter grid (`svg_filter.rs`), with
// `svg_filter_common.wgsl` and the W3C blend functions prepended.
//
// fs_point: per-pixel primitives. Each output pixel splits into the parts
// inside both input regions, one, or neither (within the output region);
// the operator runs once per part on the inputs' unclipped values
// (stored / coverage) and the parts are summed by area: exact for constant
// pixels, and no region multiplies a pixel twice.
// fs_offset: a shift (bilinear, exact for the box reconstruction).
// fs_blur: one pass of a Gaussian along an arbitrary axis (grids not
// aligned with the blur's axes).
// fs_replicate: a device grid onto a finer one, pixel for pixel.

@group(0) @binding(0) var in1: texture_2d<f32>;
@group(0) @binding(1) var in2: texture_2d<f32>;
@group(0) @binding(2) var<uniform> fe: Fe;
@group(0) @binding(3) var<storage, read> table: array<f32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 2.0 - 1.0, 1.0 - f32(i >> 1u) * 2.0, 0.0, 1.0);
}

const OP_IDENTITY: u32 = 1u;
const OP_FLOOD: u32 = 2u;
const OP_COMPOSITE: u32 = 3u;
const OP_BLEND: u32 = 4u;
const OP_MATRIX: u32 = 5u;
const OP_TRANSFER: u32 = 6u;
const OP_TINT: u32 = 7u;

// Texel `q` of an image whose texel 0 is `origin` (grid pixels):
// transparent outside it.
fn fetch_at(t: texture_2d<f32>, q: vec2<i32>, origin: vec2<f32>) -> vec4<f32> {
    let local = q - vec2<i32>(origin);
    let dim = vec2<i32>(textureDimensions(t));
    if (any(local < vec2<i32>(0)) || any(local >= dim)) { return vec4<f32>(0.0); }
    return textureLoad(t, local, 0);
}
fn fetch(q: vec2<i32>) -> vec4<f32> { return fetch_at(in1, q, fe.origins.xy); }
fn fetch2(q: vec2<i32>) -> vec4<f32> { return fetch_at(in2, q, fe.origins.zw); }

fn straight(p: vec4<f32>) -> vec4<f32> {
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    return vec4<f32>(p.rgb / p.a, p.a);
}

fn premultiplied(c: vec4<f32>) -> vec4<f32> {
    let s = clamp(c, vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(s.rgb * s.a, s.a);
}

// Filter Effects §15.13 feComponentTransfer, one channel `i`.
fn transfer(c: f32, i: u32) -> f32 {
    let d = fe.m[i];
    let kind = u32(d.x);
    let n = u32(d.y);
    let start = u32(d.z);
    let q = fe.m[i + 4u];
    if (kind == 1u && n > 0u) {
        if (n == 1u) { return table[start]; }
        let x = clamp(c, 0.0, 1.0) * f32(n - 1u);
        let k = min(u32(floor(x)), n - 2u);
        let v0 = table[start + k];
        return v0 + (x - f32(k)) * (table[start + k + 1u] - v0);
    }
    if (kind == 2u && n > 0u) {
        let k = min(u32(floor(clamp(c, 0.0, 1.0) * f32(n))), n - 1u);
        return table[start + k];
    }
    if (kind == 3u) { return q.x * c + q.y; }
    if (kind == 4u) { return q.x * pow(max(c, 0.0), q.y) + q.z; }
    return c;
}

fn porter_duff(a: vec4<f32>, b: vec4<f32>, op: u32) -> vec4<f32> {
    if (op == 1u) { return a * b.a; }
    if (op == 2u) { return a * (1.0 - b.a); }
    if (op == 3u) { return a * b.a + b * (1.0 - a.a); }
    if (op == 4u) { return a * (1.0 - b.a) + b * (1.0 - a.a); }
    if (op == 5u) {
        let k = fe.k;
        let r = k.x * a * b + k.y * a + k.z * b + vec4<f32>(k.w);
        let alpha = clamp(r.a, 0.0, 1.0);
        return vec4<f32>(clamp(r.rgb, vec3<f32>(0.0), vec3<f32>(alpha)), alpha);
    }
    return a + b * (1.0 - a.a);
}

// The operator on values in the primitive's colour space.
fn operate(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let op = fe.mode.x;
    if (op == OP_COMPOSITE) { return porter_duff(a, b, fe.mode.z); }
    if (op == OP_BLEND) {
        let cb = straight(b).rgb;
        let cs = straight(a).rgb;
        let mixed = blend(cb, cs, fe.mode.z);
        return vec4<f32>((1.0 - a.a) * b.rgb + (1.0 - b.a) * a.rgb + a.a * b.a * mixed,
                         a.a + b.a * (1.0 - a.a));
    }
    if (op == OP_MATRIX) {
        let c = straight(a);
        let v = array<f32, 5>(c.r, c.g, c.b, c.a, 1.0);
        var r = vec4<f32>(0.0);
        for (var row = 0u; row < 4u; row++) {
            var sum = 0.0;
            for (var col = 0u; col < 5u; col++) {
                let e = row * 5u + col;
                sum += fe.m[e / 4u][e % 4u] * v[col];
            }
            r[row] = sum;
        }
        return premultiplied(r);
    }
    if (op == OP_TRANSFER) {
        let c = straight(a);
        return premultiplied(vec4<f32>(transfer(c.r, 0u), transfer(c.g, 1u),
                                       transfer(c.b, 2u), transfer(c.a, 3u)));
    }
    return a;
}

// One region part: the operator on unclipped values, stored back.
fn part(u1: vec4<f32>, u2: vec4<f32>) -> vec4<f32> {
    let op = fe.mode.x;
    if (op == OP_FLOOD) { return fe.k; }
    // The shadow colour (premultiplied sRGB) at the input's alpha:
    // conversions commute with the scale.
    if (op == OP_TINT) { return fe.k * u1.a; }
    let a = fe_in(u1, (fe.mode.y & 1u) != 0u);
    let b = fe_in(u2, (fe.mode.y & 2u) != 0u);
    return fe_out(operate(a, b));
}

fn unclipped(stored: vec4<f32>, coverage: f32) -> vec4<f32> {
    return select(vec4<f32>(0.0), stored / coverage, coverage > 1e-7);
}

@fragment fn fs_point(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = floor(pos.xy) + fe.origin.xy;
    let q = vec2<i32>(px);
    let cs = fe_cover(fe.s, px);
    if (cs <= 0.0) { return vec4<f32>(0.0); }
    let u1 = unclipped(fetch(q), fe_cover(fe.q1, px));
    let u2 = unclipped(fetch2(q), fe_cover(fe.q2, px));
    let a1 = fe_cover(fe_isect(fe.q1, fe.s), px);
    let a2 = fe_cover(fe_isect(fe.q2, fe.s), px);
    let a12 = fe_cover(fe_isect(fe_isect(fe.q1, fe.q2), fe.s), px);
    let zero = vec4<f32>(0.0);
    var out = a12 * part(u1, u2);
    out += max(a1 - a12, 0.0) * part(u1, zero);
    out += max(a2 - a12, 0.0) * part(zero, u2);
    out += max(cs - a1 - a2 + a12, 0.0) * part(zero, zero);
    return out;
}

// `in1` at grid point `q` (pixel centres at half-integers), bilinear,
// in the primitive's space when `convert`.
fn sample(q: vec2<f32>, convert: bool) -> vec4<f32> {
    let base = floor(q - vec2<f32>(0.5));
    let f = q - vec2<f32>(0.5) - base;
    let a = vec2<i32>(base);
    var s = array<vec4<f32>, 4>(fetch(a), fetch(a + vec2<i32>(1, 0)),
                                fetch(a + vec2<i32>(0, 1)), fetch(a + vec2<i32>(1, 1)));
    if (convert) {
        let alpha = (fe.mode.y & 1u) != 0u;
        for (var i = 0; i < 4; i++) { s[i] = fe_in(s[i], alpha); }
    }
    return mix(mix(s[0], s[1], f.x), mix(s[2], s[3], f.x), f.y);
}

// k.xy: the shift in grid pixels; q1: the shifted input's region.
@fragment fn fs_offset(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = floor(pos.xy) + fe.origin.xy;
    let shifted = fe_out(sample(pos.xy + fe.origin.xy - fe.k.xy, true));
    let inside = fe_cover(fe.q1, px);
    return select(vec4<f32>(0.0), shifted * (fe_cover(fe_isect(fe.q1, fe.s), px) / inside),
                  inside > 1e-7);
}

// k.xy: the axis in grid pixels; mode.w: radius; table: weights. Pass 0
// reads the input into the primitive's space; pass 1 writes it back,
// clipped to the output region.
@fragment fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let radius = i32(fe.mode.w);
    let first = fe.mode.z == 0u;
    let at = pos.xy + fe.origin.xy;
    var sum = vec4<f32>(0.0);
    for (var k = -radius; k <= radius; k++) {
        sum += sample(at + f32(k) * fe.k.xy, first) * table[u32(k + radius)];
    }
    if (first) { return sum; }
    return fe_out(sum) * fe_cover(fe.s, floor(at));
}

// mode.z: grid pixels per input pixel.
@fragment fn fs_replicate(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let g = vec2<i32>(floor(pos.xy + fe.origin.xy));
    let rate = i32(fe.mode.z);
    return fetch(vec2<i32>(floor(vec2<f32>(g) / f32(rate))));
}
