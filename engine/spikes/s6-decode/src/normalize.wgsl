// Section 13 normalization of decoded Y'CbCr(A) planes, sampled at native depth.
// Outputs: out_y = (Y', A) at luma resolution; out_c = (Cb, Cr) at chroma
// resolution. No chroma upsampling here (section 9 owns siting-aware upsampling).

struct Params {
    kind: u32,      // 0 biplanar, 1 planar, 2 packed y416 (R=A, G=Y', B=Cb, A=Cr)
    alpha: u32,     // 0 none, 1 plane 2, 2 packed, 3 plane 3
    sx: u32,        // log2 chroma subsampling
    sy: u32,
    code_max: f32,  // unorm -> integer code (255 or 65535)
    shift: f32,     // 64 for MSB-aligned 10-bit in 16, else 1
    y_off: f32,
    y_div: f32,
    c_off: f32,
    c_div: f32,
    a_code_max: f32,
    a_shift: f32,
    a_off: f32,
    a_div: f32,
    _p0: f32,
    _p1: f32,
};

@group(0) @binding(0) var p0: texture_2d<f32>;
@group(0) @binding(1) var p1: texture_2d<f32>;
@group(0) @binding(2) var p2: texture_2d<f32>;
@group(0) @binding(3) var p3: texture_2d<f32>;
@group(0) @binding(4) var<uniform> P: Params;
@group(0) @binding(5) var out_y: texture_storage_2d<rgba32float, write>;
@group(0) @binding(6) var out_c: texture_storage_2d<rgba32float, write>;

fn code(u: f32, code_max: f32, shift: f32) -> f32 {
    return round(u * code_max) / shift;
}

fn luma(c: f32) -> f32 { return (c - P.y_off) / P.y_div; }
fn chroma(c: f32) -> f32 { return (c - P.c_off) / P.c_div; }
fn alpha(u: f32) -> f32 { return (code(u, P.a_code_max, P.a_shift) - P.a_off) / P.a_div; }

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(out_y);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let xy = vec2<i32>(id.xy);
    let cxy = vec2<i32>(i32(id.x >> P.sx), i32(id.y >> P.sy));
    var y: f32;
    var cb: f32;
    var cr: f32;
    var a: f32 = 1.0;
    if (P.kind == 2u) {
        let v = textureLoad(p0, xy, 0);
        a = alpha(v.r);
        y = luma(code(v.g, P.code_max, P.shift));
        cb = chroma(code(v.b, P.code_max, P.shift));
        cr = chroma(code(v.a, P.code_max, P.shift));
    } else if (P.kind == 0u) {
        y = luma(code(textureLoad(p0, xy, 0).r, P.code_max, P.shift));
        let c = textureLoad(p1, cxy, 0);
        cb = chroma(code(c.r, P.code_max, P.shift));
        cr = chroma(code(c.g, P.code_max, P.shift));
    } else {
        y = luma(code(textureLoad(p0, xy, 0).r, P.code_max, P.shift));
        cb = chroma(code(textureLoad(p1, cxy, 0).r, P.code_max, P.shift));
        cr = chroma(code(textureLoad(p2, cxy, 0).r, P.code_max, P.shift));
    }
    if (P.alpha == 1u) {
        a = alpha(textureLoad(p2, xy, 0).r);
    } else if (P.alpha == 3u) {
        a = alpha(textureLoad(p3, xy, 0).r);
    }
    textureStore(out_y, xy, vec4<f32>(y, a, 0.0, 1.0));
    let mx = (1u << P.sx) - 1u;
    let my = (1u << P.sy) - 1u;
    if ((id.x & mx) == 0u && (id.y & my) == 0u) {
        textureStore(out_c, cxy, vec4<f32>(cb, cr, 0.0, 1.0));
    }
}
