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

    let bits = |v: f32| v.to_bits();
    let family_query = |name: &str, size: f32| Query::Family {
        name: name.into(),
        weight: bits(400.0),
        italic: false,
        size: bits(size),
    };

    // Only the interactive engine serves fonts.
    let mut job = EngineHost::new(EngineProcess::Job).unwrap();
    assert!(job.attach(9, FONTS_PEER).is_err());

    let mut main = Main {
        host: EngineHost::new(EngineProcess::Interactive).unwrap(),
        port: 4,
        next: 0,
    };
    assert!(main.host.attach(4, FONTS_PEER).unwrap().is_empty());
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
    main.host.attach(5, FONTS_PEER).unwrap();
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
    assert!(host.attach(1, pm_node::FONTS_PEER).is_err());
}
