//! A frame that meets a still the prefetch worker is reading never waits
//! for it unboundedly on the render thread (`media/stills.rs`).

#[path = "../../tests/support/doc.rs"]
mod doc;

use super::*;
use crate::assets::MapResolver;
use pm_protocol::common::Size;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn write_still(path: &std::path::Path, shade: u8) {
    let (w, h) = (1024u32, 768u32);
    let rgba: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).flat_map(move |x| [(x / 4) as u8, (y / 3) as u8, shade, 255]))
        .collect();
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
}

struct Still {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    document: pm_model::Document,
    assets: MapResolver,
    request: RenderRequest,
}

fn still_project() -> Still {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("still.png");
    write_still(&path, 40);
    let (w, h) = (256, 192);
    let mut project = doc::project(
        w,
        h,
        2.0,
        doc::no_fill(),
        vec![doc::layer(
            "still",
            "image",
            json!({"asset":"pic","w":w,"h":h}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    doc::asset(&mut project, "pic", "image", "png", 1024, 768, 2.0);
    let document = doc::load(&project);
    let mut assets = MapResolver::default();
    assets.paths.insert("pic".into(), path.clone());
    Still {
        _dir: dir,
        path,
        document,
        assets,
        request: RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float16),
    }
}

/// Holds the prefetch workers inside their next decode (a read that does
/// not return) until the sender is dropped.
fn stall(renderer: &Renderer) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let gate = renderer.media.still_decode_gate();
    gate(true);
    let (go, release) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        // Released by the test, or after a long stall if the test failed
        // with a frame still waiting.
        let _ = release.recv_timeout(Duration::from_secs(3));
        gate(false);
    });
    (go, holder)
}

fn prepare(renderer: &mut Renderer, s: &Still) {
    let time = Ticks::new(0).unwrap();
    let prepared = renderer
        .prepare_media(&s.document, "Ptest-root", time, &s.request, &s.assets)
        .unwrap();
    assert!(prepared.failures.is_empty(), "{:?}", prepared.failures);
    let started = Instant::now();
    while !renderer.media.still_decoding() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the worker starts"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A frame drawn into its own target: what the renderer reported, and the
/// target as a [`Frame`] to read back.
fn render(
    renderer: &mut Renderer,
    s: &Still,
) -> (Result<(Rendered, Frame), RenderError>, Duration, u64) {
    let size = s.request.target_size();
    let target = Target::new(
        &renderer.device,
        "still wait test",
        size.width,
        size.height,
        working_format(Precision::Float16),
        pm_gpu::target::working_usage(),
    );
    let before = pm_perf::snapshot();
    let started = Instant::now();
    let rendered = renderer.render_into(
        &s.document,
        "Ptest-root",
        Ticks::new(0).unwrap(),
        &s.request,
        &s.assets,
        &target,
        None,
    );
    let waited = started.elapsed();
    let decodes = pm_perf::snapshot()
        .since(&before)
        .get(pm_perf::Counter::StillDecodes);
    let frame = rendered.map(|r| {
        let frame = Frame {
            scene3d_meta: Vec::new(),
            color: target,
            depth_id: None,
            legend: Vec::new(),
            failures: r.failures.clone(),
            svg_fallbacks: Vec::new(),
            seconds: r.seconds,
            footage: r.footage.clone(),
        };
        (r, frame)
    });
    (frame, waited, decodes)
}

fn wait_landed(renderer: &Renderer, landed: u64) {
    let started = Instant::now();
    while renderer.media_landed() == landed {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the decode lands"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A playing frame gives a still being read at most its budget, is held
/// (nothing of it drawn without the layer), does not decode the still
/// itself, and a later frame takes the decode once it lands. An exact
/// frame waits longer but stops when it is superseded.
#[test]
fn a_frame_never_waits_unboundedly_for_a_still_being_read() {
    let s = still_project();
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_preview_media(true, true);
    let (go, holder) = stall(&renderer);
    prepare(&mut renderer, &s);
    let landed = renderer.media_landed();

    // A paused (exact) frame, superseded 30 ms in.
    let interrupt = Arc::new(AtomicU64::new(0));
    renderer.set_interrupt(Some((interrupt.clone(), 0)));
    let seek = {
        let interrupt = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            interrupt.fetch_add(1, Ordering::AcqRel);
        })
    };
    let (frame, waited, decodes) = render(&mut renderer, &s);
    seek.join().unwrap();
    renderer.set_interrupt(None);
    assert!(
        waited < Duration::from_millis(400),
        "a superseded frame waited {waited:?} for a read that does not return"
    );
    assert!(
        matches!(frame, Err(RenderError::MediaPending)),
        "held, not drawn without the still: {:?}",
        frame.map(|(r, _)| r.failures)
    );
    assert_eq!(decodes, 0, "the frame read the still itself");

    // Playing frames: each holds within its budget, none decodes itself.
    for _ in 0..3 {
        renderer.set_playback(Some(Instant::now() + Duration::from_millis(20)));
        let (frame, waited, decodes) = render(&mut renderer, &s);
        renderer.set_playback(None);
        assert!(
            waited < Duration::from_millis(400),
            "a playing frame waited {waited:?} for a read that does not return"
        );
        assert!(matches!(frame, Err(RenderError::MediaPending)));
        assert_eq!(decodes, 0);
    }

    // The read returns: the next frame draws the worker's decode.
    drop(go);
    holder.join().unwrap();
    wait_landed(&renderer, landed);
    let (frame, _, decodes) = render(&mut renderer, &s);
    let (rendered, _) = frame.expect("drawn once the still landed");
    assert!(rendered.failures.is_empty(), "{:?}", rendered.failures);
    assert!(rendered.stand_ins.is_empty());
    assert_eq!(decodes, 0, "the frame decoded the still a second time");
}

/// While a changed still is being read, a playing frame shows the picture
/// it has of that still and says it stood in; the exact picture follows.
#[test]
fn a_playing_frame_shows_the_earlier_picture_of_a_still_being_read() {
    let s = still_project();
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_preview_media(true, true);
    let (frame, _, _) = render(&mut renderer, &s);
    let first = renderer.read_rgba(&frame.unwrap().1).unwrap();
    // The file changes (another stamp) while the worker is held.
    std::thread::sleep(Duration::from_millis(20));
    write_still(&s.path, 200);
    let (go, holder) = stall(&renderer);
    prepare(&mut renderer, &s);
    let landed = renderer.media_landed();
    renderer.set_playback(Some(Instant::now() + Duration::from_millis(20)));
    let (frame, waited, decodes) = render(&mut renderer, &s);
    renderer.set_playback(None);
    assert!(waited < Duration::from_millis(400), "waited {waited:?}");
    let (rendered, frame) = frame.expect("drawn from the earlier picture");
    assert_eq!(rendered.stand_ins.len(), 1, "a stand-in is reported");
    assert_eq!(decodes, 0);
    assert!(renderer.read_rgba(&frame).unwrap() == first);
    drop(go);
    holder.join().unwrap();
    wait_landed(&renderer, landed);
    let (frame, _, _) = render(&mut renderer, &s);
    let (rendered, frame) = frame.unwrap();
    assert!(rendered.stand_ins.is_empty());
    assert!(
        renderer.read_rgba(&frame).unwrap() != first,
        "the changed still is drawn once read"
    );
}
