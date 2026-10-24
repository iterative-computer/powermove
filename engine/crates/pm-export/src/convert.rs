//! Encoder input on the CPU (section 17, "Encoder input"; section 9).
//!
//! Converts premultiplied RGBA working frames (Rec.709 primaries; E-encoded in
//! Standard, linear with linear blending on) into what each encoder takes:
//! `y416` for ProRes 4444, `v216` for ProRes 422 (amendment 6), `420v` for H.264
//! and HEVC Main, `x420` for HEVC Main10, `v0a8` for HEVC with alpha (S2),
//! `yuv420p` and `yuva420p` for libvpx, and full-range RGBA for PNG. This is the
//! reference the GPU encoder-input pass is checked against and the fallback when
//! no GPU pass runs. It follows S2's prototype pass
//! (`engine/spikes/s2-precision/src/pipeline.wgsl`) exactly, in f64:
//!
//! * alpha: opaque outputs take premultiplied values (compositing over black);
//!   straight outputs unpremultiply, guarded below α = 1/4096; premultiplied outputs
//!   premultiply in the encoded domain;
//! * Rec.709 matrix (Kr 0.2126, Kb 0.0722);
//! * normalization: 16-bit Y′ = 4096 + 56064·Y′, C = 32768 + 57344·C (section 13);
//!   10-bit Y′ = 64 + 876·Y′, C = 512 + 896·C; 8-bit Y′ = 16 + 219·Y′,
//!   C = 128 + 224·C; RGB full range 0..2ⁿ − 1;
//! * chroma: 4:2:2 by point decimation at the co-sited (even) column; 4:2:0 with
//!   siting type 0 by point decimation horizontally and, vertically, the declared
//!   filter `[−1, 3, 3, −1] / 4` on rows 2y − 1 to 2y + 2 (clamped), on Y′CbCr: the
//!   exact inverse of the renderer's linear vertical upsampling
//!   (`pm_color::chroma::VERTICAL_420`, section 2 invariant (1));
//! * dither (when on): `dither_offset` at the output's LSB — 16 units of `y416`
//!   (12-bit), 64 units of `v216` (10-bit), one code of `x420` (10-bit), one 8-bit
//!   or 16-bit code; never on alpha;
//! * rounding half to even (WGSL `round`), then clipping to the codable range:
//!   12-bit 16..4079 (`y416` 256..65264), 10-bit 4..1019 (`v216` 256..65216,
//!   `x420` 4..1019 in the high bits), 8-bit 1..254. Alpha and RGB are full range.

use crate::Error;
use crate::dither::dither_offset;
use crate::settings::{AlphaOutput, InputFormat, WorkingEncoding};

pub const KR: f64 = 0.2126;
pub const KB: f64 = 0.0722;
pub const KG: f64 = 1.0 - KR - KB;

/// Below this alpha, unpremultiplied color is zero (section 9, "Alpha").
pub const ALPHA_GUARD: f64 = 1.0 / 4096.0;

/// A premultiplied RGBA working frame, row-major, tightly packed.
#[derive(Clone, Copy, Debug)]
pub struct WorkingFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [f32],
}

/// One plane of encoder input, tightly packed rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaneData {
    U16(Vec<u16>),
    U8(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub data: PlaneData,
    /// Samples (u16 or u8 units) per row.
    pub row_samples: usize,
    pub rows: usize,
}

impl Plane {
    pub fn u16(&self) -> Option<&[u16]> {
        match &self.data {
            PlaneData::U16(v) => Some(v),
            PlaneData::U8(_) => None,
        }
    }

    pub fn u8(&self) -> Option<&[u8]> {
        match &self.data {
            PlaneData::U8(v) => Some(v),
            PlaneData::U16(_) => None,
        }
    }

    /// Bytes per tightly packed row.
    pub fn row_bytes(&self) -> usize {
        match self.data {
            PlaneData::U16(_) => self.row_samples * 2,
            PlaneData::U8(_) => self.row_samples,
        }
    }
}

/// An encoder-input frame: what is copied into the CVPixelBuffer VideoToolbox encodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncoderInput {
    pub format: InputFormat,
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Plane>,
}

/// The plane geometry of `format` at `width` × `height`: (samples per row, rows, 16-bit).
pub fn plane_layout(format: InputFormat, width: u32, height: u32) -> Vec<(usize, usize, bool)> {
    let (w, h) = (width as usize, height as usize);
    match format {
        InputFormat::Y416 => vec![(w * 4, h, true)],
        InputFormat::V216 => vec![(w * 2, h, true)],
        InputFormat::V420 => vec![(w, h, false), (w, h / 2, false)],
        InputFormat::X420 => vec![(w, h, true), (w, h / 2, true)],
        InputFormat::V0a8 => vec![(w, h, false), (w, h / 2, false), (w, h, false)],
        InputFormat::I420 => vec![(w, h, false), (w / 2, h / 2, false), (w / 2, h / 2, false)],
        InputFormat::I420a => vec![
            (w, h, false),
            (w / 2, h / 2, false),
            (w / 2, h / 2, false),
            (w, h, false),
        ],
        InputFormat::Rgba8 => vec![(w * 4, h, false)],
        InputFormat::Rgba16 => vec![(w * 4, h, true)],
    }
}

impl EncoderInput {
    /// Checks the planes match the format's layout.
    pub fn validate(&self) -> Result<(), Error> {
        let layout = plane_layout(self.format, self.width, self.height);
        let ok = layout.len() == self.planes.len()
            && layout
                .iter()
                .zip(&self.planes)
                .all(|(&(samples, rows, wide), p)| {
                    p.row_samples == samples
                        && p.rows == rows
                        && match &p.data {
                            PlaneData::U16(v) => wide && v.len() == samples * rows,
                            PlaneData::U8(v) => !wide && v.len() == samples * rows,
                        }
                });
        if ok {
            Ok(())
        } else {
            Err(Error::InvalidFrame(format!(
                "encoder input does not match '{}' at {}×{}",
                String::from_utf8_lossy(&self.format.fourcc()),
                self.width,
                self.height
            )))
        }
    }
}

/// How working values become encoder input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvertParams {
    pub working: WorkingEncoding,
    pub alpha: AlphaOutput,
    pub dither: bool,
    /// Reproduce the legacy PNG byte boundaries for documents containing
    /// level-1 layers. All-level-2 documents retain straight-alpha precision.
    pub legacy_png: bool,
    /// Frame index: the dither seed.
    pub frame: u32,
}

/// The display encoding E: the sRGB piecewise curve extended to all reals.
pub fn e_encode(x: f64) -> f64 {
    let a = x.abs();
    let v = if a <= 0.003_130_8 {
        12.92 * a
    } else {
        1.055 * a.powf(1.0 / 2.4) - 0.055
    };
    v.copysign(x)
}

/// E⁻¹.
pub fn e_decode(x: f64) -> f64 {
    let a = x.abs();
    let v = if a <= 0.040_45 {
        a / 12.92
    } else {
        ((a + 0.055) / 1.055).powf(2.4)
    };
    v.copysign(x)
}

pub fn rgb_to_ycbcr(c: [f64; 3]) -> [f64; 3] {
    let y = KR * c[0] + KG * c[1] + KB * c[2];
    [
        y,
        (c[2] - y) / (2.0 * (1.0 - KB)),
        (c[0] - y) / (2.0 * (1.0 - KR)),
    ]
}

pub fn ycbcr_to_rgb(y: f64, cb: f64, cr: f64) -> [f64; 3] {
    let r = y + 2.0 * (1.0 - KR) * cr;
    let b = y + 2.0 * (1.0 - KB) * cb;
    [r, (y - KR * r - KB * b) / KG, b]
}

/// Encoded (stream-domain) RGB and alpha of a premultiplied working pixel.
pub fn output_rgba(w: [f32; 4], working: WorkingEncoding, alpha: AlphaOutput) -> ([f64; 3], f64) {
    let c = [w[0] as f64, w[1] as f64, w[2] as f64];
    let a = (w[3] as f64).clamp(0.0, 1.0);
    let encode = |c: [f64; 3]| match working {
        WorkingEncoding::Standard => c,
        WorkingEncoding::Linear => [e_encode(c[0]), e_encode(c[1]), e_encode(c[2])],
    };
    let straight = || {
        if a < ALPHA_GUARD {
            [0.0; 3]
        } else {
            [c[0] / a, c[1] / a, c[2] / a]
        }
    };
    match alpha {
        AlphaOutput::None => (encode(c), 1.0),
        AlphaOutput::Straight => (encode(straight()), a),
        AlphaOutput::Premultiplied => match working {
            // Identity output transform: premultiplied working values are written as they are.
            WorkingEncoding::Standard => (c, a),
            WorkingEncoding::Linear => {
                let e = encode(straight());
                ([e[0] * a, e[1] * a, e[2] * a], a)
            }
        },
    }
}

/// Today's transparent PNG export reads the premultiplied accumulator into
/// RGBA8, truncates the unpremultiplication in `alphaFrame`, and puts that
/// ImageData into a Canvas2D before encoding it. Reproduce both byte boundaries;
/// dividing the original floats by alpha changes existing translucent pixels.
fn legacy_png_rgba(p: [f32; 4]) -> [u8; 4] {
    let q = p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round_ties_even() as u32);
    let a = q[3];
    if a == 0 {
        return [0; 4];
    }
    let mut out = [0, 0, 0, a as u8];
    let normalized_a = a as f32 * (1.0f32 / 255.0);
    let reciprocal_a = 1.0f32 / normalized_a;
    for c in 0..3 {
        let straight = (q[c] * 255 / a).min(255);
        // Canvas2D stores ImageData in premultiplied N32. Chromium 152's
        // ARM64 SkConvertPixels uses SkSwizzler's RP-simulating unpremul:
        // normalize each byte in f32, divide by normalized alpha, scale
        // back, then round to even. An integer reciprocal table differs
        // on half codes (e.g. 5/6 -> 212, 11/22 -> 128, 38/60 -> 161).
        // Preserve these separate f32 operations; exact rational division
        // and reassociating the scale both change existing PNG pixels.
        let premul = (straight * a + 127) / 255;
        let normalized_c = premul as f32 * (1.0f32 / 255.0);
        out[c] = ((normalized_c * reciprocal_a) * 255.0)
            .min(255.0)
            .round_ties_even() as u8;
    }
    out
}

fn code(v: f64, lo: f64, hi: f64) -> f64 {
    v.round_ties_even().clamp(lo, hi)
}

fn pixel(frame: &WorkingFrame, x: usize, y: usize) -> [f32; 4] {
    let i = (y * frame.width as usize + x) * 4;
    [
        frame.rgba[i],
        frame.rgba[i + 1],
        frame.rgba[i + 2],
        frame.rgba[i + 3],
    ]
}

/// The declared 4:2:0 vertical filter (`pm_color::chroma::VERTICAL_420`): the
/// exact inverse of the renderer's linear vertical upsampling, so 4:2:0 sources
/// keep their chroma (section 2 invariant (1)). Taps on rows 2y − 1 to 2y + 2.
const VERTICAL_420: [(i64, f64); 4] = [(-1, -0.25), (0, 0.75), (1, 0.75), (2, -0.25)];

fn u16_plane(data: Vec<u16>, row_samples: usize, rows: usize) -> Plane {
    Plane {
        data: PlaneData::U16(data),
        row_samples,
        rows,
    }
}

fn u8_plane(data: Vec<u8>, row_samples: usize, rows: usize) -> Plane {
    Plane {
        data: PlaneData::U8(data),
        row_samples,
        rows,
    }
}

fn working_dimensions(frame: &WorkingFrame) -> Result<(usize, usize), Error> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(frame.rgba.len())
    {
        return Err(Error::InvalidFrame(format!(
            "working frame {}×{} has {} floats",
            w,
            h,
            frame.rgba.len()
        )));
    }
    Ok((w, h))
}

fn rgba_quant(
    frame: &WorkingFrame,
    params: &ConvertParams,
    x: usize,
    y: usize,
    scale: f64,
    wide: bool,
) -> [f64; 4] {
    let p = pixel(frame, x, y);
    if params.legacy_png
        && !wide
        && params.working == WorkingEncoding::Standard
        && params.alpha == AlphaOutput::Straight
    {
        return legacy_png_rgba(p).map(f64::from);
    }
    let (c, a) = output_rgba(p, params.working, params.alpha);
    let d = |ch| {
        if params.dither {
            dither_offset(params.frame, x as u32, y as u32, ch)
        } else {
            0.0
        }
    };
    [
        code(c[0] * scale + d(0), 0.0, scale),
        code(c[1] * scale + d(1), 0.0, scale),
        code(c[2] * scale + d(2), 0.0, scale),
        code(a * scale, 0.0, scale),
    ]
}

fn fill_rgba8(frame: &WorkingFrame, params: &ConvertParams, out: &mut [u8]) {
    let w = frame.width as usize;
    crate::par::rows(out, w * 4, |y, row| {
        for x in 0..w {
            let q = rgba_quant(frame, params, x, y, 255.0, false);
            for k in 0..4 {
                row[x * 4 + k] = q[k] as u8;
            }
        }
    });
}

/// Convert to tightly packed RGBA8 in a caller-owned buffer. The entire
/// destination is overwritten; its capacity can be reused across frames.
pub(crate) fn convert_rgba8_into(
    frame: &WorkingFrame,
    params: &ConvertParams,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let (w, h) = working_dimensions(frame)?;
    out.resize(w * h * 4, 0);
    fill_rgba8(frame, params, out);
    Ok(())
}

/// Converts one working frame to `format`. Rows are converted in parallel
/// ([`crate::par`]); every sample depends only on its own inputs, so the result is
/// the same bytes on any number of threads.
pub fn convert(
    frame: &WorkingFrame,
    format: InputFormat,
    params: &ConvertParams,
) -> Result<EncoderInput, Error> {
    let (w, h) = working_dimensions(frame)?;
    let horizontal = !matches!(
        format,
        InputFormat::Y416 | InputFormat::Rgba8 | InputFormat::Rgba16
    );
    let vertical = !matches!(
        format,
        InputFormat::Y416 | InputFormat::V216 | InputFormat::Rgba8 | InputFormat::Rgba16
    );
    if horizontal && w % 2 != 0 || vertical && h % 2 != 0 {
        return Err(Error::InvalidFrame(
            "subsampled encoder input needs even dimensions".into(),
        ));
    }
    let d = |x: usize, y: usize, ch: u32| -> f64 {
        if params.dither {
            dither_offset(params.frame, x as u32, y as u32, ch)
        } else {
            0.0
        }
    };
    let ycc = |x: usize, y: usize| -> ([f64; 3], f64) {
        let (c, a) = output_rgba(pixel(frame, x, y), params.working, params.alpha);
        (rgb_to_ycbcr(c), a)
    };
    // Chroma of 4:2:0 sample (cx, cy): siting type 0, point decimation
    // horizontally (the even column), the declared filter vertically.
    let chroma420 = |cx: usize, cy: usize| -> (f64, f64) {
        let (mut cb, mut cr) = (0.0, 0.0);
        for (k, wt) in VERTICAL_420 {
            let row = (2 * cy as i64 + k).clamp(0, h as i64 - 1) as usize;
            let (v, _) = ycc(2 * cx, row);
            cb += wt * v[1];
            cr += wt * v[2];
        }
        (cb, cr)
    };
    // Quantizers at the output depth (dither in the output domain, then round
    // half to even and clip to the codable range).
    let luma8 = |v: f64, x: usize, y: usize| code(v * 219.0 + 16.0 + d(x, y, 0), 1.0, 254.0) as u8;
    let chroma8 = |c: f64, x: usize, y: usize, ch: u32| {
        code(c * 224.0 + 128.0 + d(x, y, ch), 1.0, 254.0) as u8
    };
    let alpha8 = |a: f64| code(a * 255.0, 0.0, 255.0) as u8;
    let luma10 =
        |v: f64, x: usize, y: usize| (code(v * 876.0 + 64.0 + d(x, y, 0), 4.0, 1019.0) as u16) << 6;
    let chroma10 = |c: f64, x: usize, y: usize, ch: u32| {
        (code(c * 896.0 + 512.0 + d(x, y, ch), 4.0, 1019.0) as u16) << 6
    };
    let luma_plane8 = || {
        let mut luma = vec![0u8; w * h];
        crate::par::rows(&mut luma, w, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                *o = luma8(ycc(x, y).0[0], x, y);
            }
        });
        u8_plane(luma, w, h)
    };
    let alpha_plane8 = || {
        let mut alpha = vec![0u8; w * h];
        crate::par::rows(&mut alpha, w, |y, row| {
            for (x, o) in row.iter_mut().enumerate() {
                *o = alpha8(ycc(x, y).1);
            }
        });
        u8_plane(alpha, w, h)
    };
    // Interleaved CbCr (`420v`, `v0a8`).
    let cbcr_plane8 = || {
        let mut chroma = vec![0u8; w * h / 2];
        crate::par::rows(&mut chroma, w, |cy, row| {
            for cx in 0..w / 2 {
                let (cb, cr) = chroma420(cx, cy);
                row[cx * 2] = chroma8(cb, cx, cy, 1);
                row[cx * 2 + 1] = chroma8(cr, cx, cy, 2);
            }
        });
        u8_plane(chroma, w, h / 2)
    };
    // Planar Cb or Cr (`yuv420p`).
    let chroma_plane8 = |ch: u32| {
        let cw = w / 2;
        let mut out = vec![0u8; cw * (h / 2)];
        crate::par::rows(&mut out, cw, |cy, row| {
            for (cx, o) in row.iter_mut().enumerate() {
                let (cb, cr) = chroma420(cx, cy);
                *o = chroma8(if ch == 1 { cb } else { cr }, cx, cy, ch);
            }
        });
        u8_plane(out, cw, h / 2)
    };
    let planes = match format {
        InputFormat::Y416 => {
            let mut out = vec![0u16; w * h * 4];
            crate::par::rows(&mut out, w * 4, |y, row| {
                for x in 0..w {
                    let (v, a) = ycc(x, y);
                    let o = &mut row[x * 4..x * 4 + 4];
                    o[0] = code(a * 65535.0, 0.0, 65535.0) as u16;
                    o[1] = code(v[0] * 56064.0 + 4096.0 + 16.0 * d(x, y, 0), 256.0, 65264.0) as u16;
                    o[2] =
                        code(v[1] * 57344.0 + 32768.0 + 16.0 * d(x, y, 1), 256.0, 65264.0) as u16;
                    o[3] =
                        code(v[2] * 57344.0 + 32768.0 + 16.0 * d(x, y, 2), 256.0, 65264.0) as u16;
                }
            });
            vec![u16_plane(out, w * 4, h)]
        }
        InputFormat::V216 => {
            let mut out = vec![0u16; w * h * 2];
            crate::par::rows(&mut out, w * 2, |y, row| {
                for t in 0..w / 2 {
                    let (x0, x1) = (2 * t, 2 * t + 1);
                    let (a, _) = ycc(x0, y);
                    let (b, _) = ycc(x1, y);
                    let o = &mut row[t * 4..t * 4 + 4];
                    o[0] =
                        code(a[1] * 57344.0 + 32768.0 + 64.0 * d(t, y, 1), 256.0, 65216.0) as u16;
                    o[1] =
                        code(a[0] * 56064.0 + 4096.0 + 64.0 * d(x0, y, 0), 256.0, 65216.0) as u16;
                    o[2] =
                        code(a[2] * 57344.0 + 32768.0 + 64.0 * d(t, y, 2), 256.0, 65216.0) as u16;
                    o[3] =
                        code(b[0] * 56064.0 + 4096.0 + 64.0 * d(x1, y, 0), 256.0, 65216.0) as u16;
                }
            });
            vec![u16_plane(out, w * 2, h)]
        }
        InputFormat::V420 => vec![luma_plane8(), cbcr_plane8()],
        InputFormat::V0a8 => vec![luma_plane8(), cbcr_plane8(), alpha_plane8()],
        InputFormat::I420 => vec![luma_plane8(), chroma_plane8(1), chroma_plane8(2)],
        InputFormat::I420a => vec![
            luma_plane8(),
            chroma_plane8(1),
            chroma_plane8(2),
            alpha_plane8(),
        ],
        InputFormat::X420 => {
            let mut luma = vec![0u16; w * h];
            crate::par::rows(&mut luma, w, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    *o = luma10(ycc(x, y).0[0], x, y);
                }
            });
            let mut chroma = vec![0u16; w * h / 2];
            crate::par::rows(&mut chroma, w, |cy, row| {
                for cx in 0..w / 2 {
                    let (cb, cr) = chroma420(cx, cy);
                    row[cx * 2] = chroma10(cb, cx, cy, 1);
                    row[cx * 2 + 1] = chroma10(cr, cx, cy, 2);
                }
            });
            vec![u16_plane(luma, w, h), u16_plane(chroma, w, h / 2)]
        }
        InputFormat::Rgba8 | InputFormat::Rgba16 => {
            // Full-range RGB in the output encoding, straight alpha (PNG), never
            // dithered alpha. Opaque outputs are over black with alpha at full.
            let (scale, wide) = match format {
                InputFormat::Rgba8 => (255.0, false),
                _ => (65535.0, true),
            };
            if wide {
                let mut out = vec![0u16; w * h * 4];
                crate::par::rows(&mut out, w * 4, |y, row| {
                    for x in 0..w {
                        let q = rgba_quant(frame, params, x, y, scale, true);
                        for k in 0..4 {
                            row[x * 4 + k] = q[k] as u16;
                        }
                    }
                });
                vec![u16_plane(out, w * 4, h)]
            } else {
                let mut out = vec![0u8; w * h * 4];
                fill_rgba8(frame, params, &mut out);
                vec![u8_plane(out, w * 4, h)]
            }
        }
    };
    Ok(EncoderInput {
        format,
        width: frame.width,
        height: frame.height,
        planes,
    })
}

/// Decoded values of one encoder-input pixel, normalized, for verification:
/// Y′CbCr formats give (Y′, Cb, Cr, A) with A = 1 when the format has no alpha;
/// RGBA formats give (R, G, B, A) in 0..1.
pub fn sample_ycbcra(input: &EncoderInput, x: usize, y: usize) -> [f64; 4] {
    let w = input.width as usize;
    let y8 = |v: u8| (v as f64 - 16.0) / 219.0;
    let c8 = |v: u8| (v as f64 - 128.0) / 224.0;
    match input.format {
        InputFormat::Y416 => {
            let p = input.planes[0].u16().expect("y416 is 16-bit");
            let i = (y * w + x) * 4;
            [
                (p[i + 1] as f64 - 4096.0) / 56064.0,
                (p[i + 2] as f64 - 32768.0) / 57344.0,
                (p[i + 3] as f64 - 32768.0) / 57344.0,
                p[i] as f64 / 65535.0,
            ]
        }
        InputFormat::V216 => {
            let p = input.planes[0].u16().expect("v216 is 16-bit");
            let i = (y * w + (x & !1)) * 2;
            let yv = if x.is_multiple_of(2) {
                p[i + 1]
            } else {
                p[i + 3]
            };
            [
                (yv as f64 - 4096.0) / 56064.0,
                (p[i] as f64 - 32768.0) / 57344.0,
                (p[i + 2] as f64 - 32768.0) / 57344.0,
                1.0,
            ]
        }
        InputFormat::V420 | InputFormat::V0a8 => {
            let l = input.planes[0].u8().expect("8-bit luma");
            let c = input.planes[1].u8().expect("8-bit chroma");
            let ci = (y / 2) * w + (x / 2) * 2;
            let a = match input.format {
                InputFormat::V0a8 => {
                    input.planes[2].u8().expect("8-bit alpha")[y * w + x] as f64 / 255.0
                }
                _ => 1.0,
            };
            [y8(l[y * w + x]), c8(c[ci]), c8(c[ci + 1]), a]
        }
        InputFormat::X420 => {
            let l = input.planes[0].u16().expect("x420 is 16-bit");
            let c = input.planes[1].u16().expect("x420 is 16-bit");
            let ci = (y / 2) * w + (x / 2) * 2;
            [
                ((l[y * w + x] >> 6) as f64 - 64.0) / 876.0,
                ((c[ci] >> 6) as f64 - 512.0) / 896.0,
                ((c[ci + 1] >> 6) as f64 - 512.0) / 896.0,
                1.0,
            ]
        }
        InputFormat::I420 | InputFormat::I420a => {
            let l = input.planes[0].u8().expect("8-bit luma");
            let cb = input.planes[1].u8().expect("8-bit chroma");
            let cr = input.planes[2].u8().expect("8-bit chroma");
            let ci = (y / 2) * (w / 2) + x / 2;
            let a = match input.format {
                InputFormat::I420a => {
                    input.planes[3].u8().expect("8-bit alpha")[y * w + x] as f64 / 255.0
                }
                _ => 1.0,
            };
            [y8(l[y * w + x]), c8(cb[ci]), c8(cr[ci]), a]
        }
        InputFormat::Rgba8 => {
            let p = input.planes[0].u8().expect("8-bit RGBA");
            let i = (y * w + x) * 4;
            [0, 1, 2, 3].map(|k| p[i + k] as f64 / 255.0)
        }
        InputFormat::Rgba16 => {
            let p = input.planes[0].u16().expect("16-bit RGBA");
            let i = (y * w + x) * 4;
            [0, 1, 2, 3].map(|k| p[i + k] as f64 / 65535.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_png_keeps_the_legacy_byte_boundaries() {
        assert_eq!(legacy_png_rgba([0.0; 4]), [0; 4]);
        assert_eq!(legacy_png_rgba([0.2, 0.4, 0.6, 1.0]), [51, 102, 153, 255]);
        assert_eq!(legacy_png_rgba([0.2, 0.1, 0.0, 0.5]), [102, 52, 0, 128]);
        // Quantization precedes alpha division; RGB is clipped only after
        // division, including the out-of-gamut RGB > alpha bitmap case.
        assert_eq!(legacy_png_rgba([0.01, 0.0, 0.0, 0.001]), [0; 4]);
        assert_eq!(legacy_png_rgba([0.7, 0.0, 0.0, 0.5]), [255, 0, 0, 128]);
        let values = [0.2, 0.1, 0.0, 0.5];
        let f = WorkingFrame {
            width: 1,
            height: 1,
            rgba: &values,
        };
        let p = params(AlphaOutput::Straight, false);
        let byte = convert(&f, InputFormat::Rgba8, &p).unwrap();
        assert_eq!(byte.planes[0].u8().unwrap(), [102, 52, 0, 128]);
        let wide = convert(&f, InputFormat::Rgba16, &p).unwrap();
        assert_eq!(wide.planes[0].u16().unwrap(), [26214, 13107, 0, 32768]);
    }

    #[test]
    fn level2_png_unpremultiplies_before_byte_quantization() {
        let values = [0.0003, 0.0006, 0.0009, 0.003];
        let frame = WorkingFrame {
            width: 1,
            height: 1,
            rgba: &values,
        };
        let mut p = params(AlphaOutput::Straight, false);
        let legacy = convert(&frame, InputFormat::Rgba8, &p).unwrap();
        p.legacy_png = false;
        let precise = convert(&frame, InputFormat::Rgba8, &p).unwrap();
        assert_eq!(precise.planes[0].u8().unwrap(), [26, 51, 76, 1]);
        assert_ne!(precise, legacy);
        // The flag is irrelevant to native-depth PNG output.
        let wide = convert(&frame, InputFormat::Rgba16, &p).unwrap();
        p.legacy_png = true;
        assert_eq!(convert(&frame, InputFormat::Rgba16, &p).unwrap(), wide);
    }

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Vec<f32> {
        let f = &f;
        (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| f(x, y)))
            .collect::<Vec<_>>()
    }

    fn params(alpha: AlphaOutput, dither: bool) -> ConvertParams {
        ConvertParams {
            legacy_png: true,
            working: WorkingEncoding::Standard,
            alpha,
            dither,
            frame: 0,
        }
    }

    #[test]
    fn reusable_rgba8_matches_convert_for_modes_and_special_values() {
        let values = [
            0.0,
            0.0,
            0.0,
            0.0,
            0.2,
            0.1,
            0.0,
            0.5,
            0.0003,
            0.0006,
            0.0009,
            0.003,
            -0.25,
            1.25,
            0.5,
            1.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            0.5,
            -0.0,
            1.0,
            f32::INFINITY,
            1.0,
            0.0,
            0.0,
            1.0 / 8192.0,
            0.7,
            0.0,
            0.0,
            0.5,
        ];
        let frame = WorkingFrame {
            width: 4,
            height: 2,
            rgba: &values,
        };
        let mut reused = vec![0xaa; 128];
        for working in [WorkingEncoding::Standard, WorkingEncoding::Linear] {
            for alpha in [
                AlphaOutput::None,
                AlphaOutput::Straight,
                AlphaOutput::Premultiplied,
            ] {
                for legacy_png in [false, true] {
                    for dither in [false, true] {
                        let params = ConvertParams {
                            working,
                            alpha,
                            legacy_png,
                            dither,
                            frame: 37,
                        };
                        let expected = convert(&frame, InputFormat::Rgba8, &params).unwrap();
                        convert_rgba8_into(&frame, &params, &mut reused).unwrap();
                        assert_eq!(
                            reused.as_slice(),
                            expected.planes[0].u8().unwrap(),
                            "{params:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reusable_rgba8_resizes_and_rejects_invalid_frames_without_changing_buffer() {
        let mut out = Vec::with_capacity(128);
        let allocation = out.as_ptr();
        let params = params(AlphaOutput::Straight, false);
        for (width, height, pixel) in [(2, 2, 0.2), (4, 3, 0.8), (1, 1, 0.4)] {
            let values = [pixel, pixel * 0.5, 0.0, 0.5].repeat((width * height) as usize);
            let frame = WorkingFrame {
                width,
                height,
                rgba: &values,
            };
            convert_rgba8_into(&frame, &params, &mut out).unwrap();
            assert_eq!(out.len(), (width * height * 4) as usize);
            assert_eq!(out.as_ptr(), allocation);
            assert_eq!(
                out.as_slice(),
                convert(&frame, InputFormat::Rgba8, &params).unwrap().planes[0]
                    .u8()
                    .unwrap()
            );
        }
        let before = out.clone();
        for (width, height, values) in [(0, 1, vec![]), (2, 2, vec![0.0; 15])] {
            let frame = WorkingFrame {
                width,
                height,
                rgba: &values,
            };
            assert!(matches!(
                convert_rgba8_into(&frame, &params, &mut out),
                Err(Error::InvalidFrame(_))
            ));
            assert_eq!(out, before);
        }
    }

    #[test]
    fn e_curve_round_trips_and_extends_to_all_reals() {
        for i in -100..=200 {
            let x = i as f64 / 100.0;
            assert!((e_decode(e_encode(x)) - x).abs() < 1e-12, "{x}");
        }
        assert!((e_encode(0.5) - 0.735_357).abs() < 1e-6);
        assert_eq!(e_encode(-0.5), -e_encode(0.5));
    }

    #[test]
    fn matrix_round_trips() {
        for c in [[1.0, 0.0, 0.0], [0.2, 0.5, 0.9], [-0.1, 1.2, 0.3]] {
            let v = rgb_to_ycbcr(c);
            let back = ycbcr_to_rgb(v[0], v[1], v[2]);
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-12);
            }
        }
    }

    /// Black, white and the primaries land on the textbook Rec.709 codes.
    #[test]
    fn reference_codes_at_8_10_and_12_bits() {
        let colors: [[f32; 4]; 3] = [
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ];
        let f = frame(2, 2, |x, y| colors[((x + y) as usize).min(2)]);
        let wf = WorkingFrame {
            width: 2,
            height: 2,
            rgba: &f,
        };
        let v420 = convert(&wf, InputFormat::V420, &params(AlphaOutput::None, false)).unwrap();
        // (0,0) black, (1,0) white, (1,1) red.
        assert_eq!(v420.planes[0].u8().unwrap(), &[16, 235, 235, 63]);
        let y416 = convert(&wf, InputFormat::Y416, &params(AlphaOutput::None, false)).unwrap();
        let p = y416.planes[0].u16().unwrap();
        assert_eq!(&p[0..4], &[65535, 4096, 32768, 32768]);
        assert_eq!(&p[4..8], &[65535, 60160, 32768, 32768]);
        // Red at 12 bits: Y′ 0.2126 → 16 + 219·16·0.2126 → 1004.9 (×16 = 16079.9 → 16080).
        let red = &p[12..16];
        assert_eq!(
            red[1],
            (0.2126f64 * 56064.0 + 4096.0).round_ties_even() as u16
        );
        assert_eq!(red[3], 61440, "Cr of pure red is +0.5: 12-bit 3840 × 16");
        let v216 = convert(&wf, InputFormat::V216, &params(AlphaOutput::None, false)).unwrap();
        let p = v216.planes[0].u16().unwrap();
        // Row 0: black (co-sited chroma) and white.
        assert_eq!(&p[0..4], &[32768, 4096, 32768, 60160]);
    }

    #[test]
    fn values_on_the_output_grid_survive_dither_and_neutral_stays_neutral() {
        // Every 8-bit luma code 16..=235 as a grey, set exactly.
        for code8 in 16..=235u32 {
            let yv = (code8 as f64 - 16.0) / 219.0;
            let f = frame(64, 64, |_, _| {
                let v = yv as f32;
                [v, v, v, 1.0]
            });
            let wf = WorkingFrame {
                width: 64,
                height: 64,
                rgba: &f,
            };
            for frame_index in 0..2 {
                let out = convert(
                    &wf,
                    InputFormat::V420,
                    &ConvertParams {
                        frame: frame_index,
                        ..params(AlphaOutput::None, true)
                    },
                )
                .unwrap();
                assert!(
                    out.planes[0]
                        .u8()
                        .unwrap()
                        .iter()
                        .all(|&v| v as u32 == code8),
                    "code {code8}"
                );
                assert!(
                    out.planes[1].u8().unwrap().iter().all(|&v| v == 128),
                    "neutral chroma at code {code8}"
                );
            }
        }
    }

    /// The bit-depth bar's dither clause as S2 measured it: on a shallow ramp (0.40 to
    /// 0.42 over 1,920 columns), 16-column windows of the column means over 64 rows are
    /// within 0.05 CV of ideal with dither (S2: 0.009), and not without (S2: 0.48).
    #[test]
    fn dither_removes_banding_on_a_shallow_ramp() {
        let (w, h) = (1920u32, 64u32);
        let ideal = |x: usize| (0.40 + 0.02 * x as f64 / (w - 1) as f64) * 219.0 + 16.0;
        let f = frame(w, h, |x, _| {
            let v = (0.40 + 0.02 * x as f64 / (w - 1) as f64) as f32;
            [v, v, v, 1.0]
        });
        let wf = WorkingFrame {
            width: w,
            height: h,
            rgba: &f,
        };
        let worst = |dither: bool| {
            let out = convert(&wf, InputFormat::V420, &params(AlphaOutput::None, dither)).unwrap();
            let l = out.planes[0].u8().unwrap();
            let column: Vec<f64> = (0..w as usize)
                .map(|x| {
                    (0..h as usize)
                        .map(|y| l[y * w as usize + x] as f64)
                        .sum::<f64>()
                        / h as f64
                })
                .collect();
            (0..w as usize - 16)
                .map(|x0| {
                    let got = column[x0..x0 + 16].iter().sum::<f64>() / 16.0;
                    let want = (x0..x0 + 16).map(ideal).sum::<f64>() / 16.0;
                    (got - want).abs()
                })
                .fold(0.0, f64::max)
        };
        let (on, off) = (worst(true), worst(false));
        assert!(off > 0.4, "undithered banding {off}");
        assert!(on < 0.05, "dithered window error {on}");
    }

    #[test]
    fn dither_is_deterministic_and_never_touches_alpha() {
        let f = frame(64, 64, |x, y| {
            let a = (x + y) as f32 / 126.0;
            [0.3 * a, 0.6 * a, 0.2 * a, a]
        });
        let wf = WorkingFrame {
            width: 64,
            height: 64,
            rgba: &f,
        };
        let p = |frame| ConvertParams {
            frame,
            ..params(AlphaOutput::Straight, true)
        };
        let a = convert(&wf, InputFormat::Y416, &p(3)).unwrap();
        assert_eq!(a, convert(&wf, InputFormat::Y416, &p(3)).unwrap());
        assert_ne!(a, convert(&wf, InputFormat::Y416, &p(4)).unwrap());
        let undithered = convert(
            &wf,
            InputFormat::Y416,
            &params(AlphaOutput::Straight, false),
        )
        .unwrap();
        let (pa, pu) = (
            a.planes[0].u16().unwrap(),
            undithered.planes[0].u16().unwrap(),
        );
        for i in (0..pa.len()).step_by(4) {
            assert_eq!(pa[i], pu[i], "alpha sample {i}");
        }
    }

    #[test]
    fn alpha_modes() {
        // 50% alpha, premultiplied red.
        let px = [0.5f32, 0.0, 0.0, 0.5];
        let (c, a) = output_rgba(px, WorkingEncoding::Standard, AlphaOutput::Straight);
        assert_eq!((c, a), ([1.0, 0.0, 0.0], 0.5));
        let (c, _) = output_rgba(px, WorkingEncoding::Standard, AlphaOutput::Premultiplied);
        assert_eq!(c, [0.5, 0.0, 0.0]);
        let (c, a) = output_rgba(px, WorkingEncoding::Standard, AlphaOutput::None);
        assert_eq!((c, a), ([0.5, 0.0, 0.0], 1.0));
        // Linear working values: E after unpremultiplying, premultiplied again in E.
        let lin = [0.25f32, 0.0, 0.0, 0.5];
        let (c, _) = output_rgba(lin, WorkingEncoding::Linear, AlphaOutput::Premultiplied);
        assert!((c[0] - e_encode(0.5) * 0.5).abs() < 1e-12);
        // Below the guard, color is dropped.
        let (c, a) = output_rgba(
            [1e-5, 1e-5, 1e-5, 1e-4],
            WorkingEncoding::Standard,
            AlphaOutput::Straight,
        );
        assert_eq!(c, [0.0; 3]);
        assert!((a - 1e-4).abs() < 1e-9);
    }

    #[test]
    fn canvas_png_keeps_skia_float32_half_code_rounding() {
        // Captured through Chromium 152 Canvas2D putImageData -> PNG.
        // These witnesses distinguish the float32 RP sequence from an
        // integer reciprocal, exact division, half-up, and unconditional floor.
        for (input, expected) in [
            ([4u8, 3, 5, 6], [170, 128, 212, 6]),
            ([14, 11, 17, 22], [162, 128, 197, 22]),
            ([38, 29, 47, 60], [161, 123, 200, 60]),
        ] {
            let p = input.map(|c| c as f32 / 255.0);
            assert_eq!(legacy_png_rgba(p), expected, "{input:?}");
        }
    }

    #[test]
    fn canvas_png_matches_every_captured_channel_alpha_pair() {
        let reference = include_bytes!("../tests/fixtures/canvas-alpha-arm64.bin");
        assert_eq!(reference.len(), 256 * 256);
        for a in 0..=255u32 {
            for c in 0..=255u32 {
                let p = [c, c, c, a].map(|v| v as f32 / 255.0);
                let actual = legacy_png_rgba(p);
                let expected = reference[(a * 256 + c) as usize];
                assert_eq!(
                    actual,
                    [expected, expected, expected, a as u8],
                    "c={c}, a={a}"
                );
            }
        }
    }

    #[test]
    fn superwhites_and_subblacks_clip_to_the_codable_range() {
        let f = frame(2, 2, |x, _| {
            if x == 0 {
                [-0.5, -0.5, -0.5, 1.0]
            } else {
                [2.0, 2.0, 2.0, 1.0]
            }
        });
        let wf = WorkingFrame {
            width: 2,
            height: 2,
            rgba: &f,
        };
        let v = convert(&wf, InputFormat::V420, &params(AlphaOutput::None, false)).unwrap();
        assert_eq!(&v.planes[0].u8().unwrap()[..2], &[1, 254]);
        let v = convert(&wf, InputFormat::V216, &params(AlphaOutput::None, false)).unwrap();
        assert_eq!(v.planes[0].u16().unwrap()[1], 256);
        assert_eq!(v.planes[0].u16().unwrap()[3], 65216);
        let v = convert(&wf, InputFormat::Y416, &params(AlphaOutput::None, false)).unwrap();
        assert_eq!(v.planes[0].u16().unwrap()[1], 256);
        assert_eq!(v.planes[0].u16().unwrap()[5], 65264);
    }

    #[test]
    fn chroma_siting_and_sampling_round_trip() {
        // Distinct colors per pixel: 4:2:2 takes the even column; 4:2:0 with two rows
        // (clamped taps) averages them.
        let f = frame(4, 2, |x, y| [x as f32 / 4.0, y as f32 / 2.0, 0.25, 1.0]);
        let wf = WorkingFrame {
            width: 4,
            height: 2,
            rgba: &f,
        };
        let v216 = convert(&wf, InputFormat::V216, &params(AlphaOutput::None, false)).unwrap();
        let s = sample_ycbcra(&v216, 1, 0);
        let even = rgb_to_ycbcr([0.0, 0.0, 0.25]);
        assert!(
            (s[1] - even[1]).abs() < 1e-4,
            "odd pixel carries the co-sited chroma"
        );
        let v420 = convert(&wf, InputFormat::V420, &params(AlphaOutput::None, false)).unwrap();
        let s = sample_ycbcra(&v420, 2, 1);
        let mean = (rgb_to_ycbcr([0.5, 0.0, 0.25])[2] + rgb_to_ycbcr([0.5, 0.5, 0.25])[2]) / 2.0;
        assert!((s[2] - mean).abs() < 0.5 / 224.0 + 1e-9);
        assert!(v420.validate().is_ok() && v216.validate().is_ok());
        assert!(
            convert(
                &WorkingFrame {
                    width: 3,
                    height: 2,
                    rgba: &[0.0; 24]
                },
                InputFormat::V216,
                &params(AlphaOutput::None, false)
            )
            .is_err()
        );
    }

    /// The 10-bit, alpha, planar, and RGBA inputs agree with the 8-bit
    /// interleaved reference and land on their textbook codes.
    #[test]
    fn hevc_vpx_and_png_inputs() {
        let colors: [[f32; 4]; 4] = [
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.5, 0.0, 0.0, 0.5],
            [0.0, 0.0, 0.0, 0.0],
        ];
        let f = frame(4, 4, |x, y| colors[((x / 2) + 2 * (y / 2)) as usize]);
        let wf = WorkingFrame {
            width: 4,
            height: 4,
            rgba: &f,
        };
        let opaque = params(AlphaOutput::None, false);
        let x420 = convert(&wf, InputFormat::X420, &opaque).unwrap();
        assert!(x420.validate().is_ok());
        let l = x420.planes[0].u16().unwrap();
        assert_eq!(l[0], 64 << 6, "10-bit black");
        assert_eq!(l[2], 940 << 6, "10-bit white");
        assert!(l.iter().all(|v| v & 0x3f == 0), "MSB-aligned");
        assert_eq!(
            x420.planes[1].u16().unwrap()[2],
            512 << 6,
            "neutral chroma (white over black column)"
        );

        let v420 = convert(&wf, InputFormat::V420, &opaque).unwrap();
        let i420 = convert(&wf, InputFormat::I420, &opaque).unwrap();
        assert!(i420.validate().is_ok());
        assert_eq!(v420.planes[0], i420.planes[0], "same luma");
        let inter = v420.planes[1].u8().unwrap();
        let (cb, cr) = (i420.planes[1].u8().unwrap(), i420.planes[2].u8().unwrap());
        for i in 0..cb.len() {
            assert_eq!(
                (inter[2 * i], inter[2 * i + 1]),
                (cb[i], cr[i]),
                "chroma {i}"
            );
        }

        let straight = params(AlphaOutput::Straight, false);
        let v0a8 = convert(&wf, InputFormat::V0a8, &straight).unwrap();
        let i420a = convert(&wf, InputFormat::I420a, &straight).unwrap();
        assert!(v0a8.validate().is_ok() && i420a.validate().is_ok());
        let a = v0a8.planes[2].u8().unwrap();
        assert_eq!(
            (a[0], a[8], a[10]),
            (255, 128, 0),
            "alpha full range, 0.5 → 128"
        );
        assert_eq!(v0a8.planes[2], i420a.planes[3]);
        // Straight red under 50% alpha is full red.
        let red = sample_ycbcra(&v0a8, 0, 2);
        assert!((red[0] - KR).abs() < 0.5 / 219.0, "{red:?}");

        let rgba8 = convert(&wf, InputFormat::Rgba8, &straight).unwrap();
        let p = rgba8.planes[0].u8().unwrap();
        assert_eq!(&p[0..4], &[0, 0, 0, 255]);
        assert_eq!(&p[8..12], &[255, 255, 255, 255]);
        let at = |x: usize, y: usize| (y * 4 + x) * 4;
        assert_eq!(
            &p[at(0, 2)..at(0, 2) + 4],
            &[255, 0, 0, 128],
            "straight red"
        );
        assert_eq!(&p[at(2, 2)..at(2, 2) + 4], &[0, 0, 0, 0]);
        let rgba16 = convert(&wf, InputFormat::Rgba16, &opaque).unwrap();
        let q = rgba16.planes[0].u16().unwrap();
        assert_eq!(
            &q[at(0, 2)..at(0, 2) + 4],
            &[32768, 0, 0, 65535],
            "over black"
        );
        // PNG takes any size; subsampled inputs refuse odd sizes.
        let odd = frame(3, 3, |_, _| [0.25, 0.5, 0.75, 1.0]);
        let owf = WorkingFrame {
            width: 3,
            height: 3,
            rgba: &odd,
        };
        assert!(convert(&owf, InputFormat::Rgba16, &opaque).is_ok());
        assert!(convert(&owf, InputFormat::I420, &opaque).is_err());
        assert!(convert(&owf, InputFormat::Y416, &opaque).is_ok());
    }

    /// Dither leaves every value on the 8-bit RGB grid where it is and never
    /// touches alpha, and the 10-bit Y′CbCr grid the same.
    #[test]
    fn dither_keeps_grid_values_in_rgb_and_10_bit() {
        for code8 in [0u32, 1, 17, 128, 254, 255] {
            let v = code8 as f32 / 255.0;
            let f = frame(32, 32, |_, _| [v, v, v, 1.0]);
            let wf = WorkingFrame {
                width: 32,
                height: 32,
                rgba: &f,
            };
            let out = convert(&wf, InputFormat::Rgba8, &params(AlphaOutput::None, true)).unwrap();
            assert!(
                out.planes[0]
                    .u8()
                    .unwrap()
                    .chunks_exact(4)
                    .all(|p| p == [code8 as u8, code8 as u8, code8 as u8, 255]),
                "RGB code {code8}"
            );
        }
        for code10 in [64u32, 65, 500, 939, 940] {
            let v = ((code10 as f64 - 64.0) / 876.0) as f32;
            let f = frame(32, 32, |_, _| [v, v, v, 1.0]);
            let wf = WorkingFrame {
                width: 32,
                height: 32,
                rgba: &f,
            };
            let out = convert(&wf, InputFormat::X420, &params(AlphaOutput::None, true)).unwrap();
            assert!(
                out.planes[0]
                    .u16()
                    .unwrap()
                    .iter()
                    .all(|&s| s >> 6 == code10 as u16),
                "10-bit luma {code10}"
            );
            assert!(out.planes[1].u16().unwrap().iter().all(|&s| s >> 6 == 512));
        }
    }
}
