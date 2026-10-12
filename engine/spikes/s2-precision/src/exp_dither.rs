//! Output dither (section 17): deterministic blue-noise RPDF of ±0.5 LSB seeded by
//! frame and pixel, applied after the matrix in the output domain.
//!
//! Measures, through the prototype's encoder-input passes:
//! - the bit-depth bar's dither clause (section 2): with dither on an 8-bit output, the
//!   16-column local mean of a 16-bit grey ramp is within 0.5 CV of ideal, on the bar's
//!   full-range ramp and on a shallow ramp where undithered banding is widest;
//! - the same on the opt-in 10-bit output;
//! - values already on the output grid are unchanged with dither on;
//! - determinism (two renders byte-identical) and per-frame seeding (frames differ);
//! - alpha is never dithered.

use serde::Serialize;

use crate::bluenoise;
use crate::cf::{Result, err};
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::pixbuf::{self, PixelBuffer};

#[derive(Serialize)]
pub struct OffsetStats {
    /// Offsets (rank + 0.5) / 4096 − 0.5 of the 64×64 tile.
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Strictly inside (−0.5, 0.5), so on-grid values cannot move.
    pub strictly_inside_half: bool,
}

#[derive(Serialize)]
pub struct RampRow {
    pub ramp: String,
    pub output: String,
    pub dither: bool,
    pub width: usize,
    pub rows: usize,
    /// Maximum over rows and non-overlapping 16-column windows of |mean(code) − mean(ideal)|.
    pub max_local_mean_err_16col: f64,
    /// The same on the column means over all rows (what a viewer sees on a smooth gradient).
    pub max_local_mean_err_16col_all_rows: f64,
    /// Maximum per-pixel |code − ideal|.
    pub max_pixel_err: f64,
    /// Longest run of equal codes along a row, in columns (banding width).
    pub longest_flat_run: usize,
    pub monotonic_column_means: bool,
    /// Luma window bar (8-bit with dither on): max_local_mean_err_16col ≤ 0.5.
    pub pass_bar_0_5: bool,
}

#[derive(Serialize)]
pub struct DitherReport {
    pub offsets: OffsetStats,
    pub ramps: Vec<RampRow>,
    /// Y′ codes 16..235 (8-bit) set exactly through y416, output to '420v' with dither on.
    pub on_grid_8bit_changed: usize,
    /// Y′ codes 64..940 (10-bit) set exactly through y416, output to 'x420' with dither on.
    pub on_grid_10bit_changed: usize,
    /// Two renders of the same frame with dither on: differing bytes ('420v').
    pub repeat_differing_bytes: usize,
    /// Frame 0 versus frame 1 with dither on: differing luma samples (per-frame seeding).
    pub frame_seed_differing_samples: usize,
    /// Alpha samples that differ between dither off and on ('y416' and 'l64r' outputs).
    pub alpha_changed_y416: usize,
    pub alpha_changed_l64r: usize,
}

fn offsets() -> OffsetStats {
    let n = (bluenoise::SIZE * bluenoise::SIZE) as f64;
    let v: Vec<f64> = bluenoise::tile().iter().map(|&r| (r as f64 + 0.5) / n - 0.5).collect();
    let min = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    OffsetStats { min, max, mean: v.iter().sum::<f64>() / n, strictly_inside_half: min > -0.5 && max < 0.5 }
}

/// An opaque grey ramp as 'l64r' (16-bit RGBA): column x has value lo + (hi − lo)·x/(w − 1),
/// rounded to 16 bits. Returns the buffer and the exact per-column E values.
fn grey_ramp(w: usize, h: usize, lo: f64, hi: f64) -> Result<(PixelBuffer, Vec<f64>)> {
    let pb = PixelBuffer::new(w, h, pixbuf::RGBA64)?;
    let vals: Vec<u16> =
        (0..w).map(|x| ((lo + (hi - lo) * x as f64 / (w - 1) as f64) * 65535.0).round() as u16).collect();
    let mut d = vec![0u16; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            d[o..o + 4].copy_from_slice(&[vals[x], vals[x], vals[x], 65535]);
        }
    }
    pb.write_u16(None, &d)?;
    pb.tag_709();
    Ok((pb, vals.iter().map(|&v| v as f64 / 65535.0).collect()))
}

/// Luma codes of an output buffer ('420v' as 8-bit codes, 'x420' as 10-bit codes).
fn luma_codes(pb: &PixelBuffer) -> Result<Vec<f64>> {
    match pb.format() {
        pixbuf::V420 => Ok(pb.read_u8(Some(0))?.0.iter().map(|&v| v as f64).collect()),
        pixbuf::X420 => Ok(pb.read_u16(Some(0))?.0.iter().map(|&v| (v >> 6) as f64).collect()),
        f => err(format!("luma_codes: unsupported '{}'", crate::cf::fourcc_str(f))),
    }
}

/// Window statistics of `codes` (w × h) against per-column ideal codes.
pub fn ramp_stats(codes: &[f64], ideal: &[f64], w: usize, h: usize) -> (f64, f64, f64, usize, bool) {
    let mut max_win = 0f64;
    let mut max_px = 0f64;
    let mut longest = 0usize;
    for y in 0..h {
        let row = &codes[y * w..(y + 1) * w];
        for x0 in (0..w - 15).step_by(16) {
            let m: f64 = row[x0..x0 + 16].iter().sum::<f64>() / 16.0;
            let i: f64 = ideal[x0..x0 + 16].iter().sum::<f64>() / 16.0;
            max_win = max_win.max((m - i).abs());
        }
        let mut run = 1;
        for x in 0..w {
            max_px = max_px.max((row[x] - ideal[x]).abs());
            if x > 0 {
                run = if row[x] == row[x - 1] { run + 1 } else { 1 };
            }
            longest = longest.max(run);
        }
    }
    let col: Vec<f64> = (0..w).map(|x| (0..h).map(|y| codes[y * w + x]).sum::<f64>() / h as f64).collect();
    let mut max_all = 0f64;
    for x0 in (0..w - 15).step_by(16) {
        let m: f64 = col[x0..x0 + 16].iter().sum::<f64>() / 16.0;
        let i: f64 = ideal[x0..x0 + 16].iter().sum::<f64>() / 16.0;
        max_all = max_all.max((m - i).abs());
    }
    // Monotonic over 16-column window means (the per-column means of a dithered ramp
    // are noisy by design).
    let wins: Vec<f64> = (0..w / 16).map(|i| col[i * 16..i * 16 + 16].iter().sum::<f64>() / 16.0).collect();
    let mono = wins.windows(2).all(|p| p[1] >= p[0] - 1e-9);
    (max_win, max_all, max_px, longest, mono)
}

fn ramp_row(gpu: &Gpu, name: &str, lo: f64, hi: f64, format: u32, dither: bool) -> Result<RampRow> {
    let (w, h) = (1920usize, 64usize);
    let (src, e) = grey_ramp(w, h, lo, hi)?;
    let p = PipeParams { dither, ..PipeParams::standard(Precision::F32) };
    let out = gpu.engine_convert(&src, format, &p)?;
    let codes = luma_codes(&out)?;
    let (scale, off) = if format == pixbuf::V420 { (219.0, 16.0) } else { (876.0, 64.0) };
    let ideal: Vec<f64> = e.iter().map(|v| v * scale + off).collect();
    let (max_win, max_all, max_px, longest, mono) = ramp_stats(&codes, &ideal, w, h);
    Ok(RampRow {
        ramp: name.into(),
        output: crate::cf::fourcc_str(format),
        dither,
        width: w,
        rows: h,
        max_local_mean_err_16col: max_win,
        max_local_mean_err_16col_all_rows: max_all,
        max_pixel_err: max_px,
        longest_flat_run: longest,
        monotonic_column_means: mono,
        pass_bar_0_5: max_win <= 0.5,
    })
}

/// A y416 buffer whose pixels are opaque neutral greys at exact codes of `bits` depth.
fn on_grid(bits: u32) -> Result<(PixelBuffer, Vec<u16>)> {
    let (lo, hi) = if bits == 8 { (16u16, 235u16) } else { (64, 940) };
    let codes: Vec<u16> = (lo..=hi).collect();
    let (w, h) = (codes.len().next_multiple_of(2), 64usize);
    let pb = PixelBuffer::new(w, h, pixbuf::Y416)?;
    let shift = 16 - bits;
    let mut d = vec![0u16; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let c = codes[x.min(codes.len() - 1)];
            d[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[65535, c << shift, 32768, 32768]);
        }
    }
    pb.write_u16(None, &d)?;
    pb.tag_709();
    let expect: Vec<u16> = (0..w).map(|x| codes[x.min(codes.len() - 1)]).collect();
    Ok((pb, expect))
}

fn on_grid_changed(gpu: &Gpu, bits: u32) -> Result<usize> {
    let (src, expect) = on_grid(bits)?;
    let format = if bits == 8 { pixbuf::V420 } else { pixbuf::X420 };
    let mut changed = 0;
    for frame in 0..4 {
        let p = PipeParams { dither: true, frame, ..PipeParams::standard(Precision::F32) };
        let out = gpu.engine_convert(&src, format, &p)?;
        let codes = luma_codes(&out)?;
        let w = src.width();
        changed += codes.iter().enumerate().filter(|(i, c)| **c != expect[i % w] as f64).count();
        // Neutral chroma stays exactly neutral.
        let (c, _, _) = if bits == 8 {
            let (v, a, b) = out.read_u8(Some(1))?;
            (v.iter().map(|&x| x as u16).collect::<Vec<_>>(), a, b)
        } else {
            let (v, a, b) = out.read_u16(Some(1))?;
            (v.iter().map(|&x| x >> 6).collect(), a, b)
        };
        let neutral = if bits == 8 { 128 } else { 512 };
        changed += c.iter().filter(|&&v| v != neutral).count();
    }
    Ok(changed)
}

fn alpha_changed(gpu: &Gpu, format: u32) -> Result<usize> {
    let (w, h) = (1920usize, 32usize);
    let src = PixelBuffer::new(w, h, pixbuf::RGBA64)?;
    let mut d = vec![0u16; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let a = ((x * 65535 + (w - 1) / 2) / (w - 1)) as u16;
            let o = (y * w + x) * 4;
            d[o..o + 4].copy_from_slice(&[a / 2, a / 3, a / 4, a]);
        }
    }
    src.write_u16(None, &d)?;
    src.tag_709();
    let off = gpu.engine_convert(&src, format, &PipeParams::standard(Precision::F32))?;
    let on = gpu.engine_convert(
        &src,
        format,
        &PipeParams { dither: true, frame: 3, ..PipeParams::standard(Precision::F32) },
    )?;
    let (a, _, _) = off.read_u16(None)?;
    let (b, _, _) = on.read_u16(None)?;
    let ai = if format == pixbuf::Y416 { 0 } else { 3 };
    Ok(a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(p, q)| p[ai] != q[ai]).count())
}

pub fn run(gpu: &Gpu) -> Result<DitherReport> {
    let mut ramps = vec![];
    for (name, lo, hi) in [("full 0 → 1 (bar)", 0.0, 1.0), ("shallow 0.40 → 0.42", 0.40, 0.42)] {
        for format in [pixbuf::V420, pixbuf::X420] {
            for dither in [false, true] {
                ramps.push(ramp_row(gpu, name, lo, hi, format, dither)?);
            }
        }
    }
    let (src, _) = grey_ramp(1920, 64, 0.40, 0.42)?;
    let p0 = PipeParams { dither: true, frame: 0, ..PipeParams::standard(Precision::F32) };
    let a = gpu.engine_convert(&src, pixbuf::V420, &p0)?;
    let b = gpu.engine_convert(&src, pixbuf::V420, &p0)?;
    let c = gpu.engine_convert(&src, pixbuf::V420, &PipeParams { frame: 1, ..p0 })?;
    let mut repeat = 0;
    for plane in 0..2 {
        let (x, _, _) = a.read_u8(Some(plane))?;
        let (y, _, _) = b.read_u8(Some(plane))?;
        repeat += x.iter().zip(&y).filter(|(p, q)| p != q).count();
    }
    let (x, _, _) = a.read_u8(Some(0))?;
    let (z, _, _) = c.read_u8(Some(0))?;
    Ok(DitherReport {
        offsets: offsets(),
        ramps,
        on_grid_8bit_changed: on_grid_changed(gpu, 8)?,
        on_grid_10bit_changed: on_grid_changed(gpu, 10)?,
        repeat_differing_bytes: repeat,
        frame_seed_differing_samples: x.iter().zip(&z).filter(|(p, q)| p != q).count(),
        alpha_changed_y416: alpha_changed(gpu, pixbuf::Y416)?,
        alpha_changed_l64r: alpha_changed(gpu, pixbuf::RGBA64)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_are_strictly_inside_half_and_centered() {
        let o = offsets();
        assert!(o.strictly_inside_half);
        assert!(o.mean.abs() < 1e-12, "{}", o.mean);
    }

    #[test]
    fn ramp_stats_measure_banding() {
        // A hard-quantized shallow ramp: windows straddling a step are off by up to 0.5.
        let w = 64;
        let ideal: Vec<f64> = (0..w).map(|x| 100.0 + x as f64 / 64.0).collect();
        let codes: Vec<f64> = ideal.iter().map(|v| v.round()).collect();
        let (win, all, px, run, mono) = ramp_stats(&codes, &ideal, w, 1);
        assert!(win > 0.2 && win <= 0.5, "{win}");
        assert_eq!(win, all);
        assert!(px <= 0.5);
        assert_eq!(run, 32);
        assert!(mono);
    }
}
