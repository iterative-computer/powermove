//! Section 2's Phase 3b bars through the job engine's export pipeline (render,
//! encode, mux, verify), read back through the codecs:
//!
//! - **Bit depth**: a 16-bit grey ramp across 3,840 columns exported to ProRes
//!   422 HQ (10-bit limited range) holds every luma code 64 to 940, monotonic,
//!   steps of at most 1 CV, Cb = Cr exactly neutral; a chroma ramp covers 64 to
//!   960. A 16-bit PNG export equals the 16-bit source; with default dither an
//!   8-bit output's 16-column local means are within 0.5 CV of ideal.
//! - **Alpha**: ProRes 4444 alpha is exact at the source's 16 bits, straight and
//!   premultiplied outputs carry the right color; HEVC alpha's PSNR is at least
//!   that of AVFoundation's HEVC-with-alpha preset on the same frames.
//! - **SDR color**: a 40-patch chart's patch centers are within 1 CV of the
//!   analytic codes at 8 bits (H.264) and 10 bits (HEVC Main10, ProRes 422 HQ);
//!   ΔE2000 maximum 1.0 and mean 0.5 at 8 bits, maximum 0.5 at 10 bits, decoded
//!   by FFmpeg with zimg and by AVFoundation.
//! - **Audio**: a click at frame N lands at sample floor(N × sr × den / num) ± 1;
//!   total samples equal round(duration × sr).
//!
//! Each prints its measurements (`MEASURE …`) for docs/engine/export.md.

#![cfg(target_os = "macos")]
#![allow(deprecated)]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_av_foundation::*;
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeRange};
use objc2_foundation::{NSString, NSURL};
use pm_export::convert::{EncoderInput, PlaneData};
use pm_export::mac::pixbuf::{PixelBuffer, output_attributes};
use pm_export::mac::reader::decode_frame;
use pm_export::render::*;
use pm_export::timescale::video_timing;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, AudioCoding, Container as WireContainer, DeliveryOptions,
    ExportFormat, ExportSettings, ProResProfile as WireProRes, QualityTier,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::{Value, json};
use support::doc::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-bars3b-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg")
}

fn settings(
    format: ExportFormat,
    alpha: WireAlpha,
    w: u32,
    h: u32,
    frames: u64,
    rate: (u32, u32),
) -> ExportSettings {
    let image = format.is_image();
    ExportSettings {
        format,
        alpha,
        quality: QualityTier::Max,
        size: Size::new(w, h),
        frame_rate: FrameRate::new(rate.0, rate.1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(
                frames as i64 * i64::from(rate.1) * TICKS_PER_SECOND / i64::from(rate.0),
            )
            .unwrap(),
        },
        audio: false,
        motion_blur: false,
        color: if image {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            }
        },
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: DeliveryOptions::default(),
    }
}

/// A 12-bit ProRes 4444 XQ source (with 16-bit alpha when `alpha`) written by
/// the exporter itself from straight RGBA in E: high-precision ramps for the
/// bars (pm-render routes 16-bit PNG stills to the current engine for parity,
/// pm-render.md "Image classes").
fn prores_source(
    path: &Path,
    w: u32,
    h: u32,
    frames: u32,
    alpha: bool,
    f: impl Fn(u32, u32, u32) -> [f64; 4],
) {
    let spec = ExportSpec {
        width: w,
        height: h,
        rate: Rate::FPS_24,
        frames: u64::from(frames),
        codec: Codec::ProRes(ProResProfile::P4444Xq),
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
        alpha: if alpha {
            AlphaOutput::Straight
        } else {
            AlphaOutput::None
        },
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: path.to_path_buf(),
        overwrite: Overwrite::Replace,
        mode: None,
    };
    let mut job = pm_export::export(spec).unwrap();
    assert!(frames >= 2, "a video source needs two frames for its rate");
    for n in 0..frames {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = f(x, y, n);
                rgba.extend([(r * a) as f32, (g * a) as f32, (b * a) as f32, a as f32]);
            }
        }
        job.push_frame(&WorkingFrame {
            width: w,
            height: h,
            rgba: &rgba,
        })
        .unwrap();
    }
    job.finish().unwrap();
}

/// A composition showing the video `path` full frame, untransformed.
fn video_job(path: &Path, settings: ExportSettings, destination: PathBuf) -> ExportJob {
    let (w, h) = (settings.size.width, settings.size.height);
    let dur = 1.0;
    let mut doc = project(
        w,
        h,
        dur,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1 }),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            dur,
        )],
    );
    doc["fps"] = json!(24);
    asset(&mut doc, "clip", "video", "mov", w, h, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", path.to_path_buf());
    job_of(&doc, media, settings, destination)
}

fn job_of(
    doc: &Value,
    media: MediaPaths,
    settings: ExportSettings,
    destination: PathBuf,
) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 3,
    }
}

fn export(job: &ExportJob) -> Delivered {
    let mut renderer = Renderer::headless().unwrap();
    run_export(
        &mut renderer,
        job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{}: {e}", job.destination.display()))
}

fn u16s(input: &EncoderInput, k: usize) -> &[u16] {
    match &input.planes[k].data {
        PlaneData::U16(v) => v,
        PlaneData::U8(_) => panic!("16-bit plane expected"),
    }
}

fn u8s(input: &EncoderInput, k: usize) -> &[u8] {
    match &input.planes[k].data {
        PlaneData::U8(v) => v,
        PlaneData::U16(_) => panic!("8-bit plane expected"),
    }
}

// ---------------------------------------------------------------------------
// Bit depth
// ---------------------------------------------------------------------------

#[test]
fn bit_depth_bar_ramps_through_prores_422_hq_and_16_bit_png() {
    let dir = scratch("depth");
    let (w, h) = (3840u32, 16u32);
    let grey = dir.join("grey-source.mov");
    prores_source(&grey, w, h, 2, false, |x, _, _| {
        let v = x as f64 / (w - 1) as f64;
        [v, v, v, 1.0]
    });
    let fmt = ExportFormat::ProRes {
        profile: WireProRes::Hq,
    };
    let out = dir.join("grey.mov");
    export(&video_job(
        &grey,
        settings(fmt, WireAlpha::None, w, h, 1, (24, 1)),
        out.clone(),
    ));
    let decoded = decode_frame(
        &out,
        0,
        video_timing(Rate::FPS_24).unwrap(),
        InputFormat::V216,
    )
    .unwrap();
    let p = u16s(&decoded, 0);
    let row = 8usize;
    let luma: Vec<u16> = (0..w as usize)
        .map(|x| {
            let i = (row * w as usize + (x & !1)) * 2;
            (if x % 2 == 0 { p[i + 1] } else { p[i + 3] } as f64 / 64.0).round() as u16
        })
        .collect();
    let chroma_neutral = (0..h as usize).all(|y| {
        (0..w as usize / 2).all(|t| {
            let i = (y * w as usize + 2 * t) * 2;
            p[i] == 32768 && p[i + 2] == 32768
        })
    });
    let codes: std::collections::BTreeSet<u16> = luma.iter().copied().collect();
    let max_step = luma
        .windows(2)
        .map(|s| s[1] as i32 - s[0] as i32)
        .max()
        .unwrap();
    let monotonic = luma.windows(2).all(|s| s[1] >= s[0]);
    let lo_hi = (*codes.first().unwrap(), *codes.last().unwrap());
    eprintln!(
        "MEASURE bit depth (ProRes 422 HQ, grey ramp, {w} columns): {} luma codes {lo_hi:?}, monotonic {monotonic}, largest step {max_step} CV, Cb = Cr neutral {chroma_neutral}",
        codes.len()
    );
    assert_eq!(lo_hi, (64, 940));
    assert_eq!(codes.len(), 877, "every luma code 64 to 940");
    assert!(monotonic && max_step <= 1);
    assert!(chroma_neutral, "Cb = Cr exactly neutral");

    // Chroma ramp: yellow (Cb −0.5) to blue (Cb +0.5) in E.
    let chroma = dir.join("chroma-source.mov");
    prores_source(&chroma, w, h, 2, false, |x, _, _| {
        let t = x as f64 / (w - 1) as f64;
        [1.0 - t, 1.0 - t, t, 1.0]
    });
    let out = dir.join("chroma.mov");
    export(&video_job(
        &chroma,
        settings(fmt, WireAlpha::None, w, h, 1, (24, 1)),
        out.clone(),
    ));
    let decoded = decode_frame(
        &out,
        0,
        video_timing(Rate::FPS_24).unwrap(),
        InputFormat::V216,
    )
    .unwrap();
    let p = u16s(&decoded, 0);
    let cb: Vec<u16> = (0..w as usize / 2)
        .map(|t| (p[(row * w as usize + 2 * t) * 2] as f64 / 64.0).round() as u16)
        .collect();
    let codes: std::collections::BTreeSet<u16> = cb.iter().copied().collect();
    let lo_hi = (*codes.first().unwrap(), *codes.last().unwrap());
    eprintln!(
        "MEASURE bit depth (ProRes 422 HQ, chroma ramp): {} Cb codes {lo_hi:?}",
        codes.len()
    );
    assert_eq!(lo_hi, (64, 960));
    assert_eq!(codes.len(), 897, "every chroma code 64 to 960");

    // 16-bit PNG output (the PNG writer from a float ramp): every column's
    // code, monotonic, steps of at most ⌈65535 / (columns − 1)⌉.
    let out = dir.join("grey16.png");
    let spec = ExportSpec {
        width: w,
        height: h,
        rate: Rate::FPS_24,
        frames: 1,
        codec: Codec::Png { sixteen_bit: true },
        container: Container::PngStill,
        tags: ColorTags::SrgbImage,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: out.clone(),
        overwrite: Overwrite::Fail,
        mode: None,
    };
    let mut png_job = pm_export::export(spec).unwrap();
    let rgba: Vec<f32> = (0..h)
        .flat_map(|_| {
            (0..w).flat_map(|x| {
                let v = (x as f64 / (w - 1) as f64) as f32;
                [v, v, v, 1.0]
            })
        })
        .collect();
    png_job
        .push_frame(&WorkingFrame {
            width: w,
            height: h,
            rgba: &rgba,
        })
        .unwrap();
    png_job.finish().unwrap();
    let (_, read) = pm_export::png::read(&out, true).unwrap();
    let r = read.unwrap();
    let px = u16s(&r, 0);
    let reds: Vec<u16> = (0..w as usize)
        .map(|x| px[(row * w as usize + x) * 4])
        .collect();
    // The ideal code of each column's f32 working value.
    let ideal16 = |x: u32| ((x as f64 / (w - 1) as f64) as f32 as f64 * 65535.0).round() as u16;
    let exact = reds
        .iter()
        .enumerate()
        .all(|(x, &v)| v == ideal16(x as u32));
    let step = reds
        .windows(2)
        .map(|s| s[1] as i32 - s[0] as i32)
        .max()
        .unwrap();
    let monotonic = reds.windows(2).all(|s| s[1] >= s[0]);
    eprintln!(
        "MEASURE bit depth (16-bit PNG): every column on its ideal code {exact}, monotonic {monotonic}, largest step {step}"
    );
    assert!(exact && monotonic);
    assert!(step as u32 <= 65535u32.div_ceil(w - 1));

    // Default dither on an 8-bit output: 16-column local means within 0.5 CV.
    let (sw, sh) = (1920u32, 64u32);
    let shallow = |x: u32| 0.40 + 0.02 * x as f64 / (sw - 1) as f64;
    let src = dir.join("shallow.mov");
    prores_source(&src, sw, sh, 2, false, |x, _, _| {
        let v = shallow(x);
        [v, v, v, 1.0]
    });
    let mut s = settings(
        ExportFormat::PngStill { sixteen_bit: false },
        WireAlpha::None,
        sw,
        sh,
        1,
        (24, 1),
    );
    s.dither = true;
    let out = dir.join("shallow8.png");
    export(&video_job(&src, s, out.clone()));
    let (_, read) = pm_export::png::read(&out, true).unwrap();
    let r = read.unwrap();
    let px = u8s(&r, 0);
    let column = |x: usize| {
        (0..sh as usize)
            .map(|y| px[(y * sw as usize + x) * 4] as f64)
            .sum::<f64>()
            / sh as f64
    };
    // The source's own 12-bit codes are the ideal: E codes × 255.
    let ideal = |x: usize| {
        let code12 = (shallow(x as u32) * 3504.0 + 256.0).round();
        (code12 - 256.0) / 3504.0 * 255.0
    };
    let worst = (0..sw as usize - 16)
        .map(|x0| {
            ((x0..x0 + 16).map(column).sum::<f64>() / 16.0
                - (x0..x0 + 16).map(ideal).sum::<f64>() / 16.0)
                .abs()
        })
        .fold(0.0, f64::max);
    eprintln!(
        "MEASURE bit depth (8-bit PNG, default dither): worst 16-column mean error {worst:.3} CV"
    );
    assert!(worst <= 0.5);
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// Alpha
// ---------------------------------------------------------------------------

/// The straight color of the alpha source at row `y`.
fn alpha_color(y: u32, h: u32) -> [f64; 3] {
    let c = y as f64 / (h - 1) as f64;
    [1.0 - c / 2.0, c, 0.3]
}

/// The alpha source's alpha at column `x`: a ramp over all 16-bit codes.
fn alpha_code(x: u32, w: u32) -> u16 {
    ((x as f64 / (w - 1) as f64) * 65535.0).round() as u16
}

fn alpha_source(dir: &Path, w: u32, h: u32, frames: u32) -> PathBuf {
    let path = dir.join("alpha-source.mov");
    prores_source(&path, w, h, frames, true, |x, y, _| {
        let [r, g, b] = alpha_color(y, h);
        [r, g, b, alpha_code(x, w) as f64 / 65535.0]
    });
    path
}

#[test]
fn alpha_bar_prores_4444_exact_straight_and_premultiplied() {
    let dir = scratch("alpha");
    let (w, h) = (1024u32, 32u32);
    let src = alpha_source(&dir, w, h, 2);
    let timing = video_timing(Rate::FPS_24).unwrap();
    // The source's alpha samples, as VideoToolbox decodes them.
    let source = decode_frame(&src, 0, timing, InputFormat::Y416).unwrap();
    let src_alpha: Vec<u16> = u16s(&source, 0).chunks_exact(4).map(|p| p[0]).collect();
    let src_exact = (0..h as usize)
        .all(|y| (0..w as usize).all(|x| src_alpha[y * w as usize + x] == alpha_code(x as u32, w)));
    assert!(src_exact, "the 16-bit alpha source decodes to its codes");
    for mode in [WireAlpha::Straight, WireAlpha::Premultiplied] {
        let fmt = ExportFormat::ProRes {
            profile: WireProRes::P4444,
        };
        let out = dir.join(format!("{mode:?}.mov"));
        export(&video_job(
            &src,
            settings(fmt, mode, w, h, 1, (24, 1)),
            out.clone(),
        ));
        let y416 = decode_frame(&out, 0, timing, InputFormat::Y416).unwrap();
        let p = u16s(&y416, 0);
        let mut alpha_exact = true;
        let mut worst_color = 0.0f64;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                let a = alpha_code(x as u32, w);
                alpha_exact &= p[i] == a;
                let af = a as f64 / 65535.0;
                // Below 1/64 the color of straight output is a division by a tiny
                // alpha; compare where the color carries weight.
                if af < 1.0 / 64.0 {
                    continue;
                }
                let rgb = alpha_color(y as u32, h);
                let rgb = if mode == WireAlpha::Premultiplied {
                    rgb.map(|v| v * af)
                } else {
                    rgb
                };
                let want = pm_export::convert::rgb_to_ycbcr(rgb);
                let got = [
                    (p[i + 1] as f64 - 4096.0) / 56064.0,
                    (p[i + 2] as f64 - 32768.0) / 57344.0,
                    (p[i + 3] as f64 - 32768.0) / 57344.0,
                ];
                // 12-bit code values.
                let err = (0..3)
                    .map(|k| (got[k] - want[k]).abs() * if k == 0 { 3504.0 } else { 3584.0 })
                    .fold(0.0, f64::max);
                worst_color = worst_color.max(err);
            }
        }
        eprintln!(
            "MEASURE alpha (ProRes 4444, {mode:?}): alpha exact at 16 bits {alpha_exact}, worst color {worst_color:.2} CV at 12 bits (α ≥ 1/64)"
        );
        assert!(alpha_exact, "{mode:?}: alpha exact at the source's 16 bits");
        assert!(worst_color <= 4.0, "{mode:?}: color {worst_color}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// Re-encodes `src` with an AVAssetExportSession preset.
fn apple_export(src: &Path, dst: &Path, preset: &NSString) {
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&file_url(src), None) };
    let session = unsafe {
        AVAssetExportSession::initWithAsset_presetName(
            AVAssetExportSession::alloc(),
            &asset,
            preset,
        )
    }
    .expect("export session");
    unsafe {
        session.setOutputURL(Some(&file_url(dst)));
        session.setOutputFileType(AVFileTypeQuickTimeMovie);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let done = RcBlock::new(move || {
        let _ = tx.send(());
    });
    unsafe { session.exportAsynchronouslyWithCompletionHandler(&done) };
    rx.recv_timeout(std::time::Duration::from_secs(300))
        .unwrap();
    assert_eq!(
        unsafe { session.status() },
        AVAssetExportSessionStatus::Completed,
        "{:?}",
        unsafe { session.error() }
    );
}

/// Decodes frame `index` of `path` to `v0a8` (HEVC with alpha) and returns its
/// alpha plane.
fn hevc_alpha(path: &Path, index: u64, timing: pm_export::timescale::VideoTiming) -> Vec<u8> {
    let decoded = decode_frame(path, index, timing, InputFormat::V0a8).unwrap();
    u8s(&decoded, 2).to_vec()
}

fn psnr8(a: &[u8], b: &[u8]) -> f64 {
    let mse = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| (x as f64 - y as f64).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

#[test]
fn alpha_bar_hevc_alpha_psnr_matches_avfoundations_preset() {
    let dir = scratch("hevc-alpha");
    let (w, h) = (640u32, 360u32);
    let src = alpha_source(&dir, w, h, 24);
    // A ProRes 4444 master with alpha of moving material is the shared source.
    let dur = 1.0;
    let mut layers = vec![layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1 }),
        f64::from(w) / 2.0,
        f64::from(h) / 2.0,
        dur,
    )];
    set(
        &mut layers[0],
        "rotation",
        keys(json!(0), &[(0.0, json!(0)), (dur, json!(20))]),
    );
    let mut doc = project(w, h, dur, no_fill(), layers);
    doc["fps"] = json!(24);
    asset(&mut doc, "clip", "video", "mov", w, h, 1.0);
    let mut media = MediaPaths::default();
    media.insert("clip", src.clone());
    let frames = 24u64;
    let master = dir.join("master.mov");
    export(&job_of(
        &doc,
        media.clone(),
        settings(
            ExportFormat::ProRes {
                profile: WireProRes::P4444,
            },
            WireAlpha::Straight,
            w,
            h,
            frames,
            (24, 1),
        ),
        master.clone(),
    ));
    let engine = dir.join("engine.mov");
    let mut s = settings(
        ExportFormat::Hevc {
            ten_bit: false,
            container: WireContainer::Mov,
        },
        WireAlpha::Straight,
        w,
        h,
        frames,
        (24, 1),
    );
    s.quality = QualityTier::Max;
    export(&job_of(&doc, media, s, engine.clone()));
    let apple = dir.join("apple.mov");
    apple_export(&master, &apple, unsafe {
        AVAssetExportPresetHEVCHighestQualityWithAlpha
    });
    let timing = video_timing(Rate::FPS_24).unwrap();
    let apple_timing = timing;
    let (mut ours, mut theirs, mut n) = (0.0, 0.0, 0);
    for f in [0u64, 11, 23] {
        let reference: Vec<u8> = {
            let y416 = decode_frame(&master, f, timing, InputFormat::Y416).unwrap();
            u16s(&y416, 0)
                .chunks_exact(4)
                .map(|p| ((p[0] as f64) / 257.0).round() as u8)
                .collect()
        };
        let e = psnr8(&hevc_alpha(&engine, f, timing), &reference);
        let a = psnr8(&hevc_alpha(&apple, f, apple_timing), &reference);
        eprintln!(
            "MEASURE alpha (HEVC with alpha, frame {f}): engine {e:.2} dB, AVFoundation preset {a:.2} dB"
        );
        ours += e;
        theirs += a;
        n += 1;
    }
    let (ours, theirs) = (ours / n as f64, theirs / n as f64);
    let bytes = |p: &Path| std::fs::metadata(p).unwrap().len();
    eprintln!(
        "MEASURE alpha (HEVC with alpha): mean alpha PSNR engine {ours:.2} dB ({} bytes), AVFoundation {theirs:.2} dB ({} bytes)",
        bytes(&engine),
        bytes(&apple)
    );
    assert!(
        ours + 1e-9 >= theirs,
        "HEVC alpha PSNR {ours:.2} dB below AVFoundation's {theirs:.2} dB"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// SDR color
// ---------------------------------------------------------------------------

/// The chart: ColorChecker (sRGB D65 values), a grey scale, primaries and
/// secondaries.
const CHART: [[u8; 3]; 40] = [
    [115, 82, 68],
    [194, 150, 130],
    [98, 122, 157],
    [87, 108, 67],
    [133, 128, 177],
    [103, 189, 170],
    [214, 126, 44],
    [80, 91, 166],
    [193, 90, 99],
    [94, 60, 108],
    [157, 188, 64],
    [224, 163, 46],
    [56, 61, 150],
    [70, 148, 73],
    [175, 54, 60],
    [231, 199, 31],
    [187, 86, 149],
    [8, 133, 161],
    [243, 243, 242],
    [200, 200, 200],
    [160, 160, 160],
    [122, 122, 121],
    [85, 85, 85],
    [52, 52, 52],
    [0, 0, 0],
    [28, 28, 28],
    [56, 56, 56],
    [84, 84, 84],
    [113, 113, 113],
    [141, 141, 141],
    [170, 170, 170],
    [198, 198, 198],
    [226, 226, 226],
    [255, 255, 255],
    [255, 0, 0],
    [0, 255, 0],
    [0, 0, 255],
    [0, 255, 255],
    [255, 0, 255],
    [255, 255, 0],
];

const CW: u32 = 1280;
const CH: u32 = 640;
const COLS: u32 = 8;
const ROWS: u32 = 5;

fn chart_doc() -> Value {
    let (pw, ph) = (CW / COLS, CH / ROWS);
    let layers = CHART
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (col, row) = (i as u32 % COLS, i as u32 / COLS);
            layer(
                &format!("p{i}"),
                "solid",
                json!({ "color": format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]), "w": pw, "h": ph }),
                // A solid's position is its top-left corner (anchor 0, 0).
                f64::from(col * pw),
                f64::from(row * ph),
                1.0,
            )
        })
        .collect();
    project(CW, CH, 1.0, no_fill(), layers)
}

fn srgb_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    let lin = rgb.map(|v| {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let x = 0.412_456_4 * lin[0] + 0.357_576_1 * lin[1] + 0.180_437_5 * lin[2];
    let y = 0.212_672_9 * lin[0] + 0.715_152_2 * lin[1] + 0.072_175_0 * lin[2];
    let z = 0.019_333_9 * lin[0] + 0.119_192_0 * lin[1] + 0.950_304_1 * lin[2];
    let (xn, yn, zn) = (0.950_47, 1.0, 1.088_83);
    let f = |t: f64| {
        if t > (6.0f64 / 29.0).powi(3) {
            t.cbrt()
        } else {
            t / (3.0 * (6.0f64 / 29.0).powi(2)) + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (f(x / xn), f(y / yn), f(z / zn));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 (Sharma, Wu, and Dalal 2005).
fn delta_e_2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |b: f64, ap: f64| {
        if b == 0.0 && ap == 0.0 {
            0.0
        } else {
            let h = b.atan2(ap).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hm).to_radians().cos()
        + 0.32 * (3.0 * hm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hm - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(2.0 * dtheta.to_radians()).sin() * rc;
    let _ = PI;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
        .sqrt()
}

/// RGB of frame 0 decoded by FFmpeg with zimg (limited-range BT.709 matrix, no
/// transfer change: the codes are E), as interleaved 16-bit samples. zimg
/// writes planar `gbrp16le` itself; asking for packed `rgb48le` would make
/// FFmpeg convert through 8-bit `gbrp` first.
fn ffmpeg_rgb48(path: &Path) -> Vec<u16> {
    let out = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "zscale=matrixin=709:rangein=limited:range=full:filter=bicubic:dither=none,format=gbrp16le",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let planar: Vec<u16> = out
        .stdout
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let n = planar.len() / 3;
    // Planes are G, B, R.
    (0..n)
        .flat_map(|i| [planar[2 * n + i], planar[i], planar[n + i]])
        .collect()
}

/// RGB of frame 0 decoded by AVFoundation to 32BGRA (the reader converts with
/// the stream's matrix and range).
fn avfoundation_bgra(path: &Path) -> Vec<u8> {
    let url = file_url(path);
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
    let track = unsafe { asset.tracksWithMediaType(AVMediaTypeVideo.unwrap()) }
        .firstObject()
        .unwrap();
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }.unwrap();
    let settings = output_attributes(u32::from_be_bytes(*b"BGRA"));
    let output = unsafe {
        AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            &track,
            Some(&settings),
        )
    };
    unsafe {
        reader.addOutput(&output);
        reader.setTimeRange(CMTimeRange {
            start: CMTime::new(0, 24000),
            duration: CMTime::new(1000, 24000),
        });
        assert!(reader.startReading());
    }
    let sb = unsafe { output.copyNextSampleBuffer() }.expect("a frame");
    let sb: &CMSampleBuffer = &sb;
    let image = unsafe { sb.image_buffer() }.unwrap();
    let pb = unsafe { PixelBuffer::retain(std::ptr::NonNull::from(&*image)) };
    let bytes = pb.bytes().unwrap();
    unsafe { reader.cancelReading() };
    bytes
}

#[test]
fn sdr_color_bar_chart_through_h264_hevc10_and_prores() {
    let dir = scratch("sdr");
    let doc = chart_doc();
    let (pw, ph) = (CW / COLS, CH / ROWS);
    let block = 24u32;
    let cases = [
        (
            "h264",
            ExportFormat::H264,
            InputFormat::V420,
            8,
            1.0,
            Some(0.5),
        ),
        (
            "hevc10",
            ExportFormat::Hevc {
                ten_bit: true,
                container: WireContainer::Mp4,
            },
            InputFormat::X420,
            10,
            0.5,
            None,
        ),
        (
            "prores-hq",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            InputFormat::V216,
            10,
            0.5,
            None,
        ),
    ];
    for (name, format, input, depth, max_de, mean_de) in cases {
        let ext = if matches!(format, ExportFormat::ProRes { .. }) {
            "mov"
        } else {
            "mp4"
        };
        let out = dir.join(format!("{name}.{ext}"));
        let mut s = settings(format, WireAlpha::None, CW, CH, 2, (24000, 1001));
        s.dither = false;
        export(&job_of(&doc, MediaPaths::default(), s, out.clone()));
        let decoded =
            decode_frame(&out, 0, video_timing(Rate::FPS_23_976).unwrap(), input).unwrap();
        let scale = 2f64.powi(depth - 8);
        let mut worst_cv = 0.0f64;
        let rgb48 = ffmpeg_rgb48(&out);
        let bgra = avfoundation_bgra(&out);
        let (mut de_ff, mut de_av, mut de_ideal) = (Vec::new(), Vec::new(), Vec::new());
        for (i, c) in CHART.iter().enumerate() {
            let (col, row) = (i as u32 % COLS, i as u32 / COLS);
            let (cx, cy) = (col * pw + pw / 2 - block / 2, row * ph + ph / 2 - block / 2);
            let want_rgb = c.map(|v| v as f64 / 255.0);
            let want = pm_export::convert::rgb_to_ycbcr(want_rgb);
            let want_codes = [
                (16.0 + 219.0 * want[0]) * scale,
                (128.0 + 224.0 * want[1]) * scale,
                (128.0 + 224.0 * want[2]) * scale,
            ];
            let (mut sum, mut ff, mut av) = ([0.0f64; 3], [0.0f64; 3], [0.0f64; 3]);
            for y in cy..cy + block {
                for x in cx..cx + block {
                    let s = pm_export::convert::sample_ycbcra(&decoded, x as usize, y as usize);
                    sum[0] += (16.0 + 219.0 * s[0]) * scale;
                    sum[1] += (128.0 + 224.0 * s[1]) * scale;
                    sum[2] += (128.0 + 224.0 * s[2]) * scale;
                    let i = (y * CW + x) as usize;
                    for k in 0..3 {
                        ff[k] += rgb48[i * 3 + k] as f64 / 65535.0;
                    }
                    av[0] += bgra[i * 4 + 2] as f64 / 255.0;
                    av[1] += bgra[i * 4 + 1] as f64 / 255.0;
                    av[2] += bgra[i * 4] as f64 / 255.0;
                }
            }
            let n = (block * block) as f64;
            for k in 0..3 {
                worst_cv = worst_cv.max((sum[k] / n - want_codes[k]).abs());
            }
            if std::env::var_os("PM_BARS_VERBOSE").is_some() {
                eprintln!(
                    "patch {c:?}: codes {:?} want {:?}; ffmpeg rgb {:?}; avf rgb {:?}",
                    sum.map(|v| (v / n * 100.0).round() / 100.0),
                    want_codes.map(|v| (v * 100.0).round() / 100.0),
                    ff.map(|v| (v / n * 255.0 * 100.0).round() / 100.0),
                    av.map(|v| (v / n * 255.0 * 100.0).round() / 100.0)
                );
            }
            de_ff.push(delta_e_2000(
                srgb_to_lab(want_rgb),
                srgb_to_lab(ff.map(|v| v / n)),
            ));
            de_av.push(delta_e_2000(
                srgb_to_lab(want_rgb),
                srgb_to_lab(av.map(|v| v / n)),
            ));
            // A perfect codec at this depth: the analytic codes, rounded, back to RGB.
            let q = want_codes.map(|v| v.round());
            let back = pm_export::convert::ycbcr_to_rgb(
                (q[0] / scale - 16.0) / 219.0,
                (q[1] / scale - 128.0) / 224.0,
                (q[2] / scale - 128.0) / 224.0,
            );
            de_ideal.push(delta_e_2000(
                srgb_to_lab(want_rgb),
                srgb_to_lab(back.map(|v| v.clamp(0.0, 1.0))),
            ));
        }
        let worst_patch = |v: &[f64]| {
            v.iter()
                .enumerate()
                .fold((0, 0.0), |m, (i, &d)| if d > m.1 { (i, d) } else { m })
        };
        let (wi, wd) = worst_patch(&de_ff);
        let (ii, idl) = worst_patch(&de_ideal);
        eprintln!(
            "MEASURE SDR color ({name}): worst FFmpeg patch {:?} ΔE {wd:.3}; quantization alone (analytic codes at {depth} bits) worst {:?} ΔE {idl:.3}",
            CHART[wi], CHART[ii]
        );
        let stats = |v: &[f64]| {
            (
                v.iter().copied().fold(0.0, f64::max),
                v.iter().sum::<f64>() / v.len() as f64,
            )
        };
        let (ff_max, ff_mean) = stats(&de_ff);
        let (av_max, av_mean) = stats(&de_av);
        eprintln!(
            "MEASURE SDR color ({name}, {depth}-bit): patch centers within {worst_cv:.3} CV; ΔE2000 FFmpeg+zimg max {ff_max:.3} mean {ff_mean:.3}; AVFoundation (8-bit BGRA) max {av_max:.3} mean {av_mean:.3}"
        );
        assert!(worst_cv <= 1.0, "{name}: patch centers {worst_cv} CV");
        assert!(ff_max <= max_de, "{name}: FFmpeg ΔE2000 max {ff_max}");
        if let Some(mean) = mean_de {
            assert!(ff_mean <= mean, "{name}: FFmpeg ΔE2000 mean {ff_mean}");
        }
        // AVFoundation's reader delivers 8-bit BGRA: judged at the 8-bit bars.
        assert!(
            av_max <= 1.0 && av_mean <= 0.5,
            "{name}: AVFoundation ΔE2000 max {av_max} mean {av_mean}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// A mono 32-bit float WAV at 48 kHz: a single full-scale click at sample 0.
fn click_wav(path: &Path, seconds: f64) {
    let n = (seconds * 48_000.0) as u32;
    let mut d = Vec::new();
    d.extend(b"RIFF");
    d.extend((36 + n * 4).to_le_bytes());
    d.extend(b"WAVEfmt ");
    d.extend(16u32.to_le_bytes());
    d.extend(3u16.to_le_bytes());
    d.extend(1u16.to_le_bytes());
    d.extend(48_000u32.to_le_bytes());
    d.extend((48_000u32 * 4).to_le_bytes());
    d.extend(4u16.to_le_bytes());
    d.extend(32u16.to_le_bytes());
    d.extend(b"data");
    d.extend((n * 4).to_le_bytes());
    for i in 0..n {
        d.extend((if i == 0 { 1.0f32 } else { 0.0 }).to_le_bytes());
    }
    std::fs::write(path, d).unwrap();
}

/// The audio samples of a file as f32 (FFmpeg decode), first channel.
fn decoded_audio(path: &Path) -> Vec<f32> {
    let out = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "1", "-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

#[test]
fn audio_bar_clicks_land_on_their_samples_and_totals_match() {
    let dir = scratch("audio");
    let click = dir.join("click.wav");
    click_wav(&click, 0.5);
    for (num, den) in [(24000u32, 1001u32), (25, 1), (30000, 1001), (60000, 1001)] {
        for n in [0u64, 1, 7, 37] {
            let fps = num as f64 / den as f64;
            let dur = 2.0;
            let mut a = audio("click", "click", n as f64 / fps, 0.5);
            a["dur"] = json!(0.5);
            let mut doc = project(64, 36, dur, solid_fill("#000000"), vec![a]);
            doc["fps"] = json!(fps);
            asset(&mut doc, "click", "audio", "wav", 0, 0, 0.5);
            let mut media = MediaPaths::default();
            media.insert("click", click.clone());
            let frames = (dur * fps).round() as u64;
            let mut s = settings(
                ExportFormat::ProRes {
                    profile: WireProRes::Proxy,
                },
                WireAlpha::None,
                64,
                36,
                frames,
                (num, den),
            );
            s.audio = true;
            s.delivery.audio_coding = Some(AudioCoding::PcmFloat);
            let out = dir.join(format!("click-{num}-{den}-{n}.mov"));
            let delivered = export(&job_of(&doc, media, s, out.clone()));
            let samples = decoded_audio(&out);
            let at = samples.iter().position(|&v| v > 0.5).unwrap_or_else(|| {
                panic!(
                    "{num}/{den} frame {n}: no click in {} samples, peak {}",
                    samples.len(),
                    samples.iter().fold(0f32, |m, &v| m.max(v.abs()))
                )
            });
            let want = (n as u128 * 48_000 * den as u128 / num as u128) as usize;
            let total = (frames as f64 * den as f64 / num as f64 * 48_000.0).round() as usize;
            assert!(
                at.abs_diff(want) <= 1,
                "{num}/{den} frame {n}: click at {at}, expected {want}"
            );
            assert_eq!(samples.len(), total, "{num}/{den}: total samples");
            assert_eq!(
                delivered.report.audio.as_ref().unwrap().samples as usize,
                total
            );
            if n == 37 {
                eprintln!(
                    "MEASURE audio ({num}/{den}): click at frame {n} lands at sample {at} (expected {want}); {} samples = round(duration × sr)",
                    samples.len()
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
