//! Skia's high-precision raster pipeline for RGB matrix/TRC readPixels.
//! Its fused multiply-adds and power approximation differ from skcms's
//! standalone transform, enough to change the final byte at gamut edges.
//!
//! Derived from Skia (Copyright Google LLC), BSD 3-Clause license;
//! Chromium 152 pin b6d106297ff9ef2ff8094033695d045e87775581,
//! src/opts/SkRasterPipeline_opts.h.
use crate::sys::skcms as s;

fn pow(x: f32, y: f32) -> f32 {
    if x == 0.0 || x == 1.0 {
        return x;
    }
    let e = x.to_bits() as f32 * (1.0 / (1 << 23) as f32);
    let m = f32::from_bits((x.to_bits() & 0x007fffff) | 0x3f000000);
    let log = (-m).mul_add(1.4980303, e - 124.22552) - 1.72588 / (0.35208872 + m);
    let x = log * y;
    let f = x - x.floor();
    let mut v = (-f).mul_add(1.4901291, x + 121.274055);
    v += 27.728024 / (4.8425255 - f);
    v *= (1 << 23) as f32;
    f32::from_bits(v.clamp(0.0, 0x7f800000u32 as f32).round_ties_even() as u32)
}

fn transfer(v: f32, t: s::skcms_TransferFunction) -> f32 {
    let x = v.abs();
    let r = if x <= t.d {
        t.c.mul_add(x, t.f)
    } else {
        pow(t.a.mul_add(x, t.b), t.g) + t.e
    };
    r.copysign(v)
}

/// Returns false for profiles SkColorSpace cannot express as matrix/TRC;
/// those retain the skcms ICC transform (CLUTs and distinct table curves).
pub(super) unsafe fn transform(
    profile: &s::skcms_ICCProfile,
    rgba: &[u8],
    out: &mut [f32],
) -> bool {
    unsafe {
        if !profile.has_toXYZD50
            || !profile.has_trc
            || profile.has_A2B
            || !profile.trc.iter().all(|c| {
                c.__bindgen_anon_2.table_entries == 0 && c.__bindgen_anon_1.parametric.g >= 0.0
            })
        {
            return false;
        }
        let destination = &*s::skcms_sRGB_profile();
        let mut inverse = s::skcms_Matrix3x3::default();
        if !s::skcms_Matrix3x3_invert(&destination.toXYZD50, &mut inverse) {
            return false;
        }
        let matrix = s::skcms_Matrix3x3_concat(&inverse, &profile.toXYZD50).vals;
        let encode = *s::skcms_sRGB_Inverse_TransferFunction();
        for (src, dst) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            let a = f32::from(src[3]) * (1.0 / 255.0);
            let scale = if a > 0.0 { 1.0 / a } else { 0.0 };
            let rgb = [0, 1, 2].map(|c| {
                transfer(
                    f32::from(src[c]) * (1.0 / 255.0) * scale,
                    profile.trc[c].__bindgen_anon_1.parametric,
                )
            });
            for (c, m) in matrix.iter().enumerate() {
                let v = m[0].mul_add(rgb[0], m[1].mul_add(rgb[1], m[2] * rgb[2]));
                dst[c] = transfer(v, encode) * a;
            }
            dst[3] = a;
        }
        true
    }
}
