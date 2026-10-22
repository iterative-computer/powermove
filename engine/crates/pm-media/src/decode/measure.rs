//! Bounded, original-source decode measurement for a machine's automatic
//! media policy. Random seek latency is kept separate from sustained
//! sequential frame cost. No proxy, reduced representation, read-ahead,
//! generated file, or retained decoded-frame cache participates.

use super::service::{DecodeService, MediaSource, ServiceConfig, SessionGuard};
use crate::error::{MediaError, Result};
use crate::frame::DecodedFrame;
use crate::probe::CostClass;
use crate::vt::DecodePurpose;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

const MIN_FRESH_FRAMES: usize = 8;
const MAX_FRESH_FRAMES: usize = 16;
const MAX_CANDIDATES: usize = 32;

/// Measured cost of decoding the original full-size display representation.
/// `needs_proxy` is a decode-only signal; the caller also considers GPU
/// headroom, power state, and the viewer's output size.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeMeasurement {
    pub codec: String,
    pub coded_size: (u32, u32),
    pub cost_class: CostClass,
    pub frame_budget_ms: f64,
    /// P95 of fresh sequential exact frame requests, excluding warm-up seeks.
    pub sequential_p95_ms: Option<f64>,
    /// P95 of the separate initial seek in each sampled window.
    pub random_seek_p95_ms: Option<f64>,
    pub fresh_frames: usize,
    pub sufficient_samples: bool,
    /// True only after at least eight fresh samples exceed the source budget.
    pub needs_proxy: bool,
}

/// Measures video from its original path. Non-video media return `None`.
/// The first frame of each window is a random-seek warm-up and is never
/// included in sustained decode cost. Decoder and pool state are dropped on
/// return, including errors; the service's decoded-frame cache is disabled.
pub fn measure_source(path: &Path) -> Result<Option<DecodeMeasurement>> {
    measure_source_with_pause(path, || Ok(()))
}

/// As [`measure_source`], calling `wait_until_idle` before opening and
/// before each exact frame request. Job callers wait for playback and
/// higher-priority work there, without holding a media or GPU lock.
pub fn measure_source_with_pause(
    path: &Path,
    mut wait_until_idle: impl FnMut() -> Result<()>,
) -> Result<Option<DecodeMeasurement>> {
    wait_until_idle()?;
    let mut head = [0u8; 512];
    let read = {
        use std::io::Read;
        std::fs::File::open(path)?.read(&mut head)?
    };
    if crate::image::sniff(&head[..read]) != crate::image::ImageFormat::Other
        && crate::image::animation(path).ok().flatten().is_none()
    {
        return Ok(None);
    }
    let source = match MediaSource::open(path) {
        Ok(source) => source,
        Err(MediaError::Invalid(message)) if message == "no video stream" => return Ok(None),
        Err(error) => return Err(error),
    };
    let service = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        pool_free_bytes: 0,
        idle_sessions: 0,
        session_window: 1,
        ..ServiceConfig::default()
    });
    measure_video(&source, &service, &mut wait_until_idle).map(Some)
}

fn percentile95(times: &[f64]) -> Option<f64> {
    if times.is_empty() {
        return None;
    }
    let mut ordered = times.to_vec();
    ordered.sort_by(f64::total_cmp);
    Some(ordered[(ordered.len() * 95).div_ceil(100) - 1])
}

/// (warm-up frame, number of following candidates, fresh-frame cap).
fn windows(frames: usize) -> Vec<(usize, usize, usize)> {
    if frames < 2 {
        return Vec::new();
    }
    if frames >= 64 {
        // Two distinct sections capture changes in picture complexity.
        return [frames / 4, frames / 4 * 3]
            .into_iter()
            .map(|middle| (middle - 1, 16.min(frames - middle), 8))
            .collect();
    }
    let candidates = (frames - 1).min(MAX_CANDIDATES);
    let start = (frames - 1 - candidates) / 2;
    vec![(start, candidates, MAX_FRESH_FRAMES)]
}

/// One exact frame's active wall time. The callback also runs between
/// decoder input batches, so a newly started playback pauses a long seek.
/// Time spent waiting for higher-priority work is excluded from the sample.
fn timed_frame(
    session: &mut SessionGuard<'_>,
    index: usize,
    wait_until_idle: &mut impl FnMut() -> Result<()>,
) -> Result<(DecodedFrame, f64)> {
    wait_until_idle()?;
    let started = Instant::now();
    let mut paused = std::time::Duration::ZERO;
    let mut aborted = None;
    let frame = session.frame_with(index, &mut |_| {
        let pause_started = Instant::now();
        match wait_until_idle() {
            Ok(()) => {
                paused += pause_started.elapsed();
                false
            }
            Err(error) => {
                aborted = Some(error);
                true
            }
        }
    })?;
    if let Some(error) = aborted {
        return Err(error);
    }
    let frame = frame.ok_or(MediaError::Cancelled)?;
    let milliseconds = started.elapsed().saturating_sub(paused).as_secs_f64() * 1_000.0;
    Ok((frame, milliseconds))
}

fn measure_video(
    source: &Arc<MediaSource>,
    service: &DecodeService,
    wait_until_idle: &mut impl FnMut() -> Result<()>,
) -> Result<DecodeMeasurement> {
    let video = source
        .probe
        .video
        .as_ref()
        .ok_or_else(|| MediaError::invalid("no video stream"))?;
    wait_until_idle()?;
    let mut session = service.session(source, 0, DecodePurpose::Display)?;
    let mut sequential = Vec::with_capacity(MAX_FRESH_FRAMES);
    let mut seeks = Vec::with_capacity(2);
    for (warmup, candidates, cap) in windows(source.index.frames.len()) {
        let (frame, milliseconds) = timed_frame(&mut session, warmup, wait_until_idle)?;
        if warmup > 0 {
            seeks.push(milliseconds);
        }
        drop(frame);
        let stop_at = (warmup + 1 + candidates).min(source.index.frames.len());
        let target = sequential.len() + cap;
        for n in warmup + 1..stop_at {
            let before = session.stats().samples_fed;
            let (frame, elapsed) = timed_frame(&mut session, n, wait_until_idle)?;
            drop(frame);
            if session.stats().samples_fed > before {
                sequential.push(elapsed);
            }
            if sequential.len() >= target {
                break;
            }
        }
    }
    let fresh_frames = sequential.len();
    let frame_budget_ms = source.rate().den() as f64 * 1_000.0 / source.rate().num() as f64;
    let sequential_p95_ms = percentile95(&sequential);
    let sufficient_samples = fresh_frames >= MIN_FRESH_FRAMES;
    Ok(DecodeMeasurement {
        codec: source.info.codec.clone(),
        coded_size: (video.width, video.height),
        cost_class: video.decode.class,
        frame_budget_ms,
        sequential_p95_ms,
        random_seek_p95_ms: percentile95(&seeks),
        fresh_frames,
        sufficient_samples,
        needs_proxy: sufficient_samples
            && sequential_p95_ms.is_some_and(|p95| p95 > frame_budget_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_decode_measurement_uses_fresh_frames_and_drops_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pattern.mov");
        let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                    .join("Library/Caches/powermove-engine/ffmpeg/out")
            })
            .join("bin/ffmpeg");
        let encoded = std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30",
                "-frames:v",
                "60",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "2",
                "-pix_fmt",
                "yuv422p10le",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(encoded.success());
        let original = std::fs::read(&path).unwrap();
        let source = MediaSource::open(&path).unwrap();
        let service = DecodeService::new(ServiceConfig {
            cache_bytes: 0,
            pool_free_bytes: 0,
            idle_sessions: 0,
            session_window: 1,
            ..ServiceConfig::default()
        });
        let mut pause_checks = 0;
        let measured = measure_video(&source, &service, &mut || {
            pause_checks += 1;
            Ok(())
        })
        .unwrap();
        assert!(measured.sufficient_samples, "{measured:?}");
        assert!((MIN_FRESH_FRAMES..=MAX_FRESH_FRAMES).contains(&measured.fresh_frames));
        assert!(pause_checks > measured.fresh_frames);
        assert!(measured.sequential_p95_ms.unwrap().is_finite());
        assert!(measured.random_seek_p95_ms.unwrap().is_finite());
        assert_eq!(measured.coded_size, (160, 90));
        assert_eq!(service.cache().stats().frames, 0);
        assert_eq!(service.usage().sessions, 0);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 1);
    }

    #[test]
    fn short_source_never_claims_sufficient_evidence() {
        assert!(windows(1).is_empty());
        assert_eq!(windows(6), [(0, 5, MAX_FRESH_FRAMES)]);
        assert_eq!(percentile95(&[]), None);
        assert_eq!(percentile95(&[2.0, 4.0, 6.0]), Some(6.0));
    }

    #[test]
    fn still_and_audio_sources_have_no_decode_measurement() {
        let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../apps/desktop/e2e/fixtures");
        assert_eq!(
            measure_source(&fixtures.join("still-red.png")).unwrap(),
            None
        );
        assert_eq!(measure_source(&fixtures.join("tone.wav")).unwrap(), None);
    }
}
