//! HDR input tone mapping (plan section 9; owner decision HDR-1): PQ and
//! HLG sources are decoded per BT.2100 and mapped to SDR with the BT.2390
//! EETF.
//!
//! Per pixel, in f64 here and in f32 in [`wgsl`]:
//!
//! 1. The signal to display light in cd/m²: PQ through the ST 2084 EOTF;
//!    HLG through its inverse OETF and the BT.2100 OOTF for a 1000 cd/m²
//!    display (system gamma 1.2), BT.2408's route from HLG to PQ light.
//! 2. The BT.2390 EETF on each component in the PQ domain, from the
//!    content's peak (the mastering display's, else 1000 cd/m²) to SDR
//!    reference white, 203 cd/m² (BT.2408), with black at zero.
//! 3. Display light / 203: linear light with SDR white at 1.0, in the
//!    source's primaries. The caller converts primaries in linear light and
//!    encodes for the working space; out-of-gamut values are not clipped.

use crate::primaries::{Mat3, conversion};
use pm_protocol::color::Primaries;

/// BT.2020 luma coefficients (Kr, Kg, Kb).
pub const KR: f64 = 0.2627;
pub const KG: f64 = 0.6780;
pub const KB: f64 = 0.0593;

/// SDR reference white in HDR display light (BT.2408): SDR 1.0.
pub const SDR_WHITE: f64 = 203.0;

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

/// The BT.2390 EETF on one PQ-encoded component (0..1, ST 2084), mapping
/// content mastered at `source_peak` cd/m² to SDR white. Black stays at
/// zero, so the toe term is zero.
pub fn eetf(e: f64, source_peak: f64) -> f64 {
    let pw = pq_inverse_eotf(source_peak);
    let max_lum = pq_inverse_eotf(SDR_WHITE) / pw;
    let e1 = (e / pw).clamp(0.0, 1.0);
    if max_lum >= 1.0 {
        return e1 * pw;
    }
    let ks = 1.5 * max_lum - 0.5;
    if e1 < ks {
        return e1 * pw;
    }
    let t = (e1 - ks) / (1.0 - ks);
    let (t2, t3) = (t * t, t * t * t);
    let p = (2.0 * t3 - 3.0 * t2 + 1.0) * ks
        + (t3 - 2.0 * t2 + t) * (1.0 - ks)
        + (-2.0 * t3 + 3.0 * t2) * max_lum;
    p * pw
}

/// HDR signal R′G′B′ (0..1) to tone-mapped linear light, SDR white 1.0, in
/// the signal's primaries. `peak` is the content peak in cd/m² (PQ).
pub fn to_sdr(signal: HdrSignal, rgb: [f64; 3], peak: f64) -> [f64; 3] {
    let (nits, peak) = match signal {
        HdrSignal::Pq => (rgb.map(pq_eotf), peak),
        HdrSignal::Hlg => (
            to_normalized_display(HdrSignal::Hlg, rgb, HLG_PEAK).map(|c| c * HLG_PEAK),
            HLG_PEAK,
        ),
    };
    nits.map(|c| pq_eotf(eetf(pq_inverse_eotf(c), peak)) / SDR_WHITE)
}

/// The BT.2020 to Rec.709 matrix in linear light.
pub fn bt2020_to_709() -> Mat3 {
    conversion(Primaries::Bt2020, Primaries::Bt709).expect("known primaries")
}

/// WGSL defining `pm_hdr_to_sdr(signal: vec3<f32>, hlg: bool, peak: f32)
/// -> vec3<f32>`, [`to_sdr`] in f32.
pub fn wgsl() -> String {
    format!(
        r#"
fn pm_pq_eotf(e: vec3<f32>) -> vec3<f32> {{
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let p = pow(max(e, vec3<f32>(0.0)), vec3<f32>(1.0 / m2));
    return 10000.0 * pow(max(p - c1, vec3<f32>(0.0)) / (c2 - c3 * p), vec3<f32>(1.0 / m1));
}}

fn pm_pq_inverse_eotf(nits: vec3<f32>) -> vec3<f32> {{
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let y = pow(max(nits / 10000.0, vec3<f32>(0.0)), vec3<f32>(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vec3<f32>(m2));
}}

fn pm_hlg_inverse_oetf(e: vec3<f32>) -> vec3<f32> {{
    let a = 0.17883277;
    let b = 1.0 - 4.0 * a;
    let c = 0.55991073;
    let x = max(e, vec3<f32>(0.0));
    let lo = x * x / 3.0;
    let hi = (exp((x - c) / a) + b) / 12.0;
    return select(hi, lo, x <= vec3<f32>(0.5));
}}

fn pm_eetf(e: vec3<f32>, source_peak: f32) -> vec3<f32> {{
    let pw = pm_pq_inverse_eotf(vec3<f32>(source_peak)).x;
    let max_lum = pm_pq_inverse_eotf(vec3<f32>({white})).x / pw;
    let e1 = clamp(e / pw, vec3<f32>(0.0), vec3<f32>(1.0));
    if (max_lum >= 1.0) {{
        return e1 * pw;
    }}
    let ks = 1.5 * max_lum - 0.5;
    let t = (e1 - ks) / (1.0 - ks);
    let t2 = t * t;
    let t3 = t2 * t;
    let p = (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks)
        + (-2.0 * t3 + 3.0 * t2) * max_lum;
    return select(p, e1, e1 < vec3<f32>(ks)) * pw;
}}

fn pm_hdr_to_sdr(signal: vec3<f32>, hlg: bool, peak: f32) -> vec3<f32> {{
    var nits: vec3<f32>;
    var source_peak = peak;
    if (hlg) {{
        let s = pm_hlg_inverse_oetf(signal);
        let ys = dot(vec3<f32>({kr}, {kg}, {kb}), s);
        nits = {hlg_peak} * s * select(0.0, pow(ys, {gamma} - 1.0), ys > 0.0);
        source_peak = {hlg_peak};
    }} else {{
        nits = pm_pq_eotf(signal);
    }}
    return pm_pq_eotf(pm_eetf(pm_pq_inverse_eotf(nits), source_peak)) / {white};
}}
"#,
        white = f(SDR_WHITE),
        hlg_peak = f(HLG_PEAK),
        gamma = f(HLG_GAMMA),
        kr = f(KR),
        kg = f(KG),
        kb = f(KB),
    )
}

fn f(v: f64) -> String {
    format!("{v:.9e}")
}

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
    fn eetf_follows_bt2390() {
        let pw = pq_inverse_eotf(1000.0);
        let max_lum = pq_inverse_eotf(203.0) / pw;
        let ks = 1.5 * max_lum - 0.5;
        // Identity below the knee; the peak lands on the target maximum;
        // continuous and increasing in between; above the peak it holds.
        for i in 0..=1000 {
            let e = i as f64 / 1000.0 * ks * pw;
            assert!((eetf(e, 1000.0) - e).abs() < 1e-12);
        }
        assert!((eetf(pw, 1000.0) - max_lum * pw).abs() < 1e-12);
        assert!((eetf(1.0, 1000.0) - max_lum * pw).abs() < 1e-12);
        let mut last = 0.0;
        for i in 0..=10_000 {
            let v = eetf(i as f64 / 10_000.0 * pw, 1000.0);
            assert!(v >= last - 1e-15, "{i}");
            last = v;
        }
        // Content no brighter than SDR white is not compressed.
        let e = pq_inverse_eotf(150.0);
        assert_eq!(eetf(e, 200.0), e);
    }

    #[test]
    fn sdr_white_peak_and_black() {
        // PQ 1000 cd/m² maps to SDR white; black to black; a low grey is
        // unchanged (below the knee), relative to 203 cd/m².
        for c in to_sdr(HdrSignal::Pq, [pq_inverse_eotf(1000.0); 3], 1000.0) {
            assert!((c - 1.0).abs() < 1e-9, "{c}");
        }
        assert_eq!(to_sdr(HdrSignal::Pq, [0.0; 3], 1000.0), [0.0; 3]);
        let grey = to_sdr(HdrSignal::Pq, [pq_inverse_eotf(50.0); 3], 1000.0);
        assert!((grey[0] - 50.0 / 203.0).abs() < 1e-9, "{grey:?}");
        // HLG peak white is the 1000 cd/m² display peak: SDR white.
        for c in to_sdr(HdrSignal::Hlg, [1.0; 3], 1000.0) {
            assert!((c - 1.0).abs() < 1e-6, "{c}");
        }
    }

    #[test]
    fn saturated_bt2020_colors_stay_out_of_gamut_unclipped() {
        let g = pq_inverse_eotf(100.0);
        let sdr = crate::primaries::apply(
            &bt2020_to_709(),
            to_sdr(HdrSignal::Pq, [0.0, g, 0.0], 1000.0),
        );
        assert!(sdr[0] < 0.0 && sdr[2] < 0.0 && sdr[1] > 0.0, "{sdr:?}");
    }
}
