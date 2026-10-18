//! Retiming on the render side (docs/engine/motion.md): "Keep frame rate
//! when nested" quantizes a child composition to its own frames, and an
//! Interpret Footage frame-rate override shows frame n at n / rate. The
//! frame selection of retimed video against today's is in `motion.rs`
//! (`retime/speed`).

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::ticks;

/// The largest difference between two frames.
fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// A 60 fps root with a 24 fps child whose solid slides 2 pixels per
/// child frame, so every child frame differs.
fn nested(keep: bool) -> pm_model::Document {
    let mut slider = layer(
        "s",
        "solid",
        json!({ "color": "#FFFFFF", "w": 10.0, "h": 10.0 }),
        0.0,
        5.0,
        4.0,
    );
    set(
        &mut slider,
        "position.x",
        json!({ "v": 0, "expr": null, "kf": [
            { "t": 0.0, "v": 0.0, "i": "k0", "eo": [0.333, 0.333], "ei": [0.667, 0.667], "hold": false },
            { "t": 2.0, "v": 96.0, "i": "k1", "eo": [0.333, 0.333], "ei": [0.667, 0.667], "hold": false }
        ] }),
    );
    let mut pc = layer(
        "pc",
        "precomp",
        json!({ "comp": "K", "w": 120, "h": 20 }),
        0.0,
        0.0,
        4.0,
    );
    if keep {
        pc["d"]["keepFrameRate"] = json!(true);
    }
    let mut doc = project(120, 20, 4.0, no_fill(), vec![pc]);
    doc["fps"] = json!(60);
    composition(&mut doc, "K", 120, 20, 4.0, vec![slider]);
    doc["comps"]["K"]["fps"] = json!(24);
    load(&doc)
}

fn render(renderer: &mut Renderer, doc: &pm_model::Document, seconds: f64) -> Vec<f32> {
    let request = RenderRequest::sized(Size::new(120, 20), Precision::Float32);
    let f = renderer
        .render(
            doc,
            "Ptest-root",
            ticks(seconds),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    renderer.read_rgba(&f).unwrap()
}

#[test]
fn keep_frame_rate_holds_each_child_frame() {
    let mut renderer = Renderer::headless().unwrap();
    let keep = nested(true);
    let free = nested(false);
    // Root frames 0..10 at 60 fps against child frames at 24 fps.
    for n in 0..10 {
        let t = f64::from(n) / 60.0;
        let child_frame = (t * 24.0 + 1e-9).floor();
        let held = render(&mut renderer, &keep, t);
        let d = max_diff(&held, &render(&mut renderer, &free, child_frame / 24.0));
        assert_eq!(d, 0.0, "root frame {n} shows child frame {child_frame}");
    }
    // Without the switch the child follows the root time.
    let t = 9.0 / 60.0;
    let d = max_diff(
        &render(&mut renderer, &free, t),
        &render(&mut renderer, &keep, t),
    );
    assert!(d > 0.5, "{d}");
}

/// The frame-counter video of the newest motion capture, if any.
fn frame_video() -> Option<std::path::PathBuf> {
    let capture = support::newest_capture("motion-")?;
    let path = capture.join("generated/media/corpus-frames-30.mp4");
    path.is_file().then_some(path)
}

fn video_doc(rate: Option<serde_json::Value>) -> pm_model::Document {
    let l = layer(
        "v",
        "video",
        json!({ "asset": "a", "fit": "fill", "w": 320, "h": 180 }),
        160.0,
        90.0,
        10.0,
    );
    let mut doc = project(320, 180, 10.0, no_fill(), vec![l]);
    asset(&mut doc, "a", "video", "mp4", 320, 180, 4.0);
    if let Some(r) = rate {
        doc["assets"]["a"]["interpret"] = json!({ "frameRate": r });
    }
    load(&doc)
}

#[test]
fn a_frame_rate_override_shows_frame_n_at_n_over_the_rate() {
    let Some(video) = frame_video() else {
        eprintln!("no motion capture with corpus-frames-30.mp4; skipping");
        return;
    };
    let mut renderer = Renderer::headless().unwrap();
    let mut media = MapResolver::default();
    media.paths.insert("a".into(), video);
    let mut frame = |doc: &pm_model::Document, t: f64| {
        let request = RenderRequest::sized(Size::new(320, 180), Precision::Float16);
        let f = renderer
            .render(doc, "Ptest-root", ticks(t), &request, &media)
            .unwrap();
        assert!(f.failures.is_empty(), "{:?}", f.failures);
        renderer.read_rgba(&f).unwrap()
    };
    let native = video_doc(None);
    let slow = video_doc(Some(json!([15, 1])));
    let pal = video_doc(Some(json!(25)));
    // At 15 fps frame 15 shows at 1 s; natively at 0.5 s.
    let same = |a: Vec<f32>, b: Vec<f32>, what: &str| {
        let d = max_diff(&a, &b);
        assert_eq!(d, 0.0, "{what}: max difference {d}");
    };
    same(frame(&slow, 1.0), frame(&native, 0.5), "15 fps at 1 s");
    same(
        frame(&slow, 1.0 + 0.8 / 15.0),
        frame(&native, 0.5),
        "15 fps within frame 15",
    );
    let d = max_diff(&frame(&slow, 1.0), &frame(&native, 1.0));
    assert!(d > 0.1, "the override changes the frame: {d}");
    // At 25 fps frame 37 shows at 1.48 s; natively at 37 / 30 s.
    same(
        frame(&pal, 1.48),
        frame(&native, 37.0 / 30.0),
        "25 fps frame 37",
    );
    // Past the last frame (120 frames at 15 fps end at 8 s): the last one
    // (natively today's end clamp, dur − 0.04 s, never reaches it).
    same(
        frame(&slow, 9.0),
        frame(&slow, 119.0 / 15.0),
        "past the end",
    );
    let d = max_diff(&frame(&slow, 9.0), &frame(&slow, 118.0 / 15.0));
    assert!(d > 0.1, "the last frame, not the one before: {d}");
}
