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
use pm_render::{RenderRequest, Renderer};
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
        delivery: Default::default(),
    }
}

fn job(
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
        let job = job(
            &doc,
            media.clone(),
            settings(format, alpha, dur),
            destination.clone(),
        );
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
            let mode = std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode();
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

    fn abandoned(&self, _: u64) {
        panic!("a supported frame never abandons a submission");
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
fn offline_export_matches_the_same_project_with_missing_layers_hidden() {
    let dur = 0.2;
    let (mut doc, mut media) = generated(dur);
    // A live solid uses the offline clip as both parent and track matte. The
    // export must retain those relationships in the original snapshot.
    doc["layers"][0]["parent"] = json!("vid");
    doc["layers"][0]["matteSource"] = json!("vid");
    media.paths.remove("clip");
    let mut omitted = doc.clone();
    for layer in omitted["layers"].as_array_mut().unwrap() {
        if layer["d"]["asset"] == "clip" {
            layer["type"] = json!("null");
        }
    }
    let mut s = settings(
        ExportFormat::ProRes {
            profile: WireProRes::Hq,
        },
        WireAlpha::None,
        dur,
    );
    s.audio = false;
    let render = |document: &Value| {
        let job = job(document, media.clone(), s.clone(), PathBuf::from("/unused"));
        let mut renderer = Renderer::headless().unwrap();
        let plan = plan_export(&job).unwrap();
        run_export_into(
            &mut renderer,
            &job,
            plan,
            MemorySink::default(),
            &Unrestricted,
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap()
        .frames
    };
    assert_eq!(render(&doc), render(&omitted));
    let export_job = job(&doc, media, s, PathBuf::from("/unused"));
    let plan = plan_export(&export_job).unwrap();
    assert!(Arc::ptr_eq(&plan.document, &export_job.document));
    assert_eq!(
        offline_media(
            &export_job.document,
            &export_job.composition,
            &export_job.media
        )
        .iter()
        .map(|c| c.asset.as_str())
        .collect::<Vec<_>>(),
        vec!["clip"]
    );

    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Clean(scratch("offline-byte-identity"));
    let mut png = settings(
        ExportFormat::PngStill { sixteen_bit: false },
        WireAlpha::None,
        1.0 / 30.0,
    );
    png.audio = false;
    png.color = OutputColor::SrgbImage;
    let write = |document: &Value, name: &str| {
        let output = dir.0.join(name);
        let job = job(
            document,
            export_job.media.as_ref().clone(),
            png.clone(),
            output.clone(),
        );
        run_export(
            &mut Renderer::headless().unwrap(),
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap();
        std::fs::read(output).unwrap()
    };
    assert_eq!(write(&doc, "missing.png"), write(&omitted, "omitted.png"));
}

#[test]
fn png_byte_compatibility_follows_the_frozen_document() {
    let mut renderer = Renderer::headless().unwrap();
    for compat in [1, 2] {
        let mut solid = layer(
            "low-alpha",
            "solid",
            json!({"color":"#336699", "w":64, "h":32}),
            W as f64 / 2.0,
            H as f64 / 2.0,
            1.0,
        );
        solid["compat"] = json!(compat);
        set(&mut solid, "opacity", json!(0.3));
        let doc = project(W, H, 1.0, no_fill(), vec![solid]);
        let mut s = settings(
            ExportFormat::PngStill { sixteen_bit: false },
            WireAlpha::Straight,
            1.0 / 30.0,
        );
        s.audio = false;
        s.color = OutputColor::SrgbImage;
        let job = job(&doc, MediaPaths::default(), s, PathBuf::from("/unused.png"));
        let plan = plan_export(&job).unwrap();
        let output = run_export_into(
            &mut renderer,
            &job,
            plan,
            MemorySink::default(),
            &Unrestricted,
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap();
        let rgba = output.frames[0].planes[0].u8().unwrap();
        let center = ((H / 2 * W + W / 2) * 4) as usize;
        assert_eq!(
            &rgba[center..center + 4],
            if compat == 1 {
                &[0, 0, 0, 1]
            } else {
                &[51, 102, 153, 1]
            }
        );
    }
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
    let request = plan.request.clone();
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
    assert!(
        out.audio.iter().any(|v| v.abs() > 0.01),
        "the tone and the clip sound"
    );
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
                legacy_png: true,
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
fn unsupported_projects_route_and_offline_media_plans_before_writing() {
    let dir = scratch("routing");
    let dur = 1.0;
    let (mut doc, media) = generated(dur);
    doc["layers"]
        .as_array_mut()
        .unwrap()
        .insert(0, text("caption", dur));
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
    // Offline media is not routed (decision OFFLINE-1): an unattended export
    // draws the missing clip as nothing, like today's exporter, and reports
    // it. `offline_export_matches_the_same_project_with_missing_layers_hidden`
    // checks the rendered frames; planning here still writes nothing.
    let (doc, mut media) = generated(dur);
    media.paths.remove("clip");
    let j = job(
        &doc,
        media,
        settings(ExportFormat::H264, WireAlpha::None, dur),
        destination.clone(),
    );
    let plan = plan_export(&j).expect("an offline clip does not route the export");
    assert_eq!(
        plan.offline
            .iter()
            .map(|clip| clip.asset.as_str())
            .collect::<Vec<_>>(),
        vec!["clip"]
    );
    let (doc, media) = generated(dur);
    let mut s = settings(ExportFormat::H264, WireAlpha::None, dur);
    s.format = ExportFormat::Webm {
        codec: pm_protocol::priority::WebmCodec::Vp9,
    };
    // WebM is a job engine format since Phase 3b; planning writes nothing.
    assert!(plan_export(&job(&doc, media, s, destination.clone())).is_ok());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "nothing written"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Counts what `render_frame` told the gate.
#[derive(Default)]
struct Ledger {
    admitted: AtomicU32,
    finished: AtomicU32,
    abandoned: AtomicU32,
}

impl GpuGate for Ledger {
    fn admit(&self, _: u64, units: u32, _: &AtomicBool) -> Option<u32> {
        self.admitted.fetch_add(1, Ordering::Relaxed);
        Some(units)
    }

    fn finished(&self, _: u64, _: u32, _: f64) {
        self.finished.fetch_add(1, Ordering::Relaxed);
    }

    fn abandoned(&self, _: u64) {
        self.abandoned.fetch_add(1, Ordering::Relaxed);
    }
}

/// Every admission ends in `finished` or `abandoned`, including a render
/// that fails after admission (a text layer when nothing routed the frame
/// first); an offline clip is a layer failure of a finished render; stills
/// route before admitting anything.
#[test]
fn every_admission_is_released_when_a_render_fails() {
    let dur = 0.2;
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::sized(Size::new(W, H), Precision::Float16);
    let (mut text_doc, media) = generated(dur);
    text_doc["layers"]
        .as_array_mut()
        .unwrap()
        .insert(0, text("caption", dur));
    let ledger = Ledger::default();
    let result = render_frame(
        &mut renderer,
        &load(&text_doc),
        "Ptest-root",
        Ticks::ZERO,
        &request,
        &media,
        &ledger,
        1,
        &AtomicBool::new(false),
    );
    assert!(
        matches!(result, Err(JobError::Unsupported(_))),
        "{:?}",
        result.err()
    );
    assert_eq!(ledger.admitted.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.abandoned.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.finished.load(Ordering::Relaxed), 0);
    let (offline_doc, mut offline) = generated(dur);
    offline.paths.remove("clip");
    let ledger = Ledger::default();
    let frame = render_frame(
        &mut renderer,
        &load(&offline_doc),
        "Ptest-root",
        Ticks::ZERO,
        &request,
        &offline,
        &ledger,
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        !frame.failures.is_empty(),
        "the offline clip failed its layer"
    );
    assert_eq!(ledger.admitted.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.finished.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.abandoned.load(Ordering::Relaxed), 0);
    // The still path routes the text layer before the gate is asked.
    let ledger = Ledger::default();
    let mut routing = StillRouting::new();
    let still = render_still(
        &mut renderer,
        &load(&text_doc),
        "Ptest-root",
        Ticks::ZERO,
        1280,
        &media,
        &mut routing,
        &ledger,
        1,
        &AtomicBool::new(false),
    );
    match still {
        Err(JobError::Unsupported(list)) => {
            assert!(list.iter().any(|u| u.contains("text")), "{list:?}")
        }
        other => panic!("expected routing, got {:?}", other.err()),
    }
    assert_eq!(ledger.admitted.load(Ordering::Relaxed), 0);
    // A supported frame after the failures: admitted and finished.
    let (doc, media) = generated(dur);
    let ledger = Ledger::default();
    render_frame(
        &mut renderer,
        &load(&doc),
        "Ptest-root",
        Ticks::ZERO,
        &request,
        &media,
        &ledger,
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(ledger.admitted.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.finished.load(Ordering::Relaxed), 1);
    assert_eq!(ledger.abandoned.load(Ordering::Relaxed), 0);
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
    assert!(
        matches!(result, Err(JobError::Cancelled)),
        "{:?}",
        result.err()
    );
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
    let mut j = job(
        &doc,
        media,
        settings(ExportFormat::H264, WireAlpha::None, dur),
        host.clone(),
    );
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

/// Prints submission-to-completion times (diagnostic).
#[test]
#[ignore = "diagnostic"]
fn gpu_times_by_band() {
    struct Print(Mutex<Vec<(u32, f64)>>);
    impl GpuGate for Print {
        fn admit(&self, _: u64, units: u32, _: &AtomicBool) -> Option<u32> {
            Some(if std::env::var_os("BANDS").is_some() {
                1
            } else {
                units
            })
        }
        fn finished(&self, _: u64, units: u32, us: f64) {
            self.0.lock().unwrap().push((units, us));
        }
        fn abandoned(&self, _: u64) {}
    }
    let (doc, media) = generated(1.0);
    let job = job(
        &doc,
        media,
        settings(ExportFormat::H264, WireAlpha::None, 1.0),
        PathBuf::from("/x.mp4"),
    );
    let mut r = Renderer::headless().unwrap();
    let p = Print(Mutex::new(vec![]));
    let req = pm_render::RenderRequest::sized(Size::new(W, H), Precision::Float16);
    for n in 0..10 {
        render_frame(
            &mut r,
            &job.document,
            &job.composition,
            seconds(n as f64 / 10.0),
            &req,
            &job.media,
            &p,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
    }
    eprintln!("{:?}", p.0.lock().unwrap());
}

/// An alpha export keeps its alpha channel when every frame is opaque.
#[test]
fn opaque_frames_keep_the_alpha_channel_of_an_alpha_export() {
    let dir = scratch("opaque-alpha");
    let doc = project(
        W,
        H,
        0.5,
        no_fill(),
        vec![layer(
            "bg",
            "solid",
            json!({ "color": "#3FCF8E", "w": W, "h": H }),
            0.0,
            0.0,
            0.5,
        )],
    );
    for (alpha, name) in [
        (WireAlpha::Straight, "straight"),
        (WireAlpha::Premultiplied, "premultiplied"),
    ] {
        let mut s = settings(
            ExportFormat::ProRes {
                profile: WireProRes::P4444,
            },
            alpha,
            0.5,
        );
        s.audio = false;
        let j = job(
            &doc,
            MediaPaths::default(),
            s,
            dir.join(format!("{name}.mov")),
        );
        let mut renderer = Renderer::headless().unwrap();
        let delivered = run_export(
            &mut renderer,
            &j,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(delivered.report.alpha, "{name}");
        assert_eq!(delivered.report.pixel_format, "4:4:4:4");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// An alpha export whose first frame is opaque keeps its alpha channel, and
/// the later transparent frames verify (alpha is in the sampled blocks).
#[test]
fn alpha_survives_an_opaque_first_frame() {
    let dir = scratch("late-alpha");
    let mut bg = layer(
        "bg",
        "solid",
        json!({ "color": "#3FCF8E", "w": W, "h": H }),
        0.0,
        0.0,
        0.25,
    );
    bg["dur"] = json!(0.25);
    let doc = project(
        W,
        H,
        0.5,
        no_fill(),
        vec![
            layer(
                "small",
                "solid",
                json!({ "color": "#E83A5E", "w": 40, "h": 40 }),
                10.0,
                10.0,
                0.5,
            ),
            bg,
        ],
    );
    let mut s = settings(
        ExportFormat::ProRes {
            profile: WireProRes::P4444,
        },
        WireAlpha::Straight,
        0.5,
    );
    s.audio = false;
    let j = job(&doc, MediaPaths::default(), s, dir.join("late.mov"));
    let mut renderer = Renderer::headless().unwrap();
    let delivered = run_export(
        &mut renderer,
        &j,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut no_progress(),
    )
    .unwrap();
    assert!(delivered.report.alpha);
    assert_eq!(delivered.report.pixel_format, "4:4:4:4");
    if let Some(ffprobe) = ffprobe() {
        let out = std::process::Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=pix_fmt",
                "-of",
                "csv=p=0",
            ])
            .arg(&delivered.path)
            .output()
            .unwrap();
        // ffprobe may list the stream again under its program; the first
        // entry is the video stream's own.
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().map(str::trim).find(|l| !l.is_empty());
        assert_eq!(first, Some("yuva444p12le"), "{text}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Gates that measure get each submission's GPU time from timestamps on
/// devices that have them; the span grows with the work and stays within
/// the wall time an idle GPU takes for it.
#[test]
fn measuring_gates_get_gpu_time_from_timestamps() {
    struct Spans(Mutex<Vec<(f64, bool)>>);
    impl GpuGate for Spans {
        fn admit(&self, _: u64, units: u32, _: &AtomicBool) -> Option<u32> {
            Some(units)
        }
        fn finished(&self, _: u64, _: u32, _: f64) {
            panic!("the renderer reports through finished_timed");
        }
        fn finished_timed(&self, _: u64, _: u32, us: f64, gpu: bool) {
            self.0.lock().unwrap().push((us, gpu));
        }
        fn abandoned(&self, _: u64) {}
    }
    let mut renderer = Renderer::headless().unwrap();
    if !renderer
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        eprintln!("no timestamp queries on this device: wall time only");
        return;
    }
    let mut median = |layers: usize, width: u32| {
        let doc = load(&heavy_precomp(layers));
        let gate = Spans(Mutex::new(vec![]));
        let mut routing = StillRouting::new();
        for _ in 0..5 {
            render_still(
                &mut renderer,
                &doc,
                "Ptest-root",
                Ticks::ZERO,
                width,
                &MediaPaths::default(),
                &mut routing,
                &gate,
                1,
                &AtomicBool::new(false),
            )
            .unwrap();
        }
        let samples = gate.0.into_inner().unwrap();
        assert!(
            samples.iter().all(|&(us, gpu)| gpu && us > 0.0),
            "{samples:?}"
        );
        let mut us: Vec<f64> = samples[1..].iter().map(|s| s.0).collect();
        us.sort_by(f64::total_cmp);
        us[us.len() / 2]
    };
    let light = median(4, 320);
    let heavy = median(200, 3840);
    eprintln!("GPU span: light {light:.0} µs, heavy {heavy:.0} µs");
    assert!(
        heavy > 4.0 * light,
        "light {light:.0} µs, heavy {heavy:.0} µs"
    );
    // Unrestricted (the CLI, exports without playback) does not time.
    assert!(!Unrestricted.measures());
}

/// The harness's over-bound pass (`frame-bridge-measure.mjs`
/// `playback-engine-agent-snapshots-heavy`): `layers` solids of the 4K
/// measurement project inside a full-size 4K precomposition.
fn heavy_precomp(layers: usize) -> Value {
    let dur = 4.0;
    let solids: Vec<Value> = (0..layers)
        .map(|i| {
            let mut l = layer(
                &format!("s{i}"),
                "solid",
                json!({ "color": format!("#{:06x}", (i * 0x2a7f13) & 0xffffff), "w": 640 + (i % 5) * 240, "h": 360 + (i % 3) * 200 }),
                ((i * 157) % 3200) as f64,
                ((i * 89) % 1800) as f64,
                dur,
            );
            set(&mut l, "opacity", json!(85));
            l
        })
        .collect();
    let pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "heavy", "w": 3840, "h": 2160 }),
        1920.0,
        1080.0,
        dur,
    );
    let mut doc = project(3840, 2160, dur, solid_fill("#101010"), vec![pre]);
    composition(&mut doc, "heavy", 3840, 2160, dur, solids);
    doc
}

/// Uncontended GPU time of the heavy pass: whole agent stills and single
/// 16-row bands (each renders the whole precomposition), as GPU timestamp
/// spans and as wall time. Run with `--ignored --nocapture` on an otherwise
/// idle GPU.
#[test]
#[ignore = "timing measurement"]
fn measure_heavy_precomp_band_cost() {
    struct Timed {
        samples: Mutex<Vec<f64>>,
        bands: bool,
        gpu: bool,
    }
    impl GpuGate for Timed {
        fn admit(&self, _: u64, units: u32, _: &AtomicBool) -> Option<u32> {
            Some(if self.bands { 1 } else { units })
        }
        fn finished(&self, _: u64, _: u32, us: f64) {
            self.samples.lock().unwrap().push(us);
        }
        fn abandoned(&self, _: u64) {}
        fn measures(&self) -> bool {
            self.gpu
        }
    }
    let mut renderer = Renderer::headless().unwrap();
    for (layers, width) in [
        (96usize, 720u32),
        (96, 1920),
        (96, 3840),
        (400, 3840),
        (1500, 3840),
    ] {
        let doc = load(&heavy_precomp(layers));
        for bands in [false, true] {
            let mut line = format!(
                "{layers} layers, {width} px, {}:",
                if bands {
                    "one band each"
                } else {
                    "whole frames"
                }
            );
            for gpu in [true, false] {
                let gate = Timed {
                    samples: Mutex::new(vec![]),
                    bands,
                    gpu,
                };
                let mut routing = StillRouting::new();
                for _ in 0..4 {
                    render_still(
                        &mut renderer,
                        &doc,
                        "Ptest-root",
                        Ticks::ZERO,
                        width,
                        &MediaPaths::default(),
                        &mut routing,
                        &gate,
                        1,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                }
                let samples = gate.samples.into_inner().unwrap();
                // The first frame's submissions are cold.
                let mut us: Vec<f64> = samples[samples.len() / 4..].to_vec();
                us.sort_by(f64::total_cmp);
                let p = |q: f64| us[((us.len() - 1) as f64 * q).round() as usize];
                line += &format!(
                    "  {} p50 {:.0} p90 {:.0} max {:.0} µs (n {})",
                    if gpu { "GPU span" } else { "wall" },
                    p(0.5),
                    p(0.9),
                    p(1.0),
                    us.len()
                );
            }
            println!("{line}");
        }
    }
}
