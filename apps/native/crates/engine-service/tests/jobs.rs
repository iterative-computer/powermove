use pm_engine_service::{MediaCatalog, jobs::*};
use pm_protocol::{
    color::{OutputColor, Precision},
    common::{FrameRate, SessionId, Size, TickRange, Ticks},
    priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, JobState, QualityTier},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn request(destination: std::path::PathBuf, seconds: i32) -> ExportRequest {
    request_for(include_str!("fixtures/shapes.json"), destination, seconds)
}
/// The shapes fixture plus a video layer whose original is offline.
fn offline_request(destination: std::path::PathBuf, preview: bool) -> ExportRequest {
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    project["assets"]["gone"] = serde_json::json!({
        "id": "gone", "name": "interview take 3.mov", "kind": "video", "format": "mp4",
        "w": 320, "h": 180, "dur": 3, "fps": 30, "persisted": true
    });
    let mut layer = project["layers"][0].clone();
    layer["id"] = "Lgone".into();
    layer["type"] = "video".into();
    layer["d"] = serde_json::json!({"asset": "gone", "w": 320, "h": 180});
    project["layers"].as_array_mut().unwrap().push(layer);
    let mut r = request_for(&project.to_string(), destination, 1);
    r.preview = preview;
    r
}
fn request_for(source: &str, destination: std::path::PathBuf, seconds: i32) -> ExportRequest {
    let project = pm_model::json::parse(source).unwrap();
    #[cfg(feature = "kernel")]
    let s = pm_kernel::document::DocumentStore::new_untitled(project).snapshot();
    #[cfg(not(feature = "kernel"))]
    let s = pm_engine_service::Snapshot {
        doc_id: 1,
        revision: 1,
        document: std::sync::Arc::new(pm_model::Document::new(
            Some(pm_model::canonical::canonical_project(project)),
            1,
        )),
    };
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: false },
        alpha: AlphaOutput::Straight,
        quality: QualityTier::High,
        size: Size::new(320, 180),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::from_whole_seconds(seconds),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    ExportRequest::new(
        "fixture-export".into(),
        &s,
        JobSpec::Export {
            session: SessionId::new(1).unwrap(),
            composition: s.open_comp().unwrap().as_str().into(),
            settings,
        },
        MediaCatalog::default(),
        destination,
    )
    .unwrap()
}
#[test]
fn child_export_starts_reports_progress_and_completes() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("complete");
    let child = ExportHandle::start(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        request(destination.clone(), 1),
    )
    .unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.state, JobState::Done, "{status:?}");
    assert_eq!(status.progress, 1.);
    assert!(destination.is_dir());
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 30);
}
#[test]
fn child_export_cancel_does_not_publish_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("cancelled");
    let mut child = ExportHandle::start(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        request(destination.clone(), 600),
    )
    .unwrap();
    let start = Instant::now();
    while child.status().progress == 0. {
        let status = child.status();
        assert!(!status.state.is_terminal(), "{status:?}");
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(5));
    }
    child.cancel().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.state, JobState::Cancelled, "{status:?}");
    assert!(!destination.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "temporary output not cleaned up"
    );
}

#[test]
fn interactive_activity_defers_export_and_cancel_wakes_the_gate() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let activity = Arc::new(AtomicBool::new(true));
    let mut child = ExportHandle::start_scheduled(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        request(dir.path().join("deferred"), 1),
        activity.clone(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        child.status().progress,
        0.,
        "export submitted while interactive"
    );
    activity.store(false, Ordering::Release);
    assert_eq!(child.wait().unwrap().state, JobState::Done);
    activity.store(true, Ordering::Release);
    child = ExportHandle::start_scheduled(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        request(dir.path().join("cancel-deferred"), 1),
        activity,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    child.cancel().unwrap();
    assert_eq!(child.wait().unwrap().state, JobState::Cancelled);
    assert!(!dir.path().join("cancel-deferred").exists());
}

#[test]
fn child_export_services_font_requests_for_visible_text() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("text");
    let mut request = request(destination.clone(), 1);
    let mut doc: serde_json::Value = serde_json::from_str(&request.document).unwrap();
    let comp = doc["compId"].as_str().unwrap().to_owned();
    doc["comps"][&comp]["layers"] = serde_json::json!([{
        "id":"synthetic-title","type":"text","name":"Synthetic title","from":0,"dur":1,"on":true,
        "d":{"text":"Synthetic font request","font":"Helvetica","size":32,"color":"#FFFFFF"},
        "p":{"position.x":{"v":160,"kf":[],"expr":null},"position.y":{"v":90,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null},"opacity":{"v":100,"kf":[],"expr":null}},"fx":[],"masks":[]
    }]);
    doc["comps"][&comp]["bg"] = serde_json::json!("#000000");
    doc["comps"][&comp]["backgroundFill"] =
        serde_json::json!({"type":"solid","stops":[{"id":"black","color":"#000000","position":0}]});
    request.document = doc.to_string();
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Done, "{status:?}");
    let mut frames = std::fs::read_dir(&destination)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    frames.sort();
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&frames[0])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let white = decoded
        .stdout
        .chunks_exact(3)
        .filter(|p| p.iter().all(|c| *c > 200))
        .count();
    assert!(
        white > 20,
        "text export silently lost its glyphs: {white} bright pixels"
    );
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 30);
}

#[test]
fn an_export_that_would_fill_the_disk_fails_without_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("full");
    let mut request = request(destination.clone(), 1);
    request.disk_reserve = u64::MAX;
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    assert!(
        status.error.unwrap().message.contains("almost full"),
        "the reason is the disk"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// The formats the Export sheet now offers, encoded by the real child and read
/// back: ProRes 422 HQ, HEVC Main 10, and a 16-bit PNG.
#[test]
fn child_exports_prores_422_hevc_10_bit_and_16_bit_png() {
    use pm_protocol::{
        color::QuickTimeTag,
        priority::{Container, ProResProfile},
    };
    let dir = tempfile::tempdir().unwrap();
    let video = OutputColor::Rec709QuickTime {
        candidate: QuickTimeTag::Cicp1_13_1,
    };
    for (name, format, color, alpha, codec, depth) in [
        (
            "hq.mov",
            ExportFormat::ProRes {
                profile: ProResProfile::Hq,
            },
            video,
            AlphaOutput::None,
            "prores",
            10,
        ),
        (
            "main10.mp4",
            ExportFormat::Hevc {
                ten_bit: true,
                container: Container::Mp4,
            },
            video,
            AlphaOutput::None,
            "hevc",
            10,
        ),
        (
            "still.png",
            ExportFormat::PngStill { sixteen_bit: true },
            OutputColor::SrgbImage,
            AlphaOutput::Straight,
            "png",
            16,
        ),
    ] {
        let destination = dir.path().join(name);
        let mut request = request(destination.clone(), 1);
        let JobSpec::Export { settings, .. } = &mut request.spec else {
            unreachable!()
        };
        settings.format = format;
        settings.color = color;
        settings.alpha = alpha;
        if format.is_image() {
            settings.range.end = Ticks::new(settings.range.start.get() + 1).unwrap();
        }
        let status =
            ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
                .unwrap()
                .wait()
                .unwrap();
        assert_eq!(status.state, JobState::Done, "{name}: {status:?}");
        if format.is_image() {
            // IHDR: the bit depth is byte 24 of a PNG.
            let png = std::fs::read(&destination).unwrap();
            assert_eq!(&png[1..4], b"PNG");
            assert_eq!(png[24], depth, "{name}");
            continue;
        }
        let source = pm_media::decode::service::MediaSource::open(&destination).unwrap();
        assert_eq!(source.info.codec, codec, "{name}");
        assert!(source.info.bit_depth >= depth, "{name}: {:?}", source.info);
    }
}

/// The export child installs the extension definitions its request pins.
#[test]
fn child_export_renders_extension_effects_from_its_resources() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("ext.png");
    let mut request = request(destination.clone(), 1);
    let mut document: serde_json::Value = serde_json::from_str(&request.document).unwrap();
    for comp in document["comps"].as_object_mut().unwrap().values_mut() {
        for layer in comp["layers"].as_array_mut().unwrap() {
            layer["fx"] = serde_json::json!([{"id":"e1","type":"ext.invert","on":true,"p":{}}]);
        }
    }
    request.document = document.to_string();
    let JobSpec::Export { settings, .. } = &mut request.spec else {
        unreachable!()
    };
    settings.format = ExportFormat::PngStill { sixteen_bit: false };
    settings.range.end = Ticks::new(settings.range.start.get() + 1).unwrap();
    let run = |request: ExportRequest| {
        ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
            .unwrap()
            .wait()
            .unwrap()
    };
    let without = run(serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap());
    assert_eq!(
        without.state,
        JobState::Failed,
        "an unknown effect never exports silently"
    );
    let mut invert = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    invert.id = "ext.invert".into();
    invert.origin = pm_protocol::definitions::DefinitionOrigin::Local;
    request.resources.generation = 1;
    request.resources.definitions = vec![invert];
    let status = run(request);
    assert_eq!(status.state, JobState::Done, "{status:?}");
    assert!(destination.is_file());
}

#[test]
fn a_preview_still_draws_offline_media_and_an_export_refuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));
    let refused = ExportHandle::start(exe, offline_request(dir.path().join("export"), false))
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    assert!(
        refused
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("offline")),
        "{refused:?}"
    );
    let preview = dir.path().join("preview");
    let drawn = ExportHandle::start(exe, offline_request(preview.clone(), true))
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(drawn.state, JobState::Done, "{drawn:?}");
    assert_eq!(std::fs::read_dir(preview).unwrap().count(), 30);
}
