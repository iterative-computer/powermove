//! Decoded frames from every backend: plane layout, the section 13
//! normalization constants, CPU views (verification only: the decode path
//! never reads pixels on the CPU), a framemd5-compatible hash, and a CPU
//! reference of the normalization.

use crate::apple::cf::*;
use crate::error::{MediaError, Result};
use crate::pool::SlotRef;
use pm_protocol::color::{ColorDescription, Range, SampleAlignment, Subsampling, normalization};
use pm_protocol::interop::{DecodedLayout, SurfaceFormat};
use serde::Serialize;
use std::sync::Arc;

/// The texture format of one imported plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum PlaneFormat {
    R8,
    Rg8,
    R16,
    Rg16,
    Rgba8,
    Rgba16,
    Rgba16Float,
    Rgba32Float,
}

impl PlaneFormat {
    pub const fn bytes(self) -> usize {
        match self {
            PlaneFormat::R8 => 1,
            PlaneFormat::Rg8 | PlaneFormat::R16 => 2,
            PlaneFormat::Rg16 | PlaneFormat::Rgba8 => 4,
            PlaneFormat::Rgba16 | PlaneFormat::Rgba16Float => 8,
            PlaneFormat::Rgba32Float => 16,
        }
    }
    pub const fn components(self) -> usize {
        match self {
            PlaneFormat::R8 | PlaneFormat::R16 => 1,
            PlaneFormat::Rg8 | PlaneFormat::Rg16 => 2,
            _ => 4,
        }
    }
}

/// How samples are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Layout {
    /// Y′ plane plus interleaved CbCr (VideoToolbox `420v`, `420f`, `x420`,
    /// `xf20`, `x422`, `xf22`).
    Biplanar,
    /// Biplanar plus a full-resolution 8-bit alpha plane (`v0a8`, HEVC with alpha).
    BiplanarAlpha,
    /// Y′, Cb, Cr, then A planes, LSB-aligned (FFmpeg and dav1d).
    Planar,
    /// G, B, R, then A planes, LSB-aligned (FFmpeg `gbrp*`).
    PlanarRgb,
    /// `y416`: one RGBA16 texel per pixel holding A, Y′, Cb, Cr.
    Ayuv16,
    /// `v216`: one RGBA16 texel per two pixels holding Cb, Y′0, Cr, Y′1
    /// (co-sited 4:2:2, 16-bit; section 26 amendment 6).
    Cbycry16,
    /// Packed RGBA (image readers); see `PixelFormat::rgba_premultiplied`.
    Rgba,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PlaneDesc {
    pub format: PlaneFormat,
    /// Texels.
    pub width: usize,
    pub height: usize,
}

/// Everything an importer needs to sample a decoded frame correctly.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PixelFormat {
    pub layout: Layout,
    /// Code bits per sample.
    pub bits: u8,
    pub alignment: SampleAlignment,
    pub range: Range,
    pub log2_chroma_w: u8,
    pub log2_chroma_h: u8,
    /// Alpha code bits (0 when there is no alpha).
    pub alpha_bits: u8,
    pub planes: Vec<PlaneDesc>,
    pub width: usize,
    pub height: usize,
    /// Pixel format name (VideoToolbox four-character code or FFmpeg name).
    pub label: String,
    /// Packed RGBA only: the samples are premultiplied.
    pub rgba_premultiplied: bool,
}

/// Section 13 normalization of a format's raw sample codes (the integer read
/// from memory): Y′ = (v − y_off) / y_div, C = (v − c_off) / c_div,
/// A = v / a_div.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Norm {
    pub y_off: f64,
    pub y_div: f64,
    pub c_off: f64,
    pub c_div: f64,
    pub a_div: f64,
}

impl PixelFormat {
    pub fn has_alpha(&self) -> bool {
        self.alpha_bits > 0
    }

    pub fn subsampling(&self) -> Subsampling {
        match (self.log2_chroma_w, self.log2_chroma_h) {
            (0, 0) => Subsampling::S444,
            (1, 0) => Subsampling::S422,
            _ => Subsampling::S420,
        }
    }

    /// The section 13 constants for this format, from contract 4's
    /// reference (`pm_protocol::color::normalization`).
    pub fn norm(&self) -> Norm {
        let n = match self.layout {
            Layout::PlanarRgb | Layout::Rgba => {
                let m = ((1u64 << self.bits) - 1) as f64;
                pm_protocol::color::Normalization { luma_offset: 0.0, luma_scale: m, chroma_offset: 0.0, chroma_scale: m }
            }
            _ => normalization(self.bits, self.alignment, self.range),
        };
        Norm {
            y_off: n.luma_offset,
            y_div: n.luma_scale,
            c_off: n.chroma_offset,
            c_div: n.chroma_scale,
            a_div: if self.alpha_bits > 0 { ((1u64 << self.alpha_bits) - 1) as f64 } else { 1.0 },
        }
    }

    /// The contract 3 layout, where one exists. `v216` and `v0a8` have no
    /// variant in contract 3 version 0 (see docs/engine/media.md).
    pub fn decoded_layout(&self) -> Option<DecodedLayout> {
        let full_range = self.range == Range::Full;
        Some(match (self.layout, self.bits, self.log2_chroma_w, self.log2_chroma_h) {
            (Layout::Biplanar, 8, 1, 1) => DecodedLayout::Biplanar420_8 { full_range },
            (Layout::Biplanar, 8, 1, 0) => DecodedLayout::Biplanar422_8 { full_range },
            (Layout::Biplanar, 10, 1, 1) => DecodedLayout::Biplanar420_10 { full_range },
            (Layout::Biplanar, 10, 1, 0) => DecodedLayout::Biplanar422_10 { full_range },
            (Layout::Ayuv16, ..) => DecodedLayout::Ayuv4444_16,
            (Layout::Planar, bits, ..) => {
                DecodedLayout::PlanarLsb { bits, subsampling: self.subsampling(), alpha: self.has_alpha() }
            }
            (Layout::Rgba, ..) => DecodedLayout::Rgba {
                format: match self.planes[0].format {
                    PlaneFormat::Rgba8 => SurfaceFormat::Bgra8Unorm,
                    PlaneFormat::Rgba16 => SurfaceFormat::Rgba16Unorm,
                    PlaneFormat::Rgba16Float => SurfaceFormat::Rgba16Float,
                    _ => SurfaceFormat::Rgba32Float,
                },
            },
            _ => return None,
        })
    }

    /// Bytes the planes occupy (visible area), for cache accounting.
    pub fn visible_bytes(&self) -> usize {
        self.planes.iter().map(|p| p.width * p.height * p.format.bytes()).sum()
    }
}

/// A software-decoded frame whose planes live in pool slots.
pub struct PoolFrame {
    pub planes: Vec<PoolPlane>,
    /// Keeps the decoder's own reference alive (an AVFrame, a Dav1dPicture).
    pub owner: Option<Box<dyn Send + Sync>>,
    /// The pixels were copied into the pool (a decoder without direct
    /// rendering, or a conversion): not zero-copy.
    pub copied: bool,
}

/// One plane in pool memory.
#[derive(Clone)]
pub struct PoolPlane {
    pub slot: SlotRef,
    /// Byte offset of the plane in its slot.
    pub offset: usize,
    pub stride: usize,
    /// The address the decoder reported for the plane (AVFrame.data,
    /// Dav1dPicture.data), to verify that a texture aliases what the decoder wrote.
    pub decoder_ptr: usize,
}

impl PoolPlane {
    pub fn ptr(&self) -> *const u8 {
        unsafe { self.slot.ptr().add(self.offset) }
    }
}

/// The memory behind a decoded frame.
#[derive(Clone)]
pub enum Pixels {
    /// An IOSurface-backed CVPixelBuffer from VideoToolbox (or ImageIO).
    CoreVideo(Cf),
    /// Planes in pool memory (FFmpeg, dav1d, image readers).
    Pool(Arc<PoolFrame>),
}

/// A decoded frame. Cloning shares the pixels.
#[derive(Clone)]
pub struct DecodedFrame {
    /// Composition timestamp of the sample in the media timebase.
    pub pts: i64,
    pub format: Arc<PixelFormat>,
    pub pixels: Pixels,
    /// The source's interpreted color, when the decode service knows it.
    pub color: Option<ColorDescription>,
}

impl std::fmt::Debug for DecodedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DecodedFrame(pts {}, {} {}x{})", self.pts, self.format.label, self.format.width, self.format.height)
    }
}

/// A CPU view of one plane (verification).
pub struct PlaneView<'a> {
    pub ptr: *const u8,
    pub stride: usize,
    pub desc: &'a PlaneDesc,
}

impl DecodedFrame {
    /// Whether the planes were copied on the way into GPU-shareable memory.
    pub fn copied(&self) -> bool {
        matches!(&self.pixels, Pixels::Pool(p) if p.copied)
    }

    /// Bytes this frame holds (cache accounting): the CVPixelBuffer's data
    /// size, or the pool slots it references.
    pub fn bytes(&self) -> usize {
        match &self.pixels {
            Pixels::CoreVideo(pb) => unsafe { CVPixelBufferGetDataSize(pb.as_mut_ptr()) },
            Pixels::Pool(p) => {
                let mut seen: Vec<usize> = p.planes.iter().map(|pl| pl.slot.id()).collect();
                seen.sort_unstable();
                seen.dedup();
                p.planes
                    .iter()
                    .filter(|pl| {
                        if let Some(i) = seen.iter().position(|&s| s == pl.slot.id()) {
                            seen.remove(i);
                            true
                        } else {
                            false
                        }
                    })
                    .map(|pl| pl.slot.len())
                    .sum()
            }
        }
    }

    /// Runs `f` with CPU views of the planes (CVPixelBuffers locked read-only).
    pub fn with_planes<R>(&self, f: impl FnOnce(&[PlaneView<'_>]) -> R) -> Result<R> {
        let d = &self.format;
        match &self.pixels {
            Pixels::CoreVideo(pb) => unsafe {
                let p = pb.as_mut_ptr();
                let st = CVPixelBufferLockBaseAddress(p, kCVPixelBufferLock_ReadOnly);
                if st != 0 {
                    return Err(MediaError::CoreVideo { what: "lock".into(), status: st });
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
                        desc: pd,
                    })
                    .collect();
                let r = f(&views);
                CVPixelBufferUnlockBaseAddress(p, kCVPixelBufferLock_ReadOnly);
                Ok(r)
            },
            Pixels::Pool(fr) => {
                let views: Vec<PlaneView> = d
                    .planes
                    .iter()
                    .zip(&fr.planes)
                    .map(|(pd, pl)| PlaneView { ptr: pl.ptr(), stride: pl.stride, desc: pd })
                    .collect();
                Ok(f(&views))
            }
        }
    }

    /// MD5 of the visible planes packed without padding: the rawvideo layout
    /// FFmpeg's framemd5 hashes for the equivalent pixel format (nv12,
    /// p010le, p210le, ayuv64le, yuv420p, ...).
    pub fn md5_hex(&self) -> Result<String> {
        use md5::Digest as _;
        self.with_planes(|planes| {
            let mut h = md5::Md5::new();
            for p in planes {
                let row = p.desc.width * p.desc.format.bytes();
                for y in 0..p.desc.height {
                    h.update(unsafe { std::slice::from_raw_parts(p.ptr.add(y * p.stride), row) });
                }
            }
            h.finalize().iter().map(|b| format!("{b:02x}")).collect()
        })
    }

    /// Raw sample code at (plane, x, y, component).
    fn code(p: &PlaneView<'_>, x: usize, y: usize, c: usize) -> f64 {
        let b = p.desc.format.bytes();
        let cb = b / p.desc.format.components();
        let off = y * p.stride + x * b + c * cb;
        unsafe {
            if cb == 1 {
                *p.ptr.add(off) as f64
            } else {
                u16::from_le_bytes([*p.ptr.add(off), *p.ptr.add(off + 1)]) as f64
            }
        }
    }

    /// CPU reference of the normalization in f64, rounded to f32:
    /// (luma-resolution [Y′, A], chroma-resolution [Cb, Cr]). RGB layouts
    /// return [G, A] and [B, R] in the same positions (no matrix).
    pub fn reference(&self) -> Result<(Vec<[f32; 2]>, Vec<[f32; 2]>)> {
        let d = self.format.clone();
        let n = d.norm();
        self.with_planes(|pl| {
            let y = |v: f64| ((v - n.y_off) / n.y_div) as f32;
            let c = |v: f64| ((v - n.c_off) / n.c_div) as f32;
            let a = |v: f64| (v / n.a_div) as f32;
            let (w, h) = (d.width, d.height);
            let cw = crate::pool::ceil_rshift(w, d.log2_chroma_w);
            let chh = crate::pool::ceil_rshift(h, d.log2_chroma_h);
            let mut ya = vec![[0f32; 2]; w * h];
            let mut cc = vec![[0f32; 2]; cw * chh];
            for yy in 0..h {
                for x in 0..w {
                    let (lv, av) = match d.layout {
                        Layout::Ayuv16 => (y(Self::code(&pl[0], x, yy, 1)), a(Self::code(&pl[0], x, yy, 0))),
                        Layout::Cbycry16 => (y(Self::code(&pl[0], x / 2, yy, 1 + 2 * (x & 1))), 1.0),
                        Layout::BiplanarAlpha => (y(Self::code(&pl[0], x, yy, 0)), a(Self::code(&pl[2], x, yy, 0))),
                        Layout::Planar | Layout::PlanarRgb if d.has_alpha() => {
                            (y(Self::code(&pl[0], x, yy, 0)), a(Self::code(&pl[3], x, yy, 0)))
                        }
                        Layout::Rgba => (y(Self::code(&pl[0], x, yy, 1)), a(Self::code(&pl[0], x, yy, 3))),
                        _ => (y(Self::code(&pl[0], x, yy, 0)), 1.0),
                    };
                    ya[yy * w + x] = [lv, av];
                }
            }
            for yy in 0..chh {
                for x in 0..cw {
                    let (cb, cr) = match d.layout {
                        Layout::Ayuv16 => (Self::code(&pl[0], x, yy, 2), Self::code(&pl[0], x, yy, 3)),
                        Layout::Cbycry16 => (Self::code(&pl[0], x, yy, 0), Self::code(&pl[0], x, yy, 2)),
                        Layout::Biplanar | Layout::BiplanarAlpha => {
                            (Self::code(&pl[1], x, yy, 0), Self::code(&pl[1], x, yy, 1))
                        }
                        Layout::Planar | Layout::PlanarRgb => (Self::code(&pl[1], x, yy, 0), Self::code(&pl[2], x, yy, 0)),
                        Layout::Rgba => (Self::code(&pl[0], x, yy, 2), Self::code(&pl[0], x, yy, 0)),
                    };
                    cc[yy * cw + x] = [c(cb), c(cr)];
                }
            }
            (ya, cc)
        })
    }
}

/// VideoToolbox pixel formats the engine requests.
pub mod fourcc {
    use crate::apple::cf::fcc;
    pub const F_420V: u32 = fcc(b"420v");
    pub const F_420F: u32 = fcc(b"420f");
    pub const F_X420: u32 = fcc(b"x420");
    pub const F_XF20: u32 = fcc(b"xf20");
    pub const F_X422: u32 = fcc(b"x422");
    pub const F_XF22: u32 = fcc(b"xf22");
    pub const F_Y416: u32 = fcc(b"y416");
    pub const F_V216: u32 = fcc(b"v216");
    pub const F_V0A8: u32 = fcc(b"v0a8");
}

/// The pixel format of a VideoToolbox (or other CoreVideo) buffer.
pub fn corevideo_format(pb: &Cf) -> Result<PixelFormat> {
    use fourcc::*;
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
        let base = |layout, bits, alignment, full: bool, cw, ch, alpha_bits, planes| PixelFormat {
            layout,
            bits,
            alignment,
            range: if full { Range::Full } else { Range::Limited },
            log2_chroma_w: cw,
            log2_chroma_h: ch,
            alpha_bits,
            planes,
            width: w,
            height: h,
            label: fourcc(f),
            rgba_premultiplied: false,
        };
        Ok(match f {
            F_420V | F_420F => {
                base(Layout::Biplanar, 8, SampleAlignment::Lsb, f == F_420F, 1, 1, 0, vec![plane(0, R8), plane(1, Rg8)])
            }
            F_V0A8 => base(
                Layout::BiplanarAlpha,
                8,
                SampleAlignment::Lsb,
                false,
                1,
                1,
                8,
                vec![plane(0, R8), plane(1, Rg8), plane(2, R8)],
            ),
            F_X420 | F_XF20 | F_X422 | F_XF22 => base(
                Layout::Biplanar,
                10,
                SampleAlignment::MsbIn16,
                f == F_XF20 || f == F_XF22,
                1,
                if f == F_X422 || f == F_XF22 { 0 } else { 1 },
                0,
                vec![plane(0, R16), plane(1, Rg16)],
            ),
            F_Y416 => base(
                Layout::Ayuv16,
                16,
                SampleAlignment::Full16,
                false,
                0,
                0,
                16,
                vec![PlaneDesc { format: Rgba16, width: w, height: h }],
            ),
            F_V216 => base(
                Layout::Cbycry16,
                16,
                SampleAlignment::Full16,
                false,
                1,
                0,
                0,
                vec![PlaneDesc { format: Rgba16, width: w.div_ceil(2), height: h }],
            ),
            other => return Err(MediaError::unsupported(format!("CoreVideo pixel format {}", fourcc(other)))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(layout: Layout, bits: u8, alignment: SampleAlignment, range: Range) -> PixelFormat {
        PixelFormat {
            layout,
            bits,
            alignment,
            range,
            log2_chroma_w: 1,
            log2_chroma_h: 1,
            alpha_bits: 0,
            planes: vec![],
            width: 2,
            height: 2,
            label: String::new(),
            rgba_premultiplied: false,
        }
    }

    #[test]
    fn section_13_constants() {
        // x420: c = v / 64; Y′ = (c − 64) / 876.
        let n = fmt(Layout::Biplanar, 10, SampleAlignment::MsbIn16, Range::Limited).norm();
        assert_eq!((n.y_off, n.y_div, n.c_off, n.c_div), (4096.0, 56064.0, 32768.0, 57344.0));
        // y416 and v216: Y′ = (v − 4096) / 56064, C = (v − 32768) / 57344.
        let n = fmt(Layout::Ayuv16, 16, SampleAlignment::Full16, Range::Limited).norm();
        assert_eq!((n.y_off, n.y_div, n.c_off, n.c_div), (4096.0, 56064.0, 32768.0, 57344.0));
        // FFmpeg LSB 10-bit: 16·4, 219·4, 128·4, 224·4.
        let n = fmt(Layout::Planar, 10, SampleAlignment::Lsb, Range::Limited).norm();
        assert_eq!((n.y_off, n.y_div, n.c_off, n.c_div), (64.0, 876.0, 512.0, 896.0));
        // Full range 8-bit: 0..255, chroma centered at 128.
        let n = fmt(Layout::Planar, 8, SampleAlignment::Lsb, Range::Full).norm();
        assert_eq!((n.y_off, n.y_div, n.c_off, n.c_div), (0.0, 255.0, 128.0, 255.0));
    }

    #[test]
    fn contract_layouts() {
        let f = fmt(Layout::Biplanar, 10, SampleAlignment::MsbIn16, Range::Limited);
        assert_eq!(f.decoded_layout(), Some(DecodedLayout::Biplanar420_10 { full_range: false }));
        assert_eq!(fmt(Layout::Cbycry16, 16, SampleAlignment::Full16, Range::Limited).decoded_layout(), None);
    }
}
