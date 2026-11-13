//! Camera log curves in f64, written from the vendors' documents in their own
//! units and sharing no code with `pm-color`.

/// A vendor curve, code value 0..1 to scene linear with mid grey at 0.18.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    AppleLog,
    SLog3,
    VLog,
    LogC3,
    LogC4,
    /// ACEScct (S-2016-001); linear is ACES scene light.
    AcesCct,
    /// RED Log3G10 (version 2, with its 0.01 offset).
    Log3G10,
    /// Blackmagic Film Generation 5.
    BmdFilm5,
    /// FUJIFILM F-Log.
    FLog,
    /// Nikon N-Log.
    NLog,
}

impl Curve {
    pub const ALL: [Curve; 10] = [
        Curve::AppleLog,
        Curve::SLog3,
        Curve::VLog,
        Curve::LogC3,
        Curve::LogC4,
        Curve::AcesCct,
        Curve::Log3G10,
        Curve::BmdFilm5,
        Curve::FLog,
        Curve::NLog,
    ];

    pub fn to_linear(self, p: f64) -> f64 {
        match self {
            // Apple Log Profile White Paper: P to scene reflectance R.
            Curve::AppleLog => {
                let r0 = -0.05641088;
                let (c, beta, gamma, delta) = (47.28711236, 0.00964052, 0.08550479, 0.69336945);
                let pt = c * (0.01f64 - r0).powi(2);
                if p < 0.0 {
                    r0
                } else if p < pt {
                    (p / c).sqrt() + r0
                } else {
                    ((p - delta) / gamma).exp2() - beta
                }
            }
            // Sony's 10-bit code value form.
            Curve::SLog3 => {
                let cv = p * 1023.0;
                if cv >= 171.2102946929 {
                    10f64.powf((cv - 420.0) / 261.5) * 0.19 - 0.01
                } else {
                    (cv - 95.0) * 0.01125 / 76.2102946929
                }
            }
            // V-Log/V-Gamut Reference Manual, section 3.
            Curve::VLog => {
                if p < 0.181 {
                    (p - 0.125) / 5.6
                } else {
                    10f64.powf((p - 0.598206) / 0.241514) - 0.00873
                }
            }
            // ALEXA Log C Curve, EI 800 row.
            Curve::LogC3 => {
                let (a, b, c, d, e, f) =
                    (5.555556, 0.052272, 0.24719, 0.385537, 5.367655, 0.092809);
                if p > e * 0.010591 + f {
                    (10f64.powf((p - d) / c) - b) / a
                } else {
                    (p - f) / e
                }
            }
            // ARRI LogC4 specification, section 3.
            Curve::LogC4 => {
                let a = (2f64.powi(18) - 16.0) / 117.45;
                let b = 928.0 / 1023.0;
                let c = 95.0 / 1023.0;
                if p >= 0.0 {
                    (2f64.powf(14.0 * (p - c) / b + 6.0) - 64.0) / a
                } else {
                    let s = 7.0 * 2f64.ln() * 2f64.powf(7.0 - 14.0 * c / b) / (a * b);
                    let t = (2f64.powf(14.0 * (-c / b) + 6.0) - 64.0) / a;
                    p * s + t
                }
            }
            // S-2016-001 section 4.4, ACEScct to linear AP1.
            Curve::AcesCct => {
                if p <= 0.155_251_141_552_511 {
                    (p - 0.072_905_534_195_835_5) / 10.540_237_741_654_5
                } else if p < (65504f64.log2() + 9.72) / 17.52 {
                    2f64.powf(p * 17.52 - 9.72)
                } else {
                    65504.0
                }
            }
            // RED's Log3G10 white paper (v2): a, b, c, g.
            Curve::Log3G10 => {
                let (a, b, c, g) = (0.224282, 155.975327, 0.01, 15.1927);
                if p < 0.0 {
                    p / g - c
                } else {
                    (10f64.powf(p / a) - 1.0) / b - c
                }
            }
            // Blackmagic Generation 5 Color Science, Film curve.
            Curve::BmdFilm5 => {
                let (a, b, c) = (
                    0.086_928_760_654_912_24,
                    0.005_494_072_432_257_808,
                    0.530_013_339_229_193_9,
                );
                let (d, e) = (8.283_605_932_402_494, 0.092_465_753_424_657_53);
                if p < d * 0.005 + e {
                    (p - e) / d
                } else {
                    ((p - c) / a).exp() - b
                }
            }
            // F-Log Data Sheet: code value to scene reflection.
            Curve::FLog => {
                let (a, b, c, d, e, f) =
                    (0.555556, 0.009468, 0.344676, 0.790453, 8.735631, 0.092864);
                if p >= 0.100_537_775_223_865 {
                    (10f64.powf((p - d) / c) - b) / a
                } else {
                    (p - f) / e
                }
            }
            // N-Log Specification: 10-bit code value to reflectance.
            Curve::NLog => {
                let cv = p * 1023.0;
                if cv < 452.0 {
                    (cv / 650.0).powi(3) - 0.0075
                } else {
                    ((cv - 619.0) / 150.0).exp()
                }
            }
        }
    }

    /// Bisection on [`Self::to_linear`]: the inverse, by no second formula;
    /// except N-Log, whose specification gives its encoding separately (its
    /// two cuts, 0.328 and code 452, do not meet exactly).
    pub fn from_linear(self, linear: f64) -> f64 {
        if self == Curve::NLog {
            return if linear < 0.328 {
                650.0 * (linear + 0.0075).cbrt()
            } else {
                150.0 * linear.ln() + 619.0
            } / 1023.0;
        }
        let (mut low, mut high) = (-1.0, 2.0);
        for _ in 0..200 {
            let middle = (low + high) / 2.0;
            if self.to_linear(middle) < linear {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(curve: Curve, linear: f64) -> f64 {
        curve.from_linear(linear) * 1023.0
    }

    #[test]
    fn published_code_values() {
        // Sony: 0 %, 18 %, 90 % reflectance at CV 95, 420, 598.
        assert!((code(Curve::SLog3, 0.0) - 95.0).abs() < 0.5);
        assert!((code(Curve::SLog3, 0.18) - 420.0).abs() < 0.5);
        assert!((code(Curve::SLog3, 0.9) - 598.0).abs() < 0.5);
        // Panasonic: 0 %, 18 %, 90 % at CV 128, 433, 602.
        assert!((code(Curve::VLog, 0.0) - 128.0).abs() < 0.5);
        assert!((code(Curve::VLog, 0.18) - 433.0).abs() < 0.5);
        assert!((code(Curve::VLog, 0.9) - 602.0).abs() < 0.5);
        // ARRI LogC3 EI 800: 0 % at CV 95, 18 % at CV 400.
        assert!((code(Curve::LogC3, 0.0) - 95.0).abs() < 0.5);
        assert!((code(Curve::LogC3, 0.18) - 400.0).abs() < 0.5);
        // ARRI LogC4: black at 95 / 1023, 18 % at 0.278, clip at 469.8.
        assert!((code(Curve::LogC4, 0.0) - 95.0).abs() < 1e-6);
        assert!((Curve::LogC4.from_linear(0.18) - 0.2784).abs() < 1e-3);
        assert!((Curve::LogC4.to_linear(1.0) - 469.79).abs() < 0.01);
        // ACES: 0 at 0.0729055, 18 % at 0.4135884 (S-2016-001).
        assert!((Curve::AcesCct.from_linear(0.0) - 0.072_905_5).abs() < 1e-6);
        assert!((Curve::AcesCct.from_linear(0.18) - 0.413_588_4).abs() < 1e-6);
        // RED: 18 % at one third.
        assert!((Curve::Log3G10.from_linear(0.18) - 1.0 / 3.0).abs() < 1e-5);
        // FUJIFILM: 0 %, 18 %, 90 % at CV 95, 470, 705.
        assert!((code(Curve::FLog, 0.0) - 95.0).abs() < 0.5);
        assert!((code(Curve::FLog, 0.18) - 470.0).abs() < 0.5);
        assert!((code(Curve::FLog, 0.9) - 705.0).abs() < 0.5);
        // Apple: 18 % at 48.8 %, 0 % at 15.0 %.
        assert!((Curve::AppleLog.from_linear(0.18) - 0.4883).abs() < 1e-3);
        assert!((Curve::AppleLog.from_linear(0.0) - 0.1505).abs() < 1e-3);
    }
}
