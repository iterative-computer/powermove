// Section 13 normalization of decoded frames, sampled at native depth.
//
// Y′CbCr layouts write out0 = (Y′, 0, 0, A) at luma resolution and
// out1 = (Cb, Cr) at chroma resolution: no chroma upsampling here (section 9
// owns siting-aware upsampling). RGB layouts write out0 = (R, G, B, A).
//
// Raw codes: an n-bit unorm texel u is the integer v = round(u · (2ⁿ − 1));
// the constants apply to v (MSB-aligned formats fold their shift into them).

struct Params {
    kind: u32,        // 0 biplanar, 1 planar, 2 y416 (A,Y′,Cb,Cr), 3 v216 (Cb,Y′0,Cr,Y′1), 4 planar G,B,R, 5 packed RGBA
    alpha: u32,       // 0 none, 1 plane 2, 2 packed (y416 .r, RGBA .a), 3 plane 3
    sx: u32,          // log2 chroma subsampling
    sy: u32,
    code_max: f32,    // unorm → code (255 or 65535); 0 for float textures
    a_code_max: f32,  // the same for the alpha plane
    y_off: f32,
    y_div: f32,
    c_off: f32,
    c_div: f32,
    a_div: f32,
    _pad: f32,
};

@group(0) @binding(0) var p0: texture_2d<f32>;
@group(0) @binding(1) var p1: texture_2d<f32>;
@group(0) @binding(2) var p2: texture_2d<f32>;
@group(0) @binding(3) var p3: texture_2d<f32>;
@group(0) @binding(4) var<uniform> P: Params;
@group(0) @binding(5) var out0: texture_storage_2d<rgba32float, write>;
@group(0) @binding(6) var out1: texture_storage_2d<rg32float, write>;

fn code(u: f32, m: f32) -> f32 {
    if (m == 0.0) {
        return u;
    }
    return round(u * m);
}

fn luma(u: f32) -> f32 { return (code(u, P.code_max) - P.y_off) / P.y_div; }
fn chroma(u: f32) -> f32 { return (code(u, P.code_max) - P.c_off) / P.c_div; }
fn alpha(u: f32) -> f32 {
    if (P.a_code_max == 0.0) {
        return u;
    }
    return round(u * P.a_code_max) / P.a_div;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(out0);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let xy = vec2<i32>(id.xy);
    let cxy = vec2<i32>(i32(id.x >> P.sx), i32(id.y >> P.sy));
    let first = ((id.x & ((1u << P.sx) - 1u)) == 0u) && ((id.y & ((1u << P.sy) - 1u)) == 0u);
    var a: f32 = 1.0;
    if (P.kind >= 4u) {
        var rgb: vec3<f32>;
        if (P.kind == 4u) {
            rgb = vec3<f32>(luma(textureLoad(p2, xy, 0).r), luma(textureLoad(p0, xy, 0).r), luma(textureLoad(p1, xy, 0).r));
            if (P.alpha == 3u) {
                a = alpha(textureLoad(p3, xy, 0).r);
            }
        } else {
            let v = textureLoad(p0, xy, 0);
            rgb = vec3<f32>(luma(v.r), luma(v.g), luma(v.b));
            if (P.alpha == 2u) {
                a = alpha(v.a);
            }
        }
        textureStore(out0, xy, vec4<f32>(rgb, a));
        return;
    }
    var y: f32;
    var cb: f32;
    var cr: f32;
    if (P.kind == 2u) {
        let v = textureLoad(p0, xy, 0);
        a = alpha(v.r);
        y = luma(v.g);
        cb = chroma(v.b);
        cr = chroma(v.a);
    } else if (P.kind == 3u) {
        let t = textureLoad(p0, vec2<i32>(xy.x / 2, xy.y), 0);
        if ((id.x & 1u) == 0u) {
            y = luma(t.g);
        } else {
            y = luma(t.a);
        }
        let c = textureLoad(p0, cxy, 0);
        cb = chroma(c.r);
        cr = chroma(c.b);
    } else if (P.kind == 0u) {
        y = luma(textureLoad(p0, xy, 0).r);
        let c = textureLoad(p1, cxy, 0);
        cb = chroma(c.r);
        cr = chroma(c.g);
    } else {
        y = luma(textureLoad(p0, xy, 0).r);
        cb = chroma(textureLoad(p1, cxy, 0).r);
        cr = chroma(textureLoad(p2, cxy, 0).r);
    }
    if (P.alpha == 1u) {
        a = alpha(textureLoad(p2, xy, 0).r);
    } else if (P.alpha == 3u) {
        a = alpha(textureLoad(p3, xy, 0).r);
    }
    textureStore(out0, xy, vec4<f32>(y, 0.0, 0.0, a));
    if (first) {
        textureStore(out1, cxy, vec4<f32>(cb, cr, 0.0, 0.0));
    }
}
