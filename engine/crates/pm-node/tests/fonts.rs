//! The interactive engine's font service end to end, through `EngineHost`
//! and the font port's wire format, on the main thread (AppKit's font
//! manager). Mirrors what main does: open with a generation base, answer the
//! families a document names, fetch subsets, lay text out over them in a
//! replay exactly as `pm-wasm` does, add a project font, and recover after a
//! restart.

#[cfg(target_os = "macos")]
fn main() {
    use pm_engine::Action;
    use pm_node::fonts::wire::{self, Op, Status};
    use pm_node::{EngineHost, FONTS_PEER};
    use pm_protocol::error::EngineProcess;
    use pm_text::answers::{Answers, Query, ReplayResolver};
    use pm_text::compat::{Compat, TextContent};
    use pm_text::font::FontBytes;

    struct Main {
        host: EngineHost,
        port: u32,
        next: u32,
    }

    impl Main {
        fn call(&mut self, op: Op, json: &str, tail: &[u8]) -> (Status, Vec<u8>) {
            self.next += 1;
            let actions = self
                .host
                .receive(self.port, &wire::encode_request(self.next, op, json, tail));
            let [Action::Send { port, bytes }] = actions.as_slice() else {
                panic!("one reply expected, got {actions:?}")
            };
            assert_eq!(port.0, self.port);
            let reply = wire::decode_reply(bytes).expect("a reply");
            assert_eq!(reply.id, self.next);
            (reply.status, reply.payload.to_vec())
        }

        fn json(&mut self, op: Op, json: &str, tail: &[u8]) -> serde_json::Value {
            let (status, payload) = self.call(op, json, tail);
            assert_eq!(
                status,
                Status::Json,
                "{}",
                String::from_utf8_lossy(&payload)
            );
            serde_json::from_slice(&payload).unwrap()
        }

        fn error(&mut self, op: Op, json: &str, tail: &[u8]) -> String {
            let (status, payload) = self.call(op, json, tail);
            assert_eq!(status, Status::Error);
            String::from_utf8(payload).unwrap()
        }
    }

    /// Render threads (the presenter, the job lanes) lay text out through
    /// the engine's font hub, answered by this thread's `pump`, exactly as
    /// the registry lays it out on this thread; renders on another thread
    /// equal renders here; project fonts and seeds move the generation; and
    /// a restarted engine with main's replay lays text out the same.
    fn render_threads(main: &mut Main) {
        use pm_render::text::TextFonts;
        use pm_render::text::served::ServedFonts;
        use pm_text::compat::{FontCacheSeed, app_ui_font_cache_seeds};

        use pm_text::served::FontServer;
        type Glyphs = Vec<(u32, u64, u64, String)>;
        fn glyphs(
            fonts: &mut dyn FnMut(pm_text::font::FaceId) -> String,
            lines: &[pm_text::compat::LineRuns],
        ) -> Glyphs {
            let mut out = Vec::new();
            for l in lines {
                for r in &l.runs {
                    for g in &r.glyphs {
                        out.push((g.id, g.x.to_bits(), g.y.to_bits(), String::new()));
                    }
                    out.push((u32::MAX, l.width.to_bits(), 0, fonts(r.face)));
                }
            }
            out
        }
        let pump_until = |main: &mut Main, done: &dyn Fn() -> bool| {
            let started = std::time::Instant::now();
            while !done() {
                main.host.pump();
                std::thread::sleep(std::time::Duration::from_micros(200));
                assert!(
                    started.elapsed().as_secs() < 60,
                    "render thread never finished"
                );
            }
        };
        let contents = vec![
            // The real project's case (amendment 19): system-ui at 24 px.
            TextContent {
                text: "system-ui 24 px measured as the editor does".into(),
                font: Some("system-ui".into()),
                size: Some(24.0),
                // Text layers default to 500; the UI's entries are 400.
                weight: Some(400.0),
                ..Default::default()
            },
            TextContent {
                text: "Mixed 中文 العربية עברית हिन्दी 😀🇰🇷 text".into(),
                font: Some("Helvetica".into()),
                size: Some(31.5),
                weight: Some(700.0),
                ..Default::default()
            },
            TextContent {
                text: "Hidden system font, bold italic".into(),
                font: Some("system-ui".into()),
                size: Some(18.0),
                weight: Some(700.0),
                italic: Some(true),
                ..Default::default()
            },
        ];
        let seeds = app_ui_font_cache_seeds(&[2.0]);
        main.json(
            Op::Seeds,
            &serde_json::json!({ "seeds": seeds }).to_string(),
            &[],
        );
        assert!(
            main.error(
                Op::Seeds,
                r#"{"seeds":[{"family":"","weight":400,"italic":false,"size":1,"specified":1}]}"#,
                &[]
            )
            .contains("invalid family")
        );
        let hub = main.host.font_hub().clone();
        assert!(hub.stats().open);
        assert_eq!(hub.stats().seeds, 2);

        // The reference: the registry on this thread with the same seeds.
        let mut reference = Compat::new(pm_text::registry::CoreTextRegistry::new());
        reference.set_font_cache_seeds(seeds.clone());
        let want: Vec<Glyphs> = contents
            .iter()
            .map(|c| {
                let lines = reference.line_runs(c);
                let mut names = |id| {
                    use pm_text::font::FontResolver;
                    reference.resolver().face(id).postscript_name.clone()
                };
                glyphs(&mut names, &lines)
            })
            .collect();

        let thread = {
            let hub = hub.clone();
            let contents = contents.clone();
            std::thread::spawn(move || {
                let mut fonts = ServedFonts::new(hub.clone(), hub.clone());
                let out: Vec<Glyphs> = contents
                    .iter()
                    .map(|c| {
                        fonts.sync();
                        let lines = fonts.line_runs(c);
                        // Full fonts with outlines, not subsets.
                        for r in lines.iter().flat_map(|l| &l.runs) {
                            assert!(fonts.face(r.face).data.as_slice().len() > 10_000);
                        }
                        let mut names = |id| fonts.face(id).postscript_name.clone();
                        glyphs(&mut names, &lines)
                    })
                    .collect();
                assert!(fonts.take_failures().is_empty());
                (out, fonts.seeds(), fonts.round_trips())
            })
        };
        pump_until(main, &|| thread.is_finished());
        let (got, used_seeds, trips) = thread.join().unwrap();
        assert_eq!(used_seeds, seeds);
        for (i, (a, b)) in got.iter().zip(&want).enumerate() {
            assert_eq!(
                a, b,
                "case {i}: a render thread lays text out as the registry does"
            );
        }
        eprintln!(
            "render thread: {} layouts in {trips} round trips",
            got.len()
        );

        // Seeds change the system-ui layout (the editor's 2x state), and
        // without them the hub gives the fresh renderer's answer.
        let fresh: Glyphs = {
            let mut c = Compat::new(pm_text::registry::CoreTextRegistry::new());
            let lines = c.line_runs(&contents[0]);
            let mut names = |id| {
                use pm_text::font::FontResolver;
                c.resolver().face(id).postscript_name.clone()
            };
            glyphs(&mut names, &lines)
        };
        assert_ne!(fresh, want[0], "the seeds matter for system-ui at 24 px");

        // A whole frame drawn on another thread equals the frame drawn here.
        let doc = text_document();
        let render = |r: &mut pm_render::Renderer| {
            let request = pm_render::RenderRequest::sized(
                pm_protocol::common::Size::new(960, 540),
                pm_protocol::color::Precision::Float16,
            );
            let f = r
                .render(
                    &doc,
                    "Ptest-root",
                    pm_protocol::common::Ticks::new(0).unwrap(),
                    &request,
                    &NoMedia,
                )
                .expect("text renders");
            r.read_rgba(&f).unwrap()
        };
        let mut here = pm_render::Renderer::headless().unwrap();
        let mut measure =
            pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new());
        measure.compat().set_font_cache_seeds(seeds.clone());
        here.set_text_fonts(Some(Box::new(OnThisThread(measure))));
        assert!(
            here.unsupported_with_media(&doc, "Ptest-root", &NoMedia)
                .is_empty()
        );
        let expected = render(&mut here);
        let thread = {
            let hub = hub.clone();
            let doc = doc.clone();
            std::thread::spawn(move || {
                let mut r = pm_render::Renderer::headless().unwrap();
                r.set_text_fonts(Some(Box::new(ServedFonts::new(hub.clone(), hub.clone()))));
                let issues = r.unsupported_with_media(&doc, "Ptest-root", &NoMedia);
                assert!(issues.is_empty(), "{issues:?}");
                let request = pm_render::RenderRequest::sized(
                    pm_protocol::common::Size::new(960, 540),
                    pm_protocol::color::Precision::Float16,
                );
                let f = r
                    .render(
                        &doc,
                        "Ptest-root",
                        pm_protocol::common::Ticks::new(0).unwrap(),
                        &request,
                        &NoMedia,
                    )
                    .expect("text renders on a render thread");
                let first = r.read_rgba(&f).unwrap();
                (first, r)
            })
        };
        pump_until(main, &|| thread.is_finished());
        let (got, mut threaded) = thread.join().unwrap();
        assert_eq!(got.len(), expected.len());
        assert!(
            got == expected,
            "a frame drawn on a render thread equals the frame drawn here"
        );
        assert!(
            got.chunks_exact(8).any(|p| p != &got[..8]),
            "the frame has ink"
        );

        // A project font moves the generation: the next layer starts over
        // with a fresh measurer, and the raster cache key changes.
        let g0 = hub.generation();
        let font = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").unwrap();
        main.json(
            Op::AddProjectFont,
            r#"{"family":"Helvetica","weight":"700","style":"normal"}"#,
            &font,
        );
        assert!(hub.generation() > g0);
        let doc2 = doc.clone();
        let thread = std::thread::spawn(move || {
            let request = pm_render::RenderRequest::sized(
                pm_protocol::common::Size::new(960, 540),
                pm_protocol::color::Precision::Float16,
            );
            let f = threaded
                .render(
                    &doc2,
                    "Ptest-root",
                    pm_protocol::common::Ticks::new(0).unwrap(),
                    &request,
                    &NoMedia,
                )
                .expect("text renders after a font change");
            threaded.read_rgba(&f).unwrap()
        });
        pump_until(main, &|| thread.is_finished());
        let after = thread.join().unwrap();
        assert!(
            after != expected,
            "the project font (Arial for Helvetica bold) is drawn"
        );
        main.json(Op::ClearProjectFonts, "{}", &[]);

        // Different seeds move the generation too.
        let g1 = hub.generation();
        let one: Vec<FontCacheSeed> = app_ui_font_cache_seeds(&[1.0]);
        main.json(
            Op::Seeds,
            &serde_json::json!({ "seeds": one }).to_string(),
            &[],
        );
        assert!(hub.generation() > g1);
        main.json(
            Op::Seeds,
            &serde_json::json!({ "seeds": seeds }).to_string(),
            &[],
        );

        // Each window is its own renderer: an editor window only ever on a
        // 1x display and a popout on a 2x display measure system-ui 24 px
        // differently, and so does the engine for each window's views and
        // for the export main opened a session for. The fallback (every
        // window's seeds) applies only to work of no known window.
        let mut both = seeds.clone();
        both.extend(one.clone());
        main.json(
            Op::Seeds,
            &serde_json::json!({
                "seeds": both,
                "owners": [{ "owner": 1, "seeds": one }, { "owner": 2, "seeds": seeds }],
                "sessions": [{ "session": 40, "owner": 1 }],
            })
            .to_string(),
            &[],
        );
        main.host.attach(30, "renderer", Some(1)).unwrap();
        main.host.attach(31, "renderer", Some(2)).unwrap();
        let thread = {
            let hub = hub.clone();
            let content = contents[0].clone();
            std::thread::spawn(move || {
                use pm_render::text::served::SeedScope;
                let mut fonts = ServedFonts::new(hub.clone(), hub.clone());
                let layout = |fonts: &mut ServedFonts, scope| {
                    fonts.set_seed_scope(scope);
                    fonts.sync();
                    let lines = fonts.line_runs(&content);
                    let mut names = |id| fonts.face(id).postscript_name.clone();
                    (glyphs(&mut names, &lines), fonts.seeds())
                };
                let export = SeedScope {
                    port: None,
                    session: Some(40),
                };
                let out = [
                    layout(&mut fonts, SeedScope::port(30)),
                    layout(&mut fonts, SeedScope::port(31)),
                    layout(&mut fonts, export),
                    // Back to the first window: its parked measurer.
                    layout(&mut fonts, SeedScope::port(30)),
                ];
                (out, fonts.rebuilds())
            })
        };
        pump_until(main, &|| thread.is_finished());
        let (per_window, rebuilds) = thread.join().unwrap();
        assert_eq!(per_window[0], (fresh.clone(), one.clone()), "the 1x window");
        assert_eq!(
            per_window[1],
            (want[0].clone(), seeds.clone()),
            "the 2x window"
        );
        assert_eq!(
            per_window[2],
            (fresh.clone(), one.clone()),
            "the 1x window's export"
        );
        assert_eq!(per_window[3], per_window[0]);
        assert_eq!(
            rebuilds, 0,
            "switching windows parks measurers, never rebuilds"
        );
        main.host.detach(30);
        main.host.detach(31);
        main.json(
            Op::Seeds,
            &serde_json::json!({ "seeds": seeds }).to_string(),
            &[],
        );

        // A restarted engine: main opens the new port above every generation
        // it has seen and replays the seeds; text lays out as before.
        let mut restarted = Main {
            host: EngineHost::new(EngineProcess::Job).unwrap(),
            port: 7,
            next: 0,
        };
        restarted.host.attach(7, FONTS_PEER, None).unwrap();
        let base = hub.generation() + 100;
        restarted.json(
            Op::Open,
            &serde_json::json!({ "generationBase": base }).to_string(),
            &[],
        );
        restarted.json(
            Op::Seeds,
            &serde_json::json!({ "seeds": seeds }).to_string(),
            &[],
        );
        let hub2 = restarted.host.font_hub().clone();
        let content = contents[0].clone();
        let thread = std::thread::spawn(move || {
            let mut fonts = ServedFonts::new(hub2.clone(), hub2.clone());
            let lines = fonts.line_runs(&content);
            let mut names = |id| fonts.face(id).postscript_name.clone();
            glyphs(&mut names, &lines)
        });
        pump_until(&mut restarted, &|| thread.is_finished());
        assert_eq!(
            thread.join().unwrap(),
            want[0],
            "a restarted engine lays out as before"
        );

        // Shutdown ends a render thread's wait instead of hanging it.
        let hub3 = restarted.host.font_hub().clone();
        let waiting = std::thread::spawn(move || {
            let mut fonts = ServedFonts::new(hub3.clone(), hub3.clone());
            let _ = fonts.line_runs(&TextContent {
                text: "never answered".into(),
                font: Some("Avenir Next".into()),
                size: Some(77.0),
                ..Default::default()
            });
            fonts.take_failures()
        });
        while !restarted.host.font_hub().has_requests() {
            std::thread::yield_now();
        }
        drop(restarted);
        let failures = waiting.join().unwrap();
        assert!(
            failures.iter().any(|f| f.contains("shutting down")),
            "{failures:?}"
        );
    }

    /// A document with text in three styles (system-ui seeds, a hidden
    /// system font, emoji, a variable axis).
    fn text_document() -> pm_model::Document {
        let ch = |v: serde_json::Value| serde_json::json!({ "v": v, "kf": [], "expr": null });
        let layer = |id: &str, d: serde_json::Value, x: f64, y: f64| {
            serde_json::json!({
                "id": id, "type": "text", "name": id, "from": 0, "dur": 5, "on": true, "lock": false,
                "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
                "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
                "p": {
                    "perspective": ch(50.into()), "position.z": ch(0.into()), "anchor.z": ch(0.into()),
                    "scale.z": ch(100.into()), "rotation.x": ch(0.into()), "rotation.y": ch(0.into()),
                    "orientation.x": ch(0.into()), "orientation.y": ch(0.into()), "orientation.z": ch(0.into()),
                    "anchor.x": ch(0.into()), "anchor.y": ch(0.into()), "position.x": ch(x.into()),
                    "position.y": ch(y.into()), "scale.x": ch(100.into()), "scale.y": ch(100.into()),
                    "rotation": ch(0.into()), "opacity": ch(100.into()), "skew": ch(0.into())
                },
                "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d, "locked_intent": {}
            })
        };
        let v = serde_json::json!({
            "id": "Ptest", "name": "test", "w": 960, "h": 540, "fps": 30, "dur": 5, "bg": "#000000",
            "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
            "compId": "Ptest-root", "compName": "test", "markers": [], "work": [0, 5], "params": {},
            "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "assets": {}, "comps": {},
            "layers": [
                layer("a", serde_json::json!({"text": "system-ui at 24 px", "font": "system-ui", "weight": 400, "size": 24, "color": "#ffffff", "align": "center"}), 480.0, 120.0),
                layer("b", serde_json::json!({"text": "Helvetica Bold 😀 中文", "font": "Helvetica", "weight": 700, "size": 40, "color": "#ffd43b", "align": "center"}), 480.0, 270.0),
                layer("c", serde_json::json!({"text": "Variable axis", "font": "Avenir Next", "size": 36, "color": "#7fdbff", "align": "center", "fontAxis.wght": 650}), 480.0, 420.0)
            ]
        });
        pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1)
    }

    struct NoMedia;
    impl pm_render::AssetResolver for NoMedia {
        fn path(&self, _: &str, _: Option<&pm_model::Node>) -> Option<std::path::PathBuf> {
            None
        }
    }

    /// The registry on this (the main) thread, for the reference render.
    struct OnThisThread(pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>);
    // SAFETY: created, used, and dropped on the main thread only.
    unsafe impl Send for OnThisThread {}
    impl pm_render::text::TextFonts for OnThisThread {
        fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
            &mut self.0
        }
        fn line_runs(&mut self, d: &TextContent) -> Vec<pm_text::compat::LineRuns> {
            self.0.compat().line_runs(d)
        }
        fn text_runs(
            &mut self,
            d: &TextContent,
            text: &str,
            tracking: f64,
        ) -> Vec<pm_text::compat::GlyphRun> {
            self.0.compat().text_runs(d, text, tracking).0
        }
        fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
            use pm_text::font::FontResolver;
            self.0.compat().resolver().face(id)
        }
        fn em_advance(&mut self, face: pm_text::font::FaceId, size: f32) -> Option<f32> {
            use pm_text::font::FontResolver;
            self.0.compat().resolver_mut().em_advance(face, size)
        }
    }

    let bits = |v: f32| v.to_bits();
    let family_query = |name: &str, size: f32| Query::Family {
        name: name.into(),
        weight: bits(400.0),
        italic: false,
        size: bits(size),
    };

    // Both engines serve fonts (the job engine for its exports and agent
    // frames).
    let mut job = EngineHost::new(EngineProcess::Job).unwrap();
    assert!(job.attach(9, FONTS_PEER, None).unwrap().is_empty());
    drop(job);

    let mut main = Main {
        host: EngineHost::new(EngineProcess::Interactive).unwrap(),
        port: 4,
        next: 0,
    };
    assert!(main.host.attach(4, FONTS_PEER, None).unwrap().is_empty());
    let status: serde_json::Value = serde_json::from_str(&main.host.status_json()).unwrap();
    assert_eq!(status["fontPorts"], 1);

    // Open: the generation starts at main's base.
    assert_eq!(
        main.json(Op::Open, r#"{"generationBase":40}"#, &[])["generation"],
        40
    );
    assert_eq!(main.json(Op::Generation, "{}", &[])["generation"], 40);

    // Answers for the families a document names, then their subsets.
    let queries = vec![
        family_query("Helvetica", 24.0),
        family_query("Menlo", 13.0),
        family_query("system-ui", 17.0),
    ];
    let started = std::time::Instant::now();
    let (status, raw) = main.call(
        Op::Answer,
        &serde_json::json!({ "queries": queries }).to_string(),
        &[],
    );
    let answer_ms = started.elapsed().as_secs_f64() * 1e3;
    assert_eq!(status, Status::Json);
    // Main reads the generation off the end of the reply (`answerGeneration`).
    assert!(raw.ends_with(br#","generation":40}"#));
    let reply: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(reply["generation"], 40);
    let answers: Answers = serde_json::from_value(reply).unwrap();
    assert!(answers.faces.len() >= 3, "{:?}", answers.faces);
    let mut replay = ReplayResolver::new(&answers);
    let mut subset_bytes = 0;
    let started = std::time::Instant::now();
    for (blob, index) in answers.sources() {
        let (status, bytes) = main.call(
            Op::Subset,
            &serde_json::json!({ "blob": blob, "index": index }).to_string(),
            &[],
        );
        assert_eq!(status, Status::Bytes);
        subset_bytes += bytes.len();
        replay.add_font(&blob, index, FontBytes::new(bytes), 0);
    }
    let subset_ms = started.elapsed().as_secs_f64() * 1e3;

    // pm-wasm's layout over the subsets equals the engine's own layout over
    // the full fonts (what the engine renders).
    let content = TextContent {
        text: "Powermove 1.1 — fonts".into(),
        font: Some("Helvetica".into()),
        size: Some(24.0),
        ..Default::default()
    };
    let mut wasm_side = Compat::new(replay);
    // The replay may still miss fallback queries for this string; loop as
    // the renderer does until it converges.
    for _ in 0..6 {
        wasm_side.resolver_mut().clear_reports();
        let _ = wasm_side.line_runs(&content);
        let misses = std::mem::take(&mut wasm_side.resolver_mut().misses);
        let pending: Vec<(String, u32)> =
            wasm_side.resolver().pending_blobs.iter().cloned().collect();
        if misses.is_empty() && pending.is_empty() {
            break;
        }
        for (blob, index) in pending {
            let (_, bytes) = main.call(
                Op::Subset,
                &serde_json::json!({ "blob": blob, "index": index }).to_string(),
                &[],
            );
            wasm_side
                .resolver_mut()
                .add_font(&blob, index, FontBytes::new(bytes), 0);
        }
        if misses.is_empty() {
            wasm_side.reset_session();
            continue;
        }
        let reply = main.json(
            Op::Answer,
            &serde_json::json!({ "queries": misses, "known": 40 }).to_string(),
            &[],
        );
        let more: Answers = serde_json::from_value(reply).unwrap();
        wasm_side.resolver_mut().add_answers(&more);
        for (blob, index) in more.sources() {
            let (_, bytes) = main.call(
                Op::Subset,
                &serde_json::json!({ "blob": blob, "index": index }).to_string(),
                &[],
            );
            wasm_side
                .resolver_mut()
                .add_font(&blob, index, FontBytes::new(bytes), 0);
        }
        wasm_side.reset_session();
    }
    let ours = wasm_side.line_runs(&content);
    let engine_runs = main.host.fonts().unwrap().line_runs(&content);
    assert_eq!(ours.len(), engine_runs.len());
    for (a, b) in ours.iter().zip(&engine_runs) {
        assert_eq!(a.width.to_bits(), b.width.to_bits());
        let glyphs = |l: &pm_text::compat::LineRuns| {
            l.runs
                .iter()
                .flat_map(|r| {
                    r.glyphs
                        .iter()
                        .map(|g| (g.id, g.x.to_bits(), g.y.to_bits()))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(glyphs(a), glyphs(b));
    }
    // The engine renders from the full font: the face has outlines.
    let face = engine_runs[0].runs[0].face;
    let info = main.host.fonts().unwrap().face(face).unwrap();
    assert!(info.data.as_slice().len() > 10_000);
    assert!(main.host.fonts().unwrap().face(u32::MAX).is_none());

    // A project font changes the generation; answers name it.
    let font = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf")
        .or_else(|_| std::fs::read("/System/Library/Fonts/Helvetica.ttc"))
        .unwrap();
    let g = main.json(
        Op::AddProjectFont,
        r#"{"family":"Brand Sans","weight":"400","style":"normal"}"#,
        &font,
    )["generation"]
        .as_u64()
        .unwrap();
    assert_eq!(g, 41);
    let reply = main.json(
        Op::Answer,
        &serde_json::json!({ "queries": [family_query("Brand Sans", 24.0)], "known": 41 })
            .to_string(),
        &[],
    );
    let project: Answers = serde_json::from_value(reply).unwrap();
    assert!(
        project.faces.iter().any(|f| f.custom),
        "{:?}",
        project.faces
    );
    assert_eq!(
        main.json(Op::SetFontSource, r#"{"family":"Brand Sans"}"#, &font)["generation"],
        42
    );
    assert_eq!(
        main.json(Op::ClearProjectFonts, "{}", &[])["generation"],
        43
    );
    let names = main.json(Op::FamilyNames, "{}", &[]);
    assert!(names.as_array().unwrap().iter().any(|n| n == "Helvetica"));

    // Level 2 inputs.
    let faces = main.json(Op::FamilyFaces, r#"{"family":"Helvetica"}"#, &[]);
    assert!(!faces.as_array().unwrap().is_empty());

    // Refusals: bad arguments, unknown faces from a current requester, data
    // that is not a font, and malformed messages.
    assert!(
        main.error(Op::Subset, r#"{"blob":"../x","index":0}"#, &[])
            .contains("invalid subset key")
    );
    assert!(
        !main
            .error(Op::AddProjectFont, r#"{"family":"Bad"}"#, b"not a font")
            .is_empty()
    );
    assert!(
        main.error(Op::Answer, r#"{"queries":[],"extra":1}"#, &[])
            .contains("invalid font request arguments")
    );
    let stale = Query::Fallback {
        weight: bits(400.0),
        italic: false,
        size: bits(24.0),
        ch: 0x4e00,
        primary: 9999,
        emoji: false,
    };
    assert!(
        main.error(
            Op::Answer,
            &serde_json::json!({ "queries": [stale.clone()], "known": 43 }).to_string(),
            &[]
        )
        .contains("unknown face")
    );
    let (status, _) = {
        let actions = main.host.receive(4, &[1, 2]);
        let [Action::Send { bytes, .. }] = actions.as_slice() else {
            panic!()
        };
        let r = wire::decode_reply(bytes).unwrap();
        (r.status, ())
    };
    assert_eq!(status, Status::Error);

    // A requester holding an earlier generation (a restarted engine's main
    // replaying, or a renderer that missed a change) gets the queries it can
    // still ask answered and the new generation, never an error.
    let reply = main.json(
        Op::Answer,
        &serde_json::json!({ "queries": [stale, family_query("Helvetica", 12.0)], "known": 7 })
            .to_string(),
        &[],
    );
    assert_eq!(reply["generation"], 43);
    assert_eq!(reply["answers"].as_array().unwrap().len(), 1);

    // A second font port (main reconnecting) shares the launch's registry.
    main.host.attach(5, FONTS_PEER, None).unwrap();
    assert!(main.host.detach(4).is_empty());
    main.port = 5;
    assert_eq!(main.json(Op::Generation, "{}", &[])["generation"], 43);

    // Messages on a detached font port go nowhere near the protocol engine.
    assert!(
        main.host
            .receive(4, &wire::encode_request(1, Op::Generation, "{}", &[]))
            .iter()
            .all(|a| !matches!(a, Action::Send { .. }))
    );

    render_threads(&mut main);

    eprintln!(
        "font service: answer (3 families) {answer_ms:.2} ms, {} subsets {:.1} KB in {subset_ms:.2} ms",
        answers.sources().len(),
        subset_bytes as f64 / 1024.0
    );
    eprintln!("ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    let mut host =
        pm_node::EngineHost::new(pm_protocol::error::EngineProcess::Interactive).unwrap();
    assert!(host.attach(1, pm_node::FONTS_PEER, None).is_err());
}
