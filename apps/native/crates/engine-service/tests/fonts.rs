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
    let mut pixels = None;
    eventually("the captured frame", || {
        let status = viewer.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        let stamp = viewer.stamp().unwrap();
        match status.frame.filter(|f| f.stamp == stamp) {
            Some(frame) if frame.pixels.is_some() => pixels = frame.pixels,
            // A font change landed since: the capture went to a frame of
            // the fonts before.
            Some(_) => viewer.capture_next().unwrap(),
            None => {}
        }
        pixels.is_some()
    });
    pixels.unwrap()
}

/// The viewer paused at `ticks` and settled on its composition-size picture
/// (drawn and refined, or the frame on disk): the frame's report and the
/// scope's reading of the picture left on screen.
fn settled(
    viewer: &Viewer,
    scope: &async_channel::Receiver<ScopeResult>,
    ticks: i64,
) -> (FrameReport, pm_render::scopes::ScopeFrame) {
    while scope.try_recv().is_ok() {}
    viewer
        .command(pm_protocol::clock::TransportCommand::Seek {
            time: pm_protocol::common::Ticks::new(ticks).unwrap(),
            mode: pm_protocol::clock::SeekMode::Exact,
        })
        .unwrap();
    let (mut last, mut heard) = (None, Instant::now());
    eventually("the scope reads the settled picture", || {
        if let Ok(frame) = scope.try_recv() {
            last = Some(frame.unwrap());
            heard = Instant::now();
        }
        // The fast picture is the drawable's size; the one that stays is
        // the composition's.
        last.as_ref().is_some_and(|f| f.pixels == 640 * 360)
            && heard.elapsed() > Duration::from_millis(800)
    });
    let status = viewer.status();
    assert!(status.error.is_none(), "{:?}", status.error);
    (status.frame.unwrap(), last.unwrap())
}

/// The first frame of the composition exported now (a PNG's bytes), or
/// why the export failed. `fonts` changes the request the app would send.
fn exported(
    to: &Path,
    fonts: impl FnOnce(&mut pm_text::system_fonts::Inventory),
) -> Result<Vec<u8>, String> {
    use pm_protocol::priority::JobState;
    let mut request = export_request(to);
    fonts(request.fonts.as_mut().expect("an export names its fonts"));
    let exe = Path::new(env!("CARGO_BIN_EXE_pm-engine-service"));
    let status = jobs::ExportHandle::start(exe, request)
        .unwrap()
        .wait()
        .unwrap();
    if status.state != JobState::Done {
        return Err(status.error.map(|e| e.message).unwrap_or_default());
    }
    let mut frames: Vec<_> = std::fs::read_dir(to)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    frames.sort();
    Ok(std::fs::read(&frames[0]).unwrap())
}

/// The app's request for the composition's first frame as a PNG in `to`.
fn export_request(to: &Path) -> jobs::ExportRequest {
    use pm_protocol::{
        color::{OutputColor, Precision},
        common::{FrameRate, SessionId, Size, TickRange, Ticks},
        priority::{AlphaOutput, ExportFormat, ExportSettings, JobSpec, QualityTier},
    };
    let s = snapshot();
    let settings = ExportSettings {
        format: ExportFormat::PngSequence { sixteen_bit: false },
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(320, 180),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(TICKS_PER_SECOND / 30).unwrap(),
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
    jobs::ExportRequest::new(
        "font-export".into(),
        &s,
        JobSpec::Export {
            session: SessionId::new(1).unwrap(),
            composition: s.open_comp().unwrap().as_str().into(),
            settings,
        },
        MediaCatalog::default(),
        to.to_owned(),
    )
    .unwrap()
}

/// The font file written again with the bytes it has, its modification
/// time put back: nothing a list of names, lengths and times shows, and no
/// registration changes. Another version of the file all the same (what a
/// font tool saving in place does, whatever it writes).
fn rewritten_in_place(font: &Path) {
    let bytes = std::fs::read(font).unwrap();
    let modified = std::fs::metadata(font).unwrap().modified().unwrap();
    // The status-change time has the file system's resolution.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(font, bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(font)
        .unwrap()
        .set_modified(modified)
        .unwrap();
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
    let exported_fallback = exported(&dir.path().join("fallback"), |_| {}).unwrap();
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

    // An export draws with the fonts of the app that asked for it, as the
    // viewer does; asked for fonts its process cannot have, it fails
    // rather than export another face.
    assert_ne!(
        exported(&dir.path().join("installed"), |_| {}).unwrap(),
        exported_fallback,
        "the export fell back although the app has the font"
    );
    assert_eq!(
        exported(&dir.path().join("unmatched"), |fonts| {
            fonts.0.push((
                dir.path().join("gone.ttf"),
                pm_text::file_version::FileVersion {
                    len: 1,
                    ..Default::default()
                },
            ));
        }),
        Err(jobs::FONTS_CHANGED.to_owned())
    );
    assert!(!dir.path().join("unmatched").exists());

    // The font is active for this process alone, and a range is rendered
    // now: the fill child draws with the app's fonts, so the frame on disk
    // is the picture the viewer draws.
    let (frames, scope) = async_channel::bounded(1);
    viewer
        .scopes(Some((pm_render::scopes::ScopeConfig::PANEL, frames)))
        .unwrap();
    let frame = |n: i64| n * TICKS_PER_SECOND / 30;
    let (report, drawn) = settled(&viewer, &scope, frame(31));
    assert_eq!(report.scale, 2., "not rendered yet: drawn and refined");
    let fill = |from: f64, to: f64, activity: jobs::Activity| {
        eventually("the fill starts", || {
            disk.start_fill(from, to, exe, activity.clone(), |_| {})
                .is_ok()
        });
    };
    let filled = || {
        eventually("the fill ends", || {
            !disk.bars().running && disk.bars().done > 0
        });
        assert_eq!(disk.bars().error, None);
    };
    fill(1., 1.1, jobs::Activity::default());
    filled();
    assert!(rendered(), "{:?}", disk.bars());
    // Frame 30 starts the reader on the range; frame 31 is then read.
    settled(&viewer, &scope, frame(60));
    settled(&viewer, &scope, frame(30));
    let (report, cached) = settled(&viewer, &scope, frame(31));
    assert_eq!(report.scale, 1., "in the range: the frame on disk");
    assert!(
        cached == drawn,
        "the rendered frame is not the picture drawn with the activated font"
    );

    // A fill waiting for its turn when the font goes away draws, when its
    // turn comes, with the fonts its frames' keys were taken from: when the
    // font is back, those frames are that state's pictures.
    let (_, drawn) = settled(&viewer, &scope, frame(61));
    let waiting = jobs::Activity::default();
    waiting.set(true);
    fill(2., 2.1, waiting.clone());
    pm_text::system_fonts::unregister_for_process(&font).unwrap();
    eventually("the viewer draws without the font", || {
        picture(&viewer) == fallback
    });
    waiting.set(false);
    filled();
    pm_text::system_fonts::register_for_process(&font).unwrap();
    eventually("the viewer draws with the font again", || {
        picture(&viewer) == installed
    });
    settled(&viewer, &scope, frame(30));
    settled(&viewer, &scope, frame(60));
    let (report, cached) = settled(&viewer, &scope, frame(61));
    assert_eq!(report.scale, 1., "rendered while the font was away");
    assert!(
        cached == drawn,
        "a frame rendered while the font was away is not its key's picture"
    );
    viewer.scopes(None).unwrap();

    // What holds a job to its fonts (`system_fonts::Fence`): asking it
    // lists nothing (a frame's check costs one look per font file read,
    // not one per installed font), and a font removed and installed again
    // in between does not pass, though the list is as it was.
    use pm_text::system_fonts;
    let asked = system_fonts::Inventory::current();
    let fence = system_fonts::Fence::new(&asked);
    let listed = system_fonts::listings();
    for _ in 0..100 {
        fence.hold().unwrap();
    }
    assert_eq!(
        system_fonts::listings(),
        listed,
        "holding a job to its fonts lists every installed font"
    );
    let changes = system_fonts::epoch();
    system_fonts::unregister_for_process(&font).unwrap();
    system_fonts::register_for_process(&font).unwrap();
    eventually("both changes are heard", || {
        system_fonts::epoch() >= changes + 2
    });
    assert_eq!(asked.verify(), Ok(()), "the list is as it was");
    assert!(
        fence.hold().is_err(),
        "a font removed and installed again while a job rendered goes unseen"
    );
    eventually("the viewer draws with the font again", || {
        picture(&viewer) == installed
    });

    // An export waiting for its turn when a font file it draws with is
    // rewritten in place: it fails rather than deliver frames drawn with
    // other fonts than the ones it was asked for, and leaves no file.
    let read_as = |font: &Path| {
        system_fonts::files_read()
            .into_iter()
            .find(|(path, _)| path == font)
            .map(|(_, version)| version)
    };
    let before = read_as(&font).expect("the app drew with the font");
    let waiting = jobs::Activity::default();
    waiting.set(true);
    let to = dir.path().join("rewritten");
    let export =
        jobs::ExportHandle::start_scheduled(exe, export_request(&to), waiting.clone()).unwrap();
    eventually("the export waits for its turn", || {
        export.gpu_requests() > 0
    });
    rewritten_in_place(&font);
    waiting.set(false);
    let status = export.wait().unwrap();
    assert_eq!(
        (
            status.state,
            status.error.map(|e| e.message).unwrap_or_default()
        ),
        (
            pm_protocol::priority::JobState::Failed,
            jobs::FONTS_CHANGED.to_owned()
        ),
        "an export delivered frames drawn after its font file was rewritten"
    );
    assert!(
        !to.exists() || std::fs::read_dir(&to).unwrap().next().is_none(),
        "the failed export left a file"
    );
    // And the app itself reads the rewritten file again (it would go on
    // drawing the bytes it had: nothing announces a rewrite in place).
    eventually("the app reads the rewritten font again", || {
        picture(&viewer);
        read_as(&font).is_some_and(|version| version != before)
    });

    pm_text::system_fonts::unregister_for_process(&font).unwrap();
    eventually("the first state's frames are this state's again", rendered);
    assert_eq!(
        picture(&viewer),
        fallback,
        "the font is gone: the text falls back as before"
    );
    viewer.retire();
}
