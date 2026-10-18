//! Original animation timing with the existing importer's slot semantics.
//!
//! Keep this equal to `src/shared/animated-image.ts`: delays below 10 ms
//! become 100 ms, a common divisor gives exact slots, otherwise cumulative
//! rounding resamples within the existing import budget. Loop metadata is
//! retained by `Animation`; a timeline clip plays one iteration, as today.

use super::Animation;
use crate::{MediaError, Result};
use serde::Serialize;
use std::path::Path;

pub const MAX_SEQUENCE_FRAMES: usize = 2400;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationTiming {
    pub fps: f64,
    pub repeats: Vec<usize>,
    pub frames: usize,
}

impl AnimationTiming {
    /// Microsecond durations returned by Chromium's ImageDecoder. Invalid,
    /// missing (zero), and sub-10 ms durations follow the shipped importer.
    pub fn from_microseconds(durations: &[f64], max_frames: usize) -> Result<Self> {
        if durations.is_empty() {
            return Err(MediaError::invalid("this animation has no frames"));
        }
        if durations.len() > MAX_SEQUENCE_FRAMES || max_frames == 0 {
            return Err(MediaError::invalid(
                "animation exceeds the import frame budget",
            ));
        }
        let delays: Vec<f64> = durations
            .iter()
            .map(|v| {
                let ms = (v / 1000.0).round();
                if ms.is_finite() && ms >= 10.0 {
                    ms
                } else {
                    100.0
                }
            })
            .collect();
        let gcd = |mut a: f64, mut b: f64| {
            while b != 0.0 {
                (a, b) = (b, a % b);
            }
            a
        };
        let unit = delays.iter().copied().reduce(gcd).expect("nonempty");
        let fps = 1000.0 / unit;
        let repeats: Vec<usize> = delays.iter().map(|ms| (ms / unit) as usize).collect();
        let frames = repeats.iter().try_fold(0usize, |a, b| a.checked_add(*b));
        if (1.0..=240.0).contains(&fps) && frames.is_some_and(|n| n <= max_frames) {
            return Ok(Self {
                fps,
                repeats,
                frames: frames.unwrap(),
            });
        }
        let seconds = delays.iter().sum::<f64>() / 1000.0;
        if !seconds.is_finite() {
            return Err(MediaError::invalid("animation duration is not finite"));
        }
        let average = delays.len() as f64 / seconds.max(1.0 / 120.0);
        let mut fps = (average * 4.0).round().clamp(12.0, 120.0);
        if fps * seconds > max_frames as f64 {
            fps = (max_frames as f64 / seconds.max(1.0 / 120.0))
                .floor()
                .max(1.0);
        }
        let (mut elapsed, mut placed) = (0.0, 0usize);
        let mut repeats = Vec::with_capacity(delays.len());
        for ms in delays {
            elapsed += ms;
            let slots = ((elapsed / 1000.0 * fps).round() - placed as f64).max(1.0);
            if slots > usize::MAX as f64 {
                return Err(MediaError::invalid(
                    "animation duration exceeds addressable slots",
                ));
            }
            let slots = slots as usize;
            placed = placed
                .checked_add(slots)
                .ok_or_else(|| MediaError::invalid("animation slot count overflow"))?;
            repeats.push(slots);
        }
        Ok(Self {
            fps,
            repeats,
            frames: placed,
        })
    }

    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.fps
    }

    /// Source animation frame occupying the CFR slot showing at `seconds`.
    /// The last frame holds after the end; before zero there is no frame.
    pub fn frame_at(&self, seconds: f64) -> Option<usize> {
        if !seconds.is_finite() || seconds < 0.0 || self.frames == 0 {
            return None;
        }
        // Same 1e-7 second correction as sequencePlaybackTime.
        let slot = (((seconds + 1e-7) * self.fps).floor() as usize).min(self.frames - 1);
        let mut end = 0usize;
        self.repeats.iter().position(|n| {
            end += n;
            slot < end
        })
    }
}

impl Animation {
    pub fn compatibility_timing(&self) -> Result<AnimationTiming> {
        if self.frames != self.delays.len() {
            return Err(MediaError::invalid(
                "animation frame count does not match its delays",
            ));
        }
        AnimationTiming::from_microseconds(
            &self
                .delays
                .iter()
                .map(|d| d * 1_000_000.0)
                .collect::<Vec<_>>(),
            MAX_SEQUENCE_FRAMES,
        )
    }
}

/// Read timing from the original bytes rather than the browser rendition.
pub fn read_timing(path: &Path) -> Result<Option<AnimationTiming>> {
    super::animation(path)?
        .map(|a| a.compatibility_timing())
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(ms: &[f64], budget: usize) -> AnimationTiming {
        AnimationTiming::from_microseconds(
            &ms.iter().map(|v| v * 1000.0).collect::<Vec<_>>(),
            budget,
        )
        .unwrap()
    }

    #[test]
    fn shipped_timing_vectors_and_slot_selection() {
        let t = timing(&[80.0, 80.0, 90.0], 2400);
        assert_eq!(
            t,
            AnimationTiming {
                fps: 100.0,
                repeats: vec![8, 8, 9],
                frames: 25
            }
        );
        for (s, frame) in [(0.0, 0), (0.079, 0), (0.08, 1), (0.16, 2), (0.25, 2)] {
            assert_eq!(t.frame_at(s), Some(frame));
        }
        assert_eq!(t.frame_at(-0.01), None);
        assert_eq!(t.frame_at(f64::NAN), None);
        assert_eq!(
            timing(&[0.0, f64::NAN, 4.0, 20.0], 2400).repeats,
            vec![5, 5, 5, 1]
        );
        let t = timing(&[83.0, 84.0].repeat(6), 2400);
        assert_eq!((t.fps, t.frames), (48.0, 48));
        assert_eq!(t.repeats, vec![4; 12]);
        assert_eq!(timing(&[17.0; 400], 2400).frames, 400);
        let t = timing(&[33.0; 60], 240);
        assert_eq!((t.fps, t.frames), (1000.0 / 33.0, 60));
        assert!((t.duration() - 1.98).abs() <= 1.0 / t.fps);
    }

    #[test]
    fn invalid_animation_has_a_named_error() {
        assert!(AnimationTiming::from_microseconds(&[], 2400).is_err());
        assert!(AnimationTiming::from_microseconds(&vec![100000.0; 2401], 2400).is_err());
        assert!(AnimationTiming::from_microseconds(&[100000.0], 0).is_err());
    }
}
