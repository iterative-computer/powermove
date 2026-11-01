//! Effects and transitions.
//!
//! Ports `addEffect`, `removeEffect`, `setEffect` and `setTransition` of
//! `legacy/core/editing.ts`, `PM.mkEffect` (`legacy/gl/shaders.ts`),
//! `PM.mkTransition` (`legacy/gl/transitions.ts`), the effect stack of the
//! inspector (`extensions/inspector/EffectsSection.svelte`: toggle, reset,
//! remove, reorder, Option+↑/↓) and the effect clipboard
//! (`extensions/inspector/effect-clipboard.ts`).
//!
//! The core holds no effect registry: callers pass the definition (from the
//! effects lane's registry) so `add_effect` can build the instance exactly
//! as `PM.mkEffect` did.

use pm_model::{Map, Value};

use super::{Cx, keyframes, layers, new_id, properties, track, values};
use crate::error::CommandError;
use crate::model::LayerLoc;

/// One parameter of an effect or transition definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDefinition {
    /// The key (`k`).
    pub key: String,
    /// `number`, `color`, `toggle`, `select`, `point`, `layer`, `texture`, ...
    pub kind: String,
    /// The default (`def`, already resolved for the layer when it has a
    /// `defaultFrom`).
    pub default: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// A layer/texture reference input (stored in `inputs`, not `p`).
    pub reference: bool,
}

/// What `PM.mkEffect` needs from the registry.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDefinition {
    /// `type` (`blur`, `glow`, ...).
    pub effect_type: String,
    pub version: u32,
    pub label: String,
    pub params: Vec<ParamDefinition>,
    /// Supports source-owner processing (`sourceProcessing`).
    pub source_processing: bool,
    /// Reads the backdrop (refuses source processing).
    pub backdrop: bool,
}

/// What `PM.mkTransition` needs.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionDefinition {
    pub transition_type: String,
    pub params: Vec<ParamDefinition>,
}

/// `add_effect`.
#[derive(Clone, Debug, PartialEq)]
pub struct AddEffect {
    pub layer_id: String,
    /// The instance id (make one with [`super::new_id`]`("f")`).
    pub effect_id: String,
    pub definition: EffectDefinition,
    /// Parameter values by key: a plain value sets `v`; a whole channel
    /// `{ v, kf, expr }` (effect paste) keeps its animation with fresh key
    /// ids. Unknown keys are ignored, as before.
    pub parameters: Map,
    /// Layer inputs by key (`validateEffectInputs`).
    pub inputs: Option<Value>,
    /// Expanded in the inspector (default true).
    pub open: bool,
    /// `on` (default true).
    pub enabled: bool,
    /// An animated `on` channel (effect paste).
    pub enabled_animation: Option<Value>,
    /// Stack position (default: the end).
    pub index: Option<usize>,
}

/// The editable fields of `set_effect`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectPatch {
    /// `on` (keys `<id>.$enabled` when animated).
    pub enabled: Option<bool>,
    /// Inspector disclosure.
    pub open: Option<bool>,
    /// Stack index, clamped.
    pub index: Option<usize>,
    /// Merged over the existing inputs.
    pub inputs: Option<Value>,
    /// The playhead: where an animated Enabled switch is keyed.
    pub time: f64,
}

/// In or out transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}

/// `set_transition`'s transition.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionSpec {
    pub definition: TransitionDefinition,
    /// Seconds, 0.02–600 (default 0.5).
    pub duration: Option<f64>,
    /// Static values by key; colors `#RRGGBB`, toggles booleans, else finite
    /// numbers. The same type keeps its existing channels (keys survive).
    pub params: Map,
}

/// Effect commands. Layers whose type has no effects refuse
/// (`<Type> layers do not support visual effects`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EffectCommand {
    /// `add_effect` (`addEffect`). Label "Add Effect".
    Add(Box<AddEffect>),
    /// `remove_effect` (`removeEffect`): by id, or every effect of a type.
    /// `Effect not found: x`. Label "Remove Effect".
    Remove { layer_id: String, effect: String },
    /// `set_effect` (`setEffect`): toggle, disclosure, reorder, inputs.
    /// Label "Toggle Effect" / "Reorder Effect" / "Edit Effect".
    Set {
        layer_id: String,
        effect: String,
        patch: EffectPatch,
    },
    /// Duplicate effects in place (each copy right after its original, fresh
    /// key ids, ids from `new_ids`). Label "Duplicate Effect".
    Duplicate {
        layer_id: String,
        effect_ids: Vec<String>,
        new_ids: Vec<String>,
    },
    /// Reset: every parameter back to its definition default (keys and
    /// expressions removed), inputs cleared. Label "Reset Effect".
    Reset {
        layer_id: String,
        effect_id: String,
        definition: EffectDefinition,
    },
    /// `set_transition` (`setTransition`): `None` removes the edge's
    /// transition. Label "Set Transition" / "Remove Transition".
    SetTransition {
        layer_id: String,
        edge: Edge,
        transition: Option<Box<TransitionSpec>>,
    },
}

impl EffectCommand {
    pub fn label(&self) -> String {
        match self {
            EffectCommand::Add(_) => "Add Effect",
            EffectCommand::Remove { .. } => "Remove Effect",
            EffectCommand::Set { patch, .. } => match patch {
                EffectPatch {
                    enabled: Some(_),
                    open: None,
                    index: None,
                    inputs: None,
                    ..
                } => "Toggle Effect",
                EffectPatch {
                    enabled: None,
                    open: None,
                    index: Some(_),
                    inputs: None,
                    ..
                } => "Reorder Effect",
                _ => "Edit Effect",
            },
            EffectCommand::Duplicate { .. } => "Duplicate Effect",
            EffectCommand::Reset { .. } => "Reset Effect",
            EffectCommand::SetTransition {
                transition: Some(_),
                ..
            } => "Set Transition",
            EffectCommand::SetTransition { .. } => "Remove Transition",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            EffectCommand::Add(c) => add_effect(cx, c),
            EffectCommand::Remove { layer_id, effect } => remove_effect(cx, layer_id, effect),
            EffectCommand::Set {
                layer_id,
                effect,
                patch,
            } => set_effect(cx, layer_id, effect, patch),
            EffectCommand::Duplicate {
                layer_id,
                effect_ids,
                new_ids,
            } => duplicate(cx, layer_id, effect_ids, new_ids),
            EffectCommand::Reset {
                layer_id,
                effect_id,
                definition,
            } => reset(cx, layer_id, effect_id, definition),
            EffectCommand::SetTransition {
                layer_id,
                edge,
                transition,
            } => set_transition(cx, layer_id, *edge, transition.as_deref()),
        }
    }
}

/// The layer, refusing types without effects
/// (`<Type> layers do not support visual effects`).
fn effect_layer(cx: &Cx<'_>, layer_id: &str) -> Result<(LayerLoc, Value), CommandError> {
    let (loc, layer) = cx.layer_mut_checked(layer_id)?;
    let t = values::text(&layer, "type").unwrap_or_default();
    if !layers::has_effects(t) {
        return Err(CommandError::Invalid(format!(
            "{} layers do not support visual effects",
            capitalized(t)
        )));
    }
    Ok((loc, layer))
}

fn capitalized(t: &str) -> String {
    let mut c = t.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn fx_list(layer: &mut Value) -> &mut Vec<Value> {
    let m = values::obj(layer);
    if m.get("fx").and_then(Value::as_array).is_none() {
        m.insert("fx", Value::Array(Vec::new()));
    }
    m.get_mut("fx").and_then(Value::as_array_mut).expect("made")
}

fn fx_at(layer: &Value, i: usize) -> Option<&Value> {
    layer.get("fx")?.as_array()?.get(i)
}

/// `layer.fx.find(item => item.id === effect || item.type === effect)`.
fn find_effect(layer: &Value, effect: &str) -> Option<usize> {
    layer.get("fx")?.as_array()?.iter().position(|e| {
        values::text(e, "id") == Some(effect) || values::text(e, "type") == Some(effect)
    })
}

/// `emptyReference(type)`.
fn empty_reference(kind: &str) -> Value {
    let mut m = Map::new();
    match kind {
        "path" => {
            m.insert("kind", values::s("path"));
            m.insert("layerId", Value::Null);
            m.insert("pathId", Value::Null);
        }
        "asset" => {
            m.insert("kind", values::s("asset"));
            m.insert("assetId", Value::Null);
        }
        _ => {
            m.insert("layerId", Value::Null);
            m.insert("stage", values::s("source"));
        }
    }
    Value::Object(m)
}

/// `validateEffectInputs(value)` (shape checks), then the keys against the
/// definition (`Unknown layer input: key`).
fn check_inputs(inputs: &Value, def: Option<&EffectDefinition>) -> Result<Map, CommandError> {
    let bad = |m: &str| CommandError::Invalid(m.to_owned());
    let map = inputs
        .as_object()
        .ok_or_else(|| bad("Effect inputs must be an object"))?;
    let id_ok = |v: Option<&Value>| match v {
        Some(Value::Null) => true,
        Some(Value::String(s)) => {
            let s = s.to_string_lossy();
            !s.trim().is_empty() && s.encode_utf16().count() <= 256
        }
        _ => false,
    };
    let mut out = Map::new();
    for (k, r) in map.iter() {
        let key = k.to_string_lossy();
        let key_ok = key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && key.chars().all(|c| c.is_ascii_alphanumeric());
        if !key_ok || r.as_object().is_none() {
            return Err(bad("Invalid effect reference"));
        }
        let kind = values::text(r, "kind");
        let fields: Vec<String> = r
            .as_object()
            .map(|m| m.keys().map(|k| k.to_string_lossy()).collect())
            .unwrap_or_default();
        match kind {
            Some("path") => {
                let (l, p) = (r.get("layerId"), r.get("pathId"));
                if !id_ok(l)
                    || !id_ok(p)
                    || l.is_some_and(Value::is_null) != p.is_some_and(Value::is_null)
                    || fields
                        .iter()
                        .any(|f| !["kind", "layerId", "pathId"].contains(&f.as_str()))
                {
                    return Err(bad("Invalid path reference"));
                }
            }
            Some("asset") => {
                if !id_ok(r.get("assetId"))
                    || fields
                        .iter()
                        .any(|f| !["kind", "assetId"].contains(&f.as_str()))
                {
                    return Err(bad("Invalid asset reference"));
                }
            }
            _ => {
                if !id_ok(r.get("layerId")) {
                    return Err(bad("Invalid effect reference"));
                }
            }
        }
        if let Some(def) = def
            && !def.params.iter().any(|p| {
                p.key == key
                    && ((p.kind == "layer" && kind.is_none()) || Some(p.kind.as_str()) == kind)
            })
        {
            return Err(CommandError::Invalid(format!("Unknown layer input: {key}")));
        }
        out.insert(k.clone(), r.clone());
    }
    if out.len() > 4 {
        return Err(bad("At most four effect references are supported"));
    }
    Ok(out)
}

/// A pasted channel `{ v, kf, expr }` with fresh key ids.
fn fresh_channel(base: &Value, channel: &Value) -> Value {
    let mut out = base.clone();
    let m = values::obj(&mut out);
    m.insert("v", channel.get("v").cloned().unwrap_or(Value::Null));
    m.insert(
        "expr",
        match channel.get("expr") {
            Some(Value::String(s)) => Value::String(s.clone()),
            _ => Value::Null,
        },
    );
    let keys: Vec<Value> = channel
        .get("kf")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .map(|k| {
                    let mut k = k.clone();
                    values::obj(&mut k).insert("i", values::s(&new_id("k")));
                    k
                })
                .collect()
        })
        .unwrap_or_default();
    m.insert("kf", Value::Array(keys));
    out
}

/// `PM.mkEffect(type, version, layer)`.
fn make_effect(id: &str, def: &EffectDefinition) -> Value {
    let mut p = Map::new();
    let mut inputs = Map::new();
    for param in &def.params {
        if param.reference {
            inputs.insert(param.key.as_str(), empty_reference(&param.kind));
        } else {
            p.insert(param.key.as_str(), values::property(param.default.clone()));
        }
    }
    let mut m = Map::new();
    m.insert("id", values::s(id));
    m.insert("type", values::s(&def.effect_type));
    m.insert("version", Value::Number(f64::from(def.version.max(1))));
    m.insert("on", Value::Bool(true));
    m.insert("p", Value::Object(p));
    if !inputs.is_empty() {
        m.insert("inputs", Value::Object(inputs));
    }
    Value::Object(m)
}

/// `addEffect(command)`.
fn add_effect(cx: &mut Cx<'_>, c: &AddEffect) -> Result<(), CommandError> {
    let (loc, mut layer) = effect_layer(cx, &c.layer_id)?;
    layers::safe_patch(&c.parameters, "effect parameters")?;
    let fps = cx.fps(loc.comp_id.as_deref());
    let from = values::num(&layer, "from").unwrap_or(0.0);
    let mut fx = make_effect(&c.effect_id, &c.definition);
    {
        let m = values::obj(&mut fx);
        m.insert("open", Value::Bool(c.open));
        m.insert("on", Value::Bool(c.enabled));
        if let Some(saved) = c
            .enabled_animation
            .as_ref()
            .filter(|v| values::is_property(v))
        {
            let keys = keyframes::normalize_keyframes(
                saved
                    .get("kf")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|k| track::js_number(k.get("t").unwrap_or(&Value::Null)) >= -from)
                    .collect(),
                &Value::Bool(true),
            )
            .into_iter()
            .map(|mut k| {
                values::obj(&mut k).insert("i", values::s(&new_id("k")));
                k
            })
            .collect();
            let mut on = Map::new();
            on.insert(
                "v",
                Value::Bool(saved.get("v").and_then(Value::as_bool) != Some(false)),
            );
            on.insert(
                "expr",
                match saved.get("expr") {
                    Some(Value::String(s)) => Value::String(s.clone()),
                    _ => Value::Null,
                },
            );
            on.insert("kf", Value::Array(keys));
            m.insert("on", Value::Object(on));
        }
        let p = m.get_mut("p").and_then(Value::as_object_mut).expect("made");
        for (k, v) in c.parameters.iter() {
            let Some(base) = p.get_key(k).cloned() else {
                continue;
            };
            let next = if v.as_object().is_some_and(|o| o.contains_key("v")) {
                fresh_channel(&base, v)
            } else {
                let mut b = base;
                values::obj(&mut b).insert("v", v.clone());
                b
            };
            p.insert(k.clone(), next);
        }
    }
    let _ = fps;
    if let Some(inputs) = &c.inputs {
        let checked = check_inputs(inputs, Some(&c.definition))?;
        let m = values::obj(&mut fx);
        if m.get("inputs").and_then(Value::as_object).is_none() {
            m.insert("inputs", Value::Object(Map::new()));
        }
        let existing = m
            .get_mut("inputs")
            .and_then(Value::as_object_mut)
            .expect("made");
        for (k, v) in checked.iter() {
            existing.insert(k.clone(), v.clone());
        }
    }
    if find_effect(&layer, &c.effect_id)
        .and_then(|i| fx_at(&layer, i))
        .is_some_and(|e| values::text(e, "id") == Some(c.effect_id.as_str()))
    {
        return Err(CommandError::Invalid(format!(
            "Effect id already exists: {}",
            c.effect_id
        )));
    }
    let list = fx_list(&mut layer);
    let at = c.index.unwrap_or(list.len()).min(list.len());
    list.insert(at, fx);
    cx.put_layer(&loc, layer)
}

/// `removeEffect(command)`: by id, or every effect of a type.
fn remove_effect(cx: &mut Cx<'_>, layer_id: &str, effect: &str) -> Result<(), CommandError> {
    let (loc, mut layer) = effect_layer(cx, layer_id)?;
    let list = fx_list(&mut layer);
    let before = list.len();
    list.retain(|e| {
        values::text(e, "id") != Some(effect) && values::text(e, "type") != Some(effect)
    });
    if list.len() == before {
        return Err(CommandError::EffectNotFound(effect.to_owned()));
    }
    cx.put_layer(&loc, layer)
}

/// `setEffect(command)`.
fn set_effect(
    cx: &mut Cx<'_>,
    layer_id: &str,
    effect: &str,
    patch: &EffectPatch,
) -> Result<(), CommandError> {
    let (loc, mut layer) = effect_layer(cx, layer_id)?;
    let index = find_effect(&layer, effect)
        .ok_or_else(|| CommandError::EffectNotFound(effect.to_owned()))?;
    let id = fx_at(&layer, index)
        .and_then(|e| values::text(e, "id"))
        .unwrap_or_default()
        .to_owned();
    if let Some(inputs) = &patch.inputs {
        // The definition is the registry's; the shape is checked here.
        let checked = check_inputs(inputs, None)?;
        let fx = &mut fx_list(&mut layer)[index];
        let m = values::obj(fx);
        if m.get("inputs").and_then(Value::as_object).is_none() {
            m.insert("inputs", Value::Object(Map::new()));
        }
        let existing = m
            .get_mut("inputs")
            .and_then(Value::as_object_mut)
            .expect("made");
        for (k, v) in checked.iter() {
            existing.insert(k.clone(), v.clone());
        }
    }
    if let Some(open) = patch.open {
        values::obj(&mut fx_list(&mut layer)[index]).insert("open", Value::Bool(open));
    }
    if let Some(i) = patch.index {
        let list = fx_list(&mut layer);
        let to = i.min(list.len().saturating_sub(1));
        if to != index {
            let e = list.remove(index);
            list.insert(to, e);
        }
    }
    let enabled = patch.enabled;
    let animated = layer
        .get("fx")
        .and_then(Value::as_array)
        .and_then(|l| {
            l.iter()
                .find(|e| values::text(e, "id") == Some(id.as_str()))
        })
        .and_then(|e| e.get("on"))
        .is_some_and(values::is_property);
    if let Some(on) = enabled.filter(|_| !animated) {
        let list = fx_list(&mut layer);
        if let Some(e) = list
            .iter_mut()
            .find(|e| values::text(e, "id") == Some(id.as_str()))
        {
            values::obj(e).insert("on", Value::Bool(on));
        }
    }
    cx.put_layer(&loc, layer)?;
    if let Some(on) = enabled.filter(|_| animated) {
        let mut c =
            properties::SetProperty::new(layer_id, format!("{id}.$enabled"), on, patch.time);
        c.preserve_hand_edits = false;
        return properties::set_property(cx, &c);
    }
    Ok(())
}

/// Duplicate effects in place.
fn duplicate(
    cx: &mut Cx<'_>,
    layer_id: &str,
    effect_ids: &[String],
    new_ids: &[String],
) -> Result<(), CommandError> {
    if effect_ids.len() != new_ids.len() {
        return Err(CommandError::Invalid(
            "new_ids must pair with effect_ids".into(),
        ));
    }
    let (loc, mut layer) = effect_layer(cx, layer_id)?;
    for (id, new) in effect_ids.iter().zip(new_ids) {
        let list = fx_list(&mut layer);
        let index = list
            .iter()
            .position(|e| values::text(e, "id") == Some(id.as_str()))
            .ok_or_else(|| CommandError::EffectNotFound(id.clone()))?;
        let mut copy = list[index].clone();
        values::obj(&mut copy).insert("id", values::s(new));
        renew_effect_keys(&mut copy);
        list.insert(index + 1, copy);
    }
    cx.put_layer(&loc, layer)
}

fn renew_effect_keys(fx: &mut Value) {
    let renew = |ch: &mut Value| {
        if let Some(keys) = ch
            .as_object_mut()
            .and_then(|m| m.get_mut("kf"))
            .and_then(Value::as_array_mut)
        {
            for k in keys {
                values::obj(k).insert("i", values::s(&new_id("k")));
            }
        }
    };
    if let Some(on) = fx.as_object_mut().and_then(|m| m.get_mut("on")) {
        renew(on);
    }
    if let Some(p) = fx
        .as_object_mut()
        .and_then(|m| m.get_mut("p"))
        .and_then(Value::as_object_mut)
    {
        for (_, ch) in p.iter_mut() {
            renew(ch);
        }
    }
}

/// Reset: parameters back to their defaults, inputs emptied.
fn reset(
    cx: &mut Cx<'_>,
    layer_id: &str,
    effect_id: &str,
    def: &EffectDefinition,
) -> Result<(), CommandError> {
    let (loc, mut layer) = effect_layer(cx, layer_id)?;
    let list = fx_list(&mut layer);
    let index = list
        .iter()
        .position(|e| values::text(e, "id") == Some(effect_id))
        .ok_or_else(|| CommandError::EffectNotFound(effect_id.to_owned()))?;
    let fresh = make_effect(effect_id, def);
    let e = values::obj(&mut list[index]);
    e.insert("p", fresh.get("p").cloned().unwrap_or_default());
    match fresh.get("inputs") {
        Some(i) => {
            e.insert("inputs", i.clone());
        }
        None => {
            e.remove("inputs");
        }
    }
    cx.put_layer(&loc, layer)
}

fn is_hex_color(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        let h = s.strip_prefix('#').unwrap_or("");
        (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// `setTransition(command)`.
fn set_transition(
    cx: &mut Cx<'_>,
    layer_id: &str,
    edge: Edge,
    transition: Option<&TransitionSpec>,
) -> Result<(), CommandError> {
    let (loc, mut layer) = cx.layer_mut_checked(layer_id)?;
    let field = match edge {
        Edge::In => "transitionIn",
        Edge::Out => "transitionOut",
    };
    let Some(spec) = transition else {
        values::obj(&mut layer).insert(field, Value::Null);
        return cx.put_layer(&loc, layer);
    };
    let def = &spec.definition;
    if def.transition_type.is_empty() {
        return Err(CommandError::Invalid("Transition type is required".into()));
    }
    let duration = match spec.duration {
        None => 0.5,
        Some(d) => values::finite(d, "transition duration")?,
    };
    if !(0.02..=600.0).contains(&duration) {
        return Err(CommandError::Invalid(
            "Transition duration must be between 0.02 and 600 seconds".into(),
        ));
    }
    layers::safe_patch(&spec.params, "transition parameters")?;
    for (k, v) in spec.params.iter() {
        let key = k.to_string_lossy();
        let param =
            def.params.iter().find(|p| p.key == key).ok_or_else(|| {
                CommandError::Invalid(format!("Unknown transition parameter: {key}"))
            })?;
        let ok = match param.kind.as_str() {
            "color" => is_hex_color(v),
            "toggle" => v.as_bool().is_some(),
            _ => v.as_f64().is_some_and(f64::is_finite),
        };
        if !ok {
            let what = match param.kind.as_str() {
                "color" => "a hex color",
                "toggle" => "a boolean",
                _ => "a finite number",
            };
            return Err(CommandError::Invalid(format!(
                "Transition parameter “{key}” must be {what}"
            )));
        }
    }
    let mut p = Map::new();
    for param in &def.params {
        p.insert(param.key.as_str(), values::property(param.default.clone()));
    }
    let existing = layer.get(field).cloned().unwrap_or(Value::Null);
    if values::text(&existing, "type") == Some(def.transition_type.as_str())
        && !values::flag(&existing, "missing")
        && let Some(old) = existing.get("p").and_then(Value::as_object)
    {
        for param in &def.params {
            if let Some(ch) = old.get(&param.key).filter(|c| c.is_truthy()) {
                p.insert(param.key.as_str(), ch.clone());
            }
        }
    }
    for (k, v) in spec.params.iter() {
        if let Some(ch) = p.get_key_mut(k) {
            values::obj(ch).insert("v", v.clone());
        }
    }
    let mut t = Map::new();
    t.insert("type", values::s(&def.transition_type));
    t.insert("dur", Value::Number(duration));
    t.insert("p", Value::Object(p));
    values::obj(&mut layer).insert(field, Value::Object(t));
    cx.put_layer(&loc, layer)
}
