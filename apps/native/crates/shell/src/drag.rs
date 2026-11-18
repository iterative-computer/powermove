//! Panel dragging: the motion constants and the pointer rules that don't
//! need a window (threshold, steady target choice, edge auto-scroll, tab
//! slides). The view shows the layout a drop would give while the drag is in
//! progress; everything that moves for it moves on [`move_spring`].
use crate::layout::{DropPreview, DropTarget};
use pm_design::{
    Tokens,
    motion::{Animated, Clock, POINT_PRECISION, Spring},
};
use std::{
    cell::Cell,
    collections::HashMap,
    time::{Duration, Instant},
};

/// Panels, tabs and extension bodies are at rest this long after a layout
/// change, for a move of about a panel's width (critically damped: no
/// overshoot). Longer moves take a few frames more.
pub const MOVE: f64 = 0.22;
/// The spring's own settling time (to 0.3 % of the way) that brings a
/// 100 pt move fully to rest (a twentieth of a point, half a point a
/// second) inside [`MOVE`]. Most of the way is covered in about 120 ms.
const MOVE_SETTLING: f64 = MOVE * 0.6;
/// The drag chip fades in over this at pick-up.
pub const LIFT: f32 = 0.12;
/// After a release the chip glides to the slot (or home) and the slot's
/// wash fades over this.
pub const SETTLE: f32 = 0.18;
/// A new target is taken once the pointer is this clear of the current one.
pub const HYSTERESIS: f32 = 5.;
/// A drag held this near a scrolling column's top or bottom scrolls it ...
pub const SCROLL_EDGE: f32 = 28.;
/// ... at up to this many points a second, at the very edge.
pub const SCROLL_SPEED: f32 = 720.;
/// The layout re-flows to a new target once the pointer has rested on it
/// this long ...
pub const REST: f32 = 0.1;
/// ... moving no more than this. A sweep across the workspace re-flows
/// nothing on the way; a release never waits.
pub const REST_SLOP: f32 = 6.;
/// A column slot's ring: the accent at this opacity, a hairline ...
pub const SLOT_RING: f32 = 0.6;
/// ... over this much accent tint on the panel in it.
pub const SLOT_TINT: f32 = 0.04;
/// The chip sits this far right of and below the pointer.
pub const CHIP_OFFSET: f32 = 10.;

/// The chip keeps this far inside the window's edges.
pub const CHIP_MARGIN: f32 = 8.;
/// Where a chip of `size` shows for the place `at` beside the pointer, in a
/// window of `window` size: there, or as near as keeps it wholly inside.
pub fn chip_origin(at: (f32, f32), size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let fit = |v: f32, length: f32, most: f32| v.min(most - length - CHIP_MARGIN).max(CHIP_MARGIN);
    (fit(at.0, size.0, window.0), fit(at.1, size.1, window.1))
}
/// The spring every panel move uses. Reduce Motion: jump.
pub fn move_spring(t: &Tokens) -> Spring {
    if t.accessibility.reduce_motion {
        Spring::SNAP
    } else {
        Spring::settling(MOVE_SETTLING)
    }
}
/// Standard ease-out for the timed fades and glides (0–1 in, 0–1 out).
pub fn ease_out(progress: f32) -> f32 {
    1. - (1. - progress.clamp(0., 1.)).powi(3)
}
/// How far a timed transition that began at `since` and lasts `seconds` has
/// come, eased; 1 under Reduce Motion.
pub fn progress(since: Instant, seconds: f32, t: &Tokens) -> f32 {
    if t.accessibility.reduce_motion {
        return 1.;
    }
    ease_out(now().saturating_duration_since(since).as_secs_f32() / seconds)
}

thread_local! {
    static FRAME_TIME: Cell<Option<Instant>> = const { Cell::new(None) };
}
/// The time this module's motion reads: the real clock, or the frame time a
/// headless recording or a test set with [`set_frame_time`].
pub fn now() -> Instant {
    FRAME_TIME.get().unwrap_or_else(Instant::now)
}
/// Pins [`now`] (headless recordings step it a frame at a time); `None`
/// returns to the real clock.
#[doc(hidden)]
pub fn set_frame_time(at: Option<Instant>) {
    FRAME_TIME.set(at);
}
/// Advances a pinned frame time by `seconds`.
#[doc(hidden)]
pub fn advance_frame_time(seconds: f32) {
    if let Some(at) = FRAME_TIME.get() {
        FRAME_TIME.set(Some(at + Duration::from_secs_f32(seconds)));
    }
}

/// Whether a press that has moved from `start` to `at` is a drag yet.
pub fn lifted(start: (f32, f32), at: (f32, f32), threshold: f32) -> bool {
    (at.0 - start.0).hypot(at.1 - start.1) >= threshold
}

/// Whether the pointer at `at` is still resting where it came to rest.
pub fn within_rest(rest: (f32, f32), at: (f32, f32)) -> bool {
    (at.0 - rest.0).hypot(at.1 - rest.1) <= REST_SLOP
}
/// Whether a pointer at rest since `since` has rested long enough at `at`.
pub fn rested(since: Instant, at: Instant) -> bool {
    at.saturating_duration_since(since).as_secs_f32() >= REST
}

/// The target to show at `at`, given the one showing now: `target_at` says
/// what a point would drop on. The current target is kept until the pointer
/// is [`HYSTERESIS`] clear of it on every side, so a pointer on a boundary
/// never flickers between neighbours.
pub fn steady(
    current: Option<DropPreview>,
    at: (f32, f32),
    target_at: impl Fn((f32, f32)) -> Option<DropPreview>,
) -> Option<DropPreview> {
    let here = target_at(at);
    let kind = |p: &Option<DropPreview>| p.map(|p| p.target);
    let showing: Option<DropTarget> = kind(&current);
    if kind(&here) == showing {
        // The same target: keep the preview as first shown.
        return current.or(here);
    }
    let h = HYSTERESIS;
    let near = [(-h, 0.), (h, 0.), (0., -h), (0., h)]
        .into_iter()
        .any(|(dx, dy)| kind(&target_at((at.0 + dx, at.1 + dy))) == showing);
    if near { current } else { here }
}

/// Points a second a column scrolls for a drag at `y`, in a column whose
/// viewport runs `top..bottom`: negative near the top, positive near the
/// bottom, faster the nearer the edge, nothing in between.
pub fn autoscroll(y: f32, top: f32, bottom: f32) -> f32 {
    let edge = SCROLL_EDGE.min((bottom - top) / 3.).max(1.);
    if y < top || y > bottom {
        0.
    } else if y < top + edge {
        -SCROLL_SPEED * (1. - (y - top) / edge)
    } else if y > bottom - edge {
        SCROLL_SPEED * (1. - (bottom - y) / edge)
    } else {
        0.
    }
}

/// Tabs sliding along their strip to the places a layout change gave them
/// (a tab dragged between its neighbours): each tab's horizontal offset from
/// its new place, arriving on [`move_spring`]. A tab that came from another
/// header appears in place.
#[derive(Default)]
pub struct TabSlides {
    /// Where each tab last showed (its left edge and its header's top).
    last: HashMap<String, (f32, f32)>,
    seen: HashMap<String, (f32, f32)>,
    moving: HashMap<String, Animated<1>>,
    clock: Clock,
    /// The next frame follows a discrete change: tabs that moved slide.
    pub armed: bool,
    arming: bool,
}
impl TabSlides {
    /// Start of a frame: steps what is under way.
    pub fn begin(&mut self) {
        let dt = self.clock.tick(now());
        self.moving.retain(|_, slide| slide.step(dt));
        self.arming = std::mem::take(&mut self.armed);
        self.seen.clear();
    }
    /// The offset to draw tab `id` at this frame, given its place
    /// (`x`, with its header's top `y`), in window points.
    pub fn offset(&mut self, id: &str, x: f32, y: f32, t: &Tokens) -> f32 {
        if self.arming
            && let Some(&(was_x, was_y)) = self.last.get(id)
            && (was_y - y).abs() < t.metrics.header / 2.
            && (was_x - x).abs() >= 1.
        {
            let spring = move_spring(t);
            if spring.snaps() {
                self.moving.remove(id);
            } else {
                let mut slide = Animated::new([was_x - x], Spring::SNAP, POINT_PRECISION);
                slide.animate_to([0.], spring);
                self.moving.insert(id.to_owned(), slide);
            }
        }
        let offset = self.moving.get(id).map_or(0., |s| s.value[0]);
        self.seen.insert(id.to_owned(), (x + offset, y));
        offset
    }
    /// Whether any tab is still on its way.
    pub fn travelling(&self) -> bool {
        !self.moving.is_empty()
    }
    /// End of a frame; true while a tab still travels.
    pub fn end(&mut self) -> bool {
        self.last = std::mem::take(&mut self.seen);
        self.moving.retain(|id, _| self.last.contains_key(id));
        if self.moving.is_empty() {
            self.clock.rest();
        }
        !self.moving.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Near, Rect};

    fn preview(group: usize) -> DropPreview {
        DropPreview {
            target: DropTarget::Split { dock: 0, group },
            rect: Rect::default(),
            near: Near::Nothing,
        }
    }
    /// Two zones meeting at x = 100, nothing left of 0.
    fn zones(at: (f32, f32)) -> Option<DropPreview> {
        match at.0 {
            x if x < 0. => None,
            x if x < 100. => Some(preview(0)),
            _ => Some(preview(1)),
        }
    }
    #[test]
    fn a_press_becomes_a_drag_only_past_the_threshold() {
        assert!(!lifted((10., 10.), (10., 10.), 5.), "a click");
        assert!(!lifted((10., 10.), (13., 13.), 5.), "a shaky click");
        assert!(lifted((10., 10.), (13., 14.), 5.), "5 pt away");
        assert!(lifted((10., 10.), (10., 4.), 5.));
    }
    #[test]
    fn a_target_holds_until_the_pointer_is_clear_of_it() {
        let target = |p: Option<DropPreview>| p.map(|p| p.target);
        // Nothing showing: the first target is taken once inside it.
        let first = steady(None, (50., 0.), zones);
        assert_eq!(target(first), target(Some(preview(0))));
        // Just over the boundary: still within reach of the first.
        let held = steady(first, (102., 0.), zones);
        assert_eq!(target(held), target(first));
        // Back and forth on the boundary never changes it.
        for x in [99., 101., 98., 103., 100.] {
            assert_eq!(target(steady(first, (x, 0.), zones)), target(first), "{x}");
        }
        // Clear of it: the neighbour takes over, and holds the same way.
        let second = steady(first, (106., 0.), zones);
        assert_eq!(target(second), target(Some(preview(1))));
        assert_eq!(target(steady(second, (97., 0.), zones)), target(second));
        assert_eq!(target(steady(second, (94., 0.), zones)), target(first));
        // Leaving every target is held the same way, then lets go.
        assert_eq!(target(steady(first, (-3., 0.), zones)), target(first));
        assert_eq!(steady(first, (-6., 0.), zones), None);
    }
    #[test]
    fn the_chip_stays_inside_the_window() {
        let (chip, window) = ((180., 24.), (1440., 900.));
        assert_eq!(chip_origin((500., 300.), chip, window), (500., 300.), "free of the edges");
        assert_eq!(chip_origin((1400., 890.), chip, window), (1252., 868.), "right and bottom");
        assert_eq!(chip_origin((-20., 2.), chip, window), (8., 8.), "left and top");
    }
    #[test]
    fn the_layout_waits_for_the_pointer_to_rest() {
        let start = Instant::now();
        assert!(within_rest((100., 100.), (104., 103.)), "a hand never holds still");
        assert!(!within_rest((100., 100.), (107., 100.)), "moving on");
        assert!(!rested(start, start + Duration::from_millis(40)), "in passing");
        assert!(rested(start, start + Duration::from_millis(110)));
    }
    #[test]
    fn a_drag_near_a_column_edge_scrolls_towards_it() {
        assert_eq!(autoscroll(300., 100., 600.), 0., "the middle is still");
        assert!(autoscroll(110., 100., 600.) < 0., "up near the top");
        assert!(autoscroll(590., 100., 600.) > 0., "down near the bottom");
        assert!(
            autoscroll(101., 100., 600.).abs() > autoscroll(120., 100., 600.).abs(),
            "faster nearer the edge"
        );
        assert!(autoscroll(100., 100., 600.).abs() <= SCROLL_SPEED);
        assert_eq!(autoscroll(90., 100., 600.), 0., "outside the column");
    }
    #[test]
    fn tabs_slide_along_their_strip_and_reduce_motion_places_them() {
        for reduced in [false, true] {
            let t = Tokens::new(
                pm_design::Scheme::Dark,
                pm_design::Accessibility {
                    reduce_motion: reduced,
                    ..Default::default()
                },
            );
            let start = Instant::now();
            set_frame_time(Some(start));
            let mut s = TabSlides::default();
            s.begin();
            assert_eq!(s.offset("a", 100., 40., &t), 0.);
            assert_eq!(s.offset("b", 180., 40., &t), 0.);
            assert!(!s.end());
            // A reorder swaps them: each starts where it was.
            s.armed = true;
            s.begin();
            let (a, b) = (s.offset("a", 180., 40., &t), s.offset("b", 100., 40., &t));
            let moving = s.end();
            if reduced {
                assert_eq!((a, b, moving), (0., 0., false), "placed at once");
            } else {
                assert_eq!((a, b), (-80., 80.));
                assert!(moving);
                // Part of the way after a few frames, there after the move.
                advance_frame_time(0.05);
                s.begin();
                let part = s.offset("a", 180., 40., &t);
                s.offset("b", 100., 40., &t);
                assert!(s.end());
                assert!(part < 0. && part > -80., "{part}");
                for _ in 0..40 {
                    advance_frame_time(1. / 60.);
                    s.begin();
                    s.offset("a", 180., 40., &t);
                    s.offset("b", 100., 40., &t);
                    s.end();
                }
                s.begin();
                assert_eq!(s.offset("a", 180., 40., &t), 0.);
                assert!(!s.end());
            }
            // A tab arriving from another header does not fly in.
            s.armed = true;
            s.begin();
            assert_eq!(s.offset("a", 500., 400., &t), 0.);
            s.end();
            set_frame_time(None);
        }
    }
    #[test]
    fn the_move_spring_never_overshoots() {
        let t = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let mut a = Animated::new([100.], Spring::SNAP, POINT_PRECISION);
        a.animate_to([0.], move_spring(&t));
        // A 100 pt move: never past its place, and at rest within MOVE.
        let mut frames = 0;
        while a.step(1. / 120.) {
            assert!(a.value[0] >= -POINT_PRECISION, "overshoot: {}", a.value[0]);
            frames += 1;
        }
        assert!(frames <= (MOVE * 120.) as usize, "at rest within MOVE: {frames} frames");
    }
}
