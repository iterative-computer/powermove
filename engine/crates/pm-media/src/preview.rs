//! Source posters and filmstrips, independent of a browser element or a
//! composition. Hosts resolve capability handles before opening a reader.
//! Frames retain native depth, alpha, orientation and color facts; the
//! renderer applies the source reading before thumbnail encoding.

use crate::decode::service::{DecodeService, MediaSource, ServiceConfig};
use crate::frame::DecodedFrame;
use crate::image::{self, StillImage, animation::AnimationTiming};
use crate::pool::FramePool;
use crate::vt::DecodePurpose;
use crate::{MediaError, Result};
use pm_protocol::color::ColorDescription;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub const MAX_FILMSTRIP_FRAMES: usize = 64;

#[derive(Clone, Debug)]
pub enum PreviewFrame {
    Image(StillImage),
    Video {
        frame: DecodedFrame,
        color: ColorDescription,
    },
}

impl PreviewFrame {
    pub fn decoded(&self) -> &DecodedFrame {
        match self {
            Self::Image(i) => &i.frame,
            Self::Video { frame, .. } => frame,
        }
    }
}

enum Source {
    Still,
    Animation(AnimationTiming),
    Sequence {
        sequence: image::sequence::ImageSequence,
        fps: f64,
    },
    Video {
        source: Arc<MediaSource>,
    },
}

/// One source, with a reusable video session and bounded decode cache.
/// A changed file reopens before the next read; offline/corrupt bytes are
/// errors, never a request to use another engine.
pub struct PreviewReader {
    path: PathBuf,
    stamp: (u64, SystemTime),
    source: Source,
    pool: FramePool,
    service: Arc<DecodeService>,
}

fn stamp(path: &Path) -> Result<(u64, SystemTime)> {
    let m = std::fs::metadata(path)?;
    Ok((m.len(), m.modified()?))
}

impl Drop for PreviewReader {
    fn drop(&mut self) {
        if let Source::Video { source } = &self.source {
            self.service.release(None, Some(source.id));
        }
    }
}

impl PreviewReader {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_service(path, Self::preview_service())
    }

    fn preview_service() -> Arc<DecodeService> {
        Arc::new(DecodeService::new(ServiceConfig {
            cache_bytes: 16 << 20,
            pool_free_bytes: 16 << 20,
            idle_sessions: 1,
            session_window: 2,
            ..ServiceConfig::default()
        }))
    }

    /// Engine consumers should supply their shared, lease-controlled
    /// service so previews share hardware arbitration and decoded memory.
    pub fn open_with_service(path: &Path, service: Arc<DecodeService>) -> Result<Self> {
        let signature = stamp(path)?;
        let mut head = [0; 512];
        let n = std::fs::File::open(path)?.read(&mut head)?;
        let source = if image::sniff(&head[..n]) != image::ImageFormat::Other {
            let timing = match image::sniff(&head[..n]) {
                image::ImageFormat::Png
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
                | image::ImageFormat::Heif
                | image::ImageFormat::Avif => image::animation::read_timing(path)?,
                _ => None,
            };
            match timing {
                Some(t) => Source::Animation(t),
                None => Source::Still,
            }
        } else {
            Source::Video {
                source: MediaSource::open(path)?,
            }
        };
        Ok(Self {
            path: path.into(),
            stamp: signature,
            source,
            pool: FramePool::new(0),
            service,
        })
    }

    /// Open exactly the host's selected files. Available frames play
    /// consecutively, including mixed padding and gaps, as today's import.
    pub fn open_sequence(paths: &[PathBuf], fps: f64) -> Result<Self> {
        if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
            return Err(MediaError::invalid(
                "sequence frame rate must be between 1 and 240 fps",
            ));
        }
        let sequence = image::sequence::ImageSequence::from_paths(paths)?;
        let path = sequence
            .available_file(0)
            .expect("at least two files")
            .to_path_buf();
        Ok(Self {
            stamp: stamp(&path)?,
            path,
            source: Source::Sequence { sequence, fps },
            pool: FramePool::new(0),
            service: Self::preview_service(),
        })
    }

    fn refresh(&mut self) -> Result<()> {
        // Sequence frames decode afresh by their selected paths, so a
        // replaced member is never served from a stale decoded cache.
        if matches!(self.source, Source::Sequence { .. }) {
            return Ok(());
        }
        if stamp(&self.path)? != self.stamp {
            let mut next = Self::open_with_service(&self.path, self.service.clone())?;
            // Outstanding still frames remain counted across replacements.
            next.pool = self.pool.clone();
            *self = next;
        }
        Ok(())
    }

    pub fn duration(&mut self) -> Result<f64> {
        self.refresh()?;
        Ok(match &self.source {
            Source::Still => 0.0,
            Source::Animation(t) => t.duration(),
            Source::Sequence { sequence, fps } => sequence.frames.len() as f64 / fps,
            Source::Video { source, .. } => source.probe.duration_us.unwrap_or(0) as f64 / 1e6,
        })
    }

    /// Still/animation frames held by this reader's pool (including
    /// results a consumer retains), to add to the engine's decoder lease
    /// category. The shared video service reports its own usage once.
    pub fn image_bytes(&self) -> usize {
        self.pool.stats().bytes
    }

    /// Geometry, field order and HDR metadata for the renderer's source
    /// pass. Decoded video frames themselves stay in coded orientation.
    pub fn video_probe(&mut self) -> Result<Option<crate::probe::VideoProbe>> {
        self.refresh()?;
        Ok(match &self.source {
            Source::Video { source } => source.probe.video.clone(),
            _ => None,
        })
    }

    /// Exact frame at source time. The caller's order is preserved, so
    /// reverse scrubbing and non-monotonic filmstrip requests work too.
    pub fn frame_at(&mut self, seconds: f64) -> Result<PreviewFrame> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(MediaError::invalid(
                "preview time must be finite and nonnegative",
            ));
        }
        self.refresh()?;
        match &self.source {
            Source::Still => image::read(&self.path, &self.pool).map(PreviewFrame::Image),
            Source::Animation(timing) => {
                let i = timing
                    .frame_at(seconds)
                    .ok_or_else(|| MediaError::invalid("animation has no preview frame"))?;
                image::read_animation_frame(&self.path, i, &self.pool).map(PreviewFrame::Image)
            }
            Source::Sequence { sequence, fps } => {
                let index =
                    (((seconds + 1e-7) * fps).floor() as usize).min(sequence.frames.len() - 1);
                let path = sequence.available_file(index).expect("clamped");
                image::read(path, &self.pool).map(PreviewFrame::Image)
            }
            Source::Video { source } => {
                let ticks = seconds * pm_time::TICKS_PER_SECOND as f64;
                if ticks >= i64::MAX as f64 {
                    return Err(MediaError::invalid("preview time exceeds the engine clock"));
                }
                let n = source
                    .select(ticks.round() as i64)
                    .ok_or_else(|| MediaError::invalid("no video frame at preview time"))?;
                let frame = self
                    .service
                    .session(source, 0, DecodePurpose::Export)?
                    .frame(n)?;
                Ok(PreviewFrame::Video {
                    color: frame.color.unwrap_or(source.color),
                    frame,
                })
            }
        }
    }

    /// Today's poster time: min(0.5 seconds, ten percent of the clip).
    pub fn poster(&mut self) -> Result<PreviewFrame> {
        let time = (self.duration()? * 0.1).clamp(0.0, 0.5);
        self.frame_at(time)
    }

    /// Stream frames to the consumer rather than retaining a filmstrip of
    /// full-resolution images. An error/cancellation from the consumer
    /// stops decoding immediately. No workers are left running on drop.
    pub fn filmstrip(
        &mut self,
        times: &[f64],
        mut consume: impl FnMut(usize, f64, PreviewFrame) -> Result<()>,
    ) -> Result<()> {
        if times.is_empty() || times.len() > MAX_FILMSTRIP_FRAMES {
            return Err(MediaError::invalid("a filmstrip needs 1 to 64 frames"));
        }
        if times.iter().any(|t| !t.is_finite() || *t < 0.0) {
            return Err(MediaError::invalid(
                "preview time must be finite and nonnegative",
            ));
        }
        for (i, &time) in times.iter().enumerate() {
            consume(i, time, self.frame_at(time)?)?;
        }
        Ok(())
    }
}
