//! A font installed or removed while a project is open.
//!
//! CoreText posts its registered-fonts-changed notification on the main run
//! loop, so this test is its own program (`harness = false`), shaped as the
//! app is: the main thread runs the run loop and answers the font asks; the
//! engine service and the scenario run beside it. The font is a fixture
//! registered for this process only (nothing is installed) and unregistered
//! again.

use pm_engine_service::*;
use pm_promedia::cache::BarState;
use pm_protocol::common::TICKS_PER_SECOND;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The fixture's family: no Mac has it installed.
const FAMILY: &str = "Recorder Test";

fn main() {
    let broker = pm_text::host::FontBroker::new();
    // The loop below answers asks every few milliseconds.
    broker.set_wake(Arc::new(|| {}));
    let mut fonts = pm_text::host::FontService::new().expect("the main thread's font service");
    let scenario = std::thread::Builder::new()
        .name("scenario".into())
        .spawn({
            let broker = broker.clone();
            move || a_font_installed_while_a_project_is_open_retires_its_rendered_frames(broker)
        })
        .unwrap();
    while !scenario.is_finished() {
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.005);
        objc2_foundation::NSRunLoop::mainRunLoop().runUntilDate(&until);
        pm_text::host::serve(&mut fonts, broker.take_asks());
        std::thread::sleep(Duration::from_millis(1));
    }
    if scenario.join().is_err() {
        println!(
            "test a_font_installed_while_a_project_is_open_retires_its_rendered_frames ... FAILED"
        );
        std::process::exit(101);
    }
    println!("test a_font_installed_while_a_project_is_open_retires_its_rendered_frames ... ok");
    println!("\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
}

/// One line of text in a family this Mac lacks, so it falls back.
fn snapshot() -> Snapshot {
    let mut project: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    project["layers"] = serde_json::json!([{
        "id":"title","type":"text","name":"Title","from":0,"dur":3,"on":true,
        "d":{"text":"ABBA","font":FAMILY,"size":120,"color":"#FFFFFF"},
        "p":{"position.x":{"v":320,"kf":[],"expr":null},"position.y":{"v":180,"kf":[],"expr":null},
             "scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null},
             "opacity":{"v":100,"kf":[],"expr":null}},"fx":[],"masks":[]
    }]);
    let project = pm_model::json::parse(&project.to_string()).unwrap();
    #[cfg(feature = "kernel")]
    {
        pm_kernel::document::DocumentStore::new_untitled(project).snapshot()
    }
    #[cfg(not(feature = "kernel"))]
    {
        Snapshot {
            doc_id: 1,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        }
    }
}

fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(60), "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The paused picture, captured once the viewer has settled on it.
fn picture(viewer: &Viewer) -> Vec<[f64; 3]> {
    viewer.capture_next().unwrap();
    let stamp = viewer.stamp().unwrap();
    let mut pixels = None;
    eventually("the captured frame", || {
        let status = viewer.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        pixels = status
            .frame
            .filter(|f| f.stamp == stamp)
            .and_then(|f| f.pixels);
        pixels.is_some()
    });
    pixels.unwrap()
}

/// Text that fell back because its family was absent is rendered and cached;
/// the family is then installed. Nothing in the document changes, yet the
/// frames rendered with the fallback are not this state's any more (the
/// render bar shows them unrendered at once) and the paused picture is drawn
/// again with the installed font. Removing the font brings the first state
/// back: its frames are rendered again and its picture is the first one.
fn a_font_installed_while_a_project_is_open_retires_its_rendered_frames(
    broker: pm_text::host::FontBroker,
) {
    let dir = tempfile::tempdir().unwrap();
    let font = dir.path().join("probe.ttf");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../recorder/tests/fixtures/recorder-test.ttf"),
        &font,
    )
    .unwrap();
    let service = EngineService::start_with_fonts(
        pm_gpu::Gpu::new("font-change-test").unwrap(),
        pm_render::text::host::RenderFonts::new(broker),
        None,
    )
    .unwrap();
    let disk = service.render_cache();
    disk.set_storage(Some((dir.path().join("frames"), 1 << 30)))
        .unwrap();
    let s = snapshot();
    let viewer = service
        .viewer(ViewerConfig {
            composition: s.open_comp().unwrap(),
            snapshot: s,
            media: Default::default(),
            output: Output {
                width: 320,
                height: 180,
                display: 0,
                headroom: 1.,
                crop: None,
            },
            layer: None,
            rate: pm_time::Rate::FPS_30,
            end: Some(3 * TICKS_PER_SECOND),
            audio: false,
        })
        .unwrap();
    let fallback = picture(&viewer);
    assert!(
        fallback.iter().any(|p| p[0] > 0.5),
        "the text is drawn (with a fallback font)"
    );
    // Frames 30 to 32 rendered by the fill child.
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));
    eventually("the scene is observed and the fill starts", || {
        disk.start_fill(1., 1.1, exe, jobs::Activity::default(), |_| {})
            .is_ok()
    });
    eventually("the fill ends", || {
        !disk.bars().running && disk.bars().done > 0
    });
    assert_eq!(disk.bars().error, None);
    let rendered = || {
        disk.bars()
            .bars
            .iter()
            .any(|(_, _, state)| *state == BarState::Rendered)
    };
    assert!(rendered(), "{:?}", disk.bars());

    pm_text::system_fonts::register_for_process(&font).unwrap();
    eventually("the frames rendered with the fallback are retired", || {
        !rendered()
    });
    let installed = picture(&viewer);
    assert_ne!(
        installed, fallback,
        "the paused picture is drawn with the installed font"
    );

    pm_text::system_fonts::unregister_for_process(&font).unwrap();
    eventually("the first state's frames are this state's again", rendered);
    assert_eq!(
        picture(&viewer),
        fallback,
        "the font is gone: the text falls back as before"
    );
    viewer.retire();
}
