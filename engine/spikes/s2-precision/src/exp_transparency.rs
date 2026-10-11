//! Pipeline transparency (section 2): ProRes 4444 XQ → y416 → engine → y416 sink, and
//! ProRes 422 HQ → x422 → engine → x422 sink, against the VideoToolbox reference decode.
//! Also runs the same path at f16 and with linear blending on (f16 vs f32, 12-bit source).

use serde::Serialize;

use crate::cf::Result;
use crate::color::DiffStats;
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material::Material;
use crate::pixbuf::{self, PixelBuffer};
use crate::vt;

#[derive(Serialize, Clone)]
pub struct ChannelStats {
    pub channel: String,
    /// Differences in the sink's native 16-bit units.
    pub at16: DiffStats,
    /// Differences after rounding both to 12-bit (y416) or 10-bit (x422) codes.
    pub at_codec_depth: DiffStats,
}

#[derive(Serialize)]
pub struct Case {
    pub source: String,
    pub params: PipeParams,
    pub frames: usize,
    pub channels: Vec<ChannelStats>,
    /// Pixels excluded from color comparison because α = 0 (color is not recoverable
    /// from premultiplied working values).
    pub alpha_zero_pixels: u64,
    pub exact_at_16: bool,
    pub exact_at_codec_depth: bool,
    pub reference_outside_codable_range: u64,
    /// Mismatches against the reference clipped to the codable range (section 9 clip rule).
    pub mismatches_vs_clipped_reference: u64,
}

fn configs() -> Vec<(&'static str, PipeParams)> {
    vec![
        ("f32 standard, clip", PipeParams { precision: Precision::F32, linear: false, premultiply: true, clip: true }),
        ("f32 standard, no clip", PipeParams { precision: Precision::F32, linear: false, premultiply: true, clip: false }),
        ("f16 standard, no clip", PipeParams { precision: Precision::F16, linear: false, premultiply: true, clip: false }),
        ("f32 linear, no clip", PipeParams { precision: Precision::F32, linear: true, premultiply: true, clip: false }),
        ("f16 linear, no clip", PipeParams { precision: Precision::F16, linear: true, premultiply: true, clip: false }),
    ]
}

pub fn compare_y416(reference: &PixelBuffer, out: &PixelBuffer, acc: &mut [ChannelStats; 4], alpha_zero: &mut u64, outside: &mut u64, vs_clipped: &mut u64) -> Result<()> {
    let (a, _, _) = reference.read_u16(None)?;
    let (b, _, _) = out.read_u16(None)?;
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        // Alpha always compared.
        acc[3].at16.add(pa[0] as i64, pb[0] as i64);
        acc[3].at_codec_depth.add(((pa[0] as i64) + 8) >> 4, ((pb[0] as i64) + 8) >> 4);
        if pa[0] == 0 {
            *alpha_zero += 1;
            continue;
        }
        for c in 0..3 {
            let (x, y) = (pa[c + 1] as i64, pb[c + 1] as i64);
            if !(256..=65264).contains(&x) {
                *outside += 1;
            }
            if x.clamp(256, 65264) != y {
                *vs_clipped += 1;
            }
            acc[c].at16.add(x, y);
            acc[c].at_codec_depth.add((x + 8) >> 4, (y + 8) >> 4);
        }
    }
    Ok(())
}

pub fn compare_x422(reference: &PixelBuffer, out: &PixelBuffer, acc: &mut [ChannelStats; 4], outside: &mut u64, vs_clipped: &mut u64) -> Result<()> {
    for plane in 0..2 {
        let (a, _, _) = reference.read_u16(Some(plane))?;
        let (b, _, _) = out.read_u16(Some(plane))?;
        for (i, (&x, &y)) in a.iter().zip(&b).enumerate() {
            let ch = if plane == 0 { 0 } else { 1 + (i % 2) };
            let (x, y) = (x as i64, y as i64);
            if !(4 * 64..=1019 * 64).contains(&x) {
                *outside += 1;
            }
            if x.clamp(4 * 64, 1019 * 64) != y {
                *vs_clipped += 1;
            }
            acc[ch].at16.add(x, y);
            acc[ch].at_codec_depth.add((x + 32) >> 6, (y + 32) >> 6);
        }
    }
    Ok(())
}

fn new_acc() -> [ChannelStats; 4] {
    ["Y", "Cb", "Cr", "A"].map(|c| ChannelStats { channel: c.into(), at16: DiffStats::default(), at_codec_depth: DiffStats::default() })
}

pub fn run(gpu: &Gpu, m: &Material) -> Result<Vec<Case>> {
    let mut cases = vec![];
    for (name, path, fmt) in [
        ("ProRes 4444 XQ (opaque) → y416", &m.prores4444xq, pixbuf::Y416),
        ("ProRes 4444 XQ (alpha ramp) → y416", &m.prores4444xq_alpha, pixbuf::Y416),
        ("ProRes 422 HQ → x422", &m.prores422hq, pixbuf::X422),
    ] {
        let dec = vt::decode_file(path, fmt)?;
        for (cname, p) in configs() {
            let mut acc = new_acc();
            let mut az = 0;
            let mut outside = 0;
            let mut vs_clipped = 0;
            for f in &dec.frames {
                let out = gpu.engine_copy(&f.buffer, &p)?;
                if fmt == pixbuf::Y416 {
                    compare_y416(&f.buffer, &out, &mut acc, &mut az, &mut outside, &mut vs_clipped)?;
                } else {
                    compare_x422(&f.buffer, &out, &mut acc, &mut outside, &mut vs_clipped)?;
                }
            }
            let channels: Vec<ChannelStats> =
                acc.into_iter().filter(|c| fmt == pixbuf::Y416 || c.channel != "A").collect();
            let exact16 = channels.iter().all(|c| c.at16.mismatches == 0);
            let exactd = channels.iter().all(|c| c.at_codec_depth.mismatches == 0);
            cases.push(Case {
                source: format!("{name} [{cname}]"),
                params: p,
                frames: dec.frames.len(),
                channels,
                alpha_zero_pixels: az,
                exact_at_16: exact16,
                exact_at_codec_depth: exactd,
                reference_outside_codable_range: outside,
                mismatches_vs_clipped_reference: vs_clipped,
            });
        }
    }
    Ok(cases)
}
