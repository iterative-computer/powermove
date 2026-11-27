//! Camera log curves from the vendors' published transforms. Every curve maps
//! a normalized code (the value after the file's range is removed) to scene
//! linear where 0.18 is mid grey. Below the toe each stays on its published
//! lower segment, so sub-black codes keep their value. Above 1.0 each log
//! segment continues, so highlights survive into the working space.
//!
//! | Curve | Source | Gamut |
//! |---|---|---|
//! | Apple Log | Apple Log Profile White Paper (2023) | BT.2020 |
//! | S-Log3 | Sony, Technical Summary for S-Gamut3.Cine/S-Log3 | S-Gamut3, S-Gamut3.Cine |
//! | V-Log | Panasonic, V-Log/V-Gamut Reference Manual | V-Gamut |
//! | LogC3 | ARRI, ALEXA Log C Curve, EI 800 | ARRI Wide Gamut 3 |
//! | LogC4 | ARRI, LogC4 Logarithmic Color Space Specification | ARRI Wide Gamut 4 |

use pm_protocol::color::Transfer;

const APPLE_R0: f64 = -0.05641088;
const APPLE_RT: f64 = 0.01;
const APPLE_C: f64 = 47.28711236;
const APPLE_B: f64 = 0.00964052;
const APPLE_G: f64 = 0.08550479;
const APPLE_D: f64 = 0.69336945;

const VLOG_CUT: f64 = 0.01;
const VLOG_B: f64 = 0.00873;
const VLOG_C: f64 = 0.241514;
const VLOG_D: f64 = 0.598206;

const LOGC3_CUT: f64 = 0.010591;
const LOGC3_A: f64 = 5.555556;
const LOGC3_B: f64 = 0.052272;
const LOGC3_C: f64 = 0.247190;
const LOGC3_D: f64 = 0.385537;
const LOGC3_E: f64 = 5.367655;
const LOGC3_F: f64 = 0.092809;

const LOGC4_A: f64 = (262144.0 - 16.0) / 117.45;
const LOGC4_B: f64 = (1023.0 - 95.0) / 1023.0;
const LOGC4_C: f64 = 95.0 / 1023.0;

fn logc4_s() -> f64 {
    7.0 * core::f64::consts::LN_2 * libm::pow(2.0, 7.0 - 14.0 * LOGC4_C / LOGC4_B)
        / (LOGC4_A * LOGC4_B)
}

fn logc4_t() -> f64 {
    (libm::pow(2.0, 14.0 * (-LOGC4_C / LOGC4_B) + 6.0) - 64.0) / LOGC4_A
}

/// Whether `transfer` is a camera log curve.
pub const fn is_log(transfer: Transfer) -> bool {
    matches!(
        transfer,
        Transfer::AppleLog | Transfer::SLog3 | Transfer::VLog | Transfer::LogC3 | Transfer::LogC4
    )
}

/// A normalized code to scene linear, or `None` for any other transfer.
pub fn decode(transfer: Transfer, code: f64) -> Option<f64> {
    Some(match transfer {
        Transfer::AppleLog => {
            if code < 0.0 {
                APPLE_R0
            } else if code < APPLE_C * (APPLE_RT - APPLE_R0) * (APPLE_RT - APPLE_R0) {
                libm::sqrt(code / APPLE_C) + APPLE_R0
            } else {
                libm::pow(2.0, (code - APPLE_D) / APPLE_G) - APPLE_B
            }
        }
        Transfer::SLog3 => {
            if code >= 171.2102946929 / 1023.0 {
                libm::pow(10.0, (code * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
            } else {
                (code * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0)
            }
        }
        Transfer::VLog => {
            if code < 0.181 {
                (code - 0.125) / 5.6
            } else {
                libm::pow(10.0, (code - VLOG_D) / VLOG_C) - VLOG_B
            }
        }
        Transfer::LogC3 => {
            if code > LOGC3_E * LOGC3_CUT + LOGC3_F {
                (libm::pow(10.0, (code - LOGC3_D) / LOGC3_C) - LOGC3_B) / LOGC3_A
            } else {
                (code - LOGC3_F) / LOGC3_E
            }
        }
        Transfer::LogC4 => {
            if code >= 0.0 {
                (libm::pow(2.0, 14.0 * (code - LOGC4_C) / LOGC4_B + 6.0) - 64.0) / LOGC4_A
            } else {
                code * logc4_s() + logc4_t()
            }
        }
        _ => return None,
    })
}

/// Scene linear to a normalized code, or `None` for any other transfer.
pub fn encode(transfer: Transfer, linear: f64) -> Option<f64> {
    Some(match transfer {
        Transfer::AppleLog => {
            if linear < APPLE_R0 {
                0.0
            } else if linear < APPLE_RT {
                APPLE_C * (linear - APPLE_R0) * (linear - APPLE_R0)
            } else {
                APPLE_G * libm::log2(linear + APPLE_B) + APPLE_D
            }
        }
        Transfer::SLog3 => {
            if linear >= 0.01125 {
                (420.0 + libm::log10((linear + 0.01) / (0.18 + 0.01)) * 261.5) / 1023.0
            } else {
                (linear * (171.2102946929 - 95.0) / 0.01125 + 95.0) / 1023.0
            }
        }
        Transfer::VLog => {
            if linear < VLOG_CUT {
                5.6 * linear + 0.125
            } else {
                VLOG_C * libm::log10(linear + VLOG_B) + VLOG_D
            }
        }
        Transfer::LogC3 => {
            if linear > LOGC3_CUT {
                LOGC3_C * libm::log10(LOGC3_A * linear + LOGC3_B) + LOGC3_D
            } else {
                LOGC3_E * linear + LOGC3_F
            }
        }
        Transfer::LogC4 => {
            let t = logc4_t();
            if linear >= t {
                (libm::log2(LOGC4_A * linear + 64.0) - 6.0) / 14.0 * LOGC4_B + LOGC4_C
            } else {
                (linear - t) / logc4_s()
            }
        }
        _ => return None,
    })
}

/// The shader code a curve is selected with in [`wgsl`] and `source.wgsl`.
pub const fn shader_curve(transfer: Transfer) -> Option<u32> {
    Some(match transfer {
        Transfer::AppleLog => 10,
        Transfer::SLog3 => 11,
        Transfer::VLog => 12,
        Transfer::LogC3 => 13,
        Transfer::LogC4 => 14,
        _ => return None,
    })
}

/// WGSL defining `pm_camera_log_decode(c: f32, curve: u32) -> f32` and
/// `pm_camera_log_decode3`, the f32 form of [`decode`].
pub fn wgsl() -> String {
    format!(
        r#"
fn pm_camera_log_decode(c: f32, curve: u32) -> f32 {{
    switch curve {{
        case 10u: {{
            if (c < 0.0) {{ return {apple_r0:?}; }}
            if (c < {apple_cut:?}) {{ return sqrt(c / {apple_c:?}) + {apple_r0:?}; }}
            return pow(2.0, (c - {apple_d:?}) / {apple_g:?}) - {apple_b:?};
        }}
        case 11u: {{
            if (c >= 171.2102946929 / 1023.0) {{
                return pow(10.0, (c * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01;
            }}
            return (c * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0);
        }}
        case 12u: {{
            if (c < 0.181) {{ return (c - 0.125) / 5.6; }}
            return pow(10.0, (c - {vlog_d:?}) / {vlog_c:?}) - {vlog_b:?};
        }}
        case 13u: {{
            if (c > {logc3_cut:?}) {{
                return (pow(10.0, (c - {logc3_d:?}) / {logc3_c:?}) - {logc3_b:?}) / {logc3_a:?};
            }}
            return (c - {logc3_f:?}) / {logc3_e:?};
        }}
        default: {{
            if (c >= 0.0) {{
                return (pow(2.0, 14.0 * (c - {logc4_c:?}) / {logc4_b:?} + 6.0) - 64.0) / {logc4_a:?};
            }}
            return c * {logc4_s:?} + {logc4_t:?};
        }}
    }}
}}

fn pm_camera_log_decode3(c: vec3<f32>, curve: u32) -> vec3<f32> {{
    return vec3<f32>(
        pm_camera_log_decode(c.x, curve),
        pm_camera_log_decode(c.y, curve),
        pm_camera_log_decode(c.z, curve),
    );
}}
"#,
        apple_r0 = APPLE_R0,
        apple_cut = APPLE_C * (APPLE_RT - APPLE_R0) * (APPLE_RT - APPLE_R0),
        apple_c = APPLE_C,
        apple_d = APPLE_D,
        apple_g = APPLE_G,
        apple_b = APPLE_B,
        vlog_d = VLOG_D,
        vlog_c = VLOG_C,
        vlog_b = VLOG_B,
        logc3_cut = LOGC3_E * LOGC3_CUT + LOGC3_F,
        logc3_d = LOGC3_D,
        logc3_c = LOGC3_C,
        logc3_b = LOGC3_B,
        logc3_a = LOGC3_A,
        logc3_f = LOGC3_F,
        logc3_e = LOGC3_E,
        logc4_c = LOGC4_C,
        logc4_b = LOGC4_B,
        logc4_a = LOGC4_A,
        logc4_s = logc4_s(),
        logc4_t = logc4_t(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [Transfer; 5] = [
        Transfer::AppleLog,
        Transfer::SLog3,
        Transfer::VLog,
        Transfer::LogC3,
        Transfer::LogC4,
    ];

    #[test]
    fn curves_are_monotonic_and_round_trip_over_the_camera_range() {
        for curve in CURVES {
            let mut previous = f64::NEG_INFINITY;
            for i in 0..=4096 {
                let code = f64::from(i) / 4096.0;
                let light = decode(curve, code).unwrap();
                assert!(light >= previous, "{curve:?} at {code}");
                previous = light;
                let back = encode(curve, light).unwrap();
                assert!((back - code).abs() < 1e-9, "{curve:?} {code}: {back}");
            }
        }
    }

    #[test]
    fn segments_meet_at_their_published_breaks() {
        for (curve, break_light) in [
            (Transfer::AppleLog, APPLE_RT),
            (Transfer::SLog3, 0.01125),
            (Transfer::VLog, VLOG_CUT),
            (Transfer::LogC3, LOGC3_CUT),
            (Transfer::LogC4, logc4_t()),
        ] {
            let low = encode(curve, break_light - 1e-9).unwrap();
            let high = encode(curve, break_light + 1e-9).unwrap();
            assert!((low - high).abs() < 1e-5, "{curve:?}: {low} {high}");
        }
    }

    #[test]
    fn other_transfers_have_no_camera_curve() {
        assert!(decode(Transfer::Pq, 0.5).is_none());
        assert!(encode(Transfer::Linear, 0.5).is_none());
        assert!(!is_log(Transfer::Hlg));
    }
}
