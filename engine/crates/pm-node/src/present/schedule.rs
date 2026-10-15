//! Render-ahead for playback: which frame to render next and when.
//!
//! Frames are rendered `lead` frames before they are shown, so the viewer's
//! jitter buffer holds them until their predicted display time. Frame `k`
//! shows unwrapped media `[k F, (k + 1) F)`; at a negative rate frames are
//! visited in decreasing `k`. A frame whose whole display interval already
//! passed is skipped (an engine-side drop the stats count).
//!
//! With the viewer's vsync hint (amendment 18), a frame's render starts at a
//! fixed phase of the display's refresh grid rather than at an arbitrary
//! instant: the first refresh plus `offset` at or after its due time. The
//! offset puts the engine's GPU work just after Chromium's compositor has
//! drawn that refresh, so the two do not contend for the GPU at viz's
//! deadline (S1: viz's draw took 2.6 to 2.7 ms instead of 0.6 ms for frames
//! that reached the display a refresh late while engine frames rendered).

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
}

impl Schedule {
    /// Forgets the position (play, seek, loop change: a clock discontinuity).
    pub fn reset(&mut self) {
        self.next_k = None;
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
        let Some((from, until)) = Self::interval(clock, k) else {
            return Next::Idle;
        };
        if until < now as f64 {
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
        self.next_k = Some(k + Self::step(clock));
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

    #[test]
    fn a_stopped_clock_schedules_nothing() {
        let clock = TransportClock::new(FRAME);
        assert_eq!(Schedule::default().next(&clock, 0, 4), Next::Idle);
    }
}
