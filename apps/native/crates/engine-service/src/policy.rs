//! Frame identities are local to a viewer; document ids never recycle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub display: u32,
    pub headroom: f64,
    /// `width`×`height` is a visible window of a larger canvas (a viewer
    /// zoomed past its pane). `None`: the whole composition.
    pub crop: Option<Crop>,
}
/// The drawable's place on the whole composition canvas, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub canvas_width: u32,
    pub canvas_height: u32,
}
impl Output {
    /// This output drawn into a `width`×`height` drawable. A crop keeps its
    /// place on the canvas, scaled with the drawable, and stays inside it.
    pub fn resized(self, width: u32, height: u32) -> Output {
        if (width, height) == (self.width, self.height) || width == 0 || height == 0 {
            return self;
        }
        let sx = f64::from(width) / f64::from(self.width);
        let sy = f64::from(height) / f64::from(self.height);
        let scaled = |v: u32, s: f64| (f64::from(v) * s).round() as u32;
        Output {
            width,
            height,
            crop: self.crop.map(|c| {
                let (x, y) = (scaled(c.x, sx), scaled(c.y, sy));
                Crop {
                    x,
                    y,
                    canvas_width: scaled(c.canvas_width, sx).max(x + width),
                    canvas_height: scaled(c.canvas_height, sy).max(y + height),
                }
            }),
            ..self
        }
    }
    pub fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0 && self.width <= 16384 && self.height <= 16384,
            "invalid output size"
        );
        if let Some(c) = self.crop {
            anyhow::ensure!(
                c.x.checked_add(self.width)
                    .is_some_and(|r| r <= c.canvas_width)
                    && c.y
                        .checked_add(self.height)
                        .is_some_and(|b| b <= c.canvas_height),
                "crop outside its canvas"
            );
        }
        anyhow::ensure!(
            self.headroom.is_finite() && self.headroom >= 1.,
            "invalid EDR headroom"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub generation: u64,
    pub doc_id: u64,
    pub revision: u64,
    pub sequence: u64,
}
/// What a viewer frame is for. Paused frames are exact; the others may trade
/// accuracy for their deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Moment {
    #[default]
    Paused,
    /// Paused while dragging: video may stand in until its exact frame lands.
    Scrub,
    Playing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub content: u64,
    pub revision: u64,
    pub time: i64,
    pub width: u32,
    pub height: u32,
    pub crop: Option<Crop>,
    pub moment: Moment,
}
impl CacheKey {
    pub fn same_content(self, other: Self) -> bool {
        self.content == other.content
            && self.revision == other.revision
            && self.width == other.width
            && self.height == other.height
            && self.crop == other.crop
    }
}

/// The viewer's render resolution (Auto, Full, ½, ¼ in the viewer chrome).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Auto,
    Full,
    Half,
    Quarter,
}
/// Auto never renders below half resolution; a slower source needs a proxy.
pub const AUTO_FLOOR: f64 = 0.5;
/// Playing frames in the slow-frame window.
const WINDOW: usize = 12;
/// Frames over this share of the budget leave no room for a slow one.
const OVER: f64 = 0.85;
/// A step down after this many slow frames in the window. One is a hiccup.
const SLOW_FRAMES: usize = 3;
/// Half resolution costs about a quarter of the fill, so frames this far under
/// budget at half predict a full-resolution frame inside it.
const UNDER: f64 = 0.2;
/// Quiet frames before trying full resolution, doubled by each failed try.
const FIRST_HOLD: u32 = 120;
const LAST_HOLD: u32 = 1920;

/// The Auto ladder: measured worker-to-completion cost against the frame
/// budget, full resolution down to [`AUTO_FLOOR`] and back with hysteresis.
#[derive(Debug, Clone)]
pub struct AutoQuality {
    scale: f64,
    recent: std::collections::VecDeque<bool>,
    quiet: u32,
    hold: u32,
    tried_full: Option<u32>,
}
impl Default for AutoQuality {
    fn default() -> Self {
        Self {
            scale: 1.,
            recent: Default::default(),
            quiet: 0,
            hold: FIRST_HOLD,
            tried_full: None,
        }
    }
}
impl AutoQuality {
    pub fn scale(&self) -> f64 {
        self.scale
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn record(&mut self, cost_ms: f64, budget_ms: f64) {
        if !cost_ms.is_finite() || !budget_ms.is_finite() || budget_ms <= 0. {
            return;
        }
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(cost_ms > OVER * budget_ms);
        if let Some(frames) = &mut self.tried_full {
            *frames += 1;
            if *frames as usize > WINDOW * 4 {
                // Full resolution held: the next ladder step starts fresh.
                self.tried_full = None;
                self.hold = FIRST_HOLD;
            }
        }
        let slow = self.recent.iter().filter(|s| **s).count();
        if self.scale > AUTO_FLOOR && slow >= SLOW_FRAMES {
            self.scale = AUTO_FLOOR;
            self.recent.clear();
            self.quiet = 0;
            if self.tried_full.take().is_some() {
                self.hold = (self.hold * 2).min(LAST_HOLD);
            }
            return;
        }
        if self.scale < 1. {
            self.quiet = if cost_ms < UNDER * budget_ms {
                self.quiet + 1
            } else {
                0
            };
            if self.quiet >= self.hold {
                self.scale = 1.;
                self.recent.clear();
                self.quiet = 0;
                self.tried_full = Some(0);
            }
        }
    }
}
impl Quality {
    /// Render scale for this frame. Auto is exact whenever the viewer stops.
    pub fn scale(self, auto: &AutoQuality, moment: Moment) -> f64 {
        match self {
            Self::Auto if moment == Moment::Playing => auto.scale(),
            Self::Auto | Self::Full => 1.,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(auto: &mut AutoQuality, frames: usize, cost: f64) {
        for _ in 0..frames {
            auto.record(cost, 16.67);
        }
    }

    #[test]
    fn auto_drops_to_the_half_floor_and_no_further() {
        let mut auto = AutoQuality::default();
        run(&mut auto, 2, 30.);
        assert_eq!(auto.scale(), 1., "two slow frames are a hiccup");
        run(&mut auto, 1, 30.);
        assert_eq!(auto.scale(), AUTO_FLOOR);
        run(&mut auto, 600, 90.);
        assert_eq!(auto.scale(), AUTO_FLOOR, "half resolution is the floor");
    }

    #[test]
    fn auto_returns_to_full_only_after_a_quiet_hold_and_backs_off_a_failed_try() {
        let mut auto = AutoQuality::default();
        run(&mut auto, 3, 30.);
        run(&mut auto, FIRST_HOLD as usize - 1, 2.);
        assert_eq!(auto.scale(), AUTO_FLOOR);
        run(&mut auto, 1, 2.);
        assert_eq!(auto.scale(), 1.);
        // Full resolution does not hold: the next try waits twice as long.
        run(&mut auto, 3, 30.);
        assert_eq!(auto.scale(), AUTO_FLOOR);
        run(&mut auto, FIRST_HOLD as usize, 2.);
        assert_eq!(auto.scale(), AUTO_FLOOR);
        run(&mut auto, FIRST_HOLD as usize, 2.);
        assert_eq!(auto.scale(), 1.);
    }

    #[test]
    fn a_frame_near_budget_at_half_does_not_count_as_quiet() {
        let mut auto = AutoQuality::default();
        run(&mut auto, 3, 30.);
        for _ in 0..2000 {
            auto.record(2., 16.67);
            auto.record(8., 16.67);
        }
        assert_eq!(auto.scale(), AUTO_FLOOR);
    }

    #[test]
    fn stopped_frames_are_full_resolution_in_auto_and_fixed_otherwise() {
        let mut auto = AutoQuality::default();
        run(&mut auto, 3, 30.);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Playing), AUTO_FLOOR);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Paused), 1.);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Scrub), 1.);
        assert_eq!(Quality::Half.scale(&auto, Moment::Paused), 0.5);
        assert_eq!(Quality::Quarter.scale(&auto, Moment::Playing), 0.25);
        assert_eq!(Quality::Full.scale(&auto, Moment::Playing), 1.);
    }

    #[test]
    fn a_crop_must_lie_on_its_canvas() {
        let output = |x, width| Output {
            width,
            height: 100,
            display: 0,
            headroom: 1.,
            crop: Some(Crop {
                x,
                y: 0,
                canvas_width: 400,
                canvas_height: 100,
            }),
        };
        assert!(output(300, 100).validate().is_ok());
        assert!(output(301, 100).validate().is_err());
        assert!(output(u32::MAX, 100).validate().is_err());
    }
}
