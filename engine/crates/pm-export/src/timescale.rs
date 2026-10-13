//! Movie and media timescales (section 17, "Muxers"; amendment 7).
//!
//! NTSC rates use the rate numerator (24000, 30000, 48000, 60000, 120000, 240000)
//! with a frame duration of 1001; an integer rate f uses f × 1000 with a frame
//! duration of 1000; audio tracks use the sample rate; the movie timescale matches the
//! video's. AVAssetWriter's default of 600 cannot represent 1001/24000 and stores
//! 23.976 fps as 24.000 fps (S2), so these are always set.

use pm_time::Rate;

use crate::Error;

/// Timescale and frame duration of a video track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoTiming {
    pub timescale: i32,
    pub frame_duration: i64,
}

impl VideoTiming {
    /// Presentation time of frame `n` in timescale units.
    pub fn pts(self, n: u64) -> i64 {
        n as i64 * self.frame_duration
    }

    /// Duration of `frames` frames in timescale units.
    pub fn duration(self, frames: u64) -> i64 {
        frames as i64 * self.frame_duration
    }
}

/// The section 17 timing for `rate`. Rates that are neither NTSC nor integer use the
/// reduced rational (`num` per second, `den` per frame) when it fits.
pub fn video_timing(rate: Rate) -> Result<VideoTiming, Error> {
    let (num, den) = (rate.num(), rate.den());
    let (timescale, frame_duration) = if den == 1 {
        (num.checked_mul(1000), 1000)
    } else if rate.is_ntsc() {
        (Some(num), 1001)
    } else {
        (Some(num), den)
    };
    match timescale.and_then(|t| i32::try_from(t).ok()) {
        Some(timescale) if timescale > 0 => Ok(VideoTiming {
            timescale,
            frame_duration,
        }),
        _ => Err(Error::InvalidSpec(format!(
            "frame rate {rate} has no QuickTime timescale"
        ))),
    }
}

/// An audio track's timescale: its sample rate.
pub fn audio_timescale(sample_rate: u32) -> i32 {
    sample_rate as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_17_timescales_for_every_supported_rate() {
        let want: [(Rate, i32, i64); 16] = [
            (Rate::FPS_23_976, 24_000, 1001),
            (Rate::FPS_24, 24_000, 1000),
            (Rate::FPS_25, 25_000, 1000),
            (Rate::FPS_29_97, 30_000, 1001),
            (Rate::FPS_30, 30_000, 1000),
            (Rate::FPS_47_952, 48_000, 1001),
            (Rate::FPS_48, 48_000, 1000),
            (Rate::FPS_50, 50_000, 1000),
            (Rate::FPS_59_94, 60_000, 1001),
            (Rate::FPS_60, 60_000, 1000),
            (Rate::FPS_90, 90_000, 1000),
            (Rate::FPS_100, 100_000, 1000),
            (Rate::FPS_119_88, 120_000, 1001),
            (Rate::FPS_120, 120_000, 1000),
            (Rate::FPS_239_76, 240_000, 1001),
            (Rate::FPS_240, 240_000, 1000),
        ];
        for (rate, timescale, frame_duration) in want {
            let t = video_timing(rate).unwrap();
            assert_eq!(
                (t.timescale, t.frame_duration),
                (timescale, frame_duration),
                "{rate}"
            );
            // Exact: frame n starts at n / rate seconds.
            assert_eq!(
                t.pts(7) as i128 * rate.num() as i128,
                7 * rate.den() as i128 * t.timescale as i128
            );
        }
        assert!(
            pm_time::SUPPORTED_RATES
                .iter()
                .all(|r| video_timing(*r).is_ok())
        );
    }

    #[test]
    fn other_rationals_and_audio() {
        let t = video_timing(Rate::new(25, 2).unwrap()).unwrap();
        assert_eq!((t.timescale, t.frame_duration), (25, 2));
        assert!(
            video_timing(Rate::new(3_000_000, 1).unwrap()).is_err(),
            "f × 1000 overflows i32"
        );
        assert_eq!(audio_timescale(48_000), 48_000);
    }
}
