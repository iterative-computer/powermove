//! The channel memo (`pm_eval` `channel_memo`, docs/engine/perf.md) gives
//! every frame the plan and the pixels a renderer without it gives. Two
//! renderers, one with the memo (the default) and one without, plan the same
//! document through a long seeded sequence of times (most frames move only
//! the time, so the memo answers) and edits of everything a channel's answer
//! depends on: group membership and nested groups (the layers' kept group
//! ancestry), keyframe values, times, and eases (linear, Bezier handles,
//! hold, temporal eases, springs), keys added and removed, the layer's
//! `from`, expressions switched on and off, falsy and missing channels,
//! malformed tracks (a `null` key, a non-object key, string values), layers
//! reordered, inserted, and deleted (their nodes move and keep their
//! revisions), resyncs, and replaced documents. Every plan must be identical,
//! sampled frames identical bit for bit, and the memo must have answered.

use std::path::PathBuf;

use pm_model::Document;
use pm_model::json::{Value, parse};
use pm_model::patch::{Patch, PathPart};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_render::{AssetResolver, RenderRequest, Renderer};

struct NoMedia;

impl AssetResolver for NoMedia {
    fn path(&self, _asset: &str, _record: Option<&pm_model::Node>) -> Option<PathBuf> {
        None
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
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One keyframe's easing, of every kind the evaluator prepares.
fn ease(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => String::new(),
        1 => format!(",\"eo\":[{:.3},{:.3}],\"ei\":[{:.3},{:.3}]", rng.unit(), rng.unit(), rng.unit(), rng.unit()),
        2 => ",\"hold\":true".into(),
        3 => format!(
            ",\"inInterp\":\"bezier\",\"outInterp\":\"bezier\",\"inEase\":{{\"speed\":{:.2},\"influence\":{:.1}}},\"outEase\":{{\"speed\":{:.2},\"influence\":{:.1}}}",
            rng.unit() * 200.0,
            10.0 + rng.unit() * 80.0,
            rng.unit() * 200.0,
            10.0 + rng.unit() * 80.0
        ),
        4 => ",\"spring\":{\"stiffness\":180,\"damping\":12,\"mass\":1}".into(),
        _ => ",\"inInterp\":\"linear\",\"outInterp\":\"linear\",\"inEase\":{\"speed\":0,\"influence\":16.67},\"outEase\":{\"speed\":0,\"influence\":16.67}".into(),
    }
}

/// A channel (`L.p[key]`) of every form `PM.ev` reads.
fn channel(rng: &mut Rng, base: f64, span: f64) -> String {
    match rng.below(16) {
        0 => format!("{{\"v\":{base}}}"),
        1 => format!("{{\"v\":{base},\"kf\":[]}}"),
        2 => format!("{{\"v\":{base},\"kf\":[],\"expr\":\"value + time * 20\"}}"),
        3 => "0".into(),
        4 => "null".into(),
        5 => format!(
            "{{\"v\":{base},\"kf\":[{{\"t\":0,\"v\":{base}}},{{\"t\":1,\"v\":{base}}}],\"expr\":\"value * 1.5\"}}"
        ),
        6 => format!("{{\"v\":{base},\"kf\":[null,{{\"t\":1,\"v\":{base}}}]}}"),
        7 => format!("{{\"v\":{base},\"kf\":[{{\"t\":0,\"v\":{base}}},3]}}"),
        8 => format!("{{\"v\":{base},\"kf\":[{{\"t\":0,\"v\":\"a\"}},{{\"t\":1,\"v\":\"b\"}}]}}"),
        _ => {
            let n = 1 + rng.below(4);
            let mut t = rng.unit() * 2.0 - 0.5;
            let keys: Vec<String> = (0..n)
                .map(|_| {
                    let key = format!(
                        "{{\"t\":{t:.4},\"v\":{:.3}{}}}",
                        base + (rng.unit() - 0.5) * span,
                        ease(rng)
                    );
                    t += 0.25 + rng.unit() * 3.0;
                    key
                })
                .collect();
            format!("{{\"v\":{base},\"kf\":[{}]}}", keys.join(","))
        }
    }
}

fn layer(rng: &mut Rng, id: &str, extra: &str) -> String {
    let from = match rng.below(12) {
        0 => "\"from\":-1.5".to_owned(),
        1 => "\"from\":\"x\"".to_owned(),
        2 => "\"from\":2".to_owned(),
        _ => "\"from\":0".to_owned(),
    };
    let p = if rng.below(30) == 0 {
        "null".to_owned()
    } else {
        format!(
            "{{\"opacity\":{},\"position.x\":{},\"position.y\":{},\"scale.x\":{},\"scale.y\":{},\"rotation\":{},\"anchor.x\":{{\"v\":0}},\"anchor.y\":{{\"v\":0}}}}",
            channel(rng, 100.0, 120.0),
            channel(rng, 640.0, 900.0),
            channel(rng, 360.0, 500.0),
            channel(rng, 100.0, 80.0),
            channel(rng, 100.0, 80.0),
            channel(rng, 0.0, 360.0),
        )
    };
    format!(
        "{{\"id\":\"{id}\",\"type\":\"solid\",\"name\":\"{id}\",{from},\"dur\":12,\"on\":true,\"d\":{{\"color\":\"#{:06x}\",\"w\":{},\"h\":{}}},\"p\":{p}{extra}}}",
        rng.next() & 0xff_ffff,
        40 + rng.below(300),
        30 + rng.below(200),
    )
}

fn document(rng: &mut Rng) -> Value {
    let mut layers = vec![
        "{\"id\":\"n1\",\"type\":\"null\",\"name\":\"n1\",\"from\":0,\"dur\":12,\"on\":true,\"d\":{},\"p\":{\"position.x\":{\"v\":300,\"kf\":[{\"t\":0,\"v\":300},{\"t\":4,\"v\":700}]},\"position.y\":{\"v\":200}}}".to_owned(),
        "{\"id\":\"g1\",\"type\":\"group\",\"name\":\"g1\",\"from\":0,\"dur\":12,\"on\":true,\"d\":{},\"p\":{\"opacity\":{\"v\":80,\"kf\":[{\"t\":0,\"v\":100},{\"t\":6,\"v\":40}]}}}".to_owned(),
        "{\"id\":\"g2\",\"type\":\"group\",\"name\":\"g2\",\"group\":\"g1\",\"from\":0,\"dur\":12,\"on\":true,\"d\":{},\"p\":{\"position.x\":{\"v\":40,\"kf\":[{\"t\":0,\"v\":0},{\"t\":5,\"v\":200}]}}}".to_owned(),
    ];
    for i in 0..36 {
        let extra = match i % 6 {
            0 => ",\"parent\":\"n1\"",
            1 => ",\"group\":\"g1\"",
            2 => ",\"group\":\"g2\"",
            _ => "",
        };
        layers.push(layer(rng, &format!("s{i}"), extra));
    }
    let nested: Vec<String> = (0..4).map(|i| layer(rng, &format!("ns{i}"), "")).collect();
    layers.push("{\"id\":\"pc\",\"type\":\"precomp\",\"name\":\"pc\",\"from\":0,\"dur\":12,\"on\":true,\"d\":{\"comp\":\"N\",\"w\":480,\"h\":270},\"p\":{\"position.x\":{\"v\":900},\"position.y\":{\"v\":500,\"kf\":[{\"t\":0,\"v\":500},{\"t\":3,\"v\":300}]}}}".to_owned());
    let text = format!(
        "{{\"id\":\"Pmemo\",\"name\":\"Memo\",\"compId\":\"C\",\"assets\":{{}},\"comps\":{{\"C\":{{\"name\":\"Main\",\"w\":1280,\"h\":720,\"fps\":30,\"dur\":10,\"bg\":\"#141821\",\"layers\":[{}]}},\"N\":{{\"name\":\"Nested\",\"w\":480,\"h\":270,\"fps\":30,\"dur\":10,\"bg\":\"#000000\",\"layers\":[{}]}}}}}}",
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

fn layers_of(doc: &Document, comp: &str) -> Vec<Value> {
    match doc.lookup(&layers_path(comp)).map(|n| n.to_value()) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// One random edit of something a channel's answer depends on.
fn edit(doc: &mut Document, rng: &mut Rng) -> String {
    let comp = if rng.below(6) == 0 { "N" } else { "C" };
    let mut layers = layers_of(doc, comp);
    if layers.is_empty() {
        return "nothing".into();
    }
    let i = rng.below(layers.len());
    let channels = ["opacity", "position.x", "position.y", "scale.x", "rotation"];
    let ch = channels[rng.below(channels.len())];
    // A patch through a missing or primitive member fails as it does in
    // JavaScript; the document still moves to the next revision.
    let set = |doc: &mut Document, path: Vec<PathPart>, v: Value| {
        let _ = doc.apply(&[Patch::set(path, v)]);
    };
    let at = |rest: &[&str]| {
        let mut p = layers_path(comp);
        p.push(PathPart::Index(i as f64));
        p.extend(rest.iter().map(|k| key(k)));
        p
    };
    match rng.below(11) {
        0 => {
            let v = parse(&channel(rng, 100.0, 200.0)).unwrap();
            set(doc, at(&["p", ch]), v);
            format!("channel {comp}[{i}].{ch}")
        }
        1 => {
            let v = Value::Number((rng.unit() * 4.0 - 1.0).round());
            set(doc, at(&["from"]), v);
            format!("from {comp}[{i}]")
        }
        2 => {
            set(
                doc,
                at(&["p", ch, "expr"]),
                Value::String("value + 7".into()),
            );
            format!("expr on {comp}[{i}].{ch}")
        }
        3 => {
            set(doc, at(&["p", ch, "expr"]), Value::Null);
            format!("expr off {comp}[{i}].{ch}")
        }
        4 => {
            // A keyframe's value (the first key, when there is one).
            set(
                doc,
                at(&["p", ch, "kf", "0", "v"]),
                Value::Number(rng.unit() * 500.0),
            );
            format!("key value {comp}[{i}].{ch}")
        }
        5 => {
            // Reordered: every layer node moves, revisions kept.
            let a = rng.below(layers.len());
            let b = rng.below(layers.len());
            layers.swap(a, b);
            set(doc, layers_path(comp), Value::Array(layers));
            format!("swap {comp}[{a}] [{b}]")
        }
        6 => {
            let removed = layers.remove(i);
            layers.insert(rng.below(layers.len() + 1), removed);
            if rng.below(2) == 0 && layers.len() > 4 {
                layers.remove(rng.below(layers.len()));
            }
            let id = format!("x{}", rng.next() % 10_000);
            layers.insert(
                rng.below(layers.len() + 1),
                parse(&layer(rng, &id, "")).unwrap(),
            );
            set(doc, layers_path(comp), Value::Array(layers));
            format!("move, delete, insert in {comp}")
        }
        9 => {
            // Group membership: what the layer's ancestry reads.
            let g = ["\"g1\"", "\"g2\"", "null", "\"s3\"", "\"missing\""][rng.below(5)];
            set(doc, at(&["group"]), parse(g).unwrap());
            format!("group {comp}[{i}] = {g}")
        }
        10 => {
            // A group that stops (or starts) being one.
            let ty = ["\"group\"", "\"solid\""][rng.below(2)];
            set(doc, at(&["type"]), parse(ty).unwrap());
            format!("type {comp}[{i}] = {ty}")
        }
        7 => {
            let v = doc.to_value().unwrap();
            let revision = doc.revision() + 1;
            doc.resync(Some(v), revision);
            "resync".into()
        }
        _ => {
            let v = doc.to_value().unwrap();
            let revision = doc.revision() + 1;
            *doc = Document::new(Some(v), revision);
            "replace".into()
        }
    }
}

fn run(seed: u64, steps: usize) -> u64 {
    let mut with = Renderer::headless().expect("a Metal device");
    let mut without = Renderer::headless().expect("a Metal device");
    without.set_channel_memo(false);
    let mut rng = Rng(seed);
    let mut doc = Document::new(Some(document(&mut rng)), 1);
    let request = RenderRequest::at_scale(Size::new(1280, 720), 0.5, Precision::Float16);
    let mut last = String::from("start");
    for step in 0..steps {
        if step > 0 && rng.below(5) == 0 {
            last = edit(&mut doc, &mut rng);
        }
        // Times inside and outside every layer's span, on and between keys.
        let seconds = match rng.below(5) {
            0 => (rng.below(9) as f64) * 0.5 - 1.0,
            _ => rng.unit() * 12.0 - 1.0,
        };
        let time = Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap();
        let a = with.plan(&doc, "C", time, &request, &NoMedia);
        let b = without.plan(&doc, "C", time, &request, &NoMedia);
        assert_eq!(
            format!("{a:?}"),
            format!("{b:?}"),
            "seed {seed:#x}, step {step}, {seconds} s, after {last}"
        );
        if step % 40 == 7 {
            let ra = with.render(&doc, "C", time, &request, &NoMedia);
            let rb = without.render(&doc, "C", time, &request, &NoMedia);
            match (ra, rb) {
                (Ok(ra), Ok(rb)) => {
                    let pa = with.read_rgba(&ra).unwrap();
                    let pb = without.read_rgba(&rb).unwrap();
                    assert!(
                        pa.iter().zip(&pb).all(|(x, y)| x.to_bits() == y.to_bits()),
                        "pixels differ: seed {seed:#x}, step {step}, after {last}"
                    );
                }
                (a, b) => assert_eq!(format!("{:?}", a.err()), format!("{:?}", b.err())),
            }
        }
    }
    assert_eq!(without.channel_memo_stats(), (0, 0));
    with.channel_memo_stats().0
}

#[test]
fn memoized_channels_plan_and_draw_what_reading_them_does() {
    // PM_MEMO_FUZZ=<seeds> runs that many more seeds of 2,000 steps.
    let extra: u64 = std::env::var("PM_MEMO_FUZZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut runs: Vec<(u64, usize)> = [
        0x9e37_79b9_7f4a_7c15u64,
        0x2545_f491_4f6c_dd1d,
        0x5851_f42d_4c95_7f2d,
    ]
    .into_iter()
    .map(|seed| (seed, 400))
    .collect();
    runs.extend((1..=extra).map(|k| (k.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1, 2000)));
    let mut hits = 0;
    for (seed, steps) in runs {
        hits += run(seed, steps);
    }
    eprintln!("{hits} channel answers from the memo");
    assert!(hits > 50_000, "only {hits} memo answers");
}

/// An unchanged document keeps its entries across frames; any edit, a
/// clone, or another document drops them.
#[test]
fn the_memo_answers_only_its_documents_edition() {
    let mut r = Renderer::headless().expect("a Metal device");
    let mut rng = Rng(7);
    let mut doc = Document::new(Some(document(&mut rng)), 1);
    let request = RenderRequest::at_scale(Size::new(640, 360), 0.25, Precision::Float16);
    let t = |s: f64| Ticks::new((s * TICKS_PER_SECOND as f64) as i64).unwrap();
    // The first plan at an edition only notes it; the second fills; the
    // third answers.
    r.plan(&doc, "C", t(0.5), &request, &NoMedia).unwrap();
    assert_eq!(r.channel_memo_stats(), (0, 0));
    r.plan(&doc, "C", t(1.0), &request, &NoMedia).unwrap();
    let (h1, m1) = r.channel_memo_stats();
    assert!(m1 > 0);
    r.plan(&doc, "C", t(1.5), &request, &NoMedia).unwrap();
    let (h2, m2) = r.channel_memo_stats();
    assert!(
        h2 > h1 && m2 == m1,
        "an unchanged document answers ({h1}/{m1} → {h2}/{m2})"
    );
    // An edit: a new edition, nothing answered from the old one.
    doc.apply(&[Patch::set(
        vec![key("comps"), key("C"), key("w")],
        Value::Number(1280.0),
    )])
    .unwrap();
    r.plan(&doc, "C", t(1.5), &request, &NoMedia).unwrap();
    assert_eq!(r.channel_memo_stats(), (h2, m2));
    // A clone is other nodes: another edition.
    let copy = doc.clone();
    assert_ne!(copy.edition(), doc.edition());
    assert_eq!(copy, doc);
    r.plan(&copy, "C", t(1.5), &request, &NoMedia).unwrap();
    assert_eq!(r.channel_memo_stats(), (h2, m2));
}
