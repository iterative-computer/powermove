//! Clock samples from the output device (contract 6).
//!
//! The device callback knows two facts about the first frame of each buffer
//! it fills: which media time the frame carries (from the ring's tags) and
//! when the device presents it (the callback's host time plus the output
//! latency). [`ClockCore::observe`] turns them into a [`ClockSample`] line,
//! published on every discontinuity (a new epoch or state) and at least
//! every [`REFRESH_US`] while running, so the line re-anchors to the device
//! clock and drift between the device's sample clock and the host clock
//! never accumulates. [`ClockSlot`] hands samples from the callback to other
//! threads without blocking the callback.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pm_protocol::clock::{ClockSample, TransportState};
use pm_protocol::common::{HostMicros, PlaybackRate, Seq, TICKS_PER_SECOND, TickRange, Ticks};

use crate::ring::Heard;

/// Republication interval while running (contract 6: at least 4 a second).
pub const REFRESH_US: u64 = 100_000;

/// Now, in host microseconds (`mach_absolute_time`, as `CLOCK_UPTIME_RAW`;
/// the domain of `pm_engine::clock::host_now` and of CoreAudio host times).
pub fn host_now_us() -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn clock_gettime_nsec_np(clock_id: libc::clockid_t) -> u64;
        }
        // SAFETY: no preconditions; returns 0 on error.
        unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) / 1_000 }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is valid and writable.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
    }
}

/// Ticks per device frame (exact for every supported rate, which divide the
/// flick).
pub fn ticks_per_frame(rate: u32) -> i64 {
    TICKS_PER_SECOND / rate as i64
}

/// The clock state owned by the device callback.
#[derive(Debug)]
pub struct ClockCore {
    sample_rate: u32,
    /// Shared by every producer of samples for one clock (the device
    /// callback and the control side), so `seq` only grows.
    seq: Arc<AtomicU64>,
    last: Option<ClockSample>,
}

impl ClockCore {
    pub fn new(sample_rate: u32, seq: Arc<AtomicU64>) -> ClockCore {
        ClockCore {
            sample_rate,
            seq,
            last: None,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn last(&self) -> Option<&ClockSample> {
        self.last.as_ref()
    }

    /// The device presents the first frame of a buffer at host time
    /// `presented_us` and it plays `heard`. Returns a sample to publish when
    /// the line changed or is due for a refresh.
    ///
    /// The line wraps by the loop range of the chunk heard (its tag), not by
    /// the transport's: a run from before the loop start is a straight line
    /// until it reaches the loop, and a run past the loop end never wraps.
    pub fn observe(
        &mut self,
        heard: Heard,
        presented_us: u64,
        latency_us: u32,
    ) -> Option<ClockSample> {
        let state = heard.tag.state;
        let rate = if state == TransportState::Stopped {
            PlaybackRate::STOPPED
        } else {
            heard.tag.rate
        };
        // Frames advance the media by the chunk's rate (J/K/L): exact in
        // ticks for every supported rate.
        let media = heard.tag.media_ticks
            + (heard.offset as i128
                * ticks_per_frame(self.sample_rate) as i128
                * heard.tag.rate.num as i128
                / heard.tag.rate.den.max(1) as i128) as i64;
        let loop_range: Option<TickRange> = heard.tag.loop_range;
        if let Some(last) = &self.last {
            let same_line =
                last.epoch == heard.tag.epoch && last.state == state && last.rate == rate;
            // An underrun or a loop wrap moves the media off the line: publish
            // at once (a new `seq` replaces the line).
            let on_line = HostMicros::new(presented_us).is_ok_and(|at| {
                (last.media_time_at(at).get() - media).abs() <= TICKS_PER_SECOND / 1000
            });
            // A new loop range changes how the line wraps: publish it at once.
            let same_loop = last.loop_range == loop_range;
            if same_line && same_loop && on_line && presented_us < last.host_time.get() + REFRESH_US
            {
                return None;
            }
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let sample = ClockSample {
            seq: Seq::new(seq).unwrap_or(Seq::ZERO),
            epoch: heard.tag.epoch,
            state,
            host_time: HostMicros::new(presented_us).unwrap_or(HostMicros::MAX),
            media_time: Ticks::saturating_from_i128(media as i128),
            rate,
            output_latency_us: latency_us,
            sample_rate: self.sample_rate,
            loop_range,
        };
        self.last = Some(sample);
        Some(sample)
    }

    /// A stopped line at `media` (pause, seek while paused), published by
    /// the control side. A stopped line does not loop, so it carries no
    /// loop range.
    pub fn stopped(&mut self, epoch: u32, media: i64, now_us: u64, latency_us: u32) -> ClockSample {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let sample = ClockSample {
            seq: Seq::new(seq).unwrap_or(Seq::ZERO),
            epoch,
            state: TransportState::Stopped,
            host_time: HostMicros::new(now_us).unwrap_or(HostMicros::MAX),
            media_time: Ticks::saturating_from_i128(media as i128),
            rate: PlaybackRate::STOPPED,
            output_latency_us: latency_us,
            sample_rate: self.sample_rate,
            loop_range: None,
        };
        self.last = Some(sample);
        sample
    }

    /// Forgets the last line, so the next observation publishes.
    pub fn invalidate(&mut self) {
        self.last = None;
    }
}

/// Epochs only grow; a callback that read a chunk just before the
/// transport moved on must not publish its old line over the new one.
fn newer(old: Option<ClockSample>, sample: &ClockSample) -> bool {
    old.is_none_or(|old| (old.epoch, old.seq) < (sample.epoch, sample.seq))
}

/// The latest published sample, written by the callback without blocking.
#[derive(Default)]
pub struct ClockSlot {
    latest: Mutex<Option<ClockSample>>,
    /// Bumped on every store.
    version: AtomicU64,
}

impl ClockSlot {
    /// Stores a sample unless another thread holds the slot (the callback
    /// retries on its next buffer). Returns whether it was stored.
    pub fn try_store(&self, sample: ClockSample) -> bool {
        match self.latest.try_lock() {
            Ok(mut g) => {
                if newer(*g, &sample) {
                    *g = Some(sample);
                    self.version.fetch_add(1, Ordering::Release);
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Stores a sample, waiting for the slot (control threads only).
    pub fn store(&self, sample: ClockSample) {
        let mut g = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        if newer(*g, &sample) {
            *g = Some(sample);
            self.version.fetch_add(1, Ordering::Release);
        }
    }

    pub fn latest(&self) -> Option<ClockSample> {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::Tag;

    fn heard(media: i64, epoch: u32, offset: usize) -> Heard {
        looped(media, epoch, offset, None)
    }

    fn looped(media: i64, epoch: u32, offset: usize, loop_range: Option<TickRange>) -> Heard {
        Heard {
            tag: Tag {
                media_ticks: media,
                epoch,
                state: TransportState::Playing,
                frames: 256,
                loop_range,
                rate: PlaybackRate::NORMAL,
            },
            offset,
        }
    }

    /// J/K/L: frames heard at −2× move the line back two frames each, and
    /// a rate change starts a new line at once.
    #[test]
    fn lines_follow_the_chunk_rate() {
        let mut core = ClockCore::new(48_000, Arc::default());
        let mut h = heard(10 * TICKS_PER_SECOND, 1, 0);
        h.tag.rate = PlaybackRate { num: -2, den: 1 };
        h.tag.state = TransportState::Shuttling;
        let a = core.observe(h, 1_000_000, 0).unwrap();
        assert_eq!(a.rate, PlaybackRate { num: -2, den: 1 });
        assert!(a.validate_ok());
        // 480 frames in: 10 s − 2 × 10 ms, on the line.
        let mut later = h;
        later.offset = 480;
        assert!(core.observe(later, 1_010_000, 0).is_none());
        assert_eq!(
            a.media_time_at(HostMicros::new(1_010_000).unwrap()).get(),
            10 * TICKS_PER_SECOND - TICKS_PER_SECOND / 50
        );
        let mut forward = later;
        forward.tag.rate = PlaybackRate { num: 2, den: 1 };
        let b = core.observe(forward, 1_010_000, 0).unwrap();
        assert_eq!(b.rate.num, 2);
    }

    #[test]
    fn publishes_on_discontinuity_and_refresh() {
        let mut core = ClockCore::new(48_000, Arc::default());
        let a = core.observe(heard(0, 1, 0), 1_000_000, 5000).unwrap();
        assert_eq!(a.media_time.get(), 0);
        assert_eq!(a.host_time.get(), 1_000_000);
        // Same line, too soon.
        assert!(core.observe(heard(0, 1, 100), 1_002_083, 5000).is_none());
        // Refresh due.
        let b = core
            .observe(heard(0, 1, 200), 1_000_000 + REFRESH_US, 5000)
            .unwrap();
        assert_eq!(b.media_time.get(), 200 * 14_700);
        assert!(b.seq > a.seq);
        // Off the line (an underrun delayed the media by 20 ms): immediately.
        let d = core
            .observe(
                heard(0, 1, 400),
                1_000_000 + REFRESH_US + 20_000 + 400 * 1_000_000 / 48_000,
                5000,
            )
            .unwrap();
        assert_eq!(d.media_time.get(), 400 * 14_700);
        // New epoch: immediately.
        let c = core
            .observe(
                heard(7 * TICKS_PER_SECOND, 2, 0),
                1_000_000 + REFRESH_US + 10,
                5000,
            )
            .unwrap();
        assert_eq!(c.epoch, 2);
        assert!(c.validate_ok());
    }

    #[test]
    fn a_new_loop_range_publishes_at_once() {
        let mut core = ClockCore::new(48_000, Arc::default());
        let a = core.observe(heard(0, 1, 0), 1_000_000, 5000).unwrap();
        assert!(a.loop_range.is_none());
        // Same line, too soon: nothing new.
        assert!(core.observe(heard(0, 1, 100), 1_002_083, 5000).is_none());
        // The chunks heard enter a loop; the media is still on the line.
        let range = Some(TickRange {
            start: Ticks::new(0).unwrap(),
            end: Ticks::new(TICKS_PER_SECOND).unwrap(),
        });
        let b = core
            .observe(looped(0, 1, 200, range), 1_004_166, 5000)
            .unwrap();
        assert_eq!(b.epoch, a.epoch, "a loop range keeps the epoch");
        assert!(b.seq > a.seq);
        assert_eq!(b.loop_range, range);
        assert!(
            core.observe(looped(0, 1, 300, range), 1_006_250, 5000)
                .is_none()
        );
    }

    /// The end of a run: the stopped sample is published once the ring has
    /// played out, on the transport's next epoch. A callback that observed
    /// the run's last chunk just before, and stores its sample after (a
    /// higher `seq`, on the run's epoch), must not replace the stop.
    #[test]
    fn a_late_sample_of_an_ended_run_does_not_replace_the_stop() {
        let seq = Arc::new(AtomicU64::new(0));
        let mut device = ClockCore::new(48_000, seq.clone());
        let mut control = ClockCore::new(48_000, seq);
        let slot = ClockSlot::default();
        let stop = control.stopped(4, TICKS_PER_SECOND, 2_000_000, 5000);
        slot.store(stop);
        let late = device.observe(heard(0, 3, 0), 2_000_100, 5000).unwrap();
        assert!(late.seq > stop.seq);
        assert!(slot.try_store(late));
        assert_eq!(slot.latest(), Some(stop));
    }

    trait ValidateOk {
        fn validate_ok(&self) -> bool;
    }
    impl ValidateOk for ClockSample {
        fn validate_ok(&self) -> bool {
            use pm_protocol::validate::Validate;
            self.validate().is_ok()
        }
    }
}
