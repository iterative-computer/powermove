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
    pub studio_preview: crate::studio::Preview,
    /// `FreeView::key` of the viewer's Free View.
    pub studio_view: u64,
}
impl CacheKey {
    pub fn same_content(self, other: Self) -> bool {
        self.content == other.content
            && self.revision == other.revision
            && self.width == other.width
            && self.height == other.height
            && self.crop == other.crop
            && self.studio_preview == other.studio_preview
            && self.studio_view == other.studio_view
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
/// The resolution Auto drags at when an exact frame is slower than a refresh.
pub const AUTO_FLOOR: f64 = 0.5;
/// Half resolution costs about a quarter of the fill, so a reduced frame this
/// far under a refresh predicts a full-resolution frame inside it.
const UNDER: f64 = 0.2;

/// A dragged playhead waits this long without moving before its frame is
/// drawn again exact (After Effects' adaptive resolution).
pub const SCRUB_SETTLE_SECONDS: f64 = 0.1;

/// Auto's adaptive resolution, which applies only while the playhead is
/// dragged (After Effects' Adaptive Resolution): a full-resolution still that
/// took longer than a refresh puts the dragged frames at [`AUTO_FLOOR`] until
/// an exact frame fits a refresh again. Playback and stopped frames are always
/// full resolution in Auto: a frame that misses its deadline is dropped, never
/// drawn softer than the paused picture.
#[derive(Debug, Clone, Default)]
pub struct AutoQuality {
    slow_stills: bool,
}
impl AutoQuality {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// A paused or dragged frame drawn at `scale` took `cost_ms` against a
    /// refresh of `interval_ms`. A full one over the refresh is slow; a
    /// reduced one far enough under it predicts a full one inside it.
    pub fn record_still(&mut self, cost_ms: f64, interval_ms: f64, scale: f64) {
        if !cost_ms.is_finite() || !interval_ms.is_finite() || interval_ms <= 0. {
            return;
        }
        if scale >= 1. {
            self.slow_stills = cost_ms > interval_ms;
        } else if cost_ms < UNDER * interval_ms {
            self.slow_stills = false;
        }
    }
}
impl Quality {
    /// Render scale for this frame. Auto is exact except for a slow drag.
    pub fn scale(self, auto: &AutoQuality, moment: Moment) -> f64 {
        match self {
            Self::Auto if moment == Moment::Scrub && auto.slow_stills => AUTO_FLOOR,
            Self::Auto | Self::Full => 1.,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_still_drags_at_half_and_stops_exact() {
        let mut auto = AutoQuality::default();
        assert_eq!(Quality::Auto.scale(&auto, Moment::Scrub), 1.);
        auto.record_still(41., 16.67, 1.);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Scrub), AUTO_FLOOR);
        auto.record_still(12., 16.67, AUTO_FLOOR);
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Scrub),
            AUTO_FLOOR,
            "still slow at half"
        );
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Paused),
            1.,
            "a stop is exact"
        );
        assert_eq!(
            Quality::Full.scale(&auto, Moment::Scrub),
            1.,
            "Full means full"
        );
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Playing),
            1.,
            "playback is never reduced in Auto"
        );
        auto.record_still(9., 16.67, 1.);
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Scrub),
            1.,
            "a cheap place drags exact"
        );
        // One slow still (a first frame compiling its programs) costs one
        // reduced drag frame: that one is cheap enough to go back to full.
        auto.record_still(300., 16.67, 1.);
        auto.record_still(0.8, 16.67, AUTO_FLOOR);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Scrub), 1.);
    }

    #[test]
    fn auto_plays_at_full_resolution_however_slow_and_fixed_qualities_stay_fixed() {
        let mut auto = AutoQuality::default();
        // A slow place: drags are reduced, playback and stops are exact.
        auto.record_still(90., 16.67, 1.);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Scrub), AUTO_FLOOR);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Playing), 1.);
        assert_eq!(Quality::Auto.scale(&auto, Moment::Paused), 1.);
        assert_eq!(Quality::Half.scale(&auto, Moment::Paused), 0.5);
        assert_eq!(Quality::Half.scale(&auto, Moment::Playing), 0.5);
        assert_eq!(Quality::Quarter.scale(&auto, Moment::Playing), 0.25);
        assert_eq!(Quality::Full.scale(&auto, Moment::Scrub), 1.);
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
