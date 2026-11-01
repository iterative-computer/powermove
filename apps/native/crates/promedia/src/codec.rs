//! Native codecs are injected; VideoToolbox/AVFoundation belong to the macOS host.
//! Adapters preserve timestamps, colour interpretation, alpha and audio. The
//! encoder owns a private partial output and must not publish before finish.
use crate::media::MediaInfo;
use std::io;
#[derive(Clone, Debug, PartialEq)]
pub enum Sample {
    Video {
        tick: i64,
        duration: u64,
        width: u32,
        height: u32,
        rgba: Vec<[f32; 4]>,
    },
    Audio {
        tick: i64,
        channels: u16,
        sample_rate: u32,
        interleaved: Vec<f32>,
    },
}
impl Sample {
    pub fn validate(&self) -> io::Result<()> {
        let valid = match self {
            Self::Video {
                duration,
                width,
                height,
                rgba,
                ..
            } => {
                *duration > 0
                    && *width > 0
                    && *height > 0
                    && (*width as usize).checked_mul(*height as usize) == Some(rgba.len())
                    && rgba.iter().flatten().all(|v| v.is_finite())
            }
            Self::Audio {
                channels,
                sample_rate,
                interleaved,
                ..
            } => {
                *channels > 0
                    && *sample_rate > 0
                    && !interleaved.is_empty()
                    && interleaved.len() % usize::from(*channels) == 0
                    && interleaved.iter().all(|v| v.is_finite())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid decoded sample",
            ))
        }
    }
}
pub trait Decoder: Send {
    fn info(&self) -> &MediaInfo;
    fn next_sample(&mut self) -> io::Result<Option<Sample>>;
}
pub trait Encoder: Send {
    fn write(&mut self, sample: &Sample) -> io::Result<()>;
    /// Flush and sync the private output. No model attachment here.
    fn finish(&mut self) -> io::Result<()>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Pause,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscodeOutcome {
    Complete { samples: u64 },
    Paused { samples: u64 },
    Cancelled { samples: u64 },
}
/// Bounded to one decoded sample in flight. Adapters scale/encode explicitly;
/// stopping does not call finish. The host drops/releases adapters before
/// acknowledging a control request. Resume uses a new decoder/encoder or an
/// adapter checkpoint; this function never silently duplicates partial output.
pub fn transcode(
    decoder: &mut dyn Decoder,
    encoder: &mut dyn Encoder,
    mut control: impl FnMut(u64) -> Control,
) -> io::Result<TranscodeOutcome> {
    let info = decoder.info().clone();
    info.validate()?;
    let mut count = 0u64;
    let mut video_seen = false;
    loop {
        match control(count) {
            Control::Pause => return Ok(TranscodeOutcome::Paused { samples: count }),
            Control::Cancel => return Ok(TranscodeOutcome::Cancelled { samples: count }),
            Control::Continue => {}
        }
        match decoder.next_sample()? {
            Some(sample) => {
                match control(count) {
                    Control::Pause => return Ok(TranscodeOutcome::Paused { samples: count }),
                    Control::Cancel => return Ok(TranscodeOutcome::Cancelled { samples: count }),
                    Control::Continue => {}
                }
                sample.validate()?;
                match &sample {
                    Sample::Video { width, height, .. } => {
                        if *width != info.width || *height != info.height {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "decoded dimensions disagree with source metadata",
                            ));
                        }
                        video_seen = true;
                    }
                    Sample::Audio {
                        channels,
                        sample_rate,
                        ..
                    } => {
                        if *channels != info.audio_channels
                            || *sample_rate != info.audio_sample_rate
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "decoded audio disagrees with source metadata",
                            ));
                        }
                    }
                }
                encoder.write(&sample)?;
                count = count
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("sample count overflow"))?;
            }
            None => {
                // Recheck after a blocking decoder call before final publication.
                match control(count) {
                    Control::Pause => return Ok(TranscodeOutcome::Paused { samples: count }),
                    Control::Cancel => return Ok(TranscodeOutcome::Cancelled { samples: count }),
                    Control::Continue => {}
                }
                if !video_seen {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "decoder returned no video samples",
                    ));
                }
                encoder.finish()?;
                return Ok(TranscodeOutcome::Complete { samples: count });
            }
        }
    }
}
