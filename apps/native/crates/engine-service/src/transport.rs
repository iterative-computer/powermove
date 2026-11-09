//! Reuse the existing presenter policy without its bridge transport.
use crate::clock::TransportClock;
use crate::media::MediaMap;
use crate::audio::{AudioCtl, AudioLink};
use pm_protocol::{
    clock::{ClockSample, TransportCommand, TransportState},
    common::PlaybackRate,
};

pub use crate::clock::PREROLL_US;
#[derive(Clone, Copy, Debug)]
pub enum ShuttleKey {
    J,
    K,
    L,
}
/// Shipped state-dependent decline while stopped unless from_pause is set.
/// Repeated J/L saturate at ±2x and remain consumed, as PM.jkl does.
pub fn shuttle(key: ShuttleKey, rate: PlaybackRate, from_pause: bool) -> Option<TransportCommand> {
    let r = rate.num as f64 / rate.den.max(1) as f64;
    if r == 0. && !from_pause {
        return None;
    }
    let num = match key {
        ShuttleKey::K => return (r != 0.).then_some(TransportCommand::Pause),
        ShuttleKey::J => {
            if r < 0. {
                -2
            } else {
                -1
            }
        }
        ShuttleKey::L => {
            if r > 0. {
                2
            } else {
                1
            }
        }
    };
    Some(TransportCommand::Play {
        rate: PlaybackRate { num, den: 1 },
    })
}

pub struct Transport {
    pub clock: TransportClock,
    pub audio: Option<AudioLink>,
    awaiting: Option<(i64, Option<u32>, PlaybackRate)>,
    pub fallback: bool,
    audible: bool,
    /// Latest known vsync (host s) and refresh interval for this viewer.
    pub grid: Option<(f64, f64)>,
}
/// The first vsync on `grid` at or after `at` (host µs). The picture can only
/// change on vsync, so a run whose first sample is heard exactly on that line
/// lets every following frame onset meet its audio (refresh-multiple rates).
pub fn align_to_grid(at: i64, grid: Option<(f64, f64)>) -> i64 {
    let Some((vsync, interval)) = grid.filter(|(v, i)| v.is_finite() && *v > 0. && *i >= 0.001)
    else {
        return at;
    };
    let t = at as f64 / 1e6;
    let k = ((t - vsync) / interval).ceil();
    let aligned = vsync + k * interval;
    // Rounding can land a microsecond early; never start before `at`.
    ((aligned * 1e6).round() as i64).max(at)
}
impl Transport {
    pub fn new(frame_ticks: i64) -> Self {
        Self {
            clock: TransportClock::new(frame_ticks),
            audio: None,
            awaiting: None,
            fallback: false,
            audible: false,
            grid: None,
        }
    }
    /// Worker only. AudioLink lazily opens Playback after WARM_IDLE if audible.
    pub fn enable_audio(
        &mut self,
        doc: &pm_model::Document,
        comp: String,
        media: MediaMap,
        levels: crate::audio::LevelsSink,
    ) -> anyhow::Result<()> {
        self.audible =
            pm_audio::graph::audible(doc, &comp, &mut pm_eval::Evaluator::new()).unwrap_or(false);
        let link =
            AudioLink::start(doc.clone(), comp, media, levels).map_err(anyhow::Error::msg)?;
        link.send(AudioCtl::Warm);
        self.audio = Some(link);
        Ok(())
    }
    pub fn update(&mut self, snapshot: &crate::Snapshot, media: MediaMap, comp: &str) {
        self.audible =
            pm_audio::graph::audible(&snapshot.document, comp, &mut pm_eval::Evaluator::new())
                .unwrap_or(false);
        if let Some(audio) = &self.audio {
            // Composition identity is fixed for a viewer.
            audio.send(AudioCtl::Document {
                root: snapshot.document.root().cloned(),
                revision: pm_protocol::common::Revision::new(snapshot.revision)
                    .expect("kernel revision"),
            });
            audio.send(AudioCtl::Media(media));
        }
    }
    pub fn command(&mut self, cmd: TransportCommand, now: i64) {
        let before = self
            .audio
            .as_ref()
            .and_then(|a| a.clock())
            .and_then(|c| c.latest())
            .map(|s| s.epoch);
        let rate = self.awaiting.map_or(self.clock.rate(), |a| a.2);
        let running = self.clock.is_playing() || self.awaiting.is_some();
        match cmd {
            TransportCommand::Play { rate } => {
                if running && rate == self.clock.rate() {
                    return;
                }
                let mut from = self.clock.media_at(now);
                if !running {
                    let range = self.clock.loop_range();
                    let start = range.map_or(0, |r| r.start.get());
                    let end = range.map(|r| r.end.get()).or(self.clock.end());
                    if rate.num > 0 && end.is_some_and(|end| from >= end) {
                        from = start;
                    }
                    if rate.num < 0 && from <= start {
                        from = end.unwrap_or(from);
                    }
                    self.clock.hold(from, now);
                }
                let start = align_to_grid(now + PREROLL_US, self.grid);
                self.clock.play_at(rate, now, start);
                if let Some(a) = self.audio.as_ref().filter(|_| self.audible) {
                    a.send(AudioCtl::Play {
                        from,
                        rate,
                        at: start,
                    });
                    self.clock.hold(from, now);
                    self.awaiting = Some((
                        now + crate::clock::AUDIO_START_TIMEOUT_US,
                        before,
                        rate,
                    ));
                }
            }
            TransportCommand::PlayAround {
                pre_roll,
                post_roll,
            } => {
                let here = self.clock.media_at(now);
                if let Some(a) = self.audio.as_ref().filter(|_| self.audible) {
                    a.send(AudioCtl::PlayAround {
                        at: here,
                        pre: pre_roll,
                        post: post_roll,
                    });
                    self.clock.hold(here, now);
                    self.awaiting = Some((
                        now + crate::clock::AUDIO_START_TIMEOUT_US,
                        before,
                        PlaybackRate::NORMAL,
                    ));
                } else {
                    self.clock
                        .play_around(here, pre_roll.get(), post_roll.get(), now);
                }
            }
            TransportCommand::Seek { time, mode } => {
                self.clock.apply(cmd, now);
                if let Some(a) = &self.audio {
                    if running && (self.audible || self.clock.following()) {
                        a.send(AudioCtl::Seek(time.get()));
                        self.clock.hold(time.get(), now);
                        self.awaiting = Some((
                            now + crate::clock::AUDIO_START_TIMEOUT_US,
                            before,
                            rate,
                        ));
                    } else {
                        a.send(if mode == pm_protocol::clock::SeekMode::Scrub {
                            AudioCtl::Scrub(time.get())
                        } else {
                            AudioCtl::Seek(time.get())
                        });
                    }
                }
            }
            TransportCommand::AudioScrub { enabled } => {
                if let Some(a) = &self.audio {
                    a.send(AudioCtl::AudioScrub(enabled));
                }
            }
            _ => {
                self.clock.apply(cmd, now);
                if matches!(cmd, TransportCommand::Pause | TransportCommand::Step { .. }) {
                    self.awaiting = None;
                }
                if let Some(a) = &self.audio {
                    a.send(AudioCtl::Transport(cmd));
                }
            }
        }
    }
    pub fn poll(&mut self, now: i64) -> ClockSample {
        if let Some(a) = &self.audio {
            if let Some(s) = a.clock().and_then(|c| c.latest()) {
                let accept = match self.awaiting {
                    Some((_, epoch, _)) => {
                        s.state != TransportState::Stopped && Some(s.epoch) != epoch
                    }
                    None => self.clock.following(),
                };
                if accept {
                    self.clock.follow(&s);
                    self.awaiting = None;
                }
            }
            if let Some((deadline, _, rate)) = self.awaiting
                && (a.failed() || now >= deadline)
            {
                self.awaiting = None;
                self.clock.apply(TransportCommand::Play { rate }, now);
                a.send(AudioCtl::Pause);
                self.fallback = true;
            }
        }
        self.clock.stop_at_end(now);
        self.clock.sample(now)
    }
    /// Lift a wrapped frame time onto this audio epoch's unwrapped line near
    /// its intended presentation. Loops are continuous; seek/play/device
    /// discontinuities are fenced by poll() before this line is followed.
    pub fn audio_host_for(
        &self,
        time: pm_protocol::common::Ticks,
        near_host_us: i64,
    ) -> Option<f64> {
        if !self.clock.following() {
            return None;
        }
        let predicted = self.clock.unwrapped_at(near_host_us);
        let mut media = time.get();
        if let Some(range) = self.clock.loop_range().filter(|r| r.is_valid()) {
            let wrapped = self.clock.wrap(predicted);
            if (wrapped != predicted || (range.start.get()..range.end.get()).contains(&predicted))
                && (range.start.get()..range.end.get()).contains(&media)
            {
                let length = range.end.get() - range.start.get();
                let passes = ((predicted - media) as f64 / length as f64).round() as i64;
                media = media.saturating_add(passes.saturating_mul(length));
            }
        }
        self.clock.host_at(media)
    }
    pub fn running(&self) -> bool {
        self.clock.is_playing() || self.awaiting.is_some()
    }
    pub fn rate(&self) -> PlaybackRate {
        self.awaiting.map_or(self.clock.rate(), |a| a.2)
    }
}

/// Device presentation timestamps only. The caller matches a video media time
/// to the host timestamp at which audio plays that SAME media time.
#[derive(Default)]
pub struct AvMeasurements {
    offsets_ms: Vec<f64>,
}
#[derive(Debug)]
pub struct AvReport {
    pub samples: usize,
    pub mean_abs_ms: f64,
    pub p95_jitter_ms: f64,
}
impl AvMeasurements {
    pub fn record(&mut self, video_host_us: u64, audio_host_us: u64) {
        self.offsets_ms
            .push((video_host_us as f64 - audio_host_us as f64) / 1000.);
    }
    pub fn report(&self) -> Option<AvReport> {
        if self.offsets_ms.is_empty() {
            return None;
        }
        let mean = self.offsets_ms.iter().sum::<f64>() / self.offsets_ms.len() as f64;
        let mut jitter: Vec<_> = self.offsets_ms.iter().map(|v| (v - mean).abs()).collect();
        jitter.sort_by(f64::total_cmp);
        Some(AvReport {
            samples: jitter.len(),
            mean_abs_ms: self.offsets_ms.iter().map(|v| v.abs()).sum::<f64>() / jitter.len() as f64,
            p95_jitter_ms: jitter[(jitter.len() * 95).div_ceil(100) - 1],
        })
    }
}

#[cfg(test)]
mod grid_tests {
    use super::align_to_grid;
    #[test]
    fn starts_land_on_the_next_vsync_and_never_early() {
        let grid = Some((100.0, 1. / 120.));
        // 100.05 s is between vsyncs 6 (100.05) exactly and 7.
        assert_eq!(align_to_grid(100_050_000, grid), 100_050_000);
        let at = align_to_grid(100_051_000, grid);
        assert_eq!(at, 100_058_333);
        assert!(at >= 100_051_000);
        // Old phase extrapolates; no grid or a bad one leaves `at` alone.
        let k = (align_to_grid(160_000_001, grid) - 100_000_000) as f64 / (1e6 / 120.);
        assert!((k - k.round()).abs() < 1e-3, "extrapolated phase: {k}");
        assert_eq!(align_to_grid(5, None), 5);
        assert_eq!(align_to_grid(5, Some((f64::NAN, 0.01))), 5);
    }
}
