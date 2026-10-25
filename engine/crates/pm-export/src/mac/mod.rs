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
use crate::timescale::VideoTiming;
use crate::verify::{self, BlockSample, Expected};
use std::sync::atomic::AtomicBool;

/// The production sink: VideoToolbox → AVAssetWriter → verify → move into place.
pub struct VideoToolboxSink {
    // Field order is drop order: the encoder and the writer (which cancels writing)
    // stop before `output` removes the temporary file.
    encoder: vt::Encoder,
    writer: Option<writer::Writer>,
    pool: pixbuf::Pool,
    output: AtomicOutput,
    spec: ExportSpec,
    timing: VideoTiming,
    sampled: Vec<u64>,
    blocks: Vec<BlockSample>,
    /// Alpha exports: samples re-described with depth 32 (see `alpha`).
    alpha: Option<alpha::AlphaDescriptions>,
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
            timing,
            pool: pixbuf::Pool::new(spec.input_format(), spec.width, spec.height)?,
            encoder: vt::Encoder::new(spec, timing)?,
            writer: Some(writer),
            output,
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            // ProRes describes opaque frames without alpha (see `alpha`); HEVC
            // with alpha is its own codec type and describes its alpha layer.
            alpha: (spec.alpha != crate::settings::AlphaOutput::None
                && matches!(spec.codec, crate::settings::Codec::ProRes(_)))
            .then(alpha::AlphaDescriptions::default),
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
        if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
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
        let movie = Movie::read(&temp)?;
        let format = self.spec.input_format();
        let decoded = self
            .sampled
            .iter()
            .map(|&f| reader::decode_frame(&temp, f, self.timing, format).map(|d| (f, d)))
            .collect::<Result<Vec<_>, _>>()?;
        let expected = Expected::new(
            &self.spec,
            self.timing,
            audio_frames,
            std::mem::take(&mut self.blocks),
        );
        let mut report = verify::check(&expected, &movie, &decoded)?;
        report.hardware_encoder = self.encoder.hardware;
        report.bytes = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}

impl Sink for VideoToolboxSink {
    type Output = Delivered;

    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        if self.sampled.contains(&index) {
            self.blocks.extend(verify::block_means(input, index));
        }
        let buffer = self.pool.buffer()?;
        buffer.write(input)?;
        buffer.tag(self.spec.tags, self.spec.alpha);
        self.encoder.encode(&buffer, index)?;
        let samples = self.encoder.take()?;
        let samples = self.described(samples)?;
        self.writer().push_video(samples)
    }

    fn audio(&mut self, interleaved: &[f32]) -> Result<(), Error> {
        self.writer().push_audio(interleaved)
    }

    fn finish(self) -> Result<Delivered, Error> {
        self.finish_inner(None)
    }

    fn finish_with_cancel(self, cancel: &AtomicBool) -> Result<Delivered, Error> {
        self.finish_inner(Some(cancel))
    }
}
