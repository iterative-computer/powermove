//! CSS Color 4 and 5 colors as Chromium's canvas `fillStyle` takes them:
//! `lab()`, `lch()`, `oklab()`, `oklch()`, `color()` in every predefined
//! space, `color-mix()`, relative colors (`rgb(from …)`), `calc()` in
//! channels, `none`, `currentcolor` and the system colors a detached canvas
//! resolves. [`super::color`] parses the legacy sRGB forms itself and asks
//! this module for the rest.
//!
//! A port of Blink's color model (`platform/graphics/color.cc`,
//! `core/css/properties/css_color_function_parser.cc`,
//! `core/css/style_color.cc`, `core/css/css_color_mix_value.cc`) and
//! Chromium's conversions (`ui/gfx/color_conversions.cc`,
//! `skia/ext/skcms_ext.cc`, the parts of skcms they call), in the same
//! `f32` parameters and arithmetic, so the color Skia paints with
//! (`Color::toSkColor4f`, sRGB, not clamped) is the same. Verified on the
//! closure capture's `css-color` cases.

// A port: Blink's arithmetic and constants are kept as written (the clamps
// of `FromColorSpace` are `min(max())` in its order; its constants carry
// their source precision).
#![allow(
    clippy::excessive_precision,
    clippy::manual_clamp,
    clippy::wrong_self_convention,
    clippy::needless_range_loop
)]

use super::color::Rgba;

// ---------------------------------------------------------------------
// skcms and Chromium's skcms_ext

type Mat = [[f32; 3]; 3];
type Vec3 = [f32; 3];

#[derive(Clone, Copy, PartialEq)]
struct Tf {
    g: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

const TF_LINEAR: Tf = Tf {
    g: 1.0,
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 0.0,
    e: 0.0,
    f: 0.0,
};
const TF_SRGB: Tf = Tf {
    g: 2.4,
    a: (1.0 / 1.055) as f32,
    b: (0.055 / 1.055) as f32,
    c: (1.0 / 12.92) as f32,
    d: 0.04045,
    e: 0.0,
    f: 0.0,
};
const TF_2DOT2: Tf = Tf {
    g: 2.2,
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 0.0,
    e: 0.0,
    f: 0.0,
};
const TF_REC2020: Tf = Tf {
    g: 2.22222,
    a: 0.909672,
    b: 0.0903276,
    c: 0.222222,
    d: 0.0812429,
    e: 0.0,
    f: 0.0,
};
const TF_PROPHOTO: Tf = Tf {
    g: 1.8,
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 0.0,
    e: 0.0,
    f: 0.0,
};

const fn fixed(v: i32) -> f32 {
    v as f32 * (1.0 / 65536.0)
}

const GAMUT_SRGB: Mat = [
    [fixed(0x6FA2), fixed(0x6299), fixed(0x24A0)],
    [fixed(0x38F5), fixed(0xB785), fixed(0x0F84)],
    [fixed(0x0390), fixed(0x18DA), fixed(0xB6CF)],
];
const GAMUT_ADOBE: Mat = [
    [fixed(0x9c18), fixed(0x348d), fixed(0x2631)],
    [fixed(0x4fa5), fixed(0xa02c), fixed(0x102f)],
    [fixed(0x04fc), fixed(0x0f95), fixed(0xbe9c)],
];
const GAMUT_P3: Mat = [
    [0.515102, 0.291965, 0.157153],
    [0.241182, 0.692236, 0.0665819],
    [-0.00104941, 0.0418818, 0.784378],
];
const GAMUT_REC2020: Mat = [
    [0.673459, 0.165661, 0.125100],
    [0.279033, 0.675338, 0.0456288],
    [-0.00193139, 0.0299794, 0.797162],
];
const GAMUT_XYZ: Mat = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mat_apply(m: &Mat, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// `skcms_Matrix3x3_concat`.
fn mat_concat(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0f32; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    m
}

/// `skcms_Matrix3x3_invert` (in `double`).
fn mat_invert(src: &Mat) -> Option<Mat> {
    let a00 = f64::from(src[0][0]);
    let a01 = f64::from(src[1][0]);
    let a02 = f64::from(src[2][0]);
    let a10 = f64::from(src[0][1]);
    let a11 = f64::from(src[1][1]);
    let a12 = f64::from(src[2][1]);
    let a20 = f64::from(src[0][2]);
    let a21 = f64::from(src[1][2]);
    let a22 = f64::from(src[2][2]);
    let mut b0 = a00 * a11 - a01 * a10;
    let mut b1 = a00 * a12 - a02 * a10;
    let mut b2 = a01 * a12 - a02 * a11;
    let mut b3 = a20;
    let mut b4 = a21;
    let mut b5 = a22;
    let det = b0 * b5 - b1 * b4 + b2 * b3;
    if det == 0.0 {
        return None;
    }
    let invdet = 1.0 / det;
    if !(invdet as f32).is_finite() {
        return None;
    }
    b0 *= invdet;
    b1 *= invdet;
    b2 *= invdet;
    b3 *= invdet;
    b4 *= invdet;
    b5 *= invdet;
    let mut d = [[0.0f32; 3]; 3];
    d[0][0] = (a11 * b5 - a12 * b4) as f32;
    d[1][0] = (a02 * b4 - a01 * b5) as f32;
    d[2][0] = b2 as f32;
    d[0][1] = (a12 * b3 - a10 * b5) as f32;
    d[1][1] = (a00 * b5 - a02 * b3) as f32;
    d[2][1] = (-b1) as f32;
    d[0][2] = (a10 * b4 - a11 * b3) as f32;
    d[1][2] = (a01 * b3 - a00 * b4) as f32;
    d[2][2] = b0 as f32;
    if d.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    Some(d)
}

/// `skcms::Matrix3x3_apply_inverse` (the identity early-outs).
fn mat_apply_inverse(m: &Mat, v: Vec3) -> Vec3 {
    if *m == GAMUT_XYZ {
        return v;
    }
    let inv = mat_invert(m).unwrap_or(GAMUT_XYZ);
    mat_apply(&inv, v)
}

/// `skcms_AdaptToXYZD50`.
fn adapt_to_xyz_d50(wx: f32, wy: f32) -> Mat {
    let w_xyz = [wx / wy, 1.0, (1.0 - wx - wy) / wy];
    let w_d50 = [0.96422f32, 1.0, 0.82521];
    let xyz_to_lms: Mat = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let lms_to_xyz: Mat = [
        [0.9869929, -0.1470543, 0.1599627],
        [0.4323053, 0.5183603, 0.0492912],
        [-0.0085287, 0.0400428, 0.9684867],
    ];
    let src = mat_apply(&xyz_to_lms, w_xyz);
    let dst = mat_apply(&xyz_to_lms, w_d50);
    let scale: Mat = [
        [dst[0] / src[0], 0.0, 0.0],
        [0.0, dst[1] / src[1], 0.0],
        [0.0, 0.0, dst[2] / src[2]],
    ];
    let m = mat_concat(&scale, &xyz_to_lms);
    mat_concat(&lms_to_xyz, &m)
}

/// `skcms_PrimariesToXYZD50`.
#[allow(clippy::too_many_arguments)]
fn primaries_to_xyz_d50(
    rx: f32,
    ry: f32,
    gx: f32,
    gy: f32,
    bx: f32,
    by: f32,
    wx: f32,
    wy: f32,
) -> Mat {
    let primaries: Mat = [
        [rx, gx, bx],
        [ry, gy, by],
        [1.0 - rx - ry, 1.0 - gx - gy, 1.0 - bx - by],
    ];
    let inv = mat_invert(&primaries).unwrap_or(GAMUT_XYZ);
    let w_xyz = [wx / wy, 1.0, (1.0 - wx - wy) / wy];
    let xyz = mat_apply(&inv, w_xyz);
    let to_xyz = mat_concat(
        &primaries,
        &[[xyz[0], 0.0, 0.0], [0.0, xyz[1], 0.0], [0.0, 0.0, xyz[2]]],
    );
    mat_concat(&adapt_to_xyz_d50(wx, wy), &to_xyz)
}

fn d65_to_d50() -> Mat {
    adapt_to_xyz_d50(0.3127, 0.3290)
}

fn prophoto_gamut() -> Mat {
    primaries_to_xyz_d50(
        0.7347, 0.2653, 0.1596, 0.8404, 0.0366, 0.0001, 0.34567, 0.35850,
    )
}

/// skcms_ext's `TransferFunction_eval` (`std::pow`).
fn tf_eval_ext(f: &Tf, x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0f32 } else { 1.0 };
    let ax = x.abs();
    if ax <= f.d {
        return sign * (f.c * ax + f.f);
    }
    let base = (f.a * ax + f.b).max(0.0);
    if f.g == 1.0 {
        return sign * (base + f.e);
    }
    sign * (base.powf(f.g) + f.e)
}

/// skcms_ext's `TransferFunction_invert` (`std::pow`).
fn tf_invert_ext(src: &Tf) -> Option<Tf> {
    let mut inv = Tf {
        g: 0.0,
        a: 0.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 0.0,
        f: 0.0,
    };
    let d_l = src.c * src.d + src.f;
    let d_r = (src.a * src.d + src.b).powf(src.g) + src.e;
    if (d_l - d_r).abs() > 1.0 / 512.0 {
        return None;
    }
    inv.d = d_l;
    if inv.d > 0.0 {
        inv.c = 1.0 / src.c;
        inv.f = -src.f / src.c;
    }
    let k = src.a.powf(-src.g);
    inv.g = 1.0 / src.g;
    inv.a = k;
    inv.b = -k * src.e;
    inv.e = -src.b / src.a;
    if inv.a < 0.0 {
        return None;
    }
    if inv.a * inv.d + inv.b < 0.0 {
        inv.b = -inv.a * inv.d;
    }
    let s = tf_eval_ext(src, 1.0);
    if !s.is_finite() {
        return None;
    }
    let sign = if s < 0.0 { -1.0f32 } else { 1.0 };
    let s = s * sign;
    if s < inv.d {
        inv.f = 1.0 - sign * inv.c * s;
    } else {
        inv.e = 1.0 - sign * (inv.a * s + inv.b).powf(inv.g);
    }
    Some(inv)
}

fn tf_apply(f: &Tf, v: Vec3) -> Vec3 {
    [
        tf_eval_ext(f, v[0]),
        tf_eval_ext(f, v[1]),
        tf_eval_ext(f, v[2]),
    ]
}

fn tf_apply_inverse(f: &Tf, v: Vec3) -> Vec3 {
    if *f == TF_LINEAR {
        return v;
    }
    let inv = tf_invert_ext(f).unwrap_or(TF_LINEAR);
    tf_apply(&inv, v)
}

// skcms's own approximations (`skcms_TransferFunction_eval`), which
// `Color::ToSRGB` uses for `srgb-linear`.
fn log2f_(x: f32) -> f32 {
    let bits = x.to_bits() as i32;
    let e = bits as f32 * (1.0 / (1 << 23) as f32);
    let m = f32::from_bits(((bits & 0x007f_ffff) | 0x3f00_0000) as u32);
    e - 124.225_52 - 1.498_030_3 * m - 1.725_88 / (0.352_088_7 + m)
}

fn floorf_(x: f32) -> f32 {
    let r = (x as i32) as f32;
    if r > x { r - 1.0 } else { r }
}

fn exp2f_(x: f32) -> f32 {
    if x > 128.0 {
        return f32::INFINITY;
    } else if x < -127.0 {
        return 0.0;
    }
    let fract = x - floorf_(x);
    let fbits = (1u32 << 23) as f32
        * (x + 121.274_06 - 1.490_129_1 * fract + 27.728_023 / (4.842_525_7 - fract));
    if fbits >= i32::MAX as f32 {
        return f32::INFINITY;
    } else if fbits < 0.0 {
        return 0.0;
    }
    f32::from_bits(fbits as i32 as u32)
}

fn powf_(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    exp2f_(log2f_(x) * y)
}

fn skcms_eval(f: &Tf, x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0f32 } else { 1.0 };
    let x = x * sign;
    sign * if x < f.d {
        f.c * x + f.f
    } else {
        powf_(f.a * x + f.b, f.g) + f.e
    }
}

fn skcms_invert(src: &Tf) -> Option<Tf> {
    let mut inv = Tf {
        g: 0.0,
        a: 0.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 0.0,
        f: 0.0,
    };
    let d_l = src.c * src.d + src.f;
    let d_r = powf_(src.a * src.d + src.b, src.g) + src.e;
    if (d_l - d_r).abs() > 1.0 / 512.0 {
        return None;
    }
    inv.d = d_l;
    if inv.d > 0.0 {
        inv.c = 1.0 / src.c;
        inv.f = -src.f / src.c;
    }
    let k = powf_(src.a, -src.g);
    inv.g = 1.0 / src.g;
    inv.a = k;
    inv.b = -k * src.e;
    inv.e = -src.b / src.a;
    if inv.a < 0.0 {
        return None;
    }
    if inv.a * inv.d + inv.b < 0.0 {
        inv.b = -inv.a * inv.d;
    }
    let s = skcms_eval(src, 1.0);
    if !s.is_finite() {
        return None;
    }
    let sign = if s < 0.0 { -1.0f32 } else { 1.0 };
    let s = s * sign;
    if s < inv.d {
        inv.f = 1.0 - sign * inv.c * s;
    } else {
        inv.e = 1.0 - sign * powf_(inv.a * s + inv.b, inv.g);
    }
    Some(inv)
}

// ---------------------------------------------------------------------
// gfx color conversions

const D50: Vec3 = [0.9642, 1.0, 0.8251];

fn lab_to_xyz_d50(l: f32, a: f32, b: f32) -> Vec3 {
    let y = (l + 16.0) / 116.0;
    let x = y + a / 500.0;
    let z = y - b / 200.0;
    let f = |t: f32| {
        let delta = 24.0f32 / 116.0;
        if t <= delta {
            (108.0f32 / 841.0) * (t - (16.0f32 / 116.0))
        } else {
            t * t * t
        }
    };
    [f(x) * D50[0], f(y) * D50[1], f(z) * D50[2]]
}

fn xyz_d50_to_lab(x: f32, y: f32, z: f32) -> Vec3 {
    let f = |t: f32| {
        let limit = (24.0f32 / 116.0) * (24.0f32 / 116.0) * (24.0f32 / 116.0);
        if t <= limit {
            (841.0f32 / 108.0) * t + (16.0f32 / 116.0)
        } else {
            t.powf(1.0 / 3.0)
        }
    };
    let (x, y, z) = (f(x / D50[0]), f(y / D50[1]), f(z / D50[2]));
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

const XYZ_D65_TO_LMS: Mat = [
    [0.819_022_44, 0.361_906_25, -0.128_873_78],
    [0.032_983_667, 0.929_286_85, 0.036_144_668],
    [0.048_177_2, 0.264_239_53, 0.633_547_83],
];
const OKLAB_TO_LMS: Mat = [
    [1.0, 0.396_337_8, 0.215_803_76],
    [1.0, -0.105_561_34, -0.063_854_17],
    [1.000_000_1, -0.089_484_18, -1.291_485_5],
];

fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn oklab_gamut_map(l: f32, a: f32, b: f32) -> Vec3 {
    let normal_r = [0.409702f32, -0.912219];
    let normal_m = [-0.397919f32, -0.917421];
    let normal_b = [-0.906800f32, 0.421562];
    let normal_c = [-0.171122f32, 0.985250];
    let normal_g = [0.460276f32, 0.887776];
    let normal_y = [0.947925f32, 0.318495];
    let (c0_yr, cw_yr, ck_yr) = (
        0.091132f32,
        [0.070370f32, 0.034139],
        [0.018170f32, 0.378550],
    );
    let (c0_rm, cw_rm, ck_rm) = (
        0.113902f32,
        [0.090836f32, 0.036251],
        [0.226781f32, 0.018764],
    );
    let (c0_mb, cw_mb, ck_mb) = (
        0.161739f32,
        [-0.008202f32, -0.264819],
        [0.187156f32, -0.284304],
    );
    let (c0_bc, cw_bc, ck_bc) = (
        0.102047f32,
        [-0.014804f32, -0.162608],
        [-0.276786f32, 0.004193],
    );
    let (c0_cg, cw_cg, ck_cg) = (
        0.092029f32,
        [-0.038533f32, -0.001650],
        [-0.232572f32, -0.094331],
    );
    let (c0_gy, cw_gy, ck_gy) = (
        0.081709f32,
        [-0.034601f32, -0.002215],
        [0.012185f32, 0.338031],
    );
    let one_minus_l = (1.0f64 - f64::from(l)) as f32;
    let ab = [a, b];
    let (c0, cw, ck) = if dot2(ab, normal_r) < 0.0 {
        if dot2(ab, normal_g) < 0.0 {
            if dot2(ab, normal_c) < 0.0 {
                (c0_bc, cw_bc, ck_bc)
            } else {
                (c0_cg, cw_cg, ck_cg)
            }
        } else if dot2(ab, normal_y) < 0.0 {
            (c0_gy, cw_gy, ck_gy)
        } else {
            (c0_yr, cw_yr, ck_yr)
        }
    } else if dot2(ab, normal_b) < 0.0 {
        if dot2(ab, normal_m) < 0.0 {
            (c0_rm, cw_rm, ck_rm)
        } else {
            (c0_mb, cw_mb, ck_mb)
        }
    } else {
        (c0_bc, cw_bc, ck_bc)
    };
    let mut alpha = 1.0f32;
    let w_denom = dot2(cw, ab);
    if w_denom > 0.0 {
        let w_num = c0 * one_minus_l;
        if w_num < w_denom {
            alpha = alpha.min(w_num / w_denom);
        }
    }
    let k_denom = dot2(ck, ab);
    if k_denom > 0.0 {
        let k_num = c0 * l;
        if k_num < k_denom {
            alpha = alpha.min(k_num / k_denom);
        }
    }
    [l, alpha * a, alpha * b]
}

fn oklab_to_xyz_d50(l: f32, a: f32, b: f32, gamut_map: bool) -> Vec3 {
    let [l, a, b] = if gamut_map {
        oklab_gamut_map(l, a, b)
    } else {
        [l, a, b]
    };
    let lms = mat_apply(&OKLAB_TO_LMS, [l, a, b]);
    let cubed = [
        lms[0] * lms[0] * lms[0],
        lms[1] * lms[1] * lms[1],
        lms[2] * lms[2] * lms[2],
    ];
    let d65 = mat_apply_inverse(&XYZ_D65_TO_LMS, cubed);
    mat_apply(&d65_to_d50(), d65)
}

fn pow_ext(x: f32, p: f32) -> f32 {
    if x < 0.0 { -(-x).powf(p) } else { x.powf(p) }
}

fn xyz_d50_to_oklab(x: f32, y: f32, z: f32) -> Vec3 {
    let d65 = mat_apply_inverse(&d65_to_d50(), [x, y, z]);
    let cubed = mat_apply(&XYZ_D65_TO_LMS, d65);
    let lms = [
        pow_ext(cubed[0], 1.0 / 3.0),
        pow_ext(cubed[1], 1.0 / 3.0),
        pow_ext(cubed[2], 1.0 / 3.0),
    ];
    mat_apply_inverse(&OKLAB_TO_LMS, lms)
}

fn lch_to_lab(l: f32, c: f32, h: f32) -> Vec3 {
    let r = (f64::from(h) * std::f64::consts::PI / 180.0) as f32;
    [l, c * r.cos(), c * r.sin()]
}

fn lab_to_lch(l: f32, a: f32, b: f32) -> Vec3 {
    [
        l,
        (a * a + b * b).sqrt(),
        (f64::from(b.atan2(a)) * 180.0 / std::f64::consts::PI) as f32,
    ]
}

fn srgb_to_xyz_d50(v: Vec3) -> Vec3 {
    mat_apply(&GAMUT_SRGB, tf_apply(&TF_SRGB, v))
}

fn xyz_d50_to_srgb(v: Vec3) -> Vec3 {
    tf_apply_inverse(&TF_SRGB, mat_apply_inverse(&GAMUT_SRGB, v))
}

fn hsl_to_srgb(h: f32, s: f32, l: f32) -> Vec3 {
    if s == 0.0 {
        return [l, l, l];
    }
    let f = |n: f32| {
        let k = ((f64::from(n + h / 30.0)) % 12.0) as f32;
        let a = s * l.min(1.0 - l);
        l - a * (-1.0f32).max((k - 3.0).min(9.0 - k).min(1.0))
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn srgb_to_hsl(r: f32, g: f32, b: f32) -> Vec3 {
    let (min, max) = (r.min(g).min(b), r.max(g).max(b));
    let (mut hue, mut sat) = (0.0f32, 0.0f32);
    let light = min + (max - min) / 2.0;
    let d = max - min;
    if d != 0.0 {
        sat = if light == 0.0 || light == 1.0 {
            0.0
        } else {
            (max - light) / light.min(1.0 - light)
        };
        hue = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hue *= 60.0;
    }
    if sat < 0.0 {
        hue += 180.0;
        sat = sat.abs();
    }
    if hue >= 360.0 {
        hue -= 360.0;
    }
    [hue, sat, light]
}

fn hwb_to_srgb(h: f32, w: f32, b: f32) -> Vec3 {
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray, gray, gray];
    }
    let [r, g, bl] = hsl_to_srgb(h, 1.0, 0.5);
    [
        r + (w - (w + b) * r),
        g + (w - (w + b) * g),
        bl + (w - (w + b) * bl),
    ]
}

fn srgb_to_hwb(r: f32, g: f32, b: f32) -> Vec3 {
    let (min, max) = (r.min(g).min(b), r.max(g).max(b));
    let mut hue = 0.0f32;
    let d = max - min;
    if d != 0.0 {
        hue = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hue *= 60.0;
    }
    if hue >= 360.0 {
        hue -= 360.0;
    }
    [hue, min, 1.0 - max]
}

// ---------------------------------------------------------------------
// Blink's Color

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    SrgbLegacy,
    Srgb,
    SrgbLinear,
    DisplayP3,
    A98,
    ProPhoto,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Oklab,
    Lch,
    Oklch,
    Hsl,
    Hwb,
}

impl Space {
    fn hue_index(self) -> Option<usize> {
        match self {
            Space::Hsl | Space::Hwb => Some(0),
            Space::Lch | Space::Oklch => Some(2),
            _ => None,
        }
    }

    fn rgb_or_xyz(self) -> bool {
        matches!(
            self,
            Space::Srgb
                | Space::SrgbLinear
                | Space::DisplayP3
                | Space::A98
                | Space::ProPhoto
                | Space::Rec2020
                | Space::XyzD50
                | Space::XyzD65
                | Space::SrgbLegacy
        )
    }

    fn params(self) -> Option<(Mat, Tf)> {
        Some(match self {
            Space::Srgb => (GAMUT_SRGB, TF_SRGB),
            Space::SrgbLinear => (GAMUT_SRGB, TF_LINEAR),
            Space::DisplayP3 => (GAMUT_P3, TF_SRGB),
            Space::A98 => (GAMUT_ADOBE, TF_2DOT2),
            Space::ProPhoto => (prophoto_gamut(), TF_PROPHOTO),
            Space::Rec2020 => (GAMUT_REC2020, TF_REC2020),
            Space::XyzD50 => (GAMUT_XYZ, TF_LINEAR),
            Space::XyzD65 => (d65_to_d50(), TF_LINEAR),
            _ => return None,
        })
    }

    /// `ColorFunction::kMetadataMap`: percentage bases (NaN: none).
    fn percentage(self) -> [f64; 3] {
        let na = f64::NAN;
        match self {
            Space::SrgbLegacy => [255.0, 255.0, 255.0],
            Space::Lab => [100.0, 125.0, 125.0],
            Space::Oklab => [1.0, 0.4, 0.4],
            Space::Lch => [100.0, 150.0, na],
            Space::Oklch => [1.0, 0.4, na],
            Space::Hsl | Space::Hwb => [na, 100.0, 100.0],
            _ => [1.0, 1.0, 1.0],
        }
    }

    /// Channel keywords of relative colors.
    fn channel_names(self) -> [&'static str; 3] {
        match self {
            Space::SrgbLegacy
            | Space::Srgb
            | Space::SrgbLinear
            | Space::DisplayP3
            | Space::A98
            | Space::ProPhoto
            | Space::Rec2020 => ["r", "g", "b"],
            Space::XyzD50 | Space::XyzD65 => ["x", "y", "z"],
            Space::Lab | Space::Oklab => ["l", "a", "b"],
            Space::Lch | Space::Oklch => ["l", "c", "h"],
            Space::Hsl => ["h", "s", "l"],
            Space::Hwb => ["h", "w", "b"],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Color {
    space: Space,
    p: [f32; 3],
    none: [bool; 3],
    alpha: f32,
    alpha_none: bool,
}

const ACHROMATIC: f32 = 0.02;

fn unit_circle(angle: f32) -> f32 {
    ((angle % 360.0) + 360.0) % 360.0
}

impl Color {
    /// `Color::FromColorSpace`.
    fn from_space(space: Space, p: [Option<f32>; 3], alpha: Option<f32>) -> Color {
        let mut c = Color {
            space,
            p: [
                p[0].unwrap_or(0.0),
                p[1].unwrap_or(0.0),
                p[2].unwrap_or(0.0),
            ],
            none: [p[0].is_none(), p[1].is_none(), p[2].is_none()],
            alpha: alpha.map_or(0.0, |a| a.clamp(0.0, 1.0)),
            alpha_none: alpha.is_none(),
        };
        if matches!(space, Space::Lab | Space::Lch | Space::Oklab | Space::Oklch)
            && !c.p[0].is_nan()
        {
            c.p[0] = if matches!(space, Space::Lab | Space::Lch) {
                c.p[0].max(0.0).min(100.0)
            } else {
                c.p[0].max(0.0).min(1.0)
            };
        }
        if matches!(space, Space::Lch | Space::Oklch) {
            c.p[1] = c.p[1].max(0.0);
        }
        c
    }

    /// `Color::FromRGBA` (legacy 8-bit colors).
    fn legacy(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color::from_space(
            Space::SrgbLegacy,
            [Some(f32::from(r)), Some(f32::from(g)), Some(f32::from(b))],
            Some(a),
        )
    }

    fn to_xyz_d50(&self, gamut_map: bool) -> Vec3 {
        let [p0, p1, p2] = self.p;
        match self.space {
            Space::SrgbLegacy => srgb_to_xyz_d50([p0 / 255.0, p1 / 255.0, p2 / 255.0]),
            Space::Lab => lab_to_xyz_d50(p0, p1, p2),
            Space::Oklab => oklab_to_xyz_d50(p0, p1, p2, gamut_map),
            Space::Lch => {
                let [l, a, b] = lch_to_lab(p0, p1, p2);
                lab_to_xyz_d50(l, a, b)
            }
            Space::Oklch => {
                let [l, a, b] = lch_to_lab(p0, p1, p2);
                oklab_to_xyz_d50(l, a, b, gamut_map)
            }
            Space::Hsl => srgb_to_xyz_d50(hsl_to_srgb(p0, p1, p2)),
            Space::Hwb => srgb_to_xyz_d50(hwb_to_srgb(p0, p1, p2)),
            space => {
                let (m, t) = space.params().expect("rgb or xyz");
                mat_apply(&m, tf_apply(&t, self.p))
            }
        }
    }

    /// `Color::ToSRGB`.
    fn to_srgb(&self, gamut_map: bool) -> Vec3 {
        let [p0, p1, p2] = self.p;
        match self.space {
            Space::Srgb => self.p,
            Space::SrgbLegacy => [p0 / 255.0, p1 / 255.0, p2 / 255.0],
            Space::SrgbLinear => {
                let inv = skcms_invert(&TF_SRGB).unwrap_or(TF_LINEAR);
                [
                    skcms_eval(&inv, p0),
                    skcms_eval(&inv, p1),
                    skcms_eval(&inv, p2),
                ]
            }
            Space::Hsl => hsl_to_srgb(p0, p1, p2),
            Space::Hwb => hwb_to_srgb(p0, p1, p2),
            _ => xyz_d50_to_srgb(self.to_xyz_d50(gamut_map)),
        }
    }

    fn resolve_missing(&mut self) {
        for i in 0..3 {
            if self.none[i] {
                self.p[i] = 0.0;
                self.none[i] = false;
            }
        }
    }

    /// `Color::ConvertToColorSpace`.
    fn convert(&mut self, to: Space) {
        if self.space == to {
            return;
        }
        self.resolve_missing();
        match to {
            Space::Lab => {
                self.p = if self.space == Space::Lch {
                    lch_to_lab(self.p[0], self.p[1], self.p[2])
                } else {
                    let [x, y, z] = self.to_xyz_d50(false);
                    xyz_d50_to_lab(x, y, z)
                };
            }
            Space::Oklab => {
                if self.space == Space::Oklch {
                    self.p = lch_to_lab(self.p[0], self.p[1], self.p[2]);
                } else {
                    let [x, y, z] = self.to_xyz_d50(false);
                    self.p = xyz_d50_to_oklab(x, y, z);
                }
            }
            Space::Lch => {
                let [l, a, b] = if self.space == Space::Lab {
                    self.p
                } else {
                    let [x, y, z] = self.to_xyz_d50(false);
                    xyz_d50_to_lab(x, y, z)
                };
                self.p = lab_to_lch(l, a, b);
                self.p[2] = unit_circle(self.p[2]);
                if self.p[1] <= ACHROMATIC {
                    self.none[2] = true;
                }
            }
            Space::Oklch => {
                if self.space == Space::Oklab {
                    self.p = lab_to_lch(self.p[0], self.p[1], self.p[2]);
                } else {
                    let [x, y, z] = self.to_xyz_d50(false);
                    let [l, a, b] = xyz_d50_to_oklab(x, y, z);
                    self.p = lab_to_lch(l, a, b);
                    self.p[2] = unit_circle(self.p[2]);
                }
                if self.p[1] <= ACHROMATIC {
                    self.none[2] = true;
                }
            }
            Space::Srgb => self.p = self.to_srgb(false),
            Space::SrgbLegacy => {
                let [r, g, b] = self.to_srgb(false);
                self.p = [
                    (f64::from(r) * 255.0) as f32,
                    (f64::from(g) * 255.0) as f32,
                    (f64::from(b) * 255.0) as f32,
                ];
            }
            Space::Hsl => {
                let [r, g, b] = self.to_srgb(false);
                self.p = srgb_to_hsl(r, g, b);
                if self.p[1] == 0.0 {
                    self.none[0] = true;
                }
            }
            Space::Hwb => {
                let [r, g, b] = self.to_srgb(false);
                self.p = srgb_to_hwb(r, g, b);
                if self.p[1] + self.p[2] >= 1.0 {
                    self.none[0] = true;
                }
            }
            to => {
                let (m, t) = to.params().expect("rgb or xyz");
                let xyz = self.to_xyz_d50(false);
                self.p = tf_apply_inverse(&t, mat_apply_inverse(&m, xyz));
            }
        }
        self.space = to;
    }

    /// `Color::GetAnalogousMissingComponents`.
    fn analogous_missing(&self, to: Space) -> [bool; 3] {
        let [n0, n1, n2] = self.none;
        if n0 && n1 && n2 {
            return [n0, n1, n2];
        }
        let is_lab = |s: Space| matches!(s, Space::Lab | Space::Oklab);
        let is_lch = |s: Space| matches!(s, Space::Lch | Space::Oklch);
        match self.space {
            s if s.rgb_or_xyz() => {
                if to.rgb_or_xyz() {
                    return [n0, n1, n2];
                }
            }
            Space::Lab | Space::Oklab => {
                if is_lab(to) {
                    return [n0, n1, n2];
                }
                if is_lch(to) {
                    let set = n1 && n2;
                    return [n0, set, set];
                }
                if to == Space::Hsl {
                    let set = n1 && n2;
                    return [set, set, n0];
                }
            }
            Space::Lch | Space::Oklch => {
                if is_lch(to) {
                    return [n0, n1, n2];
                }
                if to == Space::Hsl {
                    return [n2, n1, n0];
                }
                if to == Space::Hwb {
                    let set = n0 && n1;
                    return [n2, set, set];
                }
                if is_lab(to) {
                    let set = n1 && n2;
                    return [n0, set, set];
                }
            }
            Space::Hsl => {
                if is_lch(to) {
                    return [n2, n1, n0];
                }
                if is_lab(to) {
                    let set = n0 && n1;
                    return [n2, set, set];
                }
                if to == Space::Hwb {
                    let set = n1 && n2;
                    return [n0, set, set];
                }
            }
            Space::Hwb => {
                if to == Space::Hsl {
                    let set = n1 && n2;
                    return [n0, set, set];
                }
                if is_lch(to) {
                    let set = n1 && n2;
                    return [set, set, n0];
                }
            }
            _ => {}
        }
        [false, false, false]
    }

    fn convert_for_interpolation(&mut self, to: Space) {
        if self.space == to {
            return;
        }
        let analogous = self.analogous_missing(to);
        self.convert(to);
        for (i, &m) in analogous.iter().enumerate() {
            if m {
                self.p[i] = 0.0;
                self.none[i] = true;
            }
        }
    }

    fn premultiply(&mut self) -> f32 {
        if self.alpha_none {
            return self.alpha;
        }
        let hue = self.space.hue_index();
        let alpha = self.alpha;
        if hue != Some(0) {
            self.p[0] *= alpha;
        }
        self.p[1] *= alpha;
        if hue != Some(2) {
            self.p[2] *= alpha;
        }
        self.alpha = 1.0;
        alpha
    }

    fn unpremultiply(&mut self) {
        if self.alpha_none || self.alpha == 0.0 {
            return;
        }
        let hue = self.space.hue_index();
        if hue != Some(0) {
            self.p[0] /= self.alpha;
        }
        self.p[1] /= self.alpha;
        if hue != Some(2) {
            self.p[2] /= self.alpha;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum HueMethod {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

fn blend(from: f32, to: f32, progress: f64) -> f32 {
    (f64::from(from) + f64::from(to - from) * progress) as f32
}

fn hue_interpolation(mut v1: f32, mut v2: f32, percentage: f32, method: HueMethod) -> f32 {
    match method {
        HueMethod::Shorter => {
            let diff = v2 - v1;
            if diff > 180.0 {
                v1 += 360.0;
            } else if diff < -180.0 {
                v2 += 360.0;
            }
        }
        HueMethod::Longer => {
            let diff = v2 - v1;
            if diff > 0.0 && diff < 180.0 {
                v1 += 360.0;
            } else if diff > -180.0 && diff <= 0.0 {
                v2 += 360.0;
            }
        }
        HueMethod::Increasing => {
            if v2 < v1 {
                v2 += 360.0;
            }
        }
        HueMethod::Decreasing => {
            if v1 < v2 {
                v1 += 360.0;
            }
        }
    }
    unit_circle(blend(v1, v2, f64::from(percentage)))
}

/// `Color::InterpolateColors` and `FromColorMix`.
fn mix(
    space: Space,
    method: HueMethod,
    mut c1: Color,
    mut c2: Color,
    percentage: f32,
    alpha_multiplier: f32,
) -> Color {
    c1.convert_for_interpolation(space);
    c2.convert_for_interpolation(space);
    // `SubstituteMissingParameters`.
    for i in 0..3 {
        if c1.none[i] && !c2.none[i] {
            c1.p[i] = c2.p[i];
            c1.none[i] = false;
        } else if c2.none[i] && !c1.none[i] {
            c2.p[i] = c1.p[i];
            c2.none[i] = false;
        }
    }
    if c1.alpha_none && !c2.alpha_none {
        c1.alpha = c2.alpha;
        c1.alpha_none = false;
    } else if c2.alpha_none && !c1.alpha_none {
        c2.alpha = c1.alpha;
        c2.alpha_none = false;
    }
    let a1 = c1.premultiply();
    let a2 = c2.premultiply();
    let hue = space.hue_index();
    let pf = f64::from(percentage);
    let param = |i: usize| {
        if c1.none[i] && c2.none[i] {
            None
        } else if hue == Some(i) {
            Some(hue_interpolation(c1.p[i], c2.p[i], percentage, method))
        } else {
            Some(blend(c1.p[i], c2.p[i], pf))
        }
    };
    let alpha = if c1.alpha_none && c2.alpha_none {
        None
    } else {
        Some(blend(a1, a2, pf))
    };
    let mut out = Color::from_space(space, [param(0), param(1), param(2)], alpha);
    out.unpremultiply();
    out.alpha *= alpha_multiplier;
    out
}

// ---------------------------------------------------------------------
// Parsing

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Pct(f64),
    Dim(f64, String),
    Ident(String),
    Hash(String),
    Func(String, Vec<Tok>),
    Comma,
    Slash,
    Delim(char),
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

/// A CSS tokenizer for one color value (whitespace dropped; `+` and `-`
/// between operands are delimiters).
fn tokenize(s: &[char], i: &mut usize, close: bool) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    while *i < s.len() {
        let c = s[*i];
        if c.is_whitespace() {
            *i += 1;
            continue;
        }
        if c == ')' {
            if close {
                *i += 1;
                return Some(out);
            }
            return None;
        }
        if c == ',' {
            out.push(Tok::Comma);
            *i += 1;
            continue;
        }
        if c == '/' {
            out.push(Tok::Slash);
            *i += 1;
            continue;
        }
        if c == '*' {
            out.push(Tok::Delim('*'));
            *i += 1;
            continue;
        }
        let starts_number = |j: usize| -> bool {
            let d = |k: usize| s.get(k).is_some_and(|c| c.is_ascii_digit());
            match s.get(j) {
                Some('+') | Some('-') => d(j + 1) || (s.get(j + 1) == Some(&'.') && d(j + 2)),
                Some('.') => d(j + 1),
                Some(c) => c.is_ascii_digit(),
                None => false,
            }
        };
        if (c == '+' || c == '-')
            && !starts_number(*i)
            && !(c == '-'
                && s.get(*i + 1)
                    .is_some_and(|n| n.is_ascii_alphabetic() || *n == '-'))
        {
            out.push(Tok::Delim(c));
            *i += 1;
            continue;
        }
        if starts_number(*i) {
            let start = *i;
            if matches!(s[*i], '+' | '-') {
                *i += 1;
            }
            while *i < s.len() && s[*i].is_ascii_digit() {
                *i += 1;
            }
            if *i < s.len() && s[*i] == '.' && s.get(*i + 1).is_some_and(|c| c.is_ascii_digit()) {
                *i += 1;
                while *i < s.len() && s[*i].is_ascii_digit() {
                    *i += 1;
                }
            }
            if *i < s.len() && matches!(s[*i], 'e' | 'E') {
                let mut j = *i + 1;
                if matches!(s.get(j), Some('+') | Some('-')) {
                    j += 1;
                }
                if s.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    *i = j;
                    while *i < s.len() && s[*i].is_ascii_digit() {
                        *i += 1;
                    }
                }
            }
            let text: String = s[start..*i].iter().collect();
            let v: f64 = text.parse().ok()?;
            // Out-of-range numbers (`1e999`) are refused rather than
            // guessed at.
            if !v.is_finite() {
                return None;
            }
            if *i < s.len() && s[*i] == '%' {
                *i += 1;
                out.push(Tok::Pct(v));
            } else if *i < s.len() && (s[*i].is_ascii_alphabetic()) {
                let us = *i;
                while *i < s.len() && is_name(s[*i]) {
                    *i += 1;
                }
                out.push(Tok::Dim(
                    v,
                    s[us..*i].iter().collect::<String>().to_ascii_lowercase(),
                ));
            } else {
                out.push(Tok::Num(v));
            }
            continue;
        }
        if c == '#' {
            let start = *i + 1;
            *i += 1;
            while *i < s.len() && is_name(s[*i]) {
                *i += 1;
            }
            out.push(Tok::Hash(s[start..*i].iter().collect()));
            continue;
        }
        if is_name(c) {
            let start = *i;
            while *i < s.len() && is_name(s[*i]) {
                *i += 1;
            }
            let name: String = s[start..*i].iter().collect::<String>().to_ascii_lowercase();
            if *i < s.len() && s[*i] == '(' {
                *i += 1;
                let inner = tokenize(s, i, true)?;
                out.push(Tok::Func(name, inner));
            } else {
                out.push(Tok::Ident(name));
            }
            continue;
        }
        return None;
    }
    if close { None } else { Some(out) }
}

/// A `calc()` value: a number, a percentage, or an angle in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Calc {
    Number(f64),
    Percent(f64),
    Angle(f64),
}

fn angle_degrees(v: f64, unit: &str) -> Option<f64> {
    Some(match unit {
        "deg" => v,
        "rad" => v * 180.0 / std::f64::consts::PI,
        "grad" => v * 360.0 / 400.0,
        "turn" => v * 360.0,
        _ => return None,
    })
}

type Keywords<'a> = &'a [(&'a str, Option<f64>)];

/// Evaluates a math function's arguments (`calc`, `min`, `max`, `clamp`
/// with the four operators), channel keywords resolved through `keys`.
fn eval_calc(name: &str, args: &[Tok], keys: Keywords) -> Option<Calc> {
    match name {
        "calc" => eval_sum(args, keys),
        "min" | "max" => {
            let parts = split_commas(args);
            let vals: Option<Vec<Calc>> = parts.iter().map(|p| eval_sum(p, keys)).collect();
            let vals = vals?;
            let first = *vals.first()?;
            let mut best = first;
            for v in &vals[1..] {
                let (a, b) = (calc_value(best), calc_value(*v));
                if std::mem::discriminant(&best) != std::mem::discriminant(v) {
                    return None;
                }
                if (name == "min" && b < a) || (name == "max" && b > a) {
                    best = *v;
                }
            }
            Some(best)
        }
        "clamp" => {
            let parts = split_commas(args);
            if parts.len() != 3 {
                return None;
            }
            let lo = eval_sum(&parts[0], keys)?;
            let v = eval_sum(&parts[1], keys)?;
            let hi = eval_sum(&parts[2], keys)?;
            let r = calc_value(v).max(calc_value(lo)).min(calc_value(hi));
            Some(with_value(v, r))
        }
        _ => None,
    }
}

fn split_commas(args: &[Tok]) -> Vec<Vec<Tok>> {
    let mut out = vec![Vec::new()];
    for t in args {
        if *t == Tok::Comma {
            out.push(Vec::new());
        } else {
            out.last_mut().expect("non-empty").push(t.clone());
        }
    }
    out
}

fn calc_value(c: Calc) -> f64 {
    match c {
        Calc::Number(v) | Calc::Percent(v) | Calc::Angle(v) => v,
    }
}

fn with_value(c: Calc, v: f64) -> Calc {
    match c {
        Calc::Number(_) => Calc::Number(v),
        Calc::Percent(_) => Calc::Percent(v),
        Calc::Angle(_) => Calc::Angle(v),
    }
}

fn eval_sum(toks: &[Tok], keys: Keywords) -> Option<Calc> {
    // Split on top-level + and - (and signed numbers following an operand).
    let mut terms: Vec<(char, Vec<Tok>)> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    let mut sign = '+';
    for t in toks {
        match t {
            Tok::Delim(c @ ('+' | '-')) if !cur.is_empty() => {
                terms.push((sign, std::mem::take(&mut cur)));
                sign = *c;
            }
            Tok::Num(v)
                if !cur.is_empty() && !matches!(cur.last(), Some(Tok::Delim('*'))) && *v < 0.0 =>
            {
                terms.push((sign, std::mem::take(&mut cur)));
                sign = '-';
                cur.push(Tok::Num(-v));
            }
            _ => cur.push(t.clone()),
        }
    }
    terms.push((sign, cur));
    let mut acc: Option<Calc> = None;
    for (s, term) in terms {
        let v = eval_product(&term, keys)?;
        let v = if s == '-' {
            with_value(v, -calc_value(v))
        } else {
            v
        };
        acc = Some(match acc {
            None => v,
            Some(a) => {
                if std::mem::discriminant(&a) != std::mem::discriminant(&v) {
                    return None;
                }
                with_value(a, calc_value(a) + calc_value(v))
            }
        });
    }
    acc
}

fn eval_product(toks: &[Tok], keys: Keywords) -> Option<Calc> {
    let mut acc: Option<Calc> = None;
    let mut op = '*';
    for t in toks {
        if let Tok::Delim(c @ '*') = t {
            op = *c;
            continue;
        }
        if *t == Tok::Slash {
            op = '/';
            continue;
        }
        let v = eval_atom(t, keys)?;
        acc = Some(match acc {
            None => v,
            Some(a) => match (op, a, v) {
                ('*', a, Calc::Number(n)) => with_value(a, calc_value(a) * n),
                ('*', Calc::Number(n), b) => with_value(b, n * calc_value(b)),
                ('/', a, Calc::Number(n)) => with_value(a, calc_value(a) / n),
                _ => return None,
            },
        });
        op = '*';
    }
    acc
}

fn eval_atom(t: &Tok, keys: Keywords) -> Option<Calc> {
    match t {
        Tok::Num(v) => Some(Calc::Number(*v)),
        Tok::Pct(v) => Some(Calc::Percent(*v)),
        Tok::Dim(v, u) => angle_degrees(*v, u).map(Calc::Angle),
        Tok::Ident(name) => match name.as_str() {
            "pi" => Some(Calc::Number(std::f64::consts::PI)),
            "e" => Some(Calc::Number(std::f64::consts::E)),
            _ => keys
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| Calc::Number(v.unwrap_or(0.0))),
        },
        Tok::Func(name, args) if name == "calc" => eval_sum(args, keys),
        Tok::Func(name, args) => eval_calc(name, args, keys),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Percentage,
    None,
    Relative,
}

/// A parsed channel: its kind and value (degrees for hues).
type Channel = (Kind, f64);

fn channel(t: &Tok, hue: bool, keys: Option<Keywords>) -> Option<Channel> {
    match t {
        Tok::Ident(n) if n == "none" => Some((Kind::None, 0.0)),
        Tok::Num(v) => Some((Kind::Number, *v)),
        Tok::Dim(v, u) if hue => Some((Kind::Number, angle_degrees(*v, u)?)),
        Tok::Pct(v) if !hue => Some((Kind::Percentage, *v)),
        Tok::Func(name, args) if matches!(name.as_str(), "calc" | "min" | "max" | "clamp") => {
            let k = keys.unwrap_or(&[]);
            let v = eval_calc(name, args, k)?;
            if !calc_value(v).is_finite() {
                return None;
            }
            match v {
                Calc::Number(v) => Some((
                    if keys.is_some() {
                        Kind::Relative
                    } else {
                        Kind::Number
                    },
                    v,
                )),
                Calc::Angle(v) if hue => Some((
                    if keys.is_some() {
                        Kind::Relative
                    } else {
                        Kind::Number
                    },
                    v,
                )),
                Calc::Percent(v) if !hue => Some((
                    if keys.is_some() {
                        Kind::Relative
                    } else {
                        Kind::Percentage
                    },
                    v,
                )),
                _ => None,
            }
        }
        Tok::Ident(n) => {
            let keys = keys?;
            keys.iter().find(|(k, _)| k == n).map(|(_, v)| match v {
                Some(v) => (Kind::Relative, *v),
                None => (Kind::None, 0.0),
            })
        }
        _ => None,
    }
}

fn space_of_function(name: &str) -> Option<Space> {
    Some(match name {
        "rgb" | "rgba" => Space::SrgbLegacy,
        "hsl" | "hsla" => Space::Hsl,
        "hwb" => Space::Hwb,
        "lab" => Space::Lab,
        "lch" => Space::Lch,
        "oklab" => Space::Oklab,
        "oklch" => Space::Oklch,
        _ => return None,
    })
}

fn space_of_argument(name: &str) -> Option<Space> {
    Some(match name {
        "srgb" => Space::Srgb,
        "srgb-linear" => Space::SrgbLinear,
        "display-p3" => Space::DisplayP3,
        "a98-rgb" => Space::A98,
        "prophoto-rgb" => Space::ProPhoto,
        "rec2020" => Space::Rec2020,
        "xyz-d50" => Space::XyzD50,
        "xyz" | "xyz-d65" => Space::XyzD65,
        _ => return None,
    })
}

/// `MakePerColorSpaceAdjustments`.
fn adjust(
    relative: bool,
    legacy: bool,
    space: Space,
    ch: &mut [Option<f64>; 3],
    alpha: &mut Option<f64>,
) {
    if let Some(h) = space.hue_index()
        && let Some(v) = ch[h].as_mut()
    {
        if !v.is_finite() {
            *v = 0.0;
        }
        *v = ((*v % 360.0) + 360.0) % 360.0;
    }
    if space == Space::SrgbLegacy {
        for c in ch.iter_mut().flatten() {
            if !c.is_finite() {
                *c = if *c > 0.0 { 255.0 } else { 0.0 };
            } else if !relative {
                *c = c.clamp(0.0, 255.0);
            }
        }
        if legacy && let Some(a) = alpha.as_mut() {
            *a = (*a * 255.0).round() / 255.0;
        }
    }
    if matches!(space, Space::Hsl | Space::Hwb) {
        for c in ch[1..].iter_mut().flatten() {
            let mut v = *c / 100.0;
            if legacy {
                v = v.clamp(0.0, 1.0);
            } else if !relative {
                v = v.max(0.0);
            }
            *c = v;
        }
    }
}

/// What a color value resolves to. `currentcolor` is the canvas's color
/// (black: the raster canvas is never in the document).
fn color_value(t: &Tok) -> Option<Color> {
    match t {
        Tok::Hash(h) => super::color::hex_rgba(h).map(legacy_from_rgba),
        Tok::Ident(name) => match name.as_str() {
            "currentcolor" => Some(Color::legacy(0, 0, 0, 1.0)),
            "highlight" => Some(Color::legacy(128, 188, 254, 0.6)),
            other => system_color(other)
                .map(|[r, g, b]| Color::legacy(r, g, b, 1.0))
                .or_else(|| super::color::named_rgba(other).map(legacy_from_rgba)),
        },
        Tok::Func(name, args) => function(name, args),
        _ => None,
    }
}

fn legacy_from_rgba(c: Rgba) -> Color {
    let b = |v: f64| (v * 255.0).round() as u8;
    Color::legacy(b(c[0]), b(c[1]), b(c[2]), c[3] as f32)
}

/// System colors as a detached canvas resolves them (light scheme), the
/// ones measured on the closure capture.
fn system_color(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "canvastext" | "activeborder" | "buttonborder" | "buttontext" | "captiontext"
        | "fieldtext" | "highlighttext" | "inactiveborder" | "infotext" | "marktext"
        | "menutext" | "selecteditemtext" | "threeddarkshadow" | "threedhighlight"
        | "threedlightshadow" | "threedshadow" | "windowframe" | "windowtext" => [0, 0, 0],
        "canvas" | "activecaption" | "appworkspace" | "background" | "field"
        | "inactivecaption" | "infobackground" | "menu" | "scrollbar" | "window" => [255, 255, 255],
        "buttonface" | "buttonhighlight" | "buttonshadow" | "threedface" => [239, 239, 239],
        "linktext" => [0, 0, 238],
        "graytext" | "inactivecaptiontext" => [128, 128, 128],
        "activetext" => [255, 0, 0],
        "mark" => [255, 255, 0],
        "selecteditem" => [179, 215, 255],
        "visitedtext" => [85, 26, 139],
        _ => return None,
    })
}

fn function(name: &str, args: &[Tok]) -> Option<Color> {
    if name == "color-mix" {
        return color_mix(args);
    }
    let mut i = 0;
    let mut origin: Option<Color> = None;
    if args.first() == Some(&Tok::Ident("from".into())) {
        origin = Some(color_value(args.get(1)?)?);
        i = 2;
    }
    let space = if name == "color" {
        let Tok::Ident(s) = args.get(i)? else {
            return None;
        };
        i += 1;
        space_of_argument(s)?
    } else {
        space_of_function(name)?
    };
    // Relative colors: the origin in this space, its channels as keywords.
    let keys: Option<Vec<(&str, Option<f64>)>> = origin.map(|mut o| {
        o.convert_for_interpolation(space);
        let names = space.channel_names();
        let mut vals: Vec<Option<f64>> = (0..3)
            .map(|k| (!o.none[k]).then_some(f64::from(o.p[k])))
            .collect();
        if matches!(space, Space::Hsl | Space::Hwb) {
            for v in vals[1..].iter_mut().flatten() {
                *v = f64::from((*v as f32) * 100.0);
            }
        }
        vec![
            (names[0], vals[0]),
            (names[1], vals[1]),
            (names[2], vals[2]),
            ("alpha", (!o.alpha_none).then_some(f64::from(o.alpha))),
        ]
    });
    let keys_ref: Option<Keywords> = keys.as_deref();
    let hue = space.hue_index();
    let mut chans: Vec<Channel> = Vec::new();
    let mut commas = [false, false];
    for k in 0..3 {
        let c = channel(args.get(i)?, hue == Some(k), keys_ref)?;
        chans.push(c);
        i += 1;
        if k < 2 && args.get(i) == Some(&Tok::Comma) {
            commas[k] = true;
            i += 1;
        }
    }
    if commas[0] != commas[1] {
        return None;
    }
    let legacy = commas[0];
    let mut alpha: Option<Channel> = None;
    if legacy {
        if origin.is_some() || !matches!(space, Space::SrgbLegacy | Space::Hsl) {
            return None;
        }
        if args.get(i) == Some(&Tok::Comma) {
            i += 1;
            alpha = Some(alpha_channel(args.get(i)?, keys_ref)?);
            i += 1;
        }
    } else if args.get(i) == Some(&Tok::Slash) {
        i += 1;
        alpha = Some(alpha_channel(args.get(i)?, keys_ref)?);
        i += 1;
    }
    if i != args.len() {
        return None;
    }
    if legacy {
        if chans.iter().any(|c| c.0 == Kind::None) || alpha.is_some_and(|a| a.0 == Kind::None) {
            return None;
        }
        if space == Space::SrgbLegacy {
            let pct = chans.iter().filter(|c| c.0 == Kind::Percentage).count();
            if pct != 0 && pct != 3 {
                return None;
            }
        } else if chans[1..].iter().any(|c| c.0 == Kind::Number) {
            return None;
        }
    }
    let bases = space.percentage();
    let mut values: [Option<f64>; 3] = [None, None, None];
    for (k, &(kind, v)) in chans.iter().enumerate() {
        values[k] = match kind {
            Kind::None => None,
            Kind::Number | Kind::Relative => Some(v),
            Kind::Percentage => {
                if bases[k].is_nan() {
                    return None;
                }
                Some(v / 100.0 * bases[k])
            }
        };
    }
    let mut a: Option<f64> = match alpha {
        None => {
            if let Some(keys) = keys_ref {
                keys.iter()
                    .find(|(k, _)| *k == "alpha")
                    .and_then(|(_, v)| *v)
            } else {
                Some(1.0)
            }
        }
        Some((Kind::None, _)) => None,
        Some((Kind::Number, v)) => Some(v.clamp(0.0, 1.0)),
        Some((Kind::Percentage, v)) => Some((v / 100.0).clamp(0.0, 1.0)),
        Some((Kind::Relative, v)) => Some(v.clamp(0.0, 1.0)),
    };
    adjust(origin.is_some(), legacy, space, &mut values, &mut a);
    let mut c = Color::from_space(
        space,
        [
            values[0].map(|v| v as f32),
            values[1].map(|v| v as f32),
            values[2].map(|v| v as f32),
        ],
        a.map(|v| v as f32),
    );
    // A relative color in the legacy space resolves to `color(srgb …)`.
    if origin.is_some() && matches!(space, Space::SrgbLegacy | Space::Hsl | Space::Hwb) {
        c.convert(Space::Srgb);
    }
    Some(c)
}

fn alpha_channel(t: &Tok, keys: Option<Keywords>) -> Option<Channel> {
    match t {
        Tok::Ident(n) if n == "none" => Some((Kind::None, 0.0)),
        Tok::Num(v) => Some((Kind::Number, *v)),
        Tok::Pct(v) => Some((Kind::Percentage, *v)),
        Tok::Func(name, args) if matches!(name.as_str(), "calc" | "min" | "max" | "clamp") => {
            match eval_calc(name, args, keys.unwrap_or(&[]))? {
                Calc::Number(v) => Some((Kind::Number, v)),
                Calc::Percent(v) => Some((Kind::Percentage, v)),
                Calc::Angle(_) => None,
            }
        }
        Tok::Ident(n) => keys?.iter().find(|(k, _)| k == n).map(|(_, v)| match v {
            Some(v) => (Kind::Relative, *v),
            None => (Kind::None, 0.0),
        }),
        _ => None,
    }
}

fn color_mix(args: &[Tok]) -> Option<Color> {
    let mut i = 0;
    let mut space = Space::Oklab;
    let mut method = HueMethod::Shorter;
    if args.first() == Some(&Tok::Ident("in".into())) {
        let Tok::Ident(s) = args.get(1)? else {
            return None;
        };
        space = match s.as_str() {
            "xyz" | "xyz-d65" => Space::XyzD65,
            "xyz-d50" => Space::XyzD50,
            "srgb-linear" => Space::SrgbLinear,
            "display-p3" => Space::DisplayP3,
            "a98-rgb" => Space::A98,
            "prophoto-rgb" => Space::ProPhoto,
            "rec2020" => Space::Rec2020,
            "lab" => Space::Lab,
            "oklab" => Space::Oklab,
            "lch" => Space::Lch,
            "oklch" => Space::Oklch,
            "srgb" => Space::Srgb,
            "hsl" => Space::Hsl,
            "hwb" => Space::Hwb,
            _ => return None,
        };
        i = 2;
        if space.hue_index().is_some()
            && let Some(Tok::Ident(m)) = args.get(i)
        {
            let read = match m.as_str() {
                "shorter" => Some(HueMethod::Shorter),
                "longer" => Some(HueMethod::Longer),
                "increasing" => Some(HueMethod::Increasing),
                "decreasing" => Some(HueMethod::Decreasing),
                _ => None,
            };
            if let Some(read) = read {
                method = read;
                i += 1;
                if args.get(i) != Some(&Tok::Ident("hue".into())) {
                    return None;
                }
                i += 1;
            }
        }
        if args.get(i) != Some(&Tok::Comma) {
            return None;
        }
        i += 1;
    }
    let operand = |i: &mut usize| -> Option<(Color, Option<f64>)> {
        let mut color = None;
        let mut pct = None;
        for _ in 0..2 {
            match args.get(*i) {
                Some(Tok::Pct(v)) if pct.is_none() => {
                    if !(0.0..=100.0).contains(v) {
                        return None;
                    }
                    pct = Some(*v);
                    *i += 1;
                }
                Some(Tok::Comma) | None => break,
                Some(t) if color.is_none() => {
                    color = Some(color_value(t)?);
                    *i += 1;
                }
                _ => return None,
            }
        }
        Some((color?, pct))
    };
    let (c1, p1) = operand(&mut i)?;
    if args.get(i) != Some(&Tok::Comma) {
        return None;
    }
    i += 1;
    let (c2, p2) = operand(&mut i)?;
    if i != args.len() {
        return None;
    }
    // `CSSColorMixValue::NormalizePercentages`.
    let mut q1 = p1.map_or(0.5, |p| p.clamp(0.0, 100.0) / 100.0);
    let mut q2 = p2.map_or(0.5, |p| p.clamp(0.0, 100.0) / 100.0);
    if p1.is_some() && p2.is_none() {
        q2 = 1.0 - q1;
    } else if p2.is_some() && p1.is_none() {
        q1 = 1.0 - q2;
    }
    let sum = q1 + q2;
    let (amount, multiplier) = if sum == 0.0 {
        (0.5, 0.0)
    } else {
        (q2 / sum, if sum <= 1.0 { sum } else { 1.0 })
    };
    Some(mix(space, method, c1, c2, amount as f32, multiplier as f32))
}

/// Whether Chromium bakes gamut mapping into `toSkColor4f`
/// (`kBakedGamutMapping`); the closure capture's out-of-gamut `oklch()`
/// shows it does not.
const BAKED_GAMUT_MAPPING: bool = false;

/// A CSS Color 4/5 color as Skia paints it on the canvas: straight sRGB
/// (`Color::toSkColor4f`, unclamped) and alpha. `None` when the string is
/// not such a color.
pub fn parse(value: &str) -> Option<Rgba> {
    let chars: Vec<char> = value.trim().chars().collect();
    let mut i = 0;
    let toks = tokenize(&chars, &mut i, false)?;
    let [t] = toks.as_slice() else {
        return None;
    };
    let c = color_value(t)?;
    let [r, g, b] = c.to_srgb(BAKED_GAMUT_MAPPING);
    let a = if c.alpha_none { 0.0 } else { c.alpha };
    Some([f64::from(r), f64::from(g), f64::from(b), f64::from(a)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(s: &str) -> [i32; 4] {
        let c = parse(s).unwrap_or_else(|| panic!("{s} does not parse"));
        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        [q(c[0]), q(c[1]), q(c[2]), q(c[3])]
    }

    /// The closure capture's solid pixels (premultiplied there; these are
    /// opaque or compared straight).
    #[test]
    fn colors_match_the_closure_capture() {
        let cases: &[(&str, [i32; 3])] = &[
            ("lab(52.2345% 40.1645 59.9971)", [198, 93, 6]),
            ("lch(52.2345% 72.2 56.2)", [198, 93, 6]),
            ("oklab(59.686% 0.1009 0.1192)", [198, 93, 7]),
            ("oklch(59.69% 0.156 49.7)", [198, 93, 8]),
            ("oklch(70% 0.4 30)", [255, 0, 0]),
            ("lch(none 50 120)", [0, 16, 0]),
            ("oklch(80% 0.12 none)", [253, 156, 186]),
            ("color(srgb 0.5 0.2 0.9)", [128, 51, 229]),
            ("color(srgb-linear 0.2 0.5 0.1)", [124, 188, 89]),
            ("color(display-p3 1 0.5 0)", [255, 118, 0]),
            ("color(a98-rgb 0.8 0.2 0.3)", [237, 48, 76]),
            ("color(rec2020 0.7 0.3 0.2)", [225, 68, 58]),
            ("color(prophoto-rgb 0.6 0.4 0.2)", [211, 106, 49]),
            ("color(xyz-d65 0.3 0.2 0.1)", [206, 84, 81]),
            ("color(xyz-d50 0.3 0.2 0.1)", [198, 86, 96]),
            ("color(xyz 0.25 0.4 0.6)", [0, 193, 198]),
            ("color-mix(in srgb, #ff0000 30%, #0000ff)", [77, 0, 178]),
            ("color-mix(in oklch, red, blue)", [186, 0, 194]),
            ("color-mix(in lab, #34c759 40%, white)", [186, 234, 188]),
            ("color-mix(in hsl shorter hue, red, blue)", [255, 0, 255]),
            (
                "color-mix(in srgb-linear, #ff9500, #5856d6 75%)",
                [154, 106, 188],
            ),
            ("hsl(from red calc(h + 120) s l)", [0, 255, 0]),
            ("oklch(from #ff3b30 calc(l - 0.1) c h)", [218, 0, 0]),
            ("rgb(calc(255 / 2) 0 calc(100 + 20))", [128, 0, 120]),
            ("currentcolor", [0, 0, 0]),
            ("CanvasText", [0, 0, 0]),
            ("Canvas", [255, 255, 255]),
            ("LinkText", [0, 0, 238]),
            ("GrayText", [128, 128, 128]),
        ];
        let mut wrong = Vec::new();
        for (s, want) in cases {
            let got = bytes(s);
            if got[..3] != want[..] {
                wrong.push(format!("{s}: {got:?} want {want:?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
        // Translucent ones (straight alpha here).
        assert_eq!(bytes("lab(50% 0 0 / 0.5)")[3], 128);
        assert_eq!(bytes("color(display-p3 0 1 0 / 40%)")[3], 102);
        assert_eq!(bytes("color-mix(in oklab, red 20%, transparent)")[3], 51);
        assert_eq!(bytes("rgb(from #3478f6 r g b / 50%)")[3], 128);
    }

    #[test]
    fn strings_that_are_not_colors_do_not_parse() {
        for s in [
            "lab(50%)",
            "color(foo 1 2 3)",
            "rgb(from red r g)",
            "color-mix(in srgb, red)",
            "lab(1 2 3) x",
            "AccentColor",
            "oklch(1 2 3deg 4)",
        ] {
            assert!(parse(s).is_none(), "{s}");
        }
    }
}
