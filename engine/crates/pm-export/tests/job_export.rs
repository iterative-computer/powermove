//! The job engine's export pipeline end to end on generated projects (plan
//! section 17): capability routing, pm-render frames, pm-audio's offline mix,
//! VideoToolbox encode, AVAssetWriter mux, verification, and the atomic move;
//! sliced rendering equals whole frames; cancellation leaves nothing behind.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ProResProfile as WireProRes,
    QualityTier,
};
use pm_render::Renderer;
use serde_json::{Value, json};
use support::doc::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("apps/desktop/e2e/fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-job-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seconds(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

const W: u32 = 320;
const H: u32 = 180;

/// Solids with animated transforms, a PNG with alpha, an H.264 video with its
/// soundtrack, a precomp, a group, and a WAV tone from 0.5 s.
fn generated(dur: f64) -> (Value, MediaPaths) {
    let mut layers = vec![];
    let mut mover = layer(
        "mover",
        "solid",
        json!({ "color": "#E83A5E", "w": 90, "h": 60 }),
        40.0,
        60.0,
        dur,
    );
    set(
        &mut mover,
        "position.x",
        keys(json!(40), &[(0.0, json!(40)), (dur, json!(260))]),
    );
    set(
        &mut mover,
        "rotation",
        keys(json!(0), &[(0.0, json!(0)), (dur, json!(45))]),
    );
    layers.push(mover);
    layers.push(audio("tone", "tone", 0.5, dur));
    let mut image = layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "contain", "w": 80, "h": 45 }),
        250.0,
        40.0,
        dur,
    );
    set(&mut image, "rotation", json!(12));
    layers.push(image);
    let mut video = layer(
        "vid",
        "video",
        json!({ "asset": "clip", "fit": "cover", "w": 120, "h": 68, "trim": 0, "speed": 1, "embeddedAudio": true }),
        80.0,
        110.0,
        dur,
    );
    set(&mut video, "opacity", json!(85));
    layers.push(video);
    let mut pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 160, "h": 90 }),
        200.0,
        120.0,
        dur,
    );
    set(&mut pre, "scale.x", json!(50));
    set(&mut pre, "scale.y", json!(50));
    layers.push(pre);
    layers.push(layer(
        "back",
        "solid",
        json!({ "color": "#202436", "w": 320, "h": 180 }),
        160.0,
        90.0,
        dur,
    ));
    let mut doc = project(W, H, dur, solid_fill("#14161C"), layers);
    composition(
        &mut doc,
        "inner",
        160,
        90,
        dur,
        vec![layer(
            "in-solid",
            "solid",
            json!({ "color": "#3FCF8E", "w": 160, "h": 90 }),
            80.0,
            45.0,
            dur,
        )],
    );
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    asset(&mut doc, "clip", "video", "mp4", 320, 180, 2.0);
    asset(&mut doc, "tone", "audio", "wav", 0, 0, 2.0);
    let mut media = MediaPaths::default();
    media.insert("still", fixture("still-alpha.png"));
    media.insert("clip", fixture("h264-aac.mp4"));
    media.insert("tone", fixture("tone.wav"));
    (doc, media)
}

fn settings(format: ExportFormat, alpha: WireAlpha, dur: f64) -> ExportSettings {
    ExportSettings {
        format,
        alpha,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: seconds(dur),
        },
        audio: true,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float32,
        dither: matches!(format, ExportFormat::H264),
        draft_from_proxies: false,
    }
}

fn job(doc: &Value, media: MediaPaths, settings: ExportSettings, destination: PathBuf) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 7,
    }
}

fn no_progress() -> impl FnMut(Progress) + Send {
    |_| {}
}

fn ffprobe() -> Option<PathBuf> {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        });
    let p = dir.join("bin/ffprobe");
    p.exists().then_some(p)
}

#[test]
fn generated_projects_export_and_verify_in_every_phase_2_format() {
    let dur = 2.0;
    let (doc, media) = generated(dur);
    let dir = scratch("formats");
    let mut renderer = Renderer::headless().unwrap();
    let cases = [
        (
            "hq",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            WireAlpha::None,
            "mov",
            "apch",
            "lpcm",
        ),
        (
            "4444",
            ExportFormat::ProRes {
                profile: WireProRes::P4444,
            },
            WireAlpha::Straight,
            "mov",
            "ap4h",
            "lpcm",
        ),
        (
            "h264",
            ExportFormat::H264,
            WireAlpha::None,
            "mp4",
            "avc1",
            "mp4a",
        ),
    ];
    for (name, format, alpha, ext, codec, audio_codec) in cases {
        let destination = dir.join(format!("{name}.{ext}"));
        let job = job(&doc, media.clone(), settings(format, alpha, dur), destination.clone());
        let seen = Mutex::new(vec![]);
        let mut progress = |p: Progress| seen.lock().unwrap().push(p.frames_done);
        let delivered = run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut progress,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(delivered.path, destination);
        let r = &delivered.report;
        assert_eq!(r.codec, codec, "{name}");
        assert_eq!(r.frames, 60, "{name}: 2 s at 30 fps");
        assert_eq!(r.cicp, (1, 13, 1), "{name}");
        assert!(!r.full_range, "{name}");
        assert_eq!(r.alpha, alpha != WireAlpha::None, "{name}");
        let audio = r.audio.as_ref().expect("audio track");
        assert_eq!(audio.codec, audio_codec, "{name}");
        // round(duration × sr) samples (section 2, audio bar).
        assert_eq!(audio.samples, 96_000, "{name}");
        assert!(r.max_pixel_error_cv() <= verify::pixel_tolerance(job_codec(format)));
        assert_eq!(*seen.lock().unwrap(), (1..=60).collect::<Vec<u64>>());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&destination).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
        if let Some(ffprobe) = ffprobe() {
            let out = std::process::Command::new(ffprobe)
                .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
                .args(["-show_entries", "stream=nb_read_frames,color_transfer"])
                .args(["-of", "csv=p=0"])
                .arg(&destination)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("60") && text.contains("iec61966-2-1"),
                "{name}: ffprobe {text}"
            );
        }
    }
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

fn job_codec(format: ExportFormat) -> Codec {
    match format {
        ExportFormat::ProRes { .. } => Codec::ProRes(ProResProfile::Hq),
        _ => Codec::H264 {
            bitrate_bps: 16_000_000,
        },
    }
}

/// Admits a varying number of units, as the slicing governor does during
/// playback, and counts submissions.
struct Bands {
    next: AtomicU32,
    submissions: AtomicU32,
}

impl GpuGate for Bands {
    fn admit(&self, _: u64, units: u32, cancel: &AtomicBool) -> Option<u32> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        self.submissions.fetch_add(1, Ordering::Relaxed);
        Some((self.next.fetch_add(1, Ordering::Relaxed) % 3 + 1).min(units))
    }

    fn finished(&self, _: u64, _: u32, measured_us: f64) {
        assert!(measured_us >= 0.0);
    }
}

#[test]
fn sliced_frames_equal_whole_frames_bit_for_bit() {
    let dur = 0.5;
    let (doc, media) = generated(dur);
    let mut renderer = Renderer::headless().unwrap();
    let mut encode = |gate: &dyn GpuGate| {
        let mut s = settings(
            ExportFormat::ProRes {
                profile: WireProRes::P4444Xq,
            },
            WireAlpha::Straight,
            dur,
        );
        s.audio = false;
        let job = job(&doc, media.clone(), s, PathBuf::from("/unused.mov"));
        let plan = plan_export(&job).unwrap();
        run_export_into(
            &mut renderer,
            &job,
            plan,
            MemorySink::default(),
            gate,
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap()
    };
    let whole = encode(&Unrestricted);
    let bands = Bands {
        next: AtomicU32::new(0),
        submissions: AtomicU32::new(0),
    };
    let sliced = encode(&bands);
    assert_eq!(whole.frames.len(), 15);
    assert!(
        bands.submissions.load(Ordering::Relaxed) > 15 * 4,
        "every frame was banded"
    );
    assert!(whole.frames == sliced.frames, "banded frames differ");
}

#[test]
fn the_pipeline_matches_rendering_and_converting_each_frame() {
    let dur = 0.2;
    let (doc, media) = generated(dur);
    let mut renderer = Renderer::headless().unwrap();
    let mut s = settings(ExportFormat::H264, WireAlpha::None, dur);
    s.audio = true;
    let job = job(&doc, media.clone(), s, PathBuf::from("/unused.mp4"));
    let plan = plan_export(&job).unwrap();
    let timeline = plan.timeline;
    let request = plan.request;
    let out = run_export_into(
        &mut renderer,
        &job,
        plan,
        MemorySink::default(),
        &Unrestricted,
        &AtomicBool::new(false),
        &mut no_progress(),
    )
    .unwrap();
    assert_eq!(out.frames.len() as u64, timeline.frames);
    // 6 frames at 30 fps: 0.2 s of stereo at 48 kHz.
    assert_eq!(out.audio.len(), 9600 * 2);
    assert!(out.audio.iter().any(|v| v.abs() > 0.01), "the tone and the clip sound");
    for n in [0u64, timeline.frames - 1] {
        let frame = render_frame(
            &mut renderer,
            &job.document,
            &job.composition,
            timeline.tick(n),
            &request,
            &job.media,
            &Unrestricted,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        let input = convert::convert(
            &WorkingFrame {
                width: W,
                height: H,
                rgba: &frame.rgba,
            },
            InputFormat::V420,
            &convert::ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                dither: true,
                frame: n as u32,
            },
        )
        .unwrap();
        assert!(out.frames[n as usize] == input, "frame {n}");
    }
}

#[test]
fn unsupported_projects_and_offline_media_are_routed_before_writing() {
    let dir = scratch("routing");
    let dur = 1.0;
    let (mut doc, media) = generated(dur);
    doc["layers"].as_array_mut().unwrap().insert(0, text("caption", dur));
    let destination = dir.join("routed.mov");
    let j = job(
        &doc,
        media.clone(),
        settings(
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            WireAlpha::None,
            dur,
        ),
        destination.clone(),
    );
    match plan_export(&j) {
        Err(JobError::Unsupported(list)) => {
            assert!(list.iter().any(|u| u.contains("text")), "{list:?}")
        }
        other => panic!("expected routing, got {:?}", other.err()),
    }
    let (doc, mut media) = generated(dur);
    media.paths.remove("clip");
    let j = job(
        &doc,
        media,
        settings(ExportFormat::H264, WireAlpha::None, dur),
        destination.clone(),
    );
    let mut renderer = Renderer::headless().unwrap();
    match run_export(
        &mut renderer,
        &j,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut no_progress(),
    ) {
        Err(JobError::Unsupported(list)) => {
            assert!(list.iter().any(|u| u.contains("offline") || u.contains("missing")), "{list:?}")
        }
        Err(e) => panic!("expected routing, got {e}"),
        Ok(_) => panic!("expected routing, got a file"),
    }
    let (doc, media) = generated(dur);
    let mut s = settings(ExportFormat::H264, WireAlpha::None, dur);
    s.format = ExportFormat::Webm {
        codec: pm_protocol::priority::WebmCodec::Vp9,
    };
    assert!(matches!(
        plan_export(&job(&doc, media, s, destination.clone())),
        Err(JobError::Unsupported(_))
    ));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing written");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_cancelled_export_leaves_nothing_behind() {
    let dir = scratch("cancel");
    let dur = 2.0;
    let (doc, media) = generated(dur);
    let j = job(
        &doc,
        media,
        settings(
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            WireAlpha::None,
            dur,
        ),
        dir.join("cancelled.mov"),
    );
    let cancel = AtomicBool::new(false);
    let mut renderer = Renderer::headless().unwrap();
    let mut progress = |p: Progress| {
        if p.frames_done == 3 {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let result = run_export(&mut renderer, &j, &Unrestricted, &cancel, &mut progress);
    assert!(matches!(result, Err(JobError::Cancelled)), "{:?}", result.err());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "no destination, temporary, or work file"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exports_replace_the_hosts_temporary_file_in_place() {
    let dir = scratch("inplace");
    let host = dir.join(".powermove-00112233aabbccdd.part");
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&host)
            .unwrap();
    }
    let dur = 0.5;
    let (doc, media) = generated(dur);
    let mut j = job(&doc, media, settings(ExportFormat::H264, WireAlpha::None, dur), host.clone());
    j.overwrite = Overwrite::InPlace;
    let mut renderer = Renderer::headless().unwrap();
    let delivered = run_export(
        &mut renderer,
        &j,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut no_progress(),
    )
    .unwrap();
    assert_eq!(delivered.path, host);
    let bytes = std::fs::metadata(&host).unwrap().len();
    assert!(bytes > 1000);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names.len(), 1, "only the host's file: {names:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
