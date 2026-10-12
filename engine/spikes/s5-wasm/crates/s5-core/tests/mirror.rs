use s5_core::Mirror;
use s5_core::patch::encode_frames;
use serde_json::{Value, json};

fn key(t: f64, v: f64) -> Value {
    json!({"t": t, "v": v, "i": format!("k{t}"), "inInterp": "bezier", "outInterp": "bezier",
           "inEase": {"speed": 0, "influence": 33.333333333333336},
           "outEase": {"speed": 0, "influence": 33.333333333333336},
           "autoBezier": false, "continuous": false})
}

fn layer(id: &str, x: f64, group: Option<&str>) -> Value {
    json!({
        "id": id, "type": if id.starts_with('G') { "group" } else { "shape" }, "name": id,
        "from": 0, "dur": 10, "on": true, "parent": null, "group": group,
        "p": {
            "position.x": {"v": x, "kf": [], "expr": null},
            "position.y": {"v": 0, "kf": [key(0.0, 0.0), key(2.0, 100.0)], "expr": null},
            "anchor.x": {"v": 0, "kf": [], "expr": null},
            "anchor.y": {"v": 0, "kf": [], "expr": null},
            "scale.x": {"v": 100, "kf": [], "expr": null},
            "scale.y": {"v": 100, "kf": [], "expr": null},
            "rotation": {"v": 30, "kf": [], "expr": null},
            "skew": {"v": 0, "kf": [], "expr": null},
            "opacity": {"v": 100, "kf": [], "expr": null}
        },
        "d": {"shape": "rect", "w": 100, "h": 50}
    })
}

fn doc(layers: Vec<Value>) -> Value {
    json!({"id": "P", "fps": 30, "layers": layers, "revision": 1, "edits": []})
}

fn set(path: Value, value: Value) -> (u8, String, Option<String>) {
    (1, path.to_string(), Some(value.to_string()))
}

#[test]
fn load_and_evaluate() {
    let mut m = Mirror::new();
    m.load(
        doc(vec![layer("A", 10.0, None), layer("B", 20.0, None)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(m.layer_count(), 2);
    let ax = m.intern("A", "position.x");
    let by = m.intern("B", "position.y");
    let missing = m.intern("A", "nope");
    let ghost = m.intern("Z", "position.x");
    assert_eq!(m.ev(ax, 0.0), 10.0);
    assert_eq!(m.ev(by, 1.0), 50.0);
    assert_eq!(m.ev(missing, 1.0), 0.0);
    assert!(m.ev(ghost, 1.0).is_nan());
}

#[test]
fn reorder_invalidates_nothing() {
    let mut m = Mirror::new();
    let layers: Vec<Value> = (0..50)
        .map(|i| layer(&format!("L{i}"), i as f64, None))
        .collect();
    m.load(doc(layers.clone()).to_string().as_bytes()).unwrap();
    let ids: Vec<u32> = (0..50)
        .map(|i| m.intern(&format!("L{i}"), "position.x"))
        .collect();
    for &h in &ids {
        m.ev(h, 0.5);
    }
    let mut reordered = layers.clone();
    reordered.swap(3, 40);
    let frames = encode_frames(&[set(json!(["layers"]), Value::Array(reordered))]);
    m.apply_framed(&frames).unwrap();
    let s = m.stats();
    assert_eq!(
        (s.layers_kept, s.layers_parsed, s.handles_invalidated),
        (50, 0, 0)
    );
    assert_eq!(m.layer_ids()[3].as_deref(), Some("L40"));
    assert_eq!(m.ev(ids[40], 0.5), 40.0);
}

#[test]
fn insert_and_delete_invalidate_only_those_ids() {
    let mut m = Mirror::new();
    let layers: Vec<Value> = (0..20)
        .map(|i| layer(&format!("L{i}"), i as f64, None))
        .collect();
    m.load(doc(layers.clone()).to_string().as_bytes()).unwrap();
    let handles: Vec<u32> = (0..20)
        .map(|i| m.intern(&format!("L{i}"), "position.x"))
        .collect();
    let pending = m.intern("NEW", "position.x");
    for &h in &handles {
        m.ev(h, 0.0);
    }
    assert!(m.ev(pending, 0.0).is_nan());
    let mut next = layers.clone();
    next.insert(5, layer("NEW", 99.0, None));
    next.remove(10); // L9
    m.apply_framed(&encode_frames(&[set(
        json!(["layers"]),
        Value::Array(next),
    )]))
    .unwrap();
    let s = m.stats();
    assert_eq!(s.layers_added, 1);
    assert_eq!(s.layers_removed, 1);
    assert_eq!(s.layers_parsed, 1);
    assert_eq!(s.layers_kept, 19);
    assert_eq!(s.handles_invalidated, 2, "the NEW handle and the L9 handle");
    assert_eq!(m.ev(pending, 0.0), 99.0);
    assert!(m.ev(handles[9], 0.0).is_nan());
    assert_eq!(m.ev(handles[10], 0.0), 10.0);
}

#[test]
fn property_edit_invalidates_one_channel() {
    let mut m = Mirror::new();
    m.load(
        doc(vec![layer("A", 10.0, None), layer("B", 20.0, None)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let ax = m.intern("A", "position.x");
    let ay = m.intern("A", "position.y");
    let bx = m.intern("B", "position.x");
    m.ev(ax, 1.0);
    m.ev(ay, 1.0);
    m.ev(bx, 1.0);
    let frames = encode_frames(&[
        set(json!(["layers", 0, "p", "position.x", "v"]), json!(55)),
        set(json!(["revision"]), json!(2)),
    ]);
    m.apply_framed(&frames).unwrap();
    assert_eq!(m.stats().handles_invalidated, 1);
    assert_eq!(m.ev(ax, 1.0), 55.0);
    assert_eq!(m.ev(ay, 1.0), 50.0);
    // A later structural patch carrying the JS serialization still matches.
    let mut a = layer("A", 55.0, None);
    a["p"]["position.x"]["v"] = json!(55);
    m.apply_framed(&encode_frames(&[set(
        json!(["layers"]),
        json!([a, layer("B", 20.0, None)]),
    )]))
    .unwrap();
    assert_eq!(
        m.stats().layers_parsed,
        0,
        "re-serialized bytes equal the JS bytes"
    );
}

#[test]
fn keyframe_edit_and_from_change() {
    let mut m = Mirror::new();
    m.load(doc(vec![layer("A", 10.0, None)]).to_string().as_bytes())
        .unwrap();
    let ay = m.intern("A", "position.y");
    let ax = m.intern("A", "position.x");
    assert_eq!(m.ev(ay, 1.0), 50.0);
    m.apply_framed(&encode_frames(&[set(
        json!(["layers", 0, "p", "position.y", "kf", 1, "v"]),
        json!(200),
    )]))
    .unwrap();
    assert_eq!(m.ev(ay, 1.0), 100.0);
    m.ev(ax, 1.0);
    m.apply_framed(&encode_frames(&[set(
        json!(["layers", 0, "from"]),
        json!(1),
    )]))
    .unwrap();
    assert_eq!(m.stats().handles_invalidated, 2);
    assert_eq!(m.ev(ay, 1.0), 0.0);
}

#[test]
fn id_change_and_element_delete() {
    let mut m = Mirror::new();
    m.load(
        doc(vec![layer("A", 1.0, None), layer("B", 2.0, None)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let a = m.intern("A", "position.x");
    let c = m.intern("C", "position.x");
    assert_eq!(m.ev(a, 0.0), 1.0);
    m.apply_framed(&encode_frames(&[set(
        json!(["layers", 0, "id"]),
        json!("C"),
    )]))
    .unwrap();
    assert!(m.ev(a, 0.0).is_nan());
    assert_eq!(m.ev(c, 0.0), 1.0);
    m.apply_framed(&encode_frames(&[(
        0,
        json!(["layers", 0]).to_string(),
        None,
    )]))
    .unwrap();
    assert!(m.ev(c, 0.0).is_nan());
    assert_eq!(m.layer_count(), 1);
}

#[test]
fn duplicate_ids_resolve_to_the_first() {
    let mut m = Mirror::new();
    m.load(
        doc(vec![layer("A", 1.0, None), layer("A", 2.0, None)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let a = m.intern("A", "position.x");
    assert_eq!(m.ev(a, 0.0), 1.0);
    m.apply_framed(&encode_frames(&[set(
        json!(["layers"]),
        json!([layer("A", 2.0, None), layer("A", 1.0, None)]),
    )]))
    .unwrap();
    assert_eq!(m.ev(a, 0.0), 2.0);
    assert_eq!(m.layer_count(), 2);
}

#[test]
fn layer_order_patch() {
    let mut m = Mirror::new();
    let layers: Vec<Value> = (0..10)
        .map(|i| layer(&format!("L{i}"), i as f64, None))
        .collect();
    m.load(doc(layers).to_string().as_bytes()).unwrap();
    let h = m.intern("L3", "position.x");
    let n = m.intern("N", "position.x");
    m.ev(h, 0.0);
    assert!(m.ev(n, 0.0).is_nan());
    let ids = json!(["N", "L9", "L8", "L7", "L6", "L5", "L4", "L3", "L2", "L1"]).to_string();
    let new_layer = layer("N", 42.0, None).to_string();
    let mut frame = Vec::new();
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.push(2);
    let path = br#"["layers"]"#;
    frame.extend_from_slice(&(path.len() as u32).to_le_bytes());
    frame.extend_from_slice(path);
    frame.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    frame.extend_from_slice(ids.as_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.extend_from_slice(&(new_layer.len() as u32).to_le_bytes());
    frame.extend_from_slice(new_layer.as_bytes());
    m.apply_framed(&frame).unwrap();
    let s = m.stats();
    assert_eq!((s.layers_added, s.layers_removed, s.layers_kept), (1, 1, 9));
    assert_eq!(s.handles_invalidated, 1);
    assert_eq!(m.ev(n, 0.0), 42.0);
    assert_eq!(m.ev(h, 0.0), 3.0);
    assert_eq!(m.layer_ids()[0].as_deref(), Some("N"));
}

#[test]
fn shadow_replay_matches_a_value_model() {
    // Apply the same patch sequence to a serde_json tree with JS semantics
    // and to the mirror; the serialized documents must be equal.
    let mut layers: Vec<Value> = (0..8)
        .map(|i| {
            layer(
                &format!("L{i}"),
                i as f64,
                if i > 0 { Some("G0") } else { None },
            )
        })
        .collect();
    layers[0]["id"] = json!("G0");
    layers[0]["type"] = json!("group");
    let mut shadow = doc(layers.clone());
    let mut m = Mirror::new();
    m.load(shadow.to_string().as_bytes()).unwrap();
    let mut seq: Vec<(Value, Option<Value>)> = vec![
        (
            json!(["layers", 2, "p", "rotation", "v"]),
            Some(json!(45.5)),
        ),
        (json!(["layers", 3, "d", "w"]), Some(json!(7))),
        (json!(["layers", 3, "d", "h"]), None),
        (json!(["revision"]), Some(json!(2))),
        (json!(["edits"]), Some(json!([{"id": "e1"}]))),
        (json!(["edits", 0, "label"]), Some(json!("move"))),
    ];
    let mut reordered = layers.clone();
    reordered.reverse();
    seq.push((json!(["layers"]), Some(Value::Array(reordered))));
    seq.push((json!(["layers", 1]), None));
    seq.push((json!(["layers", 0, "p", "position.y", "kf", 0]), None));
    for (path, value) in &seq {
        let parts: Vec<s5_core::patch::PathPart> = serde_json::from_value(path.clone()).unwrap();
        match value {
            Some(v) => {
                s5_core::patch::apply_value_patch(&mut shadow, &parts, s5_core::patch::Op::Set(v))
                    .unwrap()
            }
            None => {
                s5_core::patch::apply_value_patch(&mut shadow, &parts, s5_core::patch::Op::Delete)
                    .unwrap()
            }
        }
        let frame = match value {
            Some(v) => encode_frames(&[(1, path.to_string(), Some(v.to_string()))]),
            None => encode_frames(&[(0, path.to_string(), None)]),
        };
        m.apply_framed(&frame).unwrap();
        let mirrored: Value = serde_json::from_slice(&m.to_json()).unwrap();
        assert_eq!(mirrored, shadow, "after {path}");
    }
}

#[test]
fn world_matrix_with_groups_and_parents() {
    let mut g = layer("G1", 100.0, None);
    g["p"]["rotation"]["v"] = json!(90);
    g["p"]["position.y"] = json!({"v": 0, "kf": [], "expr": null});
    let mut child = layer("C", 10.0, Some("G1"));
    child["p"]["rotation"]["v"] = json!(0);
    child["p"]["position.y"] = json!({"v": 0, "kf": [], "expr": null});
    let mut m = Mirror::new();
    m.load(doc(vec![g, child]).to_string().as_bytes()).unwrap();
    let w = m.world_matrix(1, 0.0).unwrap();
    // Rotated 90 degrees about the group's origin at x=100: child x=10 maps to (100, 10).
    assert!((w[4] - 100.0).abs() < 1e-9, "{w:?}");
    assert!((w[5] - 10.0).abs() < 1e-9, "{w:?}");
    let mut out = vec![0.0; 12];
    assert_eq!(m.world_all(0.0, &mut out), 2);
    assert_eq!(&out[6..12], &w);
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    let mut m = Mirror::new();
    assert!(m.load(b"{\"layers\": [ {\"id\": 1 ").is_err());
    m.load(doc(vec![layer("A", 1.0, None)]).to_string().as_bytes())
        .unwrap();
    assert!(
        m.apply_framed(&encode_frames(&[(
            1,
            "[\"layers\"]".into(),
            Some("[{\"id\":\"A\"} {}]".into())
        )]))
        .is_err()
    );
    assert!(m.apply_framed(&[1, 0, 0]).is_err());
}
