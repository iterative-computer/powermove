//! The playback clock (contract 6; owner: Audio, section 15).
//!
//! The master clock is the audio device clock, including device latency. The
//! interactive engine publishes clock samples: "at host instant H, the
//! listener hears media time M, advancing at rate R". Every consumer (the
//! viewer's presentation, the A/V sync measurement, the job engine's slicing
//! governor) derives media time from the latest sample with the same integer
//! math, so they agree to the tick.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{AssetId, HostMicros, PlaybackRate, Seq, TICKS_PER_SECOND, TickRange, Ticks};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum TransportState {
    Stopped,
    Playing,
    /// Scrubbing or J/K/L shuttle at a non-real-time rate (audio scrub active).
    Shuttling,
}

/// A clock sample. Published on every discontinuity (play, pause, seek, rate
/// change, device change) and at least 4 times a second while playing. A loop
/// wrap is not a discontinuity: the line wraps by `loop_range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ClockSample {
    pub seq: Seq,
    /// Increases on every discontinuity; samples with the same epoch describe one continuous line.
    pub epoch: u32,
    pub state: TransportState,
    /// The host instant at which `media_time` reaches the listener (output latency included).
    pub host_time: HostMicros,
    pub media_time: Ticks,
    pub rate: PlaybackRate,
    /// The device's reported output latency, informational (already folded into `host_time`).
    pub output_latency_us: u32,
    /// The device sample rate.
    pub sample_rate: u32,
    /// The work-area loop the line wraps by: set only while the media heard
    /// is inside it and the run wraps at its end. A run from before the loop
    /// start carries none until it enters the loop; a run past the loop end,
    /// a play around, and a stopped clock carry none.
    pub loop_range: Option<TickRange>,
}

impl ClockSample {
    /// Media time heard at host instant `at`, by integer math on the sample's
    /// line, wrapped into the loop range when one is set. Stopped clocks
    /// return `media_time`.
    pub fn media_time_at(&self, at: HostMicros) -> Ticks {
        if self.state == TransportState::Stopped || self.rate.num == 0 {
            return self.media_time;
        }
        let elapsed_us = at.get() as i128 - self.host_time.get() as i128;
        // ticks = elapsed_us × TICKS_PER_SECOND / 1e6 × num / den, floored toward −∞.
        let numerator = elapsed_us * TICKS_PER_SECOND as i128 * self.rate.num as i128;
        let denominator = 1_000_000i128 * self.rate.den as i128;
        let mut t = self.media_time.get() as i128 + numerator.div_euclid(denominator);
        if let Some(range) = self.loop_range {
            let start = range.start.get() as i128;
            let length = range.end.get() as i128 - start;
            if length > 0 {
                t = start + (t - start).rem_euclid(length);
            }
        }
        Ticks::saturating_from_i128(t)
    }
}

impl Validate for ClockSample {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Clock;
        ensure(self.rate.is_valid(), C, || {
            "rate denominator must be positive".into()
        })?;
        ensure((8_000..=192_000).contains(&self.sample_rate), C, || {
            format!("sample rate {} outside 8 to 192 kHz", self.sample_rate)
        })?;
        ensure(
            (self.state == TransportState::Stopped) == (self.rate.num == 0),
            C,
            || "a stopped clock has rate 0 and a running clock does not".into(),
        )?;
        if let Some(range) = self.loop_range {
            ensure(range.is_valid(), C, || {
                "loop range must be non-empty".into()
            })?;
        }
        Ok(())
    }
}

/// How a seek lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SeekMode {
    /// Exact frame, original quality as soon as it decodes.
    Exact,
    /// Scrub: nearest fast frame first, bypassing the jitter buffer.
    Scrub,
}

/// Transport commands from the renderer (notifications; the clock samples are the answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "command"
)]
pub enum TransportCommand {
    Play {
        rate: PlaybackRate,
    },
    Pause,
    Seek {
        time: Ticks,
        mode: SeekMode,
    },
    /// Frame stepping by whole frames (negative steps back).
    Step {
        frames: i32,
    },
    SetLoop {
        range: Option<TickRange>,
    },
    /// Play around the current frame (pre-roll, post-roll in ticks).
    PlayAround {
        pre_roll: Ticks,
        post_roll: Ticks,
    },
    AudioScrub {
        enabled: bool,
    },
}

/// The mix's monitor and its data for the renderer's audio panels (plan
/// section 16, appendix C: "Timeline waveforms; mixer meters, solo, retune |
/// Engine"). One event carries the four messages, each naming its
/// direction; all are session-bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "audio"
)]
pub enum AudioMessage {
    /// Renderer → engine: the mixer's monitor solo, the strips heard (top-
    /// level layer ids; empty: every strip). A listening aid: never part of
    /// the document or an export.
    Monitor { solo: Vec<String> },
    /// Engine → renderer: the levels of what is heard now, about 30 times
    /// a second while the engine plays, and once (silent) when it stops.
    Levels(AudioLevels),
    /// Renderer → engine: the waveform envelope of an asset of the
    /// session's document. Answered with [`AudioMessage::Peaks`].
    PeaksWanted {
        asset: String,
        /// Echoed by the decoder so replaced sources and resyncs cannot
        /// accept a prior request's envelope.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        request: Option<Seq>,
    },
    /// Engine → renderer: an asset's waveform envelope (`peakEnvelope` of
    /// today's engine: `peaks_per_second` buckets a second of the source,
    /// each the largest magnitude over every channel), or why there is none.
    Peaks(AudioPeaks),
    /// Renderer → engine: source monitor transport, isolated from the composition.
    SourceMonitor {
        asset: Option<AssetId>,
        time: Ticks,
        playing: bool,
        generation: u32,
    },
    /// Engine → renderer: source monitor clock/status.
    SourceStatus {
        asset: AssetId,
        time: Ticks,
        playing: bool,
        generation: u32,
        levels: ChannelLevels,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        error: Option<String>,
    },
}

/// Linear levels per bus channel (peak magnitude and RMS over the last
/// render), with the media time they were heard at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioLevels {
    pub media_time: Ticks,
    pub master: ChannelLevels,
    pub strips: Vec<StripLevels>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StripLevels {
    /// The strip's top-level layer id.
    pub id: String,
    pub levels: ChannelLevels,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ChannelLevels {
    pub peak: Vec<f32>,
    pub rms: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioPeaks {
    pub asset: String,
    /// The request generation of `PeaksWanted` (absent for older senders).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub request: Option<Seq>,
    pub peaks_per_second: f64,
    /// Program seconds of the source.
    pub duration: f64,
    pub peaks: Vec<f32>,
    /// Why there are no peaks (no audio stream, offline media, a decode
    /// failure).
    pub error: Option<String>,
}

/// The most strips a levels message carries.
pub const MAX_LEVEL_STRIPS: usize = 4096;
/// The most buckets a peaks message carries (about 40 hours at 100 a
/// second; the 16 MiB message limit holds 4 bytes a bucket).
pub const MAX_PEAKS: usize = 3_600_000;

impl Validate for AudioMessage {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Clock;
        let finite = |l: &ChannelLevels| {
            l.peak.len() == l.rms.len()
                && l.peak.len() <= 32
                && l.peak
                    .iter()
                    .chain(&l.rms)
                    .all(|v| v.is_finite() && *v >= 0.0)
        };
        match self {
            AudioMessage::SourceMonitor {
                asset,
                time,
                playing,
                ..
            } => ensure(
                time.get() >= 0
                    && asset.as_ref().is_none_or(AssetId::is_valid)
                    && (!playing || asset.is_some()),
                C,
                || "invalid source monitor state".into(),
            ),
            AudioMessage::SourceStatus {
                asset,
                time,
                levels,
                ..
            } => ensure(
                asset.is_valid() && time.get() >= 0 && finite(levels),
                C,
                || "invalid source status".into(),
            ),
            AudioMessage::Monitor { solo } => ensure(solo.len() <= MAX_LEVEL_STRIPS, C, || {
                "too many solo strips".into()
            }),
            AudioMessage::Levels(levels) => ensure(
                levels.strips.len() <= MAX_LEVEL_STRIPS
                    && finite(&levels.master)
                    && levels.strips.iter().all(|s| finite(&s.levels)),
                C,
                || "levels must be finite, non-negative, and per channel".into(),
            ),
            AudioMessage::PeaksWanted { asset, .. } => {
                ensure(!asset.is_empty(), C, || "peaks need an asset".into())
            }
            AudioMessage::Peaks(peaks) => ensure(
                peaks.peaks.len() <= MAX_PEAKS
                    && peaks.peaks_per_second.is_finite()
                    && peaks.peaks_per_second > 0.0
                    && peaks.duration.is_finite()
                    && peaks.duration >= 0.0
                    && peaks.peaks.iter().all(|v| v.is_finite() && *v >= 0.0),
                C,
                || "peaks must be finite and non-negative".into(),
            ),
        }
    }
}

/// J/K/L rates: 1× and 2× forward and reverse (section 15).
pub const JKL_RATES: [PlaybackRate; 4] = [
    PlaybackRate { num: 1, den: 1 },
    PlaybackRate { num: 2, den: 1 },
    PlaybackRate { num: -1, den: 1 },
    PlaybackRate { num: -2, den: 1 },
];

impl Validate for TransportCommand {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Clock;
        match self {
            TransportCommand::Play { rate } => ensure(rate.is_valid() && rate.num != 0, C, || {
                "play needs a non-zero rate".into()
            }),
            TransportCommand::SetLoop { range: Some(range) } => ensure(range.is_valid(), C, || {
                "loop range must be non-empty".into()
            }),
            TransportCommand::Step { frames } => {
                ensure(*frames != 0, C, || "a step moves at least one frame".into())
            }
            TransportCommand::PlayAround {
                pre_roll,
                post_roll,
            } => ensure(pre_roll.get() >= 0 && post_roll.get() >= 0, C, || {
                "rolls are non-negative".into()
            }),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_audio_generation_and_levels_round_trip() {
        let wanted = AudioMessage::SourceMonitor {
            asset: Some("a".into()),
            time: Ticks::from_whole_seconds(2),
            playing: true,
            generation: 4,
        };
        assert!(wanted.validate().is_ok());
        let bytes = rmp_serde::to_vec_named(&wanted).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<AudioMessage>(&bytes).unwrap(),
            wanted
        );
        let status = AudioMessage::SourceStatus {
            asset: "a".into(),
            time: Ticks::from_whole_seconds(2),
            playing: true,
            generation: 4,
            levels: ChannelLevels {
                peak: vec![0.5, 0.25],
                rms: vec![0.2, 0.1],
            },
            error: None,
        };
        assert!(status.validate().is_ok());
        let bytes = rmp_serde::to_vec_named(&status).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<AudioMessage>(&bytes).unwrap(),
            status
        );
    }

    fn sample(rate: PlaybackRate) -> ClockSample {
        ClockSample {
            seq: Seq::ZERO,
            epoch: 1,
            state: if rate.num == 0 {
                TransportState::Stopped
            } else {
                TransportState::Playing
            },
            host_time: HostMicros::new(1_000_000).unwrap(),
            media_time: Ticks::from_whole_seconds(10),
            rate,
            output_latency_us: 12_000,
            sample_rate: 48_000,
            loop_range: None,
        }
    }

    #[test]
    fn media_time_follows_the_line() {
        let clock = sample(PlaybackRate::NORMAL);
        assert_eq!(
            clock.media_time_at(HostMicros::new(1_500_000).unwrap()),
            Ticks::new(10 * TICKS_PER_SECOND + TICKS_PER_SECOND / 2).unwrap()
        );
        // Before the anchor: earlier media time, floored.
        assert_eq!(
            clock.media_time_at(HostMicros::new(999_999).unwrap()).get(),
            10 * TICKS_PER_SECOND - 706
        );
        let reverse = sample(PlaybackRate { num: -2, den: 1 });
        assert_eq!(
            reverse.media_time_at(HostMicros::new(2_000_000).unwrap()),
            Ticks::from_whole_seconds(8)
        );
        let stopped = sample(PlaybackRate::STOPPED);
        assert_eq!(
            stopped.media_time_at(HostMicros::new(9_000_000).unwrap()),
            Ticks::from_whole_seconds(10)
        );
    }

    #[test]
    fn ntsc_rates_stay_exact() {
        // 1001/1000 slow-down over 1001 s of host time is exactly 1000 s of media.
        let clock = ClockSample {
            media_time: Ticks::ZERO,
            ..sample(PlaybackRate {
                num: 1000,
                den: 1001,
            })
        };
        let at = HostMicros::new(1_000_000 + 1_001_000_000).unwrap();
        assert_eq!(clock.media_time_at(at), Ticks::from_whole_seconds(1000));
    }

    #[test]
    fn loops_wrap() {
        let mut clock = sample(PlaybackRate::NORMAL);
        clock.loop_range = Some(TickRange {
            start: Ticks::from_whole_seconds(10),
            end: Ticks::from_whole_seconds(12),
        });
        assert_eq!(
            clock.media_time_at(HostMicros::new(4_000_000).unwrap()),
            Ticks::from_whole_seconds(11)
        );
        assert!(clock.validate().is_ok());
    }

    #[test]
    fn validation() {
        let mut clock = sample(PlaybackRate::NORMAL);
        clock.state = TransportState::Stopped;
        assert!(clock.validate().is_err());
        assert!(
            TransportCommand::Play {
                rate: PlaybackRate::STOPPED
            }
            .validate()
            .is_err()
        );
        assert!(TransportCommand::Step { frames: -1 }.validate().is_ok());
        assert!(
            JKL_RATES
                .iter()
                .all(|r| TransportCommand::Play { rate: *r }.validate().is_ok())
        );
    }
}
