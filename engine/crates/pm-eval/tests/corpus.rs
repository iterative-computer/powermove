//! Compares the engine's layer evaluation with the parity corpus's evaluation
//! dumps (`docs/engine/corpus.md`), captured from the legacy app.
//!
//! The corpus lives outside the repository. The test reads the capture named
//! by `PM_ENGINE_CORPUS` (a capture directory), or the newest `v0-*` capture
//! under `~/Library/Application Support/Powermove Engine Corpus/`, and passes
//! with a note when there is none. `PM_CORPUS_ONLY=<prefix>` limits it to
//! projects whose `<group>/<name>` starts with the prefix; `PM_CORPUS_VERBOSE=1`
//! prints every mismatch; `PM_CORPUS_FRESH_FONT_CACHE=1` measures text as a
//! fresh renderer would instead of modelling the capture's font cache
//! (`capture_font_cache_seeds`).
//!
//! Tolerances (plan section 25): numbers within `1e-9 + 1e-9·|b|`, strings and
//! booleans exact; bounds within 1e-6.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_eval::retime::Retiming;
use pm_eval::{Comp, Evaluator, JsValue, Scope};
use pm_model::json::{Map, Value, parse};
use pm_model::{Document, JsString, Node};
use pm_text::compat::{GlyphLayout, Unit};

use pm_eval::bounds::{Bounds, HOST_ONLY, LayerHost};
use pm_eval::text_measure::CompatMeasure;
use pm_text::compat::FontCacheSeed;
use pm_text::registry::CoreTextRegistry;
use support::pm_expr_engine::PmExprEngine;

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_ENGINE_CORPUS") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join("Library/Application Support/Powermove Engine Corpus");
    let mut captures: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("v0-"))
                && p.join("projects").is_dir()
        })
        .collect();
    captures.sort();
    captures.pop()
}

/// Every project directory (`projects/<group>/<name>`) with a loaded document
/// and evaluation dumps, plus `real/<slug>`.
fn projects(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for top in ["projects", "real"] {
        let Ok(groups) = std::fs::read_dir(dir.join(top)) else {
            continue;
        };
        for group in groups.filter_map(|e| e.ok()) {
            let gp = group.path();
            if gp.join("loaded.json").is_file() {
                out.push((format!("{top}/{}", group.file_name().to_string_lossy()), gp));
                continue;
            }
            let Ok(names) = std::fs::read_dir(&gp) else {
                continue;
            };
            for name in names.filter_map(|e| e.ok()) {
                let p = name.path();
                if p.join("loaded.json").is_file() && p.join("eval").is_dir() {
                    out.push((
                        format!(
                            "{}/{}",
                            group.file_name().to_string_lossy(),
                            name.file_name().to_string_lossy()
                        ),
                        p,
                    ));
                }
            }
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Stats {
    compared: BTreeMap<&'static str, usize>,
    failed: BTreeMap<&'static str, usize>,
    max_error: BTreeMap<&'static str, f64>,
    samples: BTreeMap<&'static str, Vec<String>>,
    /// Values not compared, by category and reason.
    skipped: BTreeMap<(&'static str, &'static str), usize>,
    /// Text layers whose pm-text layout differs from the capture, by font.
    layout_fonts: BTreeMap<String, usize>,
    verbose: bool,
}

impl Stats {
    fn record(&mut self, category: &'static str, ok: bool, what: impl FnOnce() -> String) {
        *self.compared.entry(category).or_default() += 1;
        if !ok {
            *self.failed.entry(category).or_default() += 1;
            let list = self.samples.entry(category).or_default();
            if self.verbose || list.len() < 8 {
                list.push(what());
            }
        }
    }

    /// A value this run does not compare, and why.
    fn skip(&mut self, category: &'static str, reason: &'static str) {
        *self.skipped.entry((category, reason)).or_default() += 1;
    }

    fn error(&mut self, category: &'static str, e: f64) {
        let m = self.max_error.entry(category).or_insert(0.0);
        if e > *m {
            *m = e;
        }
    }
}

/// A dumped number: numbers, or `"NaN"` / `"Infinity"` / `"-Infinity"`.
fn dumped_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::String(s) => match s.as_str()? {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

fn close(ours: f64, want: f64, abs: f64, rel: f64) -> bool {
    if ours.is_nan() || want.is_nan() {
        return ours.is_nan() && want.is_nan();
    }
    if ours.is_infinite() || want.is_infinite() {
        return ours == want;
    }
    (ours - want).abs() <= abs + rel * want.abs()
}

/// The JSON the capture's `plain()` writes for a value: non-finite numbers as
/// strings, `-0` as 0.
fn plain(v: &Value) -> Value {
    match v {
        Value::Number(n) if !n.is_finite() => Value::String(JsString::from(if n.is_nan() {
            "NaN"
        } else if *n > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        })),
        Value::Number(n) if *n == 0.0 => Value::Number(0.0),
        Value::Array(items) => Value::Array(items.iter().map(plain).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), plain(v))).collect())
        }
        other => other.clone(),
    }
}

/// Deep comparison with the numeric tolerance; returns the largest error.
fn same(ours: &Value, want: &Value, abs: f64, rel: f64, err: &mut f64) -> bool {
    match (ours, want) {
        (Value::Number(a), _) if dumped_number(want).is_some() => {
            let b = dumped_number(want).unwrap();
            if a.is_finite() && b.is_finite() {
                let e = (a - b).abs() / (1.0 + b.abs());
                if e > *err {
                    *err = e;
                }
            }
            close(*a, b, abs, rel)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y, abs, rel, err))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, x)| b.get_key(k).is_some_and(|y| same(x, y, abs, rel, err)))
        }
        _ => false,
    }
}

/// Where two values first differ (a JSON path with both sides).
fn first_difference(ours: &Value, want: &Value, tol: (f64, f64), path: String) -> Option<String> {
    let mut err = 0.0;
    if same(ours, want, tol.0, tol.1, &mut err) {
        return None;
    }
    match (ours, want) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, x) in a.iter() {
                let p = format!("{path}.{}", k.to_string_lossy());
                match b.get_key(k) {
                    Some(y) => {
                        if let Some(d) = first_difference(x, y, tol, p) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{p} (only ours)")),
                }
            }
            for (k, _) in b.iter() {
                if a.get_key(k).is_none() {
                    return Some(format!("{path}.{} (only want)", k.to_string_lossy()));
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| first_difference(x, y, tol, format!("{path}[{i}]"))),
        _ => Some(format!(
            "{path} ({} vs {})",
            pm_model::json::to_canonical_string(ours),
            pm_model::json::to_canonical_string(want)
        )),
    }
}

fn js_to_value(v: &JsValue) -> Value {
    match v {
        JsValue::Undefined | JsValue::Null => Value::Null,
        JsValue::Bool(b) => Value::Bool(*b),
        JsValue::Number(n) => Value::Number(*n),
        JsValue::String(s) => Value::String(s.clone()),
        JsValue::Object(v) => (**v).clone(),
    }
}

fn numbers(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|x| Value::Number(*x)).collect())
}

fn member<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.get_key(&JsString::from(key)),
        _ => None,
    }
}

fn layer_error(state: &Value, label: &str) -> bool {
    member(state, "errors")
        .and_then(|e| member(e, &format!("*{label}")))
        .is_some()
}

struct Ctx<'a> {
    project: &'a str,
    stats: &'a mut Stats,
    measure: &'a mut CompatMeasure<CoreTextRegistry>,
}

/// Answers nothing natively unanswerable (the defaults throw `HostOnly`).
struct CorpusHost;
impl LayerHost for CorpusHost {}

const BOUNDS: (f64, f64) = (1e-6, 0.0);
/// Variable-font ink boxes: CoreText instances outlines with its own
/// variation arithmetic; measured within 1.2e-4 px on the corpus.
const VARIABLE_INK: (f64, f64) = (2e-4, 0.0);

fn bounds_value(b: &Bounds) -> Value {
    let mut m = Map::new();
    for (k, v) in [
        ("x0", b.x0),
        ("y0", b.y0),
        ("x1", b.x1),
        ("y1", b.y1),
        ("w", b.w),
        ("h", b.h),
        ("ax", b.ax),
        ("ay", b.ay),
    ] {
        m.insert(k, Value::Number(v));
    }
    Value::Object(m)
}

fn selection_value(s: &pm_eval::text_geom::Selection) -> Value {
    let mut m = Map::new();
    for (k, v) in [
        ("x0", s.x0),
        ("y0", s.y0),
        ("x1", s.x1),
        ("y1", s.y1),
        ("w", s.w),
        ("h", s.h),
    ] {
        m.insert(k, Value::Number(v));
    }
    Value::Object(m)
}

fn unit_value(u: &Unit, kind: &str) -> Value {
    let mut m = Map::new();
    m.insert("text", Value::String(JsString::from(u.text.as_str())));
    m.insert("x", Value::Number(u.x));
    m.insert("y", Value::Number(u.y));
    m.insert("w", Value::Number(u.w));
    m.insert("line", Value::Number(u.line as f64));
    if kind == "characters" {
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

fn layout_value(l: &GlyphLayout) -> Value {
    let mut m = Map::new();
    m.insert(
        "characters",
        Value::Array(
            l.characters
                .iter()
                .map(|u| unit_value(u, "characters"))
                .collect(),
        ),
    );
    m.insert(
        "words",
        Value::Array(l.words.iter().map(|u| unit_value(u, "words")).collect()),
    );
    m.insert(
        "lines",
        Value::Array(l.lines.iter().map(|u| unit_value(u, "lines")).collect()),
    );
    Value::Object(m)
}

impl Ctx<'_> {
    fn check(
        &mut self,
        category: &'static str,
        what: &str,
        ours: Result<Value, String>,
        want: Option<&Value>,
        threw: bool,
        tol: (f64, f64),
    ) {
        let want = want.unwrap_or(&Value::Null);
        match ours {
            Ok(v) => {
                let v = plain(&v);
                let mut err = 0.0;
                let ok = !threw && same(&v, want, tol.0, tol.1, &mut err);
                self.stats.error(category, err);
                let project = self.project;
                self.stats.record(category, ok, || {
                    let at = first_difference(&v, want, tol, String::new()).unwrap_or_default();
                    format!(
                        "{project} {what}: first difference at {at}: ours {} want {}{}",
                        pm_model::json::to_canonical_string(&v),
                        pm_model::json::to_canonical_string(want),
                        if threw { " (legacy threw)" } else { "" }
                    )
                });
            }
            Err(e) => {
                let ok = threw || matches!(want, Value::Null);
                let project = self.project;
                self.stats.record(category, ok, || {
                    format!(
                        "{project} {what}: ours threw {e}, want {}",
                        pm_model::json::to_canonical_string(want)
                    )
                });
            }
        }
    }
}

const NUM: (f64, f64) = (1e-9, 1e-9);

/// Why a text layer's geometry is not compared: pm-text measures it
/// differently from the capture (a pm-text difference, reported by font).
const PM_TEXT_DIFFERS: &str = "pm-text layout differs from the capture";

/// Text layouts (layer × dump) that may be left out of the comparison
/// because pm-text lays them out differently from the capture, per font
/// (amendment 19). The test fails when a font's count grows past its pin
/// (fonts not listed are pinned at 0); lower a pin when pm-text improves.
/// Empty: every text layout of the v0 and v1 captures is compared.
const PINNED_TEXT_DIFFERENCES: &[(&str, usize)] = &[];

fn compare_comp(
    cx: &mut Ctx<'_>,
    ev: &mut Evaluator,
    doc: &Document,
    comp: &Comp<'_>,
    project: &Comp<'_>,
    state: &Value,
    path: &str,
) {
    let time = dumped_number(member(state, "time").unwrap()).unwrap();
    let layers = comp.layers();
    let Some(Value::Array(dumped)) = member(state, "layers") else {
        return;
    };
    // Text layers pm-text lays out differently from the capture (font
    // resolution), and text set in variable fonts (ink boxes within about
    // 1e-4 px of CoreText's, see docs/engine/pm-eval-layers.md).
    let mut text_differs: Vec<usize> = Vec::new();
    let mut variable_text: Vec<usize> = Vec::new();
    for l in dumped {
        let index = member(l, "index").and_then(dumped_number).unwrap() as usize;
        let Some(layer) = layers.get(index) else {
            continue;
        };
        if member(l, "type")
            .and_then(|t| t.as_js_string())
            .and_then(|s| s.as_str())
            != Some("text")
        {
            continue;
        }
        let mut scope = Scope::new(ev, comp, project);
        if let Ok(layout) = scope.text_layout(layer, time, &mut *cx.measure) {
            let mut err = 0.0;
            if !member(l, "textLayout").is_some_and(|want| {
                same(
                    &plain(&layout_value(&layout)),
                    want,
                    BOUNDS.0,
                    BOUNDS.1,
                    &mut err,
                )
            }) {
                let font = scope
                    .resolved_text_content(layer, time)
                    .ok()
                    .and_then(|d| d.get(&JsString::from("font")).map(|f| f.to_js_string()))
                    .map_or_else(|| "(none)".to_owned(), |f| f.to_string_lossy());
                *cx.stats.layout_fonts.entry(font).or_default() += 1;
                if cx.stats.verbose {
                    let ours = plain(&layout_value(&layout));
                    let want = member(l, "textLayout").cloned().unwrap_or(Value::Null);
                    eprintln!(
                        "pm-text layout differs: {} {path} layer {index} at {}",
                        cx.project,
                        first_difference(&ours, &want, BOUNDS, String::new()).unwrap_or_default()
                    );
                }
                text_differs.push(index);
            }
        }
        if let Ok(d) = scope.resolved_text_content(layer, time)
            && cx
                .measure
                .uses_variations(&pm_eval::text_geom::text_content(&d))
        {
            variable_text.push(index);
        }
    }
    let members_of = |group: &Node, ev: &mut Evaluator, list: &[usize]| -> bool {
        let gid = group.get("id").map(|n| n.to_value());
        list.iter().any(|&i| {
            let Some(child) = layers.get(i) else {
                return false;
            };
            ev.group_ancestors(comp, child)
                .unwrap_or_default()
                .iter()
                .any(|g| g.get("id").map(|n| n.to_value()) == gid)
        })
    };
    for l in dumped {
        let index = member(l, "index").and_then(dumped_number).unwrap() as usize;
        let Some(layer) = layers.get(index) else {
            cx.stats.record("layer-index", false, || {
                format!("{} {path} missing layer {index}", cx.project)
            });
            continue;
        };
        let id = member(l, "id")
            .map(pm_model::json::to_canonical_string)
            .unwrap_or_default();
        let ty = member(l, "type")
            .and_then(|t| t.as_js_string())
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let at = |what: &str| format!("{path} {id} t={time} {what}");
        let mut scope = Scope::new(ev, comp, project);

        // Active state.
        let active = scope
            .active(layer, time)
            .map(Value::Bool)
            .map_err(|e| e.to_string());
        cx.check(
            "active",
            &at("active"),
            active,
            member(l, "active"),
            layer_error(l, "active"),
            NUM,
        );

        // Channels.
        if let Some(Value::Object(channels)) = member(l, "channels") {
            let props = scope.ev.all_props(layer).unwrap_or_default();
            for (key, want) in channels.iter() {
                let in_p = node_has(layer, "p", key);
                let got = if in_p {
                    scope.ev.ev(comp, Some(layer), key, time)
                } else {
                    let prop = props
                        .iter()
                        .find(|p| &p.key == key)
                        .and_then(|p| p.prop.node());
                    scope.ev.ev_p(comp, Some(layer), prop, time, key)
                };
                let threw = member(l, "errors")
                    .and_then(|e| member(e, &key.to_string_lossy()))
                    .is_some()
                    && !matches!(want, Value::Number(_) | Value::String(_) | Value::Bool(_));
                cx.check(
                    "channels",
                    &at(&format!("channel {}", key.to_string_lossy())),
                    got.map(|v| js_to_value(&v)).map_err(|e| e.to_string()),
                    Some(want),
                    threw,
                    NUM,
                );
            }
        }

        // Resolved content.
        if member(l, "content").is_some() {
            let got = scope
                .ev
                .resolve_content(comp, layer, time)
                .map(|c| {
                    Value::Object(
                        c.iter()
                            .map(|(k, v)| (k.clone(), js_to_value(v)))
                            .collect::<Map>(),
                    )
                })
                .map_err(|e| e.to_string());
            cx.check(
                "content",
                &at("content"),
                got,
                member(l, "content"),
                layer_error(l, "content"),
                NUM,
            );
        }

        if ty != "audio" {
            let got = scope
                .local_matrix(layer, time)
                .map(|m| numbers(&m))
                .map_err(|e| e.to_string());
            cx.check(
                "localMatrix",
                &at("localMatrix"),
                got,
                member(l, "localMatrix"),
                layer_error(l, "localMatrix"),
                NUM,
            );
            let got = scope
                .world_matrix(layer, time)
                .map(|m| numbers(&m))
                .map_err(|e| e.to_string());
            cx.check(
                "worldMatrix",
                &at("worldMatrix"),
                got,
                member(l, "worldMatrix"),
                layer_error(l, "worldMatrix"),
                NUM,
            );
            let got = scope
                .world_opacity(layer, time)
                .map(Value::Number)
                .map_err(|e| e.to_string());
            cx.check(
                "worldOpacity",
                &at("worldOpacity"),
                got,
                member(l, "worldOpacity"),
                layer_error(l, "worldOpacity"),
                NUM,
            );
        }
        let three_d = member(l, "layerFields").and_then(|f| member(f, "threeD"));
        if matches!(three_d, Some(Value::Bool(true))) {
            let got = scope
                .plane_matrix(layer, time, false)
                .map(|m| numbers(&m))
                .map_err(|e| e.to_string());
            cx.check(
                "planeMatrix",
                &at("planeMatrix"),
                got,
                member(l, "planeMatrix"),
                layer_error(l, "planeMatrix"),
                NUM,
            );
        }
        if let Some(want) = member(l, "textAnimation") {
            compare_text_animation(cx, &mut scope, layer, time, l, want, &at("textAnimation"));
        }
        // Text layout first: where pm-text lays the text out differently from
        // the capture (fonts the registry resolves differently), the text's
        // geometry is pm-text's difference, reported apart.
        let layout_differs = text_differs.contains(&index)
            || (ty == "group" && members_of(layer, scope.ev, &text_differs));
        let variable = variable_text.contains(&index)
            || (ty == "group" && members_of(layer, scope.ev, &variable_text));
        if ty == "text" {
            let got = scope
                .text_layout(layer, time, &mut *cx.measure)
                .map(|l| layout_value(&l))
                .map_err(|e| e.to_string());
            if layout_differs {
                cx.stats.skip("textLayout", PM_TEXT_DIFFERS);
            } else {
                cx.check(
                    "textLayout",
                    &at("textLayout"),
                    got,
                    member(l, "textLayout"),
                    layer_error(l, "textLayout"),
                    BOUNDS,
                );
            }
        }
        if ty != "audio" {
            let got = scope.bounds(layer, time, &mut *cx.measure, &mut CorpusHost);
            let category: &'static str = if variable {
                "bounds (variable font)"
            } else if ty == "text" {
                "bounds text"
            } else if ty == "group" {
                "bounds group"
            } else {
                "bounds"
            };
            let tol = if variable && !layout_differs {
                VARIABLE_INK
            } else {
                BOUNDS
            };
            match got {
                Err(e) if e.kind == HOST_ONLY => cx.stats.skip("bounds", "host answers in Phase 1"),
                _ if layout_differs => cx.stats.skip("bounds", PM_TEXT_DIFFERS),
                got => {
                    let got = got
                        .map(|b| b.map_or(Value::Null, |b| bounds_value(&b)))
                        .map_err(|e| e.to_string());
                    cx.check(
                        category,
                        &at(&format!("{ty} bounds")),
                        got,
                        member(l, "bounds"),
                        layer_error(l, "bounds"),
                        tol,
                    );
                }
            }
        }
        if ty == "text"
            && !layout_differs
            && let Some(want) = member(l, "textRaster")
        {
            let got = scope
                .text_geometry(layer, time, &mut *cx.measure)
                .map(|g| {
                    let mut m = Map::new();
                    m.insert("w", Value::Number(g.w));
                    m.insert("h", Value::Number(g.h));
                    m.insert("anchorX", Value::Number(g.anchor_x));
                    m.insert("anchorY", Value::Number(g.anchor_y));
                    m.insert("density", Value::Number(g.density(1.0)));
                    m.insert("selection", selection_value(&g.selection));
                    Value::Object(m)
                })
                .map_err(|e| e.to_string());
            let (category, tol) = if variable {
                ("textRaster (variable font)", VARIABLE_INK)
            } else {
                ("textRaster", BOUNDS)
            };
            cx.check(
                category,
                &at("textRaster"),
                got,
                Some(want),
                layer_error(l, "textRaster"),
                tol,
            );
        }
        if let Some(want) = member(l, "paths") {
            let got = scope
                .path_geometry(layer, time)
                .map(|(paths, matrices, _)| {
                    Value::Array(
                        paths
                            .iter()
                            .zip(&matrices)
                            .map(|(p, m)| {
                                let mut o = Map::new();
                                o.insert("id", js_to_value(&p.id));
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
                                o.insert("values", Value::Object(values));
                                o.insert("matrix", numbers(m));
                                Value::Object(o)
                            })
                            .collect(),
                    )
                })
                .map_err(|e| e.to_string());
            cx.check(
                "paths",
                &at("paths"),
                got,
                Some(want),
                layer_error(l, "paths"),
                NUM,
            );
        }
        if matches!(ty.as_str(), "video" | "precomp" | "audio") {
            let got = scope
                .source_time_with(layer, time, Retiming::Legacy, None)
                .map(Value::Number)
                .map_err(|e| e.to_string());
            cx.check(
                "sourceTime",
                &at("sourceTime"),
                got,
                member(l, "sourceTime"),
                layer_error(l, "sourceTime"),
                NUM,
            );
        }
    }

    // Nested compositions at their source times.
    if let Some(Value::Array(nested)) = member(state, "precomps") {
        for entry in nested {
            let comp_id = member(entry, "comp")
                .and_then(|c| c.as_js_string())
                .map(|s| s.to_string_lossy())
                .unwrap();
            let Ok(sub) = Comp::resolve(doc, &comp_id) else {
                cx.stats.record("precomp", false, || {
                    format!("{} {path}: no composition {comp_id}", cx.project)
                });
                continue;
            };
            let sub_state = member(entry, "state").unwrap();
            let want_time = dumped_number(member(entry, "time").unwrap()).unwrap();
            let got_time = dumped_number(member(sub_state, "time").unwrap()).unwrap();
            assert_eq!(want_time.to_bits(), got_time.to_bits());
            compare_comp(
                cx,
                ev,
                doc,
                &sub,
                project,
                sub_state,
                &format!("{path}>{comp_id}"),
            );
        }
    }
}

fn unit_of(v: &Value) -> Unit {
    let n = |k: &str| member(v, k).and_then(dumped_number);
    Unit {
        text: member(v, "text")
            .and_then(|t| t.as_js_string())
            .map(|s| s.to_string_lossy())
            .unwrap_or_default(),
        x: n("x").unwrap(),
        y: n("y").unwrap(),
        w: n("w").unwrap(),
        line: n("line").unwrap() as usize,
        source_start: n("sourceStart").unwrap() as usize,
        index: n("index").unwrap() as usize,
        line_unit: n("lineUnit").map(|x| x as usize),
        word: n("word").map(|x| x as usize),
    }
}

fn layout_of(v: &Value) -> GlyphLayout {
    let list = |k: &str| match member(v, k) {
        Some(Value::Array(items)) => items.iter().map(unit_of).collect(),
        _ => Vec::new(),
    };
    GlyphLayout {
        characters: list("characters"),
        words: list("words"),
        lines: list("lines"),
    }
}

fn content_value(c: &pm_eval::evaluator::Content) -> Value {
    Value::Object(
        c.iter()
            .filter(|(_, v)| !matches!(v, JsValue::Undefined))
            .map(|(k, v)| (k.clone(), js_to_value(v)))
            .collect::<Map>(),
    )
}

/// Animator weights, range styles, and posed glyphs, from the dumped layout
/// (layout itself is pm-text's comparison).
fn compare_text_animation<'d>(
    cx: &mut Ctx<'_>,
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    l: &Value,
    want: &Value,
    what: &str,
) {
    let Some(layout) = member(l, "textLayout") else {
        return;
    };
    let layout = layout_of(layout);
    let threw = layer_error(l, "textAnimation");
    let result = scope.resolved_text_content(layer, time).and_then(|d| {
        let state = scope.text_animation_state(layer, time, &layout)?;
        let glyphs = scope.animated_glyphs(&d, &layout, &state);
        Ok((state, glyphs))
    });
    let (state, glyphs) = match result {
        Ok(x) => x,
        Err(e) => {
            cx.check(
                "textAnimation",
                what,
                Err(e.to_string()),
                Some(want),
                threw,
                NUM,
            );
            return;
        }
    };
    let animators = Value::Array(
        state
            .animators
            .iter()
            .map(|a| {
                let mut m = Map::new();
                m.insert("id", js_to_value(&a.id));
                m.insert("mode", Value::String(JsString::from(a.mode.name())));
                m.insert("unit", Value::String(JsString::from(a.unit.name())));
                m.insert("values", content_value(&a.values));
                m.insert("weights", numbers(&a.weights));
                Value::Object(m)
            })
            .collect(),
    );
    cx.check(
        "textAnimators",
        &format!("{what} animators"),
        Ok(animators),
        member(want, "animators"),
        threw,
        NUM,
    );
    let styles = Value::Array(
        state
            .styles
            .iter()
            .map(|s| content_value(&s.item))
            .collect(),
    );
    cx.check(
        "textStyles",
        &format!("{what} styles"),
        Ok(styles),
        member(want, "styles"),
        threw,
        NUM,
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
                let mut m = Map::new();
                let u = &g.unit;
                m.insert("index", Value::Number(u.index as f64));
                m.insert("text", Value::String(JsString::from(u.text.as_str())));
                m.insert("line", Value::Number(u.line as f64));
                m.insert(
                    "word",
                    u.word.map_or(Value::Null, |w| Value::Number(w as f64)),
                );
                m.insert("x", Value::Number(u.x));
                m.insert("y", Value::Number(u.y));
                m.insert("w", Value::Number(u.w));
                m.insert("matrix", numbers(&g.matrix));
                m.insert("opacity", Value::Number(g.opacity));
                m.insert("blur", Value::Number(g.blur));
                m.insert("color", g.color.clone().map_or(Value::Null, Value::String));
                m.insert("tracking", Value::Number(g.tracking));
                m.insert("trackingShift", Value::Number(g.tracking_shift));
                let mut style = Map::new();
                for k in &keys {
                    style.insert(k.clone(), g.style.get(k).map_or(Value::Null, js_to_value));
                }
                m.insert("style", Value::Object(style));
                Value::Object(m)
            })
            .collect(),
    );
    cx.check(
        "textGlyphs",
        &format!("{what} glyphs"),
        Ok(glyphs),
        member(want, "glyphs"),
        threw,
        NUM,
    );
}

fn node_has(layer: &Node, member: &str, key: &JsString) -> bool {
    match layer.get(member).map(Node::kind) {
        Some(pm_model::NodeKind::Object(map)) => map.get(key.as_bytes()).is_some(),
        _ => false,
    }
}

fn main() {
    pins_fail_only_when_a_font_grows();
    corpus_evaluation_matches_legacy();
}

/// Fonts whose not-compared text layouts exceed their pins: `(font, count,
/// pin)`. Fonts without a pin are pinned at 0.
fn text_differences_over_pins(
    counts: &BTreeMap<String, usize>,
    pins: &[(&str, usize)],
) -> Vec<(String, usize, usize)> {
    counts
        .iter()
        .filter_map(|(font, &n)| {
            let pin = pins.iter().find(|(f, _)| f == font).map_or(0, |(_, p)| *p);
            (n > pin).then(|| (font.clone(), n, pin))
        })
        .collect()
}

fn pins_fail_only_when_a_font_grows() {
    let counts: BTreeMap<String, usize> =
        [("system-ui".to_owned(), 3), ("Menlo".to_owned(), 1)].into();
    assert!(text_differences_over_pins(&counts, &[("system-ui", 3), ("Menlo", 1)]).is_empty());
    assert!(text_differences_over_pins(&counts, &[("system-ui", 5), ("Menlo", 2)]).is_empty());
    assert_eq!(
        text_differences_over_pins(&counts, &[("system-ui", 2), ("Menlo", 1)]),
        [("system-ui".to_owned(), 3, 2)]
    );
    // A font without a pin is pinned at 0.
    assert_eq!(
        text_differences_over_pins(&counts, &[("system-ui", 3)]),
        [("Menlo".to_owned(), 1, 0)]
    );
    assert!(text_differences_over_pins(&BTreeMap::new(), &[]).is_empty());
}

/// The UI's type scale (`packages/tokens/tokens.css`: `--fs-sm`, `--fs-md`)
/// in CSS px. The app's UI lays system-ui text out at these sizes (font
/// previews fall back to `system-ui`, the 3D viewport chrome uses it).
const UI_SYSTEM_UI_SIZES: [f32; 2] = [12.0, 13.0];

/// The state of Blink's font cache in the renderer that captured the corpus
/// (`pm_text::compat::FontCacheSeed`).
///
/// The capture ran in the app, whose UI had laid out 400-weight `system-ui`
/// DOM text at its type scale before the canvas measured anything. At a
/// device scale factor of 2 that text is laid out at 24 and 26 px, so the
/// renderer's 24 and 26 px `system-ui` platform data carries `opsz` 12 and
/// 13 (SF's axis minimum, 17), and every canvas measuring `system-ui` at 24
/// or 26 px reuses it. Headless Chromium reproduces the capture's advances
/// bit for bit with 12 and 13 px `system-ui` DOM text at a scale factor of
/// 2, and pm-text's fresh-renderer advances without it or at a scale factor
/// of 1 (docs/engine/text.md).
fn capture_font_cache_seeds(manifest: Option<&Value>) -> Vec<FontCacheSeed> {
    let dpr = manifest
        .and_then(|m| member(m, "gpu"))
        .and_then(|g| member(g, "devicePixelRatio"))
        .and_then(dumped_number)
        .unwrap_or(1.0) as f32;
    if dpr == 1.0 {
        // Entries at the CSS size: a canvas at that size would make the same
        // ones (`opsz` at its own size).
        return Vec::new();
    }
    UI_SYSTEM_UI_SIZES
        .iter()
        .map(|&css| FontCacheSeed {
            family: "system-ui".into(),
            weight: 400.0,
            italic: false,
            size: pm_text::compat::canvas::effective_font_size(css * dpr),
            specified: css,
            cache_key: None,
        })
        .collect()
}

fn corpus_evaluation_matches_legacy() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    };
    let only = std::env::var("PM_CORPUS_ONLY").ok();
    let exclude = std::env::var("PM_CORPUS_EXCLUDE").ok();
    let mut stats = Stats {
        verbose: std::env::var("PM_CORPUS_VERBOSE").is_ok(),
        ..Stats::default()
    };
    let mut count = 0;
    let mut measure = CompatMeasure::new(CoreTextRegistry::new());
    let manifest = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|m| parse(&m).ok());
    let seeds = if std::env::var("PM_CORPUS_FRESH_FONT_CACHE").is_ok() {
        Vec::new()
    } else {
        capture_font_cache_seeds(manifest.as_ref())
    };
    eprintln!(
        "renderer font cache: {}",
        if seeds.is_empty() {
            "fresh".to_owned()
        } else {
            seeds
                .iter()
                .map(|s| {
                    format!(
                        "{} {} at {} px from {} px",
                        s.family, s.weight, s.size, s.specified
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    measure.compat().set_font_cache_seeds(seeds);
    for (name, path) in projects(&dir) {
        if only.as_deref().is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        if exclude.as_deref().is_some_and(|p| name.starts_with(p)) {
            continue;
        }
        let loaded = std::fs::read_to_string(path.join("loaded.json")).unwrap();
        let doc = Document::new(Some(parse(&loaded).unwrap()), 1);
        let mut evals: Vec<PathBuf> = std::fs::read_dir(path.join("eval"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        evals.sort();
        for eval in evals {
            let dump = parse(&std::fs::read_to_string(&eval).unwrap()).unwrap();
            let state = member(&dump, "composition").unwrap();
            let id = member(state, "id")
                .and_then(|c| c.as_js_string())
                .map(|s| s.to_string_lossy())
                .unwrap();
            let project = Comp::resolve(&doc, &id).expect("open composition");
            let mut ev = Evaluator::new().with_expressions(Arc::new(PmExprEngine));
            let mut cx = Ctx {
                project: &name,
                stats: &mut stats,
                measure: &mut measure,
            };
            compare_comp(&mut cx, &mut ev, &doc, &project, &project, state, &id);
            if let Some(want) = member(&dump, "scene3d").filter(|s| !matches!(s, Value::Null)) {
                let time = dumped_number(member(state, "time").unwrap()).unwrap();
                let mut scope = Scope::open(&mut ev, &project);
                match scope.composition_scene(time) {
                    Ok(s) if !s.recipe_parts.is_empty() => {
                        cx.stats.skip("scene3d", "recipe parts need pm-geom3d")
                    }
                    got => {
                        let got = got.map(|s| s.scene).map_err(|e| e.to_string());
                        cx.check(
                            "scene3d",
                            &format!("{id} t={time} scene3d"),
                            got,
                            Some(want),
                            false,
                            NUM,
                        );
                    }
                }
            }
            count += 1;
        }
    }
    eprintln!("corpus {}: {count} dumps", dir.display());
    let mut failed = false;
    for (category, n) in &stats.compared {
        let f = stats.failed.get(category).copied().unwrap_or(0);
        eprintln!(
            "  {category:<14} {n:>7} compared, {f:>5} mismatched, max relative error {:.3e}",
            stats.max_error.get(category).copied().unwrap_or(0.0)
        );
        if f > 0 {
            failed = true;
            for s in stats.samples.get(category).into_iter().flatten() {
                eprintln!("      {s}");
            }
        }
    }
    for ((category, reason), n) in &stats.skipped {
        eprintln!("  {category:<14} {n:>7} not compared: {reason}");
    }
    if !stats.layout_fonts.is_empty() {
        eprintln!(
            "  text layouts (layer × dump) pm-text lays out differently from the capture, by font:"
        );
        for (font, n) in &stats.layout_fonts {
            eprintln!("    {font:<24} {n:>7}");
        }
    }
    let grown = text_differences_over_pins(&stats.layout_fonts, PINNED_TEXT_DIFFERENCES);
    for (font, n, pin) in &grown {
        eprintln!("  text layouts in {font} not compared: {n}, pinned at {pin}");
    }
    for (font, pin) in PINNED_TEXT_DIFFERENCES {
        let n = stats.layout_fonts.get(*font).copied().unwrap_or(0);
        if n < *pin {
            eprintln!(
                "  text layouts in {font} not compared: {n}, below its pin of {pin}; lower the pin"
            );
        }
    }
    assert!(!failed, "corpus mismatches (see above)");
    assert!(
        grown.is_empty(),
        "text layouts left out of the comparison grew past their pins (see above)"
    );
}
