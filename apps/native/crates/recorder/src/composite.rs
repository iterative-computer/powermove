/// HTML Canvas compositing operations; blend functions follow Compositing and Blending Level 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Composite {
    #[default]
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}
impl Composite {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "source-over" => Self::SourceOver,
            "source-in" => Self::SourceIn,
            "source-out" => Self::SourceOut,
            "source-atop" => Self::SourceAtop,
            "destination-over" => Self::DestinationOver,
            "destination-in" => Self::DestinationIn,
            "destination-out" => Self::DestinationOut,
            "destination-atop" => Self::DestinationAtop,
            "lighter" => Self::Lighter,
            "copy" => Self::Copy,
            "xor" => Self::Xor,
            "multiply" => Self::Multiply,
            "screen" => Self::Screen,
            "overlay" => Self::Overlay,
            "darken" => Self::Darken,
            "lighten" => Self::Lighten,
            "color-dodge" => Self::ColorDodge,
            "color-burn" => Self::ColorBurn,
            "hard-light" => Self::HardLight,
            "soft-light" => Self::SoftLight,
            "difference" => Self::Difference,
            "exclusion" => Self::Exclusion,
            "hue" => Self::Hue,
            "saturation" => Self::Saturation,
            "color" => Self::Color,
            "luminosity" => Self::Luminosity,
            _ => return None,
        })
    }
    pub(crate) fn unbounded(self) -> bool {
        matches!(
            self,
            Self::SourceIn
                | Self::SourceOut
                | Self::DestinationIn
                | Self::DestinationAtop
                | Self::Copy
        )
    }
    pub(crate) fn apply(self, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
        let (sa, da) = (s[3], d[3]);
        let factors = match self {
            Self::SourceOver => Some((1., 1. - sa)),
            Self::SourceIn => Some((da, 0.)),
            Self::SourceOut => Some((1. - da, 0.)),
            Self::SourceAtop => Some((da, 1. - sa)),
            Self::DestinationOver => Some((1. - da, 1.)),
            Self::DestinationIn => Some((0., sa)),
            Self::DestinationOut => Some((0., 1. - sa)),
            Self::DestinationAtop => Some((1. - da, sa)),
            Self::Copy => Some((1., 0.)),
            Self::Xor => Some((1. - da, 1. - sa)),
            Self::Lighter => Some((1., 1.)),
            _ => None,
        };
        if let Some((fs, fd)) = factors {
            return std::array::from_fn(|i| (s[i] * fs + d[i] * fd).clamp(0., 1.));
        }
        let cs = std::array::from_fn(|i| if sa > 0. { s[i] / sa } else { 0. });
        let cb = std::array::from_fn(|i| if da > 0. { d[i] / da } else { 0. });
        let blend = match self {
            Self::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
            Self::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
            Self::Color => set_lum(cs, lum(cb)),
            Self::Luminosity => set_lum(cb, lum(cs)),
            _ => std::array::from_fn(|i| self.separable(cb[i], cs[i])),
        };
        [
            (1. - da) * s[0] + (1. - sa) * d[0] + sa * da * blend[0],
            (1. - da) * s[1] + (1. - sa) * d[1] + sa * da * blend[1],
            (1. - da) * s[2] + (1. - sa) * d[2] + sa * da * blend[2],
            sa + da - sa * da,
        ]
    }
    fn separable(self, b: f32, s: f32) -> f32 {
        match self {
            Self::Multiply => b * s,
            Self::Screen => b + s - b * s,
            Self::Overlay => {
                if b <= 0.5 {
                    2. * b * s
                } else {
                    1. - 2. * (1. - b) * (1. - s)
                }
            }
            Self::Darken => b.min(s),
            Self::Lighten => b.max(s),
            Self::ColorDodge => {
                if b == 0. {
                    0.
                } else if s == 1. {
                    1.
                } else {
                    (b / (1. - s)).min(1.)
                }
            }
            Self::ColorBurn => {
                if b == 1. {
                    1.
                } else if s == 0. {
                    0.
                } else {
                    1. - ((1. - b) / s).min(1.)
                }
            }
            Self::HardLight => {
                if s <= 0.5 {
                    2. * b * s
                } else {
                    1. - 2. * (1. - b) * (1. - s)
                }
            }
            Self::SoftLight => {
                if s <= 0.5 {
                    b - (1. - 2. * s) * b * (1. - b)
                } else {
                    let v = if b <= 0.25 {
                        ((16. * b - 12.) * b + 4.) * b
                    } else {
                        libm::sqrtf(b)
                    };
                    b + (2. * s - 1.) * (v - b)
                }
            }
            Self::Difference => (b - s).abs(),
            Self::Exclusion => b + s - 2. * b * s,
            _ => unreachable!(),
        }
    }
}
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn sat(c: [f32; 3]) -> f32 {
    c.into_iter().fold(f32::NEG_INFINITY, f32::max) - c.into_iter().fold(f32::INFINITY, f32::min)
}
fn set_lum(mut c: [f32; 3], l: f32) -> [f32; 3] {
    let delta = l - lum(c);
    for v in &mut c {
        *v += delta;
    }
    let (lo, hi) = (
        c.into_iter().fold(f32::INFINITY, f32::min),
        c.into_iter().fold(f32::NEG_INFINITY, f32::max),
    );
    if lo < 0. {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - lo);
        }
    }
    if hi > 1. {
        for v in &mut c {
            *v = l + (*v - l) * (1. - l) / (hi - l);
        }
    }
    c
}
fn set_sat(mut c: [f32; 3], s: f32) -> [f32; 3] {
    let mut ids = [0, 1, 2];
    ids.sort_by(|a, b| c[*a].total_cmp(&c[*b]));
    let [lo, mid, hi] = ids;
    if c[hi] > c[lo] {
        c[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        c[hi] = s;
    } else {
        c[mid] = 0.;
        c[hi] = 0.;
    }
    c[lo] = 0.;
    c
}
