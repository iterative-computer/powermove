//! The export pipeline after rendering (section 17):
//!
//! ```text
//! working frame → encoder input (matrix, siting-aware chroma, dither) → sink
//! ```
//!
//! The production sink encodes with VideoToolbox, muxes with AVAssetWriter, verifies,
//! and moves the file into place (`mac::VideoToolboxSink`). Test sinks capture the
//! encoder input itself, so tests can compare it exactly without a codec in the way.

use crate::Error;
use crate::convert::{ConvertParams, EncoderInput, WorkingFrame, convert};
use crate::settings::ExportSpec;

/// Where encoder input goes.
pub trait Sink {
    type Output;
    /// Frame `index` (0-based, in order).
    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error>;
    /// Interleaved f32 PCM at the spec's sample rate.
    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error>;
    fn finish(self) -> Result<Self::Output, Error>;
}

/// One export in progress.
pub struct Pipeline<S: Sink> {
    spec: ExportSpec,
    sink: S,
    next: u64,
    audio_frames: u64,
    legacy_png: bool,
}

impl<S: Sink> Pipeline<S> {
    pub fn new(spec: ExportSpec, sink: S) -> Result<Self, Error> {
        spec.validate()?;
        Ok(Self {
            spec,
            sink,
            next: 0,
            audio_frames: 0,
            legacy_png: false,
        })
    }

    /// Select the PNG boundary from the frozen document, once per job.
    pub fn set_legacy_png(&mut self, legacy_png: bool) {
        self.legacy_png = legacy_png;
    }

    pub fn spec(&self) -> &ExportSpec {
        &self.spec
    }

    /// Frames delivered so far.
    pub fn frames(&self) -> u64 {
        self.next
    }

    /// Audio sample frames delivered so far.
    pub fn audio_frames(&self) -> u64 {
        self.audio_frames
    }

    fn admit(&self) -> Result<u64, Error> {
        if self.next >= self.spec.frames {
            return Err(Error::InvalidFrame(format!(
                "the export has all {} frames",
                self.spec.frames
            )));
        }
        Ok(self.next)
    }

    /// Converts a premultiplied RGBA working frame on the CPU and delivers it. The frame
    /// index seeds the dither.
    pub fn push_frame(&mut self, frame: &WorkingFrame) -> Result<(), Error> {
        let index = self.admit()?;
        if frame.width != self.spec.width || frame.height != self.spec.height {
            return Err(Error::InvalidFrame(format!(
                "frame is {}×{}, the export is {}×{}",
                frame.width, frame.height, self.spec.width, self.spec.height
            )));
        }
        let params = ConvertParams {
            legacy_png: self.legacy_png,
            working: self.spec.working,
            alpha: self.spec.alpha,
            dither: self.spec.dither_enabled(),
            frame: index as u32,
        };
        let input = convert(frame, self.spec.input_format(), &params)?;
        self.sink.video(index, &input)?;
        self.next += 1;
        Ok(())
    }

    /// Delivers encoder input converted elsewhere (the GPU encoder-input pass, or
    /// Y′CbCr planes copied on pass-through).
    pub fn push_encoder_input(&mut self, input: &EncoderInput) -> Result<(), Error> {
        let index = self.admit()?;
        input.validate()?;
        if input.format != self.spec.input_format()
            || input.width != self.spec.width
            || input.height != self.spec.height
        {
            return Err(Error::InvalidFrame(
                "encoder input format or size does not match the export".into(),
            ));
        }
        self.sink.video(index, input)?;
        self.next += 1;
        Ok(())
    }

    /// Delivers interleaved f32 PCM. Push audio as the frames advance (for example one
    /// frame's worth at a time) so the muxer can interleave.
    pub fn push_audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        let Some(audio) = self.spec.audio else {
            return Err(Error::InvalidFrame("this export has no audio track".into()));
        };
        if !interleaved.len().is_multiple_of(audio.channels as usize) {
            return Err(Error::InvalidFrame(
                "audio samples are not a whole number of frames".into(),
            ));
        }
        self.sink.audio(interleaved)?;
        self.audio_frames += (interleaved.len() / audio.channels as usize) as u64;
        Ok(())
    }

    /// Finishes once every frame is delivered.
    pub fn finish(self) -> Result<S::Output, Error> {
        if self.next != self.spec.frames {
            return Err(Error::InvalidFrame(format!(
                "{} of {} frames were delivered",
                self.next, self.spec.frames
            )));
        }
        self.sink.finish()
    }

    /// Abandons the export; the sink discards partial output when dropped.
    pub fn cancel(self) {}
}

/// Test-only sinks (section 17): capture encoder input instead of encoding it.
#[cfg(any(test, feature = "test-sinks"))]
pub mod test_sinks {
    use std::fs::File;
    use std::io::{BufWriter, Read, Write};
    use std::path::{Path, PathBuf};

    use super::Sink;
    use crate::Error;
    use crate::convert::{EncoderInput, Plane, PlaneData, plane_layout};
    use crate::settings::InputFormat;

    /// Keeps every frame and all audio in memory.
    #[derive(Default, Debug)]
    pub struct MemorySink {
        pub frames: Vec<EncoderInput>,
        pub audio: Vec<f32>,
    }

    impl Sink for MemorySink {
        type Output = MemorySink;

        fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
            assert_eq!(index as usize, self.frames.len(), "frames arrive in order");
            self.frames.push(input.clone());
            Ok(())
        }

        fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
            self.audio.extend_from_slice(interleaved);
            Ok(())
        }

        fn finish(self) -> Result<MemorySink, Error> {
            Ok(self)
        }
    }

    /// Writes every frame's planes, tightly packed and little-endian, one frame after
    /// another (the S2 raw sink format). Audio is ignored.
    pub struct RawFileSink {
        path: PathBuf,
        out: BufWriter<File>,
        frames: u64,
    }

    impl RawFileSink {
        pub fn create(path: &Path) -> Result<Self, Error> {
            let file =
                File::create(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
            Ok(Self {
                path: path.to_path_buf(),
                out: BufWriter::new(file),
                frames: 0,
            })
        }
    }

    impl Sink for RawFileSink {
        type Output = (PathBuf, u64);

        fn video(&mut self, _index: u64, input: &EncoderInput) -> Result<(), Error> {
            for plane in &input.planes {
                let bytes: Vec<u8> = match &plane.data {
                    PlaneData::U16(v) => v.iter().flat_map(|s| s.to_le_bytes()).collect(),
                    PlaneData::U8(v) => v.clone(),
                };
                self.out
                    .write_all(&bytes)
                    .map_err(|e| Error::Io(e.to_string()))?;
            }
            self.frames += 1;
            Ok(())
        }

        fn audio(&mut self, _: &[f32]) -> Result<(), Error> {
            Ok(())
        }

        fn finish(mut self) -> Result<(PathBuf, u64), Error> {
            self.out.flush().map_err(|e| Error::Io(e.to_string()))?;
            Ok((self.path, self.frames))
        }
    }

    /// Reads a raw sink file back into frames.
    pub fn read_raw(
        path: &Path,
        format: InputFormat,
        width: u32,
        height: u32,
    ) -> Result<Vec<EncoderInput>, Error> {
        let mut bytes = vec![];
        File::open(path)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(|e| Error::Io(e.to_string()))?;
        let layout = plane_layout(format, width, height);
        let frame_bytes: usize = layout
            .iter()
            .map(|&(s, r, wide)| s * r * if wide { 2 } else { 1 })
            .sum();
        if frame_bytes == 0 || bytes.len() % frame_bytes != 0 {
            return Err(Error::Io(format!(
                "{} is not a whole number of frames",
                path.display()
            )));
        }
        let mut frames = vec![];
        let mut at = 0;
        while at < bytes.len() {
            let mut planes = vec![];
            for &(samples, rows, wide) in &layout {
                let n = samples * rows;
                let data = if wide {
                    let v = bytes[at..at + 2 * n]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    at += 2 * n;
                    PlaneData::U16(v)
                } else {
                    let v = bytes[at..at + n].to_vec();
                    at += n;
                    PlaneData::U8(v)
                };
                planes.push(Plane {
                    data,
                    row_samples: samples,
                    rows,
                });
            }
            frames.push(EncoderInput {
                format,
                width,
                height,
                planes,
            });
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::test_sinks::{MemorySink, RawFileSink, read_raw};
    use super::*;
    use crate::settings::*;
    use pm_time::Rate;

    fn spec(codec: Codec, frames: u64) -> ExportSpec {
        ExportSpec {
            width: 32,
            height: 16,
            rate: Rate::FPS_25,
            frames,
            codec,
            container: if matches!(codec, Codec::ProRes(_)) {
                Container::Mov
            } else {
                Container::Mp4
            },
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::Default,
            audio: Some(AudioSpec {
                sample_rate: 48_000,
                channels: 2,
                codec: AudioCodec::Aac,
            }),
            timecode: None,
            destination: PathBuf::from("/tmp/unused.mov"),
            overwrite: Overwrite::Fail,
            mode: None,
        }
    }

    fn ramp(frame: u32) -> Vec<f32> {
        (0..16u32)
            .flat_map(|y| {
                (0..32u32).flat_map(move |x| {
                    [
                        x as f32 / 31.0,
                        y as f32 / 15.0,
                        (frame % 4) as f32 / 3.0,
                        1.0,
                    ]
                })
            })
            .collect()
    }

    #[test]
    fn frames_reach_the_sink_converted_in_order_with_per_frame_dither() {
        let mut p = Pipeline::new(
            spec(
                Codec::H264 {
                    bitrate_bps: 4_000_000,
                },
                3,
            ),
            MemorySink::default(),
        )
        .unwrap();
        let f = ramp(0);
        for _ in 0..3 {
            p.push_frame(&WorkingFrame {
                width: 32,
                height: 16,
                rgba: &f,
            })
            .unwrap();
            p.push_audio(&[0.0; 2 * 1920]).unwrap();
        }
        assert!(
            p.push_frame(&WorkingFrame {
                width: 32,
                height: 16,
                rgba: &f
            })
            .is_err(),
            "a fourth frame"
        );
        let sink = p.finish().unwrap();
        assert_eq!(sink.frames.len(), 3);
        assert_eq!(sink.audio.len(), 3 * 2 * 1920);
        assert!(sink.frames.iter().all(|f| f.format == InputFormat::V420));
        // Same pixels, different frame index: the 8-bit default dither differs per frame.
        assert_ne!(sink.frames[0], sink.frames[1]);
    }

    #[test]
    fn incomplete_exports_and_mismatched_input_fail() {
        let mut p = Pipeline::new(
            spec(Codec::ProRes(ProResProfile::Hq), 2),
            MemorySink::default(),
        )
        .unwrap();
        let f = ramp(0);
        p.push_frame(&WorkingFrame {
            width: 32,
            height: 16,
            rgba: &f,
        })
        .unwrap();
        assert!(
            p.push_frame(&WorkingFrame {
                width: 16,
                height: 32,
                rgba: &f
            })
            .is_err()
        );
        assert!(
            p.push_audio(&[0.0; 3]).is_err(),
            "odd sample count for stereo"
        );
        let wrong = crate::convert::convert(
            &WorkingFrame {
                width: 32,
                height: 16,
                rgba: &f,
            },
            InputFormat::Y416,
            &ConvertParams {
                legacy_png: true,
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                dither: false,
                frame: 0,
            },
        )
        .unwrap();
        assert!(
            p.push_encoder_input(&wrong).is_err(),
            "y416 into a 4:2:2 export"
        );
        assert!(p.finish().is_err(), "one of two frames");
    }

    #[test]
    fn raw_file_sink_round_trips_every_format() {
        let dir = std::env::temp_dir();
        for (codec, i) in [
            (Codec::ProRes(ProResProfile::P4444), 0),
            (Codec::ProRes(ProResProfile::Standard), 1),
            (
                Codec::H264 {
                    bitrate_bps: 1_000_000,
                },
                2,
            ),
        ] {
            let path = dir.join(format!("pm-export-raw-{}-{i}.raw", std::process::id()));
            let mut s = spec(codec, 2);
            s.audio = None;
            let mut memory = Pipeline::new(s.clone(), MemorySink::default()).unwrap();
            let mut raw = Pipeline::new(s, RawFileSink::create(&path).unwrap()).unwrap();
            for frame in 0..2 {
                let f = ramp(frame);
                memory
                    .push_frame(&WorkingFrame {
                        width: 32,
                        height: 16,
                        rgba: &f,
                    })
                    .unwrap();
                raw.push_frame(&WorkingFrame {
                    width: 32,
                    height: 16,
                    rgba: &f,
                })
                .unwrap();
            }
            let (written, n) = raw.finish().unwrap();
            assert_eq!(n, 2);
            let back = read_raw(&written, codec.input_format(false), 32, 16).unwrap();
            assert_eq!(back, memory.finish().unwrap().frames);
            std::fs::remove_file(written).unwrap();
        }
    }
}
