//! HDR input tone mapping for Standard projects (plan section 9,
//! "Per-source interpretation"): HDR sources (iPhone HLG, HDR10, Dolby
//! Vision base layers) are mapped to SDR with ITU-R BT.2446-1 Method A.
//!
//! The pipeline per pixel, in f64 here and in f32 in [`WGSL`]:
//!
//! 1. The HDR signal (BT.2020 R′G′B′, PQ or HLG) to display light
//!    normalized to the content peak L_HDR: PQ through the ST 2084 EOTF
//!    divided by L_HDR; HLG through the inverse OETF and the BT.2100 OOTF
//!    for a 1000 cd/m² display (system gamma 1.2), which is already
//!    normalized to the display peak.
//! 2. BT.2446 Method A: R′G′B′ = RGB^(1/2.4); luma with BT.2020
//!    coefficients; the luma curve (ρ_HDR, the three-piece knee, ρ_SDR for
//!    100 cd/m²); the chroma scaling f = Y′_SDR / (1.1 Y′) and the luma
//!    correction Y′_TMO = Y′_SDR − max(0.1 C′r, 0).
//! 3. BT.2020 SDR R′G′B′ (gamma 2.4) to Rec.709: linearize with 2.4,
//!    convert primaries in linear light, re-encode with 1/2.4 (BT.2407's
//!    linear-light conversion, no clipping: out-of-gamut values stay
//!    negative or above 1 until output, as section 9 requires).
//!
//! The result is a Rec.709 SDR code value, which Standard reads exactly as
//! it reads a Rec.709 source: as display encoding E, passed through.
//! Values above L_HDR continue the curve's last piece rather than being
//! clipped (Method A is defined on 0..1; the continuation is monotonic).

use crate::primaries::{Mat3, apply, conversion};
use pm_protocol::color::Primaries;

/// BT.2020 luma coefficients (Kr, Kg, Kb).
pub const KR: f64 = 0.2627;
pub const KG: f64 = 0.6780;
pub const KB: f64 = 0.0593;

/// The SDR reference peak of Method A, cd/m².
pub const L_SDR: f64 = 100.0;

/// HLG's nominal display peak and system gamma (BT.2100 for 1000 cd/m²).
pub const HLG_PEAK: f64 = 1000.0;
pub const HLG_GAMMA: f64 = 1.2;

/// The HDR signal kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrSignal {
    Pq,
    Hlg,
}

/// SMPTE ST 2084 EOTF: signal 0..1 to cd/m² (0..10000).
pub fn pq_eotf(e: f64) -> f64 {
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = 2523.0 / 4096.0 * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 4096.0 * 32.0;
    const C3: f64 = 2392.0 / 4096.0 * 32.0;
    let p = e.max(0.0).powf(1.0 / M2);
    10000.0 * ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

/// SMPTE ST 2084 inverse EOTF: cd/m² to signal.
pub fn pq_inverse_eotf(nits: f64) -> f64 {
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = 2523.0 / 4096.0 * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 4096.0 * 32.0;
    const C3: f64 = 2392.0 / 4096.0 * 32.0;
    let y = (nits / 10000.0).max(0.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 1.0 - 4.0 * HLG_A;
// c = 0.5 − a·ln(4a)
const HLG_C: f64 = 0.559_910_73;

/// BT.2100 HLG inverse OETF: signal 0..1 to normalized scene light 0..1.
pub fn hlg_inverse_oetf(e: f64) -> f64 {
    let e = e.max(0.0);
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// BT.2100 HLG OETF: normalized scene light to signal.
pub fn hlg_oetf(l: f64) -> f64 {
    let l = l.max(0.0);
    if l <= 1.0 / 12.0 {
        (3.0 * l).sqrt()
    } else {
        HLG_A * (12.0 * l - HLG_B).ln() + HLG_C
    }
}

/// HDR signal R′G′B′ (BT.2020) to display light normalized to `peak`.
pub fn to_normalized_display(signal: HdrSignal, rgb: [f64; 3], peak: f64) -> [f64; 3] {
    match signal {
        HdrSignal::Pq => rgb.map(|c| pq_eotf(c) / peak),
        HdrSignal::Hlg => {
            // Scene light, then the OOTF F_D = L_W · Y_S^(γ−1) · E_S,
            // normalized by L_W (black level 0).
            let s = rgb.map(hlg_inverse_oetf);
            let ys = KR * s[0] + KG * s[1] + KB * s[2];
            let g = if ys > 0.0 {
                ys.powf(HLG_GAMMA - 1.0)
            } else {
                0.0
            };
            s.map(|c| c * g)
        }
    }
}

/// ρ for a peak luminance (BT.2446 eq. for ρ_HDR and ρ_SDR).
pub fn rho(peak: f64) -> f64 {
    1.0 + 32.0 * (peak / 10000.0).powf(1.0 / 2.4)
}

/// Method A's luma curve: HDR Y′ (0..1 for 0..L_HDR) to SDR Y′.
pub fn luma_curve(y: f64, peak: f64) -> f64 {
    let rh = rho(peak);
    let yp = (1.0 + (rh - 1.0) * y.max(0.0)).ln() / rh.ln();
    let yc = if yp <= 0.7399 {
        1.0770 * yp
    } else if yp < 0.9909 {
        -1.1510 * yp * yp + 2.7811 * yp - 0.6302
    } else {
        0.5 * yp + 0.5
    };
    let rs = rho(L_SDR);
    (rs.powf(yc) - 1.0) / (rs - 1.0)
}

/// Method A on normalized display light (BT.2020, 1.0 = L_HDR): the SDR
/// BT.2020 R′G′B′ for a gamma 2.4 display.
pub fn method_a(rgb: [f64; 3], peak: f64) -> [f64; 3] {
    let p = rgb.map(|c| c.max(0.0).powf(1.0 / 2.4));
    let y = KR * p[0] + KG * p[1] + KB * p[2];
    if y <= 0.0 {
        return [0.0; 3];
    }
    let cb = (p[2] - y) / 1.8814;
    let cr = (p[0] - y) / 1.4746;
    let ysdr = luma_curve(y, peak);
    let f = ysdr / (1.1 * y);
    let (cb, cr) = (f * cb, f * cr);
    let yt = ysdr - (0.1 * cr).max(0.0);
    [
        yt + 1.4746 * cr,
        yt - 0.16455 * cb - 0.57135 * cr,
        yt + 1.8814 * cb,
    ]
}

fn spow(v: f64, e: f64) -> f64 {
    v.signum() * v.abs().powf(e)
}

/// The BT.2020 to Rec.709 matrix in linear light.
pub fn bt2020_to_709() -> Mat3 {
    conversion(Primaries::Bt2020, Primaries::Bt709).expect("known primaries")
}

/// The whole input transform: HDR signal R′G′B′ to Rec.709 SDR code values
/// (read as E in Standard).
pub fn hdr_to_sdr(signal: HdrSignal, rgb: [f64; 3], peak: f64) -> [f64; 3] {
    let sdr2020 = method_a(to_normalized_display(signal, rgb, peak), peak);
    let lin = sdr2020.map(|c| spow(c, 2.4));
    apply(&bt2020_to_709(), lin).map(|c| spow(c, 1.0 / 2.4))
}

/// WGSL for the same transform. Defines
/// `pm_hdr_to_sdr(signal: vec3<f32>, hlg: bool, peak: f32, m: mat3x3<f32>) -> vec3<f32>`
/// where `m` is [`bt2020_to_709`] as columns.
pub const WGSL: &str = r#"
fn pm_pq_eotf(e: vec3<f32>) -> vec3<f32> {
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let p = pow(max(e, vec3<f32>(0.0)), vec3<f32>(1.0 / m2));
    return 10000.0 * pow(max(p - c1, vec3<f32>(0.0)) / (c2 - c3 * p), vec3<f32>(1.0 / m1));
}

fn pm_hlg_inverse_oetf(e: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 1.0 - 4.0 * a;
    let c = 0.55991073;
    let x = max(e, vec3<f32>(0.0));
    let lo = x * x / 3.0;
    let hi = (exp((x - c) / a) + b) / 12.0;
    return select(hi, lo, x <= vec3<f32>(0.5));
}

fn pm_rho(peak: f32) -> f32 {
    return 1.0 + 32.0 * pow(peak / 10000.0, 1.0 / 2.4);
}

fn pm_spow3(v: vec3<f32>, e: f32) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(e));
}

fn pm_hdr_to_sdr(signal: vec3<f32>, hlg: bool, peak: f32, m: mat3x3<f32>) -> vec3<f32> {
    let k = vec3<f32>(0.2627, 0.6780, 0.0593);
    var d: vec3<f32>;
    if (hlg) {
        let s = pm_hlg_inverse_oetf(signal);
        let ys = dot(k, s);
        let g = select(0.0, pow(ys, 0.2), ys > 0.0);
        d = s * g;
    } else {
        d = pm_pq_eotf(signal) / peak;
    }
    let p = pow(max(d, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4));
    let y = dot(k, p);
    if (y <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cb = (p.z - y) / 1.8814;
    let cr = (p.x - y) / 1.4746;
    let rh = pm_rho(peak);
    let yp = log(1.0 + (rh - 1.0) * y) / log(rh);
    var yc: f32;
    if (yp <= 0.7399) {
        yc = 1.0770 * yp;
    } else if (yp < 0.9909) {
        yc = -1.1510 * yp * yp + 2.7811 * yp - 0.6302;
    } else {
        yc = 0.5 * yp + 0.5;
    }
    let rs = pm_rho(100.0);
    let ysdr = (pow(rs, yc) - 1.0) / (rs - 1.0);
    let f = ysdr / (1.1 * y);
    let cbt = f * cb;
    let crt = f * cr;
    let yt = ysdr - max(0.1 * crt, 0.0);
    let sdr = vec3<f32>(yt + 1.4746 * crt, yt - 0.16455 * cbt - 0.57135 * crt, yt + 1.8814 * cbt);
    return pm_spow3(m * pm_spow3(sdr, 2.4), 1.0 / 2.4);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_round_trips_and_hits_its_anchors() {
        for nits in [0.0, 0.005, 1.0, 100.0, 203.0, 1000.0, 4000.0, 10000.0] {
            let s = pq_inverse_eotf(nits);
            assert!((pq_eotf(s) - nits).abs() <= 1e-9 * nits.max(1.0), "{nits}");
        }
        // ST 2084: 100 cd/m² is signal 0.508, 1000 cd/m² 0.752 (BT.2408 table).
        assert!((pq_inverse_eotf(100.0) - 0.5081).abs() < 1e-4);
        assert!((pq_inverse_eotf(1000.0) - 0.7518).abs() < 1e-4);
        assert!((pq_inverse_eotf(203.0) - 0.5807).abs() < 1e-4);
    }

    #[test]
    fn hlg_round_trips_and_is_continuous() {
        for i in 0..=1000 {
            let l = i as f64 / 1000.0;
            assert!((hlg_inverse_oetf(hlg_oetf(l)) - l).abs() < 1e-12, "{l}");
        }
        assert!((hlg_oetf(1.0 / 12.0) - 0.5).abs() < 1e-12);
        assert!((hlg_oetf(1.0) - 1.0).abs() < 1e-7);
        // BT.2408: HLG reference white is 75% signal, 203 cd/m² on a
        // 1000 cd/m² display.
        let d = to_normalized_display(HdrSignal::Hlg, [0.75; 3], HLG_PEAK);
        assert!((d[0] * HLG_PEAK - 203.0).abs() < 1.0, "{}", d[0] * HLG_PEAK);
    }

    #[test]
    fn luma_curve_endpoints_continuity_and_monotonicity() {
        assert_eq!(luma_curve(0.0, 1000.0), 0.0);
        assert!((luma_curve(1.0, 1000.0) - 1.0).abs() < 1e-12);
        let mut last = -1.0;
        for i in 0..=100_000 {
            let y = i as f64 / 100_000.0;
            let v = luma_curve(y, 1000.0);
            assert!(v > last, "not increasing at {y}");
            last = v;
        }
        // The knee's pieces meet within the published coefficients' precision.
        let rs = rho(L_SDR);
        let yc = |yp: f64| -1.1510 * yp * yp + 2.7811 * yp - 0.6302;
        assert!((1.0770 * 0.7399 - yc(0.7399)).abs() < 1e-3);
        assert!((0.5 * 0.9909 + 0.5 - yc(0.9909)).abs() < 1e-3);
        assert!(rs > 1.0);
    }

    #[test]
    fn reference_white_maps_as_derived_from_the_equations() {
        // HDR reference white (203 cd/m², BT.2408) through Method A for
        // L_HDR = 1000: Y′ = 0.203^(1/2.4) = 0.51448, Y′p = 0.76950,
        // Y′c = 0.82830, Y′SDR = 0.68679 (computed by hand from BT.2446-1).
        let grey = pq_inverse_eotf(203.0);
        let sdr = hdr_to_sdr(HdrSignal::Pq, [grey; 3], 1000.0);
        for c in sdr {
            assert!((c - 0.68679).abs() < 2e-4, "{sdr:?}");
        }
        // Greys stay neutral through the gamut conversion.
        assert!((sdr[0] - sdr[1]).abs() < 1e-9 && (sdr[1] - sdr[2]).abs() < 1e-9);
    }

    #[test]
    fn peak_white_is_sdr_white_and_black_is_black() {
        let peak = pq_inverse_eotf(1000.0);
        for c in hdr_to_sdr(HdrSignal::Pq, [peak; 3], 1000.0) {
            assert!((c - 1.0).abs() < 1e-6, "{c}");
        }
        for c in hdr_to_sdr(HdrSignal::Hlg, [1.0; 3], HLG_PEAK) {
            assert!((c - 1.0).abs() < 1e-6, "{c}");
        }
        assert_eq!(hdr_to_sdr(HdrSignal::Pq, [0.0; 3], 1000.0), [0.0; 3]);
        assert_eq!(hdr_to_sdr(HdrSignal::Hlg, [0.0; 3], 1000.0), [0.0; 3]);
    }

    #[test]
    fn saturated_bt2020_colors_stay_out_of_gamut_unclipped() {
        // A BT.2020 green at 100 cd/m² lies outside Rec.709: the result keeps
        // negative red and blue (no clip until output).
        let g = pq_inverse_eotf(100.0);
        let sdr = hdr_to_sdr(HdrSignal::Pq, [0.0, g, 0.0], 1000.0);
        assert!(sdr[0] < 0.0 && sdr[2] < 0.0 && sdr[1] > 0.0, "{sdr:?}");
    }
}
