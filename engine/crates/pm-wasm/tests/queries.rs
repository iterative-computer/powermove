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
