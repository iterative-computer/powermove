//! The presenter's transport clock (contract 6).
//!
//! Until pm-audio drives playback from the audio device clock, the
//! interactive engine runs this host-clock line: "at host instant H the
//! viewer shows media time M, advancing at rate R". It publishes the same
//! `ClockSample`s the audio clock will, so the viewer's jitter buffer and the
//! render-ahead schedule do not change when the device clock takes over.
//!
//! Frames are indexed on the unwrapped timeline (`k × frame duration`); a
//! loop range only wraps the media time a frame shows.

use pm_protocol::clock::{ClockSample, SeekMode, TransportCommand, TransportState};
use pm_protocol::common::{HostMicros, PlaybackRate, Seq, TICKS_PER_SECOND, TickRange, Ticks};

/// How long after a play command the first frame is due: enough to render
/// and deliver a few frames before the line starts moving.
pub const PREROLL_US: i64 = 50_000;

/// Samples go out at least this often while playing (contract 6: 4 a second).
pub const SAMPLE_INTERVAL_US: i64 = 250_000;

#[derive(Debug, Clone, PartialEq)]
pub struct TransportClock {
    epoch: u32,
    seq: u64,
    state: TransportState,
    rate: PlaybackRate,
    /// Host µs at which `anchor_media` is shown.
    anchor_host: i64,
    /// Unwrapped media ticks at `anchor_host`.
    anchor_media: i64,
    loop_range: Option<TickRange>,
    /// One frame in ticks (for steps).
    frame: i64,
}

impl TransportClock {
    pub fn new(frame_ticks: i64) -> Self {
        Self {
            epoch: 1,
            seq: 0,
            state: TransportState::Stopped,
            rate: PlaybackRate::STOPPED,
            anchor_host: 0,
            anchor_media: 0,
            loop_range: None,
            frame: frame_ticks.max(1),
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn is_playing(&self) -> bool {
        self.state != TransportState::Stopped && self.rate.num != 0
    }

    pub fn rate(&self) -> PlaybackRate {
        self.rate
    }

    pub fn frame_ticks(&self) -> i64 {
        self.frame
    }

    /// The unwrapped media time where the line starts (play, seek).
    pub fn anchor_media(&self) -> i64 {
        self.anchor_media
    }

    /// Unwrapped media ticks at host µs `at`.
    pub fn unwrapped_at(&self, at: i64) -> i64 {
        if !self.is_playing() {
            return self.anchor_media;
        }
        let elapsed = i128::from(at - self.anchor_host);
        let ticks = (elapsed * i128::from(TICKS_PER_SECOND) * i128::from(self.rate.num))
            .div_euclid(1_000_000 * i128::from(self.rate.den));
        (i128::from(self.anchor_media) + ticks).clamp(i128::from(i64::MIN), i128::from(i64::MAX))
            as i64
    }

    /// Host µs at which unwrapped media time `media` is shown (playing only).
    pub fn host_at(&self, media: i64) -> Option<f64> {
        if !self.is_playing() {
            return None;
        }
        let ticks = (media - self.anchor_media) as f64;
        let rate = f64::from(self.rate.num) / f64::from(self.rate.den);
        Some(self.anchor_host as f64 + ticks / TICKS_PER_SECOND as f64 * 1e6 / rate)
    }

    /// Wraps unwrapped media time into the loop range.
    pub fn wrap(&self, media: i64) -> i64 {
        match self.loop_range {
            Some(range) if range.is_valid() => {
                let start = range.start.get();
                let length = range.end.get() - start;
                start + (media - start).rem_euclid(length)
            }
            _ => media,
        }
    }

    /// Media ticks shown at host µs `at` (wrapped).
    pub fn media_at(&self, at: i64) -> i64 {
        self.wrap(self.unwrapped_at(at))
    }

    fn discontinuity(&mut self) {
        self.epoch = self.epoch.wrapping_add(1).max(1);
    }

    /// Applies a transport command at host µs `now`. Returns whether the
    /// line changed (a sample is due).
    pub fn apply(&mut self, command: TransportCommand, now: i64) -> bool {
        match command {
            TransportCommand::Play { rate } if rate.num != 0 && rate.den > 0 => {
                let media = self.media_at(now);
                self.anchor_media = media;
                self.anchor_host = now + PREROLL_US;
                self.rate = rate;
                self.state = if rate == PlaybackRate::NORMAL {
                    TransportState::Playing
                } else {
                    TransportState::Shuttling
                };
            }
            TransportCommand::Play { .. } => return false,
            TransportCommand::Pause => {
                let media = self.media_at(now);
                self.anchor_media = self.snap(media);
                self.anchor_host = now;
                self.rate = PlaybackRate::STOPPED;
                self.state = TransportState::Stopped;
            }
            TransportCommand::Seek { time, mode } => {
                self.anchor_media = match mode {
                    SeekMode::Exact | SeekMode::Scrub => time.get(),
                };
                self.anchor_host = if self.is_playing() {
                    now + PREROLL_US
                } else {
                    now
                };
            }
            TransportCommand::Step { frames } => {
                let media = self.snap(self.media_at(now));
                self.anchor_media = self.wrap(media + i64::from(frames) * self.frame);
                self.anchor_host = now;
                self.rate = PlaybackRate::STOPPED;
                self.state = TransportState::Stopped;
            }
            TransportCommand::SetLoop { range } => {
                let media = self.media_at(now);
                self.anchor_media = media;
                self.anchor_host = now;
                self.loop_range = range.filter(TickRange::is_valid);
                if self.loop_range.is_some() {
                    self.anchor_media = self.wrap(self.anchor_media);
                }
            }
            TransportCommand::PlayAround { .. } | TransportCommand::AudioScrub { .. } => {
                return false;
            }
        }
        self.discontinuity();
        true
    }

    /// The start of the frame containing `media`.
    pub fn snap(&self, media: i64) -> i64 {
        media.div_euclid(self.frame) * self.frame
    }

    /// The sample describing the line at host µs `now`.
    pub fn sample(&mut self, now: i64) -> ClockSample {
        self.seq += 1;
        let (host, media) = if self.is_playing() {
            (now.max(0), self.media_at(now))
        } else {
            (now.max(0), self.anchor_media)
        };
        ClockSample {
            seq: Seq::new(self.seq).unwrap_or(Seq::MAX),
            epoch: self.epoch,
            state: if self.is_playing() {
                self.state
            } else {
                TransportState::Stopped
            },
            host_time: HostMicros::new(host as u64).unwrap_or(HostMicros::ZERO),
            media_time: Ticks::new(media).unwrap_or(Ticks::ZERO),
            rate: if self.is_playing() {
                self.rate
            } else {
                PlaybackRate::STOPPED
            },
            output_latency_us: 0,
            sample_rate: 48_000,
            loop_range: self.loop_range,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: i64 = TICKS_PER_SECOND / 60;

    fn play(clock: &mut TransportClock, now: i64) {
        assert!(clock.apply(
            TransportCommand::Play {
                rate: PlaybackRate::NORMAL
            },
            now
        ));
    }

    #[test]
    fn play_starts_after_the_preroll_and_advances_in_real_time() {
        let mut clock = TransportClock::new(FRAME);
        play(&mut clock, 1_000_000);
        assert_eq!(clock.media_at(1_000_000 + PREROLL_US), 0);
        assert_eq!(
            clock.media_at(1_000_000 + PREROLL_US + 1_000_000),
            TICKS_PER_SECOND
        );
        let host = clock.host_at(30 * FRAME).unwrap();
        assert!((host - (1_000_000.0 + PREROLL_US as f64 + 500_000.0)).abs() < 1e-6);
    }

    #[test]
    fn samples_agree_with_the_contract_math() {
        let mut clock = TransportClock::new(FRAME);
        play(&mut clock, 2_000_000);
        let sample = clock.sample(2_500_000);
        for at in [2_600_000u64, 3_000_000, 9_123_457] {
            assert_eq!(
                sample.media_time_at(HostMicros::new(at).unwrap()).get(),
                clock.media_at(at as i64),
                "at {at}"
            );
        }
        assert!(sample.epoch > 1);
        assert!(pm_protocol::validate::Validate::validate(&sample).is_ok());
    }

    #[test]
    fn pause_lands_on_a_frame_and_stops_the_line() {
        let mut clock = TransportClock::new(FRAME);
        play(&mut clock, 0);
        clock.apply(TransportCommand::Pause, PREROLL_US + 1_010_000);
        assert!(!clock.is_playing());
        assert_eq!(clock.media_at(99_000_000), 60 * FRAME);
        let sample = clock.sample(99_000_000);
        assert_eq!(sample.state, TransportState::Stopped);
        assert_eq!(sample.rate, PlaybackRate::STOPPED);
        assert!(pm_protocol::validate::Validate::validate(&sample).is_ok());
    }

    #[test]
    fn seek_and_step_move_the_paused_frame() {
        let mut clock = TransportClock::new(FRAME);
        clock.apply(
            TransportCommand::Seek {
                time: Ticks::new(10 * FRAME + 5).unwrap(),
                mode: SeekMode::Exact,
            },
            0,
        );
        assert_eq!(clock.media_at(5), 10 * FRAME + 5);
        clock.apply(TransportCommand::Step { frames: -3 }, 10);
        assert_eq!(clock.media_at(10), 7 * FRAME);
    }

    #[test]
    fn a_loop_wraps_media_but_not_the_frame_timeline() {
        let mut clock = TransportClock::new(FRAME);
        clock.apply(
            TransportCommand::SetLoop {
                range: Some(TickRange {
                    start: Ticks::ZERO,
                    end: Ticks::new(60 * FRAME).unwrap(),
                }),
            },
            0,
        );
        play(&mut clock, 0);
        let t = PREROLL_US + 1_500_000;
        assert_eq!(clock.unwrapped_at(t), 90 * FRAME);
        assert_eq!(clock.media_at(t), 30 * FRAME);
        let sample = clock.sample(t);
        assert_eq!(
            sample
                .media_time_at(HostMicros::new(t as u64 + 100_000).unwrap())
                .get(),
            clock.media_at(t + 100_000)
        );
    }

    #[test]
    fn reverse_shuttle_runs_backwards() {
        let mut clock = TransportClock::new(FRAME);
        clock.apply(
            TransportCommand::Seek {
                time: Ticks::new(120 * FRAME).unwrap(),
                mode: SeekMode::Exact,
            },
            0,
        );
        clock.apply(
            TransportCommand::Play {
                rate: PlaybackRate { num: -2, den: 1 },
            },
            0,
        );
        assert_eq!(clock.media_at(PREROLL_US + 500_000), 60 * FRAME);
        assert_eq!(clock.sample(0).state, TransportState::Shuttling);
    }
}
