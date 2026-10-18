//! PNG sequences and stills, 8 or 16 bits per channel (plan section 17,
//! "Formats at cutover"; section 9, "Output tags").
//!
//! Frames arrive as full-range RGBA encoder input (straight alpha, or opaque over
//! black) and are written losslessly: RGB when the export is opaque, RGBA with
//! alpha. Tags: sRGB images carry an `sRGB` chunk and `cICP` 1-13-0 (full range);
//! Rec.709 images carry `cICP` 1-1-0. A sequence is a folder of
//! `<name>_00000.png` files written into a temporary folder that becomes the
//! destination when every frame is written and verified ([`crate::atomic`]).
//!
//! Verification reads every file's header (size, depth, color type, tags) and
//! decodes the first, middle, and last frames, which must equal the encoder input
//! exactly.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::Delivered;
use crate::Error;
use crate::atomic::AtomicOutput;
use crate::convert::{EncoderInput, PlaneData};
use crate::pipeline::Sink;
use crate::settings::{AlphaOutput, ColorTags, Container, ExportSpec, InputFormat, Overwrite};
use crate::verify::{self, PixelCheck, VerifyReport};

/// The file name of frame `index` of a sequence named `stem` with `frames` frames:
/// five digits (today's exporter), more when the sequence needs them.
pub fn frame_name(stem: &str, index: u64, frames: u64) -> String {
    let digits = frames.saturating_sub(1).to_string().len().max(5);
    format!("{stem}_{index:0digits$}.png")
}

/// Writes PNG frames and verifies them.
pub struct PngSink {
    spec: ExportSpec,
    output: AtomicOutput,
    stem: String,
    sampled: Vec<u64>,
    /// The encoder input of the sampled frames, compared exactly after writing.
    kept: Vec<(u64, EncoderInput)>,
    written: u64,
}

impl PngSink {
    pub fn new(spec: &ExportSpec) -> Result<Self, Error> {
        spec.validate()?;
        let output = match spec.container {
            Container::PngSequence => {
                AtomicOutput::folder(&spec.destination, spec.overwrite, spec.mode)?
            }
            Container::PngStill => match spec.overwrite {
                Overwrite::InPlace => AtomicOutput::in_place(&spec.destination, "png", spec.mode)?,
                o => AtomicOutput::new(&spec.destination, o, spec.mode)?,
            },
            _ => {
                return Err(Error::InvalidSpec(format!(
                    "{:?} is not a PNG output",
                    spec.container
                )));
            }
        };
        let stem = spec
            .destination
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "frame".into());
        Ok(Self {
            spec: spec.clone(),
            output,
            stem,
            sampled: verify::sampled_frames(spec.frames),
            kept: vec![],
            written: 0,
        })
    }

    /// The stem frame files are named after (`<stem>_00000.png`).
    pub fn set_stem(&mut self, stem: &str) {
        self.stem = stem.to_owned();
    }

    fn frame_path(&self, index: u64) -> PathBuf {
        match self.spec.container {
            Container::PngSequence => {
                self.output
                    .temp_path()
                    .join(frame_name(&self.stem, index, self.spec.frames))
            }
            _ => self.output.temp_path().to_path_buf(),
        }
    }
}

/// Encodes one frame as PNG bytes.
pub fn encode(input: &EncoderInput, alpha: bool, tags: ColorTags) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    write_png(&mut out, input, alpha, tags)?;
    Ok(out)
}

enum PngData<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
}

fn write_png<W: Write>(
    out: W,
    input: &EncoderInput,
    alpha: bool,
    tags: ColorTags,
) -> Result<(), Error> {
    let png_err = |e: png::EncodingError| Error::Encode(format!("PNG: {e}"));
    let (wide, data) = match (&input.format, &input.planes[0].data) {
        (InputFormat::Rgba8, PlaneData::U8(v)) => (false, PngData::U8(v)),
        (InputFormat::Rgba16, PlaneData::U16(v)) => (true, PngData::U16(v)),
        _ => {
            return Err(Error::InvalidFrame(
                "PNG frames are RGBA encoder input".into(),
            ));
        }
    };
    let mut enc = png::Encoder::new(out, input.width, input.height);
    enc.set_color(if alpha {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    });
    enc.set_depth(if wide {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    });
    enc.set_compression(png::Compression::Fast);
    if tags == ColorTags::SrgbImage {
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    }
    let mut writer = enc.write_header().map_err(png_err)?;
    let (p, t, m) = tags.cicp();
    // cICP: primaries, transfer, matrix (0: RGB), full range.
    writer
        .write_chunk(
            png::chunk::ChunkType(*b"cICP"),
            &[p as u8, t as u8, m as u8, 1],
        )
        .map_err(png_err)?;
    let channels = if alpha { 4 } else { 3 };
    let pixels = input.width as usize * input.height as usize;
    let bytes: Vec<u8> = match data {
        PngData::U8(v) if alpha => v.to_vec(),
        PngData::U8(v) => {
            let mut b = Vec::with_capacity(pixels * 3);
            for px in v.chunks_exact(4) {
                b.extend_from_slice(&px[..3]);
            }
            b
        }
        PngData::U16(v) => {
            let mut b = Vec::with_capacity(pixels * channels * 2);
            for px in v.chunks_exact(4) {
                for s in &px[..channels] {
                    b.extend_from_slice(&s.to_be_bytes());
                }
            }
            b
        }
    };
    writer.write_image_data(&bytes).map_err(png_err)?;
    writer.finish().map_err(png_err)
}

/// What a written PNG holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub alpha: bool,
    pub srgb: bool,
    /// (primaries, transfer, matrix, full range).
    pub cicp: Option<(u8, u8, u8, u8)>,
}

/// Reads a PNG's header and, with `decode`, its pixels as RGBA encoder input
/// (alpha at full for RGB files).
pub fn read(path: &Path, decode: bool) -> Result<(PngInfo, Option<EncoderInput>), Error> {
    let io = |e: std::io::Error| Error::Verify(format!("{}: {e}", path.display()));
    let file = File::open(path).map_err(io)?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let bad = |e: png::DecodingError| Error::Verify(format!("{}: {e}", path.display()));
    let mut reader = decoder.read_info().map_err(bad)?;
    let info = reader.info();
    let alpha = matches!(info.color_type, png::ColorType::Rgba);
    let cicp = info.coding_independent_code_points.map(|c| {
        (
            c.color_primaries,
            c.transfer_function,
            c.matrix_coefficients,
            u8::from(c.is_video_full_range_image),
        )
    });
    let meta = PngInfo {
        width: info.width,
        height: info.height,
        bit_depth: info.bit_depth as u8,
        alpha,
        srgb: info.srgb.is_some(),
        cicp,
    };
    if !decode {
        return Ok((meta, None));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| Error::Verify("PNG too large".into()))?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf).map_err(bad)?;
    let bytes = &buf[..frame.buffer_size()];
    let channels = if alpha { 4 } else { 3 };
    let (w, h) = (meta.width as usize, meta.height as usize);
    let (format, data) = if meta.bit_depth == 16 {
        let mut v = Vec::with_capacity(w * h * 4);
        for px in bytes.chunks_exact(2 * channels) {
            for k in 0..channels {
                v.push(u16::from_be_bytes([px[2 * k], px[2 * k + 1]]));
            }
            if !alpha {
                v.push(65535);
            }
        }
        (InputFormat::Rgba16, PlaneData::U16(v))
    } else {
        let mut v = Vec::with_capacity(w * h * 4);
        for px in bytes.chunks_exact(channels) {
            v.extend_from_slice(px);
            if !alpha {
                v.push(255);
            }
        }
        (InputFormat::Rgba8, PlaneData::U8(v))
    };
    Ok((
        meta.clone(),
        Some(EncoderInput {
            format,
            width: meta.width,
            height: meta.height,
            planes: vec![crate::convert::Plane {
                data,
                row_samples: w * 4,
                rows: h,
            }],
        }),
    ))
}

/// Sets every alpha sample to full (what an RGB file decodes to).
fn opaque(input: &mut EncoderInput) {
    match &mut input.planes[0].data {
        PlaneData::U8(v) => v.chunks_exact_mut(4).for_each(|p| p[3] = 255),
        PlaneData::U16(v) => v.chunks_exact_mut(4).for_each(|p| p[3] = 65535),
    }
}

impl Sink for PngSink {
    type Output = Delivered;

    fn video(&mut self, index: u64, input: &EncoderInput) -> Result<(), Error> {
        let path = self.frame_path(index);
        let file =
            File::create(&path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let mut w = BufWriter::with_capacity(1 << 20, file);
        write_png(
            &mut w,
            input,
            self.spec.alpha != AlphaOutput::None,
            self.spec.tags,
        )?;
        w.flush()
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        if self.sampled.contains(&index) {
            self.kept.push((index, input.clone()));
        }
        self.written += 1;
        Ok(())
    }

    fn audio(&mut self, _: &[f32]) -> Result<(), Error> {
        Err(Error::InvalidFrame("PNG outputs carry no audio".into()))
    }

    fn finish(self) -> Result<Delivered, Error> {
        let mut problems = vec![];
        let alpha = self.spec.alpha != AlphaOutput::None;
        let depth = self.spec.codec.depth() as u8;
        let (p, t, m) = self.spec.tags.cicp();
        let want_cicp = Some((p as u8, t as u8, m as u8, 1));
        let mut pixels: Vec<PixelCheck> = vec![];
        let mut bytes = 0u64;
        if self.written != self.spec.frames {
            problems.push(format!(
                "{} of {} frames were written",
                self.written, self.spec.frames
            ));
        }
        for index in 0..self.written {
            let path = self.frame_path(index);
            bytes += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let keep = self.kept.iter().find(|(i, _)| *i == index);
            let (info, decoded) = read(&path, keep.is_some())?;
            if (info.width, info.height) != (self.spec.width, self.spec.height) {
                problems.push(format!(
                    "frame {index} is {}×{}, expected {}×{}",
                    info.width, info.height, self.spec.width, self.spec.height
                ));
            }
            if info.bit_depth != depth || info.alpha != alpha {
                problems.push(format!(
                    "frame {index} is {}-bit {}, expected {depth}-bit {}",
                    info.bit_depth,
                    if info.alpha { "RGBA" } else { "RGB" },
                    if alpha { "RGBA" } else { "RGB" }
                ));
            }
            if info.cicp != want_cicp || info.srgb != (self.spec.tags == ColorTags::SrgbImage) {
                problems.push(format!(
                    "frame {index} tags cICP {:?}, sRGB {}; expected {want_cicp:?}",
                    info.cicp, info.srgb
                ));
            }
            if let (Some((_, want)), Some(got)) = (keep, decoded) {
                // Lossless: every sample equals the encoder input.
                let mut want = want.clone();
                if !alpha {
                    opaque(&mut want);
                }
                if got != want {
                    problems.push(format!("frame {index} does not decode to what was written"));
                }
                pixels.extend(
                    verify::block_means(&got, index)
                        .into_iter()
                        .map(|b| PixelCheck {
                            frame: index,
                            x: b.x,
                            y: b.y,
                            expected: b.mean,
                            decoded: b.mean,
                            error_cv: 0.0,
                        }),
                );
            }
        }
        if !problems.is_empty() {
            return Err(Error::Verify(problems.join("; ")));
        }
        let report = VerifyReport {
            codec: "png".into(),
            profile: self.spec.codec.name(),
            pixel_format: if alpha { "RGBA" } else { "RGB" }.into(),
            depth: u32::from(depth),
            cicp: (p, t, m),
            full_range: true,
            alpha,
            frames: self.written,
            movie_timescale: 0,
            media_timescale: 0,
            duration: 0,
            audio: None,
            timecode: None,
            pixels,
            hardware_encoder: None,
            bytes,
        };
        let path = self.output.commit()?;
        Ok(Delivered { path, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_names_follow_todays_exporter_and_grow_for_long_sequences() {
        assert_eq!(frame_name("Title", 0, 10), "Title_00000.png");
        assert_eq!(frame_name("Title", 42, 99_999), "Title_00042.png");
        assert_eq!(frame_name("Title", 42, 100_001), "Title_000042.png");
    }
}
