//! The transport (contract 6 commands, section 15): play, pause, seek,
//! loop, frame steps, play around, and audio scrub, as a state machine over
//! project sample positions. The render thread asks it what to render next;
//! every discontinuity starts a new epoch.
//!
//! Phase 2 plays at 1× only. J/K/L at 2× and in reverse is Phase 3b; a
//! `Play` at another rate is refused with [`AudioError::Unsupported`].
//!
//! Audio scrub (open question 2 of contract 6, decided here): scrub audio
//! rides on `Shuttling` samples. A scrub seek or a frame step while paused,
//! with audio scrub on, plays a short grain at the new position: one
//! composition frame or [`GRAIN_MIN_MS`], whichever is longer, with
//! [`GRAIN_FADE_MS`] fades so grains never click.

use pm_protocol::clock::{SeekMode, TransportCommand, TransportState};
use pm_protocol::common::{TICKS_PER_SECOND, TickRange};
use pm_time::Rate;

use crate::error::{AudioError, Result};
use crate::offline::sample_of_ticks;

pub const GRAIN_MIN_MS: i64 = 60;
pub const GRAIN_FADE_MS: i64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Stopped,
    Playing {
        /// Play around: stop here and return to `back`.
        until: Option<(i64, i64)>,
    },
    Grain {
        start: i64,
        end: i64,
        back: i64,
    },
}

/// What the render thread renders next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: i64,
    pub frames: usize,
    pub state: TransportState,
    pub epoch: u32,
    /// A scrub grain `[start, end)`: fade in and out at its edges.
    pub grain: Option<(i64, i64)>,
    /// The loop range (sample-aligned, in ticks) when the segment starts
    /// inside it and the run wraps at its end: the clock line of what is
    /// heard from this segment on wraps by it (contract 6). `None` before
    /// the loop start, past the loop end, in a play around, and in a grain,
    /// where the line runs straight.
    pub loop_range: Option<TickRange>,
}

/// What a command changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Nothing audible changed (for example a loop set while paused).
    None,
    /// A new epoch: drop queued audio and pre-roll from the new position.
    Restart,
    /// Playback stopped: publish a stopped clock at the position.
    Stopped,
}

#[derive(Clone, Debug)]
pub struct Transport {
    /// Project sample rate.
    rate: u32,
    /// The next sample to render while playing; the playhead while stopped.
    position: i64,
    mode: Mode,
    epoch: u32,
    loop_samples: Option<(i64, i64)>,
    loop_ticks: Option<TickRange>,
    audio_scrub: bool,
    /// The composition frame rate (frame steps, grain length).
    frame_rate: Option<Rate>,
    /// Where playback stops when not looping (the composition end).
    end: Option<i64>,
}

impl Transport {
    pub fn new(rate: u32) -> Transport {
        Transport {
            rate,
            position: 0,
            mode: Mode::Stopped,
            epoch: 1,
            loop_samples: None,
            loop_ticks: None,
            audio_scrub: false,
            frame_rate: None,
            end: None,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    pub fn position(&self) -> i64 {
        self.position
    }
    pub fn state(&self) -> TransportState {
        match self.mode {
            Mode::Stopped => TransportState::Stopped,
            Mode::Playing { .. } => TransportState::Playing,
            Mode::Grain { .. } => TransportState::Shuttling,
        }
    }
    pub fn is_running(&self) -> bool {
        self.mode != Mode::Stopped
    }
    pub fn loop_range(&self) -> Option<TickRange> {
        self.loop_ticks
    }
    pub fn audio_scrub(&self) -> bool {
        self.audio_scrub
    }

    /// Moves the playhead without a new epoch (playback syncs the transport
    /// to what is heard before a pause, a step, or a play around).
    pub fn set_position(&mut self, sample: i64) {
        self.position = sample;
    }

    /// The composition's frame rate and end (seconds as ticks).
    pub fn set_composition(&mut self, frame_rate: Option<Rate>, end_ticks: Option<i64>) {
        self.frame_rate = frame_rate;
        self.end = end_ticks.map(|t| sample_of_ticks(t, self.rate));
    }

    /// Media ticks of a project sample (exact for supported rates).
    pub fn ticks_of(&self, sample: i64) -> i64 {
        (sample as i128 * TICKS_PER_SECOND as i128 / self.rate as i128) as i64
    }

    /// The loop the audio wraps at, `[a, b)` in samples, as ticks: the
    /// clock wraps where the audio does, every pass exactly as long.
    fn loop_ticks_of(&self, a: i64, b: i64) -> TickRange {
        use pm_protocol::common::Ticks;
        TickRange {
            start: Ticks::saturating_from_i128(self.ticks_of(a) as i128),
            end: Ticks::saturating_from_i128(self.ticks_of(b) as i128),
        }
    }

    fn sample_of(&self, ticks: i64) -> i64 {
        sample_of_ticks(ticks, self.rate)
    }

    fn restart(&mut self) -> Effect {
        self.epoch += 1;
        Effect::Restart
    }

    fn grain_len(&self) -> i64 {
        let min = self.rate as i64 * GRAIN_MIN_MS / 1000;
        let frame = self
            .frame_rate
            .map(|r| (self.rate as i64 * r.den() + r.num() - 1) / r.num())
            .unwrap_or(0);
        min.max(frame)
    }

    fn start_grain(&mut self, at: i64) -> Effect {
        let end = at + self.grain_len();
        self.mode = Mode::Grain {
            start: at,
            end,
            back: at,
        };
        self.position = at;
        self.restart()
    }

    pub fn apply(&mut self, command: &TransportCommand) -> Result<Effect> {
        use pm_protocol::validate::Validate;
        command
            .validate()
            .map_err(|v| AudioError::invalid(format!("{v:?}")))?;
        Ok(match *command {
            TransportCommand::Play { rate } => {
                if rate.num as i64 != rate.den as i64 {
                    return Err(AudioError::Unsupported(format!(
                        "playback at {}/{}× (J/K/L shuttle is Phase 3b)",
                        rate.num, rate.den
                    )));
                }
                if let Mode::Grain { back, .. } = self.mode {
                    self.position = back;
                }
                if let (Some(end), None) = (self.end, self.loop_samples)
                    && self.position >= end
                {
                    self.position = 0;
                }
                self.mode = Mode::Playing { until: None };
                self.restart()
            }
            TransportCommand::Pause => match self.mode {
                Mode::Stopped => Effect::None,
                Mode::Grain { back, .. } => {
                    self.mode = Mode::Stopped;
                    self.position = back;
                    self.epoch += 1;
                    Effect::Stopped
                }
                Mode::Playing { .. } => {
                    self.mode = Mode::Stopped;
                    self.epoch += 1;
                    Effect::Stopped
                }
            },
            TransportCommand::Seek { time, mode } => {
                let at = self.sample_of(time.get());
                match self.mode {
                    Mode::Playing { .. } => {
                        self.position = at;
                        self.mode = Mode::Playing { until: None };
                        self.restart()
                    }
                    _ if mode == SeekMode::Scrub && self.audio_scrub => self.start_grain(at),
                    _ => {
                        self.mode = Mode::Stopped;
                        self.position = at;
                        self.epoch += 1;
                        Effect::Stopped
                    }
                }
            }
            TransportCommand::Step { frames } => {
                let Some(rate) = self.frame_rate else {
                    return Err(AudioError::invalid(
                        "a frame step needs the composition frame rate",
                    ));
                };
                let here = match self.mode {
                    Mode::Grain { back, .. } => back,
                    _ => self.position,
                };
                let frame = rate.frame_of(self.ticks_of(here)) + frames as i64;
                let ticks = rate
                    .frame_start(frame)
                    .map_err(|e| AudioError::invalid(e.to_string()))?;
                let at = self.sample_of(ticks).max(0);
                if self.audio_scrub && !matches!(self.mode, Mode::Playing { .. }) {
                    self.start_grain(at)
                } else {
                    self.position = at;
                    if matches!(self.mode, Mode::Playing { .. }) {
                        self.restart()
                    } else {
                        self.epoch += 1;
                        Effect::Stopped
                    }
                }
            }
            TransportCommand::SetLoop { range } => {
                self.loop_ticks = range;
                self.loop_samples =
                    range.map(|r| (self.sample_of(r.start.get()), self.sample_of(r.end.get())));
                if let Some((a, b)) = self.loop_samples
                    && b <= a
                {
                    self.loop_samples = None;
                    self.loop_ticks = None;
                }
                // The loop applies from the next chunk; queued audio stays.
                Effect::None
            }
            TransportCommand::PlayAround {
                pre_roll,
                post_roll,
            } => {
                let here = match self.mode {
                    Mode::Grain { back, .. } => back,
                    _ => self.position,
                };
                let from = (here - self.sample_of(pre_roll.get())).max(0);
                let until = here + self.sample_of(post_roll.get());
                self.position = from;
                self.mode = Mode::Playing {
                    until: Some((until, here)),
                };
                self.restart()
            }
            TransportCommand::AudioScrub { enabled } => {
                self.audio_scrub = enabled;
                Effect::None
            }
        })
    }

    /// The next segment to render, at most `max` frames, or `None` when
    /// stopped. Segments end at the loop end, a play-around end, a grain
    /// end, and the composition end, so each is linear in media time. When
    /// a run ends, the transport stops (`Effect::Stopped` from
    /// [`Self::take_stop`]).
    pub fn next(&mut self, max: usize) -> Option<Segment> {
        let state = self.state();
        match self.mode {
            Mode::Stopped => None,
            Mode::Grain { start, end, .. } => {
                if self.position >= end {
                    return None;
                }
                let n = ((end - self.position) as usize).min(max);
                let seg = Segment {
                    start: self.position,
                    frames: n,
                    state,
                    epoch: self.epoch,
                    grain: Some((start, end)),
                    loop_range: None,
                };
                self.position += n as i64;
                Some(seg)
            }
            Mode::Playing { until } => loop {
                let looping = match (until, self.loop_samples) {
                    (None, Some((a, b))) if self.position < b => Some((a, b)),
                    _ => None,
                };
                let limit = match (until, looping, self.end) {
                    (Some((u, _)), _, _) => u,
                    (None, Some((_, b)), _) => b,
                    (None, None, Some(e)) => e,
                    (None, None, None) => i64::MAX,
                };
                if self.position >= limit {
                    if let Some((a, b)) = looping.or(self.loop_samples.filter(|_| until.is_none()))
                        && limit == b
                        && b > a
                    {
                        self.position = a;
                        continue;
                    }
                    return None;
                }
                // A segment before the loop start (a run from an earlier
                // playhead) ends at it, so the range starts on the first
                // sample inside the loop, however short the loop.
                let entry = looping
                    .filter(|&(a, _)| self.position < a)
                    .map_or(limit, |(a, _)| a);
                let n = ((entry.min(limit) - self.position) as usize).min(max);
                // Only a segment inside the loop wraps; one before it is a
                // straight line.
                let loop_range = looping
                    .filter(|&(a, _)| self.position >= a)
                    .map(|(a, b)| self.loop_ticks_of(a, b));
                let seg = Segment {
                    start: self.position,
                    frames: n,
                    state,
                    epoch: self.epoch,
                    grain: None,
                    loop_range,
                };
                self.position += n as i64;
                if let Some((a, b)) = looping
                    && self.position >= b
                {
                    self.position = a;
                }
                return Some(seg);
            },
        }
    }

    /// After [`Self::next`] returned `None` while running: the run ended
    /// (composition end, play-around end, grain end). Stops, restores the
    /// play-around or grain position, and reports it.
    pub fn take_stop(&mut self) -> Effect {
        match self.mode {
            Mode::Stopped => Effect::None,
            Mode::Grain { back, .. } => {
                self.mode = Mode::Stopped;
                self.position = back;
                self.epoch += 1;
                Effect::Stopped
            }
            Mode::Playing { until } => {
                self.mode = Mode::Stopped;
                if let Some((_, back)) = until {
                    self.position = back;
                }
                self.epoch += 1;
                Effect::Stopped
            }
        }
    }

    /// Gain of a grain at a sample (fade in and out).
    pub fn grain_gain(&self, grain: (i64, i64), sample: i64) -> f32 {
        let fade = (self.rate as i64 * GRAIN_FADE_MS / 1000).max(1) as f32;
        let a = (sample - grain.0) as f32 / fade;
        let b = (grain.1 - sample) as f32 / fade;
        a.min(b).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{PlaybackRate, Ticks};

    fn t(s: f64) -> Ticks {
        Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
    }

    #[test]
    fn plays_from_the_playhead_and_loops_sample_exactly() {
        let mut tr = Transport::new(48_000);
        tr.apply(&TransportCommand::Seek {
            time: t(1.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
        tr.apply(&TransportCommand::SetLoop {
            range: Some(TickRange {
                start: t(1.0),
                end: t(1.01),
            }),
        })
        .unwrap();
        assert_eq!(
            tr.apply(&TransportCommand::Play {
                rate: PlaybackRate::NORMAL
            })
            .unwrap(),
            Effect::Restart
        );
        let mut covered = vec![];
        for _ in 0..4 {
            let s = tr.next(256).unwrap();
            covered.push((s.start, s.frames));
        }
        // 480-sample loop from 48000: 256 + 224, then wrap.
        assert_eq!(
            covered,
            vec![(48_000, 256), (48_256, 224), (48_000, 256), (48_256, 224)]
        );
        assert_eq!(tr.apply(&TransportCommand::Pause).unwrap(), Effect::Stopped);
        assert!(tr.next(256).is_none());
    }

    /// A run from before the loop plays straight into it, then wraps; only
    /// the segments inside the loop carry its range. A run from past the
    /// loop end plays straight to the end and carries none.
    #[test]
    fn a_run_before_the_loop_carries_the_range_only_inside_it() {
        let mut tr = Transport::new(48_000);
        tr.set_composition(Some(Rate::FPS_30), Some(20 * TICKS_PER_SECOND));
        let range = TickRange {
            start: t(10.0),
            end: t(12.0),
        };
        tr.apply(&TransportCommand::SetLoop { range: Some(range) })
            .unwrap();
        tr.apply(&TransportCommand::Seek {
            time: t(5.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
        tr.apply(&TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
        let mut expect = 5 * 48_000;
        let mut wrapped = false;
        for _ in 0..2_000 {
            let s = tr.next(1_000).unwrap();
            if s.start != expect {
                assert_eq!((expect, s.start), (12 * 48_000, 10 * 48_000), "wrap");
                wrapped = true;
            }
            if s.start >= 10 * 48_000 {
                assert_eq!(s.loop_range, Some(range), "inside at {}", s.start);
            } else {
                assert_eq!(s.loop_range, None, "before the loop at {}", s.start);
            }
            expect = s.start + s.frames as i64;
        }
        assert!(wrapped);
        // A loop shorter than a segment, entered from before it: the
        // segment before ends exactly at the loop start, and no segment
        // without the range reaches the loop end.
        let short = TickRange {
            start: t(10.0),
            end: t(10.0 + 100.0 / 48_000.0),
        };
        tr.apply(&TransportCommand::SetLoop { range: Some(short) })
            .unwrap();
        tr.apply(&TransportCommand::Seek {
            time: t(10.0 - 30.0 / 48_000.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
        let first = tr.next(256).unwrap();
        assert_eq!((first.start, first.frames), (10 * 48_000 - 30, 30));
        assert_eq!(first.loop_range, None);
        for _ in 0..5 {
            let s = tr.next(256).unwrap();
            assert_eq!((s.start, s.frames), (10 * 48_000, 100));
            assert!(s.loop_range.is_some());
        }
        tr.apply(&TransportCommand::SetLoop { range: Some(range) })
            .unwrap();
        // From past the loop end: straight to the composition end.
        tr.apply(&TransportCommand::Seek {
            time: t(15.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
        let mut last = 0;
        while let Some(s) = tr.next(4_096) {
            assert_eq!(s.loop_range, None);
            last = s.start + s.frames as i64;
        }
        assert_eq!(last, 20 * 48_000);
    }

    #[test]
    fn play_around_returns() {
        let mut tr = Transport::new(48_000);
        tr.apply(&TransportCommand::Seek {
            time: t(2.0),
            mode: SeekMode::Exact,
        })
        .unwrap();
        tr.apply(&TransportCommand::PlayAround {
            pre_roll: t(0.5),
            post_roll: t(0.25),
        })
        .unwrap();
        let mut total = 0;
        let first = tr.next(1 << 20).unwrap();
        assert_eq!(first.start, 72_000);
        total += first.frames;
        while let Some(s) = tr.next(1 << 20) {
            total += s.frames;
        }
        assert_eq!(total, 36_000);
        assert_eq!(tr.take_stop(), Effect::Stopped);
        assert_eq!(tr.position(), 96_000);
    }

    #[test]
    fn scrub_grains_and_steps() {
        let mut tr = Transport::new(48_000);
        tr.set_composition(Some(Rate::FPS_24), None);
        tr.apply(&TransportCommand::AudioScrub { enabled: true })
            .unwrap();
        tr.apply(&TransportCommand::Seek {
            time: t(3.0),
            mode: SeekMode::Scrub,
        })
        .unwrap();
        assert_eq!(tr.state(), TransportState::Shuttling);
        let s = tr.next(1 << 20).unwrap();
        assert_eq!((s.start, s.frames), (144_000, 2_880));
        assert!(tr.next(1 << 20).is_none());
        tr.take_stop();
        assert_eq!(tr.position(), 144_000);
        // Step one frame at 24 fps: 2000 samples, with a grain.
        tr.apply(&TransportCommand::Step { frames: 1 }).unwrap();
        assert_eq!(tr.position(), 146_000);
        let s = tr.next(1 << 20).unwrap();
        assert_eq!(s.start, 146_000);
        assert_eq!(tr.grain_gain(s.grain.unwrap(), s.start), 0.0);
        assert_eq!(tr.grain_gain(s.grain.unwrap(), s.start + 1000), 1.0);
        // Without audio scrub a step is silent.
        tr.take_stop();
        tr.apply(&TransportCommand::AudioScrub { enabled: false })
            .unwrap();
        assert_eq!(
            tr.apply(&TransportCommand::Step { frames: -2 }).unwrap(),
            Effect::Stopped
        );
        assert_eq!(tr.position(), 142_000);
        assert!(tr.next(64).is_none());
    }

    #[test]
    fn other_rates_are_phase_3b() {
        let mut tr = Transport::new(48_000);
        let r = tr.apply(&TransportCommand::Play {
            rate: PlaybackRate { num: 2, den: 1 },
        });
        assert!(matches!(r, Err(AudioError::Unsupported(_))));
    }

    #[test]
    fn stops_at_the_composition_end() {
        let mut tr = Transport::new(48_000);
        tr.set_composition(Some(Rate::FPS_30), Some(TICKS_PER_SECOND));
        tr.apply(&TransportCommand::Seek {
            time: t(0.99),
            mode: SeekMode::Exact,
        })
        .unwrap();
        tr.apply(&TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
        let s = tr.next(1 << 20).unwrap();
        assert_eq!(s.start + s.frames as i64, 48_000);
        assert!(tr.next(64).is_none());
        assert_eq!(tr.take_stop(), Effect::Stopped);
    }
}
