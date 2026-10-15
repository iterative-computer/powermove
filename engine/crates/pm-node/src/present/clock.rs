//! The presenter's transport clock (contract 6).
//!
//! The line "at host instant H the viewer shows media time M, advancing at
//! rate R". With a document, pm-audio's device clock is the master (plan
//! section 15): the presenter follows its samples ([`TransportClock::follow`])
//! and republishes them on this line, so the viewer sees one stream of
//! samples whatever drives it. Without an audio device (or for the bridge
//! test source) the line runs on the host clock from the transport
//! commands.
//!
//! Frames are indexed on the unwrapped timeline (`k × frame duration`); a
//! loop range only wraps the media time a frame shows, and only once the
//! run is inside the loop: a run from a playhead before the loop plays into
//! it and then wraps, a run from past its end plays on (as the legacy
//! transport and pm-audio do). A sample carries the loop range only while
//! the line is inside it (contract 6 invariant 2).

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
    /// Unwrapped media where the current run started (play, seek, step).
    run_start: i64,
    /// The end of the composition, where a host-clock run stops.
    end: Option<i64>,
    /// The audio clock epoch this line follows.
    followed: Option<u32>,
    /// Output latency (µs) and sample rate of the audio device followed.
    device: (u32, u32),
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
            run_start: 0,
            end: None,
            followed: None,
            device: (0, 48_000),
        }
    }

    /// A new frame grid (the composition's rate changed).
    pub fn set_frame_ticks(&mut self, frame_ticks: i64) {
        self.frame = frame_ticks.max(1);
    }

    /// Where host-clock runs stop (`None`: they do not).
    pub fn set_end(&mut self, end: Option<i64>) {
        self.end = end.filter(|e| *e > 0);
    }

    pub fn loop_range(&self) -> Option<TickRange> {
        self.loop_range
    }

    /// Whether unwrapped media `media` of the current run is inside the loop.
    fn in_loop(&self, media: i64) -> Option<TickRange> {
        self.loop_range
            .filter(|r| r.is_valid() && self.run_start < r.end.get() && media >= r.start.get())
    }

    /// Follows a sample of the audio device clock (the master clock). A
    /// new epoch (play, seek, a device change) is a discontinuity and
    /// returns true, as does the end of the run (a stopped sample while
    /// this line plays); samples of the same epoch re-anchor the line.
    pub fn follow(&mut self, sample: &ClockSample) -> bool {
        let host = sample.host_time.get() as i64;
        let media = sample.media_time.get();
        if sample.state == TransportState::Stopped || sample.rate.num == 0 {
            if !self.is_playing() || self.followed.is_none() {
                return false;
            }
            self.anchor_media = media;
            self.anchor_host = host;
            self.rate = PlaybackRate::STOPPED;
            self.state = TransportState::Stopped;
            self.followed = None;
            self.discontinuity();
            return true;
        }
        let new_run = self.followed != Some(sample.epoch) || !self.is_playing();
        let unwrapped = match sample.loop_range {
            Some(range) if !new_run && range.is_valid() => {
                // The same run: keep the frame timeline unwrapped across passes.
                let length = range.end.get() - range.start.get();
                let predicted = self.unwrapped_at(host);
                let passes = ((predicted - media) as f64 / length as f64).round() as i64;
                media + passes * length
            }
            _ => media,
        };
        self.anchor_host = host;
        self.anchor_media = unwrapped;
        self.rate = sample.rate;
        self.state = sample.state;
        self.device = (sample.output_latency_us, sample.sample_rate);
        if new_run {
            self.followed = Some(sample.epoch);
            self.run_start = media;
            self.discontinuity();
        }
        new_run
    }

    /// The device's latency and rate from the audio sample this line
    /// follows (informational, contract 6 invariant 3).
    fn device(&self) -> (u32, u32) {
        self.device
    }

    /// Stops the line at `media` (unwrapped) at host µs `now`, without
    /// snapping to a frame: a seek while playing waits here for the audio
    /// clock's new run.
    pub fn hold(&mut self, media: i64, now: i64) {
        self.anchor_media = media;
        self.anchor_host = now;
        self.run_start = media;
        self.rate = PlaybackRate::STOPPED;
        self.state = TransportState::Stopped;
        self.followed = None;
        self.discontinuity();
    }

    /// Whether this line follows the audio clock.
    pub fn following(&self) -> bool {
        self.followed.is_some()
    }

    /// Stops a host-clock run that reached the composition end at host µs
    /// `now` (not while looping). Returns true when it stopped.
    pub fn stop_at_end(&mut self, now: i64) -> bool {
        let Some(end) = self.end else {
            return false;
        };
        if !self.is_playing() || self.followed.is_some() || self.rate.num < 0 {
            return false;
        }
        let at = self.unwrapped_at(now);
        if at < end || self.in_loop(at).is_some() {
            return false;
        }
        self.anchor_media = end;
        self.anchor_host = now;
        self.rate = PlaybackRate::STOPPED;
        self.state = TransportState::Stopped;
        self.discontinuity();
        true
    }

    /// Host µs at which a playing run enters its loop, when it will.
    pub fn loop_entry_at(&self) -> Option<f64> {
        let range = self.loop_range.filter(|r| r.is_valid())?;
        if !self.is_playing() || self.run_start >= range.end.get() || self.rate.num < 0 {
            return None;
        }
        if self.anchor_media >= range.start.get() {
            return None;
        }
        self.host_at(range.start.get())
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

    /// Wraps unwrapped media time into the loop range once the run is
    /// inside it.
    pub fn wrap(&self, media: i64) -> i64 {
        match self.in_loop(media) {
            Some(range) => {
                let start = range.start.get();
                let length = range.end.get() - start;
                start + (media - start).rem_euclid(length)
            }
            None => media,
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
                self.run_start = media;
                self.followed = None;
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
                self.followed = None;
                self.anchor_media = self.snap(media);
                self.anchor_host = now;
                self.rate = PlaybackRate::STOPPED;
                self.state = TransportState::Stopped;
            }
            TransportCommand::Seek { time, mode } => {
                self.anchor_media = match mode {
                    SeekMode::Exact | SeekMode::Scrub => time.get(),
                };
                self.run_start = self.anchor_media;
                self.anchor_host = if self.is_playing() {
                    now + PREROLL_US
                } else {
                    now
                };
            }
            TransportCommand::Step { frames } => {
                let media = self.snap(self.media_at(now));
                self.anchor_media = self.wrap(media + i64::from(frames) * self.frame);
                self.run_start = self.anchor_media;
                self.followed = None;
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
        let playing = self.is_playing();
        let unwrapped = self.unwrapped_at(now);
        let inside = if playing {
            self.in_loop(unwrapped)
        } else {
            None
        };
        let (host, media) = if playing {
            (now.max(0), self.wrap(unwrapped))
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
            output_latency_us: self.device().0,
            sample_rate: self.device().1,
            loop_range: inside,
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

    fn audio(
        epoch: u32,
        state: TransportState,
        host: u64,
        media: i64,
        range: Option<TickRange>,
    ) -> ClockSample {
        ClockSample {
            seq: Seq::new(1).unwrap(),
            epoch,
            state,
            host_time: HostMicros::new(host).unwrap(),
            media_time: Ticks::new(media).unwrap(),
            rate: if state == TransportState::Stopped {
                PlaybackRate::STOPPED
            } else {
                PlaybackRate::NORMAL
            },
            output_latency_us: 12_000,
            sample_rate: 48_000,
            loop_range: range,
        }
    }

    #[test]
    fn the_line_follows_the_audio_clock_across_loop_passes() {
        let range = TickRange {
            start: Ticks::new(60 * FRAME).unwrap(),
            end: Ticks::new(90 * FRAME).unwrap(),
        };
        let mut clock = TransportClock::new(FRAME);
        clock.apply(TransportCommand::SetLoop { range: Some(range) }, 0);
        // The audio starts playing at 0.5 s, heard at host 2 s.
        assert!(clock.follow(&audio(
            5,
            TransportState::Playing,
            2_000_000,
            30 * FRAME,
            None
        )));
        assert!(clock.following());
        assert_eq!(clock.media_at(2_250_000), 45 * FRAME);
        // Before the loop the line is straight and samples carry no range.
        assert_eq!(clock.sample(2_100_000).loop_range, None);
        // It enters the loop (frames 60 to 90) at 1.0 s, host 2.5 s.
        let entry = clock.loop_entry_at().unwrap();
        assert!((entry - 2_500_000.0).abs() < 1.0, "{entry}");
        assert_eq!(clock.media_at(3_000_000), 60 * FRAME);
        // One pass later the audio re-anchors at the loop start (same epoch):
        // the frame timeline stays unwrapped.
        let k_before = clock.unwrapped_at(3_000_000);
        assert!(!clock.follow(&audio(
            5,
            TransportState::Playing,
            3_000_000,
            60 * FRAME,
            Some(range)
        )));
        assert_eq!(clock.unwrapped_at(3_000_000), k_before);
        assert_eq!(clock.sample(3_000_000).loop_range, Some(range));
        // The run ends: the line stops where the audio stopped.
        assert!(clock.follow(&audio(
            5,
            TransportState::Stopped,
            4_000_000,
            75 * FRAME,
            None
        )));
        assert!(!clock.is_playing());
        assert_eq!(clock.media_at(9_000_000), 75 * FRAME);
        assert_eq!(clock.sample(9_000_000).output_latency_us, 12_000);
    }

    #[test]
    fn a_run_from_past_the_loop_does_not_wrap_and_host_runs_stop_at_the_end() {
        let range = TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(30 * FRAME).unwrap(),
        };
        let mut clock = TransportClock::new(FRAME);
        clock.set_end(Some(120 * FRAME));
        clock.apply(TransportCommand::SetLoop { range: Some(range) }, 0);
        clock.apply(
            TransportCommand::Seek {
                time: Ticks::new(60 * FRAME).unwrap(),
                mode: SeekMode::Exact,
            },
            0,
        );
        play(&mut clock, 0);
        let t = PREROLL_US + 500_000;
        assert_eq!(clock.media_at(t), 90 * FRAME);
        assert_eq!(clock.sample(t).loop_range, None);
        assert!(!clock.stop_at_end(t));
        assert!(clock.stop_at_end(PREROLL_US + 1_000_000));
        assert_eq!(clock.media_at(PREROLL_US + 5_000_000), 120 * FRAME);
    }
}
