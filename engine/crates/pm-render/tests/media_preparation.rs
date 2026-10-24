#[path = "support/doc.rs"]
mod doc;

use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::json;
use std::sync::{Mutex, OnceLock};

fn serial_preparation() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn planned_video_preparation_reuses_exact_source_and_decoder_on_first_render() {
    let _serial = serial_preparation();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    let (w, h) = (256, 144);
    let mut project = doc::project(
        w,
        h,
        2.0,
        doc::no_fill(),
        vec![doc::layer(
            "v",
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    doc::asset(&mut project, "clip", "video", "mp4", w, h, 2.0);
    let document = doc::load(&project);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let request = RenderRequest {
        export: true,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let time = pm_protocol::common::Ticks::new(pm_time::TICKS_PER_SECOND / 2).unwrap();
    let mut renderer = Renderer::headless().unwrap();

    let prepared = renderer
        .prepare_media(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    let completed = renderer.finish_media_preparation();
    assert!(prepared.failures.is_empty(), "{:?}", prepared.failures);
    assert!(completed.is_empty(), "{completed:?}");
    assert_eq!(prepared.queued_videos, 1);
    let before = renderer.media_usage();
    assert_eq!(before.sessions, 1);

    let frame = renderer
        .render(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    renderer.wait_idle();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    // The prepared decoder and its decoded frame serve the first render:
    // one cache hit, no decode, no second session.
    let after = renderer.media_usage();
    assert_eq!(after.cache.hits, before.cache.hits + 1);
    assert_eq!(after.cache.misses, before.cache.misses);
    assert_eq!(after.sessions, 1);

    // The first renderer stays alive and idle. Two more renderers must still
    // be able to prepare; idle workers do not monopolize the global cap.
    let mut idle_renderers = Vec::new();
    for _ in 0..2 {
        let mut next = Renderer::headless().unwrap();
        let queued = next
            .prepare_media(&document, "Ptest-root", time, &request, &assets)
            .unwrap();
        assert_eq!(queued.queued_videos, 1);
        assert!(next.finish_media_preparation().is_empty());
        idle_renderers.push(next);
    }
}

#[test]
fn display_preparation_caches_selected_frame_for_first_render() {
    let _serial = serial_preparation();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    let (w, h) = (256, 144);
    let mut project = doc::project(
        w,
        h,
        2.0,
        doc::no_fill(),
        vec![doc::layer(
            "v",
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    doc::asset(&mut project, "clip", "video", "mp4", w, h, 2.0);
    let document = doc::load(&project);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let request = RenderRequest {
        export: false,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let time = pm_protocol::common::Ticks::new(pm_time::TICKS_PER_SECOND / 2).unwrap();
    let mut renderer = Renderer::headless().unwrap();

    let prepared = renderer
        .prepare_media(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    assert_eq!(prepared.queued_videos, 1);
    assert!(renderer.finish_media_preparation().is_empty());

    let viewer_before = renderer.scrub_stats();
    let frame = renderer
        .render(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    renderer.wait_idle();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let viewer_after = renderer.scrub_stats();
    assert_eq!(viewer_after.cached, viewer_before.cached + 1);
    assert_eq!(viewer_after.decoded, viewer_before.decoded);

    let prepared_rgba = renderer.read_rgba(&frame).unwrap();
    let mut cold = Renderer::headless().unwrap();
    let cold_frame = cold
        .render(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    cold.wait_idle();
    assert!(cold_frame.failures.is_empty(), "{:?}", cold_frame.failures);
    assert_eq!(prepared_rgba, cold.read_rgba(&cold_frame).unwrap());
}

#[test]
fn preparation_finds_video_inside_a_nested_composition() {
    let _serial = serial_preparation();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    let (w, h) = (256, 144);
    let mut project = doc::project(
        w,
        h,
        2.0,
        doc::no_fill(),
        vec![doc::layer(
            "pre",
            "precomp",
            json!({"comp":"inner","w":w,"h":h}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    doc::composition(
        &mut project,
        "inner",
        w,
        h,
        2.0,
        vec![doc::layer(
            "nested-video",
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        )],
    );
    doc::asset(&mut project, "clip", "video", "mp4", w, h, 2.0);
    let document = doc::load(&project);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let request = RenderRequest {
        export: true,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let time = pm_protocol::common::Ticks::new(pm_time::TICKS_PER_SECOND / 2).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let prepared = renderer
        .prepare_media(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    assert!(renderer.finish_media_preparation().is_empty());
    assert!(prepared.failures.is_empty(), "{:?}", prepared.failures);
    assert_eq!(prepared.queued_videos, 1);
}

#[test]
fn display_preparation_is_bounded_across_many_layers() {
    let _serial = serial_preparation();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/video/h264-709-limited.mp4");
    let (w, h) = (256, 144);
    let mut layers = Vec::new();
    for i in 0..12 {
        layers.push(doc::layer(
            &format!("v{i}"),
            "video",
            json!({"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
            f64::from(w) / 2.0,
            f64::from(h) / 2.0,
            2.0,
        ));
    }
    let mut project = doc::project(w, h, 2.0, doc::no_fill(), layers);
    doc::asset(&mut project, "clip", "video", "mp4", w, h, 2.0);
    let document = doc::load(&project);
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let request = RenderRequest {
        export: false,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let time = pm_protocol::common::Ticks::new(pm_time::TICKS_PER_SECOND / 2).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let prepared = renderer
        .prepare_media(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    assert!(prepared.failures.is_empty(), "{:?}", prepared.failures);
    assert_eq!(prepared.queued_videos, 12);
    assert!(renderer.finish_media_preparation().is_empty());
    // Eight distinct layer instances share one source identity, then the
    // global prepared-session cap skips the remaining four.
    assert_eq!(renderer.media_usage().sessions, 8);
}

#[test]
fn a_frame_opens_its_new_video_sources_together() {
    let _serial = serial_preparation();
    let fixtures =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
    let (w, h) = (256, 144);
    let clips = [
        "h264-709-limited.mp4",
        "hevc10-709-limited.mp4",
        "vp9-601-limited.webm",
    ];
    let layers = clips
        .iter()
        .enumerate()
        .map(|(i, _)| {
            doc::layer(
                &format!("v{i}"),
                "video",
                json!({"asset":format!("clip{i}"),"fit":"stretch","w":w,"h":h,"trim":0,"speed":1}),
                f64::from(w) / 2.0,
                f64::from(h) / 2.0,
                2.0,
            )
        })
        .collect();
    let mut project = doc::project(w, h, 2.0, doc::no_fill(), layers);
    let mut assets = MapResolver::default();
    for (i, clip) in clips.iter().enumerate() {
        let ext = clip.rsplit('.').next().unwrap();
        doc::asset(&mut project, &format!("clip{i}"), "video", ext, w, h, 2.0);
        assets.paths.insert(format!("clip{i}"), fixtures.join(clip));
    }
    let document = doc::load(&project);
    let request = RenderRequest {
        export: false,
        video_end: VideoEnd::Final,
        ..RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32)
    };
    let time = pm_protocol::common::Ticks::new(pm_time::TICKS_PER_SECOND / 2).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(&document, "Ptest-root", time, &request, &assets)
        .unwrap();
    renderer.wait_idle();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    // Every layer found its frame decoded: the sources were opened and
    // their frames decoded together before the layers drew.
    let stats = renderer.scrub_stats();
    assert_eq!((stats.cached, stats.decoded), (3, 0), "{stats:?}");
}
