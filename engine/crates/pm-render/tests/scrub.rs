//! Scrub renders (docs/engine/scrub.md): a video layer whose exact frame
//! has not decoded yet draws the nearest decoded frame while the exact one
//! decodes, the render says so, and the frame drawn once the exact video
//! landed equals an exact render. Exports never draw stand-ins.

mod support;

use pm_media::decode::service::MediaLease;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::{repo_root, ticks};

fn clip() -> (pm_model::Document, MapResolver) {
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 }),
        0.0,
        0.0,
        2.0,
    );
    let mut doc = project(160, 90, 2.0, no_fill(), vec![v]);
    asset(&mut doc, "clip", "video", "mp4", 160, 90, 2.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert(
        "clip".into(),
        repo_root().join("apps/desktop/e2e/fixtures/h264-aac.mp4"),
    );
    (load(&doc), resolver)
}

fn draw(
    r: &mut Renderer,
    doc: &pm_model::Document,
    t: f64,
    request: &RenderRequest,
    resolver: &MapResolver,
) -> (Vec<u8>, usize) {
    let target = pm_gpu::Target::new(
        r.device(),
        "scrub test",
        160,
        90,
        pm_gpu::working_format(Precision::Float16),
        pm_gpu::target::working_usage(),
    );
    let rendered = r
        .render_into(
            doc,
            "Ptest-root",
            ticks(t),
            request,
            resolver,
            &target,
            None,
        )
        .expect("render");
    assert!(rendered.failures.is_empty(), "{:?}", rendered.failures);
    let bytes = pm_gpu::readback::read_raw(r.device(), r.queue(), &target.texture).unwrap();
    (bytes, rendered.stand_ins.len())
}

#[test]
fn a_scrub_render_stands_in_until_the_exact_frame_lands() {
    let (doc, resolver) = clip();
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let mut r = Renderer::headless().unwrap();
    // A paused frame near the start (exact).
    let (start, stand_ins) = draw(&mut r, &doc, 0.1, &request, &resolver);
    assert_eq!(stand_ins, 0);
    // A scrub into the other GOP: the frame near 0.1 s stands in at once.
    r.set_scrub(true);
    let landed = r.media_landed();
    let (stood, stand_ins) = draw(&mut r, &doc, 1.6, &request, &resolver);
    // The request does not wait: frame 48 is in the second GOP, and nothing
    // decoded it yet.
    assert_eq!(stand_ins, 1);
    assert!(stood == start, "the stand-in is the frame decoded last");
    let t = std::time::Instant::now();
    while r.media_landed() == landed {
        assert!(t.elapsed().as_secs() < 20, "the exact frame never landed");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let (_, again) = draw(&mut r, &doc, 1.6, &request, &resolver);
    assert_eq!(again, 0, "the scrub target landed");
    let (scrubbed, _) = draw(&mut r, &doc, 1.6, &request, &resolver);
    r.set_scrub(false);
    let (exact, _) = draw(
        &mut Renderer::headless().unwrap(),
        &doc,
        1.6,
        &request,
        &resolver,
    );
    assert!(scrubbed == exact, "the landed frame is the exact frame");
    assert_eq!(r.scrub_stats().stand_ins, 1);
}

#[test]
fn exports_never_draw_stand_ins() {
    let (doc, resolver) = clip();
    let mut request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let mut r = Renderer::headless().unwrap();
    draw(&mut r, &doc, 0.1, &request, &resolver);
    r.set_scrub(true);
    request.export = true;
    let (bytes, stand_ins) = draw(&mut r, &doc, 1.6, &request, &resolver);
    assert_eq!(stand_ins, 0);
    let (exact, _) = draw(
        &mut Renderer::headless().unwrap(),
        &doc,
        1.6,
        &request,
        &resolver,
    );
    assert!(bytes == exact);
}

/// Converted video frames are reused from one render to the next: two
/// layers of one clip at different times, over several renders, equal
/// fresh renders (no frame is overwritten while a render still reads it).
#[test]
fn reused_frame_textures_never_change_what_a_render_reads() {
    let at = |id: &str, x: f64, trim: f64| {
        layer(
            id,
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": 80, "h": 90, "trim": trim, "speed": 1 }),
            x,
            0.0,
            2.0,
        )
    };
    let mut doc = project(
        160,
        90,
        2.0,
        no_fill(),
        vec![at("a", 0.0, 0.0), at("b", 80.0, 1.0)],
    );
    asset(&mut doc, "clip", "video", "mp4", 160, 90, 2.0);
    let doc = load(&doc);
    let (_, resolver) = clip();
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let mut r = Renderer::headless().unwrap();
    for t in [0.2, 0.4, 0.6, 0.4, 0.9] {
        let (bytes, _) = draw(&mut r, &doc, t, &request, &resolver);
        let (fresh, _) = draw(
            &mut Renderer::headless().unwrap(),
            &doc,
            t,
            &request,
            &resolver,
        );
        assert!(bytes == fresh, "render at {t} s");
    }
}

/// Renderers start with the small frame cache (export jobs and headless
/// renders read each frame once); only a lease share makes it larger, and
/// a smaller share evicts at once.
#[test]
fn the_frame_cache_follows_the_lease_share() {
    let (doc, resolver) = clip();
    let mut r = Renderer::headless().unwrap();
    assert_eq!(
        r.media_usage().cache.budget,
        pm_render::media::DEFAULT_FRAME_CACHE
    );
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let (_, stand_ins) = draw(&mut r, &doc, 0.5, &request, &resolver);
    assert_eq!(stand_ins, 0);
    assert!(r.media_usage().frame_cache > 0);
    let usage = r.apply_media_lease(MediaLease {
        frame_cache: 512 << 20,
        decoders: usize::MAX,
    });
    assert_eq!(usage.cache.budget, 512 << 20);
    let usage = r.apply_media_lease(MediaLease {
        frame_cache: 0,
        decoders: usize::MAX,
    });
    assert_eq!(usage.cache.budget, 0);
    assert_eq!(usage.frame_cache, 0);
}

/// The cost of drawing one decoded UHD frame into a 1080p viewer frame
/// (import, normalization, the source pass, compositing, GPU wait), with
/// the frames already decoded: what a drag through cached frames pays per
/// refresh. `PM_SCRUB_CLIP` names a long-GOP UHD clip (docs/engine/tracer.md).
#[test]
#[ignore]
fn drawing_cached_uhd_frames() {
    let Some(path) = std::env::var_os("PM_SCRUB_CLIP") else {
        return;
    };
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": 1920, "h": 1080, "trim": 0, "speed": 1 }),
        0.0,
        0.0,
        30.0,
    );
    let mut doc = project(1920, 1080, 30.0, no_fill(), vec![v]);
    asset(&mut doc, "clip", "video", "mp4", 3840, 2160, 30.0);
    let doc = load(&doc);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.into());
    let request = RenderRequest::at_scale(Size::new(1920, 1080), 1.0, Precision::Float16);
    let mut r = Renderer::headless().unwrap();
    // The viewer's frame-cache share (pm-node's presenter applies it).
    r.apply_media_lease(MediaLease {
        frame_cache: 512 << 20,
        decoders: usize::MAX,
    });
    let target = pm_gpu::Target::new(
        r.device(),
        "scrub bench",
        1920,
        1080,
        pm_gpu::working_format(Precision::Float16),
        pm_gpu::target::working_usage(),
    );
    let t = |k: u32| f64::from(k) * 1001.0 / 30000.0 + 0.001;
    // Decode 24 frames (the frame cache keeps them).
    for k in 1..=24 {
        r.render_into(
            &doc,
            "Ptest-root",
            ticks(t(k)),
            &request,
            &resolver,
            &target,
            None,
        )
        .unwrap();
    }
    let _ = r.device().poll(wgpu::PollType::wait_indefinitely());
    let mut times = vec![];
    for round in 0..3 {
        for k in 1..=24 {
            let t0 = std::time::Instant::now();
            r.render_into(
                &doc,
                "Ptest-root",
                ticks(t(k)),
                &request,
                &resolver,
                &target,
                None,
            )
            .unwrap();
            let _ = r.device().poll(wgpu::PollType::wait_indefinitely());
            if round > 0 {
                times.push(t0.elapsed().as_secs_f64() * 1e3);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "cached UHD frame into 1080p: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms; {:?}",
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times[times.len() - 1],
        r.scrub_stats()
    );
}
