//! Decoded frames from either backend: plane layout, the section 13
//! normalization constants, a framemd5-compatible hash, and a CPU reference of
//! the normalization (verification only; the decode path never reads pixels on
//! the CPU).

use crate::apple::*;
use crate::ff::FfFrame;
use crate::vt;
use anyhow::{Result, bail};
use md5::{Digest, Md5};

pub enum Pixels {
    /// IOSurface-backed CVPixelBuffer from VideoToolbox.
    Vt(Cf),
    /// FFmpeg frame in Metal-shared pool memory.
    Ff(FfFrame),
}

pub struct Decoded {
    pub pts: i64,
    pub px: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum PlaneFormat {
    R8,
    Rg8,
    R16,
    Rg16,
    Rgba16,
}

impl PlaneFormat {
    pub fn bytes(self) -> usize {
        match self {
            PlaneFormat::R8 => 1,
            PlaneFormat::Rg8 | PlaneFormat::R16 => 2,
            PlaneFormat::Rg16 => 4,
            PlaneFormat::Rgba16 => 8,
        }
    }
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            PlaneFormat::R8 => wgpu::TextureFormat::R8Unorm,
            PlaneFormat::Rg8 => wgpu::TextureFormat::Rg8Unorm,
            PlaneFormat::R16 => wgpu::TextureFormat::R16Unorm,
            PlaneFormat::Rg16 => wgpu::TextureFormat::Rg16Unorm,
            PlaneFormat::Rgba16 => wgpu::TextureFormat::Rgba16Unorm,
        }
    }
    pub fn metal(self) -> objc2_metal::MTLPixelFormat {
        use objc2_metal::MTLPixelFormat as M;
        match self {
            PlaneFormat::R8 => M::R8Unorm,
            PlaneFormat::Rg8 => M::RG8Unorm,
            PlaneFormat::R16 => M::R16Unorm,
            PlaneFormat::Rg16 => M::RG16Unorm,
            PlaneFormat::Rgba16 => M::RGBA16Unorm,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Kind {
    /// Y plane + interleaved CbCr plane (VideoToolbox 420v/x420/x422/...).
    Biplanar = 0,
    /// Y, Cb, Cr planes (FFmpeg planar).
    Planar = 1,
    /// One RGBA16Unorm texture holding A, Y′, Cb, Cr (y416).
    PackedAycbcr = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum AlphaSrc {
    None,
    /// Separate full-resolution alpha plane at this index.
    Plane(usize),
    /// Channel R of the packed y416 texture.
    Packed,
}

/// Normalization: code = round(unorm × code_max) / shift; Y′ = (code − y_off) / y_div,
/// C = (code − c_off) / c_div, A = (acode − a_off) / a_div.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Norm {
    pub code_max: f32,
    pub shift: f32,
    pub y_off: f32,
    pub y_div: f32,
    pub c_off: f32,
    pub c_div: f32,
    pub a_code_max: f32,
    pub a_shift: f32,
    pub a_off: f32,
    pub a_div: f32,
}

impl Norm {
    /// n-bit codes; limited range uses 16·2ⁿ⁻⁸ / 219·2ⁿ⁻⁸ (luma) and 128·2ⁿ⁻⁸ /
    /// 224·2ⁿ⁻⁸ (chroma); full range uses 2ⁿ − 1 with chroma centered at 2ⁿ⁻¹.
    pub fn new(n: u32, full: bool, code_max: f32, shift: f32, alpha_bits: u32, a_code_max: f32) -> Norm {
        let s = (1u64 << (n - 8)) as f32;
        let (y_off, y_div, c_off, c_div) = if full {
            let m = ((1u64 << n) - 1) as f32;
            (0.0, m, (1u64 << (n - 1)) as f32, m)
        } else {
            (16.0 * s, 219.0 * s, 128.0 * s, 224.0 * s)
        };
        Norm {
            code_max,
            shift,
            y_off,
            y_div,
            c_off,
            c_div,
            a_code_max,
            a_shift: 1.0,
            a_off: 0.0,
            a_div: ((1u64 << alpha_bits) - 1) as f32,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PlaneDesc {
    pub format: PlaneFormat,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FrameDesc {
    pub kind: Kind,
    pub alpha: AlphaSrc,
    pub planes: Vec<PlaneDesc>,
    pub log2_cw: u32,
    pub log2_ch: u32,
    pub width: usize,
    pub height: usize,
    pub norm: Norm,
    pub label: String,
}

pub struct PlaneView {
    pub ptr: *const u8,
    pub stride: usize,
    pub desc: PlaneDesc,
}

impl Decoded {
    pub fn desc(&self) -> Result<FrameDesc> {
        match &self.px {
            Pixels::Vt(pb) => vt_desc(pb),
            Pixels::Ff(f) => ff_desc(f),
        }
    }

    /// Runs `f` with CPU views of the planes (CVPixelBuffer locked read-only).
    pub fn with_planes<R>(&self, f: impl FnOnce(&[PlaneView]) -> R) -> Result<R> {
        let d = self.desc()?;
        match &self.px {
            Pixels::Vt(pb) => unsafe {
                let p = pb.as_mut_ptr();
                if CVPixelBufferLockBaseAddress(p, kCVPixelBufferLock_ReadOnly) != 0 {
                    bail!("lock failed");
                }
                let planar = CVPixelBufferIsPlanar(p) != 0;
                let views: Vec<PlaneView> = d
                    .planes
                    .iter()
                    .enumerate()
                    .map(|(i, pd)| PlaneView {
                        ptr: if planar { CVPixelBufferGetBaseAddressOfPlane(p, i) } else { CVPixelBufferGetBaseAddress(p) }
                            as *const u8,
                        stride: if planar { CVPixelBufferGetBytesPerRowOfPlane(p, i) } else { CVPixelBufferGetBytesPerRow(p) },
                        desc: pd.clone(),
                    })
                    .collect();
                let r = f(&views);
                CVPixelBufferUnlockBaseAddress(p, kCVPixelBufferLock_ReadOnly);
                Ok(r)
            },
            Pixels::Ff(fr) => {
                let views: Vec<PlaneView> = d
                    .planes
                    .iter()
                    .zip(&fr.layout.planes)
                    .map(|(pd, pl)| PlaneView {
                        ptr: unsafe { fr.slot.ptr().add(pl.offset) },
                        stride: pl.stride,
                        desc: pd.clone(),
                    })
                    .collect();
                Ok(f(&views))
            }
        }
    }

    /// MD5 of the visible planes packed without padding: the rawvideo layout
    /// FFmpeg's framemd5 hashes for the equivalent pixel format (nv12, p010le,
    /// p210le, ayuv64le, yuv420p, ...).
    pub fn md5(&self) -> Result<String> {
        self.with_planes(|planes| {
            let mut h = Md5::new();
            for p in planes {
                let row = p.desc.width * p.desc.format.bytes();
                for y in 0..p.desc.height {
                    h.update(unsafe { std::slice::from_raw_parts(p.ptr.add(y * p.stride), row) });
                }
            }
            format!("{:x}", h.finalize())
        })
    }

    /// CPU reference of the GPU normalization, in f64 rounded to f32:
    /// (luma-resolution [Y′, A], chroma-resolution [Cb, Cr]).
    pub fn reference(&self) -> Result<(Vec<[f32; 2]>, Vec<[f32; 2]>)> {
        let d = self.desc()?;
        self.with_planes(|pl| {
            let n = d.norm;
            let rd = |p: &PlaneView, x: usize, y: usize, ch: usize| -> f64 {
                let b = p.desc.format.bytes();
                let comps = match p.desc.format {
                    PlaneFormat::R8 | PlaneFormat::R16 => 1,
                    PlaneFormat::Rg8 | PlaneFormat::Rg16 => 2,
                    PlaneFormat::Rgba16 => 4,
                };
                let cb = b / comps;
                let off = y * p.stride + x * b + ch * cb;
                unsafe {
                    if cb == 1 {
                        *p.ptr.add(off) as f64
                    } else {
                        u16::from_le_bytes([*p.ptr.add(off), *p.ptr.add(off + 1)]) as f64
                    }
                }
            };
            let f = |v: f64, shift: f32, off: f32, div: f32| ((v / shift as f64 - off as f64) / div as f64) as f32;
            let (w, h) = (d.width, d.height);
            let cw = (w + (1 << d.log2_cw) - 1) >> d.log2_cw;
            let chh = (h + (1 << d.log2_ch) - 1) >> d.log2_ch;
            let mut ya = vec![[0f32; 2]; w * h];
            let mut cc = vec![[0f32; 2]; cw * chh];
            for y in 0..h {
                for x in 0..w {
                    let (yy, a) = match d.kind {
                        Kind::PackedAycbcr => (
                            f(rd(&pl[0], x, y, 1), n.shift, n.y_off, n.y_div),
                            f(rd(&pl[0], x, y, 0), n.a_shift, n.a_off, n.a_div),
                        ),
                        _ => (f(rd(&pl[0], x, y, 0), n.shift, n.y_off, n.y_div), 1.0),
                    };
                    let a = match d.alpha {
                        AlphaSrc::Plane(i) => f(rd(&pl[i], x, y, 0), n.a_shift, n.a_off, n.a_div),
                        _ => a,
                    };
                    ya[y * w + x] = [yy, a];
                }
            }
            for y in 0..chh {
                for x in 0..cw {
                    let (cb, cr) = match d.kind {
                        Kind::PackedAycbcr => (rd(&pl[0], x, y, 2), rd(&pl[0], x, y, 3)),
                        Kind::Biplanar => (rd(&pl[1], x, y, 0), rd(&pl[1], x, y, 1)),
                        Kind::Planar => (rd(&pl[1], x, y, 0), rd(&pl[2], x, y, 0)),
                    };
                    cc[y * cw + x] = [f(cb, n.shift, n.c_off, n.c_div), f(cr, n.shift, n.c_off, n.c_div)];
                }
            }
            (ya, cc)
        })
    }
}

fn vt_desc(pb: &Cf) -> Result<FrameDesc> {
    unsafe {
        let p = pb.as_mut_ptr();
        let f = CVPixelBufferGetPixelFormatType(p);
        let (w, h) = (CVPixelBufferGetWidth(p), CVPixelBufferGetHeight(p));
        let plane = |i: usize, format: PlaneFormat| PlaneDesc {
            format,
            width: CVPixelBufferGetWidthOfPlane(p, i),
            height: CVPixelBufferGetHeightOfPlane(p, i),
        };
        use PlaneFormat::*;
        let (kind, alpha, planes, cw, ch, norm) = match f {
            vt::F_420V | vt::F_420F => (
                Kind::Biplanar,
                AlphaSrc::None,
                vec![plane(0, R8), plane(1, Rg8)],
                1,
                1,
                Norm::new(8, f == vt::F_420F, 255.0, 1.0, 8, 255.0),
            ),
            vt::F_V0A8 => (
                Kind::Biplanar,
                AlphaSrc::Plane(2),
                vec![plane(0, R8), plane(1, Rg8), plane(2, R8)],
                1,
                1,
                Norm::new(8, false, 255.0, 1.0, 8, 255.0),
            ),
            vt::F_X420 | vt::F_XF20 | vt::F_X422 | vt::F_XF22 => (
                Kind::Biplanar,
                AlphaSrc::None,
                vec![plane(0, R16), plane(1, Rg16)],
                1,
                if f == vt::F_X422 || f == vt::F_XF22 { 0 } else { 1 },
                // MSB-aligned 10-bit in 16: c = v / 64.
                Norm::new(10, f == vt::F_XF20 || f == vt::F_XF22, 65535.0, 64.0, 8, 255.0),
            ),
            vt::F_Y416 => (
                Kind::PackedAycbcr,
                AlphaSrc::Packed,
                vec![PlaneDesc { format: Rgba16, width: w, height: h }],
                0,
                0,
                Norm::new(16, false, 65535.0, 1.0, 16, 65535.0),
            ),
            other => bail!("unexpected VideoToolbox output {}", fourcc(other)),
        };
        Ok(FrameDesc { kind, alpha, planes, log2_cw: cw, log2_ch: ch, width: w, height: h, norm, label: fourcc(f) })
    }
}

fn ff_desc(fr: &FfFrame) -> Result<FrameDesc> {
    use ffmpeg_sys_the_third as ff;
    let l = &fr.layout;
    let n = l.depth as u32;
    let fmt = if n > 8 { PlaneFormat::R16 } else { PlaneFormat::R8 };
    let code_max = if n > 8 { 65535.0 } else { 255.0 };
    let full = unsafe { (*fr.frame).color_range == ff::AVColorRange::JPEG };
    let planes: Vec<PlaneDesc> =
        l.planes.iter().map(|p| PlaneDesc { format: fmt, width: p.width, height: p.height }).collect();
    let name = unsafe {
        std::ffi::CStr::from_ptr(ff::av_get_pix_fmt_name(ff::AVPixelFormat(l.format))).to_string_lossy().into_owned()
    };
    let (kind, alpha) = match planes.len() {
        1 => (Kind::Planar, AlphaSrc::None),
        3 => (Kind::Planar, AlphaSrc::None),
        4 => (Kind::Planar, AlphaSrc::Plane(3)),
        k => bail!("{k}-plane layout {name} not handled"),
    };
    if planes.len() == 1 {
        bail!("grey frames ({name}) not handled by the spike's shader");
    }
    Ok(FrameDesc {
        kind,
        alpha,
        width: planes[0].width,
        height: planes[0].height,
        log2_cw: l.planes[1].log2_w as u32,
        log2_ch: l.planes[1].log2_h as u32,
        planes,
        norm: Norm::new(n, full, code_max, 1.0, n, code_max),
        label: name,
    })
}
