//! macOS delivery: VideoToolbox encode, AVAssetWriter mux, verification, atomic move.

mod alpha;
mod cf;
pub mod pixbuf;
pub mod reader;
mod vt;
mod writer;

use std::path::PathBuf;

use crate::Error;
use crate::atomic::AtomicOutput;
use crate::convert::EncoderInput;
use crate::mp4box::Movie;
use crate::pipeline::{Pipeline, Sink};
use crate::settings::{ExportSpec, Overwrite};
use crate::timescale::VideoTiming;
use crate::verify::{self, BlockSample, Expected, VerifyReport};

/// A delivered, verified file.
#[derive(Clone, Debug)]
pub struct Delivered {
    pub path: PathBuf,
    pub report: VerifyReport,
}

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
            pool: pixbuf::Pool::new(spec.codec.input_format(), spec.width, spec.height)?,
            encoder: vt::Encoder::new(spec, timing)?,
            writer: Some(writer),
            output,
            sampled: verify::sampled_frames(spec.frames),
            blocks: vec![],
            alpha: (spec.alpha != crate::settings::AlphaOutput::None)
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

    fn finish(mut self) -> Result<Delivered, Error> {
        let samples = self.encoder.complete()?;
        let samples = self.described(samples)?;
        let mut writer = self.writer.take().expect("writer lives until finish");
        writer.push_video(samples)?;
        let audio_frames = writer.audio_frames();
        writer.finish()?;
        let temp = self.output.temp_path().to_path_buf();
        let movie = Movie::read(&temp)?;
        let format = self.spec.codec.input_format();
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
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}

/// An export in progress on macOS.
pub type Export = Pipeline<VideoToolboxSink>;

/// Starts an export: validates, prepares the temporary file next to the destination,
/// and opens the encoder and muxer.
pub fn export(spec: ExportSpec) -> Result<Export, Error> {
    let sink = VideoToolboxSink::new(&spec)?;
    Pipeline::new(spec, sink)
}
