//! The engine's share of typing latency (plan section 25, "Canvas text
//! typing p95 ≤ 33 ms"): each keystroke changes a text layer's string and
//! renders the 1920 × 1080 frame (layout, glyph scene, Vello coverage, paint,
//! composite), timed until the GPU has finished. The viewer bridge and the
//! editor's own handling are not included.
//!
//! cargo run --release -p pm-render --example text_typing

#![recursion_limit = "256"]

#[cfg(target_os = "macos")]
fn support_ticks() -> pm_protocol::common::Ticks {
    pm_protocol::common::Ticks::new(0).expect("zero is a safe integer")
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::{MapResolver, RenderRequest, Renderer};
    use serde_json::{Value, json};

    struct Fonts(pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>);
    // SAFETY: single-threaded tool on the main thread.
    unsafe impl Send for Fonts {}
    impl pm_render::text::TextFonts for Fonts {
        fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
            &mut self.0
        }
        fn line_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
        ) -> Vec<pm_text::compat::LineRuns> {
            self.0.compat().line_runs(d)
        }
        fn text_runs(
            &mut self,
            d: &pm_text::compat::TextContent,
            t: &str,
            tracking: f64,
        ) -> Vec<pm_text::compat::GlyphRun> {
            self.0.compat().text_runs(d, t, tracking).0
        }
        fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
            use pm_text::font::FontResolver;
            self.0.compat().resolver().face(id)
        }
    }

    fn ch(v: Value) -> Value {
        json!({ "v": v, "kf": [], "expr": null })
    }
    let doc = |text: &str, d_extra: Value| {
        let mut d = json!({ "text": text, "font": "SF Pro Display", "weight": 600, "size": 96, "tracking": -2,
            "leading": 1.1, "color": "#FFFFFF", "align": "center", "italic": false, "boxWidth": 0, "boxHeight": 0 });
        for (k, v) in d_extra.as_object().unwrap() {
            d[k] = v.clone();
        }
        let p = json!({
            "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)), "scale.z": ch(json!(100)),
            "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)), "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)),
            "orientation.z": ch(json!(0)), "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(960)),
            "position.y": ch(json!(540)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)), "rotation": ch(json!(0)),
            "opacity": ch(json!(100)), "skew": ch(json!(0))
        });
        let root = json!({
            "id": "P", "name": "typing", "w": 1920, "h": 1080, "fps": 30, "dur": 10, "bg": "#101418",
            "compId": "P-root", "compName": "typing", "markers": [], "work": [0, 10], "params": {}, "revision": 0,
            "edits": [], "created": 0, "shutter": 0.5, "assets": {}, "comps": {},
            "layers": [{ "id": "t", "type": "text", "name": "t", "from": 0, "dur": 10, "on": true, "lock": false,
                "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF", "blend": "normal",
                "mblur": false, "parent": null, "group": null, "matteSource": null, "p": p, "fx": [],
                "transitionIn": null, "transitionOut": null, "masks": [], "d": d }]
        });
        pm_model::Document::new(Some(pm_model::json::parse(&root.to_string()).unwrap()), 1)
    };
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_text_fonts(Some(Box::new(Fonts(
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
    ))));
    let request = RenderRequest::sized(Size::new(1920, 1080), Precision::Float16);
    let sentence = "The quick brown fox jumps over the lazy dog while editors type captions";
    let scenarios = [
        ("title 96 px", json!({})),
        (
            "paragraph 48 px, 1400 px box",
            json!({ "size": 48, "boxWidth": 1400, "align": "left", "weight": 500 }),
        ),
        ("caption-size 36 px", json!({ "size": 36, "weight": 500 })),
    ];
    for (name, extra) in scenarios {
        // Warm up (pipelines, Vello, fonts).
        for t in ["W", "Wa"] {
            let f = renderer
                .render(
                    &doc(t, extra.clone()),
                    "P-root",
                    support_ticks(),
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            drop(f);
        }
        renderer
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mut times = Vec::new();
        for i in 1..=sentence.len() {
            let d = doc(&sentence[..i], extra.clone());
            let start = std::time::Instant::now();
            let frame = renderer
                .render(
                    &d,
                    "P-root",
                    support_ticks(),
                    &request,
                    &MapResolver::default(),
                )
                .unwrap();
            renderer
                .device()
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            drop(frame);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |p: f64| times[((times.len() as f64 - 1.0) * p).round() as usize];
        println!(
            "{name:<32} {} keystrokes: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            times.len(),
            q(0.5),
            q(0.95),
            times.last().unwrap()
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
