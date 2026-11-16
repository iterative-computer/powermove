//! Shared Light sketch palette. RGB colours; opacity stays with the paint.
use crate::Tokens;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sketch {
    pub core: u32,
    pub halo: u32,
    pub lit: u32,
    pub pencil: u32,
    pub ink: u32,
    pub wash: u32,
    pub bloom_peak: f64,
    pub dot_pitch: f64,
}
impl Tokens {
    pub fn sketch(&self) -> Sketch {
        Sketch {
            core: crate::color::mix(self.colors.accent, self.colors.text, 0.85),
            halo: self.colors.accent,
            lit: self.colors.accent,
            pencil: self.colors.muted,
            ink: self.colors.accent,
            wash: self.colors.accent_wash,
            bloom_peak: if self.scheme == crate::Scheme::Light {
                0.22
            } else {
                0.58
            },
            dot_pitch: 6.,
        }
    }
}
