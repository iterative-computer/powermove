//! Render-ahead for playback: which frame to render next and when.
//!
//! Frames are rendered `lead` frames before they are shown, so the viewer's
//! jitter buffer holds them until their predicted display time. Frame `k`
//! shows unwrapped media `[k F, (k + 1) F)`; at a negative rate frames are
//! visited in decreasing `k`. A frame whose whole display interval already
//! passed is skipped (an engine-side drop the stats count), and so is a
//! frame that cannot reach the viewer before its interval ends at the
//! render cost measured so far: when a composition costs more than a frame
//! to draw, every frame rendered in order would arrive too late and the
//! viewer would drop them all; skipping to the next frame that can still
//! be shown keeps playback on the clock at a lower frame rate (section 15,
//! drops counted).
//!
//! With the viewer's vsync hint (amendment 18), a frame's render starts at a
//! fixed phase of the display's refresh grid rather than at an arbitrary
//! instant: the first refresh plus `offset` at or after its due time. The
//! offset puts the engine's GPU work just after Chromium's compositor has
//! drawn that refresh, so the two do not contend for the GPU at viz's
//! deadline (S1: viz's draw took 2.6 to 2.7 ms instead of 0.6 ms for frames
//! that reached the display a refresh late while engine frames rendered).
//!
//! An edit during playback ([`Schedule::invalidate`]) makes the frames
//! already rendered ahead stale: they show the older revision. They are
//! rendered again, from the earliest one that can still reach the display,
//! before any new frame, as long as that leaves the next new frame on time;
//! the viewer replaces a queued frame with a newer revision of the same
//! frame (`jitter-buffer.ts`), and keeps the stale one when the new one
//! cannot be ready, so an edit never costs a frame (docs/engine/perf.md).

use super::clock::TransportClock;
use pm_protocol::common::TICKS_PER_SECOND;
use pm_protocol::frame::VsyncHint;

/// Where renders start on the display's refresh grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phase {
    pub hint: VsyncHint,
    /// Fraction of the refresh period after each refresh (BeginFrame).
    pub fraction: f64,
}

impl Phase {
    fn offset_us(&self) -> f64 {
        self.hint.period_us * self.fraction
    }

    /// The phase slot at or before `t`.
    fn slot_at_or_before(&self, t: f64) -> f64 {
        let next = self.hint.next_at_or_after(t, self.offset_us());
        if next > t {
            next - self.hint.period_us
        } else {
            next
        }
    }

    /// How long after a slot a late render may still start in it: a
    /// quarter of the period, which stays clear of the next refresh's
    /// compositor work for offsets up to half a period.
    fn window_us(&self) -> f64 {
        self.hint.period_us / 4.0
    }
}

/// Margin before the first frame of a run: render, GPU, and delivery.
pub const START_MARGIN_US: i64 = 20_000;

/// Time a finished frame needs to reach the display without a vsync hint:
/// delivery to the viewer, its next animation frame, and the two refreshes
/// it presents ahead (bridge.md, about 19 ms at 120 Hz).
pub const DELIVERY_US: f64 = 25_000.0;

/// Weight of the newest render in the cost estimate.
const COST_WEIGHT: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Next {
    /// Render frame `k` now; it is shown from host µs `shown_from`.
    Render { k: i64, shown_from: f64 },
    /// Nothing is due before host µs `at`.
    WaitUntil(f64),
    /// Frame `k`'s display interval passed before it could be rendered.
    Skip { k: i64 },
    /// The clock is stopped.
    Idle,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schedule {
    next_k: Option<i64>,
    /// Render cost of a timed frame (host µs, exponentially weighted).
    cost_us: Option<f64>,
    /// The next stale frame to render again (frames from it up to `next_k`,
    /// exclusive, were rendered at an older revision).
    redo: Option<i64>,
    /// An edit arrived: `redo` starts again from the frame on display.
    redo_pending: bool,
    /// The first frame of this run (nothing before it was rendered).
    start_k: Option<i64>,
}

impl Schedule {
    /// Forgets the position (play, seek, loop change: a clock discontinuity).
    /// The render cost estimate stays: the composition is the same.
    pub fn reset(&mut self) {
        self.next_k = None;
        self.redo = None;
        self.redo_pending = false;
        self.start_k = None;
    }

    /// The document changed: every frame rendered ahead shows an older
    /// revision. They are rendered again (see the module notes).
    pub fn invalidate(&mut self) {
        if self.next_k.is_some() {
            self.redo_pending = true;
        }
    }

    /// Whether rendering frame `k` now renders a stale frame again.
    pub fn redoing(&self, k: i64) -> bool {
        self.redo == Some(k)
    }

    /// A timed frame took `cost_us` from the start of its render until its
    /// GPU work finished.
    pub fn observe(&mut self, cost_us: f64) {
        if !(cost_us.is_finite() && cost_us >= 0.0) {
            return;
        }
        self.cost_us = Some(match self.cost_us {
            Some(c) => c + COST_WEIGHT * (cost_us - c),
            None => cost_us,
        });
    }

    /// The render cost estimate, once a frame was rendered.
    pub fn cost_us(&self) -> Option<f64> {
        self.cost_us
    }

    /// The frame index after `k` in the clock's direction.
    fn step(clock: &TransportClock) -> i64 {
        if clock.rate().num < 0 { -1 } else { 1 }
    }

    /// Host µs at which frame `k` starts and stops showing.
    fn interval(clock: &TransportClock, k: i64) -> Option<(f64, f64)> {
        let f = clock.frame_ticks();
        let a = clock.host_at(k * f)?;
        let b = clock.host_at((k + 1) * f)?;
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// One frame's duration in host µs at the clock's rate.
    pub fn frame_us(clock: &TransportClock) -> f64 {
        let rate = clock.rate();
        if rate.num == 0 {
            return clock.frame_ticks() as f64 / TICKS_PER_SECOND as f64 * 1e6;
        }
        clock.frame_ticks() as f64 / TICKS_PER_SECOND as f64 * 1e6 * f64::from(rate.den)
            / f64::from(rate.num.unsigned_abs())
    }

    /// What to do at host µs `now` with `lead` frames of render-ahead.
    pub fn next(&mut self, clock: &TransportClock, now: i64, lead: u32) -> Next {
        self.next_in_phase(clock, now, lead, None)
    }

    /// [`Self::next`] with renders aligned to a phase of the display's
    /// refresh grid when `phase` is known. A frame due at `d` starts at the
    /// first slot at or after `d`. When the thread gets to it late (woken
    /// late, or waiting for a free surface), it starts at once if the
    /// current slot began less than a quarter period ago or if waiting for
    /// the next slot would leave less than one frame before the frame shows;
    /// otherwise it waits for the next slot.
    pub fn next_in_phase(
        &mut self,
        clock: &TransportClock,
        now: i64,
        lead: u32,
        phase: Option<Phase>,
    ) -> Next {
        if !clock.is_playing() {
            return Next::Idle;
        }
        let f = clock.frame_ticks();
        let fresh = self.next_k.is_none();
        let k = *self.next_k.get_or_insert_with(|| {
            // The first frame due after the margin, but never one before the
            // line's start: during the preroll the line extrapolates
            // backwards, and the viewer keeps the paused frame until then.
            let k = clock.unwrapped_at(now + START_MARGIN_US).div_euclid(f);
            let anchor = clock.anchor_media();
            if clock.rate().num < 0 {
                k.min(anchor.div_euclid(f) + i64::from(anchor.rem_euclid(f) != 0) - 1)
            } else {
                k.max(anchor.div_euclid(f))
            }
        });
        if fresh {
            self.start_k = Some(k);
        }
        let Some((from, until)) = Self::interval(clock, k) else {
            return Next::Idle;
        };
        // Gone, or cannot reach the display before its interval ends.
        let delivery = phase.map_or(DELIVERY_US, |p| {
            (3.0 * p.hint.period_us).max(DELIVERY_US / 2.0)
        });
        let ready = now as f64 + self.cost_us.map_or(0.0, |c| c + delivery);
        if let Some(next) = self.next_redo(clock, now, k, until, ready, phase) {
            return next;
        }
        // The estimate alone skips a frame only while a frame rendered at its
        // due time could still reach the display. An estimate beyond the
        // whole lead window (one render stalled by a GPU or CPU hiccup) would
        // otherwise skip every frame for good: nothing renders, so nothing
        // corrects the estimate. Such frames render, and their measured cost
        // brings the estimate back.
        let showable = self
            .cost_us
            .is_none_or(|c| c + delivery <= f64::from(lead + 1) * Self::frame_us(clock));
        if until < now as f64 || (until < ready && showable) {
            self.next_k = Some(k + Self::step(clock));
            return Next::Skip { k };
        }
        let due = from - f64::from(lead) * Self::frame_us(clock);
        let now_f = now as f64;
        let Some(phase) = phase else {
            if due > now_f {
                return Next::WaitUntil(due);
            }
            return Next::Render {
                k,
                shown_from: from,
            };
        };
        if due > now_f {
            return Next::WaitUntil(phase.hint.next_at_or_after(due, phase.offset_us()));
        }
        let slot = phase.slot_at_or_before(now_f);
        let next_slot = slot + phase.hint.period_us;
        if now_f - slot <= phase.window_us() || from - next_slot < Self::frame_us(clock) {
            return Next::Render {
                k,
                shown_from: from,
            };
        }
        Next::WaitUntil(next_slot)
    }

    /// The frame `k` was rendered; move on.
    pub fn rendered(&mut self, clock: &TransportClock, k: i64) {
        if self.redo == Some(k) {
            self.redo = Some(k + Self::step(clock));
        } else {
            self.next_k = Some(k + Self::step(clock));
        }
    }

    /// The stale frame to render again now (or the slot to wait for), or
    /// `None` when new frames go first. `k` is the next new frame, shown
    /// until `until_k`; `ready` is when a render started now reaches the
    /// display.
    fn next_redo(
        &mut self,
        clock: &TransportClock,
        now: i64,
        k: i64,
        until_k: f64,
        ready: f64,
        phase: Option<Phase>,
    ) -> Option<Next> {
        let step = Self::step(clock);
        let f = clock.frame_ticks();
        if std::mem::take(&mut self.redo_pending) {
            // From the frame on display now (it cannot be replaced in time
            // and is passed over below) to the last one rendered.
            let on_display = clock.unwrapped_at(now).div_euclid(f);
            let first = match self.start_k {
                Some(s) if (s - on_display) * step > 0 => s,
                _ => on_display,
            };
            self.redo = ((k - first) * step > 0).then_some(first);
        }
        let now_f = now as f64;
        while let Some(j) = self.redo {
            if (k - j) * step <= 0 {
                self.redo = None;
                break;
            }
            let Some((from, _)) = Self::interval(clock, j) else {
                self.redo = None;
                break;
            };
            // A replacement must reach the viewer before the frame starts
            // showing: the viewer takes a frame from its queue for the refresh
            // that shows its start, and a copy arriving after that is dropped.
            // Too late to replace: the stale frame stays.
            if from < ready {
                self.redo = Some(j + step);
                continue;
            }
            // New frames first: render a stale one again only if the next
            // new frame can still be ready after it.
            let cost = self.cost_us.unwrap_or(0.0);
            if ready + cost > until_k {
                self.redo = None;
                break;
            }
            let Some(phase) = phase else {
                return Some(Next::Render {
                    k: j,
                    shown_from: from,
                });
            };
            let slot = phase.slot_at_or_before(now_f);
            let next_slot = slot + phase.hint.period_us;
            if now_f - slot <= phase.window_us() || from - next_slot < Self::frame_us(clock) {
                return Some(Next::Render {
                    k: j,
                    shown_from: from,
                });
            }
            return Some(Next::WaitUntil(next_slot));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::clock::TransportCommand;
    use pm_protocol::common::PlaybackRate;

    const FRAME: i64 = TICKS_PER_SECOND / 60;

    fn playing(at: i64, rate: PlaybackRate) -> TransportClock {
        let mut clock = TransportClock::new(FRAME);
        clock.apply(
            TransportCommand::Seek {
                time: pm_protocol::common::Ticks::new(600 * FRAME).unwrap(),
                mode: pm_protocol::clock::SeekMode::Exact,
            },
            at,
        );
        clock.apply(TransportCommand::Play { rate }, at);
        clock
    }

    #[test]
    fn renders_lead_frames_ahead_in_order() {
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        let mut rendered = Vec::new();
        let now = 0;
        loop {
            match schedule.next(&clock, now, 4) {
                Next::Render { k, .. } => {
                    rendered.push(k);
                    schedule.rendered(&clock, k);
                }
                Next::WaitUntil(at) => {
                    // The fifth frame waits until four frames before it shows.
                    let shown = clock
                        .host_at(rendered.last().unwrap() * FRAME + FRAME)
                        .unwrap();
                    assert!(
                        (at - (shown - 4.0 * 16_666.666)).abs() < 1.0,
                        "{at} vs {shown}"
                    );
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        // Frame 600 shows at the end of the 50 ms preroll; a lead of 4
        // frames (66.7 ms) makes 600 and 601 due at once, 602 at 16.7 ms.
        assert_eq!(rendered, vec![600, 601]);
    }

    #[test]
    fn frames_whose_interval_passed_are_skipped() {
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        assert!(matches!(
            schedule.next(&clock, 0, 4),
            Next::Render { k: 600, .. }
        ));
        // A stall: at 101 ms frames 600 to 602 (shown from 50 to 100 ms) are gone.
        assert_eq!(schedule.next(&clock, 101_000, 2), Next::Skip { k: 600 });
        assert_eq!(schedule.next(&clock, 101_000, 2), Next::Skip { k: 601 });
        assert_eq!(schedule.next(&clock, 101_000, 2), Next::Skip { k: 602 });
        assert!(matches!(
            schedule.next(&clock, 101_000, 2),
            Next::Render { k: 603, .. }
        ));
    }

    #[test]
    fn a_composition_that_costs_more_than_a_frame_skips_to_frames_it_can_still_show() {
        // 60 fps (16.7 ms frames) and a render cost of 40 ms: at 101 ms the
        // frames shown until 150 ms cannot be ready in time (101 + 40 + 25 ms).
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        schedule.observe(40_000.0);
        assert_eq!(schedule.cost_us(), Some(40_000.0));
        let mut skipped = Vec::new();
        let k = loop {
            match schedule.next(&clock, 101_000, 4) {
                Next::Skip { k } => skipped.push(k),
                Next::Render { k, .. } => break k,
                other => panic!("{other:?}"),
            }
        };
        // The run looks from 121 ms (frame 604); 604 and 605 end before
        // 166 ms; 606 ends at 166.7 ms.
        assert_eq!(skipped, vec![604, 605]);
        assert_eq!(k, 606);
        // The estimate follows new renders gradually.
        schedule.observe(0.0);
        assert_eq!(schedule.cost_us(), Some(30_000.0));
        // Keeping up, nothing is skipped: frames render a lead ahead.
        let mut schedule = Schedule::default();
        schedule.observe(5_000.0);
        assert!(matches!(
            schedule.next(&clock, 0, 4),
            Next::Render { k: 600, .. }
        ));
    }

    #[test]
    fn one_stalled_render_does_not_skip_every_later_frame() {
        // 60 fps with a lead of 4: a frame rendered at its due time has
        // 83.3 ms to reach the display. One 500 ms render (a GPU stall) puts
        // the estimate past that window for every frame; skipping on it
        // would never render again, so nothing would correct it.
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        schedule.observe(500_000.0);
        let mut rendered = Vec::new();
        let mut now = 0;
        while rendered.len() < 12 && now < 1_000_000 {
            match schedule.next(&clock, now, 4) {
                Next::Render { k, .. } => {
                    rendered.push(k);
                    schedule.rendered(&clock, k);
                    // The renders after the stall cost 5 ms.
                    schedule.observe(5_000.0);
                }
                Next::Skip { k } => panic!("frame {k} skipped on the stalled estimate"),
                Next::WaitUntil(at) => now = at.ceil() as i64,
                Next::Idle => panic!("idle while playing"),
            }
        }
        assert_eq!(rendered, (600..612).collect::<Vec<_>>());
        // The estimate came back below the window, where it skips again
        // only frames that cannot be shown.
        assert!(schedule.cost_us().unwrap() < 58_000.0);
        // A frame whose interval passed during the stall is still skipped.
        let mut stalled = Schedule::default();
        stalled.observe(500_000.0);
        assert!(matches!(
            stalled.next(&clock, 0, 4),
            Next::Render { k: 600, .. }
        ));
        stalled.rendered(&clock, 600);
        assert_eq!(stalled.next(&clock, 101_000, 4), Next::Skip { k: 601 });
    }

    #[test]
    fn reverse_playback_visits_decreasing_frames() {
        let clock = playing(0, PlaybackRate { num: -1, den: 1 });
        let mut schedule = Schedule::default();
        let mut ks = Vec::new();
        for _ in 0..3 {
            if let Next::Render { k, .. } = schedule.next(&clock, 0, 8) {
                ks.push(k);
                schedule.rendered(&clock, k);
            }
        }
        assert_eq!(ks, vec![599, 598, 597]);
    }

    fn phase(fraction: f64) -> Phase {
        Phase {
            hint: VsyncHint {
                host_time: pm_protocol::common::HostMicros::new(3_000).unwrap(),
                period_us: 8_000.0,
            },
            fraction,
        }
    }

    #[test]
    fn with_a_vsync_hint_renders_start_at_the_phase_after_their_due_time() {
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        let p = phase(0.5);
        // Frame 600 shows from 50 ms; with a lead of 1 it is due at 33.3 ms.
        // Refreshes fall at 3 + 8n ms, the phase at 7 + 8n ms: 39 ms.
        assert_eq!(
            schedule.next_in_phase(&clock, 0, 1, Some(p)),
            Next::WaitUntil(39_000.0)
        );
        assert!(matches!(
            schedule.next_in_phase(&clock, 39_000, 1, Some(p)),
            Next::Render { k: 600, .. }
        ));
        schedule.rendered(&clock, 600);
        // 601 is due at 50 ms: the next phase slot is 55 ms.
        assert_eq!(
            schedule.next_in_phase(&clock, 39_100, 1, Some(p)),
            Next::WaitUntil(55_000.0)
        );
        // Without a hint it would start at its due time.
        match schedule.next(&clock, 39_100, 1) {
            Next::WaitUntil(at) => assert!((at - 50_000.0).abs() < 1e-6, "{at}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_late_render_starts_within_a_quarter_period_or_waits_for_the_next_slot() {
        let clock = playing(0, PlaybackRate::NORMAL);
        let p = phase(0.5);
        // Frame 600 (due 33.3 ms with lead 1) looked at 40.5 ms: its slot began
        // at 39 ms, 1.5 ms ago, inside the 2 ms window.
        let mut schedule = Schedule::default();
        assert!(matches!(
            schedule.next_in_phase(&clock, 40_500, 1, Some(p)),
            Next::Render { k: 600, .. }
        ));
        // At 42 ms the window has closed; the next slot (47 ms) still leaves
        // 3 ms before 50 ms, less than a frame: render now.
        let mut schedule = Schedule::default();
        assert!(matches!(
            schedule.next_in_phase(&clock, 42_000, 1, Some(p)),
            Next::Render { k: 600, .. }
        ));
        // With a lead of 4 (due 0 ms, shown at 50 ms) at 12 ms: the slot began
        // at 7 ms, the next one (15 ms) leaves 35 ms: wait for it.
        let mut schedule = Schedule::default();
        assert_eq!(
            schedule.next_in_phase(&clock, 12_000, 4, Some(p)),
            Next::WaitUntil(15_000.0)
        );
    }

    /// Renders what is due at `now` (lead 4, no phase) and returns the frames.
    fn render_due(schedule: &mut Schedule, clock: &TransportClock, now: i64) -> Vec<i64> {
        let mut out = Vec::new();
        loop {
            match schedule.next(clock, now, 4) {
                Next::Render { k, .. } => {
                    out.push(k);
                    schedule.rendered(clock, k);
                }
                Next::Skip { .. } => {}
                _ => return out,
            }
        }
    }

    #[test]
    fn an_edit_renders_the_frames_ahead_again_from_the_first_that_can_still_show() {
        // 60 fps: frame 600 shows from 50 ms (the preroll), 601 from 66.7 ms.
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        schedule.observe(5_000.0);
        assert_eq!(render_due(&mut schedule, &clock, 0), vec![600, 601]);
        assert_eq!(render_due(&mut schedule, &clock, 34_000), vec![602, 603]);
        // An edit at 40 ms: a frame rendered now reaches the viewer at 70 ms
        // (5 ms render, 25 ms delivery), after 600 and 601 start showing;
        // 602 (from 83.3 ms) and 603 can still be replaced.
        schedule.invalidate();
        assert!(matches!(
            schedule.next(&clock, 40_000, 4),
            Next::Render { k: 602, .. }
        ));
        assert!(schedule.redoing(602));
        schedule.rendered(&clock, 602);
        assert!(matches!(
            schedule.next(&clock, 40_000, 4),
            Next::Render { k: 603, .. }
        ));
        schedule.rendered(&clock, 603);
        // Then new frames, in order: 604 is due at 50 ms (shown from 116.7 ms).
        match schedule.next(&clock, 40_000, 4) {
            Next::WaitUntil(at) => assert!((at - 50_000.0).abs() < 1.0, "{at}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            schedule.next(&clock, 50_000, 4),
            Next::Render { k: 604, .. }
        ));
        assert!(!schedule.redoing(604));
    }

    #[test]
    fn an_edit_before_the_run_starts_redoes_only_frames_that_were_rendered() {
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        assert_eq!(render_due(&mut schedule, &clock, 0), vec![600, 601]);
        // At 10 ms the line extrapolates to frame 597; nothing before 600 was rendered.
        schedule.invalidate();
        assert!(matches!(
            schedule.next(&clock, 10_000, 4),
            Next::Render { k: 600, .. }
        ));
        schedule.rendered(&clock, 600);
        assert!(matches!(
            schedule.next(&clock, 10_000, 4),
            Next::Render { k: 601, .. }
        ));
        schedule.rendered(&clock, 601);
        assert!(matches!(
            schedule.next(&clock, 10_000, 4),
            Next::WaitUntil(_)
        ));
        // A seek or a pause forgets the stale frames with the position.
        schedule.invalidate();
        schedule.reset();
        assert!(!schedule.redo_pending && schedule.redo.is_none());
    }

    #[test]
    fn new_frames_go_before_stale_ones_when_both_cannot_be_ready() {
        // 30 ms renders at 60 fps with a lead of 4: the next new frame (604,
        // shown until 133.3 ms) must start by 78 ms; a stale frame first at
        // 55 ms would make it 55 + 30 + 30 + 25 = 140 ms.
        let clock = playing(0, PlaybackRate::NORMAL);
        let mut schedule = Schedule::default();
        schedule.observe(1_000.0);
        assert_eq!(render_due(&mut schedule, &clock, 0), vec![600, 601]);
        assert_eq!(render_due(&mut schedule, &clock, 34_000), vec![602, 603]);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        schedule.observe(30_000.0);
        assert!(schedule.cost_us().unwrap() > 27_000.0);
        schedule.invalidate();
        assert!(matches!(
            schedule.next(&clock, 55_000, 4),
            Next::Render { k: 604, .. }
        ));
        assert!(!schedule.redoing(604));
    }

    #[test]
    fn a_stopped_clock_schedules_nothing() {
        let clock = TransportClock::new(FRAME);
        assert_eq!(Schedule::default().next(&clock, 0, 4), Next::Idle);
    }
}
