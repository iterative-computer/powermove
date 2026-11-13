//! macOS delivery: VideoToolbox encode, AVAssetWriter mux, verification, atomic move.

mod alpha;
mod cf;
pub mod pixbuf;
pub mod reader;
mod vt;
mod writer;

use crate::Delivered;
use crate::Error;
use crate::atomic::AtomicOutput;
use crate::convert::EncoderInput;
use crate::mp4box::Movie;
use crate::pipeline::Sink;
use crate::settings::{ExportSpec, Overwrite};
use crate::verify::{self, Evidence, Expected};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The production sink: VideoToolbox → AVAssetWriter → verify → move into place.
pub struct VideoToolboxSink {
    // Field order is drop order: the encoder and the writer (which cancels writing)
    // stop before `output` removes the temporary file.
    encoder: vt::Encoder,
    writer: Option<writer::Writer>,
    pool: pixbuf::Pool,
    output: AtomicOutput,
    spec: ExportSpec,
    evidence: Evidence,
    /// Alpha exports: samples re-described with depth 32 (see `alpha`).
    alpha: Option<alpha::AlphaDescriptions>,
    /// Frames of the written file checked so far.
    checked: Arc<AtomicU64>,
}

impl VideoToolboxSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        spec.validate()?;
        if !spec.container.is_quicktime_family() {
            return Err(Error::InvalidSpec(format!(
                "{:?} is not written by AVAssetWriter",
                spec.container
            )));
        }
        let timing = spec.timing()?;
        let output = match spec.overwrite {
            Overwrite::InPlace => {
                AtomicOutput::in_place(&spec.destination, spec.container.extension(), spec.mode)?
            }
            _ => AtomicOutput::new(&spec.destination, spec.overwrite, spec.mode)?,
        };
        let writer = writer::Writer::new(
            output.temp_path(),
            spec.container,
            timing,
            spec.frames,
            spec.audio,
            spec.timecode.as_ref(),
        )?;
        Ok(Self {
            spec: spec.clone(),
            pool: pixbuf::Pool::new(spec.input_format(), spec.width, spec.height)?,
            encoder: vt::Encoder::new(spec, timing)?,
            writer: Some(writer),
            output,
            evidence: Evidence::default(),
            // ProRes describes opaque frames without alpha (see `alpha`); HEVC
            // with alpha is its own codec type and describes its alpha layer.
            alpha: (spec.alpha != crate::settings::AlphaOutput::None
                && matches!(spec.codec, crate::settings::Codec::ProRes(_)))
            .then(alpha::AlphaDescriptions::default),
            checked: Arc::default(),
        })
    }

    fn writer(&mut self) -> &mut writer::Writer {
        self.writer.as_mut().expect("writer lives until finish")
    }

    fn described(
        &mut self,
        samples: Vec<objc2_core_foundation::CFRetained<objc2_core_media::CMSampleBuffer>>,
    ) -> Result<Vec<objc2_core_foundation::CFRetained<objc2_core_media::CMSampleBuffer>>, Error>
    {
        match self.alpha.as_mut() {
            Some(a) => samples.into_iter().map(|s| a.sample(s)).collect(),
            None => Ok(samples),
        }
    }

    fn finish_inner(mut self, cancel: Option<&AtomicBool>) -> Result<Delivered, Error> {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(Error::Cancelled);
        }
        let samples = self.encoder.complete()?;
        let samples = self.described(samples)?;
        let mut writer = self.writer.take().expect("writer lives until finish");
        writer.push_video(samples)?;
        let audio_frames = writer.audio_frames();
        writer.finish()?;
        let temp = self.output.temp_path().to_path_buf();
        if let Some(audio) = self.spec.audio
            && self.spec.container == crate::settings::Container::Mov
        {
            crate::mp4box::add_channel_layout(&temp, audio.channels)?;
        }
        let (report, problems) = examine(
            &temp,
            &self.spec,
            std::mem::take(&mut self.evidence),
            audio_frames,
            cancel,
            &self.checked,
        )?;
        if !problems.is_empty() {
            return Err(Error::Verify(problems.join("; ")));
        }
        let mut report = report;
        report.hardware_encoder = self.encoder.hardware;
        report.bytes = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}

/// Holds a written QuickTime-family file to what the export asked for and
/// to the `evidence` of the frames its encoder was handed: the container,
/// the tags, the sound, the timecode, and every frame's picture.
pub fn verify_file(
    path: &std::path::Path,
    spec: &ExportSpec,
    evidence: Evidence,
    audio_frames: u64,
    cancel: Option<&AtomicBool>,
) -> Result<verify::VerifyReport, Error> {
    let (report, problems) = examine(
        path,
        spec,
        evidence,
        audio_frames,
        cancel,
        &AtomicU64::new(0),
    )?;
    if problems.is_empty() {
        Ok(report)
    } else {
        Err(Error::Verify(problems.join("; ")))
    }
}

/// [`verify_file`], with what was measured and every problem found.
pub fn examine_file(
    path: &std::path::Path,
    spec: &ExportSpec,
    evidence: Evidence,
    audio_frames: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(verify::VerifyReport, Vec<String>), Error> {
    examine(
        path,
        spec,
        evidence,
        audio_frames,
        cancel,
        &AtomicU64::new(0),
    )
}

/// [`examine_file`], counting the frames checked in `checked`.
fn examine(
    path: &std::path::Path,
    spec: &ExportSpec,
    evidence: Evidence,
    audio_frames: u64,
    cancel: Option<&AtomicBool>,
    checked: &AtomicU64,
) -> Result<(verify::VerifyReport, Vec<String>), Error> {
    let timing = spec.timing()?;
    let movie = Movie::read(path)?;
    let decoded = std::sync::Mutex::new(verify::Decoded::default());
    let format = spec.input_format();
    let sampled = verify::sampled_frames(spec.frames);
    reader::decode_frames(
        path,
        timing,
        format,
        spec.frames,
        &|index, frame| {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            // Only the sampled frames are copied out of the decoder's buffer.
            let signature = frame.signature(format)?;
            let whole = if sampled.contains(&index) {
                Some(frame.read(format)?)
            } else {
                None
            };
            let mut decoded = decoded.lock().unwrap();
            decoded.frames.push((index, signature));
            decoded.sampled.extend(whole.map(|f| (index, f)));
            Ok(())
        },
        checked,
    )?;
    let mut decoded = decoded.into_inner().unwrap();
    decoded.complete = true;
    let expected = Expected::new(spec, timing, audio_frames, evidence);
    verify::examine(&expected, &movie, &decoded)
}

impl Sink for VideoToolboxSink {
    type Output = Delivered;

    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        self.evidence.add(index, self.spec.frames, input);
        let buffer = self.pool.buffer()?;
        buffer.write(input)?;
        buffer.tag(
            self.spec.tags,
            self.spec.alpha,
            input
                .planes
                .get(1)
                .is_some_and(|c| c.rows < input.planes[0].rows),
        );
        self.encoder.encode(&buffer, index)?;
        let samples = self.encoder.take()?;
        let samples = self.described(samples)?;
        self.writer().push_video(samples)
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        self.writer().push_audio(interleaved)
    }

    fn idle(&mut self) -> Result<(), Error> {
        if !self.encoder.holds_frames() {
            return Ok(());
        }
        let samples = self.encoder.complete()?;
        let samples = self.described(samples)?;
        self.writer().push_video(samples)
    }

    fn count_checked(&mut self, checked: Arc<AtomicU64>) {
        self.checked = checked;
    }

    fn finish(self) -> Result<Delivered, Error> {
        self.finish_inner(None)
    }

    fn finish_with_cancel(self, cancel: &AtomicBool) -> Result<Delivered, Error> {
        self.finish_inner(Some(cancel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{ConvertParams, WorkingFrame, convert};
    use crate::settings::{
        AlphaOutput, Codec, ColorTags, Container, Dither, ProResProfile, WorkingEncoding,
    };

    /// A file that fails its check is not kept, the person is told in one
    /// sentence what was found first, and nothing is tried again.
    #[test]
    fn a_file_that_fails_its_check_is_removed_and_told_in_one_sentence() {
        let dir = std::env::temp_dir().join(format!("pm-export-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (160u32, 96u32);
        let spec = ExportSpec {
            width: w,
            height: h,
            rate: pm_time::Rate::new(25, 1).unwrap(),
            frames: 4,
            codec: Codec::ProRes(ProResProfile::Hq),
            container: Container::Mov,
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::Off,
            audio: None,
            timecode: None,
            destination: dir.join("refused.mov"),
            overwrite: Overwrite::Replace,
            mode: None,
        };
        let mut sink = VideoToolboxSink::new(&spec).unwrap();
        for n in 0..4u32 {
            let rgba: Vec<f32> = (0..w * h)
                .flat_map(|i| {
                    let v = 0.2 + 0.6 * (i % w) as f32 / w as f32;
                    [v, v * 0.8 + 0.05 * n as f32, 0.5, 1.0]
                })
                .collect();
            let input = convert(
                &WorkingFrame {
                    width: w,
                    height: h,
                    rgba: &rgba,
                },
                spec.input_format(),
                &ConvertParams {
                    working: spec.working,
                    alpha: spec.alpha,
                    tags: spec.tags,
                    dither: false,
                    frame: n,
                },
            )
            .unwrap();
            sink.video(u64::from(n), &input).unwrap();
        }
        // The encoder was handed a frame 2 with colour the file does not
        // carry (as a broken encoder or reader would deliver it).
        sink.evidence.frames[2].cells[1] = [0.2; 64];
        let error = sink.finish().unwrap_err();
        assert!(matches!(error, Error::Verify(_)), "{error:?}");
        let said = error.to_string();
        assert!(
            said.starts_with("The exported file failed its check and was removed: frame 2 ")
                && said.ends_with('.')
                && !said.contains("; "),
            "{said}"
        );
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "no destination, temporary or work file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
