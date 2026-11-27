// bwdif deinterlacing (section 13) on normalized planes, in Y′CbCr before
// chroma upsampling. A port of FFmpeg's bwdif (libavfilter/bwdifdsp.c and
// vf_bwdif.c, LGPL-2.1-or-later; algorithm by Thomas Mundt) with the same
// taps, coefficients, spatial checks, edge rules, and integer arithmetic:
// normalized values are turned back into their exact integer codes (the
// normalization is affine and exact to well under half a code), filtered
// as FFmpeg filters them, and normalized again, so the output equals
// FFmpeg's bit for bit. Float images (no integer codes) use the same steps
// in f32.
//
// Lines whose parity matches the kept field are copied from `cur`; the
// others are interpolated from `prev`, `cur`, and `next`. Each plane (luma
// with alpha, and chroma at chroma resolution, whose rows alternate fields
// in interlaced material) is processed on its own, as FFmpeg does.

struct Params {
    parity: u32,   // interpolate lines with ((y ^ parity) & 1) == 1
    prev2: u32,    // parity ^ tff: 1 when the temporal pair is (prev, cur), else (cur, next)
    intra: u32,    // the first field of the first frame or the last field of the last frame
                   // (FFmpeg's YADIF_FIELD_END): every line is spatial-only filter_intra
    h: u32,
    integer: u32,  // 1: filter integer codes; 0: f32 values
    _p0: u32,
    _p1: u32,
    _p2: u32,
    off: vec4<f32>,   // per channel: code = value · div + off
    div: vec4<f32>,
    cmax: vec4<f32>,  // per channel: largest code (2ⁿ − 1)
};

@group(0) @binding(0) var tprev: texture_2d<f32>;
@group(0) @binding(1) var tcur: texture_2d<f32>;
@group(0) @binding(2) var tnext: texture_2d<f32>;
@group(0) @binding(3) var<uniform> P: Params;
@group(0) @binding(4) var out: texture_storage_2d<FORMAT, write>;

fn ld(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    return textureLoad(t, vec2<i32>(x, y), 0);
}

fn li(t: texture_2d<f32>, x: i32, y: i32) -> vec4<i32> {
    return vec4<i32>(round(ld(t, x, y) * P.div + P.off));
}

fn unnorm(c: vec4<i32>) -> vec4<f32> {
    return (vec4<f32>(c) - P.off) / P.div;
}

fn max3i(a: vec4<i32>, b: vec4<i32>, c: vec4<i32>) -> vec4<i32> { return max(max(a, b), c); }
fn min3i(a: vec4<i32>, b: vec4<i32>, c: vec4<i32>) -> vec4<i32> { return min(min(a, b), c); }
fn max3f(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>) -> vec4<f32> { return max(max(a, b), c); }
fn min3f(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>) -> vec4<f32> { return min(min(a, b), c); }

fn sh(v: vec4<i32>, n: u32) -> vec4<i32> { return v >> vec4<u32>(n); }

// FFmpeg's integer filter for one output sample.
fn filter_int(x: i32, y: i32, h: i32, mr: i32, pr: i32) -> vec4<i32> {
    let cmax = vec4<i32>(P.cmax);
    let c = li(tcur, x, y + mr);
    let e = li(tcur, x, y + pr);
    if (P.intra == 1u) {
        var p3 = 3;
        if (y + 3 >= h) { p3 = -1; }
        var m3 = -3;
        if (y <= 2) { m3 = 1; }
        let v = sh(5077 * (c + e) - 981 * (li(tcur, x, y + m3) + li(tcur, x, y + p3)), 13u);
        return clamp(v, vec4<i32>(0), cmax);
    }
    let use_prev = P.prev2 == 1u;
    let p20 = select(li(tcur, x, y), li(tprev, x, y), use_prev);
    let n20 = select(li(tnext, x, y), li(tcur, x, y), use_prev);
    let d = sh(p20 + n20, 1u);
    let td0 = abs(p20 - n20);
    let td1 = sh(abs(li(tprev, x, y + mr) - c) + abs(li(tprev, x, y + pr) - e), 1u);
    let td2 = sh(abs(li(tnext, x, y + mr) - c) + abs(li(tnext, x, y + pr) - e), 1u);
    let diff0 = max3i(sh(td0, 1u), td1, td2);
    var diff = diff0;
    let edge = (y < 4) || (y + 5 > h);
    let spat = !((y < 2) || (y + 3 > h));
    var interpol = sh(c + e, 1u);
    if (!edge || spat) {
        let p2m2 = select(li(tcur, x, y - 2), li(tprev, x, y - 2), use_prev);
        let n2m2 = select(li(tnext, x, y - 2), li(tcur, x, y - 2), use_prev);
        let p2p2 = select(li(tcur, x, y + 2), li(tprev, x, y + 2), use_prev);
        let n2p2 = select(li(tnext, x, y + 2), li(tcur, x, y + 2), use_prev);
        let b = sh(p2m2 + n2m2, 1u) - c;
        let f = sh(p2p2 + n2p2, 1u) - e;
        let dc = d - c;
        let de = d - e;
        let mx = max3i(de, dc, min(b, f));
        let mn = min3i(de, dc, max(b, f));
        diff = max3i(diff, mn, -mx);
        if (!edge) {
            let p2m4 = select(li(tcur, x, y - 4), li(tprev, x, y - 4), use_prev);
            let n2m4 = select(li(tnext, x, y - 4), li(tcur, x, y - 4), use_prev);
            let p2p4 = select(li(tcur, x, y + 4), li(tprev, x, y + 4), use_prev);
            let n2p4 = select(li(tnext, x, y + 4), li(tcur, x, y + 4), use_prev);
            let m3 = li(tcur, x, y - 3);
            let p3 = li(tcur, x, y + 3);
            let hf = sh(sh(5570 * (p20 + n20) - 3801 * (p2m2 + n2m2 + p2p2 + n2p2) + 1016 * (p2m4 + n2m4 + p2p4 + n2p4), 2u)
                + 4309 * (c + e) - 213 * (m3 + p3), 13u);
            let sp = sh(5077 * (c + e) - 981 * (m3 + p3), 13u);
            interpol = select(sp, hf, abs(c - e) > td0);
        }
    }
    let v = clamp(clamp(interpol, d - diff, d + diff), vec4<i32>(0), cmax);
    return select(v, d, diff0 == vec4<i32>(0));
}

// The same steps on f32 values (float images).
fn filter_f32(x: i32, y: i32, h: i32, mr: i32, pr: i32) -> vec4<f32> {
    let c = ld(tcur, x, y + mr);
    let e = ld(tcur, x, y + pr);
    if (P.intra == 1u) {
        var p3 = 3;
        if (y + 3 >= h) { p3 = -1; }
        var m3 = -3;
        if (y <= 2) { m3 = 1; }
        return (5077.0 * (c + e) - 981.0 * (ld(tcur, x, y + m3) + ld(tcur, x, y + p3))) / 8192.0;
    }
    let use_prev = P.prev2 == 1u;
    let p20 = select(ld(tcur, x, y), ld(tprev, x, y), use_prev);
    let n20 = select(ld(tnext, x, y), ld(tcur, x, y), use_prev);
    let d = (p20 + n20) * 0.5;
    let td0 = abs(p20 - n20);
    let td1 = (abs(ld(tprev, x, y + mr) - c) + abs(ld(tprev, x, y + pr) - e)) * 0.5;
    let td2 = (abs(ld(tnext, x, y + mr) - c) + abs(ld(tnext, x, y + pr) - e)) * 0.5;
    let diff0 = max3f(td0 * 0.5, td1, td2);
    var diff = diff0;
    let edge = (y < 4) || (y + 5 > h);
    let spat = !((y < 2) || (y + 3 > h));
    var interpol = (c + e) * 0.5;
    if (!edge || spat) {
        let p2m2 = select(ld(tcur, x, y - 2), ld(tprev, x, y - 2), use_prev);
        let n2m2 = select(ld(tnext, x, y - 2), ld(tcur, x, y - 2), use_prev);
        let p2p2 = select(ld(tcur, x, y + 2), ld(tprev, x, y + 2), use_prev);
        let n2p2 = select(ld(tnext, x, y + 2), ld(tcur, x, y + 2), use_prev);
        let b = (p2m2 + n2m2) * 0.5 - c;
        let f = (p2p2 + n2p2) * 0.5 - e;
        let dc = d - c;
        let de = d - e;
        let mx = max3f(de, dc, min(b, f));
        let mn = min3f(de, dc, max(b, f));
        diff = max3f(diff, mn, -mx);
        if (!edge) {
            let p2m4 = select(ld(tcur, x, y - 4), ld(tprev, x, y - 4), use_prev);
            let n2m4 = select(ld(tnext, x, y - 4), ld(tcur, x, y - 4), use_prev);
            let p2p4 = select(ld(tcur, x, y + 4), ld(tprev, x, y + 4), use_prev);
            let n2p4 = select(ld(tnext, x, y + 4), ld(tcur, x, y + 4), use_prev);
            let m3 = ld(tcur, x, y - 3);
            let p3 = ld(tcur, x, y + 3);
            let hf = ((5570.0 * (p20 + n20) - 3801.0 * (p2m2 + n2m2 + p2p2 + n2p2) + 1016.0 * (p2m4 + n2m4 + p2p4 + n2p4)) * 0.25
                + 4309.0 * (c + e) - 213.0 * (m3 + p3)) / 8192.0;
            let sp = (5077.0 * (c + e) - 981.0 * (m3 + p3)) / 8192.0;
            interpol = select(sp, hf, abs(c - e) > td0);
        }
    }
    return select(clamp(interpol, d - diff, d + diff), d, diff0 == vec4<f32>(0.0));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(tcur);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    if (((id.y ^ P.parity) & 1u) == 0u) {
        textureStore(out, vec2<i32>(x, y), ld(tcur, x, y));
        return;
    }
    let h = i32(P.h);
    // Neighbor lines, mirrored at the edges as vf_bwdif.c does.
    var pr = 1;
    if (y + 1 >= h) { pr = -1; }
    var mr = -1;
    if (y <= 0) { mr = 1; }
    var v: vec4<f32>;
    if (P.integer == 1u) {
        v = unnorm(filter_int(x, y, h, mr, pr));
    } else {
        v = filter_f32(x, y, h, mr, pr);
    }
    textureStore(out, vec2<i32>(x, y), v);
}
