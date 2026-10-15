//! Distribute (`align::distribute`): no TypeScript counterpart exists (the
//! alignment strip only aligns), so the kernel is checked against its own
//! definition: after the moves are applied, the chosen edges or centers sit
//! at equal steps (or the gaps between neighbours are equal), the two
//! outermost layers stay, the other axis is untouched, and parented layers
//! move in their parent's space.

use pm_edit::align::{self, Mode, Spread};
use pm_edit::{Edit, IdSource};
use pm_eval::bounds::NoHost;
use pm_eval::space3d::Rect;
use pm_eval::text_geom::{MeasuredLine, TextMeasure};
use pm_model::json::parse;
use pm_model::{Document, Patch, PathPart, Value};
use pm_text::compat::{GlyphLayout, TextContent};

struct NoText;

impl TextMeasure for NoText {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<MeasuredLine> {
        Vec::new()
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        GlyphLayout::default()
    }
    fn spaced_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
}

fn channel(v: f64) -> String {
    format!(r#"{{"v":{v},"kf":[],"expr":null}}"#)
}

/// A shape layer `w`×`h` at (`x`, `y`) with scale `s` percent.
fn shape(id: &str, x: f64, y: f64, w: f64, h: f64, s: f64, parent: Option<&str>) -> String {
    let parent = parent.map_or("null".to_owned(), |p| format!(r#""{p}""#));
    format!(
        r##"{{"id":"{id}","name":"{id}","type":"shape","from":0,"dur":5,"on":true,"lock":false,"threeD":false,"parent":{parent},"group":null,
        "p":{{"position.x":{},"position.y":{},"anchor.x":{},"anchor.y":{},"scale.x":{},"scale.y":{},"rotation":{},"skew":{},"opacity":{}}},
        "d":{{"shape":"rect","w":{w},"h":{h},"color":"#ff0000"}}}}"##,
        channel(x),
        channel(y),
        channel(0.0),
        channel(0.0),
        channel(s),
        channel(s),
        channel(0.0),
        channel(0.0),
        channel(100.0),
    )
}

fn document(layers: &[String]) -> Document {
    let text = format!(
        r#"{{"id":"P","name":"P","compId":"C","compName":"C","w":1920,"h":1080,"fps":30,"dur":5,"layers":[{}],"comps":{{}},"params":{{}}}}"#,
        layers.join(",")
    );
    let canonical = pm_model::canonical::canonical_project(parse(&text).unwrap());
    Document::new(Some(canonical), 1)
}

/// The world boxes of every layer at time 0.
fn boxes(doc: &Document) -> Vec<Rect> {
    let mut edit = Edit::new(doc, "C", "C", None, IdSource::default()).unwrap();
    edit.read(|s| {
        let mut out = Vec::new();
        for l in s.layers() {
            let b = s.bounds(l, 0.0, &mut NoText, &mut NoHost)?.unwrap();
            let m = s.world_matrix(l, 0.0)?;
            let xs = [b.x0, b.x1].map(|x| m[0] * x + m[4]);
            let ys = [b.y0, b.y1].map(|y| m[3] * y + m[5]);
            out.push(Rect {
                x0: xs[0].min(xs[1]),
                x1: xs[0].max(xs[1]),
                y0: ys[0].min(ys[1]),
                y1: ys[0].max(ys[1]),
            });
        }
        Ok(out)
    })
    .unwrap()
}

/// Applies the moves as the renderer's `set_property` commands would (static
/// channels: the value).
fn apply(doc: &Document, moves: &[align::Move]) -> Document {
    let patches: Vec<Patch> = moves
        .iter()
        .map(|m| {
            Patch::set(
                vec![
                    PathPart::from("comps"),
                    PathPart::from("C"),
                    PathPart::from("layers"),
                    PathPart::from(m.layer),
                    PathPart::from("p"),
                    PathPart::from(m.channel),
                    PathPart::from("v"),
                ],
                Value::Number(m.value),
            )
        })
        .collect();
    let mut next = doc.clone();
    let revision = next.revision() + 1;
    next.apply_at(&patches, revision).unwrap();
    next
}

fn run(doc: &Document, layers: &[usize], spread: Spread) -> Vec<align::Move> {
    let mut edit = Edit::new(doc, "C", "C", None, IdSource::default()).unwrap();
    align::distribute(&mut edit, layers, spread, 0.0, &mut NoText, &mut NoHost).unwrap()
}

/// Four boxes of different widths, out of order, one under a parent scaled
/// to 200%.
fn scene() -> Document {
    document(&[
        shape("A", 100.0, 200.0, 80.0, 40.0, 100.0, None),
        shape("B", 900.0, 300.0, 200.0, 60.0, 100.0, None),
        shape("P", 0.0, 0.0, 10.0, 10.0, 200.0, None),
        // World center (2 × 210, 2 × 120) = (420, 240), world width 2 × 30.
        shape("C", 210.0, 120.0, 30.0, 30.0, 100.0, Some("P")),
        shape("D", 1500.0, 500.0, 120.0, 120.0, 100.0, None),
    ])
}

const EPS: f64 = 2e-3; // values are rounded to three decimals

#[test]
fn centers_fall_at_equal_steps() {
    let doc = scene();
    let selection = [0, 1, 3, 4];
    let moves = run(&doc, &selection, Spread::Position(Mode::HCenter));
    assert!(moves.iter().all(|m| m.channel == "position.x"));
    // The outermost (A and D) stay.
    assert!(moves.iter().all(|m| m.layer != 0 && m.layer != 4));
    let before = boxes(&doc);
    let after = boxes(&apply(&doc, &moves));
    let center = |b: &Rect| (b.x0 + b.x1) / 2.0;
    let mut centers: Vec<f64> = selection.iter().map(|&i| center(&after[i])).collect();
    centers.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let step = (centers[3] - centers[0]) / 3.0;
    for k in 1..3 {
        assert!(
            (centers[k] - (centers[0] + step * k as f64)).abs() < EPS,
            "{centers:?}"
        );
    }
    assert_eq!(center(&after[0]), center(&before[0]));
    assert_eq!(center(&after[4]), center(&before[4]));
    // The parented layer moved in its parent's space: half the world delta.
    let c = moves.iter().find(|m| m.layer == 3).unwrap();
    let world = center(&after[3]) - center(&before[3]);
    assert!((world - 2.0 * (c.value - 210.0)).abs() < EPS);
    // The vertical axis is untouched.
    for &i in &selection {
        assert_eq!(after[i].y0, before[i].y0);
    }
}

#[test]
fn gaps_become_equal() {
    let doc = scene();
    let selection = [4, 3, 1, 0];
    let moves = run(&doc, &selection, Spread::HorizontalSpace);
    // Commands follow the selection order.
    let order: Vec<usize> = moves.iter().map(|m| m.layer).collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|l| selection.iter().position(|s| s == l));
    assert_eq!(order, sorted);
    let after = boxes(&apply(&doc, &moves));
    let mut spans: Vec<Rect> = [0usize, 1, 3, 4].iter().map(|&i| after[i]).collect();
    spans.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap());
    let gaps: Vec<f64> = spans.windows(2).map(|w| w[1].x0 - w[0].x1).collect();
    for g in &gaps {
        assert!((g - gaps[0]).abs() < EPS, "{gaps:?}");
    }
}

#[test]
fn vertical_edges_and_degenerate_selections() {
    let doc = scene();
    let moves = run(&doc, &[0, 1, 4], Spread::Position(Mode::Top));
    assert!(
        moves
            .iter()
            .all(|m| m.channel == "position.y" && m.layer == 1)
    );
    let after = boxes(&apply(&doc, &moves));
    let tops = [after[0].y0, after[1].y0, after[4].y0];
    assert!(
        ((tops[1] - tops[0]) - (tops[2] - tops[1])).abs() < EPS,
        "{tops:?}"
    );
    // Fewer than three layers, or already distributed: nothing moves.
    assert!(run(&doc, &[0, 1], Spread::HorizontalSpace).is_empty());
    let settled = apply(
        &doc,
        &run(&doc, &[0, 1, 3, 4], Spread::Position(Mode::Left)),
    );
    let position = |doc: &Document, layer: usize| {
        let path = ["comps", "C", "layers"]
            .map(PathPart::from)
            .into_iter()
            .chain([
                PathPart::from(layer),
                PathPart::from("p"),
                PathPart::from("position.x"),
                PathPart::from("v"),
            ])
            .collect::<Vec<_>>();
        doc.lookup(&path).unwrap().to_value().as_f64().unwrap()
    };
    for m in run(&settled, &[0, 1, 3, 4], Spread::Position(Mode::Left)) {
        assert!((m.value - position(&settled, m.layer)).abs() < EPS, "{m:?}");
    }
    // Locked layers are left out: two boxes remain, nothing moves.
    let locked = document(&[
        shape("A", 100.0, 200.0, 80.0, 40.0, 100.0, None),
        shape("B", 900.0, 300.0, 200.0, 60.0, 100.0, None),
        shape("D", 1500.0, 500.0, 120.0, 120.0, 100.0, None)
            .replace(r#""lock":false"#, r#""lock":true"#),
    ]);
    assert!(run(&locked, &[0, 1, 2], Spread::HorizontalSpace).is_empty());
}
