//! Paired level 2 vs today's level 1: query bundle, whole-frame GPU work,
//! and typing mutations at 1920x1080. Alternate pair order every sample;
//! three passes retain warm caches but typing strings change every sample.
//! Measures engine work through GPU completion, excluding viewer/event IPC.
//! Run under pm-electron-lock: cargo run --release -p pm-render --example text_level2_perf

#![recursion_limit = "256"]

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

    impl Drop for Fonts {
        fn drop(&mut self) {
            eprintln!("renderer cache: {:?}", self.0.level2_cache_stats());
        }
    }

    fn ch(v: Value) -> Value {
        json!({ "v": v, "kf": [], "expr": null })
    }
    let doc = |text: &str, d_extra: Value, compat: usize| {
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
            "layers": [{ "id": "t", "type": "text", "compat": compat, "name": "t", "from": 0, "dur": 10, "on": true, "lock": false,
                "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF", "blend": "normal",
                "mblur": false, "parent": null, "group": null, "matteSource": null, "p": p, "fx": [],
                "transitionIn": null, "transitionOut": null, "masks": [], "d": d }]
        });
        pm_model::Document::new(Some(pm_model::json::parse(&root.to_string()).unwrap()), 1)
    };
    fn queries(
        measure: &mut pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
        doc: &pm_model::Document,
        t: f64,
    ) {
        use pm_eval::bounds::{NoHost, PickOptions};
        let comp = pm_eval::Comp::resolve(doc, "P-root").unwrap();
        let mut ev = pm_eval::Evaluator::new();
        let mut scope = pm_eval::Scope::open(&mut ev, &comp);
        let l = &scope.layers()[0];
        let b = scope.bounds(l, t, measure, &mut NoHost).unwrap().unwrap();
        std::hint::black_box(scope.text_layout(l, t, measure).unwrap());
        std::hint::black_box(scope.text_geometry(l, t, measure).unwrap());
        std::hint::black_box(
            scope
                .pick(
                    960.0 + (b.x0 + b.x1) / 2.0,
                    540.0 + (b.y0 + b.y1) / 2.0,
                    t,
                    &PickOptions::default(),
                    measure,
                    &mut NoHost,
                )
                .unwrap(),
        );
    }
    fn percentiles(times: &[f64]) -> Value {
        let mut times = times.to_vec();
        times.sort_by(f64::total_cmp);
        let q = |p: f64| times[((times.len() - 1) as f64 * p).ceil() as usize];
        json!({"samples":times.len(),"p50_ms":q(0.5),"p95_ms":q(0.95),"p99_ms":q(0.99),"max_ms":q(1.0)})
    }
    let request = RenderRequest::sized(Size::new(1920, 1080), Precision::Float16);
    let assets = MapResolver::default();
    let sentence = "The quick brown fox jumps over the lazy dog while editors type captions";
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../../corpus/fixtures/text-level2.json")).unwrap();
    let wave = fixtures
        .iter()
        .find(|v| v["id"] == "text-level2/line-wave")
        .unwrap()["animators"]
        .clone();
    let scenarios = [
        ("title 96px", json!({})),
        (
            "paragraph 48px",
            json!({"size":48,"weight":500,"boxWidth":1400,"align":"left"}),
        ),
        (
            "caption 24px",
            json!({"size":24,"weight":400,"font":"system-ui"}),
        ),
        (
            "range typography 40px",
            json!({"size":40,"weight":400,"font":"Georgia","tracking":0,
                "styles":[{"id":"weight","start":4,"end":20,"p":{"weight":{"v":700,"kf":[],"expr":null}}},
                    {"id":"leading","start":21,"end":42,"p":{"leading":{"v":1.5,"kf":[],"expr":null},"tracking":{"v":3,"kf":[],"expr":null}}}]}),
        ),
    ];
    for (name, extra) in scenarios {
        let mut renderers: Vec<_> = (0..2)
            .map(|_| {
                let mut r = Renderer::headless().expect("Metal");
                r.set_text_fonts(Some(Box::new(Fonts(
                    pm_eval::text_measure::CompatMeasure::new(
                        pm_text::registry::CoreTextRegistry::new(),
                    ),
                ))));
                r
            })
            .collect();
        let mut measures: Vec<_> =
            (0..2)
                .map(|_| {
                    pm_eval::text_measure::CompatMeasure::new(
                        pm_text::registry::CoreTextRegistry::new(),
                    )
                })
                .collect();
        for phase in ["frame", "animated frame", "typing"] {
            let mut extra = extra.clone();
            if phase == "animated frame" {
                extra["animators"] = wave.clone();
            }
            for (index, r) in renderers.iter_mut().enumerate() {
                for i in 0..20 {
                    let d = doc(
                        if phase == "typing" {
                            "Warm up type"
                        } else {
                            sentence
                        },
                        extra.clone(),
                        index + 1,
                    );
                    let t = i as f64 / 60.0;
                    queries(&mut measures[index], &d, t);
                    let f = r
                        .render(
                            &d,
                            "P-root",
                            pm_protocol::common::Ticks::new(
                                (t * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
                            )
                            .unwrap(),
                            &request,
                            &assets,
                        )
                        .unwrap();
                    assert!(f.failures.is_empty(), "{:?}", f.failures);
                    r.device()
                        .poll(wgpu::PollType::wait_indefinitely())
                        .unwrap();
                }
            }
            let samples = if phase == "typing" {
                sentence.len()
            } else {
                120
            };
            let mut cpu = [Vec::new(), Vec::new()];
            let mut gpu = [Vec::new(), Vec::new()];
            let mut combined = [Vec::new(), Vec::new()];
            for pass in 0..3 {
                for i in 0..samples {
                    let text = if phase == "typing" {
                        &sentence[..i + 1]
                    } else {
                        sentence
                    };
                    let t = (i + pass * samples) as f64 / 60.0;
                    for index in if (i + pass) % 2 == 0 { [0, 1] } else { [1, 0] } {
                        let d = doc(text, extra.clone(), index + 1);
                        let start = std::time::Instant::now();
                        queries(&mut measures[index], &d, t);
                        let after_queries = std::time::Instant::now();
                        let r = &mut renderers[index];
                        let frame = r
                            .render(
                                &d,
                                "P-root",
                                pm_protocol::common::Ticks::new(
                                    (t * pm_protocol::common::TICKS_PER_SECOND as f64).round()
                                        as i64,
                                )
                                .unwrap(),
                                &request,
                                &assets,
                            )
                            .unwrap();
                        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                        r.device()
                            .poll(wgpu::PollType::wait_indefinitely())
                            .unwrap();
                        cpu[index].push(after_queries.duration_since(start).as_secs_f64() * 1000.0);
                        gpu[index].push(after_queries.elapsed().as_secs_f64() * 1000.0);
                        combined[index].push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
            }
            for index in 0..2 {
                println!(
                    "{}",
                    json!({"scenario":name,"phase":phase,"compat":index+1,
                    "queries":percentiles(&cpu[index]),"render_gpu":percentiles(&gpu[index]),
                    "combined":percentiles(&combined[index]),"query_cache":format!("{:?}",measures[index].level2_cache_stats())})
                );
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
