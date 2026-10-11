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
            let a = if alpha_ramp {
                if y % 270 < 30 { 0.0 } else { x as f64 / (W - 1) as f64 }
            } else {
                1.0
            };
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
    let (yo, ys, co, cs, max) = if ten_bit { (64.0, 876.0, 512.0, 896.0, 1019.0) } else { (16.0, 219.0, 128.0, 224.0, 254.0) };
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

pub fn writer_settings(codec: &NSString, extra: Option<Retained<NSDictionary<NSString, AnyObject>>>) -> Retained<NSDictionary<NSString, AnyObject>> {
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

pub fn write_frames(path: &Path, file_type: &AVFileType, settings: &NSDictionary<NSString, AnyObject>, frames: &[PixelBuffer]) -> Result<()> {
    let mut w = VideoWriter::encoding(path, file_type, settings)?;
    for (i, f) in frames.iter().enumerate() {
        w.append_pixel_buffer(f, avio::time(i as i64 * FRAME_DUR, TIMESCALE))?;
    }
    w.finish()
}

pub struct Material {
    pub dir: PathBuf,
    pub prores4444xq: PathBuf,
    pub prores4444xq_alpha: PathBuf,
    pub prores422hq: PathBuf,
    pub hevc10: PathBuf,
    pub h264: PathBuf,
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
    };
    unsafe {
        if !m.prores4444xq.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| y416_frame(f, false)).collect::<Result<_>>()?;
            write_frames(&m.prores4444xq, avio::mov(), &writer_settings(AVVideoCodecTypeAppleProRes4444XQ.unwrap(), None), &frames)?;
        }
        if !m.prores4444xq_alpha.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| y416_frame(f, true)).collect::<Result<_>>()?;
            write_frames(&m.prores4444xq_alpha, avio::mov(), &writer_settings(AVVideoCodecTypeAppleProRes4444XQ.unwrap(), None), &frames)?;
        }
        if !m.prores422hq.exists() {
            let frames: Vec<_> = (0..FRAMES).map(x422_frame).collect::<Result<_>>()?;
            write_frames(&m.prores422hq, avio::mov(), &writer_settings(AVVideoCodecTypeAppleProRes422HQ.unwrap(), None), &frames)?;
        }
        if !m.hevc10.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| yuv420_frame(f, true)).collect::<Result<_>>()?;
            let comp = Dict::new()
                .cfkey(objc2_video_toolbox::kVTCompressionPropertyKey_ProfileLevel, &*crate::vt::cf_str(objc2_video_toolbox::kVTProfileLevel_HEVC_Main10_AutoLevel))
                .i64(AVVideoAverageBitRateKey.unwrap(), 40_000_000)
                .build();
            write_frames(&m.hevc10, avio::mov(), &writer_settings(AVVideoCodecTypeHEVC.unwrap(), Some(comp)), &frames)?;
        }
        if !m.h264.exists() {
            let frames: Vec<_> = (0..FRAMES).map(|f| yuv420_frame(f, false)).collect::<Result<_>>()?;
            let comp = Dict::new().i64(AVVideoAverageBitRateKey.unwrap(), 40_000_000).build();
            write_frames(&m.h264, avio::mov(), &writer_settings(AVVideoCodecTypeH264.unwrap(), Some(comp)), &frames)?;
        }
    }
    Ok(m)
}
