//! VideoToolbox encoders: VTCopyVideoEncoderList, supported properties, native input
//! formats (the encoder-input buffer, section 17), the bit depth of HEVC with alpha,
//! HEVC-with-alpha quality against AVFoundation's preset, and playback of the results
//! through AVPlayerItemVideoOutput.
//!
//! HEVC with alpha at Main10 uses the undocumented tri-planar 'x0a8' format (10-bit
//! 4:2:0 plus an 8-bit alpha plane) as both the encoder's native input and the
//! decoder's native output. Conversions out of 'x0a8' are measured separately
//! (`alpha_format_transfer`), and every bit-depth and quality number here reads the
//! decoder's native planes directly so a converter defect cannot masquerade as an
//! encoder defect.

use std::collections::BTreeSet;
use std::path::Path;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CFString;
use objc2_foundation::NSNumber;
use objc2_video_toolbox::*;
use serde::Serialize;

use crate::avio::{self, TrackInfo};
use crate::cf::{self, Result};
use crate::color::{self, DiffStats};
use crate::exp_quicktime;
use crate::exp_tags;
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material::{self, Material};
use crate::pixbuf::{self, PixelBuffer};
use crate::vt;

#[derive(Serialize)]
pub struct SupportedProps {
    pub codec: String,
    pub encoder_id: Option<String>,
    pub error: Option<String>,
    /// Selected keys with their supported-value descriptions.
    pub selected: Vec<(String, String)>,
    pub all_keys: Vec<String>,
}

#[derive(Serialize)]
pub struct NativeInput {
    pub codec: String,
    pub settings: String,
    pub error: Option<String>,
    /// The session pool's pixel format(s), as four-character codes.
    pub pixel_formats: Vec<String>,
    pub pool_attributes: Vec<(String, String)>,
}

#[derive(Serialize)]
pub struct SessionProps {
    pub codec: String,
    pub settings: String,
    pub error: Option<String>,
    pub properties: Vec<(String, String)>,
}

#[derive(Serialize)]
pub struct FormatDescription {
    pub format: String,
    /// CVPixelFormatDescriptionCreateWithPixelFormatType, whitespace-collapsed.
    pub description: Option<String>,
    /// Whether the public CoreVideo headers of the SDK name this format.
    pub in_public_headers: bool,
}

/// A VTPixelTransferSession conversion of synthetic content with known codes.
#[derive(Serialize)]
pub struct TransferCheck {
    pub from: String,
    pub to: String,
    pub error: Option<String>,
    /// Maximum |error| in the source's code depth (8 or 10 bits) for Y′ and for Cb/Cr
    /// (chroma measured at block interiors, where any upsampling filter is exact).
    pub luma_max_err_codes: Option<f64>,
    pub chroma_max_err_codes: Option<f64>,
    /// Maximum |error| in 8-bit alpha codes.
    pub alpha_max_err_8bit: Option<f64>,
    /// Every channel within 1 code.
    pub ok: bool,
}

#[derive(Serialize)]
pub struct AlphaDepthRow {
    pub settings: String,
    pub error: Option<String>,
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub bits_per_component: Option<String>,
    pub alpha_channel_mode: Option<String>,
    pub data_rate_bps: Option<f64>,
    /// The decoder's native output format (no pixel format requested).
    pub native_format: String,
    /// Bits of the native alpha plane (both 'v0a8' and 'x0a8' carry 8-bit alpha).
    pub alpha_plane_bits: u32,
    /// Distinct alpha codes over the 16-bit alpha ramp (1920 columns, 65536 levels).
    pub alpha_distinct_levels: usize,
    /// Alpha against the ideal ramp rounded to 8 bits.
    pub alpha_max_err_8bit: i64,
    /// Distinct Y′ codes over the opaque grey ramp: 877 at 10-bit (64..940), 220 at 8-bit.
    pub luma_distinct_levels: usize,
    pub luma_expected_levels: usize,
    /// Y′ against the ideal ramp in 10-bit codes.
    pub luma_max_err_10bit: i64,
    /// The same file decoded to 'y416' through VideoToolbox's converter: Y′ error in
    /// 10-bit codes (shows the 'x0a8' conversion defect at Main10).
    pub y416_converted_luma_max_err_10bit: Option<i64>,
}

#[derive(Serialize)]
pub struct AlphaQualityRow {
    pub source: String,
    pub path: String,
    pub track: TrackInfo,
    pub native_format: String,
    pub profile: Option<String>,
    /// Alpha versus the reference decode of the ProRes 4444 source, at 8-bit.
    pub alpha_psnr_8bit: f64,
    pub alpha_max_cv_8bit: i64,
    /// Color (α > 0) versus the reference decode, at 10-bit; chroma against the
    /// reference averaged to 4:2:0 type 0 (left) siting.
    pub y_psnr_10bit: f64,
    pub cb_psnr_10bit: f64,
    pub cr_psnr_10bit: f64,
    pub ycbcr_psnr_10bit: f64,
    /// Engine rows: alpha PSNR at least Apple's preset (section 2, Alpha bar).
    pub alpha_psnr_at_least_apple: Option<bool>,
}

#[derive(Serialize)]
pub struct PlaybackRow {
    pub file: String,
    pub error: Option<String>,
    pub output_format: String,
    /// Display times (24000 timescale) of the compared frames: this file and the source.
    pub frame_time: Option<i64>,
    pub reference_frame_time: Option<i64>,
    /// AVPlayerItemVideoOutput frame 2 versus the same frame of the ProRes 4444 source
    /// through the same path, as PSNR at a 10-bit peak: premultiplied R′G′B′ (what a
    /// compositor shows), straight R′G′B′ on α ≥ 1/4, and alpha.
    pub premultiplied_rgb_psnr: Option<f64>,
    pub straight_rgb_psnr_alpha_quarter: Option<f64>,
    pub alpha_psnr: Option<f64>,
    pub premultiplied_rgb_max_abs_err: Option<f64>,
}

#[derive(Serialize)]
pub struct EncodersReport {
    pub encoders: Vec<vt::EncoderEntry>,
    pub supported_properties: Vec<SupportedProps>,
    pub native_input: Vec<NativeInput>,
    pub session_properties: Vec<SessionProps>,
    pub format_descriptions: Vec<FormatDescription>,
    pub alpha_format_transfer: Vec<TransferCheck>,
    pub hevc_alpha_depth: Vec<AlphaDepthRow>,
    pub hevc_alpha_quality: Vec<AlphaQualityRow>,
    pub hevc_alpha_playback: Vec<PlaybackRow>,
}

const SELECTED: &[&str] = &[
    "ProfileLevel",
    "OutputBitDepth",
    "AlphaChannelMode",
    "TargetQualityForAlpha",
    "AlphaLayerProperties",
    "BaseLayerProperties",
    "RequiredHEVCVideoParameterSetRepFormatBitDepths",
    "InputPixelFormat",
    "EncodesAlpha",
    "Quality",
    "AverageBitRate",
    "ConstantBitRate",
    "GammaLevel",
    "TransferFunction",
    "PreserveAlphaChannel",
    "AllowFrameReordering",
];

fn supported(codec: u32) -> SupportedProps {
    match vt::supported_properties(codec, material::W as i32, material::H as i32) {
        Ok((id, props)) => SupportedProps {
            codec: cf::fourcc_str(codec),
            encoder_id: Some(id),
            error: None,
            all_keys: props.iter().map(|(k, _)| k.clone()).collect(),
            selected: props
                .into_iter()
                .filter(|(k, _)| SELECTED.iter().any(|s| k == s))
                .map(|(k, v)| (k, v.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(900).collect()))
                .collect(),
        },
        Err(e) => SupportedProps { codec: cf::fourcc_str(codec), encoder_id: None, error: Some(e.0), selected: vec![], all_keys: vec![] },
    }
}

fn any_str(s: &CFString) -> Retained<AnyObject> {
    vt::cf_str(s)
}

fn any_num(v: f64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(NSNumber::new_f64(v))).into()
}

type Props = Vec<(&'static CFString, Retained<AnyObject>)>;

fn spec(codec: u32, properties: Props) -> vt::EncodeSpec<'static> {
    vt::EncodeSpec {
        codec,
        width: material::W,
        height: material::H,
        timescale: material::TIMESCALE,
        frame_duration: material::FRAME_DUR,
        properties,
        encoder_id: None,
    }
}

fn profile(p: &'static CFString) -> (&'static CFString, Retained<AnyObject>) {
    (unsafe { kVTCompressionPropertyKey_ProfileLevel }, any_str(p))
}

/// The engine's encoder input for HEVC with alpha is straight (unpremultiplied) 'y416',
/// so the stream is tagged straight, as AVFoundation's preset tags its output.
fn straight() -> (&'static CFString, Retained<AnyObject>) {
    (unsafe { kVTCompressionPropertyKey_AlphaChannelMode }, any_str(unsafe { kVTAlphaChannelMode_StraightAlpha }))
}

/// Rec.709 1-1-1 tags as session properties. HEVC with alpha does not carry the input
/// buffers' color attachments into the format description, so the engine sets them on
/// the session (as `exp_generation` does for every codec).
fn rec709() -> Props {
    use objc2_core_video::*;
    unsafe {
        vec![
            (kVTCompressionPropertyKey_ColorPrimaries, any_str(kCVImageBufferColorPrimaries_ITU_R_709_2)),
            (kVTCompressionPropertyKey_TransferFunction, any_str(kCVImageBufferTransferFunction_ITU_R_709_2)),
            (kVTCompressionPropertyKey_YCbCrMatrix, any_str(kCVImageBufferYCbCrMatrix_ITU_R_709_2)),
        ]
    }
}

fn with_tags(mut p: Props) -> Props {
    p.extend(rec709());
    p
}

fn pool_formats(attrs: &[(String, String)]) -> Vec<String> {
    attrs
        .iter()
        .filter(|(k, _)| k.contains("PixelFormatType"))
        .flat_map(|(_, v)| {
            v.split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u32>().ok())
                .map(cf::fourcc_str)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn format_description(format: u32, public: bool) -> FormatDescription {
    let d = objc2_core_video::CVPixelFormatDescriptionCreateWithPixelFormatType(None, format);
    FormatDescription {
        format: cf::fourcc_str(format),
        description: d.map(|d| {
            let d: &objc2_core_foundation::CFType = &d;
            cf::describe_cf(d).split_whitespace().collect::<Vec<_>>().join(" ")
        }),
        in_public_headers: public,
    }
}

// ---- Native tri-planar access ('v0a8', 'x0a8') ----

/// Planes of a tri-planar alpha buffer as codes: Y′ and CbCr at `bits` (8 or 10), alpha at 8.
/// The 10-bit formats ('x0a8', 'x2a8', 'x4a8') hold MSB-aligned u16 samples as the
/// decoder writes them (verified against an analytic ramp in `hevc_alpha_depth`).
struct Tri {
    w: usize,
    h: usize,
    /// Chroma subsampling factors: (2, 2) for 4:2:0, (2, 1) for 4:2:2, (1, 1) for 4:4:4.
    sx: usize,
    sy: usize,
    bits: u32,
    y: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
    a: Vec<u8>,
}

fn read_tri(pb: &PixelBuffer) -> Result<Tri> {
    let (w, h) = (pb.width(), pb.height());
    let f = pb.format();
    let ten = [pixbuf::X0A8, pixbuf::X2A8, pixbuf::X4A8].contains(&f);
    let (bits, y, c, cw, ch) = if ten {
        let (y, _, _) = pb.read_u16(Some(0))?;
        let (c, comps, ch) = pb.read_u16(Some(1))?;
        // MSB-aligned 10-bit; round in case low bits are populated.
        let q = |v: u16| ((v as u32 + 32) >> 6).min(1023) as u16;
        (10, y.into_iter().map(q).collect::<Vec<_>>(), c.into_iter().map(q).collect::<Vec<_>>(), comps / 2, ch)
    } else if f == pixbuf::V0A8 {
        let (y, _, _) = pb.read_u8(Some(0))?;
        let (c, comps, ch) = pb.read_u8(Some(1))?;
        (8, y.into_iter().map(u16::from).collect(), c.into_iter().map(u16::from).collect(), comps / 2, ch)
    } else {
        return cf::err(format!("read_tri: '{}' is not a tri-planar alpha format", cf::fourcc_str(f)));
    };
    let (a, _, _) = pb.read_u8(Some(2))?;
    Ok(Tri {
        w,
        h,
        sx: w / cw.max(1),
        sy: h / ch.max(1),
        bits,
        y,
        cb: c.iter().step_by(2).copied().collect(),
        cr: c.iter().skip(1).step_by(2).copied().collect(),
        a,
    })
}

fn tri_format(bits: u32, sx: usize, sy: usize) -> u32 {
    match (bits, sx, sy) {
        (8, _, _) => pixbuf::V0A8,
        (_, 2, 2) => pixbuf::X0A8,
        (_, 2, 1) => pixbuf::X2A8,
        _ => pixbuf::X4A8,
    }
}

impl Tri {
    fn cw(&self) -> usize {
        self.w / self.sx
    }
    /// Chroma site index for pixel (x, y).
    fn site(&self, x: usize, y: usize) -> usize {
        (y / self.sy) * self.cw() + x / self.sx
    }
}

fn write_tri(t: &Tri) -> Result<PixelBuffer> {
    let fmt = tri_format(t.bits, t.sx, t.sy);
    let pb = PixelBuffer::new(t.w, t.h, fmt)?;
    let mut c = Vec::with_capacity(t.cb.len() * 2);
    for (b, r) in t.cb.iter().zip(&t.cr) {
        c.push(*b);
        c.push(*r);
    }
    if t.bits == 10 {
        pb.write_u16(Some(0), &t.y.iter().map(|v| v << 6).collect::<Vec<_>>())?;
        pb.write_u16(Some(1), &c.iter().map(|v| v << 6).collect::<Vec<_>>())?;
    } else {
        pb.write_u8(Some(0), &t.y.iter().map(|v| *v as u8).collect::<Vec<_>>())?;
        pb.write_u8(Some(1), &c.iter().map(|v| *v as u8).collect::<Vec<_>>())?;
    }
    pb.write_u8(Some(2), &t.a)?;
    pb.tag_709();
    Ok(pb)
}

/// Synthetic content with known codes. Chroma is constant over 8×8 pixel blocks so
/// that block interiors are exact under any upsampling filter.
fn synthetic(bits: u32, sx: usize, sy: usize) -> Tri {
    let (w, h) = (256usize, 128usize);
    let (lo, ly, lc) = if bits == 10 { (64u32, 877u32, 897u32) } else { (16, 220, 225) };
    let mut t = Tri { w, h, sx, sy, bits, y: vec![], cb: vec![], cr: vec![], a: vec![] };
    for y in 0..h {
        for x in 0..w {
            t.y.push((lo + ((x * 7 + y * 3) as u32 % ly)) as u16);
            t.a.push(((x + 2 * y) % 256) as u8);
        }
    }
    for cy in 0..h / sy {
        for cx in 0..w / sx {
            let (bx, by) = (cx * sx / 8, cy * sy / 8);
            t.cb.push((lo + ((bx * 37 + by * 11) as u32 % lc)) as u16);
            t.cr.push((lo + ((bx * 13 + by * 29 + 7) as u32 % lc)) as u16);
        }
    }
    t
}

fn interior(x: usize, y: usize) -> bool {
    (2..6).contains(&(x % 8)) && (2..6).contains(&(y % 8))
}

/// Chroma site (4:2:0) away from the edges of its 4×4-site block.
fn site_interior(cx: usize, cy: usize) -> bool {
    (1..3).contains(&(cx % 4)) && (1..3).contains(&(cy % 4))
}

/// Measure a converted buffer against synthetic content `t`.
fn check_against(t: &Tri, got: &PixelBuffer) -> Result<(f64, f64, Option<f64>)> {
    let shift = 16 - t.bits;
    let scale = (1u32 << shift) as f64;
    let (w, h) = (t.w, t.h);
    let site = |x: usize, y: usize| t.site(x, y);
    let mut ly = 0f64;
    let mut lc = 0f64;
    let mut la: Option<f64> = None;
    let f = got.format();
    if f == pixbuf::Y416 {
        let (d, _, _) = got.read_u16(None)?;
        for y in 0..h {
            for x in 0..w {
                let p = &d[(y * w + x) * 4..(y * w + x) * 4 + 4];
                ly = ly.max((p[1] as f64 / scale - t.y[y * w + x] as f64).abs());
                if interior(x, y) {
                    let s = site(x, y);
                    lc = lc.max((p[2] as f64 / scale - t.cb[s] as f64).abs()).max((p[3] as f64 / scale - t.cr[s] as f64).abs());
                }
                let a = la.get_or_insert(0.0);
                *a = a.max((p[0] as f64 / 257.0 - t.a[y * w + x] as f64).abs());
            }
        }
    } else if f == pixbuf::X420 || f == pixbuf::V420 {
        // Codes of the target depth, expressed in the source's code depth.
        let (yv, cv, k): (Vec<f64>, Vec<f64>, f64) = if f == pixbuf::X420 {
            (
                got.read_u16(Some(0))?.0.iter().map(|v| *v as f64 / 64.0).collect(),
                got.read_u16(Some(1))?.0.iter().map(|v| *v as f64 / 64.0).collect(),
                2f64.powi(t.bits as i32 - 10),
            )
        } else {
            (
                got.read_u8(Some(0))?.0.iter().map(|v| *v as f64).collect(),
                got.read_u8(Some(1))?.0.iter().map(|v| *v as f64).collect(),
                2f64.powi(t.bits as i32 - 8),
            )
        };
        for i in 0..w * h {
            ly = ly.max((yv[i] * k - t.y[i] as f64).abs());
        }
        // 4:2:0 output sites (cx, cy) cover pixels (2cx, 2cy); compare against the
        // source site at that pixel.
        for cy in 0..h / 2 {
            for cx in 0..w / 2 {
                if site_interior(cx, cy) {
                    let (o, s) = (cy * (w / 2) + cx, site(2 * cx, 2 * cy));
                    lc = lc.max((cv[2 * o] * k - t.cb[s] as f64).abs()).max((cv[2 * o + 1] * k - t.cr[s] as f64).abs());
                }
            }
        }
    } else if f == pixbuf::RGBA_HALF {
        let (d, _, _) = got.read_u16(None)?;
        let (yo, ys, co, cs) = if t.bits == 10 { (64.0, 876.0, 512.0, 896.0) } else { (16.0, 219.0, 128.0, 224.0) };
        for y in 0..h {
            for x in 0..w {
                let p: Vec<f64> = d[(y * w + x) * 4..(y * w + x) * 4 + 4].iter().map(|v| exp_quicktime::half_to_f32(*v) as f64).collect();
                let ycc = color::rgb_to_ycbcr([p[0], p[1], p[2]]);
                ly = ly.max((ycc[0] * ys + yo - t.y[y * w + x] as f64).abs());
                if interior(x, y) {
                    let s = site(x, y);
                    lc = lc.max((ycc[1] * cs + co - t.cb[s] as f64).abs()).max((ycc[2] * cs + co - t.cr[s] as f64).abs());
                }
                let a = la.get_or_insert(0.0);
                *a = a.max((p[3] * 255.0 - t.a[y * w + x] as f64).abs());
            }
        }
    } else if [pixbuf::X0A8, pixbuf::X2A8, pixbuf::X4A8, pixbuf::V0A8].contains(&f) {
        let g = read_tri(got)?;
        if (g.sx, g.sy) != (t.sx, t.sy) {
            return cf::err("check_against: subsampling differs");
        }
        let k = 2f64.powi(t.bits as i32 - g.bits as i32);
        for i in 0..w * h {
            ly = ly.max((g.y[i] as f64 * k - t.y[i] as f64).abs());
            la = Some(la.unwrap_or(0.0).max((g.a[i] as f64 - t.a[i] as f64).abs()));
        }
        for s in 0..t.cb.len() {
            lc = lc.max((g.cb[s] as f64 * k - t.cb[s] as f64).abs()).max((g.cr[s] as f64 * k - t.cr[s] as f64).abs());
        }
    } else {
        return cf::err(format!("check_against: unsupported '{}'", cf::fourcc_str(f)));
    }
    Ok((ly, lc, la))
}

/// y416 carrying synthetic content `t` (codes MSB-aligned; chroma replicated to pixels).
fn tri_as_y416(t: &Tri) -> Result<PixelBuffer> {
    let pb = PixelBuffer::new(t.w, t.h, pixbuf::Y416)?;
    let shift = 16 - t.bits;
    let mut d = Vec::with_capacity(t.w * t.h * 4);
    for y in 0..t.h {
        for x in 0..t.w {
            let s = t.site(x, y);
            d.extend([t.a[y * t.w + x] as u16 * 257, t.y[y * t.w + x] << shift, t.cb[s] << shift, t.cr[s] << shift]);
        }
    }
    pb.write_u16(None, &d)?;
    pb.tag_709();
    Ok(pb)
}

fn transfer_checks() -> Vec<TransferCheck> {
    let mut out = vec![];
    for (bits, sx, sy) in [(10u32, 2usize, 2usize), (10, 2, 1), (10, 1, 1), (8, 2, 2)] {
        let t = synthetic(bits, sx, sy);
        let fmt = tri_format(bits, sx, sy);
        let name = cf::fourcc_str(fmt);
        let src = match write_tri(&t) {
            Ok(b) => b,
            Err(e) => {
                out.push(TransferCheck { from: name, to: "-".into(), error: Some(e.0), luma_max_err_codes: None, chroma_max_err_codes: None, alpha_max_err_8bit: None, ok: false });
                continue;
            }
        };
        let mut push = |from: String, to: u32, r: Result<PixelBuffer>| {
            let row = match r.and_then(|b| check_against(&t, &b)) {
                Ok((l, c, a)) => TransferCheck {
                    from,
                    to: cf::fourcc_str(to),
                    error: None,
                    luma_max_err_codes: Some(l),
                    chroma_max_err_codes: Some(c),
                    alpha_max_err_8bit: a,
                    ok: l <= 1.0 && c <= 1.0 && a.map(|a| a <= 1.0).unwrap_or(true),
                },
                Err(e) => TransferCheck { from, to: cf::fourcc_str(to), error: Some(e.0), luma_max_err_codes: None, chroma_max_err_codes: None, alpha_max_err_8bit: None, ok: false },
            };
            out.push(row);
        };
        for to in [pixbuf::Y416, pixbuf::X420, pixbuf::V420, pixbuf::RGBA_HALF] {
            push(name.clone(), to, vt::transfer(&src, to));
        }
        push("y416".into(), fmt, tri_as_y416(&t).and_then(|y| vt::transfer(&y, fmt)));
    }
    out
}

// ---- HEVC-with-alpha bit depth ----

/// Alpha-ramp test frames through the engine: rows 0..540 have α = column ramp over
/// grey 0.5; rows 540..1080 are opaque with a grey ramp. Output 'y416' (straight).
fn ramp_frames(gpu: &Gpu, n: usize) -> Result<(Vec<PixelBuffer>, Vec<u16>)> {
    let (w, h) = (material::W, material::H);
    let src = PixelBuffer::new(w, h, pixbuf::RGBA64)?;
    let mut d = vec![0u16; w * h * 4];
    let ramp: Vec<u16> = (0..w).map(|x| ((x as u64 * 65535 + (w as u64 - 1) / 2) / (w as u64 - 1)) as u16).collect();
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            if y < h / 2 {
                d[o..o + 4].copy_from_slice(&[32768, 32768, 32768, ramp[x]]);
            } else {
                d[o..o + 4].copy_from_slice(&[ramp[x], ramp[x], ramp[x], 65535]);
            }
        }
    }
    src.write_u16(None, &d)?;
    let p = PipeParams::standard(Precision::F32);
    let frames = (0..n).map(|_| gpu.engine_convert(&src, pixbuf::Y416, &p)).collect::<Result<_>>()?;
    Ok((frames, ramp))
}

fn ext(track: &TrackInfo, key: &str) -> Option<String> {
    track.extensions.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Decode with no pixel format requested: the decoder's native output.
fn decode_native(path: &Path) -> Result<vt::DecodeInfo> {
    let dec = vt::decode_file_native(path)?;
    if dec.frames.is_empty() {
        return cf::err("no frames decoded");
    }
    Ok(dec)
}

fn alpha_depth(dir: &Path, frames: &[PixelBuffer], ramp: &[u16], name: &str, props: Props) -> AlphaDepthRow {
    let path = dir.join(format!("alpha_{}.mov", name.replace([' ', ',', '=', '.'], "_")));
    let refs: Vec<&PixelBuffer> = frames.iter().collect();
    let mut row = AlphaDepthRow {
        settings: name.into(),
        error: None,
        codec: None,
        profile: None,
        bits_per_component: None,
        alpha_channel_mode: None,
        data_rate_bps: None,
        native_format: String::new(),
        alpha_plane_bits: 0,
        alpha_distinct_levels: 0,
        alpha_max_err_8bit: 0,
        luma_distinct_levels: 0,
        luma_expected_levels: 0,
        luma_max_err_10bit: 0,
        y416_converted_luma_max_err_10bit: None,
    };
    let r = (|| -> Result<()> {
        vt::encode_to_file(&path, avio::mov(), &spec(objc2_core_media::kCMVideoCodecType_HEVCWithAlpha, props), &refs)?;
        let track = avio::track_info(&path)?;
        row.codec = Some(track.codec.clone());
        row.bits_per_component = ext(&track, "BitsPerComponent");
        row.alpha_channel_mode = ext(&track, "AlphaChannelMode");
        row.data_rate_bps = Some(track.estimated_data_rate_bps);
        row.profile = hevc_profile(&path);
        let dec = decode_native(&path)?;
        row.native_format = cf::fourcc_str(dec.frames[0].buffer.format());
        let (w, h) = (material::W, material::H);
        let mut alphas = BTreeSet::new();
        let mut lumas = BTreeSet::new();
        // Ideal 10-bit luma of the grey ramp (E-encoded grey → Y′ = value).
        let ideal10: Vec<i64> = ramp.iter().map(|&v| (v as f64 / 65535.0 * 876.0 + 64.0).round() as i64).collect();
        for f in &dec.frames {
            let t = read_tri(&f.buffer)?;
            row.alpha_plane_bits = 8;
            for y in [h / 4, h / 4 + 1] {
                for x in 0..w {
                    let a = t.a[y * w + x];
                    alphas.insert(a);
                    let ideal = ((ramp[x] as u32 + 128) / 257) as i64;
                    row.alpha_max_err_8bit = row.alpha_max_err_8bit.max((a as i64 - ideal).abs());
                }
            }
            let k = 1i64 << (10 - t.bits);
            for y in [3 * h / 4, 3 * h / 4 + 1] {
                for x in 0..w {
                    let v = t.y[y * w + x];
                    lumas.insert(v);
                    row.luma_max_err_10bit = row.luma_max_err_10bit.max((v as i64 * k - ideal10[x]).abs());
                }
            }
            row.luma_expected_levels = if t.bits == 10 { 877 } else { 220 };
        }
        row.alpha_distinct_levels = alphas.len();
        row.luma_distinct_levels = lumas.len();
        // The converter path: decode to y416.
        if let Ok(conv) = vt::decode_file(&path, pixbuf::Y416) {
            let mut m = 0i64;
            for f in &conv.frames {
                let (d, _, _) = f.buffer.read_u16(None)?;
                for y in [3 * h / 4, 3 * h / 4 + 1] {
                    for x in 0..w {
                        let v = ((d[(y * w + x) * 4 + 1] as i64) + 32) >> 6;
                        m = m.max((v - ideal10[x]).abs());
                    }
                }
            }
            row.y416_converted_luma_max_err_10bit = Some(m);
        }
        Ok(())
    })();
    if let Err(e) = r {
        row.error = Some(e.0);
    }
    row
}

/// The base layer's profile, chroma format, and depth from the file's `hvcC` box, and
/// whether an `lhvC` (layered HEVC: the alpha layer) box is present.
fn hevc_profile(path: &Path) -> Option<String> {
    let d = exp_tags::parse_file(path).ok()?;
    let h = d.hvcc?;
    let name = match h.profile_idc {
        1 => "Main",
        2 => "Main10",
        4 => "RExt",
        _ => "other",
    };
    let cf = ["4:0:0", "4:2:0", "4:2:2", "4:4:4"][h.chroma_format_idc as usize];
    let lhvc = if d.child_boxes.iter().any(|b| b == "lhvC") { ", lhvC present" } else { ", no lhvC" };
    Some(format!("{name} (profile_idc {}), {cf}, {}-bit luma, {}-bit chroma{lhvc}", h.profile_idc, h.bit_depth_luma, h.bit_depth_chroma))
}

/// Quality of an HEVC-with-alpha file against the reference decode, from native planes.
/// Chroma is compared with the reference at the stream's siting: 4:2:0 type 0 (left,
/// vertically between rows) against the two-row average, 4:2:2 against the co-sited even
/// column, 4:4:4 per pixel.
fn alpha_quality(path: &Path, source: &str, reference: &[vt::Frame]) -> Result<AlphaQualityRow> {
    let track = avio::track_info(path)?;
    let dec = decode_native(path)?;
    let native = dec.frames[0].buffer.format();
    let (mut a8, mut y10, mut cb10, mut cr10) = (DiffStats::default(), DiffStats::default(), DiffStats::default(), DiffStats::default());
    for (f, r) in dec.frames.iter().zip(reference) {
        let t = read_tri(&f.buffer)?;
        let (rd, _, _) = r.buffer.read_u16(None)?;
        let (w, h) = (t.w, t.h);
        let k = 1i64 << (10 - t.bits);
        for i in 0..w * h {
            let q = &rd[i * 4..i * 4 + 4];
            a8.add(t.a[i] as i64, ((q[0] as i64) + 128) / 257);
            if q[0] > 0 {
                y10.add(t.y[i] as i64 * k, ((q[1] as i64) + 32) >> 6);
            }
        }
        for cy in 0..h / t.sy {
            for cx in 0..w / t.sx {
                let rows: Vec<usize> = (0..t.sy).map(|dy| ((t.sy * cy + dy) * w + t.sx * cx) * 4).collect();
                if rows.iter().any(|&i| rd[i] == 0) {
                    continue;
                }
                let avg = |c: usize| (rows.iter().map(|&i| rd[i + c] as f64).sum::<f64>() / rows.len() as f64 / 64.0).round() as i64;
                let s = cy * t.cw() + cx;
                cb10.add(t.cb[s] as i64 * k, avg(2));
                cr10.add(t.cr[s] as i64 * k, avg(3));
            }
        }
    }
    let mut all = y10.clone();
    all.merge(&cb10);
    all.merge(&cr10);
    Ok(AlphaQualityRow {
        source: source.into(),
        path: path.file_name().unwrap().to_string_lossy().into(),
        native_format: cf::fourcc_str(native),
        profile: hevc_profile(path),
        track,
        alpha_psnr_8bit: a8.psnr(255.0),
        alpha_max_cv_8bit: a8.max_abs,
        y_psnr_10bit: y10.psnr(1023.0),
        cb_psnr_10bit: cb10.psnr(1023.0),
        cr_psnr_10bit: cr10.psnr(1023.0),
        ycbcr_psnr_10bit: all.psnr(1023.0),
        alpha_psnr_at_least_apple: None,
    })
}

fn rgha_floats(pb: &PixelBuffer) -> Result<Vec<f32>> {
    if pb.format() != pixbuf::RGBA_HALF {
        return cf::err(format!("expected 'RGhA', got '{}'", cf::fourcc_str(pb.format())));
    }
    Ok(pb.read_u16(None)?.0.iter().map(|h| exp_quicktime::half_to_f32(*h)).collect())
}

fn playback(files: &[(String, std::path::PathBuf)], source: &Path, mtm: MainThreadMarker) -> Vec<PlaybackRow> {
    let t = 2 * material::FRAME_DUR;
    let reference = exp_quicktime::player_frame_timed(source, pixbuf::RGBA_HALF, t, mtm).and_then(|(f, at)| Ok((rgha_floats(&f)?, at)));
    files
        .iter()
        .map(|(name, path)| {
            let mut row = PlaybackRow {
                file: name.clone(),
                error: None,
                output_format: "RGhA".into(),
                frame_time: None,
                reference_frame_time: reference.as_ref().ok().map(|r| r.1),
                premultiplied_rgb_psnr: None,
                straight_rgb_psnr_alpha_quarter: None,
                alpha_psnr: None,
                premultiplied_rgb_max_abs_err: None,
            };
            let r = (|| -> Result<()> {
                let (refv, ref_at) = reference.as_ref().map_err(|e| cf::Error(format!("reference: {e}")))?;
                let (frame, at) = exp_quicktime::player_frame_timed(path, pixbuf::RGBA_HALF, t, mtm)?;
                row.frame_time = Some(at);
                let half = material::FRAME_DUR / 2;
                if (at - t).abs() > half || (ref_at - t).abs() > half {
                    return cf::err(format!("not frame 2: file at {at}, source at {ref_at}, requested {t} (24000 timescale)"));
                }
                let got = rgha_floats(&frame)?;
                if got.len() != refv.len() {
                    return cf::err("frame sizes differ");
                }
                let (mut se_p, mut se_s, mut n_s, mut se_a, mut mx) = (0f64, 0f64, 0u64, 0f64, 0f64);
                for (p, q) in got.chunks_exact(4).zip(refv.chunks_exact(4)) {
                    se_a += ((p[3] - q[3]) as f64 * 1023.0).powi(2);
                    for c in 0..3 {
                        let d = (p[c] * p[3] - q[c] * q[3]) as f64 * 1023.0;
                        se_p += d * d;
                        mx = mx.max(d.abs() / 1023.0);
                        if q[3] >= 0.25 {
                            se_s += ((p[c] - q[c]) as f64 * 1023.0).powi(2);
                            n_s += 1;
                        }
                    }
                }
                let n_px = (got.len() / 4) as f64;
                let ps = |se: f64, n: f64| if se == 0.0 { f64::INFINITY } else { 10.0 * (1023.0f64.powi(2) / (se / n)).log10() };
                row.premultiplied_rgb_psnr = Some(ps(se_p, 3.0 * n_px));
                row.straight_rgb_psnr_alpha_quarter = Some(ps(se_s, n_s as f64));
                row.alpha_psnr = Some(ps(se_a, n_px));
                row.premultiplied_rgb_max_abs_err = Some(mx);
                Ok(())
            })();
            if let Err(e) = r {
                row.error = Some(e.0);
            }
            row
        })
        .collect()
}

pub fn run(gpu: &Gpu, m: &Material) -> Result<EncodersReport> {
    let mtm = MainThreadMarker::new().ok_or_else(|| cf::Error("encoders experiment must run on the main thread".into()))?;
    let dir = m.dir.join("../encoders");
    std::fs::create_dir_all(&dir).map_err(|e| cf::Error(e.to_string()))?;
    let encoders = vt::encoder_list()?;
    use objc2_core_media::*;
    let codecs = [
        kCMVideoCodecType_HEVCWithAlpha,
        kCMVideoCodecType_HEVC,
        kCMVideoCodecType_H264,
        kCMVideoCodecType_AppleProRes422HQ,
        kCMVideoCodecType_AppleProRes4444,
        kCMVideoCodecType_AppleProRes4444XQ,
    ];
    let supported_properties = codecs.iter().map(|&c| supported(c)).collect();

    let main10 = || profile(unsafe { kVTProfileLevel_HEVC_Main10_AutoLevel });
    let main42210 = || profile(unsafe { kVTProfileLevel_HEVC_Main42210_AutoLevel });
    // No binding for the Main44410 constant in objc2-video-toolbox 0.3; the value is its name.
    let main44410 = || (unsafe { kVTCompressionPropertyKey_ProfileLevel }, any_str(&CFString::from_str("HEVC_Main44410_AutoLevel")));
    let configs: Vec<(u32, &str, Props)> = vec![
        (kCMVideoCodecType_HEVCWithAlpha, "default", vec![]),
        (kCMVideoCodecType_HEVCWithAlpha, "Main10", vec![main10()]),
        (kCMVideoCodecType_HEVCWithAlpha, "Main42210", vec![main42210()]),
        (kCMVideoCodecType_HEVCWithAlpha, "Main44410", vec![main44410()]),
        (kCMVideoCodecType_HEVC, "default", vec![]),
        (kCMVideoCodecType_HEVC, "Main10", vec![main10()]),
        (kCMVideoCodecType_HEVC, "Main42210", vec![main42210()]),
        (kCMVideoCodecType_H264, "default", vec![]),
        (kCMVideoCodecType_AppleProRes422Proxy, "default", vec![]),
        (kCMVideoCodecType_AppleProRes422HQ, "default", vec![]),
        (kCMVideoCodecType_AppleProRes4444, "default", vec![]),
        (kCMVideoCodecType_AppleProRes4444XQ, "default", vec![]),
    ];
    let mut native_input = vec![];
    let mut session_properties = vec![];
    const SESSION_KEYS: &[&str] = &[
        "ProfileLevel",
        "OutputBitDepth",
        "AlphaChannelMode",
        "AlphaLayerProperties",
        "BaseLayerProperties",
        "RequiredHEVCVideoParameterSetRepFormatBitDepths",
        "UsingHardwareAcceleratedVideoEncoder",
        "EncoderID",
    ];
    for (codec, settings, props) in configs {
        let (pool_attributes, error) = match vt::native_input_attributes(&spec(codec, props.clone())) {
            Ok(a) => (a, None),
            Err(e) => (vec![], Some(e.0)),
        };
        native_input.push(NativeInput { codec: cf::fourcc_str(codec), settings: settings.into(), error, pixel_formats: pool_formats(&pool_attributes), pool_attributes });
        if codec == kCMVideoCodecType_HEVCWithAlpha || codec == kCMVideoCodecType_HEVC {
            session_properties.push(match vt::session_properties(&spec(codec, props), SESSION_KEYS) {
                Ok(p) => SessionProps { codec: cf::fourcc_str(codec), settings: settings.into(), error: None, properties: p },
                Err(e) => SessionProps { codec: cf::fourcc_str(codec), settings: settings.into(), error: Some(e.0), properties: vec![] },
            });
        }
    }
    let format_descriptions = vec![format_description(pixbuf::V0A8, true), format_description(pixbuf::X0A8, false)];
    let alpha_format_transfer = transfer_checks();

    let (frames, ramp) = ramp_frames(gpu, 6)?;
    let mut hevc_alpha_depth = vec![];
    let depth_configs: Vec<(&str, Props)> = vec![
        ("default", vec![straight()]),
        ("Main10 profile", vec![straight(), main10()]),
        ("OutputBitDepth 10", vec![straight(), (unsafe { kVTCompressionPropertyKey_OutputBitDepth }, any_num(10.0))]),
        ("Main10 profile, OutputBitDepth 10", vec![straight(), main10(), (unsafe { kVTCompressionPropertyKey_OutputBitDepth }, any_num(10.0))]),
        (
            "Main10 profile, Quality 1.0, TargetQualityForAlpha 1.0",
            vec![
                straight(),
                main10(),
                (unsafe { kVTCompressionPropertyKey_Quality }, any_num(1.0)),
                (unsafe { kVTCompressionPropertyKey_TargetQualityForAlpha }, any_num(1.0)),
            ],
        ),
        ("Main42210 profile", vec![straight(), main42210()]),
        ("Main44410 profile", vec![straight(), main44410()]),
    ];
    for (name, props) in depth_configs {
        hevc_alpha_depth.push(alpha_depth(&dir, &frames, &ramp, name, with_tags(props)));
    }

    // AVFoundation's HEVC-with-alpha preset versus the engine on the same ProRes 4444 frames.
    let reference = vt::decode_file(&m.natural_4444xq_alpha, pixbuf::Y416)?.frames;
    let mut hevc_alpha_quality = vec![];
    let apple = dir.join("alpha_apple_preset.mov");
    avio::export_session(&m.natural_4444xq_alpha, &apple, unsafe { objc2_av_foundation::AVAssetExportPresetHEVCHighestQualityWithAlpha }, avio::mov())?;
    hevc_alpha_quality.push(alpha_quality(&apple, "AVAssetExportPresetHEVCHighestQualityWithAlpha", &reference)?);
    let p = PipeParams::standard(Precision::F32);
    let inputs: Vec<PixelBuffer> = reference.iter().map(|f| gpu.engine_convert(&f.buffer, pixbuf::Y416, &p)).collect::<Result<_>>()?;
    let refs: Vec<&PixelBuffer> = inputs.iter().collect();
    let mut playback_files = vec![("AVAssetExportPresetHEVCHighestQualityWithAlpha".to_string(), apple.clone())];
    let quality_configs: Vec<(&str, Props)> = vec![
        ("engine, muxa default, straight", vec![straight()]),
        ("engine, muxa Main10, straight", vec![straight(), main10()]),
        (
            "engine, muxa Main10, straight, Quality 1.0, TargetQualityForAlpha 1.0",
            vec![
                straight(),
                main10(),
                (unsafe { kVTCompressionPropertyKey_Quality }, any_num(1.0)),
                (unsafe { kVTCompressionPropertyKey_TargetQualityForAlpha }, any_num(1.0)),
            ],
        ),
        ("engine, muxa Main42210, straight", vec![straight(), main42210()]),
        ("engine, muxa Main44410, straight", vec![straight(), main44410()]),
    ];
    for (name, props) in quality_configs {
        let path = dir.join(format!("alpha_{}.mov", name.replace([' ', ',', '.'], "_")));
        vt::encode_to_file(&path, avio::mov(), &spec(kCMVideoCodecType_HEVCWithAlpha, with_tags(props)), &refs)
            .map_err(|e| cf::Error(format!("{name}: {e}")))?;
        hevc_alpha_quality.push(alpha_quality(&path, name, &reference)?);
        playback_files.push((name.to_string(), path));
    }
    let apple_alpha = hevc_alpha_quality[0].alpha_psnr_8bit;
    for r in hevc_alpha_quality.iter_mut().skip(1) {
        r.alpha_psnr_at_least_apple = Some(r.alpha_psnr_8bit >= apple_alpha);
    }
    let hevc_alpha_playback = playback(&playback_files, &m.natural_4444xq_alpha, mtm);
    Ok(EncodersReport {
        encoders,
        supported_properties,
        native_input,
        session_properties,
        format_descriptions,
        alpha_format_transfer,
        hevc_alpha_depth,
        hevc_alpha_quality,
        hevc_alpha_playback,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_codes_stay_in_range_and_cover_levels() {
        for (bits, sx, sy) in [(8u32, 2usize, 2usize), (10, 2, 2), (10, 2, 1), (10, 1, 1)] {
            let t = synthetic(bits, sx, sy);
            let (lo, hi) = if bits == 10 { (64, 940) } else { (16, 235) };
            assert!(t.y.iter().all(|&v| (lo..=hi).contains(&v)));
            let distinct: BTreeSet<_> = t.y.iter().collect();
            assert_eq!(distinct.len(), (hi - lo + 1) as usize);
            assert!(t.cb.iter().chain(&t.cr).all(|&v| v >= lo && v <= if bits == 10 { 960 } else { 240 }));
            assert_eq!(t.cb.len(), t.w * t.h / (sx * sy));
            assert_eq!(tri_format(bits, sx, sy), match (bits, sx, sy) {
                (8, _, _) => pixbuf::V0A8,
                (_, 2, 2) => pixbuf::X0A8,
                (_, 2, 1) => pixbuf::X2A8,
                _ => pixbuf::X4A8,
            });
            // Chroma is constant over 8×8 pixel blocks at every subsampling.
            assert_eq!(t.cb[t.site(0, 0)], t.cb[t.site(7, 7)]);
            assert_ne!(t.cb[t.site(0, 0)], t.cb[t.site(8, 0)]);
        }
    }

    #[test]
    fn parses_hvcc_record() {
        // Main10, level 120, 4:2:0, 10-bit (the first 23 bytes of an encoder hvcC).
        let p = [
            0x01, 0x02, 0x20, 0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xf0, 0x00, 0xfc, 0xfd, 0xfa,
            0xfa, 0x00, 0x00, 0x0f, 0x04,
        ];
        let h = exp_tags::parse_hvcc(&p).unwrap();
        assert_eq!(h, exp_tags::Hvcc { profile_idc: 2, level_idc: 120, chroma_format_idc: 1, bit_depth_luma: 10, bit_depth_chroma: 10 });
        assert!(exp_tags::parse_hvcc(&p[..10]).is_none());
    }

    #[test]
    fn interior_excludes_block_edges() {
        assert!(!interior(0, 3));
        assert!(!interior(7, 3));
        assert!(interior(2, 2));
        assert!(interior(13, 10));
    }

    #[test]
    fn pool_formats_parse_lists_and_scalars() {
        let a = vec![("PixelFormatType".to_string(), "(     2016436536 )".to_string())];
        assert_eq!(pool_formats(&a), vec!["x0a8".to_string()]);
        let b = vec![("PixelFormatType".to_string(), "875704438".to_string())];
        assert_eq!(pool_formats(&b), vec!["420v".to_string()]);
    }
}
