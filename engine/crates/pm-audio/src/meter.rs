//! Level taps for the mixer's meters: the raw sample peak and RMS of the
//! latest window of a strip or the master, per channel.
//!
//! These are the numbers the legacy taps measure (`measureBlock` over an
//! `AnalyserNode` window of 2048 samples, upmixed to two channels); the
//! ballistics (peak fall, hold, RMS integration) stay in the mixer panel
//! (`mixer/levels.ts`). Strip taps read the strip's sum before the monitor
//! solo, as the legacy taps do, so a soloed-away strip still meters.

use serde::Serialize;

/// The legacy tap window.
pub const WINDOW: usize = 2048;

/// Peak and RMS (linear) of one channel over the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Level {
    pub peak: f32,
    pub rms: f32,
}

/// A running window per channel.
#[derive(Clone, Debug)]
pub struct Tap {
    rings: Vec<Vec<f32>>,
    at: usize,
    filled: usize,
}

impl Tap {
    pub fn new(channels: usize) -> Tap {
        Tap {
            rings: vec![vec![0.0; WINDOW]; channels],
            at: 0,
            filled: 0,
        }
    }

    /// Appends planar samples (`frames` per channel).
    pub fn push(&mut self, planar: &[Vec<f32>], frames: usize) {
        let skip = frames.saturating_sub(WINDOW);
        for (ring, src) in self.rings.iter_mut().zip(planar) {
            let mut at = self.at;
            for &v in &src[skip..frames] {
                ring[at] = v;
                at = (at + 1) % WINDOW;
            }
        }
        self.at = (self.at + frames - skip) % WINDOW;
        self.filled = (self.filled + frames).min(WINDOW);
    }

    pub fn clear(&mut self) {
        for r in &mut self.rings {
            r.fill(0.0);
        }
        self.filled = 0;
    }

    /// The levels over the window, per channel (`measureBlock`).
    pub fn levels(&self) -> Vec<Level> {
        self.rings
            .iter()
            .map(|ring| {
                let mut peak = 0f32;
                let mut sum = 0f64;
                for &v in ring {
                    peak = peak.max(v.abs());
                    sum += (v as f64) * (v as f64);
                }
                Level {
                    peak,
                    rms: (sum / WINDOW as f64).sqrt() as f32,
                }
            })
            .collect()
    }
}

/// Levels of every strip and the master at one media position.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meters {
    /// Project sample index just after the window.
    pub position: i64,
    pub strips: Vec<(String, Vec<Level>)>,
    pub master: Vec<Level>,
}

impl Meters {
    /// The contract 6 levels message for these meters (`sample_rate`: the
    /// project rate, for the media time).
    pub fn to_levels(&self, sample_rate: u32) -> pm_protocol::clock::AudioLevels {
        use pm_protocol::clock::{AudioLevels, ChannelLevels, StripLevels};
        let channels = |levels: &[Level]| ChannelLevels {
            peak: levels.iter().map(|l| l.peak).collect(),
            rms: levels.iter().map(|l| l.rms).collect(),
        };
        let ticks = self.position as i128 * pm_protocol::common::TICKS_PER_SECOND as i128
            / sample_rate.max(1) as i128;
        AudioLevels {
            media_time: pm_protocol::common::Ticks::saturating_from_i128(ticks.max(0)),
            master: channels(&self.master),
            strips: self
                .strips
                .iter()
                .map(|(id, levels)| StripLevels {
                    id: id.clone(),
                    levels: channels(levels),
                })
                .collect(),
        }
    }

    /// Silence on every strip (the levels sent when playback stops).
    pub fn silent(&self) -> Meters {
        let zero = |l: &[Level]| vec![Level::default(); l.len()];
        Meters {
            position: self.position,
            strips: self
                .strips
                .iter()
                .map(|(id, l)| (id.clone(), zero(l)))
                .collect(),
            master: zero(&self.master),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_measures_the_latest_samples() {
        let mut tap = Tap::new(2);
        let loud = vec![vec![0.5; 3000], vec![-0.25; 3000]];
        tap.push(&loud, 3000);
        let l = tap.levels();
        assert_eq!(l[0].peak, 0.5);
        assert!((l[0].rms - 0.5).abs() < 1e-6);
        assert!((l[1].rms - 0.25).abs() < 1e-6);
        let quiet = vec![vec![0.0; 1024], vec![0.0; 1024]];
        tap.push(&quiet, 1024);
        let l = tap.levels();
        assert_eq!(l[0].peak, 0.5);
        assert!((l[0].rms - 0.5 * (0.5f32).sqrt()).abs() < 1e-6);
        tap.push(&quiet, 1024);
        assert_eq!(tap.levels()[0].peak, 0.0);
    }
}
