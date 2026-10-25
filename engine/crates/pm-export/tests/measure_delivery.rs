//! Phase 3b measurements (ignored; run explicitly, in release):
//!
//! - `vmaf_h264_against_x264`: section 17 asks for a VMAF comparison of the
//!   VideoToolbox H.264 encoder against x264 to decide whether a software
//!   encoder is also offered. Both encoders take the identical encoder input
//!   (the engine's dithered `420v` frames of a natural UHD ProRes clip scaled
//!   to 1080p), at today's tiers (4, 16, 40 Mbps); VMAF (libvmaf v2 model) is
//!   computed against that input. x264 and libvmaf come from a GPL FFmpeg on
//!   PATH (`PM_MEASURE_FFMPEG`, default `/opt/homebrew/bin/ffmpeg`): they are
//!   measurement tools only and never ship.
//! - `export_throughput`: frames per second of the job engine's export
//!   (render, convert, encode, mux, verify) for every format at cutover at
//!   1080p, and UHD ProRes 422 HQ with three layers (section 25's 3b rows).
//!
//! ```sh
//! cargo test --release -p pm-export --test measure_delivery -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(target_os = "macos")]

mod support;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{MatrixCoefficients, OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, Container as WireContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, QualityTier, WebmCodec,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::json;
use support::doc::*;

fn corpus(name: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/media-corpus")
        });
    let p = dir.join(name);
    p.exists().then_some(p)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-measure-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn gpl_ffmpeg() -> Option<PathBuf> {
    let p = std::env::var_os("PM_MEASURE_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/homebrew/bin/ffmpeg"));
    p.exists().then_some(p)
}

const FRAMES: u64 = 48;

fn settings(
    format: ExportFormat,
    alpha: WireAlpha,
    w: u32,
    h: u32,
    quality: QualityTier,
) -> ExportSettings {
    let image = format.is_image();
    ExportSettings {
        format,
        alpha,
        quality,
        size: Size::new(w, h),
        frame_rate: FrameRate::new(24000, 1001),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(FRAMES as i64 * 1001 * TICKS_PER_SECOND / 24000).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: if image {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            }
        },
        precision: Precision::Float32,
        dither: !matches!(
            format,
            ExportFormat::ProRes { .. } | ExportFormat::Hevc { ten_bit: true, .. }
        ),
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

/// The UHD clip full frame at `w` × `h`, with `extra` solids moving over it.
fn clip_job(
    clip: &Path,
    settings: ExportSettings,
    destination: PathBuf,
    extra: usize,
) -> ExportJob {
    let (w, h) = (settings.size.width, settings.size.height);
    let dur = FRAMES as f64 * 1001.0 / 24000.0;
    let mut layers = vec![];
    for i in 0..extra {
        let mut s = layer(
            &format!("s{i}"),
            "solid",
            json!({ "color": if i % 2 == 0 { "#E83A5E" } else { "#3FCF8E" }, "w": w / 4, "h": h / 4 }),
            0.0,
            f64::from(h) * (0.2 + 0.4 * i as f64),
            dur,
        );
        set(
            &mut s,
            "position.x",
            keys(
                json!(0),
                &[(0.0, json!(0)), (dur, json!(f64::from(w) * 0.7))],
            ),
        );
        set(&mut s, "opacity", json!(70));
        layers.push(s);
    }
    layers.push(layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": w, "h": h, "trim": 0, "speed": 1 }),
        f64::from(w) / 2.0,
        f64::from(h) / 2.0,
        dur,
    ));
    let mut doc = project(w, h, dur, no_fill(), layers);
    doc["fps"] = json!(23.976);
    asset(&mut doc, "clip", "video", "mov", 3840, 2160, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", clip.to_path_buf());
    ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 5,
    }
}

/// A renderer reading the corpus clip with the Rec.709 matrix (the corpus
/// files are tagged BT.601).
fn renderer_for(path: &Path) -> Renderer {
    let mut r = Renderer::headless().unwrap();
    let mut c = r.video_color(path).unwrap();
    c.matrix = MatrixCoefficients::Bt709;
    c.primaries = pm_protocol::color::Primaries::Bt709;
    r.interpret(path, Some(c));
    r
}

/// Writes `420v` frames as a 4:2:0 Y4M (planar).
fn write_y4m(path: &Path, frames: &[pm_export::EncoderInput]) {
    let f = &frames[0];
    let (w, h) = (f.width as usize, f.height as usize);
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(
        out,
        "YUV4MPEG2 W{w} H{h} F24000:1001 Ip A1:1 C420mpeg2 XCOLORRANGE=LIMITED"
    )
    .unwrap();
    for f in frames {
        let (pm_export::convert::PlaneData::U8(y), pm_export::convert::PlaneData::U8(c)) =
            (&f.planes[0].data, &f.planes[1].data)
        else {
            panic!("420v frames");
        };
        assert_eq!(
            (f.planes[0].row_samples, f.planes[1].row_samples),
            (w, w),
            "packed rows"
        );
        out.write_all(b"FRAME\n").unwrap();
        out.write_all(y).unwrap();
        let cb: Vec<u8> = c.chunks_exact(2).map(|p| p[0]).collect();
        let cr: Vec<u8> = c.chunks_exact(2).map(|p| p[1]).collect();
        assert_eq!(cb.len(), w / 2 * h / 2);
        out.write_all(&cb).unwrap();
        out.write_all(&cr).unwrap();
    }
}

/// Y4M carries no matrix: without these tags FFmpeg assumes BT.601 and
/// converts the reference to BT.709 on the way into x264 and libvmaf, which
/// is a different picture. Every stream is tagged as what it is.
const TAGS: &str =
    "setparams=colorspace=bt709:color_primaries=bt709:color_trc=iec61966-2-1:range=tv";

fn vmaf(ffmpeg: &Path, distorted: &Path, reference: &Path, log: &Path) -> f64 {
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(distorted)
        .arg("-i")
        .arg(reference)
        .args(["-lavfi", &format!("[0:v]setpts=PTS-STARTPTS,{TAGS}[d];[1:v]setpts=PTS-STARTPTS,{TAGS}[r];[d][r]libvmaf=log_fmt=json:log_path={}:n_threads=4", log.display())])
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
    report["pooled_metrics"]["vmaf"]["mean"].as_f64().unwrap()
}

#[test]
#[ignore = "measurement: VMAF of VideoToolbox H.264 against x264 (needs a GPL FFmpeg with libx264 and libvmaf)"]
fn vmaf_h264_against_x264() {
    let (Some(bars), Some(ff)) = (corpus("uhd-prores422hq.mov"), gpl_ffmpeg()) else {
        eprintln!("no UHD corpus clip or GPL FFmpeg: not measured");
        return;
    };
    let dir = scratch("vmaf");
    let mut sources = vec![("color bars", bars)];
    if let Some(landscape) = landscape_source(&ff, &dir) {
        sources.insert(0, ("landscape pan with grain", landscape));
    }
    for (source, clip) in sources {
        measure_vmaf(&ff, &dir, source, &clip);
    }
    if std::env::var_os("PM_MEASURE_KEEP").is_none() {
        std::fs::remove_dir_all(dir).unwrap();
    } else {
        eprintln!("kept {}", dir.display());
    }
}

/// A UHD ProRes 422 HQ clip with natural gradients, motion, and film grain:
/// `PM_MEASURE_SOURCE`, or a pan across macOS's Sonoma desktop picture with
/// temporal noise (made in the scratch folder; never committed).
fn landscape_source(ff: &Path, dir: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PM_MEASURE_SOURCE") {
        return Some(PathBuf::from(p));
    }
    let picture = Path::new("/System/Library/Desktop Pictures/Sonoma.heic");
    if !picture.exists() {
        return None;
    }
    let png = dir.join("landscape.png");
    let ok = Command::new("sips")
        .args(["-s", "format", "png"])
        .arg(picture)
        .arg("--out")
        .arg(&png)
        .output()
        .ok()?
        .status
        .success();
    let clip = dir.join("landscape.mov");
    let ok = ok
        && Command::new(ff)
            .args(["-v", "error", "-y", "-loop", "1", "-framerate", "24000/1001", "-i"])
            .arg(&png)
            .args([
                "-vf",
                &format!("scale=4800:-2:flags=lanczos,crop=3840:2160:x='40+n*12':y='60+n*4',noise=alls=5:allf=t,format=yuv422p10le,{TAGS}"),
                "-frames:v",
                &FRAMES.to_string(),
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-vendor",
                "apl0",
            ])
            .arg(&clip)
            .output()
            .ok()?
            .status
            .success();
    ok.then_some(clip)
}

fn measure_vmaf(ff: &Path, dir: &Path, source: &str, clip: &Path) {
    let (w, h) = (1920u32, 1080u32);
    // The encoder input both encoders take: the engine's dithered 420v frames.
    let s = settings(ExportFormat::H264, WireAlpha::None, w, h, QualityTier::High);
    let job = clip_job(clip, s, dir.join("unused.mp4"), 0);
    let mut renderer = renderer_for(clip);
    let plan = plan_export(&job).unwrap();
    let frames = run_export_into(
        &mut renderer,
        &job,
        plan,
        MemorySink::default(),
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap()
    .frames;
    let reference = dir.join("reference.y4m");
    write_y4m(&reference, &frames);
    for mbps in [4u32, 16, 40] {
        // VideoToolbox, from the same encoder input.
        let vt = dir.join(format!("vt-{mbps}.mp4"));
        let spec = ExportSpec {
            width: w,
            height: h,
            rate: Rate::FPS_23_976,
            frames: FRAMES,
            codec: Codec::H264 {
                bitrate_bps: mbps * 1_000_000,
            },
            container: Container::Mp4,
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::On,
            audio: None,
            timecode: None,
            destination: vt.clone(),
            overwrite: Overwrite::Fail,
            mode: None,
        };
        let mut export = pm_export::export(spec).unwrap();
        for f in &frames {
            export.push_encoder_input(f).unwrap();
        }
        export.finish().unwrap();
        let mut scores = vec![(
            "VideoToolbox",
            vmaf(ff, &vt, &reference, &dir.join("vt.json")),
            std::fs::metadata(&vt).unwrap().len(),
        )];
        for preset in ["medium", "slow"] {
            let x = dir.join(format!("x264-{mbps}-{preset}.mp4"));
            let out = Command::new(ff)
                .args(["-v", "error", "-y", "-i"])
                .arg(&reference)
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    preset,
                    "-b:v",
                    &format!("{mbps}M"),
                    "-maxrate",
                    &format!("{}M", mbps * 2),
                    "-bufsize",
                    &format!("{}M", mbps * 2),
                ])
                .args(["-g", "48", "-vf", TAGS, "-pix_fmt", "yuv420p"])
                .arg(&x)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let score = vmaf(ff, &x, &reference, &dir.join("x.json"));
            scores.push((
                if preset == "slow" {
                    "x264 slow"
                } else {
                    "x264 medium"
                },
                score,
                std::fs::metadata(&x).unwrap().len(),
            ));
            std::fs::remove_file(x).unwrap();
        }
        std::fs::remove_file(vt).unwrap();
        for (name, score, bytes) in scores {
            eprintln!(
                "MEASURE VMAF H.264 {mbps} Mbps 1080p, {FRAMES} frames of {source}: {name} {score:.2} ({:.2} Mbps actual)",
                bytes as f64 * 8.0 / (FRAMES as f64 * 1001.0 / 24000.0) / 1e6
            );
        }
    }
}

#[test]
#[ignore = "measurement: export frames per second, every format"]
fn export_throughput() {
    let Some(clip) = corpus("uhd-prores422hq.mov") else {
        eprintln!("no UHD corpus clip: not measured");
        return;
    };
    let dir = scratch("throughput");
    let hevc = |ten_bit| ExportFormat::Hevc {
        ten_bit,
        container: WireContainer::Mp4,
    };
    let mut cases: Vec<(&str, ExportFormat, WireAlpha, u32, u32, usize)> = vec![(
        "UHD ProRes 422 HQ, 3 layers",
        ExportFormat::ProRes {
            profile: WireProRes::Hq,
        },
        WireAlpha::None,
        3840,
        2160,
        2,
    )];
    for (name, format, alpha) in [
        (
            "ProRes 422 Proxy",
            ExportFormat::ProRes {
                profile: WireProRes::Proxy,
            },
            WireAlpha::None,
        ),
        (
            "ProRes 422 HQ",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            WireAlpha::None,
        ),
        (
            "ProRes 4444",
            ExportFormat::ProRes {
                profile: WireProRes::P4444,
            },
            WireAlpha::Straight,
        ),
        (
            "ProRes 4444 XQ",
            ExportFormat::ProRes {
                profile: WireProRes::P4444Xq,
            },
            WireAlpha::None,
        ),
        ("H.264", ExportFormat::H264, WireAlpha::None),
        ("HEVC", hevc(false), WireAlpha::None),
        ("HEVC 10-bit", hevc(true), WireAlpha::None),
        (
            "HEVC with alpha",
            ExportFormat::Hevc {
                ten_bit: false,
                container: WireContainer::Mov,
            },
            WireAlpha::Straight,
        ),
        (
            "WebM VP9",
            ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            WireAlpha::None,
        ),
        (
            "WebM VP8",
            ExportFormat::Webm {
                codec: WebmCodec::Vp8,
            },
            WireAlpha::None,
        ),
        (
            "PNG sequence 8-bit",
            ExportFormat::PngSequence { sixteen_bit: false },
            WireAlpha::None,
        ),
        (
            "PNG sequence 16-bit",
            ExportFormat::PngSequence { sixteen_bit: true },
            WireAlpha::Straight,
        ),
    ] {
        cases.push((name, format, alpha, 1920, 1080, 2));
    }
    for (i, (name, format, alpha, w, h, extra)) in cases.into_iter().enumerate() {
        let ext = match format {
            ExportFormat::ProRes { .. }
            | ExportFormat::Hevc {
                container: WireContainer::Mov,
                ..
            } => ".mov",
            ExportFormat::Webm { .. } => ".webm",
            ExportFormat::PngSequence { .. } => "",
            _ => ".mp4",
        };
        let out = dir.join(format!("out-{i}{ext}"));
        let job = clip_job(
            &clip,
            settings(format, alpha, w, h, QualityTier::High),
            out,
            extra,
        );
        let mut renderer = renderer_for(&clip);
        // Warm the GPU pipelines and decoder on one frame first.
        let mut warm = job.clone();
        warm.settings.range.end = Ticks::new(1001 * TICKS_PER_SECOND / 24000).unwrap();
        warm.destination = dir.join(format!("warm-{i}{ext}"));
        run_export(
            &mut renderer,
            &warm,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let t = Instant::now();
        let encoded = std::sync::Mutex::new(0.0);
        let delivered = run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |p| {
                if p.frames_done == p.frames {
                    *encoded.lock().unwrap() = t.elapsed().as_secs_f64();
                }
            },
        )
        .unwrap();
        let secs = t.elapsed().as_secs_f64();
        let encoded = *encoded.lock().unwrap();
        eprintln!(
            "MEASURE throughput {name} {w}x{h}: {:.1} fps end to end ({FRAMES} frames, {secs:.2} s: frames through the encoder at {:.1} fps, then {:.2} s to finish, verify, and commit; real time is 23.976 fps), {:.1} MB",
            FRAMES as f64 / secs,
            FRAMES as f64 / encoded,
            secs - encoded,
            delivered.report.bytes as f64 / 1e6
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
