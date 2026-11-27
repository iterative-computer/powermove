//! Interned ids follow patches: answers after every batch equal a fresh
//! evaluation, and only the ids whose inputs changed are re-planned.

use pm_eval::node::{self, Got};
use pm_eval::{Comp, Evaluator, JsResult, JsValue, PropAddress, PropId, Via, address};
use pm_model::json::parse;
use pm_model::{Document, JsString, Patch, PathPart, Value};

const DOC: &str = r##"{
  "id": "P", "compId": "C",
  "comps": { "C": { "id": "C", "name": "Main", "fps": 30, "dur": 10, "w": 100, "h": 100, "layers": [
    { "id": "A", "type": "solid", "from": 0, "dur": 10, "on": true,
      "p": { "position.x": { "v": 0, "kf": [ { "t": 0, "v": 0 }, { "t": 2, "v": 100, "inInterp": "bezier", "outInterp": "linear",
                 "inEase": { "speed": 0, "influence": 50 }, "outEase": { "speed": 0, "influence": 33 } } ], "expr": null },
             "position.y": { "v": 5, "kf": [], "expr": null },
             "opacity": { "v": 100, "kf": [ { "t": 1, "v": 0, "hold": true }, { "t": 3, "v": 100 } ], "expr": null } },
      "d": { "color": { "v": "#ff0000", "kf": [ { "t": 0, "v": "#000000" }, { "t": 4, "v": "#ffffff80" } ], "expr": null }, "w": 10 },
      "fx": [ { "id": "f1", "type": "blur", "on": true, "p": { "amount": { "v": 3, "kf": [ { "t": 0, "v": 1 }, { "t": 1, "v": 9, "spring": {} } ], "expr": null } } } ] },
    { "id": "B", "type": "text", "from": 1, "dur": 5, "on": true,
      "p": { "position.x": { "v": 7, "kf": [ { "t": 0, "v": 1, "eo": [0.5, 0], "ei": [0.5, 1] }, { "t": 3, "v": 4, "eo": [0.4, 0], "ei": [0.6, 1] } ], "expr": null },
             "rotation": { "v": 0, "kf": [ { "t": 0, "v": "a" }, { "t": 1, "v": "b" } ], "expr": null } },
      "d": { "text": { "v": "hi", "kf": [ { "t": 0, "v": "x" }, { "t": 2, "v": "y" } ], "expr": null } } }
  ] } }
}"##;

fn addresses() -> Vec<PropAddress> {
    let mut out = Vec::new();
    for layer in ["A", "B", "Z"] {
        for key in ["position.x", "position.y", "opacity", "rotation", "missing"] {
            out.push(PropAddress::channel("C", layer, key));
        }
        for key in [
            "c.color",
            "c.text",
            "f1.amount",
            "f1.$enabled",
            "position.x",
            "nope",
        ] {
            out.push(PropAddress::find("C", layer, key));
        }
        out.push(PropAddress {
            comp: JsString::from("C"),
            layer: JsString::from(layer),
            key: JsString::from("c.color"),
            via: Via::Member(vec![JsString::from("d"), JsString::from("color")]),
        });
    }
    out
}

fn same(a: &JsResult<JsValue>, b: &JsResult<JsValue>) -> bool {
    match (a, b) {
        (Ok(JsValue::Number(x)), Ok(JsValue::Number(y))) => {
            x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
        }
        (Err(x), Err(y)) => x.kind == y.kind,
        _ => a == b,
    }
}

/// The node-API answer a fresh evaluator gives.
fn fresh(doc: &Document, address: &PropAddress, time: f64) -> JsResult<JsValue> {
    let Ok(cx) = Comp::resolve(doc, "C") else {
        return Ok(JsValue::Undefined);
    };
    let Some(layer) = cx
        .layers()
        .iter()
        .find(|l| node::string(node::member(Some(l), "id")).is_some_and(|id| *id == address.layer))
    else {
        return Ok(JsValue::Undefined);
    };
    let mut ev = Evaluator::new();
    match &address.via {
        Via::Channel => ev.ev(&cx, Some(layer), &address.key, time),
        Via::Find => {
            let prop = address::find_prop(layer, &address.key)?;
            ev.ev_p(&cx, Some(layer), prop.node(), time, &address.key)
        }
        Via::Member(path) => {
            let mut n = Some(layer);
            for part in path {
                n = n.and_then(|x| match node::get(x, part.as_bytes()) {
                    Got::Node(c) => Some(c),
                    _ => None,
                });
            }
            ev.ev_p(&cx, Some(layer), n, time, &address.key)
        }
    }
}

fn check_all(ev: &mut Evaluator, doc: &Document, ids: &[(PropId, PropAddress)], times: &[f64]) {
    for &t in times {
        for (id, address) in ids {
            let got = ev.ev_id(doc, *id, t);
            let want = fresh(doc, address, t);
            assert!(
                same(&got, &want),
                "{address:?} at {t}: cached {got:?}, fresh {want:?}"
            );
            let again = ev.ev_id(doc, *id, t);
            assert!(same(&again, &want));
        }
    }
}

fn key(path: &[&str]) -> Vec<PathPart> {
    path.iter().map(|p| PathPart::from(*p)).collect()
}

fn layer_path(index: usize, rest: &[&str]) -> Vec<PathPart> {
    let mut p = key(&["comps", "C", "layers"]);
    p.push(PathPart::Index(index as f64));
    p.extend(key(rest));
    p
}

#[test]
fn edits_replan_only_what_they_touch() {
    let mut doc = Document::new(Some(parse(DOC).unwrap()), 1);
    let mut ev = Evaluator::new();
    let ids: Vec<(PropId, PropAddress)> = addresses()
        .into_iter()
        .map(|a| (ev.intern(a.clone()), a))
        .collect();
    let times = [0.5, 1.5, 2.0, 3.25];
    check_all(&mut ev, &doc, &ids, &times);
    let first = ev.cache_stats();
    // 12 ids per layer; layer Z does not exist and is never planned.
    assert_eq!(first.plans, 24);

    // A keyframe value of A's position.x.
    let mut kf = layer_path(0, &["p", "position.x", "kf"]);
    kf.push(PathPart::Index(1.0));
    kf.push(PathPart::from("v"));
    doc.apply(&[Patch::set(kf, Value::Number(250.0))]).unwrap();
    let before = ev.cache_stats();
    check_all(&mut ev, &doc, &ids, &times);
    let after = ev.cache_stats();
    // Re-planned: A's channel ids whose property changed or is absent
    // (position.x, rotation, missing) and every Find id on A (the layer
    // changed); kept: A's position.y, opacity, and member id, and all of B.
    let replanned = after.plans - before.plans;
    let a_find = [
        "c.color",
        "c.text",
        "f1.amount",
        "f1.$enabled",
        "position.x",
        "nope",
    ]
    .len() as u64;
    assert_eq!(replanned, 3 + a_find, "{before:?} -> {after:?}");

    // Reordering layers re-plans nothing.
    let root = doc.to_value().unwrap();
    let mut layers = root
        .get("comps")
        .unwrap()
        .get("C")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    layers.reverse();
    doc.apply(&[Patch::set(
        key(&["comps", "C", "layers"]),
        Value::Array(layers),
    )])
    .unwrap();
    let before = ev.cache_stats();
    check_all(&mut ev, &doc, &ids, &times);
    assert_eq!(ev.cache_stats().plans, before.plans, "reorder");

    // A layer that takes A's id in front of A: A's ids now address it.
    let mut newcomer = parse(r#"{"id":"A","type":"solid","from":0,"dur":3,"on":true,"p":{"position.x":{"v":42,"kf":[],"expr":null}}}"#).unwrap();
    let mut layers = doc
        .to_value()
        .unwrap()
        .get("comps")
        .unwrap()
        .get("C")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    layers.insert(0, newcomer.clone());
    doc.apply(&[Patch::set(
        key(&["comps", "C", "layers"]),
        Value::Array(layers),
    )])
    .unwrap();
    check_all(&mut ev, &doc, &ids, &times);
    assert_eq!(
        ev.ev_id(&doc, ids[0].0, 1.0).unwrap(),
        JsValue::Number(42.0)
    );

    // Deleting B: B's ids answer undefined.
    let layers: Vec<Value> = doc
        .to_value()
        .unwrap()
        .get("comps")
        .unwrap()
        .get("C")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l.get("id").and_then(Value::as_str) != Some("B"))
        .cloned()
        .collect();
    doc.apply(&[Patch::set(
        key(&["comps", "C", "layers"]),
        Value::Array(layers),
    )])
    .unwrap();
    check_all(&mut ev, &doc, &ids, &times);
    let b = ids
        .iter()
        .find(|(_, a)| a.layer.as_str() == Some("B"))
        .unwrap()
        .0;
    assert_eq!(ev.ev_id(&doc, b, 1.0).unwrap(), JsValue::Undefined);

    // Renaming the newcomer away from A: A's ids address the original again.
    if let Some(map) = newcomer.as_object_mut() {
        map.insert("id", Value::from("N"));
    }
    doc.apply(&[Patch::set(layer_path(0, &["id"]), Value::from("N"))])
        .unwrap();
    check_all(&mut ev, &doc, &ids, &times);

    // Releasing and reinterning reuses the slot.
    let (id, address) = ids[0].clone();
    ev.release(id);
    assert!(ev.address(id).is_none());
    let again = ev.intern(address.clone());
    assert_eq!(again, id);
    assert!(same(
        &ev.ev_id(&doc, again, 1.0),
        &fresh(&doc, &address, 1.0)
    ));
}
