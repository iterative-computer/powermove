//! Explicit effect transforms from main. These do not assign a working space
//! or destructively tone-map imported originals. Signed HDR is preserved until
//! the effect's authored Clip Output control requests clipping.
use crate::primaries;
use pm_protocol::color::Primaries;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Srgb,
    DisplayP3,
    Rec2020Gamma24,
}
impl Profile {
    fn primaries(self) -> Primaries {
        match self {
            Self::Srgb => Primaries::Bt709,
            Self::DisplayP3 => Primaries::DisplayP3,
            Self::Rec2020Gamma24 => Primaries::Bt2020,
        }
    }
}
pub fn decode_transfer(value: f64, profile: Profile) -> f64 {
    let a = value.abs();
    value.signum()
        * match profile {
            Profile::Rec2020Gamma24 => libm::pow(a, 2.4),
            _ if a <= 0.04045 => a / 12.92,
            _ => libm::pow((a + 0.055) / 1.055, 2.4),
        }
}
pub fn encode_transfer(value: f64, profile: Profile) -> f64 {
    let a = value.abs();
    value.signum()
        * match profile {
            Profile::Rec2020Gamma24 => libm::pow(a, 1. / 2.4),
            _ if a <= 0.0031308 => 12.92 * a,
            _ => 1.055 * libm::pow(a, 1. / 2.4) - 0.055,
        }
}
pub fn convert_profile(
    rgb: [f64; 3],
    input: Profile,
    output: Profile,
    linear_input: bool,
    linear_output: bool,
) -> [f64; 3] {
    if input == output && linear_input == linear_output {
        return rgb;
    }
    let linear = rgb.map(|v| {
        if linear_input {
            v
        } else {
            decode_transfer(v, input)
        }
    });
    let result = if input == output {
        linear
    } else {
        primaries::apply(
            &primaries::conversion(input.primaries(), output.primaries())
                .expect("named D65 profiles"),
            linear,
        )
    };
    result.map(|v| {
        if linear_output {
            v
        } else {
            encode_transfer(v, output)
        }
    })
}

/// The effect's premultiplied pixel contract. Amount <=0 and nonpositive alpha
/// preserve the original exactly, including emitted light at zero alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileTransform {
    pub input: Profile,
    pub output: Profile,
    pub linear_input: bool,
    pub linear_output: bool,
    pub clip: bool,
    pub amount: f64,
}
impl ProfileTransform {
    pub fn pixel(self, original: [f64; 4]) -> [f64; 4] {
        let Self {
            input,
            output,
            linear_input,
            linear_output,
            clip,
            amount,
        } = self;
        if original[3] <= 0.
            || amount <= 0.
            || (input == output && linear_input == linear_output && !clip)
        {
            return original;
        }
        let alpha = original[3];
        let rgb = [
            original[0] / alpha,
            original[1] / alpha,
            original[2] / alpha,
        ];
        let converted = convert_profile(rgb, input, output, linear_input, linear_output);
        let amount = (amount / 100.).clamp(0., 1.);
        let mut out = original;
        for i in 0..3 {
            let value = if clip {
                converted[i].clamp(0., 1.)
            } else {
                converted[i]
            };
            out[i] = (rgb[i] * (1. - amount) + value * amount) * alpha;
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompanderMode {
    Compress,
    Expand,
}
pub fn compander_pixel(original: [f64; 4], mode: CompanderMode, gain: f64, gamma: f64) -> [f64; 4] {
    if original[3] <= 0. {
        return original;
    }
    let gain = gain.max(0.001);
    let gamma = gamma.max(0.1);
    if gain == 1. && gamma == 1. {
        return original;
    }
    let mut out = original;
    for i in 0..3 {
        let value = original[i] / original[3];
        out[i] = value.signum()
            * match mode {
                CompanderMode::Compress => libm::pow(value.abs() / gain, 1. / gamma),
                CompanderMode::Expand => libm::pow(value.abs(), gamma) * gain,
            }
            * original[3];
    }
    out
}

/// The legacy preview import decision, exposed separately from the native
/// original's interpretation. HDR precedence is intentional: PQ/HLG plus
/// BT.2020 tone-maps for that SDR preview; wide/log code values are preserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoColorTreatment {
    ToneMap,
    PreserveCodeValues,
}
pub fn video_color_treatment(
    transfer: Option<&str>,
    primaries: Option<&str>,
    matrix: Option<&str>,
) -> Option<VideoColorTreatment> {
    if matches!(transfer, Some("smpte2084" | "arib-std-b67")) {
        Some(VideoColorTreatment::ToneMap)
    } else if primaries == Some("bt2020") || matches!(matrix, Some("bt2020nc" | "bt2020c")) {
        Some(VideoColorTreatment::PreserveCodeValues)
    } else {
        None
    }
}
