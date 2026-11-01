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
    let project = pm_model::json::parse(include_str!("fixtures/shapes.json")).unwrap();
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
