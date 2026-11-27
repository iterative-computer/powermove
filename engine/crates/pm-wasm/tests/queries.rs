//! The evaluation answers beyond the kernel's surface (`src/queries.rs`),
//! which `pm-cli parity eval --wasm` compares with the corpus.

use pm_model::Value;
use pm_model::json::parse;
use pm_wasm::Engine;

const PROJECT: &str = r##"{
  "id": "P1", "name": "Test", "compId": "C1", "revision": 0, "assets": {}, "params": {},
  "comps": {
    "C1": { "id": "C1", "name": "Main", "w": 1920, "h": 1080, "fps": 30, "dur": 10,
      "layers": [
        { "id": "A", "type": "solid", "name": "A", "from": 0, "dur": 10, "on": true, "parent": null,
          "p": { "skew": { "v": 0, "kf": [], "expr": "1 +" }, "opacity": { "v": 100, "kf": [], "expr": "value / 2" } },
          "d": { "color": "#ff0000", "w": 200, "h": 100, "radius": 0 }, "fx": [], "masks": [] },
        { "id": "N", "type": "precomp", "name": "N", "from": 1, "dur": 5, "on": true, "parent": null, "p": {},
          "d": { "comp": "C2", "w": 640, "h": 360, "trim": 0.5, "speed": 2 }, "fx": [], "masks": [] },
        { "id": "V", "type": "precomp", "name": "V", "from": 0, "dur": 5, "on": true, "parent": null, "p": {},
          "d": { "comp": "C2", "w": 640, "h": 360, "trim": 0,
                 "speed": { "v": 1, "kf": [ { "t": 0, "v": 1, "i": "s1" }, { "t": 2, "v": 3, "i": "s2" } ], "expr": null } },
          "fx": [], "masks": [] },
        { "id": "T", "type": "text", "name": "T", "from": 0, "dur": 10, "on": true, "parent": null, "p": {},
          "d": { "text": "Hi", "font": "Helvetica", "size": 40, "color": "#ffffff" }, "fx": [], "masks": [] }
      ] },
    "C2": { "id": "C2", "name": "Nested", "w": 640, "h": 360, "fps": 24, "dur": 3, "layers": [] }
  }
}"##;

fn engine() -> Engine {
    let mut e = Engine::new();
    e.resync(PROJECT.as_bytes(), 1).unwrap();
    e
}

fn q(e: &mut Engine, request: &str) -> Value {
    let reply = e.query(request.as_bytes());
    parse(&reply.text).unwrap_or_else(|err| panic!("{err}: {}", reply.text))
}

#[test]
fn expression_errors_follow_evaluation() {
    let mut e = engine();
    let ask = r#"{"op":"expressionError","comp":"C1","layerId":"A","key":"skew"}"#;
    assert_eq!(
        q(&mut e, ask).get("ok"),
        Some(&Value::Null),
        "nothing evaluated yet"
    );
    q(
        &mut e,
        r#"{"op":"ev","comp":"C1","layer":0,"key":"skew","t":0}"#,
    );
    assert_eq!(
        q(&mut e, ask).get("ok").and_then(Value::as_str),
        Some("Expression syntax is invalid.")
    );
    q(
        &mut e,
        r#"{"op":"ev","comp":"C1","layer":0,"key":"opacity","t":0}"#,
    );
    let ok = r#"{"op":"expressionError","comp":"C1","layerId":"A","key":"opacity"}"#;
    assert_eq!(q(&mut e, ok).get("ok"), Some(&Value::Null));
}

#[test]
fn source_time_has_both_integration_steps() {
    let mut e = engine();
    // Constant speed: local × speed + trim.
    let r = q(&mut e, r#"{"op":"sourceTime","comp":"C1","layer":1,"t":2}"#);
    assert_eq!(r.get("ok").and_then(Value::as_f64), Some(2.5));
    // Variable speed: both steps integrate ∫ speed; at t = 2 the speed ramps
    // 1 → 3 linearly, so the source time is close to 4.
    for retiming in ["engine", "legacy"] {
        let r = q(
            &mut e,
            &format!(
                r#"{{"op":"sourceTime","comp":"C1","layer":2,"t":2,"retiming":"{retiming}"}}"#
            ),
        );
        let v = r.get("ok").and_then(Value::as_f64).unwrap();
        assert!((v - 4.0).abs() < 0.05, "{retiming}: {v}");
    }
}

#[test]
fn text_queries_ask_for_fonts_and_scenes_answer() {
    let mut e = engine();
    for op in ["glyphLayout", "textRasterGeometry", "textAnimation"] {
        let r = q(
            &mut e,
            &format!(r#"{{"op":"{op}","comp":"C1","layer":3,"t":0}}"#),
        );
        assert_eq!(r.get("fonts"), Some(&Value::Bool(true)), "{op}");
    }
    let r = q(
        &mut e,
        r#"{"op":"compositionScene","comp":"C1","open":"C1","t":0}"#,
    );
    assert!(
        r.get("ok").is_some_and(|s| s.get("objects").is_some()),
        "{r:?}"
    );
    let r = q(&mut e, r#"{"op":"captionBounds","comp":"C1","t":0}"#);
    assert!(r.get("throw").is_some(), "no layer: {r:?}");
}

#[test]
fn cold_level2_queries_request_fonts_before_reporting_geometry_errors() {
    let mut e = Engine::new();
    e.resync(
        PROJECT
            .replace(
                "\"id\": \"T\", \"type\": \"text\"",
                "\"id\": \"T\", \"type\": \"text\", \"compat\": 2",
            )
            .as_bytes(),
        1,
    )
    .unwrap();
    for op in [
        "glyphLayout",
        "textLayout",
        "textRasterGeometry",
        "textAnimation",
        "bounds",
        "textRaster",
        "pick",
    ] {
        let r = q(
            &mut e,
            &format!(r#"{{"op":"{op}","comp":"C1","layer":3,"t":0,"x":0,"y":0}}"#),
        );
        assert_eq!(r.get("fonts"), Some(&Value::Bool(true)), "{op}: {r:?}");
    }
}

#[test]
fn nested_paths_and_replacement_scopes_are_native() {
    let mut e = Engine::new();
    e.resync(br#"{"id":"P","compId":"main","comps":{"main":{"fps":30,"layers":[]},"outer":{"comps":{"same":{"fps":24,"layers":[{"id":"L","from":0,"p":{"x":{"v":7}}}]}}},"same":{"fps":30,"layers":[{"id":"L","from":0,"p":{"x":{"v":99}}}]}}}"#, 1).unwrap();
    assert_eq!(q(&mut e, r#"{"op":"ev","comp":"@path:[\"comps\",\"outer\",\"comps\",\"same\"]","open":"main","layer":0,"key":"x","t":0}"#).get("ok").and_then(Value::as_f64), Some(7.0));
    assert_eq!(q(&mut e, r#"{"op":"ev","comp":"detached","open":"main","compJson":{"fps":24,"layers":[{"id":"L","from":0,"p":{"x":{"v":12}}}]},"layer":0,"key":"x","t":0}"#).get("ok").and_then(Value::as_f64), Some(12.0));
}

#[test]
fn detached_scopes_are_content_addressed_and_do_not_reset_document_plans() {
    let mut e = engine();
    let id = e
        .intern(&parse(r#"{"comp":"C1","layer":"A","key":"opacity","via":"channel"}"#).unwrap())
        .unwrap();
    let request = |v: i32| {
        format!(
            r#"{{"op":"ev","open":"C1","compJson":{{"fps":24,"layers":[{{"id":"A","from":0,"p":{{"opacity":{{"v":{v},"kf":[],"expr":"value + fps"}}}}}}]}},"layer":0,"key":"opacity","t":0}}"#
        )
    };
    assert_eq!(
        q(&mut e, &request(7)).get("ok").and_then(Value::as_f64),
        Some(31.0)
    );
    for _ in 0..8 {
        assert_eq!(
            q(&mut e, &request(7)).get("ok").and_then(Value::as_f64),
            Some(31.0)
        );
        // Same layer/channel id in the document must retain its own answer.
        assert_eq!(e.ev_id(id, 0.0).1, 50.0);
    }
    assert_eq!(e.stats.detached_misses, 1);
    assert_eq!(e.stats.detached_hits, 8);
    // A mutation outside the document has no patch: the content hash catches it.
    assert_eq!(
        q(&mut e, &request(8)).get("ok").and_then(Value::as_f64),
        Some(32.0)
    );
    assert_eq!(
        q(&mut e, &request(7)).get("ok").and_then(Value::as_f64),
        Some(31.0)
    );
    assert_eq!(e.stats.detached_misses, 2);
    for v in 20..25 {
        q(&mut e, &request(v));
    }
    let misses = e.stats.detached_misses;
    q(&mut e, &request(7));
    assert_eq!(
        e.stats.detached_misses,
        misses + 1,
        "least recently used entry evicted"
    );
    e.resync(PROJECT.as_bytes(), 2).unwrap();
    q(&mut e, &request(7));
    assert_eq!(
        e.stats.detached_misses,
        misses + 2,
        "resync drops detached state"
    );
}

#[test]
fn detached_json_text_is_parsed_once_and_mutations_are_seen() {
    let mut e = engine();
    let ask = |v: i32| {
        serde_json::json!({
        "op":"ev", "open":"C1", "layer":0, "key":"x", "t":0,
        "compJsonText": format!(r#"{{"fps":24,"layers":[{{"id":"A","from":0,"p":{{"x":{{"v":{v}}}}}}}]}}"#)
    }).to_string()
    };
    for _ in 0..5 {
        assert_eq!(
            q(&mut e, &ask(8)).get("ok").and_then(Value::as_f64),
            Some(8.0)
        );
    }
    assert_eq!(e.stats.detached_misses, 1);
    assert_eq!(e.stats.detached_hits, 4);
    assert_eq!(
        q(&mut e, &ask(9)).get("ok").and_then(Value::as_f64),
        Some(9.0)
    );
    assert_eq!(e.stats.detached_misses, 2);
    assert!(
        q(
            &mut e,
            r#"{"op":"ev","open":"C1","compJsonText":"{invalid"}"#
        )
        .get("throw")
        .is_some()
    );
}

#[test]
fn tracked_scope_reports_reads_and_accepts_only_changed_entries() {
    let mut e = engine();
    let initial = r#"{"op":"ev","scopeId":"test-scope","comp":"@detached","open":"C1","layer":0,"key":"x","t":0,"compJson":{"id":"S","fps":24,"layers":[{"id":"A","from":0,"p":{"x":{"v":8,"kf":[]}}}]}}"#;
    let answer = q(&mut e, initial);
    assert_eq!(answer.get("ok").and_then(Value::as_f64), Some(8.0));
    let reads = answer.get("reads").unwrap().as_array().unwrap();
    assert!(reads.contains(&parse(r#"["layers",0,"p","x","v"]"#).unwrap()));
    let next = r#"{"op":"ev","scopeId":"test-scope","comp":"@detached","open":"C1","layer":0,"key":"x","t":1}"#;
    assert_eq!(q(&mut e, next).get("ok").and_then(Value::as_f64), Some(8.0));
    let patched = next.replace(
        "\"t\":1",
        r#""t":1,"scopePatches":[{"path":["layers",0,"p","x","v"],"exists":true,"value":27}]"#,
    );
    assert_eq!(
        q(&mut e, &patched).get("ok").and_then(Value::as_f64),
        Some(27.0)
    );
    assert_eq!(e.stats.detached_misses, 1);
    assert_eq!(e.stats.detached_hits, 2);
    e.resync(PROJECT.as_bytes(), 2).unwrap();
    assert_eq!(
        q(&mut e, next)
            .get("throw")
            .unwrap()
            .get("message")
            .and_then(Value::as_str),
        Some("detached-scope-missing")
    );
}
