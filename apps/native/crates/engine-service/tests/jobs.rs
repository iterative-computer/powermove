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
    let child = ExportHandle::start(
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
    child.cancel();
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
    let session = JobSession::new(request(dir.path().join("cancel-deferred"), 1));
    let cancellation = session.cancellation();
    child = ExportHandle::start_scheduled(
        Path::new(env!("CARGO_BIN_EXE_pm-engine-service")),
        session,
        activity,
    )
    .unwrap();
    let start = Instant::now();
    while child.gpu_requests() == 0 {
        assert!(start.elapsed() < Duration::from_secs(30), "no GPU request");
        std::thread::sleep(Duration::from_millis(1));
    }
    child.cancel();
    assert_eq!(child.wait().unwrap().state, JobState::Cancelled);
    assert!(!dir.path().join("cancel-deferred").exists());
    assert!(cancellation.load(std::sync::atomic::Ordering::Acquire));
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
    let error = refused.error.unwrap();
    assert_eq!(
        error.message,
        "1 clip is offline: gone.png. Relink the originals, or export without them."
    );
    // The Export sheet offers Export Without Them on exactly this refusal.
    use pm_protocol::error::{ErrorCode, ErrorDetail};
    assert_eq!(error.code, ErrorCode::MediaOffline);
    assert_eq!(
        error.detail,
        Some(ErrorDetail::Offline {
            assets: vec!["Aimage".into()]
        })
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
/// A clip the export would hear stops it by name on the first attempt and on
/// Export Without Them alike: going without it can't succeed, so it is never
/// offered as a MediaOffline refusal.
#[test]
fn going_without_offline_clips_never_drops_sound_the_export_would_hear() {
    let project = with_offline_audio();
    for without_offline in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut request = request_for(&project, dir.path().join("talk.mov"), 1);
        request.without_offline = without_offline;
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
        let status = finish(request);
        assert_eq!(status.state, JobState::Failed, "{status:?}");
        let error = status.error.unwrap();
        assert!(
            error
                .message
                .contains("Audio media is missing for “Voiceover”"),
            "{}",
            error.message
        );
        assert_eq!(error.code, pm_protocol::error::ErrorCode::ExportFailed);
    }
}
/// A stereo 16-bit WAV of a full-scale 440 Hz square, `seconds` long.
fn loud_wav(path: &Path, seconds: u32) {
    let rate = 48_000u32;
    let frames = rate * seconds;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 4);
    let data = frames * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for i in 0..frames {
        let v: i16 = if (i * 440 / rate) % 2 == 0 {
            i16::MAX
        } else {
            i16::MIN + 1
        };
        bytes.extend_from_slice(&v.to_le_bytes());
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// A finished export whose mixed sound went over full scale tells the
/// person so with its file: clipped in 24-bit PCM, kept in a float track;
/// and the check of the written file reports its progress on the way.
#[test]
fn a_finished_export_says_its_sound_was_clipped_or_kept() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("loud.wav");
    loud_wav(&wav, 1);
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    let mut layer = project["layers"][0].clone();
    layer["id"] = "Lloud".into();
    layer["type"] = "audio".into();
    layer["name"] = "Loud".into();
    // Four times full scale: 12 dB over.
    layer["d"] = serde_json::json!({"asset":"Aloud","gain":4,"trim":0,"fadeIn":0,"fadeOut":0});
    project["layers"].as_array_mut().unwrap().push(layer);
    project["assets"] =
        serde_json::json!({"Aloud":{"name":"loud.wav","type":"audio","format":"wav","dur":1}});
    for (coding, word) in [
        (None, "Audio clipped:"),
        (
            Some(pm_protocol::priority::AudioCoding::PcmFloat),
            "Audio over full scale:",
        ),
    ] {
        let mut request = request_for(
            &project.to_string(),
            dir.path().join(format!("loud-{word}.mov")),
            1,
        );
        request.originals.insert("Aloud".into(), wav.clone());
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
        settings.delivery.audio_coding = coding;
        // Every status until the end: the written file's check is reported
        // as it reads the file.
        let handle =
            ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), request)
                .unwrap();
        let mut status = handle.status();
        let mut checked = vec![];
        while !matches!(
            status.state,
            JobState::Done | JobState::Failed | JobState::Cancelled
        ) {
            status = handle.next_status(&status);
            checked.extend(status.checking);
        }
        let status = handle.wait().unwrap();
        assert!(!checked.is_empty(), "no check progress");
        assert_eq!(status.state, JobState::Done, "{status:?}");
        assert!(
            status.notes.iter().any(|n| n.starts_with(word)),
            "{word} {:?}",
            status.notes
        );
    }
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
/// A delivery sent while extensions still load (the app waits, so only a
/// stray one) never says the effect isn't installed.
#[test]
fn a_delivery_sent_while_extensions_load_never_calls_an_effect_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    let mut early = request_for(&with_uninstalled_effect(true), dir.path().join("early"), 1);
    early.resources.pending = true;
    let refused = finish(early);
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    let message = refused.error.unwrap().message;
    assert!(
        message.contains("acme.glow is still loading") && !message.contains("installed"),
        "{message}"
    );
}
/// An agent's still is a look at the project, as the viewer shows it: an
/// extension layer whose extension isn't installed draws today's
/// missing-definition checkerboard, not a refusal of the whole frame. A
/// delivery still refuses it by name.
#[test]
fn a_preview_still_shows_a_layer_without_a_definition_and_an_export_refuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    let layers = project["layers"].as_array_mut().unwrap();
    let mut field = layers[0].clone();
    field["id"] = "field".into();
    field["type"] = "extension".into();
    field["d"] = serde_json::json!({"definition":"acme.field.layer","version":1,
        "w":{"v":640,"kf":[],"expr":null},"h":{"v":360,"kf":[],"expr":null},"data":{},"params":{}});
    field["fx"] = serde_json::json!([]);
    layers.insert(0, field);
    let doc = project.to_string();
    let mut still = request_for(&doc, dir.path().join("still"), 1);
    still.preview = true;
    still.bypass_missing_effects = true;
    let drawn = finish(still);
    assert_eq!(drawn.state, JobState::Done, "{drawn:?}");
    let refused = finish(request_for(&doc, dir.path().join("delivery"), 1));
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    let message = refused.error.unwrap().message;
    assert!(message.contains("acme.field.layer"), "{message}");
}

/// A broken installed shader leaves its own layer out of a viewer still;
/// it must not prevent the rest of the project from opening or rendering.
/// Delivery captures continue to report the invalid definition.
#[test]
fn a_preview_still_isolates_an_invalid_extension_shader_and_deliveries_refuse_it() {
    use pm_protocol::definitions::*;
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    let one = |document: &str, destination: std::path::PathBuf, preview| {
        let mut r = request_for(document, destination, 1);
        r.preview = preview;
        let JobSpec::Export { settings, .. } = &mut r.spec else {
            unreachable!()
        };
        settings.format = ExportFormat::PngStill { sixteen_bit: false };
        settings.range.end = Ticks::new(settings.range.start.get() + 1).unwrap();
        r
    };
    let baseline_path = dir.path().join("baseline.png");
    let baseline = finish(one(&project.to_string(), baseline_path.clone(), true));
    assert_eq!(baseline.state, JobState::Done, "{baseline:?}");

    let mut field = project["layers"][0].clone();
    field["id"] = "broken".into();
    field["type"] = "extension".into();
    field["d"] = json!({"definition":"acme.invalid.layer","version":1,
        "w":{"v":160},"h":{"v":90},"params":{"enabled":{"v":true}},"data":{}});
    field["fx"] = json!([]);
    project["layers"].as_array_mut().unwrap().insert(0, field);
    // A float comparison on a declared boolean is invalid in WebGL as well
    // as in the native validator. Do not repair the author's shader here.
    let fragment = "out vec4 result; void main(){result=vec4(u_enabled > 0.5 ? 1.0 : 0.0);}";
    let prefix = format!(
        "{}\nuniform bool u_enabled;\n",
        pm_render::shader_layers::header()
    );
    let definition = DefinitionSnapshot {
        id: "acme.invalid.layer".into(),
        kind: DefinitionKind::Layer,
        version: 1,
        origin: DefinitionOrigin::Local,
        renderer: Some(LayerRenderer::Fragment {
            fragment: fragment.into(),
        }),
        glsl: Some(WrappedGlsl {
            source: format!("{prefix}{fragment}"),
            body_line_offset: prefix.matches('\n').count() as u32,
            body_utf16_len: fragment.encode_utf16().count() as u32,
            body_utf8_len: fragment.len() as u32,
        }),
        params: vec![ParamSpec::Toggle {
            key: "enabled".into(),
            default: true,
        }],
        flags: DefinitionFlags::default(),
    };
    let still_path = dir.path().join("still.png");
    let mut still = one(&project.to_string(), still_path.clone(), true);
    still.bypass_missing_effects = true;
    still.resources.definitions = vec![definition.clone()];
    let drawn = finish(still);
    assert_eq!(drawn.state, JobState::Done, "{drawn:?}");
    assert_eq!(
        std::fs::read(still_path).unwrap(),
        std::fs::read(baseline_path).unwrap(),
        "the rest of the scene must retain the baseline's pixels"
    );

    let mut delivery = one(&project.to_string(), dir.path().join("delivery.png"), false);
    delivery.resources.definitions = vec![definition];
    let refused = finish(delivery);
    assert_eq!(refused.state, JobState::Failed, "{refused:?}");
    let message = refused.error.unwrap().message;
    assert!(
        message.contains("acme.invalid.layer") && message.contains("does not compile"),
        "{message}"
    );
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

/// The fixture with one text layer: a layer a `raster` override draws.
fn text_request(destination: std::path::PathBuf) -> ExportRequest {
    let mut request = request(destination, 1);
    let mut doc: serde_json::Value = serde_json::from_str(&request.document).unwrap();
    let comp = doc["compId"].as_str().unwrap().to_owned();
    doc["comps"][&comp]["layers"] = serde_json::json!([{
        "id":"title","type":"text","name":"Title","from":0,"dur":1,"on":true,
        "d":{"text":"Overridden","font":"Helvetica","size":32,"color":"#FFFFFF"},
        "p":{"position.x":{"v":160,"kf":[],"expr":null},"position.y":{"v":90,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null},"opacity":{"v":100,"kf":[],"expr":null}},"fx":[],"masks":[]
    }]);
    request.document = doc.to_string();
    request
}
/// A job's overrides are answered by what its session was given: every
/// frame asks, and the answers draw.
#[test]
fn an_export_draws_its_overrides_from_its_session() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("overridden");
    let mut session = JobSession::new(text_request(destination.clone()));
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    session.answer_overrides(Box::new(move |needs| {
        counted.fetch_add(needs.len(), Ordering::Relaxed);
        Ok(needs.iter().map(|_| CompatReply::Hidden).collect())
    }));
    let cancellation = session.cancellation();
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), session)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Done, "{status:?}");
    assert_eq!(asked.load(Ordering::Relaxed), 30, "one answer per frame");
    assert!(
        cancellation.load(Ordering::Acquire),
        "the finished job's pin is let go"
    );
}
/// Cancelled while an extension is drawing a layer for it: the export
/// stops, nothing is published, and its pin is let go.
#[test]
fn cancelling_while_an_extension_draws_stops_the_export_and_lets_go_of_its_pin() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("cancelled.png");
    let mut session = JobSession::new(text_request(destination.clone()));
    let cancellation = session.cancellation();
    let pinned = cancellation.clone();
    let (asking, asked) = std::sync::mpsc::channel();
    session.answer_overrides(Box::new(move |_| {
        let _ = asking.send(());
        while !pinned.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("cancelled".into())
    }));
    let child =
        ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), session).unwrap();
    asked
        .recv_timeout(Duration::from_secs(30))
        .expect("the export asked for its override layer");
    child.cancel();
    let status = child.wait().unwrap();
    assert_eq!(status.state, JobState::Cancelled, "{status:?}");
    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
/// The window answering an export's overrides closes (or reloads its
/// extensions) mid-export: the export fails with the reason and publishes
/// nothing.
#[test]
fn an_export_whose_extensions_stop_fails_with_the_reason() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let mut session = JobSession::new(text_request(dir.path().join("closed")));
    let frames = AtomicUsize::new(0);
    session.answer_overrides(Box::new(move |needs| {
        if frames.fetch_add(1, Ordering::Relaxed) < 3 {
            return Ok(needs.iter().map(|_| CompatReply::Native).collect());
        }
        Err("The project's extensions stopped during the export".into())
    }));
    let status = ExportHandle::start(Path::new(env!("CARGO_BIN_EXE_pm-engine-service")), session)
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    let message = status.error.unwrap().message;
    assert_eq!(
        message,
        "An extension could not draw a layer: The project's extensions stopped during the export"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
/// A job that never starts, or whose child ends without saying how the
/// export went, lets go of what its session pinned.
#[test]
fn a_job_that_fails_to_start_or_loses_its_child_lets_go_of_its_pin() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let session = JobSession::new(request(dir.path().join("never"), 1));
    let unstarted = session.cancellation();
    assert!(ExportHandle::start(Path::new("/nonexistent/pm-engine-job"), session).is_err());
    assert!(unstarted.load(Ordering::Acquire));
    let session = JobSession::new(request(dir.path().join("lost"), 1));
    let lost = session.cancellation();
    let child = ExportHandle::start(&fake_child(dir.path(), "exit 0"), session).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    assert!(lost.load(Ordering::Acquire));
}
/// A stand-in child: reads its request, then runs `script`.
fn fake_child(dir: &Path, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("child.sh");
    std::fs::write(&path, format!("#!/bin/sh\nread request\n{script}\n")).unwrap();
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
        checking: None,
        notes: vec![],
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
            &fake_child(dir.path(), &format!("echo '{line}'\nexec sleep 60")),
            request(dir.path().join("out"), 1),
        )
        .unwrap();
        let started = Instant::now();
        let status = child.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "{line}");
        assert_eq!(status.state, JobState::Failed, "{line}: {status:?}");
        let message = status.error.unwrap().message;
        eprintln!("MESSAGE {message}");
        assert!(message.contains(reason), "{line}: {message}");
    }
}

/// File `n`'s colour in a coded sequence: far apart in 8 bits, so a lossy
/// 4:2:0 delivery still names its frame.
fn sequence_colour(n: u16) -> [u16; 3] {
    [n * 9_000, 60_000 - n * 9_000, 20_000 + n * 5_000]
}
/// Six 16-bit frames `shot.0001.png`… read in place at 15 fps (the small
/// file a sequence asset retains), and the fixture with one layer of it
/// filling the frame.
fn with_sequence(dir: &Path) -> (String, MediaCatalog) {
    for n in 1..=6u16 {
        let mut bytes = Vec::new();
        for _ in 0..320 * 180 {
            for code in sequence_colour(n) {
                bytes.extend_from_slice(&code.to_be_bytes());
            }
        }
        let file = std::fs::File::create(dir.join(format!("shot.{n:04}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, 320, 180);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&bytes).unwrap();
    }
    let manifest = pm_media::image::sequence::Manifest {
        powermove_image_sequence: 1,
        dir: dir.into(),
        prefix: "shot.".into(),
        suffix: ".png".into(),
        padding: 4,
        first: 1,
        last: 6,
        rate: [15, 1],
    };
    let retained = dir.join("retained");
    std::fs::write(&retained, manifest.to_bytes()).unwrap();
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    project["assets"] = serde_json::json!({"seq": {
        "id": "seq", "name": "shot.[0001-0006].png", "kind": "video", "format": "png",
        "w": 320, "h": 180, "dur": 0.4, "fps": 15, "hasAudio": false, "persisted": true,
        "imageSequence": {"fps": 15, "frames": 6, "first": 1, "last": 6}
    }});
    let mut layer = project["layers"][0].clone();
    layer["id"] = "Lseq".into();
    layer["type"] = "video".into();
    layer["dur"] = 0.4.into();
    layer["p"]["position.x"]["v"] = 320.into();
    layer["p"]["position.y"]["v"] = 180.into();
    layer["p"]["opacity"]["v"] = 100.into();
    layer["d"] = serde_json::json!({"asset": "seq", "w": 640, "h": 360, "fit": "stretch"});
    project["layers"] = serde_json::json!([layer]);
    let mut media = MediaCatalog::default();
    media.originals.insert("seq".into(), retained);
    (project.to_string(), media)
}
/// The centre pixel of every frame of a delivered movie, as 16-bit RGB.
fn centres(movie: &Path) -> Vec<[u16; 3]> {
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let out = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(movie)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb48le", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let frame = 320 * 180 * 6;
    out.stdout
        .chunks_exact(frame)
        .map(|f| {
            let at = (90 * 320 + 160) * 6;
            [0, 2, 4].map(|c| u16::from_le_bytes([f[at + c], f[at + c + 1]]))
        })
        .collect()
}
/// An image sequence read in place exports through the export route as H.264
/// and ProRes with each of its files at its own times (15 fps into 30: file
/// n on frames 2(n-1) and 2(n-1)+1); a file deleted from its folder fails
/// the export naming it, and no movie is published.
/// A file of an image sequence that is there but cannot be read (a bad
/// copy) stops the export naming that file, as a damaged frame of a movie
/// does; nothing stands in for it and no movie is published.
#[test]
fn a_damaged_file_of_an_image_sequence_stops_the_export_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    let (project, media) = with_sequence(&frames);
    std::fs::write(frames.join("shot.0004.png"), vec![0x5a; 4096]).unwrap();
    let destination = dir.path().join("damaged.mov");
    let mut request = request_for(&project, destination.clone(), 1);
    request.originals = media.originals.clone();
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
    settings.range.end = Ticks::new(Ticks::from_whole_seconds(2).get() / 5).unwrap();
    let status = finish(request);
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    let message = status.error.unwrap().message;
    for part in ["couldn't be exported", "damaged", "shot.0004.png"] {
        assert!(message.contains(part), "{part} in: {message}");
    }
    assert!(!message.contains("cannot render this yet"), "{message}");
    assert!(!destination.exists());
}
#[test]
fn an_image_sequence_read_in_place_exports_its_frames_at_their_times() {
    use pm_protocol::{color::QuickTimeTag, priority::ProResProfile};
    let dir = tempfile::tempdir().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    let (project, media) = with_sequence(&frames);
    let video = OutputColor::Rec709QuickTime {
        candidate: QuickTimeTag::Cicp1_13_1,
    };
    let formats = [
        ("h264.mp4", ExportFormat::H264),
        (
            "prores.mov",
            ExportFormat::ProRes {
                profile: ProResProfile::Hq,
            },
        ),
    ];
    let request = |name: &str, format: ExportFormat| {
        let mut request = request_for(&project, dir.path().join(name), 1);
        request.originals = media.originals.clone();
        let JobSpec::Export { settings, .. } = &mut request.spec else {
            unreachable!()
        };
        settings.format = format;
        settings.color = video;
        settings.alpha = AlphaOutput::None;
        settings.range.end = Ticks::new(Ticks::from_whole_seconds(2).get() / 5).unwrap();
        request
    };
    for (name, format) in formats {
        let status = finish(request(name, format));
        assert_eq!(status.state, JobState::Done, "{name}: {status:?}");
        let shown = centres(&dir.path().join(name));
        assert_eq!(shown.len(), 12, "{name}");
        for (k, got) in shown.iter().enumerate() {
            let want = sequence_colour(k as u16 / 2 + 1);
            for c in 0..3 {
                assert!(
                    (i32::from(got[c]) - i32::from(want[c])).abs() < 2_000,
                    "{name} frame {k}: {got:?}, file {} is {want:?}",
                    k / 2 + 1
                );
            }
        }
    }
    std::fs::remove_file(frames.join("shot.0004.png")).unwrap();
    for (name, format) in formats {
        let name = format!("missing-{name}");
        let status = finish(request(&name, format));
        assert_eq!(status.state, JobState::Failed, "{name}: {status:?}");
        assert_eq!(
            status.error.unwrap().message,
            "Frame 4 of an image sequence is missing: shot.0004.png. Render it into the \
             sequence’s folder, or relink the sequence.",
            "{name}"
        );
        assert!(!dir.path().join(&name).exists(), "{name}");
    }
}
/// A short ProRes export of the shapes fixture to `destination`.
fn prores_request(destination: std::path::PathBuf) -> ExportRequest {
    let mut request = request(destination, 1);
    as_prores(&mut request);
    request
}
/// A fifth of a second of ProRes 422 HQ.
fn as_prores(request: &mut ExportRequest) {
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
    settings.range.end = Ticks::new(Ticks::from_whole_seconds(2).get() / 5).unwrap();
}
/// The files the export archive holds for `name` beside it.
fn archived(folder: &Path, name: &str) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(folder.join(".powermove-export-archive")) else {
        return vec![];
    };
    entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(name))
        .map(|p| std::fs::read(p).unwrap())
        .collect()
}
/// An existing file is replaced only when the export was told to (the
/// person said Replace): then only once the whole export and its check
/// succeeded, the old file kept in the export archive. Untold, it is never
/// replaced and the failure names it; a failed export leaves it in place.
#[test]
fn an_existing_file_is_replaced_only_when_told_and_the_old_one_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let film = dir.path().join("film.mov");
    let old = b"the last export".to_vec();
    std::fs::write(&film, &old).unwrap();
    let untold = finish(prores_request(film.clone()));
    assert_eq!(untold.state, JobState::Failed, "{untold:?}");
    assert!(untold.error.unwrap().message.contains("film.mov"));
    assert_eq!(std::fs::read(&film).unwrap(), old);
    assert!(archived(dir.path(), "film.mov").is_empty());
    // A replace that fails part way (a damaged file of a sequence) leaves
    // the existing file as it was.
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    let (project, media) = with_sequence(&frames);
    std::fs::write(frames.join("shot.0004.png"), vec![0x5a; 4096]).unwrap();
    let mut failing = request_for(&project, film.clone(), 1);
    failing.originals = media.originals.clone();
    as_prores(&mut failing);
    failing.replace = true;
    let failed = finish(failing);
    assert_eq!(failed.state, JobState::Failed, "{failed:?}");
    assert_eq!(std::fs::read(&film).unwrap(), old);
    assert!(archived(dir.path(), "film.mov").is_empty());
    let mut told = prores_request(film.clone());
    told.replace = true;
    let done = finish(told);
    assert_eq!(done.state, JobState::Done, "{done:?}");
    let now = std::fs::read(&film).unwrap();
    assert_ne!(now, old);
    assert_eq!(&now[4..8], b"ftyp", "a movie replaced the old file");
    assert_eq!(archived(dir.path(), "film.mov"), vec![old]);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
/// A file Powermove may not read is not called damaged: the export says
/// what happened and what to do, and publishes nothing.
#[test]
fn an_unreadable_file_of_an_image_sequence_says_why_it_could_not_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    let (project, media) = with_sequence(&frames);
    let locked = frames.join("shot.0004.png");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let destination = dir.path().join("locked.mov");
    let mut request = request_for(&project, destination.clone(), 1);
    request.originals = media.originals.clone();
    as_prores(&mut request);
    let status = finish(request);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(status.state, JobState::Failed, "{status:?}");
    let message = status.error.unwrap().message;
    for part in ["couldn't be exported", "shot.0004.png", "permission"] {
        assert!(message.contains(part), "{part} in: {message}");
    }
    for wrong in ["damaged", "Replace or trim"] {
        assert!(!message.contains(wrong), "{wrong} in: {message}");
    }
    assert!(!destination.exists());
}
