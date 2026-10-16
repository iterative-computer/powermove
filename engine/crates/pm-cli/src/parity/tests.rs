//! The parity plumbing without a corpus: a dump skeleton is filled with the
//! native answers, then the native and the pm-wasm paths (the engine run
//! natively; the module's Node run is the same requests) must both judge
//! every value a match, and a perturbed value must be a mismatch.

use pm_eval::text_measure::CompatMeasure;
use pm_eval::{Comp, Evaluator};
use pm_model::Document;
use pm_model::json::{Map, Value, parse};
use pm_text::answers::{Answers, ReplayResolver};

use super::compare::{Answer, Stats, plain};
use super::items::{Builder, Item, Kind};
use super::{fonts, judge_dump, native, wasm};

/// The stored form: the open composition at the root, a nested one in `comps`.
const DOC: &str = r##"{
  "id": "P1", "name": "Plumbing", "compId": "C1", "w": 640, "h": 360, "fps": 30, "dur": 4,
  "assets": {}, "params": {},
  "layers": [
    { "id": "A", "type": "solid", "name": "A", "from": 0, "dur": 4, "on": true, "parent": null,
      "p": { "position.x": { "v": 10, "kf": [ { "t": 0, "v": 0, "i": "k1" }, { "t": 2, "v": 200, "i": "k2" } ], "expr": null },
             "position.y": { "v": 40, "kf": [], "expr": null },
             "rotation": { "v": 180, "kf": [], "expr": null },
             "skew": { "v": 0, "kf": [], "expr": "1 +" } },
      "d": { "color": "#ff0000", "w": 120, "h": 60, "radius": 0 }, "fx": [], "masks": [] },
    { "id": "B", "type": "solid", "name": "B", "from": 0.5, "dur": 3, "on": true, "parent": "A", "blend": "screen",
      "p": { "position.x": { "v": 5, "kf": [], "expr": null }, "opacity": { "v": 100, "kf": [], "expr": "value / 2" } },
      "d": { "color": "#00ff00", "w": 20, "h": 20, "radius": 4 }, "fx": [], "masks": [] },
    { "id": "N", "type": "precomp", "name": "N", "from": 0, "dur": 4, "on": true, "parent": null, "p": {},
      "d": { "comp": "C2", "w": 320, "h": 180, "trim": 0, "speed": 2 }, "fx": [], "masks": [] }
  ],
  "comps": {
    "C2": { "id": "C2", "name": "Nested", "w": 320, "h": 180, "fps": 24, "dur": 3,
      "layers": [
        { "id": "S", "type": "solid", "name": "S", "from": 0, "dur": 3, "on": true, "parent": null,
          "p": { "scale.x": { "v": 50, "kf": [ { "t": 0, "v": 50, "i": "s1" }, { "t": 3, "v": 150, "i": "s2" } ], "expr": null } },
          "d": { "color": "#0000ff", "w": 40, "h": 40, "radius": 0 }, "fx": [], "masks": [] }
      ] }
  }
}"##;

/// A dump skeleton: every member the capture writes, values `null`; the
/// channel keys from `allProps`.
fn layer_skeleton(doc: &Document, comp_id: &str) -> Value {
    let comp = Comp::resolve(doc, comp_id).unwrap();
    let ev = Evaluator::new();
    let layers = comp
        .layers()
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut m = Map::new();
            for k in ["id", "name", "type"] {
                m.insert(k, l.get(k).unwrap().to_value());
            }
            m.insert("index", Value::Number(i as f64));
            let mut channels = Map::new();
            for p in ev.all_props(l).unwrap() {
                channels.insert(p.key, Value::Null);
            }
            m.insert("channels", Value::Object(channels));
            for k in [
                "active",
                "layerFields",
                "content",
                "localMatrix",
                "worldMatrix",
                "worldOpacity",
                "planeMatrix",
                "bounds",
            ] {
                m.insert(k, Value::Null);
            }
            if l.get("type")
                .and_then(|t| t.to_value().as_str().map(str::to_owned))
                .as_deref()
                == Some("precomp")
            {
                m.insert("sourceTime", Value::Null);
            }
            Value::Object(m)
        })
        .collect();
    Value::Array(layers)
}

fn dump(doc: &Document) -> Value {
    let state = |id: &str, time: f64, precomps: Value| {
        let mut m = Map::new();
        for (k, v) in [
            ("id", Value::String(id.into())),
            ("time", Value::Number(time)),
        ] {
            m.insert(k, v);
        }
        let comp = Comp::resolve(doc, id).unwrap();
        for k in ["w", "h", "fps"] {
            m.insert(k, comp.member(k).unwrap().to_value());
        }
        m.insert("layers", layer_skeleton(doc, id));
        m.insert("precomps", precomps);
        Value::Object(m)
    };
    let nested = state("C2", 2.0, Value::Array(vec![]));
    let mut entry = Map::new();
    entry.insert("layer", Value::String("N".into()));
    entry.insert("comp", Value::String("C2".into()));
    entry.insert("time", Value::Number(2.0));
    entry.insert("state", nested);
    let mut d = Map::new();
    d.insert("time", Value::Number(1.0));
    d.insert(
        "composition",
        state("C1", 1.0, Value::Array(vec![Value::Object(entry)])),
    );
    d.insert("scene3d", Value::Null);
    d.insert("shaderMeta", Value::Object(Map::new()));
    Value::Object(d)
}

fn setup() -> (Document, Vec<Item>, CompatMeasure<ReplayResolver>) {
    let doc = Document::new(Some(parse(DOC).unwrap()), 1);
    let mut measure = CompatMeasure::new(ReplayResolver::new(&Answers::default()));
    let mut builder = Builder {
        measure: &mut measure,
        skipped: Vec::new(),
    };
    let mut items = builder.dump(&doc, &dump(&doc)).unwrap();
    // The capture's values: the native answers.
    let answers = native::answer_dump(&doc, &items, &mut measure);
    for (it, a) in items.iter_mut().zip(&answers) {
        match a {
            Answer::Value(v) => it.want = plain(v),
            other => panic!("{}: {other:?}", it.label),
        }
    }
    (doc, items, measure)
}

#[test]
fn every_dumped_member_becomes_an_item() {
    let (_, items, _) = setup();
    let count = |category: &str| items.iter().filter(|i| i.category == category).count();
    assert_eq!(count("composition record"), 2);
    assert_eq!(count("layer record"), 4);
    assert_eq!(count("active"), 4);
    assert_eq!(count("layerFields"), 4);
    assert_eq!(count("worldMatrix"), 4);
    assert_eq!(count("sourceTime"), 1);
    assert_eq!(count("sourceTime, legacy step"), 1);
    assert_eq!(count("channels"), count("expression errors"));
    assert!(count("channels") >= 7, "every stored channel at least");
    // The syntax error and the precomp's source time are part of the record.
    let skew = items
        .iter()
        .find(|i| matches!(&i.kind, Kind::ExprError { key } if key.to_string_lossy() == "skew"))
        .unwrap();
    assert_eq!(skew.want.as_str(), Some("Expression syntax is invalid."));
    let source = items.iter().find(|i| i.category == "sourceTime").unwrap();
    assert_eq!(source.want, Value::Number(2.0));
    // The nested composition's layers are scoped by their path.
    assert!(items.iter().any(|i| i.path == "C1>C2" && i.layer_id == "S"));
    // `blend` is stored as a plain string: a raw layer field.
    let fields = items
        .iter()
        .find(|i| i.category == "layerFields" && i.layer_id == "B")
        .unwrap();
    assert_eq!(
        fields.want.get("blend").and_then(Value::as_str),
        Some("screen")
    );
}

#[test]
fn native_and_mirror_answers_match_and_perturbations_do_not() {
    let (doc, mut items, mut measure) = setup();
    let mut stats = Stats::new(false, 4);
    let answers = native::answer_dump(&doc, &items, &mut measure);
    judge_dump(&mut stats, "plumbing", &items, &answers);
    assert_eq!((stats.mismatches(), stats.compared()), (0, items.len()));

    // The pm-wasm path: the same values through the mirror's queries.
    let canonical = pm_model::canonical::canonical_project(parse(DOC).unwrap());
    let mut service = fonts::FontService::new(false);
    let mut discovery = wasm::Discovery::new(&mut service, &canonical).unwrap();
    discovery.dump(&items).unwrap();
    let project = discovery.finish();
    let replies = wasm::native_replies(&project, &mut service).unwrap();
    assert_eq!(replies.len(), project.requests.len());
    let answers = wasm::answers(&items, &project.plans[0], &replies);
    let mut stats = Stats::new(false, 4);
    judge_dump(&mut stats, "plumbing", &items, &answers);
    let samples: Vec<_> = stats
        .categories
        .values()
        .flat_map(|c| c.samples.clone())
        .collect();
    assert_eq!(stats.mismatches(), 0, "{samples:#?}");
    assert_eq!(stats.compared(), items.len());

    // A world matrix 2e-9 off (beyond 1e-9 + 1e-9·|b| for small entries) and
    // a wrong expression error are mismatches.
    let world = items
        .iter_mut()
        .find(|i| i.category == "worldMatrix" && i.layer_id == "B")
        .unwrap();
    if let Value::Array(m) = &mut world.want {
        m[1] = Value::Number(m[1].as_f64().unwrap() + 2e-9);
    }
    let error = items
        .iter_mut()
        .find(|i| i.category == "expression errors" && i.layer_id == "B")
        .unwrap();
    error.want = Value::String("Expression failed.".into());
    let mut stats = Stats::new(false, 4);
    judge_dump(&mut stats, "plumbing", &items, &answers);
    assert_eq!(stats.categories["worldMatrix"].mismatched, 1);
    assert_eq!(stats.categories["expression errors"].mismatched, 1);
}
