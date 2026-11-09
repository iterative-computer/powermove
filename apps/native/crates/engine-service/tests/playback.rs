//! The viewer's playback path: what a frame asks of the renderer, and what the
//! worker does when a layer, a decode or a deadline fails.
use pm_engine_service::*;
use pm_protocol::{
    clock::{SeekMode, TransportCommand as C},
    common::{PlaybackRate, TICKS_PER_SECOND, Ticks},
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../desktop/e2e/fixtures")
        .join(name)
}
fn snapshot(project: &str) -> Snapshot {
    let value = pm_model::json::parse(project).unwrap();
    #[cfg(feature = "kernel")]
    {
        pm_kernel::document::DocumentStore::new_untitled(value).snapshot()
    }
    #[cfg(not(feature = "kernel"))]
    {
        Snapshot {
            doc_id: 7,
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(value)),
                1,
            )),
        }
    }
}
/// A layer's full transform, as every saved layer carries it.
fn transform(x: f64, y: f64) -> String {
    let p = |k: &str, v: f64| format!(r#""{k}":{{"v":{v},"kf":[],"expr":null}}"#);
    [
        p("position.x", x),
        p("position.y", y),
        p("anchor.x", 0.),
        p("anchor.y", 0.),
        p("scale.x", 100.),
        p("scale.y", 100.),
        p("rotation", 0.),
        p("opacity", 100.),
    ]
    .join(",")
}
fn shape(id: &str, color: &str, x: f64) -> String {
    format!(
        r##"{{"id":"{id}","type":"shape","name":"{id}","from":0,"dur":3,"on":true,"solo":false,"mblur":false,
        "p":{{{}}},"fx":[],"masks":[],
        "d":{{"shape":"rect","w":200,"h":200,"radius":0,"color":"{color}"}}}}"##,
        transform(x, 180.)
    )
}
/// Two layers over a video clip: the middle one's media can be offline.
fn project(asset: &str) -> Snapshot {
    snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#101820",
        "layers":[{},
        {{"id":"v","type":"video","name":"clip","from":0,"dur":3,"on":true,"solo":false,"mblur":false,
          "p":{{{}}},"fx":[],"masks":[],"d":{{"asset":"{asset}","trim":0,"speed":1,"w":640,"h":360}}}}],
        "comps":{{}},"assets":{{"{asset}":{{"id":"{asset}","type":"video","name":"clip","w":640,"h":360,"dur":3}}}},
        "markers":[],"work":[0,3],"params":{{}}}}"##,
        shape("top", "#FF0000", 120.),
        transform(320., 180.)
    ))
}
fn catalog(asset: &str, file: Option<&str>) -> MediaCatalog {
    let mut media = MediaCatalog::default();
    if let Some(file) = file {
        media.originals.insert(asset.into(), fixture(file));
    }
    media
}
fn config(s: Snapshot, media: MediaCatalog, output: Output) -> ViewerConfig {
    ViewerConfig {
        composition: s.open_comp().unwrap(),
        snapshot: s,
        media,
        output,
        layer: None,
        rate: pm_time::Rate::FPS_30,
        end: Some(3 * TICKS_PER_SECOND),
        audio: false,
    }
}
fn output(width: u32, height: u32) -> Output {
    Output {
        width,
        height,
        display: 0,
        headroom: 1.,
        crop: None,
    }
}
fn service(label: &str) -> EngineService {
    EngineService::start(pm_gpu::Gpu::new(label).unwrap()).unwrap()
}
fn wait(v: &Viewer, why: &str, done: impl Fn(&ViewerStatus) -> bool) -> ViewerStatus {
    let start = Instant::now();
    loop {
        let status = v.status();
        if done(&status) {
            return status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{why}: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn scene(s: &Snapshot, media: &MediaCatalog) -> Scene {
    Scene {
        doc: s.document.clone(),
        comp: s.open_comp().unwrap(),
        media: Arc::new(media.export_paths(false)),
        content: 1,
    }
}

/// The pixel at output `(x, y)` of a 320 × 180 frame.
fn at(pixels: &[[f64; 3]], x: usize, y: usize) -> [f64; 3] {
    pixels[y * 320 + x]
}
/// The Media Offline card's ground (neutral) or hatch (red), never the
/// composition's blue-grey background (#101820).
fn card(p: [f64; 3]) -> bool {
    p[0] >= p[2] && p[0] < 0.5
}
/// The healthy red layer above the clip (comp x 120, so output x 60).
fn red(p: [f64; 3]) -> bool {
    p[0] > 0.4 && p[1] < 0.2
}

#[test]
fn an_offline_clip_draws_its_card_and_leaves_the_rest_of_the_frame_on_screen() {
    let s = project("gone");
    let service = service("isolation");
    let v = service
        .viewer(config(s, catalog("gone", None), output(320, 180)))
        .unwrap();
    v.capture_next().unwrap();
    let status = wait(&v, "a frame with an offline clip", |s| {
        s.frame.as_ref().is_some_and(|f| f.pixels.is_some())
    });
    assert_eq!(status.error, None, "an offline clip never fails the frame");
    // The card names the file, so the clip is not a "Not drawn" layer.
    assert!(
        !status.layer_failures.iter().any(|f| f.layer == "v"),
        "{:?}",
        status.layer_failures
    );
    let pixels = status.frame.unwrap().pixels.unwrap();
    assert!(card(at(&pixels, 310, 170)), "the card fills the clip's box");
    assert!(card(at(&pixels, 250, 10)), "the card fills the clip's box");
    assert!(
        red(at(&pixels, 60, 90)),
        "the healthy layer above stays on screen"
    );
}

#[test]
fn a_cropped_frame_is_the_same_grid_as_the_whole_canvas() {
    let s = project("clip");
    let media = catalog("clip", Some("h264-aac.mp4"));
    let scene = scene(&s, &media);
    let mut host = Headless::new().unwrap();
    let time = Ticks::new(TICKS_PER_SECOND / 3).unwrap();
    let whole = host
        .frame_spec_settled(
            &scene,
            FrameSpec::exact(time, output(640, 360)),
            &Default::default(),
        )
        .unwrap();
    let (x, y, w, h) = (100_u32, 60_u32, 300_u32, 200_u32);
    let crop = host
        .frame_spec_settled(
            &scene,
            FrameSpec::exact(
                time,
                Output {
                    crop: Some(Crop {
                        x,
                        y,
                        canvas_width: 640,
                        canvas_height: 360,
                    }),
                    ..output(w, h)
                },
            ),
            &Default::default(),
        )
        .unwrap();
    assert_eq!(crop.len(), (w * h) as usize);
    let mut worst = 0_f64;
    for row in 0..h as usize {
        for column in 0..w as usize {
            let a = crop[row * w as usize + column];
            let b = whole[(row + y as usize) * 640 + column + x as usize];
            for c in 0..3 {
                worst = worst.max((a[c] - b[c]).abs());
            }
        }
    }
    assert!(worst < 1e-3, "cropped pixels moved: {worst}");
}

#[test]
fn half_resolution_scales_once_to_the_drawable_and_keeps_overlays_on_the_canvas() {
    let s = project("clip");
    let media = catalog("clip", Some("h264-aac.mp4"));
    let scene = scene(&s, &media);
    let mut host = Headless::new().unwrap();
    let time = Ticks::new(TICKS_PER_SECOND / 3).unwrap();
    let full = host
        .frame_spec_settled(
            &scene,
            FrameSpec::exact(time, output(320, 180)),
            &Default::default(),
        )
        .unwrap();
    let half = host
        .frame_spec_settled(
            &scene,
            FrameSpec {
                scale: 0.5,
                ..FrameSpec::exact(time, output(320, 180))
            },
            &Default::default(),
        )
        .unwrap();
    assert_eq!(half.len(), full.len());
    let mean =
        |v: &[[f64; 3]]| v.iter().map(|p| p[0] + p[1] + p[2]).sum::<f64>() / (3 * v.len()) as f64;
    assert!((mean(&full) - mean(&half)).abs() < 0.01);
    let line = OverlayGeometry::new(vec![OverlayLine {
        ends: [0.5, 0., 0.5, 1.],
        color: [0., 1., 0., 1.],
    }])
    .unwrap();
    let lined = host
        .frame_spec_settled(
            &scene,
            FrameSpec {
                scale: 0.5,
                ..FrameSpec::exact(time, output(320, 180))
            },
            &line,
        )
        .unwrap();
    let centre = lined[90 * 320 + 160];
    assert!(centre[1] > 0.5 && centre[0] < 0.5, "{centre:?}");
}

#[test]
fn playback_frames_take_the_viewer_path_and_a_stop_lands_the_exact_frame() {
    let s = project("clip");
    let service = service("playback-path");
    let v = service
        .viewer(config(
            s,
            catalog("clip", Some("h264-aac.mp4")),
            output(320, 180),
        ))
        .unwrap();
    wait(&v, "first frame", |s| s.frame.is_some());
    v.command(C::Play {
        rate: PlaybackRate::NORMAL,
    })
    .unwrap();
    let first = wait(&v, "a playing frame", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.moment == Moment::Playing)
    });
    assert_eq!(first.error, None);
    v.command(C::Pause).unwrap();
    let stopped = wait(&v, "the exact stopped frame", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.moment == Moment::Paused && f.scale == 1. && !f.stand_ins)
    });
    assert_eq!(stopped.error, None);
}

#[test]
fn a_drag_shows_stand_ins_and_the_exact_frame_lands_without_another_input() {
    let s = project("clip");
    let service = service("scrub-landing");
    let v = service
        .viewer(config(
            s,
            catalog("clip", Some("h264-aac.mp4")),
            output(320, 180),
        ))
        .unwrap();
    wait(&v, "first frame", |s| s.frame.is_some());
    for frame in [40, 60, 80] {
        v.scrub(Ticks::new(TICKS_PER_SECOND * frame / 30).unwrap())
            .unwrap();
    }
    let at = Ticks::new(TICKS_PER_SECOND * 80 / 30).unwrap();
    let landed = wait(&v, "the exact scrub frame", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.time == at && !f.stand_ins)
    });
    assert_eq!(landed.error, None);
}

#[test]
fn a_fixed_quality_renders_every_frame_at_that_resolution() {
    let s = project("clip");
    let service = service("fixed-quality");
    let v = service
        .viewer(config(
            s,
            catalog("clip", Some("h264-aac.mp4")),
            output(320, 180),
        ))
        .unwrap();
    wait(&v, "first frame", |s| s.frame.is_some());
    v.quality(Quality::Quarter).unwrap();
    let quarter = wait(&v, "a quarter frame", |s| {
        s.frame.as_ref().is_some_and(|f| f.scale == 0.25)
    });
    assert_eq!(quarter.error, None);
    v.command(C::Seek {
        time: Ticks::new(TICKS_PER_SECOND).unwrap(),
        mode: SeekMode::Exact,
    })
    .unwrap();
    v.quality(Quality::Auto).unwrap();
    wait(&v, "Auto is full when stopped", |s| {
        s.frame.as_ref().is_some_and(|f| f.scale == 1.)
    });
}

/// View › Media: a clip whose original is missing plays its proxy in
/// Automatic, is offline in Originals, and a proxy is never counted as an
/// original.
#[test]
fn media_modes_choose_proxies_and_originals_per_clip() {
    let s = project("clip");
    let mut media = MediaCatalog::default();
    media.proxies.insert("clip".into(), fixture("h264-aac.mp4"));
    let service = service("media-modes");
    let v = service.viewer(config(s, media, output(320, 180))).unwrap();
    let settled = |v: &Viewer, why: &str| {
        let stamp = v.stamp().unwrap();
        wait(v, why, |s| {
            s.frame.as_ref().is_some_and(|f| f.stamp == stamp)
        })
    };
    v.capture_next().unwrap();
    let status = settled(&v, "Automatic");
    assert!(
        status.layer_failures.is_empty(),
        "{:?}",
        status.layer_failures
    );
    assert_eq!(status.proxied, 1);
    let proxy_frame = status.frame.clone().unwrap().pixels;
    v.capture_next().unwrap();
    v.media_mode(MediaMode::Originals).unwrap();
    let status = settled(&v, "Originals");
    // Offline in Originals: the clip draws its Media Offline card (not a
    // failed layer), and the healthy layer above stays.
    assert!(
        !status.layer_failures.iter().any(|f| f.layer == "v"),
        "{:?}",
        status.layer_failures
    );
    assert_eq!(status.proxied, 0);
    let card_frame = status.frame.clone().unwrap().pixels.expect("captured");
    assert!(card(at(&card_frame, 310, 170)));
    assert!(red(at(&card_frame, 60, 90)));
    assert_ne!(
        Some(&card_frame),
        proxy_frame.as_ref(),
        "the card replaces the proxy"
    );
    v.capture_next().unwrap();
    v.media_mode(MediaMode::ProxyOnly).unwrap();
    let status = settled(&v, "Proxy Only");
    assert!(
        status.layer_failures.is_empty(),
        "{:?}",
        status.layer_failures
    );
    assert_eq!(status.proxied, 1);
    // Back to a readable file: the card goes away.
    let back = status.frame.clone().unwrap().pixels.expect("captured");
    assert_ne!(back, card_frame, "the proxy replaces the card");

    let mut both = MediaCatalog::default();
    both.originals.insert("a".into(), "/original.mov".into());
    both.proxies.insert("a".into(), "/proxy.mov".into());
    both.originals.insert("b".into(), "/b.mov".into());
    assert_eq!(
        both.viewer_paths(MediaMode::Automatic)["a"].to_str(),
        Some("/original.mov")
    );
    assert_eq!(
        both.viewer_paths(MediaMode::ProxyPreferred)["a"].to_str(),
        Some("/proxy.mov")
    );
    assert_eq!(
        both.viewer_paths(MediaMode::ProxyPreferred)["b"].to_str(),
        Some("/b.mov")
    );
    assert!(!both.viewer_paths(MediaMode::ProxyOnly).contains_key("b"));
    assert_eq!(both.proxied(MediaMode::Automatic), 0);
    assert_eq!(both.proxied(MediaMode::ProxyPreferred), 1);
    // Optimized media plays in Automatic in place of its original, never
    // in Originals; Always Play Original wins in every mode, and a clip
    // whose original is gone is then offline rather than proxied.
    both.optimized.insert("b".into(), "/b-optimized.mov".into());
    assert_eq!(
        both.viewer_paths(MediaMode::Automatic)["b"].to_str(),
        Some("/b-optimized.mov")
    );
    assert_eq!(
        both.viewer_paths(MediaMode::Originals)["b"].to_str(),
        Some("/b.mov")
    );
    both.play_original.insert("a".into());
    both.play_original.insert("b".into());
    for mode in [
        MediaMode::Automatic,
        MediaMode::Originals,
        MediaMode::ProxyPreferred,
        MediaMode::ProxyOnly,
    ] {
        assert_eq!(both.viewer_paths(mode)["a"].to_str(), Some("/original.mov"));
        assert_eq!(both.viewer_paths(mode)["b"].to_str(), Some("/b.mov"));
    }
    let mut gone = both.clone();
    gone.originals.remove("a");
    assert!(
        !gone
            .viewer_paths(MediaMode::ProxyPreferred)
            .contains_key("a")
    );
    both.play_original.clear();
    both.optimized.clear();
    assert_eq!(
        both.export_paths(false).paths["a"].to_str(),
        Some("/original.mov")
    );
}

/// An extension's effect, given by the resource snapshot the compat runtime
/// publishes, renders natively; without the snapshot the layer is reported.
#[test]
fn extension_effects_from_the_resource_snapshot_render_natively() {
    let tinted = snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
        "layers":[{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,3],"params":{{}}}}"##,
        shape("top", "#FF0000", 320.).replace(
            r#""fx":[]"#,
            r#""fx":[{"id":"e1","type":"ext.invert","on":true,"p":{}}]"#
        )
    ));
    let service = service("extension-effects");
    let v = service
        .viewer(config(tinted, MediaCatalog::default(), output(320, 180)))
        .unwrap();
    let settled = |v: &Viewer, why: &str| {
        v.capture_next().unwrap();
        let stamp = v.stamp().unwrap();
        wait(v, why, |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == stamp && f.pixels.is_some())
        })
    };
    let missing = settled(&v, "without the extension");
    assert!(
        missing.layer_failures.iter().any(|f| f.layer == "top") || missing.error.is_some(),
        "an unknown effect is reported, never skipped: {missing:?}"
    );
    let mut invert = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    invert.id = "ext.invert".into();
    invert.origin = pm_protocol::definitions::DefinitionOrigin::Local;
    v.resources(Arc::new(pm_engine_service::resources::ResourceSnapshot {
        generation: 1,
        definitions: vec![invert],
        ..Default::default()
    }))
    .unwrap();
    let drawn = settled(&v, "with the extension");
    assert_eq!(drawn.error, None);
    assert!(
        drawn.layer_failures.is_empty(),
        "{:?}",
        drawn.layer_failures
    );
    let centre = drawn.frame.unwrap().pixels.unwrap()[90 * 320 + 160];
    // Viewer pixels are linear Display P3: sRGB cyan is (0.18, 0.97, 0.98).
    assert!(
        centre[0] < 0.25 && centre[1] > 0.9 && centre[2] > 0.9,
        "inverted red is cyan: {centre:?}"
    );
}
