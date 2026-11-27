//! Native answers: `pm-eval` on the stored document, with `pm-expr` through
//! pm-wasm's production expression engine and level 1 text measured over the
//! native font registry (CoreText).

use std::collections::HashMap;
use std::sync::Arc;

use pm_eval::bounds::{Bounds, HOST_ONLY, LayerHost};
use pm_eval::retime::Retiming;
use pm_eval::text_geom::{TextMeasure, text_content};
use pm_eval::text_measure::CompatMeasure;
use pm_eval::{Comp, Evaluator, JsResult, JsValue, Scope};
use pm_model::json::{Map, Value};
use pm_model::{Document, JsString, Node};
use pm_text::compat::{GlyphLayout, Unit};
use pm_text::font::FontResolver;
use pm_wasm::expr::PmExpressions;

use super::compare::{Answer, js_to_value, numbers, object};
use super::items::{Item, Kind};

/// Answers nothing only three.js can answer (the defaults throw `HostOnly`).
struct NoHost;
impl LayerHost for NoHost {}

pub const HOST: &str = "three.js answers in Phase 1 (3D objects, model groups)";
pub const RECIPES: &str = "model group recipe parts need pm-geom3d (Phase 3c)";

fn answer(r: JsResult<Value>) -> Answer {
    match r {
        Ok(v) => Answer::Value(v),
        Err(e) if e.kind == HOST_ONLY => Answer::Unanswered(HOST),
        Err(e) => Answer::Threw(e.message.to_string()),
    }
}

pub fn bounds_value(b: &Bounds) -> Value {
    object([
        ("x0", Value::Number(b.x0)),
        ("y0", Value::Number(b.y0)),
        ("x1", Value::Number(b.x1)),
        ("y1", Value::Number(b.y1)),
        ("w", Value::Number(b.w)),
        ("h", Value::Number(b.h)),
        ("ax", Value::Number(b.ax)),
        ("ay", Value::Number(b.ay)),
    ])
}

fn unit_value(u: &Unit, characters: bool) -> Value {
    let mut m = Map::new();
    m.insert("text", Value::String(JsString::from(u.text.as_str())));
    m.insert("x", Value::Number(u.x));
    m.insert("y", Value::Number(u.y));
    m.insert("w", Value::Number(u.w));
    m.insert("line", Value::Number(u.line as f64));
    if characters {
        m.insert(
            "lineUnit",
            u.line_unit.map_or(Value::Null, |x| Value::Number(x as f64)),
        );
        m.insert(
            "word",
            u.word.map_or(Value::Null, |x| Value::Number(x as f64)),
        );
    }
    m.insert("sourceStart", Value::Number(u.source_start as f64));
    m.insert("index", Value::Number(u.index as f64));
    Value::Object(m)
}

pub fn layout_value(l: &GlyphLayout) -> Value {
    object([
        (
            "characters",
            Value::Array(l.characters.iter().map(|u| unit_value(u, true)).collect()),
        ),
        (
            "words",
            Value::Array(l.words.iter().map(|u| unit_value(u, false)).collect()),
        ),
        (
            "lines",
            Value::Array(l.lines.iter().map(|u| unit_value(u, false)).collect()),
        ),
    ])
}

fn content_value(c: &pm_eval::evaluator::Content) -> Value {
    Value::Object(
        c.iter()
            .filter(|(_, v)| !matches!(v, JsValue::Undefined))
            .map(|(k, v)| (k.clone(), js_to_value(v)))
            .collect::<Map>(),
    )
}

/// A layer's `allProps`: keys and the channel nodes they name.
type PropList<'d> = Vec<(JsString, Option<&'d Node>)>;

/// One dump's answers, in item order. A fresh evaluator per dump, as the
/// capture evaluated each sample of a freshly settled app.
pub fn answer_dump<R: FontResolver>(
    doc: &Document,
    items: &[Item],
    measure: &mut CompatMeasure<R>,
) -> Vec<Answer> {
    let mut ev = Evaluator::new().with_expressions(Arc::new(PmExpressions));
    let mut comps: HashMap<String, Comp<'_>> = HashMap::new();
    for it in items {
        if !comps.contains_key(&it.comp)
            && let Ok(c) = Comp::resolve(doc, &it.comp)
        {
            comps.insert(it.comp.clone(), c);
        }
        if !comps.contains_key(&it.open)
            && let Ok(c) = Comp::resolve(doc, &it.open)
        {
            comps.insert(it.open.clone(), c);
        }
    }
    // `allProps` per (composition path, layer), computed once per layer.
    let mut props: HashMap<(String, usize), PropList<'_>> = HashMap::new();
    items
        .iter()
        .map(|it| {
            if let Kind::Doc(v) = &it.kind {
                return Answer::Value(v.clone());
            }
            let (Some(comp), Some(project)) = (comps.get(&it.comp), comps.get(&it.open)) else {
                return Answer::Threw(format!("no composition {}", it.comp));
            };
            let layer = it.layer.and_then(|i| comp.layers().get(i));
            let mut scope = Scope::new(&mut ev, comp, project);
            let t = it.time;
            let Some(layer) = layer else {
                return match it.kind {
                    Kind::Scene => scene(&mut scope, t),
                    _ => Answer::Threw("no layer".into()),
                };
            };
            match &it.kind {
                Kind::Doc(_) => unreachable!(),
                Kind::Active => answer(scope.active(layer, t).map(Value::Bool)),
                Kind::Channel { key, in_p } => {
                    let got = if *in_p {
                        scope.ev.ev(comp, Some(layer), key, t)
                    } else {
                        let entries = props
                            .entry((it.path.clone(), it.layer.unwrap_or(0)))
                            .or_insert_with(|| {
                                scope
                                    .ev
                                    .all_props(layer)
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|p| (p.key, p.prop.node()))
                                    .collect()
                            });
                        let prop = entries.iter().find(|(k, _)| k == key).and_then(|(_, n)| *n);
                        scope.ev.ev_p(comp, Some(layer), prop, t, key)
                    };
                    answer(got.map(|v| js_to_value(&v)))
                }
                Kind::ExprError { key } => Answer::Value(
                    scope
                        .ev
                        .expression_error(&it.comp, &it.layer_id, &key.to_string_lossy())
                        .map_or(Value::Null, |m| Value::String(JsString::from(m))),
                ),
                Kind::LayerFields { raw, evaluated } => {
                    let mut m = raw.clone();
                    for (field, key) in evaluated {
                        let prop = layer.get(field);
                        match scope
                            .ev
                            .ev_p(comp, Some(layer), prop, t, &JsString::from(*key))
                        {
                            Ok(v) => {
                                m.insert(*field, js_to_value(&v));
                            }
                            Err(e) => return answer(Err(e)),
                        }
                    }
                    Answer::Value(Value::Object(m))
                }
                Kind::Content => answer(scope.ev.resolve_content(comp, layer, t).map(|c| {
                    Value::Object(
                        c.iter()
                            .map(|(k, v)| (k.clone(), js_to_value(v)))
                            .collect::<Map>(),
                    )
                })),
                Kind::LocalMatrix => answer(scope.local_matrix(layer, t).map(|m| numbers(&m))),
                Kind::WorldMatrix => answer(scope.world_matrix(layer, t).map(|m| numbers(&m))),
                Kind::WorldOpacity => answer(scope.world_opacity(layer, t).map(Value::Number)),
                Kind::PlaneMatrix => {
                    answer(scope.plane_matrix(layer, t, false).map(|m| numbers(&m)))
                }
                Kind::Bounds => answer(
                    scope
                        .bounds(layer, t, measure, &mut NoHost)
                        .map(|b| b.map_or(Value::Null, |b| bounds_value(&b))),
                ),
                Kind::TextLayout => answer(
                    scope
                        .text_layout(layer, t, measure)
                        .map(|l| layout_value(&l)),
                ),
                Kind::TextRaster => answer(scope.text_geometry(layer, t, measure).map(|g| {
                    let s = g.selection;
                    object([
                        ("w", Value::Number(g.w)),
                        ("h", Value::Number(g.h)),
                        ("anchorX", Value::Number(g.anchor_x)),
                        ("anchorY", Value::Number(g.anchor_y)),
                        ("density", Value::Number(g.density(1.0))),
                        (
                            "selection",
                            object([
                                ("x0", Value::Number(s.x0)),
                                ("y0", Value::Number(s.y0)),
                                ("x1", Value::Number(s.x1)),
                                ("y1", Value::Number(s.y1)),
                                ("w", Value::Number(s.w)),
                                ("h", Value::Number(s.h)),
                            ]),
                        ),
                    ])
                })),
                Kind::TextAnimation => answer(text_animation(&mut scope, layer, t, measure)),
                Kind::CaptionBounds => answer(scope.caption_bounds(layer, t, measure).map(|b| {
                    object([
                        ("x0", Value::Number(b.x0)),
                        ("y0", Value::Number(b.y0)),
                        ("x1", Value::Number(b.x1)),
                        ("y1", Value::Number(b.y1)),
                        ("w", Value::Number(b.x1 - b.x0)),
                        ("h", Value::Number(b.y1 - b.y0)),
                        ("ax", Value::Number(0.0)),
                        ("ay", Value::Number(0.0)),
                    ])
                })),
                Kind::Paths { .. } => {
                    answer(scope.path_geometry(layer, t).map(|(paths, matrices, _)| {
                        Value::Array(
                            paths
                                .iter()
                                .zip(&matrices)
                                .map(|(p, m)| {
                                    let mut values: Map = p
                                        .values
                                        .iter()
                                        .map(|(k, v)| (k.clone(), js_to_value(v)))
                                        .collect();
                                    values.insert(
                                        "vertices",
                                        Value::Array(
                                            p.vertices
                                                .iter()
                                                .map(|v| {
                                                    Value::Object(
                                                        v.iter()
                                                            .map(|(k, x)| {
                                                                (k.clone(), Value::Number(*x))
                                                            })
                                                            .collect(),
                                                    )
                                                })
                                                .collect(),
                                        ),
                                    );
                                    object([
                                        ("id", js_to_value(&p.id)),
                                        ("values", Value::Object(values)),
                                        ("matrix", numbers(m)),
                                    ])
                                })
                                .collect(),
                        )
                    }))
                }
                Kind::SourceTime { legacy } => {
                    let mode = if *legacy {
                        Retiming::Legacy
                    } else {
                        Retiming::Engine
                    };
                    answer(
                        scope
                            .source_time_with(layer, t, mode, None)
                            .map(Value::Number),
                    )
                }
                Kind::Scene => scene(&mut scope, t),
            }
        })
        .collect()
}

fn scene(scope: &mut Scope<'_, '_>, t: f64) -> Answer {
    match scope.composition_scene(t) {
        Ok(s) if !s.recipe_parts.is_empty() => Answer::Unanswered(RECIPES),
        other => answer(other.map(|s| s.scene)),
    }
}

/// `textAnimationState` and `animatedGlyphs` over our own `PM.textLayout`.
fn text_animation<'d, M: TextMeasure>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    t: f64,
    measure: &mut M,
) -> JsResult<Value> {
    let d = scope.resolved_text_content(layer, t)?;
    let layout = measure.glyph_layout(&text_content(&d));
    let state = scope.text_animation_state(layer, t, &layout)?;
    let glyphs = scope.animated_glyphs(&d, &layout, &state);
    let animators = Value::Array(
        state
            .animators
            .iter()
            .map(|a| {
                object([
                    ("id", js_to_value(&a.id)),
                    ("mode", Value::String(JsString::from(a.mode.name()))),
                    ("unit", Value::String(JsString::from(a.unit.name()))),
                    ("values", content_value(&a.values)),
                    ("weights", numbers(&a.weights)),
                ])
            })
            .collect(),
    );
    let styles = Value::Array(
        state
            .styles
            .iter()
            .map(|s| content_value(&s.item))
            .collect(),
    );
    let mut keys: Vec<JsString> = ["font", "weight", "size", "color", "italic", "tracking"]
        .iter()
        .map(|k| JsString::from(*k))
        .collect();
    for s in &state.styles {
        for k in s.p.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    let glyphs = Value::Array(
        glyphs
            .iter()
            .map(|g| {
                let u = &g.unit;
                let mut style = Map::new();
                for k in &keys {
                    style.insert(k.clone(), g.style.get(k).map_or(Value::Null, js_to_value));
                }
                object([
                    ("index", Value::Number(u.index as f64)),
                    ("text", Value::String(JsString::from(u.text.as_str()))),
                    ("line", Value::Number(u.line as f64)),
                    (
                        "word",
                        u.word.map_or(Value::Null, |w| Value::Number(w as f64)),
                    ),
                    ("x", Value::Number(u.x)),
                    ("y", Value::Number(u.y)),
                    ("w", Value::Number(u.w)),
                    ("matrix", numbers(&g.matrix)),
                    ("opacity", Value::Number(g.opacity)),
                    ("blur", Value::Number(g.blur)),
                    ("color", g.color.clone().map_or(Value::Null, Value::String)),
                    ("tracking", Value::Number(g.tracking)),
                    ("trackingShift", Value::Number(g.tracking_shift)),
                    ("style", Value::Object(style)),
                ])
            })
            .collect(),
    );
    Ok(object([
        ("animators", animators),
        ("styles", styles),
        ("glyphs", glyphs),
    ]))
}
