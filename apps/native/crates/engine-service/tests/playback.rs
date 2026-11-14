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
        .join("../../../../engine/fixtures")
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
    pm_perf::count_globally(true);
    let counted = pm_perf::snapshot_global();
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
    // The card is painted on the GPU: the CPU fallback (seconds per frame in
    // a debug build) is never taken for it.
    let drawn = pm_perf::snapshot_global().since(&counted);
    assert!(
        drawn.get(pm_perf::Counter::VectorCacheMisses) > 0,
        "the card was resolved"
    );
    assert_eq!(drawn.get(pm_perf::Counter::SvgFallbackFeature), 0);
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
            // Full: the drawable's size, then the composition's (refined).
            .is_some_and(|f| f.moment == Moment::Paused && f.scale >= 1. && !f.stand_ins)
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
    // Full: the drawable's size, then the composition's (refined).
    wait(&v, "Auto is full when stopped", |s| {
        s.frame.as_ref().is_some_and(|f| f.scale >= 1.)
    });
}

/// View › Media: a clip whose original is missing plays its proxy in
/// Automatic, is offline in Originals, and a proxy is never counted as an
/// original. The viewer's picture follows the mode (read here through the
/// scopes, which analyze the picture on screen); a capture is a delivery
/// picture and reads originals whatever the mode, so the clip is offline
/// in it.
#[test]
fn media_modes_choose_proxies_and_originals_per_clip() {
    let s = project("clip");
    let mut media = MediaCatalog::default();
    media.proxies.insert("clip".into(), fixture("h264-aac.mp4"));
    let service = service("media-modes");
    let v = service.viewer(config(s, media, output(320, 180))).unwrap();
    let (frames, scope) = async_channel::bounded(1);
    v.scopes(Some((pm_render::scopes::ScopeConfig::PANEL, frames)))
        .unwrap();
    let settled = |v: &Viewer, why: &str| {
        let stamp = v.stamp().unwrap();
        wait(v, why, |s| {
            s.frame.as_ref().is_some_and(|f| f.stamp == stamp)
        })
    };
    // The next reading of a paused picture at the composition's size (the
    // refinement the viewer settles on) that `accept`s.
    let reading = |why: &str, accept: &dyn Fn(&pm_render::scopes::ScopeFrame) -> bool| {
        let start = Instant::now();
        loop {
            if let Ok(frame) = scope.try_recv() {
                let frame = frame.unwrap();
                if frame.pixels == 640 * 360 && accept(&frame) {
                    return frame;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(30), "{why}");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    let status = settled(&v, "Automatic");
    assert!(
        status.layer_failures.is_empty(),
        "{:?}",
        status.layer_failures
    );
    assert_eq!(status.proxied, 1);
    let proxy_picture = reading("the proxy's picture", &|_| true);
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
    let card_picture = reading("the card replaces the proxy", &|f| *f != proxy_picture);
    v.capture_next().unwrap();
    let captured = wait(&v, "a capture in Originals", |s| {
        s.frame.as_ref().is_some_and(|f| f.pixels.is_some())
    });
    let card_frame = captured.frame.unwrap().pixels.expect("captured");
    assert!(card(at(&card_frame, 310, 170)));
    assert!(red(at(&card_frame, 60, 90)));
    v.media_mode(MediaMode::ProxyOnly).unwrap();
    let status = settled(&v, "Proxy Only");
    assert!(
        status.layer_failures.is_empty(),
        "{:?}",
        status.layer_failures
    );
    assert_eq!(status.proxied, 1);
    // Back to a readable file: the card goes away.
    reading("the proxy replaces the card", &|f| *f == proxy_picture);
    assert_ne!(card_picture, proxy_picture);
    // A capture reads originals: the clip has none, so it is offline in the
    // captured picture although the viewer plays its proxy.
    v.capture_next().unwrap();
    let stamp = v.stamp().unwrap();
    let captured = wait(&v, "a capture in Proxy Only", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.stamp == stamp && f.pixels.is_some())
    });
    let delivered = captured.frame.unwrap().pixels.expect("captured");
    assert!(card(at(&delivered, 310, 170)));
    assert!(red(at(&delivered, 60, 90)));
    v.scopes(None).unwrap();

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
/// publishes, renders natively; without the snapshot the layer draws without
/// it and the effect is reported as not installed.
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
        missing
            .missing_effects
            .iter()
            .any(|m| m.effect == "ext.invert" && m.layers == 1)
            && missing.layer_failures.is_empty()
            && missing.error.is_none(),
        "an uninstalled effect is bypassed and reported: {missing:?}"
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

/// While the window's extensions load (a pending snapshot), a frame whose
/// effect no definition provides yet is not drawn bypassed and the effect is
/// never called not installed: the viewer waits, then draws it with the effect
/// once its extension registers it. Only a settled snapshot without it bypasses.
#[test]
fn an_effect_still_loading_is_waited_for_never_bypassed() {
    let tinted = snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
        "layers":[{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,3],"params":{{}}}}"##,
        shape("top", "#FF0000", 320.).replace(
            r#""fx":[]"#,
            r#""fx":[{"id":"e1","type":"ext.invert","on":true,"p":{}}]"#
        )
    ));
    let service = service("effects-loading");
    let v = service
        .viewer(config(tinted, MediaCatalog::default(), output(320, 180)))
        .unwrap();
    let snapshot = |generation, definitions, pending| {
        Arc::new(pm_engine_service::resources::ResourceSnapshot {
            generation,
            definitions,
            pending,
            loading: if pending {
                vec![("ext".into(), "Acme Effects".into())]
            } else {
                vec![]
            },
            ..Default::default()
        })
    };
    v.resources(snapshot(1, vec![], true)).unwrap();
    v.capture_next().unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        let status = v.status();
        assert!(
            status.frame.is_none() && status.missing_effects.is_empty(),
            "nothing is drawn or called not installed while it loads: {status:?}"
        );
        assert!(
            status.notice().is_none_or(|n| !n.contains("not installed")),
            "{status:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Past a few seconds the viewer says which extension it waits for.
    let named = wait(&v, "the wait named", |s| s.error.is_some());
    assert_eq!(
        named.error.as_deref(),
        Some("Still waiting: Acme Effects is loading (ext.invert)")
    );
    assert!(named.frame.is_none() && named.missing_effects.is_empty());
    let mut invert = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    invert.id = "ext.invert".into();
    invert.origin = pm_protocol::definitions::DefinitionOrigin::Local;
    // Registered while others still load: drawn with it at once, and the
    // line about waiting for it goes with the wait, not when a frame lands.
    v.resources(snapshot(2, vec![invert], true)).unwrap();
    assert_eq!(v.status().error, None, "the wait line outlived the wait");
    let drawn = wait(&v, "the effect registered", |s| {
        s.frame.as_ref().is_some_and(|f| f.pixels.is_some())
    });
    assert!(
        drawn.missing_effects.is_empty() && drawn.error.is_none(),
        "{drawn:?}"
    );
    let centre = drawn.frame.unwrap().pixels.unwrap()[90 * 320 + 160];
    assert!(centre[0] < 0.25 && centre[1] > 0.9, "inverted: {centre:?}");
    // Loading over and the effect really absent: bypassed and named.
    v.resources(snapshot(3, vec![], false)).unwrap();
    v.capture_next().unwrap();
    let stamp = v.stamp().unwrap();
    let bypassed = wait(&v, "settled without it", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.stamp == stamp && f.pixels.is_some())
    });
    assert!(
        bypassed
            .missing_effects
            .iter()
            .any(|m| m.effect == "ext.invert"),
        "{bypassed:?}"
    );
}

/// A frame that waited long for an extension's effect is drawn the moment the
/// effect registers, not after the backoff its retries reached (up to a
/// second; it delayed the first picture of a project with extension effects).
#[test]
fn a_frame_waiting_for_an_effect_draws_the_moment_it_registers() {
    let tinted = snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
        "layers":[{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,3],"params":{{}}}}"##,
        shape("top", "#FF0000", 320.).replace(
            r#""fx":[]"#,
            r#""fx":[{"id":"e1","type":"ext.invert","on":true,"p":{}}]"#
        )
    ));
    let service = service("effect-lands");
    let v = service
        .viewer(config(tinted, MediaCatalog::default(), output(320, 180)))
        .unwrap();
    let snapshot = |generation, definitions| {
        Arc::new(pm_engine_service::resources::ResourceSnapshot {
            generation,
            definitions,
            pending: true,
            loading: vec![("ext".into(), "Acme Effects".into())],
            ..Default::default()
        })
    };
    v.resources(snapshot(1, vec![])).unwrap();
    // The wait line appears on a refusal some seconds in: its retries are a
    // second apart by now, and one has just been made.
    wait(&v, "the wait named", |s| s.error.is_some());
    let mut invert = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    invert.id = "ext.invert".into();
    invert.origin = pm_protocol::definitions::DefinitionOrigin::Local;
    let registered = Instant::now();
    v.resources(snapshot(2, vec![invert])).unwrap();
    wait(&v, "the effect registered", |s| {
        s.frame.as_ref().is_some_and(|f| f.gpu_ok)
    });
    assert!(
        registered.elapsed() < Duration::from_millis(400),
        "drawn {:?} after the effect registered",
        registered.elapsed()
    );
}

/// Every viewer of a window (the editor's, a popped-out one) follows the
/// window's extensions as they are now: one opened while an effect loads draws
/// it once it registers instead of keeping its first snapshot forever, and
/// after the effect's code is reloaded every viewer draws the new code.
#[test]
fn every_viewer_of_a_window_follows_its_extensions_as_they_load_and_reload() {
    let project = || {
        snapshot(&format!(
            r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
            "layers":[{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,3],"params":{{}}}}"##,
            shape("top", "#FF0000", 320.).replace(
                r#""fx":[]"#,
                r#""fx":[{"id":"e1","type":"ext.invert","on":true,"p":{}}]"#
            )
        ))
    };
    let service = service("window-feed");
    let feed = pm_engine_service::resources::ResourceFeed::default();
    let publish = |generation, definitions, pending| {
        feed.publish(Arc::new(pm_engine_service::resources::ResourceSnapshot {
            generation,
            definitions,
            pending,
            ..Default::default()
        }))
        .unwrap()
    };
    // The window's extensions are loading when both viewers open.
    publish(1, vec![], true);
    let editor = service
        .viewer(config(project(), MediaCatalog::default(), output(320, 180)))
        .unwrap();
    feed.attach(&editor).unwrap();
    let popout = service
        .viewer(config(project(), MediaCatalog::default(), output(320, 180)))
        .unwrap();
    feed.attach(&popout).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        for v in [&editor, &popout] {
            let status = v.status();
            assert!(
                status.frame.is_none() && status.missing_effects.is_empty(),
                "held while it loads: {status:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut invert = pm_render::all_builtin_definitions()
        .definitions
        .into_iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap();
    invert.id = "ext.invert".into();
    invert.origin = pm_protocol::definitions::DefinitionOrigin::Local;
    let centre = |v: &Viewer, why: &str| {
        v.capture_next().unwrap();
        let stamp = v.stamp().unwrap();
        let drawn = wait(v, why, |s| {
            s.frame
                .as_ref()
                .is_some_and(|f| f.stamp == stamp && f.pixels.is_some())
        });
        assert!(
            drawn.missing_effects.is_empty() && drawn.error.is_none(),
            "{why}: {drawn:?}"
        );
        drawn.frame.unwrap().pixels.unwrap()[90 * 320 + 160]
    };
    // It registers: both draw it (red inverted is cyan).
    publish(2, vec![invert.clone()], false);
    for (v, name) in [(&editor, "editor"), (&popout, "popout")] {
        let c = centre(v, &format!("the {name} draws the registered effect"));
        assert!(c[0] < 0.25 && c[1] > 0.9, "{name} inverted: {c:?}");
    }
    // Its extension reloads with other code: held, then the new code, in both.
    publish(3, vec![], true);
    let mut reloaded = invert;
    let glsl = reloaded.glsl.as_mut().unwrap();
    let end = glsl.source.rfind('}').unwrap();
    glsl.source
        .insert_str(end, "o = vec4(0.0, 0.0, 1.0, 1.0);\n");
    publish(4, vec![reloaded], false);
    for (v, name) in [(&editor, "editor"), (&popout, "popout")] {
        let c = centre(v, &format!("the {name} draws the reloaded effect"));
        assert!(
            c[0] < 0.1 && c[1] < 0.1 && c[2] > 0.9,
            "{name} reloaded: {c:?}"
        );
    }
    // A retired viewer is forgotten; one opened now starts from the latest.
    popout.retire();
    publish(5, vec![], false);
    let late = service
        .viewer(config(project(), MediaCatalog::default(), output(320, 180)))
        .unwrap();
    feed.attach(&late).unwrap();
    late.capture_next().unwrap();
    let bypassed = wait(&late, "the late viewer draws", |s| {
        s.frame.as_ref().is_some_and(|f| f.pixels.is_some())
    });
    assert!(
        bypassed
            .missing_effects
            .iter()
            .any(|m| m.effect == "ext.invert"),
        "{bypassed:?}"
    );
}

/// Who is still loading names a wait; it is not content. While extensions
/// load, each one that finishes changes the names only: a viewer whose
/// project needs none of them keeps its frame, its installed effects and its
/// cached pictures (no new stamp, nothing drawn again).
#[test]
fn who_is_still_loading_changes_no_frame_or_render_identity() {
    let plain = snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":640,"h":360,"fps":30,"dur":3,"bg":"#000000",
        "layers":[{}],"comps":{{}},"assets":{{}},"markers":[],"work":[0,3],"params":{{}}}}"##,
        shape("top", "#FF0000", 320.)
    ));
    let service = service("loading-names");
    let v = service
        .viewer(config(plain, MediaCatalog::default(), output(320, 180)))
        .unwrap();
    let loading = |generation, names: &[&str]| {
        Arc::new(pm_engine_service::resources::ResourceSnapshot {
            generation,
            pending: true,
            loading: names
                .iter()
                .map(|n| (n.to_string(), n.to_string()))
                .collect(),
            ..Default::default()
        })
    };
    v.resources(loading(1, &["a", "b", "c"])).unwrap();
    // At rest: the paused viewer's settled picture is on screen (it lands
    // after its preview, however long the machine takes to draw it).
    let stamp = wait(&v, "the settled picture", |s| {
        s.frame.as_ref().is_some_and(|f| f.settled)
    })
    .frame
    .unwrap()
    .stamp;
    assert_eq!(
        v.stamp().unwrap(),
        stamp,
        "nothing after the settled picture"
    );
    v.resources(loading(2, &["b", "c"])).unwrap();
    v.resources(loading(3, &["c"])).unwrap();
    assert_eq!(v.stamp().unwrap(), stamp, "names alone moved the stamp");
    std::thread::sleep(Duration::from_millis(400));
    let after = v.status();
    assert_eq!(
        after.frame.unwrap().stamp,
        stamp,
        "names alone drew a frame"
    );
    // The load ending is a change of pins: frames that waited may draw now.
    v.resources(Arc::new(pm_engine_service::resources::ResourceSnapshot {
        generation: 4,
        ..Default::default()
    }))
    .unwrap();
    assert_ne!(v.stamp().unwrap(), stamp);
}

/// A project saved by the shipped app at current main
/// (`pm-kernel/tests/fixtures/main-current`) draws in the viewer: no error and
/// no hang, frames at several times, and what this build can't draw yet is
/// bypassed and named (an effect type nothing provides, and a revision newer
/// than this build), never dropped from the document.
#[cfg(feature = "kernel")]
#[test]
fn a_current_main_project_draws_and_names_what_this_build_bypasses() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kernel/tests/fixtures/main-current/current.pmv");
    let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
    let store = pm_kernel::DocumentStore::open(opened);
    let before =
        pm_model::json::to_canonical_string(&store.snapshot().document.root().unwrap().to_value());
    let service = service("main-current");
    let v = service
        .viewer(config(
            store.snapshot(),
            MediaCatalog::default(),
            output(320, 180),
        ))
        .unwrap();
    for seconds in [0, 1, 3] {
        v.command(C::Seek {
            time: Ticks::new(seconds * TICKS_PER_SECOND).unwrap(),
            mode: SeekMode::Exact,
        })
        .unwrap();
        v.capture_next().unwrap();
        let stamp = v.stamp().unwrap();
        let status = wait(&v, "a current-main frame", |s| {
            s.error.is_some()
                || s.frame
                    .as_ref()
                    .is_some_and(|f| f.stamp == stamp && f.pixels.is_some())
        });
        assert_eq!(status.error, None, "t={seconds}");
        let named = |effect: &str, revision: Option<&str>| {
            status
                .missing_effects
                .iter()
                .any(|m| m.effect == effect && m.revision.as_deref() == revision)
        };
        assert!(
            named("blur", Some("99")) && named("futureEffect", None),
            "t={seconds}: {:?}",
            status.missing_effects
        );
        // Whatever else can't draw yet (inputs, fonts this test service
        // lacks) is a named layer failure, and the notice says so.
        assert!(status.layer_failures.iter().all(|f| !f.message.is_empty()));
        assert!(status.notice().is_some());
    }
    assert_eq!(
        pm_model::json::to_canonical_string(&store.snapshot().document.root().unwrap().to_value()),
        before,
        "drawing never rewrites the document"
    );
}

/// A 64 × 64 still of one grey level.
fn grey_still(path: &std::path::Path, level: u8) {
    let (w, h) = (64u32, 64u32);
    let size = 54 + w * 3 * h;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    b.resize(size as usize, level);
    std::fs::write(path, b).unwrap();
}
/// A short composition filled by one linked still, looping.
fn looping_still(service: &EngineService, still: &std::path::Path) -> Viewer {
    let s = snapshot(&format!(
        r##"{{"id":"P","name":"P","compId":"C","compName":"C","w":128,"h":72,"fps":30,"dur":0.2,"bg":"#000000",
        "layers":[{{"id":"i","type":"image","name":"still","from":0,"dur":0.2,"on":true,"solo":false,"mblur":false,
          "p":{{{}}},"fx":[],"masks":[],"d":{{"asset":"pic","fit":"stretch","w":64,"h":64}}}}],
        "comps":{{}},"assets":{{"pic":{{"id":"pic","type":"image","name":"still","w":64,"h":64,"dur":0}}}},
        "markers":[],"work":[0,0.2],"params":{{}}}}"##,
        transform(32., 4.)
    ));
    let mut media = MediaCatalog::default();
    media.originals.insert("pic".into(), still.to_path_buf());
    let range = pm_protocol::common::TickRange {
        start: Ticks::ZERO,
        end: Ticks::new(TICKS_PER_SECOND / 5).unwrap(),
    };
    let v = service
        .viewer(ViewerConfig {
            end: Some(range.end.get()),
            ..config(s, media, output(128, 72))
        })
        .unwrap();
    wait(&v, "first frame", |s| s.frame.is_some());
    v.command(C::SetLoop { range: Some(range) }).unwrap();
    v.command(C::Play {
        rate: PlaybackRate::NORMAL,
    })
    .unwrap();
    v
}

/// A still saved again by another app while its composition loops. The
/// loop's frames are kept in memory and nothing in the document changed, yet
/// the picture on screen (which the scopes read) becomes the new file's: a
/// frame kept in memory is the picture of the media as it was observed.
#[test]
fn a_looping_frame_follows_a_still_saved_again_on_disk() {
    a_looping_frame_follows(|still| grey_still(still, 220));
}

/// The same when the new file has the old one's length and its modification
/// time is put back (the inode is the same): it is another version of the
/// file all the same.
#[test]
fn a_looping_frame_follows_a_same_length_rewrite_with_its_time_put_back() {
    a_looping_frame_follows(|still| {
        let modified = std::fs::metadata(still).unwrap().modified().unwrap();
        // The status-change time has the file system's resolution.
        std::thread::sleep(Duration::from_millis(20));
        grey_still(still, 220);
        std::fs::File::options()
            .write(true)
            .open(still)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    });
}

/// A composition looping one dark still, `save`d again (bright) meanwhile.
fn a_looping_frame_follows(save: impl Fn(&std::path::Path)) {
    let dir = tempfile::tempdir().unwrap();
    let still = dir.path().join("still.bmp");
    grey_still(&still, 40);
    let service = service("live-media");
    let v = looping_still(&service, &still);
    let (frames, inbox) = async_channel::bounded(1);
    v.scopes(Some((pm_render::scopes::ScopeConfig::PANEL, frames)))
        .unwrap();
    // The scope's reading of a frame presented from now on.
    let shown = |inbox: &async_channel::Receiver<ScopeResult>| {
        while inbox.try_recv().is_ok() {}
        let start = Instant::now();
        loop {
            if let Ok(frame) = inbox.try_recv() {
                return frame.unwrap().luma_histogram;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "no frame was analyzed"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    wait(&v, "a looped frame presented from memory", |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| f.moment == Moment::Playing && f.cache_hit)
    });
    let dark = shown(&inbox);
    save(&still);
    // Media is looked at every 2 s.
    let start = Instant::now();
    let now = loop {
        let now = shown(&inbox);
        if now != dark {
            break now;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "the loop keeps presenting the still as it was"
        );
    };
    // And it stays: no frame of the old still is left to come round again.
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        assert_eq!(shown(&inbox), now, "an old frame came round again");
    }
    // It is what a viewer that never saw the dark still plays.
    let fresh_service = self::service("live-media-fresh");
    let fresh = looping_still(&fresh_service, &still);
    let (frames, fresh_inbox) = async_channel::bounded(1);
    fresh
        .scopes(Some((pm_render::scopes::ScopeConfig::PANEL, frames)))
        .unwrap();
    assert_eq!(now, shown(&fresh_inbox));
    assert_eq!(v.status().error, None);
}
