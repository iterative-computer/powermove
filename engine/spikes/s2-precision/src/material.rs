//! Test material made with AVFoundation (AVAssetWriter → VideoToolbox), never prores_ks.

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::*;
use objc2_foundation::{NSDictionary, NSString};

use crate::avio::{self, VideoWriter};
use crate::cf::{Dict, Result};
use crate::color;
use crate::pixbuf::{self, PixelBuffer};

pub const W: usize = 1920;
pub const H: usize = 1080;
pub const FRAMES: usize = 6;
pub const TIMESCALE: i32 = 24000;
pub const FRAME_DUR: i64 = 1001;

/// Deterministic xorshift generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % ((hi - lo + 1) as u64)) as u32
    }
}

/// 75% color bars in E-encoded RGB.
const BARS: [[f64; 3]; 8] = [
    [0.75, 0.75, 0.75],
    [0.75, 0.75, 0.0],
    [0.0, 0.75, 0.75],
    [0.0, 0.75, 0.0],
    [0.75, 0.0, 0.75],
    [0.75, 0.0, 0.0],
    [0.0, 0.0, 0.75],
    [0.0, 0.0, 0.0],
];

/// Normalized Y′CbCr (Y′ in 0..1, C in −0.5..0.5) for a pixel of frame `f`.
/// Top third: luma ramp with chroma gradients. Middle: uniform noise over the
/// codable range (independent Y′, Cb, Cr: superwhites, sub-blacks, out-of-gamut RGB).
/// Bottom: 75% bars and super-black/super-white patches.
fn pattern(f: usize, x: usize, y: usize, rng: &mut Rng, depth_codes: f64) -> [f64; 3] {
    let third = H / 3;
    if y < third {
        let yv = x as f64 / (W - 1) as f64;
        let cb = (y as f64 / third as f64 - 0.5) * 0.9 * if f % 2 == 0 { 1.0 } else { -1.0 };
        let cr = ((x + f * 97) % W) as f64 / W as f64 - 0.5;
        [yv, cb * 0.9, cr * 0.9]
    } else if y < 2 * third {
        // Uniform over the codable range: about −0.07..1.09 luma, ±0.57 chroma at 12-bit.
        let lo = -0.0730;
        let hi = 1.0912;
        let r = |rng: &mut Rng| rng.next() as f64 / u64::MAX as f64;
        let _ = depth_codes;
        [lo + (hi - lo) * r(rng), (r(rng) - 0.5) * 1.13, (r(rng) - 0.5) * 1.13]
    } else {
        let bar = x * 8 / W;
        if y < 2 * third + third / 2 {
            color::rgb_to_ycbcr(BARS[bar])
        } else {
            let lvl = [-0.07, -0.02, 0.0, 0.02, 0.5, 0.98, 1.0, 1.09][bar];
            [lvl, 0.0, 0.0]
        }
    }
}

/// A y416 frame: 16-bit design values (not multiples of 16), so the encoder quantizes.
pub fn y416_frame(f: usize, alpha_ramp: bool) -> Result<PixelBuffer> {
    let pb = PixelBuffer::new(W, H, pixbuf::Y416)?;
    let mut rng = Rng::new(f as u64 + 1);
    let mut d = vec![0u16; W * H * 4];
    for y in 0..H {
        for x in 0..W {
            let [yv, cb, cr] = pattern(f, x, y, &mut rng, 4096.0);
            let i = (y * W + x) * 4;
            let a = if alpha_ramp { if y % 270 < 30 { 0.0 } else { x as f64 / (W - 1) as f64 } } else { 1.0 };
            let q = |v: f64| v.round().clamp(256.0, 65264.0) as u16;
            d[i] = (a * 65535.0).round() as u16;
            d[i + 1] = q(yv * color::Y416_Y_SCALE + color::Y416_Y_OFF);
            d[i + 2] = q(cb * color::Y416_C_SCALE + color::Y416_C_OFF);
            d[i + 3] = q(cr * color::Y416_C_SCALE + color::Y416_C_OFF);
        }
    }
    pb.write_u16(None, &d)?;
    pb.tag_709();
    Ok(pb)
}

/// An x422 frame (10-bit codes, MSB-aligned).
pub fn x422_frame(f: usize) -> Result<PixelBuffer> {
    let pb = PixelBuffer::new(W, H, pixbuf::X422)?;
    let mut rng = Rng::new(f as u64 + 101);
    let mut yp = vec![0u16; W * H];
    let mut cp = vec![0u16; W * H];
    let q = |v: f64| (v.round().clamp(4.0, 1019.0) as u16) << 6;
    for y in 0..H {
        for x in 0..W {
            let [yv, cb, cr] = pattern(f, x, y, &mut rng, 1024.0);
            yp[y * W + x] = q(yv * color::B10_Y_SCALE + color::B10_Y_OFF);
            if x % 2 == 0 {
                let i = y * W + x;
                cp[i] = q(cb * color::B10_C_SCALE + color::B10_C_OFF);
                cp[i + 1] = q(cr * color::B10_C_SCALE + color::B10_C_OFF);
            }
        }
    }
    pb.write_u16(Some(0), &yp)?;
    pb.write_u16(Some(1), &cp)?;
    pb.tag_709();
    Ok(pb)
}

/// A 4:2:0 frame, 10-bit ('x420') or 8-bit ('420v').
pub fn yuv420_frame(f: usize, ten_bit: bool) -> Result<PixelBuffer> {
    let fmt = if ten_bit { pixbuf::X420 } else { pixbuf::V420 };
    let pb = PixelBuffer::new(W, H, fmt)?;
    let mut rng = Rng::new(f as u64 + 201);
    let mut yp = vec![0u16; W * H];
    let mut cp = vec![0u16; W * H / 2];
    let (yo, ys, co, cs, max) =
        if ten_bit { (64.0, 876.0, 512.0, 896.0, 1019.0) } else { (16.0, 219.0, 128.0, 224.0, 254.0) };
    for y in 0..H {
        for x in 0..W {
            let [yv, cb, cr] = pattern(f, x, y, &mut rng, 1024.0);
            yp[y * W + x] = (yv * ys + yo).round().clamp(1.0, max) as u16;
            if x % 2 == 0 && y % 2 == 0 {
                let i = (y / 2) * W + x;
                cp[i] = (cb * cs + co).round().clamp(1.0, max) as u16;
                cp[i + 1] = (cr * cs + co).round().clamp(1.0, max) as u16;
            }
        }
    }
    if ten_bit {
        pb.write_u16(Some(0), &yp.iter().map(|v| v << 6).collect::<Vec<_>>())?;
        pb.write_u16(Some(1), &cp.iter().map(|v| v << 6).collect::<Vec<_>>())?;
    } else {
        pb.write_u8(Some(0), &yp.iter().map(|v| *v as u8).collect::<Vec<_>>())?;
        pb.write_u8(Some(1), &cp.iter().map(|v| *v as u8).collect::<Vec<_>>())?;
    }
    pb.tag_709();
    Ok(pb)
}

pub fn color_props_709() -> Retained<NSDictionary<NSString, AnyObject>> {
    unsafe {
        Dict::new()
            .str(AVVideoColorPrimariesKey.unwrap(), AVVideoColorPrimaries_ITU_R_709_2.unwrap())
            .str(AVVideoTransferFunctionKey.unwrap(), AVVideoTransferFunction_ITU_R_709_2.unwrap())
            .str(AVVideoYCbCrMatrixKey.unwrap(), AVVideoYCbCrMatrix_ITU_R_709_2.unwrap())
            .build()
    }
}

pub fn writer_settings(
    codec: &NSString,
    extra: Option<Retained<NSDictionary<NSString, AnyObject>>>,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let mut d = unsafe {
        Dict::new()
            .str(AVVideoCodecKey.unwrap(), codec)
            .i64(AVVideoWidthKey.unwrap(), W as i64)
            .i64(AVVideoHeightKey.unwrap(), H as i64)
            .dict(AVVideoColorPropertiesKey.unwrap(), &color_props_709())
    };
    if let Some(e) = extra {
        d = d.dict(unsafe { AVVideoCompressionPropertiesKey.unwrap() }, &e);
    }
    d.build()
}

pub fn write_frames(
    path: &Path,
    file_type: &AVFileType,
    settings: &NSDictionary<NSString, AnyObject>,
    frames: &[PixelBuffer],
) -> Result<()> {
    let mut w = VideoWriter::encoding(path, file_type, settings, TIMESCALE)?;
    for (i, f) in frames.iter().enumerate() {
        w.append_pixel_buffer(f, avio::time(i as i64 * FRAME_DUR, TIMESCALE))?;
    }
    w.finish()
}

/// Frames of the natural-content clips used for codec generation.
pub const NATURAL_FRAMES: usize = 48;

/// Box-Muller standard normal from the xorshift generator.
fn gauss(rng: &mut Rng) -> f64 {
    let u1 = ((rng.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    let u2 = ((rng.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// Natural-looking content inside the codable range: a moving hue gradient, a zone
/// plate, moving soft-edged discs, 75% bars, a grey ramp, and film grain of σ = 1.5
/// 10-bit luma codes. Returns E-encoded RGB and straight alpha (soft discs and a
/// vignette, with fully transparent corners) for pixel (x, y) of frame f.
fn natural(f: usize, x: usize, y: usize) -> ([f64; 3], f64) {
    use std::f64::consts::PI;
    let t = f as f64 / NATURAL_FRAMES as f64;
    let (xf, yf) = (x as f64 / W as f64, y as f64 / H as f64);
    let mut rgb = if yf > 0.85 {
        if xf < 0.5 {
            BARS[(xf * 16.0) as usize % 8]
        } else {
            let g = (xf - 0.5) * 2.0;
            [g, g, g]
        }
    } else {
        let h = 2.0 * PI * (xf * 0.7 + yf * 0.3 + t);
        [0.5 + 0.3 * h.sin(), 0.5 + 0.3 * (h + 2.1).sin(), 0.5 + 0.3 * (h + 4.2).sin()]
    };
    // Zone plate, 360 px square, upper left.
    let (zx, zy) = (x as f64 - 300.0, y as f64 - 300.0);
    if zx.abs() < 180.0 && zy.abs() < 180.0 {
        let v = 0.5 + 0.4 * (PI * (zx * zx + zy * zy) / 720.0 + t * 4.0).cos();
        rgb = [v, v, v];
    }
    // Three moving discs with soft (8 px) edges.
    let discs = [
        (0.55 + 0.3 * (2.0 * PI * t).cos(), 0.4, 0.12, [0.85, 0.2, 0.15]),
        (0.75, 0.45 + 0.25 * (2.0 * PI * t).sin(), 0.09, [0.1, 0.7, 0.3]),
        (0.35 + 0.2 * t, 0.6, 0.15, [0.2, 0.3, 0.9]),
    ];
    for (cx, cy, r, c) in discs {
        let d = ((x as f64 - cx * W as f64).powi(2) + (y as f64 - cy * H as f64).powi(2)).sqrt() - r * H as f64;
        let cov = (0.5 - d / 8.0).clamp(0.0, 1.0);
        for k in 0..3 {
            rgb[k] = rgb[k] * (1.0 - cov) + c[k] * cov;
        }
    }
    let r = ((xf - 0.5).powi(2) + (yf - 0.5).powi(2)).sqrt();
    let alpha = (1.0 - (r - 0.35) / 0.3).clamp(0.0, 1.0);
    (rgb, alpha)
}

/// A natural-content frame in `format` ('y416' with 12-bit codes, or 'x422').
pub fn natural_frame(f: usize, format: u32, with_alpha: bool) -> Result<PixelBuffer> {
    let pb = PixelBuffer::new(W, H, format)?;
    let mut rng = Rng::new(1000 + f as u64);
    let grain = 1.5 / 876.0;
    let mut ycc = vec![[0f64; 3]; W * H];
    let mut alpha = vec![0f64; W * H];
    for y in 0..H {
        for x in 0..W {
            let (rgb, a) = natural(f, x, y);
            let mut v = color::rgb_to_ycbcr(rgb);
            v[0] += grain * gauss(&mut rng);
            ycc[y * W + x] = v;
            alpha[y * W + x] = if with_alpha { a } else { 1.0 };
        }
    }
    if format == pixbuf::Y416 {
        let mut d = vec![0u16; W * H * 4];
        for i in 0..W * H {
            let [yv, cb, cr] = ycc[i];
            // 12-bit codes, MSB-aligned (× 16), inside 16..4079.
            let q = |v: f64, off: f64, scale: f64| ((v * scale + off).round().clamp(16.0, 4079.0) as u16) << 4;
            d[i * 4] = (alpha[i] * 65535.0).round() as u16;
            d[i * 4 + 1] = q(yv, 256.0, 3504.0);
            d[i * 4 + 2] = q(cb, 2048.0, 3584.0);
            d[i * 4 + 3] = q(cr, 2048.0, 3584.0);
        }
        pb.write_u16(None, &d)?;
    } else {
        let mut yp = vec![0u16; W * H];
        let mut cp = vec![0u16; W * H];
        let q = |v: f64, off: f64, scale: f64| ((v * scale + off).round().clamp(4.0, 1019.0) as u16) << 6;
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                yp[i] = q(ycc[i][0], 64.0, 876.0);
                if x % 2 == 0 {
                    // Co-sited 4:2:2: average with the right neighbor's half weight (1-2-1 filter).
                    let l = ycc[y * W + x.saturating_sub(1)];
                    let r = ycc[y * W + (x + 1).min(W - 1)];
                    let c = ycc[i];
                    cp[i] = q((l[1] + 2.0 * c[1] + r[1]) / 4.0, 512.0, 896.0);
                    cp[i + 1] = q((l[2] + 2.0 * c[2] + r[2]) / 4.0, 512.0, 896.0);
                }
            }
        }
        pb.write_u16(Some(0), &yp)?;
        pb.write_u16(Some(1), &cp)?;
    }
    pb.tag_709();
    Ok(pb)
}

pub struct Material {
    pub dir: PathBuf,
    pub prores4444xq: PathBuf,
    pub prores4444xq_alpha: PathBuf,
    pub prores422hq: PathBuf,
    pub hevc10: PathBuf,
    pub h264: PathBuf,
    /// Natural content, 48 frames, for codec generation.
    pub natural_422hq: PathBuf,
    pub natural_4444xq: PathBuf,
    pub natural_4444xq_alpha: PathBuf,
}

/// Create (or reuse) the test material in `dir`.
pub fn ensure(dir: &Path) -> Result<Material> {
    std::fs::create_dir_all(dir).map_err(|e| crate::cf::Error(e.to_string()))?;
    let m = Material {
        dir: dir.to_path_buf(),
        prores4444xq: dir.join("src_4444xq.mov"),
        prores4444xq_alpha: dir.join("src_4444xq_alpha.mov"),
        prores422hq: dir.join("src_422hq.mov"),
        hevc10: dir.join("src_hevc10.mov"),
        h264: dir.join("src_h264.mov"),
        natural_422hq: dir.join("natural_422hq.mov"),
        natural_4444xq: dir.join("natural_4444xq.mov"),
        natural_4444xq_alpha: dir.join("natural_4444xq_alpha.mov"),
    };
    unsafe {
        if !m.prores4444xq.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| y416_frame(f, false)).collect::<Result<_>>()?;
            write_frames(
                &m.prores4444xq,
                avio::mov(),
                &writer_settings(AVVideoCodecTypeAppleProRes4444XQ.unwrap(), None),
                &frames,
            )?;
        }
        if !m.prores4444xq_alpha.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| y416_frame(f, true)).collect::<Result<_>>()?;
            write_frames(
                &m.prores4444xq_alpha,
                avio::mov(),
                &writer_settings(AVVideoCodecTypeAppleProRes4444XQ.unwrap(), None),
                &frames,
            )?;
        }
        if !m.prores422hq.exists() {
            let frames: Vec<_> = (0..FRAMES).map(x422_frame).collect::<Result<_>>()?;
            write_frames(
                &m.prores422hq,
                avio::mov(),
                &writer_settings(AVVideoCodecTypeAppleProRes422HQ.unwrap(), None),
                &frames,
            )?;
        }
        if !m.hevc10.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| yuv420_frame(f, true)).collect::<Result<_>>()?;
            let comp = Dict::new()
                .cfkey(
                    objc2_video_toolbox::kVTCompressionPropertyKey_ProfileLevel,
                    &*crate::vt::cf_str(objc2_video_toolbox::kVTProfileLevel_HEVC_Main10_AutoLevel),
                )
                .i64(AVVideoAverageBitRateKey.unwrap(), 40_000_000)
                .build();
            write_frames(&m.hevc10, avio::mov(), &writer_settings(AVVideoCodecTypeHEVC.unwrap(), Some(comp)), &frames)?;
        }
        if !m.h264.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| yuv420_frame(f, false)).collect::<Result<_>>()?;
            let comp = Dict::new().i64(AVVideoAverageBitRateKey.unwrap(), 40_000_000).build();
            write_frames(&m.h264, avio::mov(), &writer_settings(AVVideoCodecTypeH264.unwrap(), Some(comp)), &frames)?;
        }
        for (path, codec, format, alpha) in [
            (&m.natural_422hq, AVVideoCodecTypeAppleProRes422HQ.unwrap(), pixbuf::X422, false),
            (&m.natural_4444xq, AVVideoCodecTypeAppleProRes4444XQ.unwrap(), pixbuf::Y416, false),
            (&m.natural_4444xq_alpha, AVVideoCodecTypeAppleProRes4444XQ.unwrap(), pixbuf::Y416, true),
        ] {
            if !path.exists() {
                let frames: Vec<_> =
                    (0..NATURAL_FRAMES).map(|f| natural_frame(f, format, alpha)).collect::<Result<_>>()?;
                write_frames(path, avio::mov(), &writer_settings(codec, None), &frames)?;
            }
        }
    }
    Ok(m)
}
