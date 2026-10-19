//! The draw reuse (`pm_render::reuse`, docs/engine/perf.md "Draw reuse")
//! gives every frame the plan a fresh build makes. A renderer with the
//! reuse on and one without plan the same document through a long seeded
//! sequence of edits of every kind a layer's draws can depend on
//! (transforms, parents, groups, composition and project fields, asset
//! records and media paths, expressions, ids, structure, solo, mattes,
//! resyncs, replaced documents, time); every plan must be identical, and
//! the reuse must actually have been used.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pm_model::json::{Value, parse};
use pm_model::ops::Op;
use pm_model::patch::{Patch, PathPart};
use pm_model::{Document, Node};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_render::{AssetResolver, RenderRequest, Renderer};

/// Paths that can change between frames (a relinked asset).
#[derive(Clone, Default)]
struct Paths(Arc<Mutex<BTreeMap<String, PathBuf>>>);

impl AssetResolver for Paths {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.0.lock().unwrap().get(asset).cloned()
    }
}

/// A seeded generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
}

fn channels(x: f64, y: f64, opacity: f64, rotation: f64) -> String {
    let c = |v: f64| format!("{{\"v\":{v},\"kf\":[],\"expr\":null}}");
    format!(
        "{{\"position.x\":{},\"position.y\":{},\"anchor.x\":{},\"anchor.y\":{},\"scale.x\":{},\"scale.y\":{},\"rotation\":{},\"skew\":{},\"opacity\":{}}}",
        c(x),
        c(y),
        c(0.0),
        c(0.0),
        c(100.0),
        c(100.0),
        c(rotation),
        c(0.0),
        c(opacity)
    )
}

fn layer(id: &str, ty: &str, d: &str, x: f64, y: f64, extra: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"type\":\"{ty}\",\"name\":\"{id}\",\"from\":0,\"dur\":10,\"on\":true,\"lock\":false,\"blend\":\"normal\",\"fx\":[],\"masks\":[],\"parent\":null,\"p\":{},\"d\":{d}{extra}}}",
        channels(x, y, 90.0, 10.0)
    )
}

fn solid(id: &str, x: f64, y: f64, extra: &str) -> String {
    layer(
        id,
        "solid",
        "{\"color\":\"#3FA7D6\",\"w\":120,\"h\":80}",
        x,
        y,
        extra,
    )
}

fn document() -> Value {
    let mut layers = vec![layer("n1", "null", "{}", 300.0, 200.0, "")];
    layers.push(layer(
        "n2",
        "null",
        "{}",
        600.0,
        400.0,
        ",\"parent\":\"n1\"",
    ));
    layers.push(layer("g1", "group", "{}", 0.0, 0.0, ",\"collapsed\":false"));
    layers.push(layer("g2", "group", "{}", 50.0, 20.0, ",\"group\":\"g1\""));
    for i in 0..24 {
        let extra = match i % 6 {
            0 => ",\"parent\":\"n1\"".to_owned(),
            1 => ",\"group\":\"g1\"".to_owned(),
            2 => ",\"group\":\"g2\"".to_owned(),
            3 => ",\"parent\":\"n2\"".to_owned(),
            _ => String::new(),
        };
        layers.push(solid(
            &format!("s{i}"),
            40.0 * f64::from(i),
            25.0 * f64::from(i % 9),
            &extra,
        ));
    }
    for i in 0..4 {
        layers.push(layer(
            &format!("i{i}"),
            "image",
            "{\"asset\":\"a1\",\"fit\":\"contain\",\"w\":160,\"h\":120}",
            100.0 + 200.0 * f64::from(i),
            500.0,
            if i == 0 { ",\"parent\":\"n2\"" } else { "" },
        ));
    }
    layers.push(layer(
        "pc",
        "precomp",
        "{\"comp\":\"N\",\"w\":480,\"h\":270}",
        900.0,
        600.0,
        "",
    ));
    let nested: Vec<String> = (0..5)
        .map(|i| solid(&format!("ns{i}"), 60.0 * f64::from(i), 40.0, ""))
        .collect();
    let text = format!(
        "{{\"id\":\"Preuse\",\"name\":\"Reuse\",\"compId\":\"C\",\"assets\":{{\"a1\":{{\"id\":\"a1\",\"kind\":\"image\",\"name\":\"a.png\",\"format\":\"png\",\"w\":256,\"h\":256}}}},\"comps\":{{\"C\":{{\"name\":\"Main\",\"w\":1280,\"h\":720,\"fps\":30,\"dur\":10,\"bg\":\"#141821\",\"layers\":[{}]}},\"N\":{{\"name\":\"Nested\",\"w\":480,\"h\":270,\"fps\":30,\"dur\":10,\"bg\":\"#000000\",\"layers\":[{}]}}}}}}",
        layers.join(","),
        nested.join(",")
    );
    parse(&text).expect("the test document parses")
}

fn key(k: &str) -> PathPart {
    PathPart::Key(k.into())
}

fn layers_path(comp: &str) -> Vec<PathPart> {
    vec![key("comps"), key(comp), key("layers")]
}

fn layer_count(doc: &Document, comp: &str) -> usize {
    match doc.lookup(&layers_path(comp)).map(Node::kind) {
        Some(pm_model::NodeKind::Array(items)) => items.len(),
        _ => 0,
    }
}

fn layer_ids(doc: &Document, comp: &str) -> Vec<String> {
    match doc.lookup(&layers_path(comp)).map(Node::kind) {
        Some(pm_model::NodeKind::Array(items)) => items
            .iter()
            .map(|l| {
                l.get("id")
                    .and_then(|n| match n.kind() {
                        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                        _ => None,
                    })
                    .unwrap_or_default()
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn at(comp: &str, index: usize, rest: &[&str]) -> Vec<PathPart> {
    let mut p = layers_path(comp);
    p.push(PathPart::Index(index as f64));
    p.extend(rest.iter().map(|k| key(k)));
    p
}

fn value(text: &str) -> Value {
    parse(text).expect("a JSON value")
}

/// One random edit; returns a label for failure messages.
fn edit(doc: &mut Document, rng: &mut Rng, paths: &Paths, serial: &mut u32) -> String {
    let comp = if rng.below(6) == 0 { "N" } else { "C" };
    let n = layer_count(doc, comp);
    let i = rng.below(n);
    let ids = layer_ids(doc, comp);
    let any_id = |rng: &mut Rng| ids[rng.below(ids.len())].clone();
    let set = |doc: &mut Document, path: Vec<PathPart>, v: Value| {
        let _ = doc.apply(&[Patch::set(path, v)]);
    };
    let ops = |doc: &mut Document, op: Op| {
        let _ = doc.apply_ops(&[op]);
    };
    // Mostly moves (a drag), then every other kind.
    let kind = match rng.below(48) {
        r if r < 26 => r % 6,
        r => r - 20,
    };
    match kind {
        0..=5 => {
            let v = rng.unit() * 1200.0;
            set(
                doc,
                at(comp, i, &["p", "position.x", "v"]),
                Value::Number(v),
            );
            format!("move {comp}[{i}]")
        }
        6 => {
            set(
                doc,
                at(comp, i, &["p", "opacity", "v"]),
                Value::Number((rng.unit() * 100.0).round()),
            );
            format!("opacity {comp}[{i}]")
        }
        7 => {
            let target = if rng.below(3) == 0 {
                "null".to_owned()
            } else {
                format!("\"{}\"", any_id(rng))
            };
            set(doc, at(comp, i, &["parent"]), value(&target));
            format!("parent {comp}[{i}] = {target}")
        }
        8 => {
            let target = match rng.below(3) {
                0 => "null".to_owned(),
                1 => "\"g1\"".to_owned(),
                _ => "\"g2\"".to_owned(),
            };
            set(doc, at(comp, i, &["group"]), value(&target));
            format!("group {comp}[{i}] = {target}")
        }
        9 => {
            set(
                doc,
                vec![key("comps"), key(comp), key("w")],
                Value::Number(800.0 + (rng.below(5) * 120) as f64),
            );
            format!("{comp}.w")
        }
        10 => {
            set(
                doc,
                vec![key("assets"), key("a1"), key("w")],
                Value::Number(128.0 + (rng.below(4) * 64) as f64),
            );
            "asset a1.w".into()
        }
        11 => {
            let path = if rng.below(2) == 0 {
                "/media/a.png"
            } else {
                "/media/b.png"
            };
            paths
                .0
                .lock()
                .unwrap()
                .insert("a1".into(), PathBuf::from(path));
            format!("a1 → {path}")
        }
        12 => {
            let mut order: Vec<String> = ids.clone();
            let a = rng.below(order.len());
            let b = rng.below(order.len());
            order.swap(a, b);
            ops(
                doc,
                Op::Order {
                    path: layers_path(comp),
                    ids: order.iter().map(|s| s.as_str().into()).collect(),
                },
            );
            format!("order {comp} swap {a} {b}")
        }
        13 => {
            *serial += 1;
            let id = format!("new{serial}");
            let v = value(&solid(&id, rng.unit() * 1000.0, rng.unit() * 600.0, ""));
            ops(
                doc,
                Op::Insert {
                    path: layers_path(comp),
                    index: rng.below(n + 1) as u32,
                    value: v,
                },
            );
            format!("insert {id} in {comp}")
        }
        14 if n > 3 => {
            let id = any_id(rng);
            ops(
                doc,
                Op::Delete {
                    path: layers_path(comp),
                    id: id.as_str().into(),
                },
            );
            format!("delete {id} from {comp}")
        }
        15 => {
            let e = if rng.below(2) == 0 {
                "\"time * 40\""
            } else {
                "null"
            };
            set(doc, at(comp, i, &["p", "position.y", "expr"]), value(e));
            format!("expr {comp}[{i}] = {e}")
        }
        16 => {
            let on = rng.below(3) != 0;
            set(doc, at(comp, i, &["on"]), Value::Bool(on));
            format!("on {comp}[{i}] = {on}")
        }
        17 => {
            // Solo one layer, or clear every solo of the composition.
            if rng.below(4) == 0 {
                set(doc, at(comp, i, &["solo"]), Value::Bool(true));
                format!("solo {comp}[{i}]")
            } else {
                let patches: Vec<Patch> = (0..n)
                    .map(|j| Patch::set(at(comp, j, &["solo"]), Value::Bool(false)))
                    .collect();
                let _ = doc.apply(&patches);
                format!("unsolo {comp}")
            }
        }
        18 => {
            let m = if rng.below(2) == 0 {
                format!("\"{}\"", any_id(rng))
            } else {
                "null".into()
            };
            set(doc, at(comp, i, &["matteSource"]), value(&m));
            format!("matte {comp}[{i}] = {m}")
        }
        19 if rng.below(3) == 0 => {
            let id = any_id(rng);
            set(doc, at(comp, i, &["id"]), value(&format!("\"{id}\"")));
            format!("id {comp}[{i}] = {id} (maybe duplicated)")
        }
        20 => {
            set(
                doc,
                at(comp, i, &["dur"]),
                Value::Number(if rng.below(2) == 0 { 0.5 } else { 10.0 }),
            );
            format!("dur {comp}[{i}]")
        }
        21 => {
            set(
                doc,
                at(comp, i, &["compat"]),
                Value::Number(if rng.below(2) == 0 { 2.0 } else { 1.0 }),
            );
            format!("compat {comp}[{i}]")
        }
        22 => {
            set(
                doc,
                at(comp, i, &["threeD"]),
                Value::Bool(rng.below(2) == 0),
            );
            format!("threeD {comp}[{i}]")
        }
        23 => {
            // A resync: the whole document again, reconciled.
            let mut v = doc.to_value().unwrap();
            if let Some(Value::Object(root)) = Some(&mut v)
                && let Some(Value::Object(comps)) = root.get_key_mut(&"comps".into())
                && let Some(Value::Object(c)) = comps.get_key_mut(&"C".into())
            {
                c.insert("h", Value::Number(600.0 + (rng.below(3) * 60) as f64));
            }
            let revision = doc.revision() + 1;
            doc.resync(Some(v), revision);
            "resync".into()
        }
        24 => {
            // A replaced document (`DocumentUpdate::Replace`), in place.
            let mut v = doc.to_value().unwrap();
            if let Value::Object(root) = &mut v
                && let Some(Value::Object(comps)) = root.get_key_mut(&"comps".into())
                && let Some(Value::Object(c)) = comps.get_key_mut(&"C".into())
                && let Some(Value::Array(layers)) = c.get_key_mut(&"layers".into())
                && let Some(Value::Object(l)) = layers.last_mut()
                && let Some(Value::Object(p)) = l.get_key_mut(&"p".into())
                && let Some(Value::Object(x)) = p.get_key_mut(&"position.x".into())
            {
                x.insert("v", Value::Number(rng.unit() * 900.0));
            }
            let revision = doc.revision() + 1;
            *doc = Document::new(Some(v), revision);
            "replace".into()
        }
        _ => {
            set(doc, at(comp, i, &["d", "color"]), value("\"#E8553A\""));
            format!("color {comp}[{i}]")
        }
    }
}

fn plans_match(seed: u64, steps: usize) -> (u64, usize) {
    let mut with = Renderer::headless().expect("a Metal device");
    let mut without = Renderer::headless().expect("a Metal device");
    with.set_draw_reuse(true);
    let paths = Paths::default();
    paths
        .0
        .lock()
        .unwrap()
        .insert("a1".into(), PathBuf::from("/media/a.png"));
    let mut doc = Document::new(Some(document()), 1);
    let request = RenderRequest::at_scale(Size::new(1280, 720), 0.5, Precision::Float16);
    let mut rng = Rng(seed);
    let mut serial = 0;
    let mut seconds = 1.0;
    let mut last = String::from("start");
    let mut compared = 0;
    for step in 0..steps {
        if step > 0 {
            if rng.below(40) == 0 {
                seconds = if seconds == 1.0 { 2.0 } else { 1.0 };
                last = format!("time {seconds}");
            } else if rng.below(8) == 0 {
                last = "no edit".into();
            } else {
                last = edit(&mut doc, &mut rng, &paths, &mut serial);
            }
        }
        let time = Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap();
        let a = with.plan(&doc, "C", time, &request, &paths);
        let b = without.plan(&doc, "C", time, &request, &paths);
        assert_eq!(
            format!("{a:?}"),
            format!("{b:?}"),
            "seed {seed}, step {step}, after {last}"
        );
        compared += 1;
    }
    (with.draw_reuse_stats().reused, compared)
}

#[test]
fn reused_draws_plan_what_a_fresh_build_plans() {
    // PM_REUSE_FUZZ=<seeds> runs that many more seeds of 2,000 steps.
    let extra: u64 = std::env::var("PM_REUSE_FUZZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut runs: Vec<(u64, usize)> = [
        0x9e37_79b9_7f4a_7c15,
        0x2545_f491_4f6c_dd1d,
        0x1234_5678_9abc_def1,
    ]
    .into_iter()
    .map(|seed| (seed, 700))
    .collect();
    runs.extend((1..=extra).map(|k| (k.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1, 2000)));
    let mut reused = 0;
    for (seed, steps) in runs {
        let (r, compared) = plans_match(seed, steps);
        assert_eq!(compared, steps);
        reused += r;
    }
    eprintln!("{reused} layer draws reused");
    // The sequences exercise the reuse, not only the fresh path.
    assert!(reused > 10_000, "only {reused} layers reused");
}

#[test]
fn a_drag_reuses_every_layer_but_the_dragged_one() {
    let mut r = Renderer::headless().expect("a Metal device");
    r.set_draw_reuse(true);
    let paths = Paths::default();
    paths
        .0
        .lock()
        .unwrap()
        .insert("a1".into(), PathBuf::from("/media/a.png"));
    let mut doc = Document::new(Some(document()), 1);
    let request = RenderRequest::at_scale(Size::new(1280, 720), 0.5, Precision::Float16);
    let time = Ticks::new(TICKS_PER_SECOND).unwrap();
    // The first two frames at a time keep entries; drags reuse them.
    for _ in 0..2 {
        r.plan(&doc, "C", time, &request, &paths).unwrap();
    }
    let before = r.draw_reuse_stats();
    let index = layer_ids(&doc, "C")
        .iter()
        .position(|id| id == "s7")
        .unwrap();
    for k in 0..10 {
        doc.apply(&[Patch::set(
            at("C", index, &["p", "position.x", "v"]),
            Value::Number(100.0 + f64::from(k)),
        )])
        .unwrap();
        r.plan(&doc, "C", time, &request, &paths).unwrap();
    }
    let after = r.draw_reuse_stats();
    // s7 is planned each frame; every other kept layer is reused.
    assert_eq!(after.planned - before.planned, 10);
    assert!(after.reused - before.reused >= 10 * 30, "{after:?}");
}

mod support;

/// The parity corpus (every capture `support::corpus_dirs` finds): each
/// project is planned at its sample times with the reuse on and off, then
/// again after moving one layer at a time; the plans must be identical.
#[test]
fn corpus_plans_with_reused_draws_match() {
    use pm_render::all_builtin_definitions;
    let captures = support::corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let mut with = Renderer::headless().expect("a Metal device");
    let mut without = Renderer::headless().expect("a Metal device");
    with.set_draw_reuse(true);
    let mut projects = 0;
    let mut frames = 0;
    for (capture, dir) in captures.iter().flat_map(|c| {
        support::projects(c)
            .into_iter()
            .map(move |d| (c.clone(), d))
    }) {
        let mut p = support::load_project(&capture, &dir);
        let mut set = all_builtin_definitions();
        set.missing = support::extension_definitions(&p.document)
            .into_iter()
            .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
            .map(|id| id.as_str().into())
            .collect();
        with.set_definitions(&set).expect("built-in definitions");
        without.set_definitions(&set).expect("built-in definitions");
        // The composition's layers and where they are (stored or canonical).
        let root = p.document.root().unwrap();
        let base: Vec<PathPart> = if root.get("layers").is_some() {
            vec![key("layers")]
        } else {
            let mut b = layers_path(&p.composition);
            b.truncate(3);
            b
        };
        let movable: Vec<usize> = match p.document.lookup(&base).map(Node::kind) {
            Some(pm_model::NodeKind::Array(items)) => items
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    l.get("p")
                        .and_then(|p| p.get("position.x"))
                        .and_then(|x| x.get("v"))
                        .is_some_and(|v| matches!(v.kind(), pm_model::NodeKind::Number(_)))
                })
                .map(|(i, _)| i)
                .take(6)
                .collect(),
            _ => Vec::new(),
        };
        projects += 1;
        for sample in p.samples.iter().take(2) {
            let request = RenderRequest {
                output_size: Size::new(sample.width, sample.height),
                ..RenderRequest::at_scale(
                    Size::new(sample.width, sample.height),
                    1.0,
                    Precision::Float16,
                )
            };
            let request = RenderRequest {
                scale: None,
                ..request
            };
            let time = support::ticks(sample.time);
            let mut check = |doc: &Document, label: &str| {
                let a = with.plan(doc, &p.composition, time, &request, &p.resolver);
                let b = without.plan(doc, &p.composition, time, &request, &p.resolver);
                assert_eq!(
                    format!("{a:?}"),
                    format!("{b:?}"),
                    "{} at {}: {label}",
                    p.name,
                    sample.time
                );
            };
            check(&p.document, "first");
            check(&p.document, "again");
            check(&p.document, "kept");
            frames += 3;
            for (step, &i) in movable.iter().enumerate() {
                let mut path = base.clone();
                path.push(PathPart::Index(i as f64));
                path.extend(["p", "position.x", "v"].iter().map(|k| key(k)));
                let v = match p.document.lookup(&path).map(Node::kind) {
                    Some(pm_model::NodeKind::Number(v)) => *v,
                    _ => continue,
                };
                p.document
                    .apply(&[Patch::set(path, Value::Number(v + 7.0 + step as f64))])
                    .unwrap();
                check(&p.document, &format!("moved layer {i}"));
                check(&p.document, &format!("moved layer {i}, again"));
                frames += 2;
            }
        }
    }
    eprintln!(
        "{projects} projects, {frames} frames, {:?}",
        with.draw_reuse_stats()
    );
    assert!(with.draw_reuse_stats().reused > 0);
}
