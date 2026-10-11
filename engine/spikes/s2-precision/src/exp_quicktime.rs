//! Which Rec.709 tags make QuickTime and Safari match the viewer (section 2, color
//! invariant bar 3; section 9 "QuickTime").
//!
//! Viewer path: E-encoded chart codes, interpreted as sRGB, converted by ColorSync to
//! the display profile (what Chromium's sRGB canvas gets from CoreAnimation).
//! Video path: the same codes through the engine's encoder input, encoded with each
//! candidate tag set, played through `AVPlayerItemVideoOutput`, and converted by
//! ColorSync from the color space AVFoundation attaches to the frame to the display
//! profile (what AVPlayerLayer gets from CoreAnimation). Both results are compared
//! with CIEDE2000 on the patch centers.

use std::path::Path;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_av_foundation::*;
use objc2_core_foundation::{CFRetained, CFRunLoop, CFString, CFType, kCFRunLoopDefaultMode};
use objc2_core_graphics::*;
use objc2_core_video::*;
use objc2_foundation::NSNumber;
use serde::Serialize;

use crate::avio;
use crate::cf::{self, Dict, Result, err};
use crate::color;
use crate::exp_tags::{self, Codec, Tags};
use crate::gpu::{Gpu, PipeParams, Precision};
use crate::material;
use crate::pixbuf::{self, PixelBuffer};

/// The chart: ColorChecker Classic (X-Rite sRGB values), a grey ramp with shadow
/// steps, and primaries and secondaries, as 8-bit sRGB codes.
pub const CHART: [(&str, [u8; 3]); 40] = [
    ("dark skin", [115, 82, 68]),
    ("light skin", [194, 150, 130]),
    ("blue sky", [98, 122, 157]),
    ("foliage", [87, 108, 67]),
    ("blue flower", [133, 128, 177]),
    ("bluish green", [103, 189, 170]),
    ("orange", [214, 126, 44]),
    ("purplish blue", [80, 91, 166]),
    ("moderate red", [193, 90, 99]),
    ("purple", [94, 60, 108]),
    ("yellow green", [157, 188, 64]),
    ("orange yellow", [224, 163, 46]),
    ("blue", [56, 61, 150]),
    ("green", [70, 148, 73]),
    ("red", [175, 54, 60]),
    ("yellow", [231, 199, 31]),
    ("magenta", [187, 86, 149]),
    ("cyan", [8, 133, 161]),
    ("white 9.5", [243, 243, 242]),
    ("neutral 8", [200, 200, 200]),
    ("neutral 6.5", [160, 160, 160]),
    ("neutral 5", [122, 122, 121]),
    ("neutral 3.5", [85, 85, 85]),
    ("black 2", [52, 52, 52]),
    ("grey 0", [0, 0, 0]),
    ("grey 3", [3, 3, 3]),
    ("grey 8", [8, 8, 8]),
    ("grey 16", [16, 16, 16]),
    ("grey 24", [24, 24, 24]),
    ("grey 32", [32, 32, 32]),
    ("grey 64", [64, 64, 64]),
    ("grey 128", [128, 128, 128]),
    ("grey 192", [192, 192, 192]),
    ("grey 255", [255, 255, 255]),
    ("red 255", [255, 0, 0]),
    ("green 255", [0, 255, 0]),
    ("blue 255", [0, 0, 255]),
    ("cyan 255", [0, 255, 255]),
    ("magenta 255", [255, 0, 255]),
    ("yellow 255", [255, 255, 0]),
];

const COLS: usize = 8;
const ROWS: usize = 5;

fn patch_rect(i: usize) -> (usize, usize, usize, usize) {
    let (pw, ph) = (material::W / COLS, material::H / ROWS);
    ((i % COLS) * pw, (i / COLS) * ph, pw, ph)
}

/// The chart as straight 16-bit RGBA ('l64r'), codes × 257 (exactly code / 255).
pub fn chart_buffer() -> Result<PixelBuffer> {
    let pb = PixelBuffer::new(material::W, material::H, pixbuf::RGBA64)?;
    let mut d = vec![0u16; material::W * material::H * 4];
    for (i, (_, c)) in CHART.iter().enumerate() {
        let (x0, y0, w, h) = patch_rect(i);
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let o = (y * material::W + x) * 4;
                d[o] = c[0] as u16 * 257;
                d[o + 1] = c[1] as u16 * 257;
                d[o + 2] = c[2] as u16 * 257;
                d[o + 3] = 65535;
            }
        }
    }
    pb.write_u16(None, &d)?;
    Ok(pb)
}

fn half_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) as u32) << 31;
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s
        } else {
            // Subnormal half → normal f32.
            let mut e = -1i32;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            s | (((e + 127 - 14) as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if e == 31 {
        s | 0x7f80_0000 | (m << 13)
    } else {
        s | (((e - 15 + 127) as u32) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

/// Mean RGB of the central 64×64 of each patch in a 'RGhA' (or 'BGRA') buffer.
fn sample_patches(pb: &PixelBuffer) -> Result<Vec<[f32; 3]>> {
    let w = pb.width();
    let rgha = pb.format() == pixbuf::RGBA_HALF;
    let (d16, d8) = if rgha { (pb.read_u16(None)?.0, vec![]) } else { (vec![], pb.read_u8(None)?.0) };
    let mut out = vec![];
    for i in 0..CHART.len() {
        let (x0, y0, pw, ph) = patch_rect(i);
        let (cx, cy) = (x0 + pw / 2, y0 + ph / 2);
        let mut acc = [0f64; 3];
        let mut n = 0f64;
        for y in cy - 32..cy + 32 {
            for x in cx - 32..cx + 32 {
                let o = (y * w + x) * 4;
                if rgha {
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += half_to_f32(d16[o + k]) as f64;
                    }
                } else {
                    // BGRA
                    acc[0] += d8[o + 2] as f64 / 255.0;
                    acc[1] += d8[o + 1] as f64 / 255.0;
                    acc[2] += d8[o] as f64 / 255.0;
                }
                n += 1.0;
            }
        }
        out.push([(acc[0] / n) as f32, (acc[1] / n) as f32, (acc[2] / n) as f32]);
    }
    Ok(out)
}

fn float_format(cs: &CGColorSpace) -> CGColorDataFormat {
    CGColorDataFormat {
        version: 0,
        colorspace_info: cs as *const CGColorSpace as *const CFType,
        // Float components, alpha last (unpremultiplied, α = 1), 32-bit little endian.
        bitmap_info: CGBitmapInfo(0x100 | 3 | (2 << 12)),
        bits_per_component: 32,
        bytes_per_row: 0,
        intent: CGColorRenderingIntent::RenderingIntentDefault,
        decode: std::ptr::null_mut(),
    }
}

/// Convert RGB triplets from one color space to another with ColorSync (via CoreGraphics).
pub fn convert(src: &[[f32; 3]], from: &CGColorSpace, to: &CGColorSpace) -> Result<Vec<[f32; 3]>> {
    let n = src.len();
    let mut s: Vec<f32> = src.iter().flat_map(|c| [c[0], c[1], c[2], 1.0]).collect();
    let mut d = vec![0f32; n * 4];
    let mut sf = float_format(from);
    let mut df = float_format(to);
    sf.bytes_per_row = n * 16;
    df.bytes_per_row = n * 16;
    let ok = unsafe { CGConvertColorDataWithFormat(n, 1, d.as_mut_ptr().cast(), df, s.as_mut_ptr().cast(), sf, None) };
    if !ok {
        return err("CGConvertColorDataWithFormat failed");
    }
    Ok(d.chunks_exact(4).map(|c| [c[0], c[1], c[2]]).collect())
}

fn named(name: &CFString) -> Result<CFRetained<CGColorSpace>> {
    CGColorSpace::with_name(Some(name)).ok_or_else(|| cf::Error(format!("no color space {name}")))
}

/// Linear sRGB (D65) → CIELAB (D65 white).
fn lab_from_linear_srgb(c: [f32; 3]) -> [f64; 3] {
    let xyz = color::lin709_to_xyz([c[0] as f64, c[1] as f64, c[2] as f64]);
    color::xyz_to_lab(xyz, color::lin709_to_xyz([1.0, 1.0, 1.0]))
}

fn spin(seconds: f64) {
    CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, seconds, false);
}

/// Play `path` with AVPlayer and return the frame at `t` from AVPlayerItemVideoOutput.
pub fn player_frame(path: &Path, format: u32, t_value: i64, mtm: MainThreadMarker) -> Result<PixelBuffer> {
    let url = avio::file_url(path);
    let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
    let attrs = Dict::new()
        .cfkey(unsafe { kCVPixelBufferPixelFormatTypeKey }, &NSNumber::new_u32(format))
        .cfkey(unsafe { kCVPixelBufferIOSurfacePropertiesKey }, &Dict::new().build())
        .build();
    let output = unsafe { AVPlayerItemVideoOutput::initWithPixelBufferAttributes(AVPlayerItemVideoOutput::alloc(), Some(&attrs)) };
    unsafe { item.addOutput(&output) };
    let player = unsafe { AVPlayer::playerWithPlayerItem(Some(&item), mtm) };
    let t0 = Instant::now();
    while unsafe { item.status() } != AVPlayerItemStatus::ReadyToPlay {
        if unsafe { item.status() } == AVPlayerItemStatus::Failed {
            return err(format!("AVPlayerItem failed for {}", path.display()));
        }
        if t0.elapsed() > Duration::from_secs(20) {
            return err("AVPlayerItem never became ready");
        }
        spin(0.01);
    }
    let t = avio::time(t_value, material::TIMESCALE);
    let zero = avio::time(0, 1);
    unsafe { player.seekToTime_toleranceBefore_toleranceAfter(t, zero, zero) };
    let t0 = Instant::now();
    loop {
        if unsafe { output.hasNewPixelBufferForItemTime(t) } {
            if let Some(pb) = unsafe { output.copyPixelBufferForItemTime_itemTimeForDisplay(t, std::ptr::null_mut()) } {
                let p = NonNull::new(Retained::as_ptr(&pb) as *mut CVPixelBuffer).unwrap();
                return Ok(unsafe { PixelBuffer::retain(p) });
            }
        }
        if t0.elapsed() > Duration::from_secs(20) {
            return err("AVPlayerItemVideoOutput produced no frame");
        }
        spin(0.01);
    }
}

#[derive(Serialize)]
pub struct Patch {
    pub name: String,
    pub srgb: [u8; 3],
    pub video_rgb: [f32; 3],
    pub delta_e: f64,
}

#[derive(Serialize)]
pub struct QtCase {
    pub tags: String,
    pub codec: Codec,
    pub container: String,
    pub output_format: String,
    pub display: String,
    pub frame_color_space: String,
    pub frame_attachments: Vec<(String, String)>,
    pub max_delta_e: f64,
    pub mean_delta_e: f64,
    pub worst_patch: String,
    pub pass_max_1: bool,
    pub patches: Vec<Patch>,
}

#[derive(Serialize)]
pub struct QtReport {
    pub method: String,
    pub displays: Vec<(String, String)>,
    pub cases: Vec<QtCase>,
    pub errors: Vec<String>,
}

#[allow(deprecated)]
fn frame_color_space(pb: &PixelBuffer) -> Option<CFRetained<CGColorSpace>> {
    unsafe {
        if let Some(v) = CVBufferCopyAttachment(pb.as_cf(), kCVImageBufferCGColorSpaceKey, std::ptr::null_mut()) {
            let p = CFRetained::into_raw(v).as_ptr() as *mut CGColorSpace;
            return Some(CFRetained::from_raw(NonNull::new_unchecked(p)));
        }
        let att = CVBufferCopyAttachments(pb.as_cf(), CVAttachmentMode::ShouldPropagate)?;
        CVImageBufferCreateColorSpaceFromAttachments(&att)
    }
}

pub fn run(gpu: &Gpu, dir: &Path) -> Result<QtReport> {
    let mtm = MainThreadMarker::new().ok_or_else(|| cf::Error("quicktime experiment must run on the main thread".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| cf::Error(e.to_string()))?;
    let srgb = named(unsafe { kCGColorSpaceSRGB })?;
    let lin = named(unsafe { kCGColorSpaceExtendedLinearSRGB })?;
    let main_display = CGDisplayCopyColorSpace(CGMainDisplayID());
    let mut displays: Vec<(String, CFRetained<CGColorSpace>)> = vec![("Display P3".into(), named(unsafe { kCGColorSpaceDisplayP3 })?)];
    displays.insert(0, ("main display profile".into(), main_display));
    let chart = chart_buffer()?;
    let codes: Vec<[f32; 3]> = CHART.iter().map(|(_, c)| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]).collect();
    let p = PipeParams::standard(Precision::F32);
    let mut cases = vec![];
    let mut errors = vec![];
    for (codec, mp4) in [(Codec::ProRes422HQ, false), (Codec::H264, true), (Codec::Hevc10, true)] {
        let frames: Vec<PixelBuffer> = (0..6).map(|_| gpu.engine_convert(&chart, codec.input_format(), &p)).collect::<Result<_>>()?;
        for tags in exp_tags::candidates() {
            let ext = if mp4 { "mp4" } else { "mov" };
            let path = dir.join(format!("qt_{codec:?}_{}.{ext}", tags.name.split(' ').next().unwrap().replace('?', "x")));
            if let Err(e) = exp_tags::encode_tagged(&path, mp4, codec, &frames, &tags) {
                errors.push(format!("{} {codec:?}: {e}", tags.name));
                continue;
            }
            for out_fmt in [pixbuf::RGBA_HALF, pixbuf::BGRA] {
                match measure(&path, out_fmt, &tags, codec, mp4, &displays, &srgb, &lin, &codes, mtm) {
                    Ok(mut c) => cases.append(&mut c),
                    Err(e) => errors.push(format!("{} {codec:?} '{}': {e}", tags.name, cf::fourcc_str(out_fmt))),
                }
            }
        }
    }
    Ok(QtReport {
        method: "Viewer: sRGB codes → display profile (ColorSync via CGConvertColorDataWithFormat, default intent). \
                 Video: engine encoder input → VideoToolbox → AVAssetWriter with the tag set → AVPlayer + \
                 AVPlayerItemVideoOutput frame 2 → the frame's attached CGColorSpace (or one created from its \
                 attachments) → display profile. Both display values → extended linear sRGB → CIELAB (D65) → CIEDE2000 \
                 on 64×64 patch-center means."
            .into(),
        displays: displays.iter().map(|(n, cs)| (n.clone(), cf::describe_cf(cs))).collect(),
        cases,
        errors,
    })
}

#[allow(clippy::too_many_arguments)]
fn measure(
    path: &Path,
    out_fmt: u32,
    tags: &Tags,
    codec: Codec,
    mp4: bool,
    displays: &[(String, CFRetained<CGColorSpace>)],
    srgb: &CGColorSpace,
    lin: &CGColorSpace,
    codes: &[[f32; 3]],
    mtm: MainThreadMarker,
) -> Result<Vec<QtCase>> {
    let frame = player_frame(path, out_fmt, 2 * material::FRAME_DUR, mtm)?;
    let vcs = frame_color_space(&frame).ok_or_else(|| cf::Error("frame has no color space".into()))?;
    let rgb = sample_patches(&frame)?;
    let mut out = vec![];
    for (dname, dcs) in displays {
        let viewer = convert(&convert(codes, srgb, dcs)?, dcs, lin)?;
        let video = convert(&convert(&rgb, &vcs, dcs)?, dcs, lin)?;
        let mut patches = vec![];
        for (i, (name, c)) in CHART.iter().enumerate() {
            let de = color::delta_e2000(lab_from_linear_srgb(viewer[i]), lab_from_linear_srgb(video[i]));
            patches.push(Patch { name: name.to_string(), srgb: *c, video_rgb: rgb[i], delta_e: de });
        }
        let worst = patches.iter().max_by(|a, b| a.delta_e.total_cmp(&b.delta_e)).unwrap();
        let max = worst.delta_e;
        let mean = patches.iter().map(|p| p.delta_e).sum::<f64>() / patches.len() as f64;
        out.push(QtCase {
            tags: tags.name.into(),
            codec,
            container: if mp4 { "MP4".into() } else { "MOV".into() },
            output_format: cf::fourcc_str(out_fmt),
            display: dname.clone(),
            frame_color_space: cf::describe_cf(&vcs),
            frame_attachments: frame.attachments(),
            max_delta_e: max,
            mean_delta_e: mean,
            worst_patch: worst.name.clone(),
            pass_max_1: max <= 1.0,
            patches,
        });
    }
    Ok(out)
}
