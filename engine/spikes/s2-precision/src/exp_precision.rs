//! f16 versus f32 working storage on exhaustive synthetic 10-, 12-, and 16-bit sources.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::cf::Result;
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material::Rng;
use crate::pixbuf::{self, PixelBuffer};

#[derive(Serialize)]
pub struct Row {
    pub source: String,
    pub precision: Precision,
    pub linear: bool,
    pub samples: u64,
    /// Maximum absolute error in source code values.
    pub max_error: i64,
    pub mismatch_fraction: f64,
    /// Distinct output codes for the distinct input codes of a ramp (when applicable).
    pub levels_in: Option<usize>,
    pub levels_out: Option<usize>,
}

fn params(p: Precision, linear: bool) -> PipeParams {
    PipeParams { precision: p, linear, premultiply: true, clip: false }
}

const MODES: [(Precision, bool); 4] =
    [(Precision::F32, false), (Precision::F16, false), (Precision::F32, true), (Precision::F16, true)];

/// y416 buffer from (A, Y, Cb, Cr) 16-bit samples, `w` pixels wide.
fn y416_from(px: &[[u16; 4]], w: usize) -> Result<PixelBuffer> {
    let h = px.len().div_ceil(w);
    let pb = PixelBuffer::new(w, h, pixbuf::Y416)?;
    let mut d = vec![0u16; w * h * 4];
    for (i, p) in px.iter().enumerate() {
        d[i * 4..i * 4 + 4].copy_from_slice(p);
    }
    // Pad with mid grey.
    for i in px.len()..w * h {
        d[i * 4..i * 4 + 4].copy_from_slice(&[65535, 32768, 32768, 32768]);
    }
    pb.write_u16(None, &d)?;
    Ok(pb)
}

/// Compare channel `ch` of the first `n` pixels at a given code shift (4 = 12-bit, 0 = 16-bit).
fn measure(
    name: &str,
    p: Precision,
    linear: bool,
    src: &[u16],
    out: &[u16],
    n: usize,
    comps: usize,
    chans: &[usize],
    shift: u32,
    ramp_channel: Option<usize>,
) -> Row {
    let mut max = 0i64;
    let mut mis = 0u64;
    let mut samples = 0u64;
    let mut lin = BTreeSet::new();
    let mut lout = BTreeSet::new();
    let half = if shift == 0 { 0 } else { 1i64 << (shift - 1) };
    for i in 0..n {
        for &c in chans {
            let a = src[i * comps + c] as i64;
            let b = out[i * comps + c] as i64;
            let (qa, qb) = ((a + half) >> shift, (b + half) >> shift);
            samples += 1;
            if qa != qb {
                mis += 1;
                max = max.max((qa - qb).abs());
            }
            if Some(c) == ramp_channel {
                lin.insert(qa);
                lout.insert(qb);
            }
        }
    }
    Row {
        source: name.into(),
        precision: p,
        linear,
        samples,
        max_error: max,
        mismatch_fraction: mis as f64 / samples as f64,
        levels_in: ramp_channel.map(|_| lin.len()),
        levels_out: ramp_channel.map(|_| lout.len()),
    }
}

pub fn run(gpu: &Gpu) -> Result<Vec<Row>> {
    let mut rows = vec![];

    // 12-bit luma ramp: every 12-bit code (as code << 4), neutral chroma, opaque.
    let ramp12: Vec<[u16; 4]> = (0..4096u32).map(|c| [65535, (c << 4) as u16, 32768, 32768]).collect();
    // 12-bit random triplets over the full 12-bit range.
    let mut rng = Rng::new(7);
    let rand12: Vec<[u16; 4]> = (0..1 << 20)
        .map(|_| [65535, (rng.range(0, 4095) << 4) as u16, (rng.range(0, 4095) << 4) as u16, (rng.range(0, 4095) << 4) as u16])
        .collect();
    // 16-bit luma ramp: every 16-bit value.
    let ramp16: Vec<[u16; 4]> = (0..65536u32).map(|c| [65535, c as u16, 32768, 32768]).collect();
    // 16-bit alpha ramp with a fixed 12-bit color (premultiplication precision).
    let alpha16: Vec<[u16; 4]> = (0..65536u32).map(|a| [a as u16, 0xB000, 0x6000, 0x9000]).collect();

    for (name, px, w, shift, chans, ramp) in [
        ("y416 12-bit luma ramp (4096 codes)", &ramp12, 512usize, 4u32, vec![1usize], Some(1usize)),
        ("y416 12-bit random Y′CbCr (1M pixels)", &rand12, 1024, 4, vec![1, 2, 3], None),
        ("y416 16-bit luma ramp (65536 values)", &ramp16, 256, 0, vec![1], Some(1)),
        ("y416 16-bit alpha ramp, fixed color: alpha", &alpha16, 256, 0, vec![0], Some(0)),
    ] {
        let src = y416_from(px, w)?;
        let (sd, _, _) = src.read_u16(None)?;
        for (p, linear) in MODES {
            let out = gpu.engine_copy(&src, &params(p, linear))?;
            let (od, _, _) = out.read_u16(None)?;
            rows.push(measure(name, p, linear, &sd, &od, px.len(), 4, &chans, shift, ramp));
        }
    }

    // 16-bit straight RGBA (a 16-bit PNG or Blender surface): every value on R=G=B.
    let pb = PixelBuffer::new(256, 256, pixbuf::RGBA64)?;
    let d: Vec<u16> = (0..65536u32).flat_map(|v| [v as u16, v as u16, v as u16, 65535]).collect();
    pb.write_u16(None, &d)?;
    for (p, linear) in MODES {
        let out = gpu.engine_copy(&pb, &params(p, linear))?;
        let (od, _, _) = out.read_u16(None)?;
        rows.push(measure("RGBA16 grey ramp (65536 values), at 16-bit", p, linear, &d, &od, 65536, 4, &[0], 0, Some(0)));
        rows.push(measure("RGBA16 grey ramp (65536 values), at 12-bit", p, linear, &d, &od, 65536, 4, &[0], 4, Some(0)));
        rows.push(measure("RGBA16 grey ramp (65536 values), at 10-bit", p, linear, &d, &od, 65536, 4, &[0], 6, Some(0)));
        rows.push(measure("RGBA16 grey ramp (65536 values), at 8-bit", p, linear, &d, &od, 65536, 4, &[0], 8, Some(0)));
    }
    // 16-bit straight RGBA with an alpha ramp and a fixed color: premultiply round trip.
    let d: Vec<u16> = (0..65536u32).flat_map(|a| [0xC123, 0x4567, 0x89AB, a as u16]).collect();
    pb.write_u16(None, &d)?;
    for (p, linear) in MODES {
        let out = gpu.engine_copy(&pb, &params(p, linear))?;
        let (od, _, _) = out.read_u16(None)?;
        // Color compared only where α ≥ 1/4096 (the half-float unpremultiply guard).
        let start = 16;
        let n = 65536 - start;
        rows.push(measure(
            "RGBA16 fixed color over alpha ramp (α ≥ 16/65535): color at 16-bit",
            p,
            linear,
            &d[start * 4..],
            &od[start * 4..],
            n,
            4,
            &[0, 1, 2],
            0,
            None,
        ));
        rows.push(measure(
            "RGBA16 fixed color over alpha ramp (α ≥ 16/65535): color at 10-bit",
            p,
            linear,
            &d[start * 4..],
            &od[start * 4..],
            n,
            4,
            &[0, 1, 2],
            6,
            None,
        ));
    }

    // 10-bit x422 luma ramp: every 10-bit code, neutral chroma.
    let pb = PixelBuffer::new(1024, 2, pixbuf::X422)?;
    let y: Vec<u16> = (0..2048u32).map(|i| ((i % 1024) << 6) as u16).collect();
    let c: Vec<u16> = vec![512 << 6; 2048];
    pb.write_u16(Some(0), &y)?;
    pb.write_u16(Some(1), &c)?;
    for (p, linear) in MODES {
        let out = gpu.engine_copy(&pb, &params(p, linear))?;
        let (oy, _, _) = out.read_u16(Some(0))?;
        rows.push(measure("x422 10-bit luma ramp (1024 codes)", p, linear, &y, &oy, 2048, 1, &[0], 6, Some(0)));
    }
    Ok(rows)
}
