//! Color Space Transform: a picture's values read in one gamut and transfer,
//! tone and gamut mapped, and written in another, as Resolve's Color Space
//! Transform, Final Cut's HDR Tools and Lumetri's input/output conversions
//! do. This is the f64 reference; [`glsl`] generates the effect's shader
//! (`pm-fx-defs/src/color-space-transform.glsl`) from the same constants.
//!
//! Light is relative: 1.0 is SDR reference white, placed at 203 cd/m² for
//! HDR (Report ITU-R BT.2408), as the rest of the engine reads it. PQ is
//! absolute (cd/m² / 203); HLG is display light on its 1,000 cd/m² reference
//! display (BT.2100 system gamma 1.2); camera logs decode to scene linear
//! where 0.18 is mid grey. An SDR picture placed in an HDR output keeps its
//! white at 203 cd/m² without expansion (BT.2408 § 5): the conservative
//! direction, so no inverse tone mapping is offered.
//!
//! Tone mapping (`TONE_MAPPING`), on display light in BT.2020:
//! - BT.2390 EETF (Report ITU-R BT.2390), source peak to target peak, on
//!   each R, G, B component in the PQ domain, on luminance (hue and
//!   saturation kept), or on the largest component (saturation kept); the
//!   last two blend toward the per-component result by Highlight
//!   Desaturation.
//! - BT.2446 Method A and Method C (Report ITU-R BT.2446-1, §§ 4.1 and 6.1),
//!   HDR to SDR: Method A with its L_HDR the source peak; Method C with its
//!   published constants (k1..k4 for a 1,000 cd/m² source) and crosstalk
//!   α = 0.33 × Highlight Desaturation. Neither applies to an HDR output.
//!
//! Gamut mapping: none, clip (negative components of the output gamut to
//! zero), or the ACES Reference Gamut Compression (ACES 1.3, in ACEScg, with
//! its published limits, thresholds and power).

use crate::primaries::{Chromaticities, Mat3, apply, chromaticities, conversion_between};
use crate::tonemap::{KB, KG, KR, SDR_WHITE, eetf_to, pq_eotf, pq_inverse_eotf};
use pm_protocol::color::{Primaries, Transfer};

/// The gamut choices, in menu order.
pub const GAMUTS: [&str; 12] = [
    "Rec. 709 / sRGB",
    "P3-D65",
    "Rec. 2020",
    "ACES AP0 (ACES2065-1)",
    "ACES AP1 (ACEScg)",
    "Sony S-Gamut3",
    "Sony S-Gamut3.Cine",
    "Panasonic V-Gamut",
    "ARRI Wide Gamut 3",
    "ARRI Wide Gamut 4",
    "RED Wide Gamut RGB",
    "Blackmagic Wide Gamut (Gen 5)",
];

/// The transfer choices, in menu order. sRGB is the project's own display
/// encoding (the working values).
pub const TRANSFERS: [&str; 18] = [
    "sRGB (working)",
    "Linear",
    "Gamma 2.2",
    "Gamma 2.4 (BT.1886)",
    "Rec. 709 (camera)",
    "PQ (ST 2084)",
    "HLG (BT.2100)",
    "ACEScct",
    "ACEScc",
    "Apple Log",
    "Sony S-Log3",
    "Panasonic V-Log",
    "ARRI LogC3 (EI 800)",
    "ARRI LogC4",
    "RED Log3G10",
    "Blackmagic Film (Gen 5)",
    "Fujifilm F-Log",
    "Nikon N-Log",
];

pub const TONE_MAPPING: [&str; 6] = [
    "None",
    "BT.2390 (per channel)",
    "BT.2390 (luminance)",
    "BT.2390 (saturation preserving)",
    "BT.2446 Method A",
    "BT.2446 Method C",
];

pub const GAMUT_MAPPING: [&str; 3] = ["None", "Clip", "ACES Reference Gamut Compression"];

/// The transfers that hold HDR display light.
pub const PQ: usize = 5;
pub const HLG: usize = 6;

/// The settings of one transform (the effect's parameters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub input_gamut: usize,
    pub input_transfer: usize,
    pub output_gamut: usize,
    pub output_transfer: usize,
    pub tone: usize,
    /// cd/m²: the content's peak (BT.2390's and Method A's source).
    pub source_peak: f64,
    /// cd/m²: an HDR output's peak; an SDR output's is SDR white.
    pub target_peak: f64,
    /// 0..100: Highlight Desaturation.
    pub desaturation: f64,
    pub gamut_mapping: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_gamut: 0,
            input_transfer: 0,
            output_gamut: 0,
            output_transfer: 0,
            tone: 0,
            source_peak: 1000.,
            target_peak: 1000.,
            desaturation: 15.,
            gamut_mapping: 0,
        }
    }
}

const D65: [f64; 2] = [0.3127, 0.3290];
const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];

/// The chromaticities of a gamut choice: the published ones (ACES TB-2014-004
/// and S-2014-004, RED, Blackmagic Generation 5 Color Science), the rest from
/// [`chromaticities`].
pub fn gamut(index: usize) -> Chromaticities {
    let tagged = |p| chromaticities(p).expect("known primaries");
    let c = |r: [f64; 2], g: [f64; 2], b: [f64; 2], w: [f64; 2]| Chromaticities { r, g, b, w };
    match index {
        1 => tagged(Primaries::DisplayP3),
        2 => tagged(Primaries::Bt2020),
        3 => c([0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.0770], ACES_WHITE),
        4 => c([0.713, 0.293], [0.165, 0.830], [0.128, 0.044], ACES_WHITE),
        5 => tagged(Primaries::SGamut3),
        6 => tagged(Primaries::SGamut3Cine),
        7 => tagged(Primaries::VGamut),
        8 => tagged(Primaries::AwgGen3),
        9 => tagged(Primaries::AwgGen4),
        10 => c(
            [0.780308, 0.304253],
            [0.121595, 1.493994],
            [0.095612, -0.084589],
            D65,
        ),
        11 => c(
            [0.7177215, 0.3171181],
            [0.2280410, 0.8615690],
            [0.1005841, -0.0820452],
            D65,
        ),
        _ => tagged(Primaries::Bt709),
    }
}

/// Linear light of gamut `from` to gamut `to` (Bradford where whites differ).
pub fn conversion(from: usize, to: usize) -> Mat3 {
    conversion_between(&gamut(from), &gamut(to))
}

fn signed(v: f64, f: impl Fn(f64) -> f64) -> f64 {
    if v < 0. { -f(-v) } else { f(v) }
}

/// The sRGB curve extended to all reals: the working encoding.
fn e_decode(v: f64) -> f64 {
    signed(v, |m| {
        if m <= 0.04045 {
            m / 12.92
        } else {
            ((m + 0.055) / 1.055).powf(2.4)
        }
    })
}
fn e_encode(l: f64) -> f64 {
    signed(l, |m| {
        if m <= 0.0031308 {
            12.92 * m
        } else {
            1.055 * m.powf(1. / 2.4) - 0.055
        }
    })
}

/// The camera curves the engine does not otherwise read, from their
/// publishers' definitions (ACES S-2016-001 and S-2014-003, RED White Paper
/// on REDWideGamutRGB and Log3G10, Blackmagic Generation 5 Color Science,
/// Fujifilm F-Log Data Sheet, Nikon N-Log Specification).
fn log_decode(t: usize, v: f64) -> f64 {
    match t {
        7 => {
            if v <= 0.155251141552511 {
                (v - 0.0729055341958355) / 10.5402377416545
            } else {
                (v.min((65504f64.log2() + 9.72) / 17.52) * 17.52 - 9.72).exp2()
            }
        }
        8 => {
            if v < (9.72 - 15.) / 17.52 {
                ((v * 17.52 - 9.72).exp2() - (-16f64).exp2()) * 2.
            } else if v < (65504f64.log2() + 9.72) / 17.52 {
                (v * 17.52 - 9.72).exp2()
            } else {
                65504.
            }
        }
        14 => {
            if v < 0. {
                v / 15.1927 - 0.01
            } else {
                (10f64.powf(v / 0.224282) - 1.) / 155.975327 - 0.01
            }
        }
        15 => {
            if v < 8.283605932402494 * 0.005 + 0.09246575342465753 {
                (v - 0.09246575342465753) / 8.283605932402494
            } else {
                ((v - 0.5300133392291939) / 0.08692876065491224).exp() - 0.005494072432257808
            }
        }
        16 => {
            if v >= 0.100537775223865 {
                (10f64.powf((v - 0.790453) / 0.344676) - 0.009468) / 0.555556
            } else {
                (v - 0.092864) / 8.735631
            }
        }
        17 => {
            if v < 452. / 1023. {
                let x = v * 1023. / 650.;
                x * x * x - 0.0075
            } else {
                ((v * 1023. - 619.) / 150.).exp()
            }
        }
        _ => unreachable!("not one of these curves"),
    }
}
fn log_encode(t: usize, l: f64) -> f64 {
    match t {
        7 => {
            if l <= 0.0078125 {
                10.5402377416545 * l + 0.0729055341958355
            } else {
                (l.log2() + 9.72) / 17.52
            }
        }
        8 => {
            if l <= 0. {
                ((-16f64).exp2().log2() + 9.72) / 17.52
            } else if l < (-15f64).exp2() {
                (((-16f64).exp2() + l * 0.5).log2() + 9.72) / 17.52
            } else {
                (l.log2() + 9.72) / 17.52
            }
        }
        14 => {
            let x = l + 0.01;
            if x < 0. {
                x * 15.1927
            } else {
                0.224282 * (x * 155.975327 + 1.).log10()
            }
        }
        15 => {
            if l < 0.005 {
                8.283605932402494 * l + 0.09246575342465753
            } else {
                0.08692876065491224 * (l + 0.005494072432257808).ln() + 0.5300133392291939
            }
        }
        16 => {
            if l >= 0.00089 {
                0.344676 * (0.555556 * l + 0.009468).log10() + 0.790453
            } else {
                8.735631 * l + 0.092864
            }
        }
        17 => {
            if l < 0.328 {
                let x = l + 0.0075;
                650. / 1023. * x.signum() * x.abs().cbrt()
            } else {
                (150. * l.ln() + 619.) / 1023.
            }
        }
        _ => unreachable!("not one of these curves"),
    }
}

/// The vendor curves `camera_log` reads.
fn camera(t: usize) -> Option<Transfer> {
    Some(match t {
        9 => Transfer::AppleLog,
        10 => Transfer::SLog3,
        11 => Transfer::VLog,
        12 => Transfer::LogC3,
        13 => Transfer::LogC4,
        _ => return None,
    })
}

fn bt709_inverse_oetf(v: f64) -> f64 {
    signed(v, |m| {
        if m < 0.081 {
            m / 4.5
        } else {
            ((m + 0.099) / 1.099).powf(1. / 0.45)
        }
    })
}
fn bt709_oetf(l: f64) -> f64 {
    signed(l, |m| {
        if m < 0.018 {
            4.5 * m
        } else {
            1.099 * m.powf(0.45) - 0.099
        }
    })
}

/// HLG on its 1,000 cd/m² reference display, relative to 203 cd/m².
fn hlg_decode(v: [f64; 3]) -> [f64; 3] {
    let s = v.map(|c| crate::tonemap::hlg_inverse_oetf(c.max(0.)));
    let y = KR * s[0] + KG * s[1] + KB * s[2];
    if y <= 0. {
        return [0.; 3];
    }
    s.map(|c| c * y.powf(0.2) * 1000. / SDR_WHITE)
}
fn hlg_encode(l: [f64; 3]) -> [f64; 3] {
    let d = l.map(|c| c * SDR_WHITE / 1000.);
    let y = KR * d[0] + KG * d[1] + KB * d[2];
    let s = if y > 0. {
        d.map(|c| c * y.powf(-1. / 6.))
    } else {
        [0.; 3]
    };
    s.map(|c| crate::tonemap::hlg_oetf(c.max(0.)))
}

/// Values in transfer `t` to relative light.
pub fn decode(t: usize, v: [f64; 3]) -> [f64; 3] {
    if t == HLG {
        return hlg_decode(v);
    }
    v.map(|c| match t {
        0 => e_decode(c),
        1 => c,
        2 => signed(c, |m| m.powf(2.2)),
        3 => signed(c, |m| m.powf(2.4)),
        4 => bt709_inverse_oetf(c),
        PQ => pq_eotf(c.clamp(0., 1.)) / SDR_WHITE,
        t => match camera(t) {
            Some(curve) => crate::camera_log::decode(curve, c).expect("a camera curve"),
            None => log_decode(t, c),
        },
    })
}

/// Relative light to values in transfer `t`.
pub fn encode(t: usize, l: [f64; 3]) -> [f64; 3] {
    if t == HLG {
        return hlg_encode(l);
    }
    l.map(|c| match t {
        0 => e_encode(c),
        1 => c,
        2 => signed(c, |m| m.powf(1. / 2.2)),
        3 => signed(c, |m| m.powf(1. / 2.4)),
        4 => bt709_oetf(c),
        PQ => pq_inverse_eotf((c * SDR_WHITE).clamp(0., 10000.)),
        t => match camera(t) {
            Some(curve) => crate::camera_log::encode(curve, c).expect("a camera curve"),
            None => log_encode(t, c),
        },
    })
}

/// The BT.2390 EETF on display light in cd/m² (PQ domain).
fn eetf_nits(nits: f64, source: f64, target: f64) -> f64 {
    if nits <= 0. {
        return nits;
    }
    pq_eotf(eetf_to(pq_inverse_eotf(nits), source, target))
}

/// BT.2446 Method A (Report ITU-R BT.2446-1 § 4.1, Tables 2 and 3): BT.2020
/// display light in cd/m², mastered at `l_hdr`, to SDR relative light
/// (1.0 the SDR display's peak, which is SDR white here).
pub fn bt2446a(nits: [f64; 3], l_hdr: f64) -> [f64; 3] {
    let rho = |l: f64| 1. + 32. * (l / 10000.).powf(1. / 2.4);
    let (rho_hdr, rho_sdr) = (rho(l_hdr), rho(100.));
    let p = nits.map(|c| (c / l_hdr).clamp(0., 1.).powf(1. / 2.4));
    let y = KR * p[0] + KG * p[1] + KB * p[2];
    let yp = (1. + (rho_hdr - 1.) * y).ln() / rho_hdr.ln();
    let yc = if yp <= 0.7399 {
        1.0770 * yp
    } else if yp < 0.9909 {
        -1.1510 * yp * yp + 2.7811 * yp - 0.6302
    } else {
        0.5 * yp + 0.5
    };
    let y_sdr = (rho_sdr.powf(yc) - 1.) / (rho_sdr - 1.);
    let f = if y > 0. { y_sdr / (1.1 * y) } else { 0. };
    let cb = f * (p[2] - y) / 1.8814;
    let cr = f * (p[0] - y) / 1.4746;
    let y_tmo = y_sdr - (0.1 * cr).max(0.);
    let r = y_tmo + 1.4746 * cr;
    let b = y_tmo + 1.8814 * cb;
    let g = (y_tmo - KR * r - KB * b) / KG;
    [r, g, b].map(|v| v.max(0.).powf(2.4))
}

/// BT.2446 Method C (Report ITU-R BT.2446-1 § 6.1, Eqs. 2 to 14): BT.2020
/// display light in cd/m² to SDR relative light (1.0 = 100 cd/m²), with
/// crosstalk `alpha` (0..0.33) and the published k1..k4.
pub fn bt2446c(nits: [f64; 3], alpha: f64) -> [f64; 3] {
    const K1: f64 = 0.83802;
    const K3: f64 = 0.74204;
    // The knee: 80 % SDR, 0.8^2.4 of 100 cd/m² (58.535; Eq. 6 rounds it to
    // 58.5, and Eq. 10's k2 and k4 are the ones 58.535 gives).
    let ip = 100. * 0.8f64.powf(2.4) / K1;
    let k2 = K1 * (1. - K3) * ip;
    let k4 = K1 * ip - k2 * (1. - K3).ln();
    let crosstalk: Mat3 = [
        [1. - 2. * alpha, alpha, alpha],
        [alpha, 1. - 2. * alpha, alpha],
        [alpha, alpha, 1. - 2. * alpha],
    ];
    let to_xyz: Mat3 = [
        [0.6370, 0.1446, 0.1689],
        [0.2627, 0.6780, 0.0593],
        [0.0000, 0.0281, 1.0610],
    ];
    let to_rgb: Mat3 = [
        [1.7167, -0.3557, -0.2534],
        [-0.6667, 1.6165, 0.0158],
        [0.0176, -0.0428, 0.9421],
    ];
    let xyz = apply(&to_xyz, apply(&crosstalk, nits.map(|c| c.max(0.))));
    if xyz[1] <= 0. || xyz[0] + xyz[1] + xyz[2] <= 0. {
        return [0.; 3];
    }
    let y_hdr = xyz[1];
    let y_sdr = if y_hdr < ip {
        K1 * y_hdr
    } else {
        k2 * (y_hdr / ip - K3).ln() + k4
    };
    // Eqs. 11 and 12 keep x and y: X = (x / y) Y_SDR is X_HDR × Y_SDR / Y_HDR,
    // the form that stays exact for dark saturated colours.
    let sdr = xyz.map(|c| c * (y_sdr / y_hdr));
    let rgb = apply(&to_rgb, sdr);
    // The inverse crosstalk (Eq. 14).
    let s = 1. / (1. - 3. * alpha);
    let sum_rgb = rgb[0] + rgb[1] + rgb[2];
    rgb.map(|c| s * (c - alpha * sum_rgb) / 100.)
}

/// The ACES 1.3 Reference Gamut Compression on ACEScg values.
pub fn reference_gamut_compression(rgb: [f64; 3]) -> [f64; 3] {
    const LIMIT: [f64; 3] = [1.147, 1.264, 1.312];
    const THRESHOLD: [f64; 3] = [0.815, 0.803, 0.880];
    const POWER: f64 = 1.2;
    let a = rgb[0].max(rgb[1]).max(rgb[2]);
    if a == 0. {
        return rgb;
    }
    std::array::from_fn(|c| {
        let d = (a - rgb[c]) / a.abs();
        let (l, t) = (LIMIT[c], THRESHOLD[c]);
        if d < t {
            return rgb[c];
        }
        let dc = {
            let s = (l - t) / (((1. - t) / (l - t)).powf(-POWER) - 1.).powf(1. / POWER);
            t + (d - t) / (1. + ((d - t) / s).powf(POWER)).powf(1. / POWER)
        };
        a - dc * a.abs()
    })
}

/// Tone mapping on BT.2020 relative light.
fn tone_map(l: [f64; 3], s: &Settings) -> [f64; 3] {
    let hdr_out = s.output_transfer == PQ || s.output_transfer == HLG;
    let target = if hdr_out { s.target_peak } else { SDR_WHITE };
    let nits = l.map(|c| c * SDR_WHITE);
    let rel = |n: [f64; 3]| n.map(|c| c / SDR_WHITE);
    let per_channel = |n: [f64; 3]| n.map(|c| eetf_nits(c, s.source_peak, target));
    let blend = |kept: [f64; 3], n: [f64; 3]| {
        let d = (s.desaturation / 100.).clamp(0., 1.);
        let pc = per_channel(n);
        std::array::from_fn(|c| kept[c] + (pc[c] - kept[c]) * d)
    };
    let scaled = |n: [f64; 3], m: f64| {
        if m <= 0. {
            return n;
        }
        let k = eetf_nits(m, s.source_peak, target) / m;
        n.map(|c| c * k)
    };
    match s.tone {
        1 | 2 | 3 if s.source_peak <= target => l,
        1 => rel(per_channel(nits)),
        2 => rel(blend(
            scaled(nits, KR * nits[0] + KG * nits[1] + KB * nits[2]),
            nits,
        )),
        3 => rel(blend(scaled(nits, nits[0].max(nits[1]).max(nits[2])), nits)),
        4 if !hdr_out => bt2446a(nits, s.source_peak),
        5 if !hdr_out => bt2446c(nits, 0.33 * (s.desaturation / 100.).clamp(0., 1.)),
        _ => l,
    }
}

/// Whether `s` leaves every value as it is.
pub fn is_identity(s: &Settings) -> bool {
    s.input_gamut == s.output_gamut
        && s.input_transfer == s.output_transfer
        && s.tone == 0
        && s.gamut_mapping == 0
}

/// One value through the transform.
pub fn transform(v: [f64; 3], s: &Settings) -> [f64; 3] {
    if is_identity(s) {
        return v;
    }
    let to_2020 = conversion(s.input_gamut, 2);
    let mut l = tone_map(apply(&to_2020, decode(s.input_transfer, v)), s);
    if s.gamut_mapping == 2 {
        let ap1 = reference_gamut_compression(apply(&conversion(2, 4), l));
        l = apply(&conversion(4, 2), ap1);
    }
    let mut out = apply(&conversion(2, s.output_gamut), l);
    if s.gamut_mapping == 1 {
        out = out.map(|c| c.max(0.));
    }
    encode(s.output_transfer, out)
}

/// The effect's shader: the LUT shader's tested encodings (the working
/// curve, PQ, HLG and the camera logs, by its space numbers) and this
/// module's matrices and mappings.
pub fn glsl() -> String {
    let lut = include_str!("../../pm-fx-defs/src/apply-color-lut.glsl");
    let start = lut.find("float lut_e_dec").expect("the LUT encodings");
    let end = lut[start..]
        .find("vec3 lut_encode")
        .expect("the end of the encodings")
        + start;
    let mat = |m: Mat3| {
        format!(
            "mat3({})",
            (0..3)
                .flat_map(|j| (0..3).map(move |i| format!("{:.10}", m[i][j])))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let table = |name: &str, to: bool| {
        let arms: String = (1..GAMUTS.len())
            .map(|g| {
                let m = if to {
                    conversion(g, 2)
                } else {
                    conversion(2, g)
                };
                format!("if(g=={g})return {};", mat(m))
            })
            .collect();
        format!(
            "mat3 {name}(int g){{{arms}return {};}}\n",
            mat(if to {
                conversion(0, 2)
            } else {
                conversion(2, 0)
            })
        )
    };
    format!(
        "{HEAD}{prelude}{to}{from}const mat3 CS_2020_TO_AP1={a};\nconst mat3 CS_AP1_TO_2020={b};\n{BODY}",
        prelude = &lut[start..end],
        to = table("cs_to_2020", true),
        from = table("cs_from_2020", false),
        a = mat(conversion(2, 4)),
        b = mat(conversion(4, 2)),
    )
}

const HEAD: &str = "#version 300 es
precision highp float;
// Generated by pm_color::cst::glsl() from the f64 reference in pm-color's
// cst.rs; regenerate with PM_BLESS=1 cargo test -p pm-color cst.
in vec2 v_st;
uniform sampler2D u_tex;
uniform float u_inputGamut,u_inputTransfer,u_outputGamut,u_outputTransfer,u_toneMapping;
uniform float u_sourcePeak,u_targetPeak,u_desaturation,u_gamutMapping;
out vec4 o;
";

const BODY: &str = r#"float cs_signed(float v,float p){return v<0.?-pow(-v,p):pow(v,p);}
float cs_709_inv(float v){float m=abs(v);float l=m<.081?m/4.5:pow((m+.099)/1.099,1./.45);return v<0.?-l:l;}
float cs_709(float l){float m=abs(l);float v=m<.018?4.5*m:1.099*pow(m,.45)-.099;return l<0.?-v:v;}
// The LUT's space number of each camera transfer (ACEScct and the logs).
int cs_lut(int t){if(t==7)return 10;if(t==9)return 2;if(t==10)return 3;if(t==11)return 5;if(t==12)return 6;if(t==13)return 7;if(t==14)return 11;if(t==15)return 12;if(t==16)return 13;return 14;}
float cs_acescc_dec(float v){if(v<(9.72-15.)/17.52)return (lut_pow2(v*17.52-9.72)-lut_pow2(-16.))*2.;if(v<(log2(65504.)+9.72)/17.52)return lut_pow2(v*17.52-9.72);return 65504.;}
float cs_acescc_enc(float l){if(l<=0.)return (-16.+9.72)/17.52;if(l<lut_pow2(-15.))return (log2(lut_pow2(-16.)+l*.5)+9.72)/17.52;return (log2(l)+9.72)/17.52;}
float cs_dec(float v,int t){if(t==0)return lut_e_dec(v);if(t==1)return v;if(t==2)return cs_signed(v,2.2);if(t==3)return cs_signed(v,2.4);if(t==4)return cs_709_inv(v);if(t==5)return lut_log_dec(v,8);if(t==8)return cs_acescc_dec(v);return lut_log_dec(v,cs_lut(t));}
float cs_enc(float l,int t){if(t==0)return lut_e_enc(l);if(t==1)return l;if(t==2)return cs_signed(l,1./2.2);if(t==3)return cs_signed(l,1./2.4);if(t==4)return cs_709(l);if(t==5)return lut_log_enc(clamp(l,0.,10000./203.),8);if(t==8)return cs_acescc_enc(l);return lut_log_enc(l,cs_lut(t));}
vec3 cs_dec3(vec3 v,int t){if(t==6)return lut_hlg_dec(v);return vec3(cs_dec(v.r,t),cs_dec(v.g,t),cs_dec(v.b,t));}
vec3 cs_enc3(vec3 l,int t){if(t==6)return lut_hlg_enc(l);return vec3(cs_enc(l.r,t),cs_enc(l.g,t),cs_enc(l.b,t));}
float cs_pq(float e){return lut_log_dec(e,8)*203.;}
float cs_pq_inv(float n){return lut_log_enc(n/203.,8);}
float cs_eetf(float e,float sp,float tp){float pw=cs_pq_inv(sp);float ml=cs_pq_inv(tp)/pw;float e1=clamp(e/pw,0.,1.);if(ml>=1.)return e1*pw;float ks=1.5*ml-.5;if(e1<ks)return e1*pw;float t=(e1-ks)/(1.-ks),t2=t*t,t3=t2*t;return ((2.*t3-3.*t2+1.)*ks+(t3-2.*t2+t)*(1.-ks)+(-2.*t3+3.*t2)*ml)*pw;}
float cs_map(float n,float sp,float tp){return n<=0.?n:cs_pq(cs_eetf(cs_pq_inv(n),sp,tp));}
vec3 cs_channels(vec3 n,float sp,float tp){return vec3(cs_map(n.r,sp,tp),cs_map(n.g,sp,tp),cs_map(n.b,sp,tp));}
vec3 cs_scaled(vec3 n,float m,float sp,float tp){return m<=0.?n:n*(cs_map(m,sp,tp)/m);}
vec3 cs_2446a(vec3 n,float lh){float rh=1.+32.*pow(lh/10000.,1./2.4),rs=1.+32.*pow(.01,1./2.4);vec3 p=pow(clamp(n/lh,0.,1.),vec3(1./2.4));float y=dot(p,vec3(.2627,.6780,.0593));float yp=log(1.+(rh-1.)*y)/log(rh);float yc=yp<=.7399?1.077*yp:yp<.9909?-1.151*yp*yp+2.7811*yp-.6302:.5*yp+.5;float ys=(pow(rs,yc)-1.)/(rs-1.);float f=y>0.?ys/(1.1*y):0.;float cb=f*(p.b-y)/1.8814,cr=f*(p.r-y)/1.4746;float yt=ys-max(.1*cr,0.);float r=yt+1.4746*cr,b=yt+1.8814*cb;float g=(yt-.2627*r-.0593*b)/.6780;return pow(max(vec3(r,g,b),0.),vec3(2.4));}
vec3 cs_2446c(vec3 n,float a){const float k1=.83802,k3=.74204;float ip=100.*pow(.8,2.4)/k1;float k2=k1*(1.-k3)*ip,k4=k1*ip-k2*log(1.-k3);mat3 ct=mat3(1.-2.*a,a,a,a,1.-2.*a,a,a,a,1.-2.*a);mat3 tx=mat3(.6370,.2627,0.,.1446,.6780,.0281,.1689,.0593,1.0610);mat3 tr=mat3(1.7167,-.6667,.0176,-.3557,1.6165,-.0428,-.2534,.0158,.9421);vec3 xyz=tx*(ct*max(n,0.));float s=xyz.x+xyz.y+xyz.z;if(xyz.y<=0.||s<=0.)return vec3(0.);float yh=xyz.y;float ys=yh<ip?k1*yh:k2*log(yh/ip-k3)+k4;vec3 rgb=tr*(xyz*(ys/yh));float sr=rgb.r+rgb.g+rgb.b;return (rgb-a*sr)/(1.-3.*a)/100.;}
vec3 cs_tone(vec3 l,int m,int ot,float sp,float tp,float ds){bool hdr=ot==5||ot==6;float t=hdr?tp:203.;vec3 n=l*203.;float d=clamp(ds/100.,0.,1.);
 if((m==1||m==2||m==3)&&sp<=t)return l;
 if(m==1)return cs_channels(n,sp,t)/203.;
 if(m==2||m==3){float k=m==2?dot(n,vec3(.2627,.6780,.0593)):max(n.r,max(n.g,n.b));vec3 kept=cs_scaled(n,k,sp,t);return mix(kept,cs_channels(n,sp,t),d)/203.;}
 if(m==4&&!hdr)return cs_2446a(n,sp);
 if(m==5&&!hdr)return cs_2446c(n,.33*d);
 return l;}
float cs_rgc1(float a,float v,float l,float t){float d=(a-v)/abs(a);if(d<t)return v;float s=(l-t)/pow(pow((1.-t)/(l-t),-1.2)-1.,1./1.2);return a-(t+(d-t)/pow(1.+pow((d-t)/s,1.2),1./1.2))*abs(a);}
vec3 cs_rgc(vec3 c){float a=max(c.r,max(c.g,c.b));if(a==0.)return c;return vec3(cs_rgc1(a,c.r,1.147,.815),cs_rgc1(a,c.g,1.264,.803),cs_rgc1(a,c.b,1.312,.880));}
void main(){
 vec4 c=texture(u_tex,v_st);
 int ig=int(u_inputGamut+.5),it=int(u_inputTransfer+.5),og=int(u_outputGamut+.5),ot=int(u_outputTransfer+.5),tm=int(u_toneMapping+.5),gm=int(u_gamutMapping+.5);
 if(c.a<=0.||!lut_finite(c.a)||(ig==og&&it==ot&&tm==0&&gm==0)){o=c;return;}
 vec3 l=cs_tone(cs_to_2020(ig)*cs_dec3(c.rgb/c.a,it),tm,ot,u_sourcePeak,u_targetPeak,u_desaturation);
 if(gm==2)l=CS_AP1_TO_2020*cs_rgc(CS_2020_TO_AP1*l);
 vec3 v=cs_from_2020(og)*l;
 if(gm==1)v=max(v,0.);
 o=vec4(cs_enc3(v,ot)*c.a,c.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primaries::mul;

    /// The effect's shader is generated from this module: PM_BLESS=1 writes it.
    #[test]
    fn the_shader_is_generated_from_these_definitions() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-fx-defs/src/color-space-transform.glsl"
        );
        let generated = glsl();
        if std::env::var_os("PM_BLESS").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        let stored = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            stored == generated,
            "color-space-transform.glsl is out of date: PM_BLESS=1 cargo test -p pm-color cst"
        );
    }

    fn close(a: &Mat3, b: &Mat3, tolerance: f64) -> bool {
        (0..3).all(|i| (0..3).all(|j| (a[i][j] - b[i][j]).abs() < tolerance))
    }

    /// The ACES gamuts against the matrix the RGC specification publishes
    /// (TRA_1, AP0 to AP1), and the camera gamuts against the LUT effect's.
    #[test]
    fn gamuts_match_their_published_matrices() {
        let tra1: Mat3 = [
            [1.4514393161, -0.2365107469, -0.2149285693],
            [-0.0765537734, 1.1762296998, -0.0996759264],
            [0.0083161484, -0.0060324498, 0.9977163014],
        ];
        assert!(
            close(&conversion(3, 4), &tra1, 1e-9),
            "{:?}",
            conversion(3, 4)
        );
        // Every conversion round-trips and a gamut to itself is the identity.
        for g in 0..GAMUTS.len() {
            let there_and_back = mul(&conversion(2, g), &conversion(g, 2));
            let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
            assert!(close(&there_and_back, &identity, 1e-12), "{}", GAMUTS[g]);
        }
        // The LUT effect's Rec. 709 to RED Wide Gamut and to Blackmagic WG.
        let lut = include_str!("../../pm-fx-defs/src/apply-color-lut.glsl");
        for (space, g) in [(11, 10), (12, 11)] {
            let arm = format!("if(s=={space})return mat3(");
            let to_gamut = &lut[lut.find("mat3 lut_to_gamut").unwrap()..];
            let at = to_gamut.find(&arm).unwrap() + arm.len();
            let n: Vec<f64> = to_gamut[at..at + to_gamut[at..].find(')').unwrap()]
                .split(',')
                .map(|v| v.trim().parse().unwrap())
                .collect();
            let m = conversion(0, g);
            for j in 0..3 {
                for i in 0..3 {
                    // The LUT's Blackmagic matrix came from a white given to
                    // more digits; they differ by 3e-5 at most.
                    assert!((m[i][j] - n[j * 3 + i]).abs() < 5e-5, "{}", GAMUTS[g]);
                }
            }
        }
    }

    /// Every transfer decodes what it encodes, PQ and HLG on their
    /// published anchors (BT.2408: 203 cd/m² is PQ 0.58, HLG 0.75).
    #[test]
    fn transfers_round_trip_and_hold_their_anchors() {
        for t in 0..TRANSFERS.len() {
            for v in [0.0, 0.01, 0.18, 0.5, 1.0, 2.5] {
                let back = decode(t, encode(t, [v; 3]));
                // Published camera curves meet at their breakpoints to about
                // 1e-7 (V-Log's segments, for one), not exactly.
                assert!(
                    (back[0] - v).abs() < 1e-6 * v.max(1.),
                    "{} at {v}: {}",
                    TRANSFERS[t],
                    back[0]
                );
            }
        }
        assert!((encode(PQ, [1.; 3])[0] - 0.5807).abs() < 1e-4);
        assert!((encode(HLG, [1.; 3])[0] - 0.75).abs() < 2e-3);
        assert!((decode(PQ, [1.; 3])[0] * SDR_WHITE - 10000.).abs() < 1e-6);
    }

    /// BT.2446 Method A on the report's equations: the source peak is the
    /// SDR peak, black is black, grey keeps its hue (no tint).
    #[test]
    fn bt2446_method_a_holds_its_end_points() {
        let top = bt2446a([1000.; 3], 1000.);
        assert!(top.iter().all(|v| (v - 1.).abs() < 1e-3), "{top:?}");
        assert_eq!(bt2446a([0.; 3], 1000.), [0.; 3]);
        let grey = bt2446a([203.; 3], 1000.);
        assert!((grey[0] - grey[2]).abs() < 1e-9 && (grey[0] - grey[1]).abs() < 1e-9);
        // HDR reference white from 1,000 cd/m²: Y'SDR 0.687 (worked from
        // Table 2: Y'p 0.7696, Y'c 0.8284).
        assert!((grey[0].powf(1. / 2.4) - 0.6867).abs() < 1e-3, "{grey:?}");
    }

    /// BT.2446 Method C on the report's numbers (Eq. 10 and § 6.1.4): k2 and
    /// k4 follow from k1 and k3, the knee is 58.5 cd/m² SDR, and below it the
    /// mapping is the linear gain k1.
    #[test]
    fn bt2446_method_c_holds_its_published_constants() {
        let (k1, k3): (f64, f64) = (0.83802, 0.74204);
        let ip = 100. * 0.8f64.powf(2.4) / k1;
        assert!((k1 * (1. - k3) * ip - 15.09968).abs() < 1e-4);
        assert!((k1 * ip - k1 * (1. - k3) * ip * (1. - k3).ln() - 78.99439).abs() < 1e-4);
        let dim = bt2446c([50.; 3], 0.);
        assert!(
            dim.iter().all(|v| (v - 50. * k1 / 100.).abs() < 1e-3),
            "{dim:?}"
        );
        let knee = bt2446c([ip; 3], 0.);
        assert!(knee.iter().all(|v| (v - 0.58535).abs() < 1e-3), "{knee:?}");
        // Continuous at the knee.
        let above = bt2446c([ip * 1.0001; 3], 0.);
        assert!((above[1] - knee[1]).abs() < 1e-4, "{knee:?} {above:?}");
        // HDR reference white (203) about 91 cd/m² SDR (Fig. 10).
        let white = bt2446c([203.; 3], 0.);
        assert!((white[1] * 100. - 90.9).abs() < 1., "{white:?}");
    }

    /// The RGC on the specification's definitions: inside the thresholds
    /// nothing moves; a distance at the limit compresses to 1 (onto the
    /// gamut boundary); the achromatic channel stays.
    #[test]
    fn the_reference_gamut_compression_holds_its_limits() {
        assert_eq!(
            reference_gamut_compression([0.5, 0.45, 0.47]),
            [0.5, 0.45, 0.47]
        );
        // Red's distance at its limit: 1 - r = 1.147 with A = 1.
        let at_limit = reference_gamut_compression([1. - 1.147, 1., 1.]);
        assert!(at_limit[0].abs() < 1e-12, "{at_limit:?}");
        assert_eq!(at_limit[1], 1.);
        // Past the threshold, compressed but still ordered.
        let past = reference_gamut_compression([0.1, 1., 0.9]);
        assert!(past[0] > 0.1 && past[0] < 0.185 + 1e-12);
    }

    /// BT.2390 in the transform: a 1,000 cd/m² peak to SDR white, black and
    /// mid grey unmoved; saturation preserving keeps the R:G:B ratios.
    #[test]
    fn bt2390_maps_the_source_peak_to_the_target() {
        let s = Settings {
            input_gamut: 2,
            input_transfer: PQ,
            output_gamut: 2,
            output_transfer: 1,
            tone: 1,
            ..Default::default()
        };
        let peak = transform(encode(PQ, [1000. / SDR_WHITE; 3]), &s);
        assert!(peak.iter().all(|v| (v - 1.).abs() < 1e-6), "{peak:?}");
        let grey = transform(encode(PQ, [0.18; 3]), &s);
        assert!(grey.iter().all(|v| (v - 0.18).abs() < 1e-6), "{grey:?}");
        let sat = Settings {
            tone: 3,
            desaturation: 0.,
            ..s
        };
        let c = transform(encode(PQ, [3., 1.5, 0.75]), &sat);
        assert!(
            (c[0] / c[1] - 2.).abs() < 1e-6 && (c[1] / c[2] - 2.).abs() < 1e-6,
            "{c:?}"
        );
    }
}
