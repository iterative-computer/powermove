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
    /// The render scale (`FrameSpec::scale`, bits): with the crop, the
    /// request's composition density and origin.
    pub scale: u64,
    /// The resampler the frame was drawn with.
    pub sampling: pm_render::sampling::Sampling,
    /// The final picture (`FrameSpec::final_picture`), not its preview at
    /// the same size.
    pub finished: bool,
    pub studio_preview: crate::studio::Preview,
    /// `ViewCamera::key` of the pane's camera.
    pub studio_view: u64,
    /// What the frame read besides its document, as observed when it was
    /// drawn: its media files' stamps and the override wrappers answering.
    pub inputs: u64,
    /// The view's output tone it was drawn for (a digest of it).
    pub tone: u64,
    /// The font registry generation it was drawn with.
    pub fonts: Option<u64>,
    /// The resources (contributed definitions) generation it was drawn with.
    pub resources: u64,
}
impl CacheKey {
    pub fn same_content(self, other: Self) -> bool {
        self.content == other.content
            && self.revision == other.revision
            && self.inputs == other.inputs
            && self.tone == other.tone
            && self.fonts == other.fonts
            && self.resources == other.resources
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
/// How long a paused picture has been on screen, its viewer asking for
/// nothing newer, before its settled picture is asked for
/// (`FrameSpec::final_picture`, drawn by `crate::settle`): a click through
/// the timeline or a held arrow key never starts one.
pub const SETTLE_IDLE_SECONDS: f64 = 0.3;
/// The GPU time a slice of a settled picture is sized to: how many units
/// (layers, effect passes) share a slice. Not a bound: a single unit takes
/// what it takes (see `pm_render::slices`).
pub const SETTLE_SLICE_SECONDS: f64 = 0.006;
/// The longest a slice waits for the worker's pictures (a viewer being
/// drawn, a playing viewer's next frame) at one slice point: past it the
/// slice goes, so another viewer playing or being dragged never keeps a
/// paused one from settling (it may then delay one of the playing frames).
pub const SETTLE_HOLD_SECONDS: f64 = 0.05;

/// Playing frames at full resolution slower than their content frame this
/// many times in a row put Auto's playback at [`AUTO_FLOOR`].
const LATE_PLAYS: u32 = 3;
/// At the floor, a playing frame this far under its content frame predicts
/// a full one inside it (half resolution measured at 0.16 to 0.38 of full).
const PLAY_UNDER: f64 = 0.25;
/// ...this many times in a row (about a second at 24 fps) before playback
/// goes back to full resolution: one cheap frame does not flip it back.
const QUICK_PLAYS: u32 = 24;

/// Auto's adaptive resolution (After Effects' Adaptive Resolution). While the
/// playhead is dragged, a full-resolution still that took longer than a
/// refresh puts the dragged frames at [`AUTO_FLOOR`] until an exact frame fits
/// a refresh again. While playing, frames that keep missing their content
/// frame at full resolution play at [`AUTO_FLOOR`] until reduced frames are
/// cheap enough for a full one to fit again. Stopped frames are always full
/// resolution in Auto, and a fixed quality is always what it says.
#[derive(Debug, Clone, Default)]
pub struct AutoQuality {
    slow_stills: bool,
    slow_play: bool,
    /// Consecutive full playing frames over their content frame.
    late_plays: u32,
    /// Consecutive reduced playing frames under [`PLAY_UNDER`].
    quick_plays: u32,
    /// The frames being played draw layers only an extension draws: their
    /// answers are per resolution, and a full-resolution frame draws them
    /// from earlier answers when the extension is slower than playback, so
    /// those frames always play at full resolution.
    overrides: bool,
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
    /// Whether the frames being played draw extension layers (a frame held
    /// for them, or the look-ahead finding them).
    pub fn overrides(&mut self, overrides: bool) {
        self.overrides = overrides;
    }
    /// A playing frame drawn at `scale` took `cost_ms` (start to GPU done)
    /// against its content frame of `frame_ms`.
    pub fn record_play(&mut self, cost_ms: f64, frame_ms: f64, scale: f64) {
        if !cost_ms.is_finite() || !frame_ms.is_finite() || frame_ms <= 0. {
            return;
        }
        if scale >= 1. {
            self.quick_plays = 0;
            self.late_plays = if cost_ms > frame_ms {
                self.late_plays + 1
            } else {
                0
            };
            if self.late_plays >= LATE_PLAYS {
                self.slow_play = true;
                self.late_plays = 0;
            }
        } else if self.slow_play {
            self.late_plays = 0;
            self.quick_plays = if cost_ms < PLAY_UNDER * frame_ms {
                self.quick_plays + 1
            } else {
                0
            };
            if self.quick_plays >= QUICK_PLAYS {
                self.slow_play = false;
                self.quick_plays = 0;
            }
        }
    }
}
impl Quality {
    /// Render scale for this frame. Auto is exact except for a slow drag or
    /// slow playback.
    pub fn scale(self, auto: &AutoQuality, moment: Moment) -> f64 {
        match self {
            Self::Auto if moment == Moment::Scrub && auto.slow_stills => AUTO_FLOOR,
            Self::Auto if moment == Moment::Playing && auto.slow_play && !auto.overrides => {
                AUTO_FLOOR
            }
            Self::Auto | Self::Full => 1.,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Playback in Auto goes to half resolution only after frames keep
    /// missing their content frame, comes back only after a run of cheap
    /// reduced frames, and never touches a stop or a fixed quality.
    #[test]
    fn slow_playback_plays_at_half_with_hysteresis() {
        let frame = 1000. / 24.;
        let mut auto = AutoQuality::default();
        let play = |auto: &AutoQuality| Quality::Auto.scale(auto, Moment::Playing);
        // Two late frames (a first frame, a seek) are not slow playback.
        auto.record_play(120., frame, 1.);
        auto.record_play(120., frame, 1.);
        auto.record_play(20., frame, 1.);
        auto.record_play(120., frame, 1.);
        assert_eq!(play(&auto), 1.);
        auto.record_play(120., frame, 1.);
        auto.record_play(120., frame, 1.);
        assert_eq!(play(&auto), AUTO_FLOOR, "three late frames in a row");
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Paused),
            1.,
            "a stop is exact"
        );
        assert_eq!(
            Quality::Auto.scale(&auto, Moment::Scrub),
            1.,
            "drags keep their own rule"
        );
        for fixed in [Quality::Full, Quality::Half, Quality::Quarter] {
            assert_eq!(
                fixed.scale(&auto, Moment::Playing),
                fixed.scale(&AutoQuality::default(), Moment::Playing),
                "{fixed:?} is always respected"
            );
        }
        // Reduced frames that would not leave room for a full one keep it
        // reduced; a run of cheap ones broken by one dear frame starts over.
        for _ in 0..40 {
            auto.record_play(15., frame, AUTO_FLOOR);
        }
        assert_eq!(play(&auto), AUTO_FLOOR);
        for _ in 0..23 {
            auto.record_play(5., frame, AUTO_FLOOR);
        }
        auto.record_play(15., frame, AUTO_FLOOR);
        for _ in 0..23 {
            auto.record_play(5., frame, AUTO_FLOOR);
        }
        assert_eq!(play(&auto), AUTO_FLOOR);
        auto.record_play(5., frame, AUTO_FLOOR);
        assert_eq!(play(&auto), 1., "a second of cheap reduced frames");
    }

    /// Frames that draw extension layers play at full resolution in Auto
    /// however late they are; without them, slow playback reduces again; a
    /// fixed quality is what it says either way.
    #[test]
    fn frames_with_extension_layers_play_full_in_auto() {
        let frame = 1000. / 24.;
        let mut auto = AutoQuality::default();
        for _ in 0..3 {
            auto.record_play(120., frame, 1.);
        }
        let play = |auto: &AutoQuality| Quality::Auto.scale(auto, Moment::Playing);
        assert_eq!(play(&auto), AUTO_FLOOR);
        auto.overrides(true);
        assert_eq!(play(&auto), 1.);
        assert_eq!(Quality::Half.scale(&auto, Moment::Playing), 0.5);
        auto.overrides(false);
        assert_eq!(play(&auto), AUTO_FLOOR);
    }

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
            "a slow drag does not reduce playback"
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
