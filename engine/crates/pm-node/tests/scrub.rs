//! A viewer frame after a scrub seek (docs/engine/scrub.md): the composer
//! reports video layers drawn with stand-ins, and once the exact video
//! landed the same frame composes without them.
#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::sync::Arc;

use pm_gpu::bridge_out::display::WorkingTexture;
use pm_node::present::compose::Composer;
use pm_node::present::document::MediaMap;
use pm_protocol::color::ViewerColorSpace;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerState};
use serde_json::json;

fn ch(v: serde_json::Value) -> serde_json::Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn doc() -> pm_model::Document {
    let layer = json!({
        "id": "v", "type": "video", "name": "v", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
            "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": [], "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 },
        "locked_intent": {}
    });
    let project = json!({
        "id": "P", "name": "p", "w": 160, "h": 90, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "P-root", "compName": "p", "markers": [], "work": [0, 2],
        "params": {}, "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer],
        "assets": { "clip": {
            "id": "clip", "name": "clip", "kind": "video", "format": "mp4", "w": 320, "h": 180,
            "dur": 2, "fps": 30, "persisted": true
        } },
        "comps": {}
    });
    pm_model::Document::new(
        Some(pm_model::json::parse(&project.to_string()).unwrap()),
        1,
    )
}

fn state() -> ViewerState {
    ViewerState {
        composition: "P-root".into(),
        size: Size::new(320, 180),
        device_pixel_ratio: 2.0,
        zoom: 1.0,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Full,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::Srgb,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(1).unwrap(),
        vsync: None,
    }
}

#[test]
fn a_scrub_frame_reports_stand_ins_until_the_exact_video_lands() {
    let gpu = pm_gpu::Gpu::new("scrub test").unwrap();
    let working = WorkingTexture::new(&gpu.device, Size::new(320, 180));
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures/h264-aac.mp4");
    let media = MediaMap(Arc::new(BTreeMap::from([(
        "clip".to_owned(),
        Some(fixtures),
    )])));
    let document = doc();
    let s = state();
    let at = |seconds: f64| Ticks::new((seconds * 705_600_000.0) as i64).unwrap();
    let mut compose = |composer: &mut Composer, t: f64| {
        let c = composer
            .compose(&document, "P-root", &media, at(t), &s, &working)
            .unwrap();
        assert!(c.failures.is_empty(), "{:?}", c.failures);
        c.stand_ins.len()
    };
    // A paused frame: exact.
    assert_eq!(compose(&mut composer, 0.1), 0);
    // Exact seeks never stand in, even far away.
    assert_eq!(compose(&mut composer, 1.9), 0);
    // A scrub into a part of the clip not decoded yet stands in at once.
    composer.renderer().set_scrub(true);
    let landed = composer.renderer().media_landed();
    assert_eq!(compose(&mut composer, 1.2), 1);
    let t = std::time::Instant::now();
    while composer.renderer().media_landed() == landed {
        assert!(t.elapsed().as_secs() < 20, "the exact frame never landed");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(compose(&mut composer, 1.2), 0);
}
