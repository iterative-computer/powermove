//! Scrub renders (docs/engine/scrub.md): a video layer whose exact frame
//! has not decoded yet draws the nearest decoded frame while the exact one
//! decodes, the render says so, and the frame drawn once the exact video
//! landed equals an exact render. Exports never draw stand-ins.

mod support;

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
    (bytes, rendered.stand_ins)
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
