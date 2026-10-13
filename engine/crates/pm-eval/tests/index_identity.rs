//! The layer index and the interned-id plans follow the node a composition
//! resolves to, not only its id: a composition that moves between the top
//! level and `comps` (the stored form's open composition), or a second
//! document at the same revision, never reuses another node's layer positions.

use pm_eval::{Comp, Evaluator, JsValue, PropAddress};
use pm_model::json::parse;
use pm_model::{Document, Patch, PathPart, Value};

/// The stored form: the open composition C at the top level with layers
/// [A, B], and a stale copy of C in `comps` with the layers in the other
/// order and other values.
const STORED: &str = r##"{
  "id": "P", "compId": "C", "fps": 30, "dur": 10,
  "layers": [
    { "id": "A", "type": "solid", "from": 0, "dur": 10, "on": true, "p": { "opacity": { "v": 11, "kf": [], "expr": null } } },
    { "id": "B", "type": "solid", "from": 0, "dur": 10, "on": true, "p": { "opacity": { "v": 22, "kf": [], "expr": null } } }
  ],
  "comps": {
    "C": { "id": "C", "fps": 30, "dur": 10, "layers": [
      { "id": "B", "type": "solid", "from": 0, "dur": 10, "on": true, "p": { "opacity": { "v": 33, "kf": [], "expr": null } } },
      { "id": "A", "type": "solid", "from": 0, "dur": 10, "on": true, "p": { "opacity": { "v": 44, "kf": [], "expr": null } } }
    ] },
    "N": { "id": "N", "fps": 30, "dur": 10, "layers": [] }
  }
}"##;

fn fresh_opacity(doc: &Document, layer: &str) -> JsValue {
    let cx = Comp::resolve(doc, "C").unwrap();
    let l = cx
        .layers()
        .iter()
        .find(|l| {
            l.get("id")
                .and_then(|n| n.to_value().as_str().map(str::to_owned))
                == Some(layer.to_owned())
        })
        .unwrap();
    Evaluator::new()
        .ev(&cx, Some(l), &"opacity".into(), 1.0)
        .unwrap()
}

#[test]
fn switching_the_open_composition_rebuilds_the_index_and_plans() {
    let mut doc = Document::new(Some(parse(STORED).unwrap()), 1);
    let mut ev = Evaluator::new();
    let a = ev.intern(PropAddress::channel("C", "A", "opacity"));
    assert_eq!(ev.ev_id(&doc, a, 1.0).unwrap(), JsValue::Number(11.0));
    // Opening N: C now resolves to the copy in `comps`, whose nodes are older
    // than the evaluator's last revision.
    doc.apply(&[Patch::set(vec![PathPart::from("compId")], Value::from("N"))])
        .unwrap();
    assert_eq!(fresh_opacity(&doc, "A"), JsValue::Number(44.0));
    assert_eq!(fresh_opacity(&doc, "B"), JsValue::Number(33.0));
    assert_eq!(ev.ev_id(&doc, a, 1.0).unwrap(), JsValue::Number(44.0));
    let b = ev.intern(PropAddress::channel("C", "B", "opacity"));
    assert_eq!(ev.ev_id(&doc, b, 1.0).unwrap(), JsValue::Number(33.0));
    // And back.
    doc.apply(&[Patch::set(vec![PathPart::from("compId")], Value::from("C"))])
        .unwrap();
    assert_eq!(ev.ev_id(&doc, a, 1.0).unwrap(), JsValue::Number(11.0));
    assert_eq!(ev.ev_id(&doc, b, 1.0).unwrap(), JsValue::Number(22.0));
    // The by-node API agrees on the same evaluator.
    let cx = Comp::resolve(&doc, "C").unwrap();
    let mut out = Vec::new();
    for l in cx.layers() {
        out.push(ev.ev(&cx, Some(l), &"opacity".into(), 1.0).unwrap());
    }
    assert_eq!(out, vec![JsValue::Number(11.0), JsValue::Number(22.0)]);
}

fn grouped(group_on: bool, group_first: bool) -> String {
    let group =
        format!(r#"{{ "id": "G1", "type": "group", "from": 0, "dur": 10, "on": {group_on} }}"#);
    let child =
        r#"{ "id": "X", "type": "solid", "group": "G1", "from": 0, "dur": 10, "on": true }"#;
    let layers = if group_first {
        format!("{group},{child}")
    } else {
        format!("{child},{group}")
    };
    format!(
        r#"{{ "id": "P", "compId": "C", "comps": {{ "C": {{ "id": "C", "fps": 30, "dur": 10, "layers": [{layers}] }} }} }}"#
    )
}

fn child<'d>(cx: &Comp<'d>) -> &'d pm_model::Node {
    cx.layers()
        .iter()
        .find(|l| l.get("id").map(|n| n.to_value()) == Some(Value::from("X")))
        .unwrap()
}

#[test]
fn by_node_calls_on_two_documents_at_one_revision_do_not_share_an_index() {
    let on = Document::new(Some(parse(&grouped(true, true)).unwrap()), 1);
    let off = Document::new(Some(parse(&grouped(false, false)).unwrap()), 1);
    let cx_on = Comp::resolve(&on, "C").unwrap();
    let cx_off = Comp::resolve(&off, "C").unwrap();
    let want_off = Evaluator::new()
        .active(&cx_off, Some(child(&cx_off)), 1.0)
        .unwrap();
    assert!(!want_off, "the group is off");

    let mut ev = Evaluator::new();
    assert!(ev.active(&cx_on, Some(child(&cx_on)), 1.0).unwrap());
    assert_eq!(
        ev.active(&cx_off, Some(child(&cx_off)), 1.0).unwrap(),
        want_off
    );
    assert!(ev.active(&cx_on, Some(child(&cx_on)), 1.0).unwrap());
    let ancestors = ev.group_ancestors(&cx_off, child(&cx_off)).unwrap();
    assert_eq!(ancestors.len(), 1);
    assert!(std::ptr::eq(ancestors[0], &cx_off.layers()[1]));

    // Interned ids switching between the two documents re-resolve too.
    let id = ev.intern(PropAddress {
        comp: "C".into(),
        layer: "G1".into(),
        key: "l.on".into(),
        via: pm_eval::Via::Member(vec!["on".into()]),
    });
    assert_eq!(ev.ev_id(&on, id, 1.0).unwrap(), JsValue::Null);
    let on_flag = Document::new(
        Some(
            parse(
                &grouped(true, true)
                    .replace(r#""on": true }"#, r#""on": {"v":1,"kf":[],"expr":null} }"#),
            )
            .unwrap(),
        ),
        1,
    );
    assert_eq!(ev.ev_id(&on_flag, id, 1.0).unwrap(), JsValue::Number(1.0));
    assert_eq!(ev.ev_id(&on, id, 1.0).unwrap(), JsValue::Null);
}
