// Port of distortion-waves 0.1.0 model.ts. Copyright (c) 2026 Jakub Antalik; MIT.
use std::fmt;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveType {
    #[default]
    Vertical,
    Horizontal,
    Center,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Palette {
    #[default]
    Colorful,
    Green,
    Ocean,
    Sunset,
    Mono,
    Single([u8; 3]),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaletteColors {
    pub colors: [[f64; 3]; 9],
    pub count: usize,
    pub ahead: [f64; 3],
    pub behind: [f64; 3],
}
impl Palette {
    pub fn hex(hex: &str) -> Result<Self, ValidationError> {
        let h = hex.strip_prefix('#').unwrap_or(hex);
        if h.len() != 6 || !h.is_ascii() {
            return Err(ValidationError("palette must have six hexadecimal digits"));
        }
        let n = u32::from_str_radix(h, 16).map_err(|_| ValidationError("invalid hex palette"))?;
        Ok(Self::Single([(n >> 16) as u8, (n >> 8) as u8, n as u8]))
    }
    pub fn colors(self) -> PaletteColors {
        let (colors, count, ahead, behind) = match self {
            Self::Colorful => (
                [
                    [255.0, 201.0, 21.0],
                    [126.0, 196.0, 255.0],
                    [180.0, 40.0, 230.0],
                    [235.0, 100.0, 160.0],
                    [255.0, 176.0, 122.0],
                    [154.0, 160.0, 255.0],
                    [127.0, 217.0, 238.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                ],
                7,
                [255.0, 122.0, 182.0],
                [90.0, 190.0, 255.0],
            ),
            Self::Green => (
                [
                    [53.0, 186.0, 0.0],
                    [140.0, 235.0, 90.0],
                    [0.0, 196.0, 110.0],
                    [190.0, 250.0, 130.0],
                    [40.0, 160.0, 60.0],
                    [110.0, 225.0, 160.0],
                    [90.0, 210.0, 40.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                ],
                7,
                [196.0, 255.0, 120.0],
                [0.0, 150.0, 70.0],
            ),
            Self::Ocean => (
                [
                    [40.0, 120.0, 255.0],
                    [90.0, 200.0, 255.0],
                    [20.0, 80.0, 200.0],
                    [120.0, 230.0, 240.0],
                    [60.0, 150.0, 255.0],
                    [150.0, 180.0, 255.0],
                    [0.0, 180.0, 220.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                ],
                7,
                [150.0, 230.0, 255.0],
                [30.0, 70.0, 220.0],
            ),
            Self::Sunset => (
                [
                    [255.0, 140.0, 60.0],
                    [255.0, 90.0, 120.0],
                    [255.0, 200.0, 80.0],
                    [240.0, 70.0, 90.0],
                    [255.0, 170.0, 120.0],
                    [220.0, 90.0, 200.0],
                    [255.0, 120.0, 80.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                ],
                7,
                [255.0, 220.0, 120.0],
                [200.0, 60.0, 160.0],
            ),
            Self::Mono => (
                [
                    [240.0, 240.0, 245.0],
                    [200.0, 205.0, 215.0],
                    [255.0, 255.0, 255.0],
                    [170.0, 175.0, 190.0],
                    [225.0, 228.0, 235.0],
                    [190.0, 195.0, 205.0],
                    [245.0, 245.0, 250.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                ],
                7,
                [255.0, 255.0, 255.0],
                [160.0, 165.0, 180.0],
            ),
            Self::Single(c) => {
                let c = c.map(f64::from);
                let light = |t| c.map(|v| v + (255. - v) * t);
                let dark = |t| c.map(|v| v * (1. - t));
                (
                    [
                        c,
                        light(0.25),
                        c,
                        dark(0.15),
                        [0.; 3],
                        [0.; 3],
                        [0.; 3],
                        [0.; 3],
                        [0.; 3],
                    ],
                    4,
                    light(0.45),
                    dark(0.3),
                )
            }
        };
        PaletteColors {
            colors,
            count,
            ahead,
            behind,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationError(pub &'static str);
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ValidationError {}
/// Lengths use the original 393×852 coordinate system. `for_size` applies auto scale.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveStyle {
    pub kind: WaveType,
    pub palette: Palette,
    pub easing: [f64; 4],
    pub anticipation: f64,
    pub travel: f64,
    pub launch_gap: f64,
    pub linger: f64,
    pub rim_fade: f64,
    pub return_speed: f64,
    pub pull: f64,
    pub inhale: f64,
    pub gather_glow: f64,
    pub gather_dots: f64,
    pub amplitude: f64,
    pub lens_width: f64,
    pub aberration: f64,
    pub swell: f64,
    pub blur: f64,
    pub blur_ahead: f64,
    pub blur_behind: f64,
    pub lead: f64,
    pub tail: f64,
    pub strength: f64,
    pub rim_width: f64,
    pub rim_unevenness: f64,
    pub rim_strength: f64,
    pub rim_softness: f64,
    pub corner_radius: f64,
    pub rim_pulse: f64,
    pub hairline_width: f64,
    pub hairline_opacity: f64,
    pub hairline_white: f64,
    pub hairline_linger: f64,
    pub hairline_fade: f64,
    pub orientation: f64,
    pub particle_count: f64,
    pub particle_size: f64,
    pub particle_reach: f64,
    pub particle_flight: f64,
    pub particle_gap: f64,
    pub particle_life: f64,
    pub particle_spread: f64,
}
impl Default for WaveStyle {
    fn default() -> Self {
        Self::new(WaveType::Vertical)
    }
}
impl WaveStyle {
    pub fn new(kind: WaveType) -> Self {
        let mut s = Self {
            kind,
            palette: Palette::Colorful,
            easing: [0.22, 1., 0.36, 1.],
            anticipation: 0.5,
            travel: 2.28,
            launch_gap: 0.0,
            linger: -0.35,
            rim_fade: 3.03,
            return_speed: 1.45,
            pull: 24.0,
            inhale: 56.0,
            gather_glow: 0.33,
            gather_dots: 1.0,
            amplitude: 34.0,
            lens_width: 150.0,
            aberration: 0.78,
            swell: 22.0,
            blur: 18.0,
            blur_ahead: 90.0,
            blur_behind: 240.0,
            lead: 130.0,
            tail: 300.0,
            strength: 0.92,
            rim_width: 14.0,
            rim_unevenness: 1.0,
            rim_strength: 0.82,
            rim_softness: 343.0,
            corner_radius: 62.0,
            rim_pulse: 0.6,
            hairline_width: 1.5,
            hairline_opacity: 0.9,
            hairline_white: 0.95,
            hairline_linger: 0.47,
            hairline_fade: 3.0,
            orientation: 0.5,
            particle_count: 64.0,
            particle_size: 1.8,
            particle_reach: 150.0,
            particle_flight: 30.0,
            particle_gap: 24.0,
            particle_life: 2.19,
            particle_spread: 1.0,
        };
        if kind == WaveType::Center {
            s.anticipation = 0.25;
            s.travel = 3.43;
            s.rim_fade = 2.51;
            s.pull = 14.4;
            s.inhale = 33.6;
            s.gather_glow = 0.0;
            s.gather_dots = 0.0;
            s.amplitude = 20.4;
            s.lens_width = 90.0;
            s.aberration = 0.0;
            s.swell = 13.2;
            s.blur = 10.8;
            s.blur_ahead = 54.0;
            s.blur_behind = 144.0;
            s.lead = 78.0;
            s.tail = 180.0;
            s.strength = 0.6;
            s.rim_strength = 0.6;
            s.rim_softness = 205.8;
            s.corner_radius = 48.0;
            s.hairline_linger = 0.96;
            s.particle_count = 89.0;
            s.particle_reach = 90.0;
            s.particle_flight = 18.0;
            s.particle_gap = 11.0;
            s.particle_life = 3.93;
            s.particle_spread = 0.64;
            s.palette = Palette::Green;
        }
        s
    }
    pub fn duration(&self) -> f64 {
        self.anticipation
            + (self.travel * 0.6)
                .max(self.linger + self.rim_fade)
                .max(self.linger + self.hairline_linger + self.hairline_fade)
                .max(self.particle_life)
    }
    /// Reject nonfinite or singular shader inputs. The web accepts finite overrides;
    /// this native boundary additionally rejects negative physical lengths and zero divisors.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.anticipation.is_finite() {
            return Err(ValidationError("anticipation must be finite"));
        }
        if !self.travel.is_finite() {
            return Err(ValidationError("travel must be finite"));
        }
        if !self.launch_gap.is_finite() {
            return Err(ValidationError("launchGap must be finite"));
        }
        if !self.linger.is_finite() {
            return Err(ValidationError("linger must be finite"));
        }
        if !self.rim_fade.is_finite() {
            return Err(ValidationError("rimFade must be finite"));
        }
        if !self.return_speed.is_finite() {
            return Err(ValidationError("returnSpeed must be finite"));
        }
        if !self.pull.is_finite() {
            return Err(ValidationError("pull must be finite"));
        }
        if !self.inhale.is_finite() {
            return Err(ValidationError("inhale must be finite"));
        }
        if !self.gather_glow.is_finite() {
            return Err(ValidationError("gatherGlow must be finite"));
        }
        if !self.gather_dots.is_finite() {
            return Err(ValidationError("gatherDots must be finite"));
        }
        if !self.amplitude.is_finite() {
            return Err(ValidationError("amplitude must be finite"));
        }
        if !self.lens_width.is_finite() {
            return Err(ValidationError("lensWidth must be finite"));
        }
        if !self.aberration.is_finite() {
            return Err(ValidationError("aberration must be finite"));
        }
        if !self.swell.is_finite() {
            return Err(ValidationError("swell must be finite"));
        }
        if !self.blur.is_finite() {
            return Err(ValidationError("blur must be finite"));
        }
        if !self.blur_ahead.is_finite() {
            return Err(ValidationError("blurAhead must be finite"));
        }
        if !self.blur_behind.is_finite() {
            return Err(ValidationError("blurBehind must be finite"));
        }
        if !self.lead.is_finite() {
            return Err(ValidationError("lead must be finite"));
        }
        if !self.tail.is_finite() {
            return Err(ValidationError("tail must be finite"));
        }
        if !self.strength.is_finite() {
            return Err(ValidationError("strength must be finite"));
        }
        if !self.rim_width.is_finite() {
            return Err(ValidationError("rimWidth must be finite"));
        }
        if !self.rim_unevenness.is_finite() {
            return Err(ValidationError("rimUnevenness must be finite"));
        }
        if !self.rim_strength.is_finite() {
            return Err(ValidationError("rimStrength must be finite"));
        }
        if !self.rim_softness.is_finite() {
            return Err(ValidationError("rimSoftness must be finite"));
        }
        if !self.corner_radius.is_finite() {
            return Err(ValidationError("cornerRadius must be finite"));
        }
        if !self.rim_pulse.is_finite() {
            return Err(ValidationError("rimPulse must be finite"));
        }
        if !self.hairline_width.is_finite() {
            return Err(ValidationError("hairlineWidth must be finite"));
        }
        if !self.hairline_opacity.is_finite() {
            return Err(ValidationError("hairlineOpacity must be finite"));
        }
        if !self.hairline_white.is_finite() {
            return Err(ValidationError("hairlineWhite must be finite"));
        }
        if !self.hairline_linger.is_finite() {
            return Err(ValidationError("hairlineLinger must be finite"));
        }
        if !self.hairline_fade.is_finite() {
            return Err(ValidationError("hairlineFade must be finite"));
        }
        if !self.orientation.is_finite() {
            return Err(ValidationError("orientation must be finite"));
        }
        if !self.particle_count.is_finite() {
            return Err(ValidationError("particleCount must be finite"));
        }
        if !self.particle_size.is_finite() {
            return Err(ValidationError("particleSize must be finite"));
        }
        if !self.particle_reach.is_finite() {
            return Err(ValidationError("particleReach must be finite"));
        }
        if !self.particle_flight.is_finite() {
            return Err(ValidationError("particleFlight must be finite"));
        }
        if !self.particle_gap.is_finite() {
            return Err(ValidationError("particleGap must be finite"));
        }
        if !self.particle_life.is_finite() {
            return Err(ValidationError("particleLife must be finite"));
        }
        if !self.particle_spread.is_finite() {
            return Err(ValidationError("particleSpread must be finite"));
        }
        if self.easing.iter().any(|v| !v.is_finite()) {
            return Err(ValidationError("easing must be finite"));
        }
        if self.anticipation < 0. {
            return Err(ValidationError("anticipation must be nonnegative"));
        }
        if self.travel < 0. {
            return Err(ValidationError("travel must be nonnegative"));
        }
        if self.launch_gap < 0. {
            return Err(ValidationError("launchGap must be nonnegative"));
        }
        if self.rim_fade < 0. {
            return Err(ValidationError("rimFade must be nonnegative"));
        }
        if self.return_speed < 0. {
            return Err(ValidationError("returnSpeed must be nonnegative"));
        }
        if self.pull < 0. {
            return Err(ValidationError("pull must be nonnegative"));
        }
        if self.inhale < 0. {
            return Err(ValidationError("inhale must be nonnegative"));
        }
        if self.gather_glow < 0. {
            return Err(ValidationError("gatherGlow must be nonnegative"));
        }
        if self.gather_dots < 0. {
            return Err(ValidationError("gatherDots must be nonnegative"));
        }
        if self.amplitude < 0. {
            return Err(ValidationError("amplitude must be nonnegative"));
        }
        if self.lens_width < 0. {
            return Err(ValidationError("lensWidth must be nonnegative"));
        }
        if self.aberration < 0. {
            return Err(ValidationError("aberration must be nonnegative"));
        }
        if self.swell < 0. {
            return Err(ValidationError("swell must be nonnegative"));
        }
        if self.blur < 0. {
            return Err(ValidationError("blur must be nonnegative"));
        }
        if self.blur_ahead < 0. {
            return Err(ValidationError("blurAhead must be nonnegative"));
        }
        if self.blur_behind < 0. {
            return Err(ValidationError("blurBehind must be nonnegative"));
        }
        if self.lead < 0. {
            return Err(ValidationError("lead must be nonnegative"));
        }
        if self.tail < 0. {
            return Err(ValidationError("tail must be nonnegative"));
        }
        if self.strength < 0. {
            return Err(ValidationError("strength must be nonnegative"));
        }
        if self.rim_width < 0. {
            return Err(ValidationError("rimWidth must be nonnegative"));
        }
        if self.rim_unevenness < 0. {
            return Err(ValidationError("rimUnevenness must be nonnegative"));
        }
        if self.rim_strength < 0. {
            return Err(ValidationError("rimStrength must be nonnegative"));
        }
        if self.rim_softness < 0. {
            return Err(ValidationError("rimSoftness must be nonnegative"));
        }
        if self.corner_radius < 0. {
            return Err(ValidationError("cornerRadius must be nonnegative"));
        }
        if self.rim_pulse < 0. {
            return Err(ValidationError("rimPulse must be nonnegative"));
        }
        if self.hairline_width < 0. {
            return Err(ValidationError("hairlineWidth must be nonnegative"));
        }
        if self.hairline_opacity < 0. {
            return Err(ValidationError("hairlineOpacity must be nonnegative"));
        }
        if self.hairline_white < 0. {
            return Err(ValidationError("hairlineWhite must be nonnegative"));
        }
        if self.hairline_linger < 0. {
            return Err(ValidationError("hairlineLinger must be nonnegative"));
        }
        if self.hairline_fade < 0. {
            return Err(ValidationError("hairlineFade must be nonnegative"));
        }
        if self.orientation < 0. {
            return Err(ValidationError("orientation must be nonnegative"));
        }
        if self.particle_count < 0. {
            return Err(ValidationError("particleCount must be nonnegative"));
        }
        if self.particle_size < 0. {
            return Err(ValidationError("particleSize must be nonnegative"));
        }
        if self.particle_reach < 0. {
            return Err(ValidationError("particleReach must be nonnegative"));
        }
        if self.particle_flight < 0. {
            return Err(ValidationError("particleFlight must be nonnegative"));
        }
        if self.particle_gap < 0. {
            return Err(ValidationError("particleGap must be nonnegative"));
        }
        if self.particle_life < 0. {
            return Err(ValidationError("particleLife must be nonnegative"));
        }
        if self.particle_spread < 0. {
            return Err(ValidationError("particleSpread must be nonnegative"));
        }
        if self.travel <= 0. {
            return Err(ValidationError("travel must be positive"));
        }
        if self.return_speed <= 0. {
            return Err(ValidationError("return_speed must be positive"));
        }
        if self.lens_width <= 0. {
            return Err(ValidationError("lens_width must be positive"));
        }
        if self.blur_ahead <= 0. {
            return Err(ValidationError("blur_ahead must be positive"));
        }
        if self.blur_behind <= 0. {
            return Err(ValidationError("blur_behind must be positive"));
        }
        if self.lead <= 0. {
            return Err(ValidationError("lead must be positive"));
        }
        if self.tail <= 0. {
            return Err(ValidationError("tail must be positive"));
        }
        if self.rim_width <= 0. {
            return Err(ValidationError("rim_width must be positive"));
        }
        if self.rim_softness <= 0. {
            return Err(ValidationError("rim_softness must be positive"));
        }
        if self.particle_count > 256. {
            return Err(ValidationError("particleCount exceeds 256"));
        }
        if self.orientation > 1. || self.particle_spread > 1. {
            return Err(ValidationError(
                "orientation and particleSpread must be within 0..=1",
            ));
        }
        Ok(())
    }
    pub fn for_size(&self, width: f64, height: f64, corner_radius: f64) -> Self {
        let k = Self::scale(width, height);
        let mut s = self.clone();
        s.corner_radius = corner_radius / k;
        s.hairline_width /= k.sqrt();
        s.particle_size /= k.sqrt();
        s
    }
    pub fn scale(width: f64, height: f64) -> f64 {
        (width * height / (393. * 852.)).sqrt().clamp(0.6, 2.2)
    }
}
/// sRGB → OKLab, as upstream's palette interpolation (not Display P3 RGB).
pub(crate) fn oklab(c: [f64; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(|v| {
        let x = v / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    });
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        (0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s) as f32,
        (1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s) as f32,
        (0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s) as f32,
    ]
}
