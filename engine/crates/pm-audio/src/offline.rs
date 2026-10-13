//! Offline render for export (plan sections 16 and 17): the mix of a time
//! range as PCM, 24-bit integer or 32-bit float for ProRes and WAV, 16-bit
//! with TPDF dither where an output needs it, and a streaming WAV writer
//! (RF64 past 4 GiB: no length limit).
//!
//! Sample counts follow section 2: a range of ticks `[t0, t1)` covers
//! project samples `round(t0 × sr)` to `round(t1 × sr)` (ticks are exact, so
//! a range starting on a sample has `round(duration × sr)` samples).

use std::io::{Seek, SeekFrom, Write};

use pm_time::TICKS_PER_SECOND;

use crate::error::Result;
use crate::mixer::{Mixer, RenderOptions};

/// Integer sample format of an export track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    /// IEEE 754 binary32, little endian.
    F32,
    /// 24-bit signed integer, little endian, no dither (its quantization sits
    /// at −144 dBFS, below the f32 mix's own noise).
    S24,
    /// 16-bit signed integer with TPDF dither (±1 LSB triangular).
    S16,
}

impl PcmFormat {
    pub fn bytes(self) -> usize {
        match self {
            PcmFormat::F32 => 4,
            PcmFormat::S24 => 3,
            PcmFormat::S16 => 2,
        }
    }
    pub fn bits(self) -> u16 {
        (self.bytes() * 8) as u16
    }
}

/// Project sample nearest a tick time (ties to even).
pub fn sample_of_ticks(ticks: i64, rate: u32) -> i64 {
    let n = ticks as i128 * rate as i128;
    let d = TICKS_PER_SECOND as i128;
    let q = n.div_euclid(d);
    let r = n.rem_euclid(d);
    let up = match (2 * r).cmp(&d) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => q % 2 != 0,
        std::cmp::Ordering::Less => false,
    };
    (q + up as i128) as i64
}

/// Samples `[first, end)` of a tick range.
pub fn sample_range(t0: i64, t1: i64, rate: u32) -> (i64, i64) {
    let a = sample_of_ticks(t0, rate);
    (a, sample_of_ticks(t1, rate).max(a))
}

/// A counter-based generator: the dither of a sample depends only on its
/// index and channel, so chunked and whole renders write the same bytes.
#[inline]
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// TPDF dither in LSB for a sample: the difference of two uniform values,
/// triangular on (−1, 1).
#[inline]
pub fn tpdf(index: i64, channel: usize) -> f64 {
    let r =
        splitmix((index as u64).wrapping_mul(64).wrapping_add(channel as u64) ^ 0x5EED_D17E_u64);
    let u1 = (r >> 40) as f64 / (1u64 << 24) as f64;
    let u2 = ((r >> 16) & 0xFF_FFFF) as f64 / (1u64 << 24) as f64;
    u1 - u2
}

/// Encodes interleaved samples (`first` is the project index of frame 0,
/// for the dither) and returns how many samples clipped.
pub fn encode(
    format: PcmFormat,
    interleaved: &[f32],
    channels: usize,
    first: i64,
    out: &mut Vec<u8>,
) -> u64 {
    let mut clipped = 0u64;
    out.reserve(interleaved.len() * format.bytes());
    match format {
        PcmFormat::F32 => {
            for &v in interleaved {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        PcmFormat::S24 => {
            for &v in interleaved {
                let x = (v as f64 * 8_388_608.0).round();
                if !(-8_388_608.0..=8_388_607.0).contains(&x) {
                    clipped += 1;
                }
                let i = x.clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(&i.to_le_bytes()[..3]);
            }
        }
        PcmFormat::S16 => {
            for (k, &v) in interleaved.iter().enumerate() {
                let frame = first + (k / channels) as i64;
                let x = (v as f64 * 32_768.0 + tpdf(frame, k % channels)).round();
                if !(-32_768.0..=32_767.0).contains(&x) {
                    clipped += 1;
                }
                let i = x.clamp(-32_768.0, 32_767.0) as i16;
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
    }
    clipped
}

/// Interleaves planar channels.
pub fn interleave(planar: &[Vec<f32>], frames: usize, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(frames * planar.len());
    for i in 0..frames {
        for c in planar {
            out.push(c[i]);
        }
    }
}

/// What an offline render produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub frames: u64,
    pub clipped: u64,
    /// Voice errors (decode or evaluation), each voice once.
    pub errors: Vec<(String, String)>,
}

/// Renders project samples `[first, end)` in blocks of `block` frames and
/// hands interleaved f32 to `sink` with the index of each block's first
/// frame. Missing source blocks are decoded on this thread.
pub fn render(
    mixer: &mut Mixer,
    first: i64,
    end: i64,
    block: usize,
    mut sink: impl FnMut(i64, &[f32]) -> Result<()>,
) -> Result<Summary> {
    let channels = mixer.format().channels();
    let mut planar = vec![Vec::new(); channels];
    let mut inter = Vec::new();
    let options = RenderOptions {
        wait: true,
        ..RenderOptions::default()
    };
    let mut summary = Summary::default();
    let mut at = first;
    while at < end {
        let n = ((end - at) as usize).min(block.max(1));
        let report = mixer.render(at, n, &mut planar, &options)?;
        for e in report.errors {
            if !summary.errors.iter().any(|(id, _)| *id == e.0) {
                summary.errors.push(e);
            }
        }
        interleave(&planar, n, &mut inter);
        sink(at, &inter)?;
        summary.frames += n as u64;
        at += n as i64;
    }
    Ok(summary)
}

/// A streaming WAV writer. Writes RIFF while the data fits in 4 GiB and
/// turns the reserved `JUNK` chunk into RF64's `ds64` beyond that.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    format: PcmFormat,
    channels: u16,
    sample_rate: u32,
    data_bytes: u64,
    frames: u64,
    clipped: u64,
    buf: Vec<u8>,
}

const HEADER: u64 = 12 + 36 + 8 + 40 + 8;

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, format: PcmFormat, channels: u16, sample_rate: u32) -> Result<Self> {
        let header = Self::header(format, channels, sample_rate, 0, 0, false);
        out.write_all(&header)?;
        Ok(WavWriter {
            out,
            format,
            channels,
            sample_rate,
            data_bytes: 0,
            frames: 0,
            clipped: 0,
            buf: Vec::new(),
        })
    }

    fn header(
        format: PcmFormat,
        channels: u16,
        rate: u32,
        data: u64,
        frames: u64,
        rf64: bool,
    ) -> Vec<u8> {
        let mut h = Vec::with_capacity(HEADER as usize);
        let riff_size = HEADER - 8 + data;
        h.extend_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
        h.extend_from_slice(&(if rf64 { u32::MAX } else { riff_size as u32 }).to_le_bytes());
        h.extend_from_slice(b"WAVE");
        // JUNK (28 bytes of payload) reserved for ds64.
        if rf64 {
            h.extend_from_slice(b"ds64");
            h.extend_from_slice(&28u32.to_le_bytes());
            h.extend_from_slice(&riff_size.to_le_bytes());
            h.extend_from_slice(&data.to_le_bytes());
            h.extend_from_slice(&frames.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
        } else {
            h.extend_from_slice(b"JUNK");
            h.extend_from_slice(&28u32.to_le_bytes());
            h.extend_from_slice(&[0u8; 28]);
        }
        // fmt: WAVE_FORMAT_EXTENSIBLE.
        let bytes = format.bytes() as u16;
        h.extend_from_slice(b"fmt ");
        h.extend_from_slice(&40u32.to_le_bytes());
        h.extend_from_slice(&0xFFFEu16.to_le_bytes());
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&rate.to_le_bytes());
        h.extend_from_slice(&(rate * (bytes * channels) as u32).to_le_bytes());
        h.extend_from_slice(&(bytes * channels).to_le_bytes());
        h.extend_from_slice(&format.bits().to_le_bytes());
        h.extend_from_slice(&22u16.to_le_bytes());
        h.extend_from_slice(&format.bits().to_le_bytes());
        let mask: u32 = match channels {
            1 => 0x4,
            2 => 0x3,
            6 => 0x60F,
            8 => 0x63F,
            _ => 0,
        };
        h.extend_from_slice(&mask.to_le_bytes());
        // KSDATAFORMAT_SUBTYPE_PCM or _IEEE_FLOAT.
        let tag: u16 = if format == PcmFormat::F32 { 3 } else { 1 };
        h.extend_from_slice(&tag.to_le_bytes());
        h.extend_from_slice(&[
            0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
        ]);
        h.extend_from_slice(b"data");
        h.extend_from_slice(&(if rf64 { u32::MAX } else { data as u32 }).to_le_bytes());
        debug_assert_eq!(h.len() as u64, HEADER);
        h
    }

    /// Appends interleaved frames; `first` is the project index of the first
    /// frame (it seeds the 16-bit dither).
    pub fn write(&mut self, first: i64, interleaved: &[f32]) -> Result<()> {
        self.buf.clear();
        self.clipped += encode(
            self.format,
            interleaved,
            self.channels as usize,
            first,
            &mut self.buf,
        );
        self.out.write_all(&self.buf)?;
        self.data_bytes += self.buf.len() as u64;
        self.frames += (interleaved.len() / self.channels as usize) as u64;
        Ok(())
    }

    /// Writes the final sizes and returns the writer, frames, and clipped
    /// sample count.
    pub fn finish(mut self) -> Result<(W, u64, u64)> {
        if self.data_bytes % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        let rf64 = HEADER - 8 + self.data_bytes > u32::MAX as u64;
        let header = Self::header(
            self.format,
            self.channels,
            self.sample_rate,
            self.data_bytes,
            self.frames,
            rf64,
        );
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&header)?;
        self.out.seek(SeekFrom::End(0))?;
        self.out.flush()?;
        Ok((self.out, self.frames, self.clipped))
    }
}

/// Reads back a WAV written by [`WavWriter`] (tests and tools): format,
/// channels, rate, and interleaved f32 samples.
pub fn read_wav(bytes: &[u8]) -> Option<(PcmFormat, u16, u32, Vec<f32>)> {
    if bytes.len() < 12 || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let (mut fmt, mut channels, mut rate, mut bits, mut data) = (0u16, 0u16, 0u32, 0u16, None);
    let mut ds64_data = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = at + 8;
        match id {
            b"ds64" => {
                ds64_data =
                    Some(u64::from_le_bytes(bytes[body + 8..body + 16].try_into().ok()?) as usize)
            }
            b"fmt " => {
                fmt = u16::from_le_bytes(bytes[body..body + 2].try_into().ok()?);
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().ok()?);
                rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
                bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().ok()?);
                if fmt == 0xFFFE {
                    fmt = u16::from_le_bytes(bytes[body + 24..body + 26].try_into().ok()?);
                }
            }
            b"data" => {
                let size = if size == u32::MAX as usize {
                    ds64_data?
                } else {
                    size
                };
                data = Some(&bytes[body..(body + size).min(bytes.len())]);
                break;
            }
            _ => {}
        }
        at = body + size + (size & 1);
    }
    let data = data?;
    let format = match (fmt, bits) {
        (3, 32) => PcmFormat::F32,
        (1, 24) => PcmFormat::S24,
        (1, 16) => PcmFormat::S16,
        _ => return None,
    };
    let samples = match format {
        PcmFormat::F32 => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect(),
        PcmFormat::S24 => data
            .chunks_exact(3)
            .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        PcmFormat::S16 => data
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0)
            .collect(),
    };
    Some((format, channels, rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_ranges_round_to_samples() {
        let t = TICKS_PER_SECOND;
        assert_eq!(sample_range(0, 2 * t, 48_000), (0, 96_000));
        // 23.976 frame 1 starts at ceil(1001/24000 s) in ticks.
        let frame = ((t as i128 * 1001 + 23_999) / 24_000) as i64;
        assert_eq!(sample_of_ticks(frame, 48_000), 2002);
        assert_eq!(sample_of_ticks(-t, 44_100), -44_100);
    }

    #[test]
    fn dither_is_triangular_and_reproducible() {
        let n = 200_000;
        let mut sum = 0.0;
        let mut sq = 0.0;
        let mut max = 0f64;
        for i in 0..n {
            let d = tpdf(i, 1);
            sum += d;
            sq += d * d;
            max = max.max(d.abs());
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "{mean}");
        // Variance of the difference of two U(0,1): 1/6.
        assert!((var - 1.0 / 6.0).abs() < 0.005, "{var}");
        assert!(max < 1.0);
        assert_eq!(tpdf(12345, 0), tpdf(12345, 0));
        assert_ne!(tpdf(12345, 0), tpdf(12345, 1));
    }

    #[test]
    fn sixteen_bit_dither_decorrelates_quantization() {
        // A −100 dBFS sine (well under 1 LSB at 16 bits) survives as a sine
        // plus noise instead of truncating to zero.
        let amp = 10f64.powf(-100.0 / 20.0) * 4.0;
        let x: Vec<f32> = (0..48_000)
            .map(|i| (amp * (i as f64 * 0.05).sin()) as f32)
            .collect();
        let mut out = Vec::new();
        encode(PcmFormat::S16, &x, 1, 0, &mut out);
        let y: Vec<f64> = out
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f64)
            .collect();
        let corr: f64 = y.iter().zip(&x).map(|(a, b)| a * *b as f64).sum::<f64>();
        assert!(corr > 0.0);
        assert!(y.iter().any(|&v| v != 0.0));
    }

    #[test]
    fn wav_round_trip() {
        for format in [PcmFormat::F32, PcmFormat::S24, PcmFormat::S16] {
            let mut w =
                WavWriter::new(std::io::Cursor::new(Vec::new()), format, 2, 48_000).unwrap();
            let x: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
            w.write(0, &x).unwrap();
            let (cur, frames, clipped) = w.finish().unwrap();
            assert_eq!((frames, clipped), (1000, 0));
            let (f, ch, rate, y) = read_wav(&cur.into_inner()).unwrap();
            assert_eq!((f, ch, rate, y.len()), (format, 2, 48_000, 2000));
            let tol = match format {
                PcmFormat::F32 => 0.0,
                PcmFormat::S24 => 1.0 / 8_388_608.0,
                PcmFormat::S16 => 2.0 / 32_768.0,
            };
            assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() <= tol));
        }
    }
}
