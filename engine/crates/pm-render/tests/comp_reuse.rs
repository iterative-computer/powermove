//! A nested composition drawn twice in one frame with the same plan (a
//! precomp used by two layers at one time) renders once, and the frame is
//! the frame drawn when each use renders its own copy: the same scene with
//! the second use pointing at an identical copy of the composition (another
//! id, so another plan) is compared bit for bit, at several times, at full
//! and region renders, and with the copy differing (no sharing then).

use std::path::PathBuf;

use pm_model::Document;
use pm_model::json::parse;
use pm_perf::{Counter, Scope};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_render::{AssetResolver, RenderRequest, Renderer};

struct NoMedia;

impl AssetResolver for NoMedia {
    fn path(&self, _asset: &str, _record: Option<&pm_model::Node>) -> Option<PathBuf> {
        None
    }
}

fn solid(id: &str, x: f64, y: f64, color: &str, extra: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"type\":\"solid\",\"name\":\"{id}\",\"from\":0,\"dur\":10,\"on\":true,\"d\":{{\"color\":\"{color}\",\"w\":120,\"h\":80}},\"p\":{{\"position.x\":{{\"v\":{x},\"kf\":[{{\"t\":0,\"v\":{x}}},{{\"t\":4,\"v\":{}}}]}},\"position.y\":{{\"v\":{y}}},\"rotation\":{{\"v\":0,\"kf\":[{{\"t\":0,\"v\":0}},{{\"t\":4,\"v\":90}}]}},\"opacity\":{{\"v\":85}}}}{extra}}}",
        x + 200.0
    )
}

fn nested(id: &str, tint: &str) -> String {
    let layers = [
        solid("a", 120.0, 90.0, "#E8553A", ""),
        solid("b", 300.0, 160.0, tint, ""),
        solid("c", 200.0, 200.0, "#3AA0E8", ",\"blend\":\"screen\""),
    ];
    format!(
        "\"{id}\":{{\"name\":\"{id}\",\"w\":480,\"h\":270,\"fps\":30,\"dur\":10,\"bg\":\"#000000\",\"backgroundFill\":{{\"type\":\"none\",\"stops\":[],\"angle\":0}},\"layers\":[{}]}}",
        layers.join(",")
    )
}

fn precomp(id: &str, comp: &str, x: f64, rotation: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"type\":\"precomp\",\"name\":\"{id}\",\"from\":0,\"dur\":10,\"on\":true,\"d\":{{\"comp\":\"{comp}\",\"w\":480,\"h\":270}},\"p\":{{\"position.x\":{{\"v\":{x}}},\"position.y\":{{\"v\":360}},\"opacity\":{{\"v\":80}},\"rotation\":{rotation}}}}}"
    )
}

/// The scene: two uses of `N`, the second through `second` (`N` or a copy).
fn document(second: &str, copy_tint: &str) -> Document {
    let rot = "{\"v\":0,\"kf\":[{\"t\":0,\"v\":0},{\"t\":4,\"v\":30}]}";
    let layers = [
        precomp("p1", "N", 360.0, "{\"v\":0}"),
        precomp("p2", second, 900.0, rot),
        solid("top", 640.0, 300.0, "#20C080", ""),
    ];
    let text = format!(
        "{{\"id\":\"Preuse\",\"name\":\"Reuse\",\"compId\":\"C\",\"assets\":{{}},\"comps\":{{\"C\":{{\"name\":\"Main\",\"w\":1280,\"h\":720,\"fps\":30,\"dur\":10,\"bg\":\"#141821\",\"layers\":[{}]}},{},{}}}}}",
        layers.join(","),
        nested("N", "#F0C030"),
        nested("M", copy_tint)
    );
    Document::new(Some(parse(&text).expect("the test document parses")), 1)
}

/// The frame's pixels and render passes.
fn frame(
    r: &mut Renderer,
    doc: &Document,
    seconds: f64,
    request: &RenderRequest,
) -> (Vec<f32>, u64) {
    let time = Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap();
    let scope = Scope::begin();
    let rendered = r
        .render(doc, "C", time, request, &NoMedia)
        .expect("renders");
    let passes = scope.finish().get(Counter::RenderPasses);
    (r.read_rgba(&rendered).expect("reads"), passes)
}

fn same(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

#[test]
fn a_precomp_used_twice_renders_once_and_draws_what_two_renders_draw() {
    let shared = document("N", "#F0C030");
    let copied = document("M", "#F0C030");
    let mut a = Renderer::headless().expect("a Metal device");
    let mut b = Renderer::headless().expect("a Metal device");
    for request in [
        RenderRequest::at_scale(Size::new(1280, 720), 1.0, Precision::Float16),
        RenderRequest::at_scale(Size::new(640, 360), 0.5, Precision::Float32),
    ] {
        for seconds in [0.0, 1.25, 2.5, 4.0, 7.0] {
            let (pa, na) = frame(&mut a, &shared, seconds, &request);
            let (pb, nb) = frame(&mut b, &copied, seconds, &request);
            assert!(
                same(&pa, &pb),
                "{seconds} s: the shared target draws other pixels"
            );
            // The second use samples the first's target: the nested
            // composition's passes are recorded once.
            assert!(na < nb, "{seconds} s: passes {na} and {nb}");
        }
    }
    // A copy that differs is its own plan: each renders.
    let differs = document("M", "#30F0C0");
    let (_, n_shared) = frame(
        &mut a,
        &shared,
        1.0,
        &RenderRequest::at_scale(Size::new(640, 360), 0.5, Precision::Float16),
    );
    let (_, n_differs) = frame(
        &mut b,
        &differs,
        1.0,
        &RenderRequest::at_scale(Size::new(640, 360), 0.5, Precision::Float16),
    );
    assert!(n_shared < n_differs, "passes {n_shared} and {n_differs}");
}

#[test]
fn shared_targets_last_one_frame() {
    // Frame after frame (the pool hands targets back after each), a
    // shared composition keeps drawing what a fresh renderer draws.
    let doc = document("N", "#F0C030");
    let request = RenderRequest::at_scale(Size::new(640, 360), 0.5, Precision::Float16);
    let mut long = Renderer::headless().expect("a Metal device");
    for k in 0..12 {
        let seconds = f64::from(k) / 3.0;
        let (pl, _) = frame(&mut long, &doc, seconds, &request);
        let mut fresh = Renderer::headless().expect("a Metal device");
        let (pf, _) = frame(&mut fresh, &doc, seconds, &request);
        assert!(same(&pl, &pf), "frame {k}");
    }
}
