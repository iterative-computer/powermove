//! A bounded Live pixel budget. Sustained slow frames lower tracing resolution;
//! a long quiet interval restores it. Output size and Refine/Final stay fixed.
const SCALES: [f32; 5] = [0.5, 0.45, 0.4, 0.375, 1. / 3.];
pub struct LiveBudget {
    level: usize,
    average: f64,
    frames: u32,
    quiet: u32,
    slow: u32,
    fixed: Option<f32>,
    adaptive: bool,
}
impl LiveBudget {
    pub fn new() -> Self {
        Self {
            level: 0,
            average: 0.,
            frames: 0,
            quiet: 0,
            slow: 0,
            adaptive: std::env::var("STUDIO_ADAPTIVE_LIVE").as_deref()!=Ok("0"),
            fixed: std::env::var("STUDIO_LIVE_SCALE")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(1. / 3., 0.5)),
        }
    }
    pub fn reset(&mut self) {
        self.level = 0;
        self.average = 0.;
        self.frames = 0;
        self.quiet = 0;
        self.slow = 0;
    }
    /// Participating media need a conservative initial budget, avoiding any
    /// expensive MetalFX target resize during the first camera drag.
    pub fn atmosphere(&mut self,enabled:bool) {if enabled {self.level=self.level.max(4);}}
    pub fn size(&mut self, width: u32, height: u32, complex: bool) -> (u32, u32) {
        if complex {
            self.level = self.level.max(2);
        }
        let scale = self.fixed.unwrap_or(SCALES[self.level]);
        (
            ((width as f32 * scale).ceil() as u32).max(1),
            ((height as f32 * scale).ceil() as u32).max(1),
        )
    }
    /// Call only for a completed traced frame, excluding cold target creation.
    pub fn observe(&mut self, milliseconds: f64) {
        if !self.adaptive || self.fixed.is_some() || !milliseconds.is_finite() || milliseconds <= 0.
        {
            return;
        }
        self.average = if self.frames == 0 {
            milliseconds
        } else {
            self.average * 0.8 + milliseconds * 0.2
        };
        self.frames += 1;
        self.slow = if self.average > 13. {
            self.slow.saturating_add(1)
        } else {
            0
        };
        let lower = self.slow >= 12;
        self.quiet = if self.average < 8. { self.quiet + 1 } else { 0 };
        let higher = self.quiet >= 120;
        if lower && self.level + 1 < SCALES.len() {
            self.level += 1;
            self.frames = 0;
            self.quiet = 0;
            self.slow = 0;
        } else if higher && self.level > 0 {
            self.level -= 1;
            self.frames = 0;
            self.quiet = 0;
            self.slow = 0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> LiveBudget {
        LiveBudget {
            level: 0,
            average: 0.,
            frames: 0,
            quiet: 0,
            slow: 0,
            fixed: None,
            adaptive: true,
        }
    }
    #[test]
    fn sustained_load_reduces_pixels_without_changing_output_contract() {
        let mut b = budget();
        assert_eq!(b.size(1920, 1080, false), (960, 540));
        for _ in 0..48 {
            b.observe(20.);
        }
        assert_eq!(b.size(1920, 1080, false), (640, 360));
        for _ in 0..200 {
            b.observe(20.);
        }
        assert_eq!(b.size(1920, 1080, false), (640, 360));
        b.reset();
        b.atmosphere(true);assert_eq!(b.size(1920,1080,false),(640,360));
        b.reset();
        assert_eq!(b.size(1920, 1080, true), (768, 432));
    }
    #[test]
    fn a_single_spike_does_not_reallocate_and_recovery_is_slow() {
        let mut b = budget();
        b.observe(50.);
        for _ in 0..11 {
            b.observe(8.);
        }
        assert_eq!(b.size(1920, 1080, false), (960, 540));
        for _ in 0..24 {
            b.observe(20.);
        }
        let reduced = b.size(1920, 1080, false);
        for _ in 0..60 {
            b.observe(7.);
        }
        assert_eq!(b.size(1920, 1080, false), reduced);
        for _ in 0..120 {
            b.observe(7.);
        }
        assert!(b.size(1920, 1080, false).0 > reduced.0);
    }
}
