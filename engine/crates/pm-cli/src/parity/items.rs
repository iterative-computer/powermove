//! Every value an evaluation dump records, as a list of items to answer and
//! judge (`docs/engine/corpus.md`, "Evaluation dumps").
//!
//! Per composition state (the open composition, then every active precomp at
//! its source time, recursively): the composition record; per layer its
//! record, `active`, every channel and its expression error, the evaluated
//! layer fields, resolved content, local and world matrices, world opacity,
//! the 3D plane matrix, `GL.bounds`, the text layout, raster geometry, and
//! animation, caption bounds, path values and group matrices, and source
//! time; per dump the 3D scene description. `shaderMeta` (compiled-program
//! uniform lists and GL cache keys) is not an evaluation value and is listed
//! as not compared.

use pm_eval::text_geom::text_content;
use pm_eval::text_measure::CompatMeasure;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::json::{Map, Value};
use pm_model::{Document, JsString, Node};
use pm_text::font::FontResolver;

use super::compare::{Rule, dumped_number, member};

/// What to evaluate for an item.
#[derive(Clone, Debug)]
pub enum Kind {
    /// Read from the document itself (records of layers and compositions).
    Doc(Value),
    Active,
    /// `PM.ev(L, key)` when `in_p`, else `PM.evP(L, prop, T, key)` with the
    /// `allProps` entry for `key`.
    Channel {
        key: JsString,
        in_p: bool,
    },
    /// `PM.expressionErrors.get(prop)` after evaluating the channel.
    ExprError {
        key: JsString,
    },
    /// The layer fields: `raw` as stored, `evaluated` through `evP` (field,
    /// key) because the stored member is a property.
    LayerFields {
        raw: Map,
        evaluated: Vec<(&'static str, &'static str)>,
    },
    Content,
    LocalMatrix,
    WorldMatrix,
    WorldOpacity,
    PlaneMatrix,
    Bounds,
    TextLayout,
    TextRaster,
    TextAnimation,
    CaptionBounds,
    /// Values and group matrix of `d.paths[i]` for each path id.
    Paths {
        ids: Vec<Value>,
    },
    SourceTime {
        legacy: bool,
    },
    Scene,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub category: &'static str,
    pub kind: Kind,
    /// `<composition path> <layer id> t=<time> <what>`.
    pub label: String,
    /// The composition evaluated and the open one.
    pub comp: String,
    pub open: String,
    /// Composition path (`C1>C2`), scoping layer indexes.
    pub path: String,
    pub layer: Option<usize>,
    pub layer_id: String,
    pub time: f64,
    pub want: Value,
    /// The dump records that the legacy call threw (the value is `null`), and
    /// the message.
    pub legacy_threw: Option<String>,
    pub rule: Rule,
    /// Text layers (indexes in this composition) whose layout must match the
    /// capture for this value to be compared.
    pub text_deps: Vec<usize>,
    /// The font of a text layout (for reports by font).
    pub font: Option<String>,
}

/// Why some dumped values are never compared.
pub const SHADER_META: &str = "shader compile metadata (the GL program's uniform list and cache key), Phase 3a shader pipeline";

/// The variable-font ink tolerance class (Phase 1 T2; `compare`): CoreText
/// instances variable outlines with its own variation arithmetic, skrifa's
/// differ by at most 1.19e-4 px on the corpus. pm-eval's corpus test uses
/// the same bound.
pub const VARIABLE_INK: (f64, f64) = (2e-4, 0.0);
pub const VARIABLE_INK_CLASS: &str = "variable-font ink (CoreText instances outlines)";

fn s(v: Option<&Value>) -> String {
    v.map(|v| match v {
        Value::String(s) => s.to_string_lossy(),
        other => pm_model::json::to_canonical_string(other),
    })
    .unwrap_or_default()
}

fn layer_error(state: &Value, label: &str) -> Option<String> {
    member(state, "errors")
        .and_then(|e| member(e, label))
        .map(|m| s(Some(m)))
}

fn node_has(layer: &Node, member: &str, key: &JsString) -> bool {
    match layer.get(member).map(Node::kind) {
        Some(pm_model::NodeKind::Object(map)) => map.get(key.as_bytes()).is_some(),
        _ => false,
    }
}

fn raw(layer: &Node, key: &str) -> Option<Value> {
    layer.get(key).map(Node::to_value)
}

/// Builds the items of one dump. `ev` and `measure` answer the structural
/// questions the build needs (group membership, the fonts of text layers);
/// they never answer an item.
pub struct Builder<'m, R: FontResolver> {
    pub measure: &'m mut CompatMeasure<R>,
    pub skipped: Vec<(&'static str, &'static str)>,
}

impl<R: FontResolver> Builder<'_, R> {
    pub fn dump(&mut self, doc: &Document, dump: &Value) -> Result<Vec<Item>, String> {
        let state = member(dump, "composition").ok_or("dump without a composition")?;
        let open = s(member(state, "id"));
        let project =
            Comp::resolve(doc, &open).map_err(|e| format!("open composition {open}: {e:?}"))?;
        let mut ev = Evaluator::new();
        let mut out = Vec::new();
        self.state(
            &mut out, &mut ev, doc, &project, &project, state, &open, &open,
        )?;
        let time = member(dump, "time")
            .and_then(dumped_number)
            .unwrap_or(f64::NAN);
        if let Some(want) = member(dump, "scene3d").filter(|s| !matches!(s, Value::Null)) {
            out.push(Item {
                category: "scene3d",
                kind: Kind::Scene,
                label: format!("{open} t={time} scene3d"),
                comp: open.clone(),
                open: open.clone(),
                path: open.clone(),
                layer: None,
                layer_id: String::new(),
                time,
                want: want.clone(),
                legacy_threw: None,
                rule: Rule::num(),
                text_deps: Vec::new(),
                font: None,
            });
        }
        if let Some(Value::Object(meta)) = member(dump, "shaderMeta") {
            for _ in meta.iter() {
                self.skipped.push(("shaderMeta", SHADER_META));
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn state<'d>(
        &mut self,
        out: &mut Vec<Item>,
        ev: &mut Evaluator,
        doc: &'d Document,
        comp: &Comp<'d>,
        project: &Comp<'d>,
        state: &Value,
        path: &str,
        open: &str,
    ) -> Result<(), String> {
        let time = member(state, "time")
            .and_then(dumped_number)
            .unwrap_or(f64::NAN);
        let comp_id = comp.id().to_string_lossy();
        let layers = comp.layers();
        let item = |category: &'static str,
                    kind: Kind,
                    what: String,
                    layer: Option<usize>,
                    layer_id: &str,
                    want: Value| Item {
            category,
            kind,
            label: format!("{path} {layer_id} t={time} {what}"),
            comp: comp_id.clone(),
            open: open.to_owned(),
            path: path.to_owned(),
            layer,
            layer_id: layer_id.to_owned(),
            time,
            want,
            legacy_threw: None,
            rule: Rule::num(),
            text_deps: Vec::new(),
            font: None,
        };

        // The composition record.
        let mut record = Map::new();
        let mut want = Map::new();
        // `{ id: comp.compId ?? comp.id, w, h, fps }`.
        for k in ["id", "w", "h", "fps"] {
            if let Some(w) = member(state, k) {
                want.insert(k, w.clone());
                let ours = if k == "id" {
                    comp.member("compId")
                        .or_else(|| comp.member("id"))
                        .map(Node::to_value)
                        .unwrap_or(Value::Null)
                } else {
                    comp.member(k).map(Node::to_value).unwrap_or(Value::Null)
                };
                record.insert(k, ours);
            }
        }
        out.push(item(
            "composition record",
            Kind::Doc(Value::Object(record)),
            "composition".into(),
            None,
            "",
            Value::Object(want),
        ));

        let Some(Value::Array(dumped)) = member(state, "layers") else {
            return Ok(());
        };
        // Text layers and their fonts, and which use font variations.
        let mut text_layers: Vec<(usize, String, bool)> = Vec::new();
        for l in dumped {
            let index = member(l, "index").and_then(dumped_number).unwrap_or(-1.0);
            let Some(layer) = (index >= 0.0).then(|| layers.get(index as usize)).flatten() else {
                continue;
            };
            if s(member(l, "type")) != "text" {
                continue;
            }
            let mut scope = Scope::new(ev, comp, project);
            let (font, variable) = match scope.resolved_text_content(layer, time) {
                Ok(d) => {
                    let font = d
                        .get(&JsString::from("font"))
                        .map(|f| f.to_js_string().to_string_lossy())
                        .unwrap_or_else(|| "(none)".to_owned());
                    (font, self.measure.uses_variations(&text_content(&d)))
                }
                Err(_) => ("(none)".to_owned(), false),
            };
            text_layers.push((index as usize, font, variable));
        }

        for l in dumped {
            let index = member(l, "index").and_then(dumped_number).unwrap_or(-1.0);
            let id = s(member(l, "id"));
            let ty = s(member(l, "type"));
            let Some(layer) = (index >= 0.0).then(|| layers.get(index as usize)).flatten() else {
                let mut it = item(
                    "layer record",
                    Kind::Doc(Value::Null),
                    "layer".into(),
                    None,
                    &id,
                    Value::Bool(true),
                );
                it.label = format!("{path}: no layer at index {index}");
                out.push(it);
                continue;
            };
            let index = index as usize;
            let at = Some(index);

            // The layer record.
            let mut want = Map::new();
            let mut record = Map::new();
            for k in ["id", "name", "type"] {
                if let Some(w) = member(l, k) {
                    want.insert(k, w.clone());
                    record.insert(k, raw(layer, k).unwrap_or(Value::Null));
                }
            }
            want.insert("index", Value::Number(index as f64));
            record.insert("index", Value::Number(index as f64));
            if let Some(w) = member(l, "definition") {
                want.insert("definition", w.clone());
                record.insert(
                    "definition",
                    layer
                        .get("d")
                        .and_then(|d| d.get("definition"))
                        .map(Node::to_value)
                        .unwrap_or(Value::Null),
                );
            }
            out.push(item(
                "layer record",
                Kind::Doc(Value::Object(record)),
                "record".into(),
                at,
                &id,
                Value::Object(want),
            ));

            macro_rules! push {
                ($category:expr, $kind:expr, $what:expr, $want:expr, $error_label:expr $(,)?) => {{
                    let mut it = item(
                        $category,
                        $kind,
                        $what.to_owned(),
                        at,
                        &id,
                        $want.cloned().unwrap_or(Value::Null),
                    );
                    it.legacy_threw = layer_error(l, &format!("*{}", $error_label));
                    out.push(it);
                }};
            }
            push!(
                "active",
                Kind::Active,
                "active",
                member(l, "active"),
                "active"
            );

            // Channels, each followed by its expression error.
            if let Some(Value::Object(channels)) = member(l, "channels") {
                for (key, want) in channels.iter() {
                    let k = key.to_string_lossy();
                    let message = layer_error(l, &k);
                    let mut it = item(
                        "channels",
                        Kind::Channel {
                            key: key.clone(),
                            in_p: node_has(layer, "p", key),
                        },
                        format!("channel {k}"),
                        at,
                        &id,
                        want.clone(),
                    );
                    if message.is_some()
                        && !matches!(want, Value::Number(_) | Value::String(_) | Value::Bool(_))
                    {
                        it.legacy_threw = message.clone();
                    }
                    out.push(it);
                    out.push(item(
                        "expression errors",
                        Kind::ExprError { key: key.clone() },
                        format!("expression error {k}"),
                        at,
                        &id,
                        message.map_or(Value::Null, |m| Value::String(JsString::from(m.as_str()))),
                    ));
                }
                // Channels whose evaluation threw (no value, an error message).
                if let Some(Value::Object(errors)) = member(l, "errors") {
                    for (key, message) in errors.iter() {
                        let k = key.to_string_lossy();
                        if k.starts_with('*') || channels.get_key(key).is_some() {
                            continue;
                        }
                        let mut it = item(
                            "channels",
                            Kind::Channel {
                                key: key.clone(),
                                in_p: node_has(layer, "p", key),
                            },
                            format!("channel {k}"),
                            at,
                            &id,
                            Value::Null,
                        );
                        it.legacy_threw = Some(s(Some(message)));
                        out.push(it);
                    }
                }
            }

            // Layer fields.
            if let Some(want) = member(l, "layerFields") {
                let mut raw_fields = Map::new();
                let mut evaluated = Vec::new();
                for (field, key) in [
                    ("blend", "l.blend"),
                    ("mblur", "l.mblur"),
                    ("on", "l.on"),
                    ("matteMode", "l.matteMode"),
                ] {
                    let n = layer.get(field);
                    if pm_eval::node::is_property(n) {
                        evaluated.push((field, key));
                    } else if let Some(v) = n.map(Node::to_value) {
                        raw_fields.insert(field, v);
                    } else if field == "matteMode" {
                        raw_fields.insert(field, Value::Null);
                    }
                }
                for field in ["matteSource", "parent", "group"] {
                    raw_fields.insert(field, raw(layer, field).unwrap_or(Value::Null));
                }
                for field in ["threeD", "solo"] {
                    raw_fields.insert(
                        field,
                        Value::Bool(raw(layer, field).is_some_and(|v| v.is_truthy())),
                    );
                }
                push!(
                    "layerFields",
                    Kind::LayerFields {
                        raw: raw_fields,
                        evaluated,
                    },
                    "layerFields",
                    Some(want),
                    "layerFields",
                );
            }

            if member(l, "content").is_some() {
                push!(
                    "content",
                    Kind::Content,
                    "content",
                    member(l, "content"),
                    "content"
                );
            }
            if ty != "audio" {
                push!(
                    "localMatrix",
                    Kind::LocalMatrix,
                    "localMatrix",
                    member(l, "localMatrix"),
                    "localMatrix"
                );
                push!(
                    "worldMatrix",
                    Kind::WorldMatrix,
                    "worldMatrix",
                    member(l, "worldMatrix"),
                    "worldMatrix"
                );
                push!(
                    "worldOpacity",
                    Kind::WorldOpacity,
                    "worldOpacity",
                    member(l, "worldOpacity"),
                    "worldOpacity"
                );
            }
            let three_d = member(l, "layerFields").and_then(|f| member(f, "threeD"));
            if matches!(three_d, Some(Value::Bool(true))) {
                push!(
                    "planeMatrix",
                    Kind::PlaneMatrix,
                    "planeMatrix",
                    member(l, "planeMatrix"),
                    "planeMatrix"
                );
            }

            // Text: which text layers this layer's geometry depends on.
            let mut deps: Vec<usize> = Vec::new();
            let mut variable = false;
            if ty == "text" {
                if let Some((_, _, v)) = text_layers.iter().find(|(i, _, _)| *i == index) {
                    deps.push(index);
                    variable = *v;
                }
            } else if ty == "group" {
                let gid = layer.get("id").map(Node::to_value);
                for (i, _, v) in &text_layers {
                    let Some(child) = layers.get(*i) else {
                        continue;
                    };
                    let member_of = ev
                        .group_ancestors(comp, child)
                        .unwrap_or_default()
                        .iter()
                        .any(|g| g.get("id").map(Node::to_value) == gid);
                    if member_of {
                        deps.push(*i);
                        variable |= *v;
                    }
                }
            }
            let font = text_layers
                .iter()
                .find(|(i, _, _)| *i == index)
                .map(|(_, f, _)| f.clone());

            if ty == "text" {
                let mut it = item(
                    "textLayout",
                    Kind::TextLayout,
                    "textLayout".into(),
                    at,
                    &id,
                    member(l, "textLayout").cloned().unwrap_or(Value::Null),
                );
                it.legacy_threw = layer_error(l, "*textLayout");
                it.rule = Rule::bounds();
                it.font = font.clone();
                out.push(it);
            }
            if ty != "audio" {
                let category: &'static str = if variable {
                    "bounds (variable font)"
                } else if ty == "text" {
                    "bounds text"
                } else if ty == "group" {
                    "bounds group"
                } else {
                    "bounds"
                };
                let mut it = item(
                    category,
                    Kind::Bounds,
                    format!("{ty} bounds"),
                    at,
                    &id,
                    member(l, "bounds").cloned().unwrap_or(Value::Null),
                );
                it.legacy_threw = layer_error(l, "*bounds");
                it.rule = Rule::bounds();
                if variable {
                    it.rule.class = Some((VARIABLE_INK, VARIABLE_INK_CLASS));
                }
                it.text_deps = deps.clone();
                out.push(it);
            }
            if ty == "text" {
                if let Some(want) = member(l, "textRaster") {
                    let category = if variable {
                        "textRaster (variable font)"
                    } else {
                        "textRaster"
                    };
                    let mut it = item(
                        category,
                        Kind::TextRaster,
                        "textRaster".into(),
                        at,
                        &id,
                        want.clone(),
                    );
                    it.legacy_threw = layer_error(l, "*textRaster");
                    it.rule = Rule::bounds();
                    if variable {
                        it.rule.class = Some((VARIABLE_INK, VARIABLE_INK_CLASS));
                    }
                    it.text_deps = deps.clone();
                    out.push(it);
                }
                if let Some(want) = member(l, "textAnimation") {
                    let mut it = item(
                        "textAnimation",
                        Kind::TextAnimation,
                        "textAnimation".into(),
                        at,
                        &id,
                        want.clone(),
                    );
                    it.legacy_threw = layer_error(l, "*textAnimation");
                    it.text_deps = deps.clone();
                    out.push(it);
                }
            }
            if let Some(want) = member(l, "captionBounds") {
                let mut it = item(
                    "captionBounds",
                    Kind::CaptionBounds,
                    "captionBounds".into(),
                    at,
                    &id,
                    want.clone(),
                );
                it.legacy_threw = layer_error(l, "*captionBounds");
                it.rule = Rule::bounds();
                out.push(it);
            }
            if let Some(want) = member(l, "paths") {
                let ids = layer
                    .get("d")
                    .and_then(|d| d.get("paths"))
                    .map(Node::to_value)
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .map(|p| member(p, "id").cloned().unwrap_or(Value::Null))
                    .collect();
                push!("paths", Kind::Paths { ids }, "paths", Some(want), "paths");
            }
            if matches!(ty.as_str(), "video" | "precomp" | "audio") {
                let nested = comp_id != open;
                let mut it = item(
                    "sourceTime",
                    Kind::SourceTime { legacy: false },
                    "sourceTime".into(),
                    at,
                    &id,
                    member(l, "sourceTime").cloned().unwrap_or(Value::Null),
                );
                it.legacy_threw = layer_error(l, "*sourceTime");
                if nested {
                    // Amendment 16: a nested composition integrates variable
                    // speed at its own rate rather than the open composition's.
                    it.rule.allowed = Some("nested-retime");
                    it.category = "sourceTime (nested)";
                }
                out.push(it.clone());
                it.kind = Kind::SourceTime { legacy: true };
                it.category = "sourceTime, legacy step";
                it.rule.allowed = None;
                it.label = format!("{path} {id} t={time} sourceTime (legacy step)");
                out.push(it);
            }
        }

        // Nested compositions at their source times.
        if let Some(Value::Array(nested)) = member(state, "precomps") {
            for entry in nested {
                let sub_id = s(member(entry, "comp"));
                let sub_state = member(entry, "state").ok_or("precomp entry without state")?;
                let sub = Comp::resolve(doc, &sub_id)
                    .map_err(|e| format!("{path}: composition {sub_id}: {e:?}"))?;
                self.state(
                    out,
                    ev,
                    doc,
                    &sub,
                    project,
                    sub_state,
                    &format!("{path}>{sub_id}"),
                    open,
                )?;
            }
        }
        Ok(())
    }
}
