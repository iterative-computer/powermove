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

/// Every export validates its input: a panic would trap the instance in the
/// editor. Truncated, corrupted, and random inputs must come back as errors.
#[test]
fn malformed_input_never_panics() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut e = engine();
    let good = binary::encode(&ops::diff_ops(
        Some(&parse(PROJECT).unwrap()),
        Some(&parse(&PROJECT.replace("\"v\": 100", "\"v\": 101")).unwrap()),
    ))
    .unwrap();
    let requests = [
        r#"{"op":"ev","comp":"C1","layer":0,"key":"rotation","t":1}"#,
        r#"{"op":"localMatrix","comp":"C1","layer":1,"t":0}"#,
        r#"{"op":"countTextUnits","lines":["ab"],"unit":"words"}"#,
    ];
    for round in 0..2_000 {
        // Corrupt a valid patch body: flip bytes, truncate, or append.
        let mut body = good.clone();
        match next() % 3 {
            0 => {
                for _ in 0..1 + next() % 4 {
                    let i = (next() as usize) % body.len();
                    body[i] = next() as u8;
                }
            }
            1 => body.truncate((next() as usize) % body.len()),
            _ => body.extend((0..next() % 16).map(|_| next() as u8)),
        }
        let base = e.revision();
        if e.apply(&body, base, base + 1).is_err() {
            e.resync(PROJECT.as_bytes(), base + 1).unwrap();
        }
        // Corrupted and random queries.
        let mut request = requests[round % requests.len()].as_bytes().to_vec();
        let i = (next() as usize) % request.len();
        request[i] = next() as u8;
        let _ = e.query(&request);
        let noise: Vec<u8> = (0..next() % 64).map(|_| next() as u8).collect();
        let _ = e.query(&noise);
        let _ = e.apply(&noise, e.revision(), e.revision() + 1);
        if round % 50 == 0 {
            let mut doc = PROJECT.as_bytes().to_vec();
            doc.truncate((next() as usize) % doc.len());
            assert!(Engine::new().resync(&doc, 1).is_err() || doc.is_empty());
        }
    }
    // Still answering after all of it.
    e.resync(PROJECT.as_bytes(), e.revision() + 1).unwrap();
    assert_eq!(
        nums(&ok(
            &mut e,
            r#"{"op":"localMatrix","comp":"C1","layer":0,"t":0}"#
        ))[4],
        100.0
    );
}

#[test]
fn font_cache_seeds_are_kept_across_registry_changes() {
    let mut e = engine();
    let seeds = r#"{"seeds":[{"family":"system-ui","weight":400,"italic":false,"size":24,"specified":12}]}"#;
    assert_eq!(e.set_font_seeds(seeds.as_bytes()), Ok(true));
    // The same seeds again (or as a bare array) change nothing.
    assert_eq!(e.set_font_seeds(seeds.as_bytes()), Ok(false));
    let bare = r#"[{"family":"system-ui","weight":400,"italic":false,"size":24,"specified":12}]"#;
    assert_eq!(e.set_font_seeds(bare.as_bytes()), Ok(false));
    // A registry generation change drops answers and faces, not the seeds.
    e.clear_fonts();
    assert_eq!(e.set_font_seeds(seeds.as_bytes()), Ok(false));
    assert_eq!(e.set_font_seeds(b"[]"), Ok(true));
    assert!(e.set_font_seeds(b"{\"seeds\":7}").is_err());
    assert!(e.set_font_seeds(b"not json").is_err());
}

#[test]
fn batched_expression_status_distinguishes_errors_without_reading_messages() {
    let mut e = engine();
    let ids = ["opacity", "skew"].map(|key| {
        e.intern(
            &parse(&format!(
                r#"{{"comp":"C1","layer":"A","key":"{key}","via":"channel"}}"#
            ))
            .unwrap(),
        )
        .unwrap()
        .index()
    });
    let mut values = [0.0; 2];
    let mut kinds = [0u8; 2];
    for time in [0.0, -0.0, 1.0, 2.0, 1.0] {
        e.ev_batch(&ids, time, &mut values, &mut kinds);
        assert_eq!(values, [50.0, 0.0]);
        assert_eq!(kinds, [0, 0x80]);
    }
}

#[test]
fn bound_scalar_plans_keep_revision_type_and_error_guards() {
    let project = |base: &str, source: &str| {
        parse(&format!(
            r#"{{"id":"P","compId":"C","assets":{{}},"comps":{{"C":{{"id":"C","fps":30,"layers":[{{"id":"L","from":0,"p":{{"opacity":{{"v":{base},"kf":[],"expr":"{source}"}}}}}}]}}}}}}"#
        )).unwrap()
    };
    let mut before = project("4", "value + time * 40");
    let mut e = Engine::new();
    e.resync(
        pm_model::json::write::to_canonical_string(&before).as_bytes(),
        1,
    )
    .unwrap();
    let id = e
        .intern(&parse(r#"{"comp":"C","layer":"L","key":"opacity","via":"channel"}"#).unwrap())
        .unwrap();
    let mut values = [0.0];
    let mut kinds = [0];
    for time in [0.0, 1.0, 2.0, -0.0] {
        e.ev_batch(&[id.index()], time, &mut values, &mut kinds);
        assert_eq!(values[0].to_bits(), (4.0 + time * 40.0).to_bits());
        assert_eq!(kinds, [0]);
    }
    for (base, source, time, expected, kind) in [
        ("4", "value / time", 0.0, 4.0, 0x80),
        ("4", "value / time", 1.0, 4.0, 0),
        ("4", "value / time", 2.0, 2.0, 0),
        (r#""4""#, "value + time * 40", 1.0, f64::NAN, 1),
        ("9", "value + time * 40", 1.0, 49.0, 0),
    ] {
        let after = project(base, source);
        let body = binary::encode(&ops::diff_ops(Some(&before), Some(&after))).unwrap();
        let revision = e.revision();
        e.apply(&body, revision, revision + 1).unwrap();
        // Evaluate twice, including the now-warm plan at the same revision.
        for _ in 0..2 {
            e.ev_batch(&[id.index()], time, &mut values, &mut kinds);
            assert_eq!(values[0].to_bits(), expected.to_bits());
            assert_eq!(kinds, [kind]);
        }
        if kind == 1 {
            assert_eq!(
                ok(
                    &mut e,
                    r#"{"op":"ev","comp":"C","layer":0,"key":"opacity","t":1}"#
                )
                .as_str(),
                Some("440")
            );
        }
        before = after;
    }
}

#[test]
fn parameter_patch_does_not_advance_a_slot_that_predates_a_layer_edit() {
    let mut engine = Engine::new();
    let mut before = parse(r#"{"id":"P","compId":"C","params":{"amp":{"value":7}},"comps":{"C":{"id":"C","layers":[{"id":"L","from":0,"p":{"a":{"v":5,"kf":[],"expr":"value + time"}}}]}}}"#).unwrap();
    engine
        .resync(
            pm_model::json::write::to_canonical_string(&before).as_bytes(),
            1,
        )
        .unwrap();
    let id = engine
        .intern(&parse(r#"{"comp":"C","layer":"L","key":"a","via":"channel"}"#).unwrap())
        .unwrap()
        .index();
    let mut values = [0.0];
    let mut kinds = [0];
    engine.ev_batch(&[id], 1.0, &mut values, &mut kinds);
    assert_eq!(values, [6.0]);
    for text in [
        r#"{"id":"P","compId":"C","params":{"amp":{"value":7}},"comps":{"C":{"id":"C","layers":[{"id":"L","from":0,"p":{"a":{"v":19,"kf":[],"expr":"value + time"}}}]}}}"#,
        r#"{"id":"P","compId":"C","params":{"amp":{"value":9}},"comps":{"C":{"id":"C","layers":[{"id":"L","from":0,"p":{"a":{"v":19,"kf":[],"expr":"value + time"}}}]}}}"#,
    ] {
        let after = parse(text).unwrap();
        let body = binary::encode(&ops::diff_ops(Some(&before), Some(&after))).unwrap();
        let base = engine.revision();
        engine.apply(&body, base, base + 1).unwrap();
        before = after;
    }
    engine.ev_batch(&[id], 1.0, &mut values, &mut kinds);
    assert_eq!(values, [20.0]);
    assert_eq!(kinds, [0]);
}

#[test]
fn edits_rebind_only_changed_fixed_inputs_and_keep_layer_local_plans() {
    let project = |amp: i32, fps: i32, opacity: i32, extra: bool, source: &str| {
        parse(&format!(r#"{{"id":"P","compId":"C","params":{{"amp":{{"value":{amp}}}}},"comps":{{"C":{{"id":"C","fps":{fps},"layers":[{}{{"id":"L","from":0,"p":{{"opacity":{{"v":{opacity},"kf":[]}},"a":{{"v":5,"kf":[],"expr":"{source}"}},"b":{{"v":5,"kf":[],"expr":"layer.p.opacity.v + t"}},"c":{{"v":5,"kf":[],"expr":"idx + value"}},"d":{{"v":5,"kf":[],"expr":"fps + value"}},"e":{{"v":5,"kf":[],"expr":"param(\"amp\") + value"}},"f":{{"v":5,"kf":[],"expr":"comp.layers.length + value"}}}}}}]}}}}}}"#,
            if extra { r#"{"id":"head"},"# } else { "" })).unwrap()
    };
    let mut before = project(7, 30, 100, false, "value + time");
    let mut engine = Engine::new();
    engine
        .resync(
            pm_model::json::write::to_canonical_string(&before).as_bytes(),
            1,
        )
        .unwrap();
    let ids: Vec<_> = ["a", "b", "c", "d", "e", "f"]
        .map(|key| {
            engine
                .intern(
                    &parse(&format!(
                        r#"{{"comp":"C","layer":"L","key":"{key}","via":"channel"}}"#
                    ))
                    .unwrap(),
                )
                .unwrap()
                .index()
        })
        .into();
    let mut values = [0.0; 6];
    let mut kinds = [0; 6];
    engine.ev_batch(&ids, 1.0, &mut values, &mut kinds);
    assert_eq!(values, [6.0, 101.0, 5.0, 35.0, 12.0, 6.0]);
    for (amp, fps, opacity, extra, source, changed, expected) in [
        (
            9,
            30,
            100,
            false,
            "value + time",
            1,
            [6.0, 101.0, 5.0, 35.0, 14.0, 6.0],
        ),
        (
            9,
            60,
            100,
            false,
            "value + time",
            1,
            [6.0, 101.0, 5.0, 65.0, 14.0, 6.0],
        ),
        (
            9,
            60,
            8,
            false,
            "value + time",
            1,
            [6.0, 9.0, 5.0, 65.0, 14.0, 6.0],
        ),
        (
            9,
            60,
            8,
            true,
            "value + time",
            2,
            [6.0, 9.0, 6.0, 65.0, 14.0, 7.0],
        ),
        (
            9,
            60,
            8,
            true,
            "value + time * 2",
            1,
            [7.0, 9.0, 6.0, 65.0, 14.0, 7.0],
        ),
        (
            9,
            60,
            8,
            false,
            "value + time * 2",
            2,
            [7.0, 9.0, 5.0, 65.0, 14.0, 6.0],
        ),
    ] {
        let after = project(amp, fps, opacity, extra, source);
        let body = binary::encode(&ops::diff_ops(Some(&before), Some(&after))).unwrap();
        let revision = engine.revision();
        let plans = engine.evaluation_cache_stats().plans;
        engine.apply(&body, revision, revision + 1).unwrap();
        for _ in 0..2 {
            engine.ev_batch(&ids, 1.0, &mut values, &mut kinds);
            assert_eq!(values, expected);
            assert_eq!(kinds, [0; 6]);
        }
        assert_eq!(engine.evaluation_cache_stats().plans - plans, changed);
        before = after;
    }
}

#[test]
fn another_layers_id_edit_cannot_advance_a_local_binding() {
    let project = |first_id: &str| {
        parse(&format!(r#"{{"id":"P","compId":"C","comps":{{"C":{{"id":"C","layers":[{{"id":"{first_id}","from":0,"p":{{"a":{{"v":100,"kf":[],"expr":"value + time"}}}}}},{{"id":"A","from":0,"p":{{"a":{{"v":5,"kf":[],"expr":"value + time"}}}}}}]}}}}}}"#)).unwrap()
    };
    let before = project("B");
    let mut engine = Engine::new();
    engine
        .resync(
            pm_model::json::write::to_canonical_string(&before).as_bytes(),
            1,
        )
        .unwrap();
    let id = engine
        .intern(&parse(r#"{"comp":"C","layer":"A","key":"a","via":"channel"}"#).unwrap())
        .unwrap()
        .index();
    let mut values = [0.0];
    let mut kinds = [0];
    engine.ev_batch(&[id], 1.0, &mut values, &mut kinds);
    assert_eq!(values, [6.0]);
    let after = project("A");
    let body = binary::encode(&ops::diff_ops(Some(&before), Some(&after))).unwrap();
    engine.apply(&body, 1, 2).unwrap();
    engine.ev_batch(&[id], 1.0, &mut values, &mut kinds);
    assert_eq!(values, [101.0]);
    assert_eq!(kinds, [0]);
}
