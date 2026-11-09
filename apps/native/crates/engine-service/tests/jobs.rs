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
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::default();
    activity.set(true);
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
    activity.set(false);
    assert_eq!(child.wait().unwrap().state, JobState::Done);
    activity.set(true);
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

/// A quiet viewer lets an export submit whole frames (one GPU submission per
/// frame); right after viewer activity its submissions are bounded, so the
/// first frame is split (the job is not measured yet: one band probes it).
#[test]
fn quiet_viewer_exports_whole_frames_and_activity_bands_them() {
    let dir = tempfile::tempdir().unwrap();
    let run = |name: &str, busy: bool| {
        let activity = Activity::default();
        activity.set(busy);
        let child = ExportHandle::start_scheduled(
            Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
            request(dir.path().join(name), 1),
            activity.clone(),
        )
        .unwrap();
        // Activity ends only once the child is waiting for its first
        // submission: it is admitted right after activity, never before.
        let start = Instant::now();
        while busy && child.gpu_requests() == 0 {
            assert!(start.elapsed() < Duration::from_secs(30), "no GPU request");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            child.submissions(),
            0,
            "submitted while the viewer was busy"
        );
        activity.set(false);
        while !child.status().state.is_terminal() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let submissions = child.submissions();
        assert_eq!(child.wait().unwrap().state, JobState::Done);
        submissions
    };
    // 30 frames of 180 rows: 12 bands each.
    let quiet = run("quiet", false);
    assert_eq!(quiet, 30, "a quiet viewer still banded the export");
    let active = run("active", true);
    assert!(
        active > 30,
        "frames right after viewer activity were not banded ({active} submissions)"
    );
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

/// The fixture plus an image layer whose original is gone.
fn with_offline_image() -> String {
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    let mut layer = project["layers"][0].clone();
    layer["id"] = "Limage".into();
    layer["type"] = "image".into();
    layer["name"] = "Gone".into();
    layer["d"] = serde_json::json!({"asset":"Aimage"});
    project["layers"].as_array_mut().unwrap().push(layer);
    project["assets"] =
        serde_json::json!({"Aimage":{"name":"gone.png","type":"image","w":40,"h":40}});
    project.to_string()
}
#[test]
fn an_offline_clip_refuses_by_name_unless_the_export_goes_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = with_offline_image();
    let refused = ExportHandle::start(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        request_for(&project, dir.path().join("refused"), 1),
    )
    .unwrap()
    .wait()
    .unwrap();
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    let message = refused.error.unwrap().message;
    assert!(message.contains("1 clip is offline: gone.png"), "{message}");
    // The Export sheet offers Export Without Them on exactly this refusal.
    assert!(
        pm_engine_service::jobs::is_offline_refusal(&message),
        "{message}"
    );
    let mut without = request_for(&project, dir.path().join("without"), 1);
    without.without_offline = true;
    let done = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), without)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(done.state, JobState::Done, "{done:?}");
    assert_eq!(
        std::fs::read_dir(dir.path().join("without"))
            .unwrap()
            .count(),
        30
    );
}
/// The fixture plus an audio clip whose original is gone.
fn with_offline_audio() -> String {
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    let mut layer = project["layers"][0].clone();
    layer["id"] = "Lvoice".into();
    layer["type"] = "audio".into();
    layer["name"] = "Voiceover".into();
    layer["d"] = serde_json::json!({"asset":"Avoice","gain":1,"trim":0,"fadeIn":0,"fadeOut":0});
    project["layers"].as_array_mut().unwrap().push(layer);
    project["assets"] =
        serde_json::json!({"Avoice":{"name":"voice.wav","type":"audio","format":"wav","dur":3}});
    project.to_string()
}
#[test]
fn going_without_offline_clips_never_drops_sound_the_export_would_hear() {
    let dir = tempfile::tempdir().unwrap();
    let project = with_offline_audio();
    let mut request = request_for(&project, dir.path().join("talk.mov"), 1);
    request.without_offline = true;
    let JobSpec::Export { settings, .. } = &mut request.spec else {
        unreachable!()
    };
    settings.format = ExportFormat::ProRes {
        profile: pm_protocol::priority::ProResProfile::Hq,
    };
    settings.color = OutputColor::Rec709QuickTime {
        candidate: pm_protocol::color::QuickTimeTag::Cicp1_13_1,
    };
    settings.alpha = AlphaOutput::None;
    settings.audio = true;
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    let message = status.error.unwrap().message;
    assert!(
        message.contains("Audio media is missing for “Voiceover”"),
        "{message}"
    );
}
/// The fixture's first layer with an effect no installed extension provides.
fn with_uninstalled_effect(on: bool) -> String {
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    project["layers"][0]["fx"] = serde_json::json!([
        {"id":"e1","type":"acme.glow","on":on,"p":{}}
    ]);
    project.to_string()
}
fn finish(request: ExportRequest) -> pm_protocol::priority::JobStatus {
    ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
        .unwrap()
        .wait()
        .unwrap()
}
#[test]
fn a_still_capture_bypasses_an_uninstalled_effect_and_a_delivery_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let enabled = with_uninstalled_effect(true);
    let mut capture = request_for(&enabled, dir.path().join("capture"), 1);
    capture.bypass_missing_effects = true;
    let done = finish(capture);
    assert_eq!(done.state, JobState::Done, "{done:?}");
    let refused = finish(request_for(&enabled, dir.path().join("delivery"), 1));
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    let message = refused.error.unwrap().message;
    assert!(message.contains("acme.glow isn't installed"), "{message}");
    // Switched off, it never draws, so a delivery doesn't need it.
    let off = finish(request_for(
        &with_uninstalled_effect(false),
        dir.path().join("off"),
        1,
    ));
    assert_eq!(off.state, JobState::Done, "{off:?}");
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

/// A delivery's override answerer starts (pinning the extensions' code and
/// storage) when the export starts; a one-frame look does not.
#[test]
fn deliveries_begin_their_override_answerer_at_start_and_looks_do_not() {
    use std::sync::{Arc, Mutex};
    let session = 987_654;
    let begun: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let seen = begun.clone();
    set_compat_answerer(
        session,
        Some((
            Arc::new(|| Some(1)),
            Arc::new(|_, _| Ok(vec![])),
            Arc::new(move |pin: &CompatPin| {
                seen.lock().unwrap().push((pin.job.clone(), pin.look));
            }),
        )),
    );
    let dir = tempfile::tempdir().unwrap();
    for (job, look) in [("delivery", false), ("look", true)] {
        let mut request = request(dir.path().join(format!("{job}.mp4")), 1);
        request.job = job.into();
        request.look = look;
        if let JobSpec::Export { session: s, .. } = &mut request.spec {
            *s = SessionId::new(session).unwrap();
        }
        // No such executable: the job never runs, but it has started.
        let _ = ExportHandle::start(Path::new("/nonexistent/pm-engine-job"), request);
    }
    set_compat_answerer(session, None);
    assert_eq!(*begun.lock().unwrap(), vec![("delivery".to_owned(), false)]);
}

/// A stand-in child: reads its request, then says `lines` and waits.
fn fake_child(dir: &Path, lines: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("child.sh");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nread request\ncat <<'END'\n{lines}\nEND\nexec sleep 60\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
/// A child line the parent can't use (unknown, malformed, or about another
/// job) fails the export now, with the reason, and the child is stopped:
/// nothing waits for a timeout.
#[test]
fn a_child_message_the_parent_cannot_use_fails_the_export_at_once() {
    let mut another = serde_json::to_value(pm_protocol::priority::JobStatus {
        job: "another".into(),
        class: pm_protocol::priority::PriorityClass::UserJob,
        state: JobState::Running,
        progress: 0.5,
        path: None,
        error: None,
        preview: None,
    })
    .unwrap();
    another["type"] = "status".into();
    let another = another.to_string();
    for (line, reason) in [
        (r#"{"compat":{"id":1,"needs":[]}}"#, "invalid message"),
        (r#"{"type":"compat","id":1}"#, "invalid message"),
        ("not json", "invalid message"),
        (another.as_str(), "another job"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let child = ExportHandle::start(
            &fake_child(dir.path(), line),
            request(dir.path().join("out"), 1),
        )
        .unwrap();
        let started = Instant::now();
        let status = child.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "{line}");
        assert_eq!(status.state, JobState::Failed, "{line}: {status:?}");
        let message = status.error.unwrap().message;
        assert!(message.contains(reason), "{line}: {message}");
    }
}
