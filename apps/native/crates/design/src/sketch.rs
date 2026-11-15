//! Shared Light sketch palette. RGB colours; opacity stays with the paint.
use crate::{Scheme, Tokens};
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
        let light = self.scheme == Scheme::Light;
        Sketch {
            core: 0xfff4ea,
            halo: if light { 0xf0580a } else { 0xffb27a },
            lit: if light { 0xf0580a } else { 0xff9a55 },
            pencil: if light { 0xf0580a } else { self.colors.text },
            ink: 0xf0580a,
            wash: self.colors.accent_wash,
            bloom_peak: if light { 0.22 } else { 0.58 },
            dot_pitch: 6.,
        }
    }
}
