//! Random patches against a fresh evaluation: after every batch, every
//! interned id (and the by-node `active`, which uses the cached layer index)
//! answers what a new evaluator answers on the same document.
//!
//! The batches cover leaf edits, keyframe and expression edits, layer
//! inserts, deletes, and reorders in every `layers` array, switching the open
//! composition of the stored form, replacing or deleting whole compositions,
//! adding and removing the stored form's top-level `layers`, and project and
//! composition parameters. Times repeat across rounds and some rounds apply
//! nothing or an empty batch, so the per-id memo is exercised at the same
//! revision and across revisions. A mock expression engine reads the value,
//! times, `idx`, parameters, and the composition.
//!
//! `PM_EVAL_SHADOW=seed:rounds` runs one longer seed on both document forms.

use std::sync::Arc;

use pm_eval::node::{self, Got};
use pm_eval::{
    CacheStats, Comp, CompiledExpression, Evaluator, ExprContext, ExprError, ExpressionEngine,
    JsResult, JsValue, PropAddress, PropId, Via, address,
};
use pm_model::json::parse;
use pm_model::{Document, JsString, Node, Patch, PathPart, Value};

const A: &str = r##"{ "id": "A", "type": "solid", "from": 0, "dur": 10, "on": true,
  "p": { "position.x": { "v": 0, "kf": [ { "t": 0, "v": 0 }, { "t": 2, "v": 100, "inInterp": "bezier", "outInterp": "linear",
             "inEase": { "speed": 0, "influence": 50 }, "outEase": { "speed": 0, "influence": 33 } } ], "expr": null },
         "position.y": { "v": 5, "kf": [], "expr": "double" },
         "opacity": { "v": 100, "kf": [ { "t": 1, "v": 0, "hold": true }, { "t": 3, "v": 100 } ], "expr": null } },
  "d": { "color": { "v": "#ff0000", "kf": [ { "t": 0, "v": "#000000" }, { "t": 4, "v": "#ffffff80" } ], "expr": null }, "w": 10 },
  "fx": [ { "id": "f1", "type": "blur", "on": true, "p": { "amount": { "v": 3, "kf": [ { "t": 0, "v": 1 }, { "t": 1, "v": 9, "spring": {} } ], "expr": null } } } ] }"##;

const B: &str = r##"{ "id": "B", "type": "text", "from": 1, "dur": 5, "on": true,
  "p": { "position.x": { "v": 7, "kf": [ { "t": 0, "v": 1, "eo": [0.5, 0], "ei": [0.5, 1] }, { "t": 3, "v": 4, "eo": [0.4, 0], "ei": [0.6, 1] } ], "expr": null },
         "rotation": { "v": 0, "kf": [ { "t": 0, "v": "a" }, { "t": 1, "v": "b" } ], "expr": null },
         "opacity": { "v": 60, "kf": [], "expr": "time" } },
  "d": { "text": { "v": "hi", "kf": [ { "t": 0, "v": "x" }, { "t": 2, "v": "y" } ], "expr": null } } }"##;

const G: &str = r##"{ "id": "G", "type": "group", "from": 0, "dur": 10,
  "on": { "v": 1, "kf": [ { "t": 0, "v": 1 }, { "t": 2, "v": 0, "hold": true } ], "expr": null },
  "p": { "opacity": { "v": 50, "kf": [], "expr": "gain" } } }"##;

const Q: &str = r##"{ "id": "Q", "type": "solid", "group": "G", "from": 0.5, "dur": 4, "on": true,
  "p": { "opacity": { "v": 30, "kf": [ { "t": 0, "v": 0 }, { "t": 1, "v": 100 } ], "expr": "double" },
         "position.x": { "v": 1, "kf": [], "expr": "idx" } } }"##;

const Z: &str = r##"{ "id": "Z", "type": "solid", "group": "Q", "from": 0, "dur": 2, "on": true,
  "p": { "position.x": { "v": 3, "kf": [], "expr": "count" }, "rotation": { "v": 2, "kf": [], "expr": "frame" } } }"##;

fn layer(text: &str) -> Value {
    parse(text).unwrap()
}

fn with(mut value: Value, path: &[&str], v: Value) -> Value {
    let mut cur = &mut value;
    for part in &path[..path.len() - 1] {
        cur = cur.as_object_mut().unwrap().get_mut(part).unwrap();
    }
    cur.as_object_mut().unwrap().insert(path[path.len() - 1], v);
    value
}

fn comp_value(id: &str, fps: f64, dur: f64, layers: Vec<Value>, gain: Option<f64>) -> Value {
    let mut map = pm_model::Map::new();
    map.insert("id", Value::from(id));
    map.insert("fps", Value::Number(fps));
    map.insert("dur", Value::Number(dur));
    map.insert("layers", Value::Array(layers));
    if let Some(g) = gain {
        map.insert(
            "params",
            parse(&format!(r#"{{"gain":{{"value":{g}}}}}"#)).unwrap(),
        );
    }
    Value::Object(map)
}

fn open_layers() -> Vec<Value> {
    vec![layer(A), layer(B), layer(G), layer(Q)]
}

/// The stale `comps.C` copy of the stored form: other order, other values.
fn stale_layers() -> Vec<Value> {
    vec![
        with(layer(Q), &["p", "opacity", "v"], Value::Number(77.0)),
        with(layer(A), &["p", "opacity", "v"], Value::Number(44.0)),
        layer(B),
    ]
}

fn nested_layers() -> Vec<Value> {
    vec![layer(B), layer(Z), layer(G), layer(A)]
}

fn canonical() -> Value {
    let mut comps = pm_model::Map::new();
    comps.insert("C", comp_value("C", 30.0, 10.0, open_layers(), None));
    comps.insert("N", comp_value("N", 25.0, 4.0, nested_layers(), Some(9.0)));
    let mut root = pm_model::Map::new();
    root.insert("id", Value::from("P"));
    root.insert("compId", Value::from("C"));
    root.insert("params", parse(r#"{"gain":{"value":4}}"#).unwrap());
    root.insert("comps", Value::Object(comps));
    Value::Object(root)
}

fn stored() -> Value {
    let mut comps = pm_model::Map::new();
    comps.insert("C", comp_value("C", 24.0, 8.0, stale_layers(), None));
    comps.insert("N", comp_value("N", 25.0, 4.0, nested_layers(), Some(9.0)));
    let mut root = pm_model::Map::new();
    root.insert("id", Value::from("P"));
    root.insert("compId", Value::from("C"));
    root.insert("fps", Value::Number(30.0));
    root.insert("dur", Value::Number(10.0));
    root.insert("params", parse(r#"{"gain":{"value":4}}"#).unwrap());
    root.insert("layers", Value::Array(open_layers()));
    root.insert("comps", Value::Object(comps));
    Value::Object(root)
}

// ── a mock expression engine ─────────────────────────────────────────────

struct Engine;
struct Expr(&'static str);

const SOURCES: [&str; 8] = [
    "double", "time", "gain", "idx", "boom", "frame", "count", "nonsense",
];

impl ExpressionEngine for Engine {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        SOURCES[..7]
            .iter()
            .find(|k| source.as_str() == Some(k))
            .map(|k| Arc::new(Expr(k)) as Arc<dyn CompiledExpression>)
    }
}

impl CompiledExpression for Expr {
    fn needs_idx(&self) -> bool {
        self.0 == "idx"
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        Ok(match self.0 {
            "double" => JsValue::Number(cx.value.to_number() * 2.0),
            "time" => JsValue::Number(cx.time * 10.0 + cx.t),
            "gain" => cx.param(&JsString::from("gain")).map_err(|e| ExprError {
                message: Some(e.message),
            })?,
            "idx" => JsValue::Number(cx.idx),
            "boom" => {
                return Err(ExprError {
                    message: Some("boom".into()),
                });
            }
            "frame" => JsValue::Number(cx.frame.map_or(-1.0, |f| f as f64)),
            // Reads the composition beyond the layer.
            "count" => JsValue::Number(cx.comp.layers().len() as f64 + cx.comp.duration()),
            _ => unreachable!(),
        })
    }
}

fn evaluator() -> Evaluator {
    Evaluator::new().with_expressions(Arc::new(Engine))
}

// ── addresses and fresh answers ──────────────────────────────────────────

fn addresses() -> Vec<PropAddress> {
    let mut out = Vec::new();
    for comp in ["C", "N"] {
        for layer in ["A", "B", "G", "Q", "Z", "Y"] {
            for key in ["position.x", "position.y", "opacity", "rotation", "missing"] {
                out.push(PropAddress {
                    comp: comp.into(),
                    layer: layer.into(),
                    key: key.into(),
                    via: Via::Channel,
                });
            }
            for key in [
                "c.color",
                "c.text",
                "f1.amount",
                "f1.$enabled",
                "l.on",
                "opacity",
            ] {
                out.push(PropAddress {
                    comp: comp.into(),
                    layer: layer.into(),
                    key: key.into(),
                    via: Via::Find,
                });
            }
            out.push(PropAddress {
                comp: comp.into(),
                layer: layer.into(),
                key: "c.color".into(),
                via: Via::Member(vec!["d".into(), "color".into()]),
            });
            out.push(PropAddress {
                comp: comp.into(),
                layer: layer.into(),
                key: "l.on".into(),
                via: Via::Member(vec!["on".into()]),
            });
        }
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

fn find_layer<'d>(cx: &Comp<'d>, id: &JsString) -> Option<&'d Node> {
    cx.layers()
        .iter()
        .find(|l| node::string(node::member(Some(l), "id")).is_some_and(|s| s == id))
}

fn fresh(doc: &Document, address: &PropAddress, time: f64) -> JsResult<JsValue> {
    let Ok(cx) = Comp::resolve(doc, &address.comp.to_string_lossy()) else {
        return Ok(JsValue::Undefined);
    };
    let Some(layer) = find_layer(&cx, &address.layer) else {
        return Ok(JsValue::Undefined);
    };
    let mut ev = evaluator();
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

// ── the generator ────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn path(parts: &[&str]) -> Vec<PathPart> {
    parts.iter().map(|p| PathPart::from(*p)).collect()
}

const ARRAYS: [&[&str]; 3] = [
    &["layers"],
    &["comps", "C", "layers"],
    &["comps", "N", "layers"],
];

fn layers_at(doc: &Document, at: &[&str]) -> Option<Vec<Value>> {
    doc.lookup(&path(at))
        .and_then(|n| n.to_value().as_array().cloned())
}

fn random_layer(rng: &mut Rng) -> Value {
    let base = layer(rng.pick(&[A, B, G, Q, Z]));
    match rng.below(4) {
        0 => with(
            base,
            &["id"],
            Value::from(*rng.pick(&["A", "B", "G", "Q", "Z", "Y"])),
        ),
        1 => with(base, &["from"], Value::Number(rng.below(4) as f64 / 2.0)),
        _ => base,
    }
}

fn random_batch(rng: &mut Rng, doc: &Document) -> Vec<Patch> {
    let values = [
        Value::Number(3.5),
        Value::from("7"),
        Value::Bool(true),
        Value::Null,
        Value::from("#00ff00"),
        parse(r#"{"v":1,"kf":[],"expr":null}"#).unwrap(),
        parse(r#"{"v":2,"kf":[{"t":0,"v":0},{"t":2,"v":8,"outInterp":"hold","inInterp":"linear","inEase":{"speed":0,"influence":10},"outEase":{"speed":0,"influence":10}}],"expr":null}"#).unwrap(),
        parse(r#"{"v":4,"kf":[],"expr":"double"}"#).unwrap(),
        parse(r#"[{"t":0,"v":5},{"t":1,"v":"x"},{"t":3,"v":1}]"#).unwrap(),
    ];
    let mut batch = Vec::new();
    for _ in 0..1 + rng.below(3) {
        let at = *rng.pick(&ARRAYS);
        let layers = layers_at(doc, at).unwrap_or_default();
        let index = if layers.is_empty() {
            0
        } else {
            rng.below(layers.len())
        };
        let in_layer = |rest: &[&str]| {
            let mut p = path(at);
            p.push(PathPart::Index(index as f64));
            p.extend(path(rest));
            p
        };
        let channel = *rng.pick(&["position.x", "position.y", "opacity", "rotation"]);
        let (target, value) = match rng.below(16) {
            0 => (in_layer(&["p", channel, "v"]), None),
            1 => {
                let mut p = in_layer(&["p", *rng.pick(&["position.x", "opacity"]), "kf"]);
                p.push(PathPart::Index(rng.below(3) as f64));
                p.push(PathPart::from(*rng.pick(&["v", "t", "hold", "spring"])));
                (p, None)
            }
            2 => (in_layer(&["from"]), None),
            3 => (in_layer(&["id"]), None),
            4 => (in_layer(&["p", channel]), None),
            5 => (in_layer(&["d", *rng.pick(&["color", "text"])]), None),
            6 => {
                let mut p = in_layer(&["fx"]);
                p.push(PathPart::Index(0.0));
                p.extend(path(&["p", "amount", "v"]));
                (p, None)
            }
            7 | 8 => {
                // An expression edit.
                let source = rng.pick(&SOURCES);
                let v = if rng.below(5) == 0 {
                    Value::Null
                } else {
                    Value::from(*source)
                };
                let target = if rng.below(4) == 0 {
                    in_layer(&["d", "color", "expr"])
                } else {
                    in_layer(&["p", channel, "expr"])
                };
                (target, Some(v))
            }
            9 => {
                // Reorder, insert, or delete in one `layers` array.
                let mut layers = layers;
                match rng.below(3) {
                    0 if !layers.is_empty() => {
                        let l = layers.remove(rng.below(layers.len()));
                        layers.insert(rng.below(layers.len() + 1), l);
                    }
                    1 => {
                        let at = rng.below(layers.len() + 1);
                        layers.insert(at, random_layer(rng));
                    }
                    _ if !layers.is_empty() => {
                        layers.remove(rng.below(layers.len()));
                    }
                    _ => {}
                }
                (path(at), Some(Value::Array(layers)))
            }
            10 => {
                // A leaf insert or delete (the array patch the editor sends).
                let mut p = path(at);
                if rng.below(2) == 0 || layers.is_empty() {
                    p.push(PathPart::Index(layers.len() as f64));
                    (p, Some(random_layer(rng)))
                } else {
                    p.push(PathPart::Index(index as f64));
                    batch.push(Patch::delete(p));
                    continue;
                }
            }
            11 => {
                // Switch the open composition.
                (
                    path(&["compId"]),
                    Some(Value::from(*rng.pick(&["C", "N", "X"]))),
                )
            }
            12 => {
                // Replace or delete a whole composition.
                let id = *rng.pick(&["C", "N"]);
                if rng.below(4) == 0 {
                    batch.push(Patch::delete(path(&["comps", id])));
                    continue;
                }
                let layers = match rng.below(3) {
                    0 => open_layers(),
                    1 => stale_layers(),
                    _ => nested_layers(),
                };
                let fps = *rng.pick(&[24.0, 25.0, 30.0]);
                let gain = (rng.below(2) == 0).then(|| rng.below(4) as f64);
                (
                    path(&["comps", id]),
                    Some(comp_value(id, fps, 6.0, layers, gain)),
                )
            }
            13 => {
                // Add or remove the stored form's top-level layers.
                if rng.below(3) == 0 {
                    batch.push(Patch::delete(path(&["layers"])));
                    continue;
                }
                let source = *rng.pick(&ARRAYS);
                (
                    path(&["layers"]),
                    Some(Value::Array(
                        layers_at(doc, source).unwrap_or_else(open_layers),
                    )),
                )
            }
            14 => {
                // Parameters: the project's and a nested composition's.
                let target = if rng.below(2) == 0 {
                    path(&["params", "gain", "value"])
                } else {
                    path(&["comps", "N", "params", "gain", "value"])
                };
                (target, Some(Value::Number(rng.below(5) as f64)))
            }
            _ => (
                path(&["comps", "C", "dur"]),
                Some(Value::Number(rng.below(12) as f64)),
            ),
        };
        let value = value.unwrap_or_else(|| match target.last() {
            Some(PathPart::Key(k)) if k.as_str() == Some("id") => {
                Value::from(*rng.pick(&["A", "B", "G", "Q", "Z", "Y"]))
            }
            Some(PathPart::Key(k)) if k.as_str() == Some("t") || k.as_str() == Some("from") => {
                Value::Number((rng.unit() * 6.0 - 1.0).floor() / 2.0)
            }
            _ => rng.pick(&values).clone(),
        });
        batch.push(if rng.below(12) == 0 {
            Patch::delete(target)
        } else {
            Patch::set(target, value)
        });
    }
    batch
}

/// Runs one seed; returns the patches applied and the cache counters.
fn shadow(initial: Value, seed: u64, rounds: usize) -> (usize, CacheStats) {
    let mut rng = Rng(seed);
    let mut doc = Document::new(Some(initial), 1);
    let mut ev = evaluator();
    let ids: Vec<(PropId, PropAddress)> = addresses()
        .into_iter()
        .map(|a| (ev.intern(a.clone()), a))
        .collect();
    let times = [0.0, 0.5, 1.0, 1.5, 2.0, 3.25, 60.0 / 29.97];
    let mut patches_applied = 0;
    for round in 0..rounds {
        let batch = match rng.below(10) {
            // Evaluate again at the same revision.
            0..=2 => None,
            // An empty batch: a new revision that changes nothing.
            3 => Some(Vec::new()),
            _ => Some(random_batch(&mut rng, &doc)),
        };
        if let Some(batch) = &batch
            && doc.apply(batch).is_ok()
        {
            patches_applied += batch.len();
        }
        let t = if rng.below(3) == 0 {
            rng.unit() * 5.0 - 0.5
        } else {
            *rng.pick(&times)
        };
        let t2 = *rng.pick(&times);
        for time in [t, t2, t] {
            for (id, address) in &ids {
                let got = ev.ev_id(&doc, *id, time);
                let want = fresh(&doc, address, time);
                assert!(
                    same(&got, &want),
                    "seed {seed} round {round}: {address:?} at {time}: cached {got:?}, fresh {want:?}\nbatch {batch:?}\ndocument {}",
                    pm_model::json::to_canonical_string(&doc.to_value().unwrap())
                );
            }
        }
        // The by-node `active` reads the cached layer index.
        for comp in ["C", "N"] {
            let Ok(cx) = Comp::resolve(&doc, comp) else {
                continue;
            };
            for l in cx.layers() {
                let got = ev.active(&cx, Some(l), t);
                let want = evaluator().active(&cx, Some(l), t);
                assert!(
                    match (&got, &want) {
                        (Ok(a), Ok(b)) => a == b,
                        (Err(a), Err(b)) => a.kind == b.kind,
                        _ => false,
                    },
                    "seed {seed} round {round}: active in {comp} at {t}: cached {got:?}, fresh {want:?}"
                );
            }
        }
    }
    let stats = ev.cache_stats();
    assert_eq!(stats.stale_indexes, 0, "{stats:?}");
    (patches_applied, stats)
}

fn seeds() -> Vec<(u64, usize)> {
    match std::env::var("PM_EVAL_SHADOW").ok().and_then(|v| {
        let (a, b) = v.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    }) {
        Some(one) => vec![one],
        None => vec![(0x9e37_79b9_7f4a_7c15, 300), (12345, 150), (987_654, 150)],
    }
}

#[test]
fn random_patches_shadow_fresh_evaluation_canonical() {
    for (seed, rounds) in seeds() {
        let (patches, stats) = shadow(canonical(), seed, rounds);
        eprintln!("canonical seed {seed}: {patches} patches; {stats:?}");
        assert!(stats.reused > 0 && stats.plans > 0 && stats.memo_hits > 0);
    }
}

#[test]
fn random_patches_shadow_fresh_evaluation_stored() {
    for (seed, rounds) in seeds() {
        let (patches, stats) = shadow(stored(), seed, rounds);
        eprintln!("stored seed {seed}: {patches} patches; {stats:?}");
        assert!(stats.reused > 0 && stats.plans > 0 && stats.memo_hits > 0);
    }
}
