//! The engine behind the module's exports, driven natively.

use pm_model::json::parse;
use pm_model::{Value, binary, ops};
use pm_wasm::{ApplyError, Engine};

/// A canonical project: a parent chain, a group, keys, an expression, and a
/// nested composition.
const PROJECT: &str = r##"{
  "id": "P1", "name": "Test", "compId": "C1", "revision": 0, "assets": {}, "params": {},
  "comps": {
    "C1": { "id": "C1", "name": "Main", "w": 1920, "h": 1080, "fps": 30, "dur": 10,
      "layers": [
        { "id": "A", "type": "solid", "name": "A", "from": 0, "dur": 10, "on": true, "parent": null,
          "p": { "position.x": { "v": 100, "kf": [], "expr": null }, "position.y": { "v": 50, "kf": [], "expr": null },
                 "rotation": { "v": 0, "kf": [ { "t": 0, "v": 0, "i": "k1", "speedOut": 0, "influenceOut": 33.333, "speedIn": 0, "influenceIn": 33.333 }, { "t": 2, "v": 90, "i": "k2", "speedOut": 0, "influenceOut": 33.333, "speedIn": 0, "influenceIn": 33.333 } ], "expr": null },
                 "opacity": { "v": 100, "kf": [], "expr": "value / 2" },
                 "scale.x": { "v": 100, "kf": [], "expr": null }, "scale.y": { "v": 100, "kf": [], "expr": null },
                 "skew": { "v": 0, "kf": [], "expr": "1 +" } },
          "d": { "color": "#ff0000", "w": 200, "h": 100, "radius": 0 }, "fx": [], "masks": [] },
        { "id": "B", "type": "solid", "name": "B", "from": 1, "dur": 5, "on": true, "parent": "A",
          "p": { "position.x": { "v": 10, "kf": [], "expr": null }, "position.y": { "v": 20, "kf": [], "expr": null },
                 "scale.x": { "v": 100, "kf": [], "expr": null }, "scale.y": { "v": 100, "kf": [], "expr": null } },
          "d": { "color": "#00ff00", "w": 50, "h": 50, "radius": 0 }, "fx": [ { "id": "F1", "type": "blur", "on": true, "p": { "amount": { "v": 4, "kf": [], "expr": null } } } ], "masks": [] }
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

fn ok(e: &mut Engine, request: &str) -> Value {
    let v = q(e, request);
    v.as_object()
        .and_then(|m| m.get("ok"))
        .cloned()
        .unwrap_or_else(|| panic!("not ok: {}", pm_model::json::write::to_canonical_string(&v)))
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
fn queries_answer_the_layer_questions() {
    let mut e = engine();
    // A rotates 0..90 over 2 s; B is parented to A.
    let world = nums(&ok(
        &mut e,
        r#"{"op":"worldMatrix","comp":"C1","layer":1,"t":2}"#,
    ));
    let a = nums(&ok(
        &mut e,
        r#"{"op":"worldMatrix","comp":"C1","layer":0,"t":2}"#,
    ));
    let local_b = nums(&ok(
        &mut e,
        r#"{"op":"localMatrix","comp":"C1","layer":1,"t":2}"#,
    ));
    let product = nums(&ok(
        &mut e,
        &format!(
            r#"{{"op":"mul","a":[{}],"b":[{}]}}"#,
            a.iter().map(f64::to_string).collect::<Vec<_>>().join(","),
            local_b
                .iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
    ));
    assert_eq!(world, product);
    assert!(
        ok(&mut e, r#"{"op":"active","comp":"C1","layer":1,"t":1.5}"#)
            .as_bool()
            .unwrap()
    );
    assert!(
        !ok(&mut e, r#"{"op":"active","comp":"C1","layer":1,"t":0.5}"#)
            .as_bool()
            .unwrap()
    );
    // Expression: opacity = value / 2.
    assert_eq!(
        ok(
            &mut e,
            r#"{"op":"ev","comp":"C1","layer":0,"key":"opacity","t":0}"#
        )
        .as_f64(),
        Some(50.0)
    );
    // findProp answers a path the renderer resolves on its live layer.
    let path = ok(
        &mut e,
        r#"{"op":"findProp","comp":"C1","layer":1,"key":"F1.amount"}"#,
    );
    assert_eq!(
        pm_model::json::write::to_canonical_string(&path),
        r#"["fx",0,"p","amount"]"#
    );
    let v = ok(
        &mut e,
        r#"{"op":"evP","comp":"C1","layer":1,"path":["fx",0,"p","amount"],"key":"F1.amount","t":3}"#,
    );
    assert_eq!(v.as_f64(), Some(4.0));
    let props = ok(&mut e, r#"{"op":"allProps","comp":"C1","layer":1}"#);
    let keys: Vec<String> = props
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p.as_object()
                .unwrap()
                .get("key")
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert!(keys.contains(&"F1.amount".to_owned()));
    assert!(keys.contains(&"position.x".to_owned()));
    // A detached layer (api.model.mkLayer) is evaluated from its JSON.
    let detached = ok(
        &mut e,
        r#"{"op":"ev","comp":"C1","layer":{"id":"X","type":"null","from":0,"dur":1,"p":{"position.x":{"v":7,"kf":[],"expr":null}}},"key":"position.x","t":0}"#,
    );
    assert_eq!(detached.as_f64(), Some(7.0));
    // Parenting cycles.
    assert!(
        ok(
            &mut e,
            r#"{"op":"wouldCycle","comp":"C1","layer":0,"parent":"B"}"#
        )
        .as_bool()
        .unwrap()
    );
    assert!(
        !ok(
            &mut e,
            r#"{"op":"wouldCycle","comp":"C1","layer":1,"parent":null}"#
        )
        .as_bool()
        .unwrap()
    );
    // Picking: B is at A's position plus its own offset, 50 px square.
    let hit = ok(&mut e, r#"{"op":"pick","comp":"C1","x":100,"y":50,"t":0}"#);
    assert_eq!(nums(&hit), vec![0.0]);
    assert!(
        ok(
            &mut e,
            r#"{"op":"pick","comp":"C1","x":1900,"y":1000,"t":0}"#
        )
        .is_null()
    );
    let b = ok(&mut e, r#"{"op":"bounds","comp":"C1","layer":0,"t":0}"#);
    assert_eq!(
        b.as_object().unwrap().get("w").unwrap().as_f64(),
        Some(200.0)
    );
    // Key at a time, within half a frame of the open composition's rate.
    assert_eq!(
        ok(
            &mut e,
            r#"{"op":"hasKeyAt","comp":"C1","layer":0,"path":["p","rotation"],"t":2.01}"#
        )
        .as_f64(),
        Some(1.0)
    );
    assert!(
        ok(
            &mut e,
            r#"{"op":"hasKeyAt","comp":"C1","layer":0,"path":["p","rotation"],"t":1}"#
        )
        .is_null()
    );
}

#[test]
fn throws_and_unknown_compositions_are_reported() {
    let mut e = engine();
    let v = q(
        &mut e,
        r#"{"op":"worldMatrix","comp":"NOPE","layer":0,"t":0}"#,
    );
    assert!(v.as_object().unwrap().contains_key("throw"));
    let v = q(&mut e, r#"{"op":"nonsense","comp":"C1"}"#);
    assert!(v.as_object().unwrap().contains_key("throw"));
    let v = e.query(b"{not json");
    assert!(v.text.starts_with("{\"error\""));
}

#[test]
fn special_numbers_are_marked() {
    let mut e = engine();
    let reply = e.query(br#"{"op":"inversePlane","m":[0,0,0,0,0,0,0,0,0]}"#);
    assert_eq!(reply.text, r#"{"ok":null}"#);
    let mut r = pm_wasm::reply::Reply::new();
    r.nums(&[-0.0, f64::NAN, f64::INFINITY, 1.5]);
    assert!(r.special);
    assert_eq!(
        r.text,
        r#"[{"\u0000":"-0"},{"\u0000":"NaN"},{"\u0000":"Infinity"},1.5]"#
    );
}

#[test]
fn patches_apply_and_gaps_are_refused() {
    let mut e = engine();
    let before = parse(PROJECT).unwrap();
    let mut after = before.clone();
    // Move B above A and change A's x.
    {
        let layers = after
            .as_object_mut()
            .unwrap()
            .get_mut("comps")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("C1")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap();
        layers.swap(0, 1);
        let a = layers[1]
            .as_object_mut()
            .unwrap()
            .get_mut("p")
            .unwrap()
            .as_object_mut()
            .unwrap();
        a.insert(
            "position.x",
            parse(r#"{"v":300,"kf":[],"expr":null}"#).unwrap(),
        );
    }
    let list = ops::diff_ops(Some(&before), Some(&after));
    assert!(list.iter().any(|op| matches!(op, ops::Op::Order { .. })));
    let body = binary::encode(&list).unwrap();
    assert_eq!(
        e.apply(&body, 2, 3),
        Err(ApplyError::Gap { have: 1, base: 2 })
    );
    assert_eq!(e.apply(&body, 1, 1), Err(ApplyError::Revision));
    e.apply(&body, 1, 2).unwrap();
    assert_eq!(e.revision(), 2);
    assert_eq!(e.hash(), Some(pm_model::hash::canonical_hash(&after)));
    let m = nums(&ok(
        &mut e,
        r#"{"op":"localMatrix","comp":"C1","layer":1,"t":0}"#,
    ));
    assert_eq!(m[4], 300.0);
    assert!(matches!(e.apply(b"PMP", 2, 3), Err(ApplyError::Decode(_))));
    // An op on an id the mirror lacks means the mirror diverged.
    let bad = binary::encode(&[ops::Op::Delete {
        path: vec![
            pm_model::PathPart::Key("comps".into()),
            pm_model::PathPart::Key("C1".into()),
            pm_model::PathPart::Key("layers".into()),
        ],
        id: "ZZZ".into(),
    }])
    .unwrap();
    assert!(matches!(e.apply(&bad, 2, 3), Err(ApplyError::Mismatch(_))));
    // A resync recovers from anything, at any revision.
    e.resync(PROJECT.as_bytes(), 1).unwrap();
    assert_eq!(e.hash(), Some(pm_model::hash::canonical_hash(&before)));
}

#[test]
fn the_fast_path_answers_by_id() {
    let mut e = engine();
    let id = e
        .intern(&parse(r#"{"comp":"C1","layer":"A","key":"rotation","via":"channel"}"#).unwrap())
        .unwrap();
    let (_, v) = e.ev_id(id, 2.0);
    assert_eq!(v, 90.0);
    let member = e
        .intern(
            &parse(r#"{"comp":"C1","layer":"B","key":"F1.amount","via":["fx","0","p","amount"]}"#)
                .unwrap(),
        )
        .unwrap();
    let mut out = [0.0; 2];
    let mut kinds = [9u8; 2];
    e.ev_batch(&[id.index(), member.index()], 1.0, &mut out, &mut kinds);
    assert_eq!(kinds, [0, 0]);
    assert_eq!(out[1], 4.0);
    // A syntax error is recorded for the channel, as `expressionErrors`.
    let bad = e
        .intern(&parse(r#"{"comp":"C1","layer":"A","key":"skew","via":"channel"}"#).unwrap())
        .unwrap();
    let (_, v) = e.ev_id(bad, 0.0);
    assert_eq!(v, 0.0);
    assert_eq!(
        e.expression_error(bad),
        Some("Expression syntax is invalid.")
    );
    assert_eq!(e.expression_error(id), None);
}

#[test]
fn helpers_without_a_document() {
    let mut e = Engine::new();
    assert_eq!(
        ok(
            &mut e,
            r#"{"op":"countTextUnits","lines":["ab cd","e"],"unit":"words"}"#
        )
        .as_f64(),
        Some(3.0)
    );
    assert!(
        ok(
            &mut e,
            r#"{"op":"expressionDiagnostic","source":"time * 2"}"#
        )
        .is_null()
    );
    assert!(
        ok(&mut e, r#"{"op":"expressionDiagnostic","source":"time *"}"#)
            .as_str()
            .is_some()
    );
    let rate = ok(&mut e, r#"{"op":"frameRate","fps":29.97}"#);
    let m = rate.as_object().unwrap();
    assert_eq!(m.get("num").unwrap().as_f64(), Some(30000.0));
    assert_eq!(m.get("den").unwrap().as_f64(), Some(1001.0));
    assert_eq!(m.get("supported").unwrap().as_bool(), Some(true));
    let odd = ok(&mut e, r#"{"op":"frameRate","fps":12.5}"#);
    assert_eq!(
        odd.as_object().unwrap().get("supported").unwrap().as_bool(),
        Some(false)
    );
    assert!(ok(&mut e, r#"{"op":"frameRate","fps":-1}"#).is_null());
}
