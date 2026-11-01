//! Layer records: fields and switches, creation, deletion, duplication,
//! paste, and stack order.
//!
//! Ports `setLayer`, `addLayer`, `deleteLayers`, `reorderLayer` and the
//! `delete_layers` lock policy of `legacy/core/editing.ts`, `PM.mkLayer`,
//! `PM.cloneLayer` and `PM.removeLayers` of `legacy/core/model.ts`, and the
//! stacking shortcuts of `ui/shortcuts.ts`.

use std::collections::HashSet;

use pm_model::{Map, Value};

use super::properties::{self, SetProperty};
use super::values::{self, property};
use super::{Cx, channels, new_id};
use crate::error::CommandError;

/// Blend modes (`PM.BLENDS`).
pub const BLEND_MODES: [&str; 9] = [
    "normal",
    "add",
    "screen",
    "multiply",
    "overlay",
    "softlight",
    "difference",
    "lighten",
    "darken",
];

/// Layer types (`TYPE_META`).
pub const LAYER_TYPES: [&str; 13] = [
    "solid",
    "text",
    "shape",
    "image",
    "video",
    "audio",
    "adjustment",
    "shader",
    "extension",
    "null",
    "precomp",
    "group",
    "captions",
];

/// A reparent request: the new parent (or none) and the time whose pose is
/// kept (`preserveParentPose(PM, layer, parent, PM.time)`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParentChange {
    pub parent: Option<String>,
    /// The playhead (composition seconds).
    pub time: f64,
}

/// Where a layer's effects run (`effectSpace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectSpace {
    Composition,
    Source,
}

/// The editable fields of `set_layer` (`LAYER_FIELDS`). `None` leaves a
/// field alone. Build with struct-update syntax:
/// `LayerPatch { solo: Some(true), ..Default::default() }`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerPatch {
    /// Trimmed; blank keeps the old name.
    pub name: Option<String>,
    /// `from` (start on the timeline, seconds). Finite.
    pub from: Option<f64>,
    /// `dur`, at least one frame. Finite.
    pub duration: Option<f64>,
    /// `on`.
    pub visible: Option<bool>,
    /// `lock` (a bare unlock is allowed on a locked layer for humans).
    pub locked: Option<bool>,
    pub shy: Option<bool>,
    pub solo: Option<bool>,
    /// One of [`BLEND_MODES`]; keys `l.blend` when it is animated. Audio
    /// layers only accept `normal`.
    pub blend: Option<String>,
    /// `mblur`; keys `l.mblur` when animated. Audio layers refuse `true`.
    pub motion_blur: Option<bool>,
    /// Reparent keeping the world pose (and 3D aim). Refuses self, a layer
    /// that cannot be a parent, a cycle; audio layers cannot have parents.
    pub parent: Option<ParentChange>,
    /// Track matte source (`matteSource`, `Some(None)` removes it); refuses
    /// a cycle; sets `matteMode` to alpha when the layer had none.
    pub matte_source: Option<Option<String>>,
    /// Label color.
    pub color: Option<String>,
    /// Timeline disclosure (`collapsed`).
    pub collapsed: Option<bool>,
    /// The 3D switch. Models, lights and cameras are always 3D; audio and
    /// adjustment layers cannot be.
    pub three_d: Option<bool>,
    /// Scale link (`scaleLinked`).
    pub scale_linked: Option<bool>,
    /// Separate Dimensions (`separateDimensions`).
    pub separate_dimensions: Option<bool>,
    /// Pre-comp layers only (`continuousRaster`).
    pub continuous_raster: Option<bool>,
    /// Effect processing space; `source` requires a 2D content layer whose
    /// effects all support source processing.
    pub effect_space: Option<EffectSpace>,
    /// The playhead: where an animated blend mode or motion blur switch is
    /// keyed (`PM.time` in the old command).
    pub time: f64,
}

/// `add_layer`: a new layer from `PM.mkLayer(type)` defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct AddLayer {
    /// The new layer's id (make one with [`super::new_id`]`("L")`); refused
    /// when taken (`Layer id already exists: id`).
    pub id: String,
    /// One of [`LAYER_TYPES`] (`Unknown layer type: x`).
    pub layer_type: String,
    /// Target composition (`None`: the open one).
    pub comp: Option<String>,
    /// Default: the type's label.
    pub name: Option<String>,
    /// Start; default: the playhead snapped to a frame.
    pub from: Option<f64>,
    /// Default: to the end of the composition (at least 0.1 s).
    pub duration: Option<f64>,
    /// Merged over the type's default content (`d`).
    pub content: Map,
    /// Static values of existing channels (`{ "opacity": 50 }`).
    pub properties: Map,
    pub color: Option<String>,
    /// Stack index (0 = top, the default), clamped.
    pub index: Option<usize>,
    pub parent: Option<String>,
    pub blend: Option<String>,
    pub motion_blur: Option<bool>,
    pub visible: Option<bool>,
    pub shy: Option<bool>,
    pub collapsed: Option<bool>,
    /// The playhead (for the default `from`).
    pub time: f64,
}

/// Stacking shortcuts (Cmd+[ / Cmd+] and their Shift variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restack {
    Forward,
    Backward,
    Front,
    Back,
}

/// Layer commands. Undo labels are on [`LayerCommand::label`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayerCommand {
    /// `set_layer` (`setLayer`): any of [`LayerPatch`]'s fields. Locked layers
    /// refuse everything but a human's bare unlock. Label: the field's name
    /// ("Rename Layer", "Hide Layer", "Solo Layer", "Set Parent", ...), else
    /// "Edit Layer".
    Set { layer_id: String, patch: LayerPatch },
    /// `add_layer` (`addLayer`). Label "Add Layer". The caller selects the
    /// new layer (the old command did unless `select: false`).
    Add(Box<AddLayer>),
    /// `delete_layers`: the layers and everything inside the groups among
    /// them; children parented to a deleted layer are unparented. Locked
    /// layers are skipped (all locked: `All targeted layers are locked`; a
    /// group with locked contents: `Unlock the group and its contents before
    /// deleting`). Label "Delete Layers".
    Delete { layer_ids: Vec<String> },
    /// Cmd+D (`duplicate`: `pasteLayers` of the selection in place): copies
    /// (`PM.cloneLayer`: numbered names, fresh key ids) inserted above the
    /// topmost one, links between copies remapped. Any locked layer refuses.
    /// `new_ids` pairs with `layer_ids` (same length). Label "Duplicate".
    Duplicate {
        layer_ids: Vec<String>,
        new_ids: Vec<String>,
    },
    /// Cmd+V layers (`pasteLayers`): clipboard records cloned
    /// (`PM.cloneLayer`), ids from `new_ids` (same length); parents, groups,
    /// mattes and effect layer inputs follow the copies or stay when they
    /// name a layer of the target composition. Inserted at `index` (the
    /// topmost selected row; default the top). With `time` (the playhead)
    /// the block keeps its relative timing and starts there (its earliest
    /// non-group layer). Label "Paste".
    Paste {
        comp: Option<String>,
        layers: Vec<Value>,
        new_ids: Vec<String>,
        index: Option<usize>,
        time: Option<f64>,
    },
    /// `reorder_layer` (`reorderLayer`): moves the layer (with a group's
    /// members) to `index`, entering or leaving the destination's group.
    /// `Unlock the destination before moving layers`. Entering or leaving a
    /// group keeps the world pose at `time` (the playhead). Label "Reorder
    /// Layer".
    Reorder {
        layer_id: String,
        index: usize,
        time: f64,
    },
    /// Bring Forward / Send Backward / Bring to Front / Send to Back
    /// (`orderLayers`): unlocked layers step past neighbours that are neither
    /// selected nor locked (locked layers are barriers). Label "Bring
    /// forward", "Send backward", "Bring to front", "Send to back".
    Restack { layer_ids: Vec<String>, to: Restack },
    /// `transform_layers`: a generated tool's higher-order edit, compiled to
    /// primitive commands (`PM.Capabilities.compile`). Not ported.
    TransformLayers { transform: Value, state: Value },
}

impl LayerCommand {
    pub fn label(&self) -> String {
        match self {
            LayerCommand::Set { patch, .. } => patch_label(patch),
            LayerCommand::Add(_) => "Add Layer",
            LayerCommand::Delete { .. } => "Delete Layers",
            LayerCommand::Duplicate { .. } => "Duplicate",
            LayerCommand::Paste { .. } => "Paste",
            LayerCommand::Reorder { .. } => "Reorder Layer",
            LayerCommand::Restack { to, .. } => match to {
                Restack::Forward => "Bring forward",
                Restack::Backward => "Send backward",
                Restack::Front => "Bring to front",
                Restack::Back => "Send to back",
            },
            LayerCommand::TransformLayers { .. } => "Transform Layers",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            LayerCommand::Set { layer_id, patch } => set_layer(cx, layer_id, patch),
            LayerCommand::Add(c) => add_layer(cx, c),
            LayerCommand::Delete { layer_ids } => delete_layers(cx, layer_ids),
            LayerCommand::Duplicate { layer_ids, new_ids } => duplicate(cx, layer_ids, new_ids),
            LayerCommand::Paste {
                comp,
                layers,
                new_ids,
                index,
                time,
            } => paste(cx, comp.as_deref(), layers, new_ids, *index, *time),
            LayerCommand::Reorder {
                layer_id,
                index,
                time,
            } => reorder(cx, layer_id, *index, *time),
            LayerCommand::Restack { layer_ids, to } => restack(cx, layer_ids, *to),
            LayerCommand::TransformLayers { .. } => {
                Err(CommandError::NotImplemented("transform_layers"))
            }
        }
    }
}

/// `TYPE_META[type].transform !== false`.
pub(crate) fn transformable(layer_type: &str) -> bool {
    layer_type != "audio"
}

/// `TYPE_META[type].effects !== false`.
pub(crate) fn has_effects(layer_type: &str) -> bool {
    layer_type != "audio"
}

fn type_label(t: &str) -> &'static str {
    match t {
        "solid" => "Solid",
        "text" => "Text",
        "shape" => "Shape",
        "image" => "Image",
        "video" => "Video",
        "audio" => "Audio",
        "adjustment" => "Adjustment",
        "shader" => "Shader",
        "extension" => "Extension",
        "null" => "Null",
        "group" => "Group",
        "precomp" => "Precomp",
        "captions" => "Captions",
        _ => "Layer",
    }
}

fn type_color(t: &str) -> &'static str {
    match t {
        "solid" | "audio" => "#4C8DFF",
        "text" => "#E8E2CF",
        "shape" => "#6C7BE8",
        "image" => "#A9A9AE",
        "video" => "#3B62E8",
        "adjustment" => "#A78BFA",
        "shader" => "#FF6B1A",
        "extension" => "#9B8CFF",
        "null" => "#6a6a70",
        "group" | "precomp" => "#3FCF8E",
        "captions" => "#F2C14E",
        _ => "#A9A9AE",
    }
}

/// `CHANNELS_3D` (`legacy/core/space-3d.ts`).
pub(crate) const CHANNELS_3D: [(&str, f64); 9] = [
    ("perspective", 50.0),
    ("position.z", 0.0),
    ("anchor.z", 0.0),
    ("scale.z", 100.0),
    ("rotation.x", 0.0),
    ("rotation.y", 0.0),
    ("orientation.x", 0.0),
    ("orientation.y", 0.0),
    ("orientation.z", 0.0),
];

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(*k, v.clone());
    }
    Value::Object(m)
}

/// `PM.mkLayer(type, opts, comp)` for the types the core can build without
/// a registry (not captions, extension or shader layers).
pub(crate) fn make_layer(
    layer_type: &str,
    id: &str,
    name: Option<&str>,
    comp: (f64, f64, f64),
    content: &Map,
    properties: &Map,
) -> Result<Value, CommandError> {
    let (w, h, dur) = comp;
    if matches!(layer_type, "captions" | "extension" | "shader") {
        return Err(CommandError::NotImplemented(match layer_type {
            "captions" => "add_layer (captions: use add_captions)",
            "extension" => "add_layer (extension layers need their definition)",
            _ => "add_layer (shader layers need the shader template)",
        }));
    }
    let mut p = Map::new();
    if transformable(layer_type) {
        for (k, v) in CHANNELS_3D {
            p.insert(k, property(n(v)));
        }
        for (k, v) in [
            ("anchor.x", 0.0),
            ("anchor.y", 0.0),
            ("position.x", w / 2.0),
            ("position.y", h / 2.0),
            ("scale.x", 100.0),
            ("scale.y", 100.0),
            ("rotation", 0.0),
            ("opacity", 100.0),
            ("skew", 0.0),
        ] {
            p.insert(k, property(n(v)));
        }
    }
    let label = name
        .filter(|s| !s.is_empty())
        .unwrap_or(type_label(layer_type));
    let mut l = Map::new();
    l.insert("id", values::s(id));
    l.insert("type", values::s(layer_type));
    l.insert("name", values::s(label));
    l.insert("from", n(0.0));
    l.insert("dur", n(dur));
    l.insert("on", Value::Bool(true));
    l.insert("lock", Value::Bool(false));
    l.insert("shy", Value::Bool(false));
    l.insert("collapsed", Value::Bool(true));
    l.insert("color", values::s(type_color(layer_type)));
    l.insert("blend", values::s("normal"));
    l.insert("mblur", Value::Bool(false));
    l.insert("parent", Value::Null);
    l.insert("fx", Value::Array(Vec::new()));
    l.insert("transitionIn", Value::Null);
    l.insert("transitionOut", Value::Null);
    l.insert("masks", Value::Array(Vec::new()));
    l.insert("locked_intent", Value::Object(Map::new()));
    let set_p = |p: &mut Map, k: &str, v: f64| {
        if let Some(ch) = p.get_mut(k).and_then(Value::as_object_mut) {
            ch.insert("v", n(v));
        }
    };
    let d = match layer_type {
        "solid" => obj(&[
            ("color", values::s("#1b1b1f")),
            ("w", n(w)),
            ("h", n(h)),
            ("radius", n(0.0)),
        ]),
        "shape" => obj(&[
            ("shape", values::s("rect")),
            ("color", values::s("#E8E2CF")),
            ("w", n(480.0)),
            ("h", n(480.0)),
            ("radius", n(24.0)),
            ("stroke", n(0.0)),
            ("strokeColor", values::s("#ffffff")),
            ("points", n(5.0)),
        ]),
        "text" => obj(&[
            ("text", values::s("Powermove")),
            ("boxWidth", property(n(0.0))),
            ("boxHeight", property(n(0.0))),
            ("font", values::s("SF Pro Display")),
            ("weight", n(600.0)),
            ("size", n(128.0)),
            ("tracking", n(-2.0)),
            ("leading", n(1.1)),
            ("color", values::s("#F2F2F2")),
            ("align", values::s("center")),
            ("italic", Value::Bool(false)),
        ]),
        "image" => obj(&[
            ("asset", Value::Null),
            ("fit", values::s("cover")),
            ("w", n(1920.0)),
            ("h", n(1080.0)),
        ]),
        "video" => obj(&[
            ("asset", Value::Null),
            ("fit", values::s("cover")),
            ("trim", n(0.0)),
            ("speed", n(1.0)),
            ("embeddedAudio", Value::Bool(false)),
            ("w", n(1920.0)),
            ("h", n(1080.0)),
        ]),
        "audio" => {
            l.insert("color", values::s("#4C8DFF"));
            obj(&[
                ("asset", Value::Null),
                ("gain", n(1.0)),
                ("trim", n(0.0)),
                ("fadeIn", n(0.0)),
                ("fadeOut", n(0.0)),
            ])
        }
        "null" => {
            set_p(&mut p, "anchor.x", 50.0);
            set_p(&mut p, "anchor.y", 50.0);
            set_p(&mut p, "opacity", 0.0);
            obj(&[
                ("color", values::s("#6A6A70")),
                ("w", n(100.0)),
                ("h", n(100.0)),
                ("radius", n(0.0)),
            ])
        }
        "adjustment" => {
            set_p(&mut p, "position.x", 0.0);
            set_p(&mut p, "position.y", 0.0);
            Value::Object(Map::new())
        }
        "group" => {
            set_p(&mut p, "position.x", 0.0);
            set_p(&mut p, "position.y", 0.0);
            l.insert("scaleLinked", Value::Bool(true));
            Value::Object(Map::new())
        }
        "precomp" => obj(&[("comp", Value::Null), ("w", n(w)), ("h", n(h))]),
        other => {
            return Err(CommandError::Invalid(format!(
                "Unknown layer type: {other}"
            )));
        }
    };
    let mut d = d;
    for (k, v) in content.iter() {
        values::obj(&mut d).insert(k.clone(), v.clone());
    }
    for (k, v) in properties.iter() {
        if let Some(ch) = p.get_key_mut(k).and_then(Value::as_object_mut) {
            ch.insert("v", v.clone());
        }
    }
    if matches!(layer_type, "solid" | "adjustment" | "precomp") {
        set_p(&mut p, "position.x", 0.0);
        set_p(&mut p, "position.y", 0.0);
    }
    l.insert("p", Value::Object(p));
    l.insert("d", d);
    Ok(Value::Object(l))
}

const FORBIDDEN_KEYS: [&str; 3] = ["__proto__", "prototype", "constructor"];

/// `safePatch(value, label)`.
pub(crate) fn safe_patch(m: &Map, label: &str) -> Result<(), CommandError> {
    for (k, _) in m.iter() {
        let k = k.to_string_lossy();
        if FORBIDDEN_KEYS.contains(&k.as_str()) {
            return Err(CommandError::Invalid(format!("Unsafe {label} key: {k}")));
        }
    }
    Ok(())
}

/// `groupAncestors(layer, layers)` by id over values.
pub(crate) fn ancestors<'v>(layers: &'v [Value], layer: &'v Value) -> Vec<&'v Value> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = values::text(layer, "id").into_iter().collect();
    let mut next = values::text(layer, "group").filter(|g| !g.is_empty());
    while let Some(id) = next {
        if seen.contains(&id) {
            break;
        }
        seen.push(id);
        let Some(group) = layers.iter().find(|l| {
            values::text(l, "id") == Some(id) && values::text(l, "type") == Some("group")
        }) else {
            break;
        };
        out.push(group);
        next = values::text(group, "group").filter(|g| !g.is_empty());
    }
    out
}

fn is_locked_in(layers: &[Value], layer: &Value) -> bool {
    values::flag(layer, "lock")
        || ancestors(layers, layer)
            .iter()
            .any(|g| values::flag(g, "lock"))
}

/// `PM.expandGroups(ids)`: the layers and every member of the groups among
/// them, in stack order.
pub(crate) fn expand_groups(layers: &[Value], ids: &HashSet<&str>) -> Vec<String> {
    layers
        .iter()
        .filter(|l| {
            values::text(l, "id").is_some_and(|i| ids.contains(i))
                || ancestors(layers, l)
                    .iter()
                    .any(|g| values::text(g, "id").is_some_and(|i| ids.contains(i)))
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect()
}

/// `PM.normalizeGroupStack()`: every group directly above its members.
pub(crate) fn normalize_group_stack(layers: Vec<Value>) -> Vec<Value> {
    let mut children: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        let parent = values::text(l, "group")
            .filter(|g| !g.is_empty())
            .map(str::to_owned);
        match children.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, list)) => list.push(i),
            None => children.push((parent, vec![i])),
        }
    }
    let mut seen = HashSet::new();
    let mut order = Vec::with_capacity(layers.len());
    fn visit(
        parent: Option<&str>,
        layers: &[Value],
        children: &[(Option<String>, Vec<usize>)],
        seen: &mut HashSet<String>,
        order: &mut Vec<usize>,
    ) {
        let Some((_, list)) = children.iter().find(|(p, _)| p.as_deref() == parent) else {
            return;
        };
        for &i in list {
            let id = values::text(&layers[i], "id")
                .unwrap_or_default()
                .to_owned();
            if !seen.insert(id.clone()) {
                continue;
            }
            order.push(i);
            if values::text(&layers[i], "type") == Some("group") {
                visit(Some(&id), layers, children, seen, order);
            }
        }
    }
    visit(None, &layers, &children, &mut seen, &mut order);
    let mut out: Vec<Value> = order.iter().map(|&i| layers[i].clone()).collect();
    for l in &layers {
        let id = values::text(l, "id").unwrap_or_default();
        if !seen.contains(id) {
            let mut l = l.clone();
            values::obj(&mut l).insert("group", Value::Null);
            out.push(l);
        }
    }
    out
}

/// Writes a composition's new layer list: records that changed are
/// patched, new ones inserted, missing ones deleted, then the order set.
pub(crate) fn write_layers(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    next: Vec<Value>,
) -> Result<(), CommandError> {
    let path = cx.layers_path(comp);
    cx.replace(path, Value::Array(next))
}

/// `PM.wouldCycle(L, parentId)`.
pub(crate) fn would_cycle(layers: &[Value], layer: &Value, parent_id: &str) -> bool {
    let id = values::text(layer, "id").unwrap_or_default();
    if parent_id == id {
        return true;
    }
    if ancestors(layers, layer)
        .iter()
        .any(|g| values::text(g, "id") == Some(parent_id))
    {
        return true;
    }
    let find = |pid: &str| layers.iter().find(|l| values::text(l, "id") == Some(pid));
    let mut cur = find(parent_id);
    let mut guard = 0;
    let is_group = values::text(layer, "type") == Some("group");
    while let Some(c) = cur {
        guard += 1;
        if guard > 256 {
            break;
        }
        if values::text(c, "id") == Some(id) {
            return true;
        }
        if is_group
            && ancestors(layers, c)
                .iter()
                .any(|g| values::text(g, "id") == Some(id))
        {
            return true;
        }
        cur = values::text(c, "parent")
            .filter(|p| !p.is_empty())
            .and_then(find);
    }
    false
}

/// `validMatteSource(layers, layer, id)`.
fn valid_matte_source(layers: &[Value], layer: &Value, id: Option<&str>) -> bool {
    let mut seen: Vec<String> = values::text(layer, "id")
        .map(str::to_owned)
        .into_iter()
        .collect();
    let mut cursor = id.map(str::to_owned);
    while let Some(c) = cursor.filter(|c| !c.is_empty()) {
        if seen.contains(&c) {
            return false;
        }
        seen.push(c.clone());
        let Some(source) = layers
            .iter()
            .find(|l| values::text(l, "id") == Some(c.as_str()))
        else {
            return false;
        };
        if matches!(
            values::text(source, "type"),
            Some("audio" | "null" | "adjustment" | "group")
        ) {
            return false;
        }
        cursor = values::text(source, "matteSource").map(str::to_owned);
    }
    true
}

/// A switch that may be animated (`l.blend`, `l.mblur`): keyed like
/// `set_property` with hand edits overwritten, else the plain value.
fn set_switch(
    cx: &mut Cx<'_>,
    layer_id: &str,
    field: &str,
    value: Value,
    time: f64,
) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer(layer_id)?;
    if layer.get(field).is_some_and(values::is_property) {
        let mut c = SetProperty::new(layer_id, format!("l.{field}"), value, time);
        c.preserve_hand_edits = false;
        properties::set_property(cx, &c)
    } else {
        let mut path = loc.path();
        path.push(field.into());
        cx.set(path, value)
    }
}

/// `setLayer(command)` with `policyCommand`'s lock rule.
fn set_layer(cx: &mut Cx<'_>, layer_id: &str, p: &LayerPatch) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer(layer_id)?;
    let unlock_only = cx.origin().is_human()
        && p.locked == Some(false)
        && *p
            == LayerPatch {
                locked: Some(false),
                time: p.time,
                ..LayerPatch::default()
            };
    if !unlock_only {
        cx.check_lock(&loc, layer_id)?;
    }
    let comp = loc.comp_id.clone();
    let fps = cx.fps(comp.as_deref());
    let layer_type = values::text(&layer, "type").unwrap_or_default().to_owned();
    if layer_type == "audio" {
        if matches!(
            &p.parent,
            Some(ParentChange {
                parent: Some(_),
                ..
            })
        ) {
            return Err(CommandError::Invalid(
                "Audio layers do not support parenting".into(),
            ));
        }
        if p.blend.as_deref().is_some_and(|b| b != "normal") {
            return Err(CommandError::Invalid(
                "Audio layers do not support blend modes".into(),
            ));
        }
        if p.motion_blur == Some(true) {
            return Err(CommandError::Invalid(
                "Audio layers do not support motion blur".into(),
            ));
        }
    }
    let at = |k: &str| {
        let mut path = loc.path();
        path.push(k.into());
        path
    };
    if let Some(name) = &p.name {
        let trimmed = values::js_trim(name);
        if !trimmed.is_empty() {
            cx.set(at("name"), values::s(trimmed))?;
        }
    }
    if let Some(from) = p.from {
        cx.set(at("from"), n(values::finite(from, "layer start")?))?;
    }
    if let Some(d) = p.duration {
        cx.set(
            at("dur"),
            n((1.0 / fps).max(values::finite(d, "layer duration")?)),
        )?;
    }
    for (field, v) in [
        ("on", p.visible),
        ("lock", p.locked),
        ("shy", p.shy),
        ("solo", p.solo),
    ] {
        if let Some(v) = v {
            cx.set(at(field), Value::Bool(v))?;
        }
    }
    if let Some(blend) = &p.blend {
        if !BLEND_MODES.contains(&blend.as_str()) {
            return Err(CommandError::Invalid(format!(
                "Unknown blend mode: {blend}"
            )));
        }
        set_switch(cx, layer_id, "blend", values::s(blend), p.time)?;
    }
    if let Some(mb) = p.motion_blur {
        set_switch(cx, layer_id, "mblur", Value::Bool(mb), p.time)?;
    }
    if let Some(source) = &p.matte_source {
        let layers = cx.layers(comp.as_deref());
        let (_, now) = cx.layer(layer_id)?;
        if !valid_matte_source(&layers, &now, source.as_deref()) {
            return Err(CommandError::Invalid(
                "Invalid matte source or matte cycle".into(),
            ));
        }
        cx.set(
            at("matteSource"),
            source.as_deref().map(values::s).unwrap_or(Value::Null),
        )?;
        if !now.get("matteMode").is_some_and(Value::is_truthy) {
            cx.set(at("matteMode"), property(values::s("alpha")))?;
        }
    }
    if let Some(change) = &p.parent {
        set_parent(cx, layer_id, change)?;
    }
    if let Some(color) = &p.color {
        cx.set(at("color"), values::s(color))?;
    }
    if let Some(c) = p.collapsed {
        cx.set(at("collapsed"), Value::Bool(c))?;
    }
    if let Some(three_d) = p.three_d {
        if three_d && matches!(layer_type.as_str(), "audio" | "adjustment") {
            return Err(CommandError::Invalid(
                "This layer cannot be a 3D layer".into(),
            ));
        }
        cx.set(at("threeD"), Value::Bool(three_d))?;
    }
    if let Some(c) = p.continuous_raster {
        if layer_type != "precomp" {
            return Err(CommandError::Invalid(
                "Only pre-comp layers can be continuously rasterized".into(),
            ));
        }
        cx.set(at("continuousRaster"), Value::Bool(c))?;
    }
    if let Some(space) = p.effect_space {
        cx.set(
            at("effectSpace"),
            values::s(match space {
                EffectSpace::Composition => "composition",
                EffectSpace::Source => "source",
            }),
        )?;
    }
    let (_, now) = cx.layer(layer_id)?;
    if values::text(&now, "effectSpace") == Some("source")
        && (values::flag(&now, "threeD")
            || matches!(
                layer_type.as_str(),
                "group" | "adjustment" | "audio" | "null"
            ))
    {
        return Err(CommandError::Invalid(
            "Source effects require a 2D content layer".into(),
        ));
    }
    if let Some(v) = p.scale_linked {
        cx.set(at("scaleLinked"), Value::Bool(v))?;
    }
    if let Some(v) = p.separate_dimensions {
        cx.set(at("separateDimensions"), Value::Bool(v))?;
    }
    Ok(())
}

/// The parent half of `setLayer`: checks, then `preserveParentPose` through
/// the engine's `pose::reparent` kernel, then `layer.parent`.
fn set_parent(cx: &mut Cx<'_>, layer_id: &str, change: &ParentChange) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer(layer_id)?;
    let comp = loc.comp_id.clone();
    let layers = cx.layers(comp.as_deref());
    let parent_index = match &change.parent {
        None => None,
        Some(pid) => {
            let index = layers
                .iter()
                .position(|l| values::text(l, "id") == Some(pid.as_str()))
                .filter(|_| pid != layer_id)
                .ok_or_else(|| CommandError::Invalid("Invalid parent layer".into()))?;
            if !transformable(values::text(&layers[index], "type").unwrap_or_default()) {
                return Err(CommandError::Invalid(
                    "This layer cannot be a parent".into(),
                ));
            }
            if would_cycle(&layers, &layer, pid) {
                return Err(CommandError::Invalid(
                    "Parenting would create a cycle".into(),
                ));
            }
            Some(index)
        }
    };
    let current = values::text(&layer, "parent").filter(|p| !p.is_empty());
    if current == change.parent.as_deref() {
        return Ok(());
    }
    let time = values::finite(change.time, "time")?;
    let index = loc.index;
    cx.kernel(comp.as_deref(), |edit| {
        pm_edit::pose::reparent(edit, index, parent_index, time)
    })?;
    let mut path = loc.path();
    path.push("parent".into());
    cx.set(
        path,
        change
            .parent
            .as_deref()
            .map(values::s)
            .unwrap_or(Value::Null),
    )
}

/// `addLayer(command)`.
fn add_layer(cx: &mut Cx<'_>, c: &AddLayer) -> Result<(), CommandError> {
    if !LAYER_TYPES.contains(&c.layer_type.as_str()) {
        return Err(CommandError::Invalid(format!(
            "Unknown layer type: {}",
            c.layer_type
        )));
    }
    safe_patch(&c.content, "content")?;
    safe_patch(&c.properties, "properties")?;
    if c.layer_type == "audio"
        && (c.parent.is_some()
            || c.blend.as_deref().is_some_and(|b| b != "normal")
            || c.motion_blur == Some(true))
    {
        return Err(CommandError::Invalid(
            "Audio layers do not support parenting, blend modes, or motion blur".into(),
        ));
    }
    if cx.all_layers().into_iter().any(|loc| {
        cx.layer_node(&loc)
            .ok()
            .and_then(|l| l.get("id"))
            .and_then(crate::model::str_of)
            == Some(c.id.as_str())
    }) {
        return Err(CommandError::Invalid(format!(
            "Layer id already exists: {}",
            c.id
        )));
    }
    let comp_node = cx.comp_node(c.comp.as_deref())?;
    let dim = |k: &str, d: f64| {
        comp_node
            .get(k)
            .and_then(crate::model::number_of)
            .unwrap_or(d)
    };
    let dims = (dim("w", 1920.0), dim("h", 1080.0), dim("dur", 5.0));
    let fps = cx.fps(c.comp.as_deref());
    let comp_dur = cx.duration(c.comp.as_deref());
    let mut layer = make_layer(
        &c.layer_type,
        &c.id,
        c.name.as_deref(),
        dims,
        &c.content,
        &c.properties,
    )?;
    if c.layer_type == "audio" {
        content_audio(&mut layer)?;
    }
    let from = match c.from {
        Some(f) => values::finite(f, "layer start")?,
        None => values::snap(values::finite(c.time, "time")?, fps),
    };
    let dur = match c.duration {
        Some(d) => (1.0 / fps).max(values::finite(d, "layer duration")?),
        None => 0.1f64.max(comp_dur - from),
    };
    {
        let m = values::obj(&mut layer);
        m.insert("from", n(from));
        m.insert("dur", n(dur));
        if let Some(color) = &c.color {
            m.insert("color", values::s(color));
        }
        if let Some(b) = &c.blend {
            if !BLEND_MODES.contains(&b.as_str()) {
                return Err(CommandError::Invalid(format!("Unknown blend mode: {b}")));
            }
            m.insert("blend", values::s(b));
        }
        if let Some(v) = c.motion_blur {
            m.insert("mblur", Value::Bool(v));
        }
        if let Some(v) = c.visible {
            m.insert("on", Value::Bool(v));
        }
        if let Some(v) = c.shy {
            m.insert("shy", Value::Bool(v));
        }
        if let Some(v) = c.collapsed {
            m.insert("collapsed", Value::Bool(v));
        }
    }
    let layers = cx.layers(c.comp.as_deref());
    if let Some(pid) = &c.parent {
        let parent = layers
            .iter()
            .find(|l| values::text(l, "id") == Some(pid.as_str()));
        let ok = parent.is_some_and(|p| {
            transformable(values::text(p, "type").unwrap_or_default())
                && !would_cycle(&layers, &layer, pid)
        });
        if !ok || pid == &c.id {
            return Err(CommandError::Invalid("Invalid parent layer".into()));
        }
        values::obj(&mut layer).insert("parent", values::s(pid));
    }
    let index = c.index.unwrap_or(0).min(layers.len());
    cx.insert_layer(c.comp.as_deref(), index, layer)
}

/// `setAudioContent(layer, patch)` for a new audio layer.
fn content_audio(layer: &mut Value) -> Result<(), CommandError> {
    let d = layer.get("d").cloned().unwrap_or_default();
    for (k, _) in d
        .as_object()
        .map(|m| m.iter().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let k = k.to_string_lossy();
        if !["asset", "trim", "gain", "fadeIn", "fadeOut"].contains(&k.as_str()) {
            return Err(CommandError::Invalid(format!(
                "Audio content field “{k}” is not editable"
            )));
        }
    }
    let number = |k: &str, fallback: f64, min: f64, max: f64| -> Result<Value, CommandError> {
        match d.get(k) {
            Some(v) if values::is_property(v) => Ok(v.clone()),
            Some(Value::Null) | None => Ok(n(values::clamp(fallback, min, max))),
            Some(v) => Ok(n(values::clamp(
                values::finite(super::track::js_number(v), k)?,
                min,
                max,
            ))),
        }
    };
    let asset = match d.get("asset") {
        Some(Value::String(s)) if !s.as_bytes().is_empty() => Value::String(s.clone()),
        Some(Value::Null) | None => Value::Null,
        _ => {
            return Err(CommandError::Invalid(
                "Audio source must be a media asset ID or null".into(),
            ));
        }
    };
    let nd = obj(&[
        ("asset", asset),
        ("trim", number("trim", 0.0, 0.0, f64::INFINITY)?),
        ("gain", number("gain", 1.0, 0.0, 4.0)?),
        ("fadeIn", number("fadeIn", 0.0, 0.0, f64::INFINITY)?),
        ("fadeOut", number("fadeOut", 0.0, 0.0, f64::INFINITY)?),
    ]);
    let m = values::obj(layer);
    m.insert("d", nd);
    m.insert("p", Value::Object(Map::new()));
    m.insert("fx", Value::Array(Vec::new()));
    m.insert("masks", Value::Array(Vec::new()));
    m.insert("parent", Value::Null);
    m.insert("blend", values::s("normal"));
    m.insert("mblur", Value::Bool(false));
    Ok(())
}

/// `delete_layers` with `policyCommand`'s lock filtering, then
/// `PM.removeLayers`.
fn delete_layers(cx: &mut Cx<'_>, layer_ids: &[String]) -> Result<(), CommandError> {
    let mut by_comp: Vec<(Option<String>, Vec<String>)> = Vec::new();
    for id in layer_ids {
        let Ok(loc) = cx.find(id) else { continue };
        match by_comp.iter_mut().find(|(c, _)| *c == loc.comp_id) {
            Some((_, list)) => list.push(id.clone()),
            None => by_comp.push((loc.comp_id, vec![id.clone()])),
        }
    }
    if by_comp.is_empty() {
        return Err(CommandError::Invalid("No layers to delete".into()));
    }
    for (comp, ids) in by_comp {
        let layers = cx.layers(comp.as_deref());
        let base: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let expanded = expand_groups(&layers, &base);
        let find = |id: &str| layers.iter().find(|l| values::text(l, "id") == Some(id));
        let locked: Vec<&Value> = expanded
            .iter()
            .filter_map(|id| find(id))
            .filter(|l| is_locked_in(&layers, l))
            .collect();
        let mut remove: HashSet<String> = expanded.iter().cloned().collect();
        if !locked.is_empty() {
            if ids
                .iter()
                .filter_map(|id| find(id))
                .any(|l| values::text(l, "type") == Some("group"))
            {
                return Err(CommandError::Invalid(
                    "Unlock the group and its contents before deleting".into(),
                ));
            }
            let names: Vec<String> = locked
                .iter()
                .map(|l| format!("“{}”", values::layer_name(l, "")))
                .collect();
            if locked.len() == expanded.len() {
                return Err(CommandError::Invalid(format!(
                    "All targeted layers are locked: {}",
                    names.join(", ")
                )));
            }
            for l in &locked {
                remove.remove(values::text(l, "id").unwrap_or_default());
            }
        }
        // `PM.removeLayers`: expand again, drop, unparent survivors.
        let again: HashSet<&str> = remove.iter().map(String::as_str).collect();
        let gone: HashSet<String> = expand_groups(&layers, &again).into_iter().collect();
        let next: Vec<Value> = layers
            .iter()
            .filter(|l| !values::text(l, "id").is_some_and(|i| gone.contains(i)))
            .cloned()
            .map(|mut l| {
                if values::text(&l, "parent").is_some_and(|p| gone.contains(p)) {
                    values::obj(&mut l).insert("parent", Value::Null);
                }
                l
            })
            .collect();
        write_layers(cx, comp.as_deref(), next)?;
    }
    Ok(())
}

/// `PM.cloneLayer(L)`: a deep copy with fresh key ids and a numbered name.
pub(crate) fn clone_layer(layer: &Value, id: &str, siblings: &[Value]) -> Value {
    let mut c = layer.clone();
    let name = values::text(layer, "name").unwrap_or_default();
    let base = strip_number(name, false);
    let count = siblings
        .iter()
        .filter(|l| {
            values::text(l, "name")
                .unwrap_or_default()
                .starts_with(base)
        })
        .count();
    let renamed = format!("{} {}", strip_number(name, true), count + 1);
    let m = values::obj(&mut c);
    m.insert("id", values::s(id));
    m.insert("name", values::s(&renamed));
    renew_keys(&mut c);
    c
}

/// `name.replace(/ (\d+)$/, '')` (`all_digits`) or `/ \d+$/`: the same here.
fn strip_number(name: &str, _all: bool) -> &str {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.len() < name.len() && trimmed.ends_with(' ') {
        &trimmed[..trimmed.len() - 1]
    } else {
        name
    }
}

/// Every key of every channel gets a fresh id (`renew` in `PM.cloneLayer`).
pub(crate) fn renew_keys(layer: &mut Value) {
    for (_, path) in channels::all(layer) {
        if let Some(prop) = channels::get_mut(layer, &path)
            && let Some(keys) = prop
                .as_object_mut()
                .and_then(|m| m.get_mut("kf"))
                .and_then(Value::as_array_mut)
        {
            for k in keys {
                values::obj(k).insert("i", values::s(&new_id("k")));
            }
        }
    }
}

fn duplicate(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    new_ids: &[String],
) -> Result<(), CommandError> {
    if layer_ids.len() != new_ids.len() {
        return Err(CommandError::Invalid(
            "new_ids must pair with layer_ids".into(),
        ));
    }
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    let comp = cx.find(first)?.comp_id;
    let layers = cx.layers(comp.as_deref());
    let mut selected = Vec::new();
    for id in layer_ids {
        let l = layers
            .iter()
            .find(|l| values::text(l, "id") == Some(id.as_str()))
            .ok_or_else(|| CommandError::LayerNotFound(id.clone()))?;
        if is_locked_in(&layers, l) {
            return Err(CommandError::Locked {
                layer_id: id.clone(),
                name: values::layer_name(l, id),
            });
        }
        selected.push(l.clone());
    }
    // `selectedStackLayers`: stack order.
    selected.sort_by_key(|l| {
        layers
            .iter()
            .position(|x| values::text(x, "id") == values::text(l, "id"))
    });
    let index = layers
        .iter()
        .position(|l| values::text(l, "id").is_some_and(|i| layer_ids.iter().any(|x| x == i)));
    let ids: Vec<String> = selected
        .iter()
        .map(|l| {
            let i = layer_ids
                .iter()
                .position(|x| Some(x.as_str()) == values::text(l, "id"))
                .expect("selected");
            new_ids[i].clone()
        })
        .collect();
    paste(cx, comp.as_deref(), &selected, &ids, index, None)
}

/// `pasteLayers(PM, clipboard, { atPlayhead })`.
fn paste(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    clipboard: &[Value],
    new_ids: &[String],
    index: Option<usize>,
    time: Option<f64>,
) -> Result<(), CommandError> {
    if clipboard.is_empty() {
        return Ok(());
    }
    if clipboard.len() != new_ids.len() {
        return Err(CommandError::Invalid(
            "new_ids must pair with layers".into(),
        ));
    }
    let comp = cx.comp_id(comp);
    let layers = cx.layers(comp.as_deref());
    let current: HashSet<String> = layers
        .iter()
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    for id in new_ids {
        if current.contains(id) || cx.find(id).is_ok() {
            return Err(CommandError::Invalid(format!(
                "Layer id already exists: {id}"
            )));
        }
    }
    let contents: Vec<&Value> = clipboard
        .iter()
        .filter(|l| values::text(l, "type") != Some("group"))
        .collect();
    let pool: Vec<&Value> = if contents.is_empty() {
        clipboard.iter().collect()
    } else {
        contents
    };
    let start = pool
        .iter()
        .map(|l| values::num(l, "from").unwrap_or(0.0))
        .fold(f64::INFINITY, f64::min);
    let offset = match time {
        Some(t) => values::finite(t, "time")? - start,
        None => 0.0,
    };
    let id_map: Vec<(String, String)> = clipboard
        .iter()
        .zip(new_ids)
        .map(|(s, n)| {
            (
                values::text(s, "id").unwrap_or_default().to_owned(),
                n.clone(),
            )
        })
        .collect();
    let mapped = |id: Option<&str>| -> Option<String> {
        let id = id?;
        id_map.iter().find(|(s, _)| s == id).map(|(_, n)| n.clone())
    };
    let mut pasted = Vec::with_capacity(clipboard.len());
    for (source, id) in clipboard.iter().zip(new_ids) {
        let mut clone = clone_layer(source, id, &layers);
        let from = values::num(source, "from").unwrap_or(0.0) + offset;
        let parent = values::text(source, "parent").filter(|p| !p.is_empty());
        let group = values::text(source, "group").filter(|g| !g.is_empty());
        let parent =
            mapped(parent).or_else(|| parent.filter(|p| current.contains(*p)).map(str::to_owned));
        let group =
            mapped(group).or_else(|| group.filter(|g| current.contains(*g)).map(str::to_owned));
        let matte = values::text(source, "matteSource").and_then(|m| mapped(Some(m)));
        let m = values::obj(&mut clone);
        m.insert("from", n(from));
        m.insert(
            "parent",
            parent.as_deref().map(values::s).unwrap_or(Value::Null),
        );
        m.insert(
            "group",
            group.as_deref().map(values::s).unwrap_or(Value::Null),
        );
        if let Some(matte) = matte {
            m.insert("matteSource", values::s(&matte));
        }
        // `remapLayerInputs(clone, idMap)`.
        if let Some(fx) = m.get_mut("fx").and_then(Value::as_array_mut) {
            for e in fx {
                if let Some(inputs) = e
                    .as_object_mut()
                    .and_then(|e| e.get_mut("inputs"))
                    .and_then(Value::as_object_mut)
                {
                    for (_, r) in inputs.iter_mut() {
                        if let Some(target) = mapped(values::text(r, "layerId")) {
                            values::obj(r).insert("layerId", values::s(&target));
                        }
                    }
                }
            }
        }
        pasted.push(clone);
    }
    let at = index.unwrap_or(0).min(layers.len());
    let mut next = layers;
    next.splice(at..at, pasted);
    let next = normalize_group_stack(next);
    write_layers(cx, comp.as_deref(), next)
}

/// `reorderLayer(command)`.
fn reorder(cx: &mut Cx<'_>, layer_id: &str, index: usize, time: f64) -> Result<(), CommandError> {
    let loc = cx.find(layer_id)?;
    cx.check_lock(&loc, layer_id)?;
    let comp = loc.comp_id.clone();
    let layers = cx.layers(comp.as_deref());
    if layers.is_empty() {
        return Ok(());
    }
    let from = loc.index;
    let to = index.min(layers.len() - 1);
    if from == to {
        return Ok(());
    }
    let ids: HashSet<&str> = [layer_id].into_iter().collect();
    let block_ids: HashSet<String> = expand_groups(&layers, &ids).into_iter().collect();
    let target = &layers[to];
    let target_id = values::text(target, "id").unwrap_or_default().to_owned();
    if block_ids.contains(&target_id) {
        return Ok(());
    }
    if is_locked_in(&layers, target) {
        return Err(CommandError::Invalid(
            "Unlock the destination before moving layers".into(),
        ));
    }
    let destination = if values::text(target, "type") == Some("group") {
        Some(target_id.clone())
    } else {
        values::text(target, "group")
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
    };
    let layer = &layers[from];
    let current_group = values::text(layer, "group")
        .filter(|g| !g.is_empty())
        .map(str::to_owned);
    if current_group != destination {
        let target_index = destination.as_ref().and_then(|g| {
            layers
                .iter()
                .position(|l| values::text(l, "id") == Some(g.as_str()))
        });
        move_into_group(
            cx,
            comp.as_deref(),
            &[from],
            target_index,
            destination.as_deref(),
            time,
        )?;
    }
    let layers = cx.layers(comp.as_deref());
    let block: Vec<Value> = layers
        .iter()
        .filter(|l| values::text(l, "id").is_some_and(|i| block_ids.contains(i)))
        .cloned()
        .collect();
    let mut remaining: Vec<Value> = layers
        .iter()
        .filter(|l| !values::text(l, "id").is_some_and(|i| block_ids.contains(i)))
        .cloned()
        .collect();
    let pos = remaining
        .iter()
        .position(|l| values::text(l, "id") == Some(target_id.as_str()))
        .map(|p| p + usize::from(to > from))
        .unwrap_or(0);
    remaining.splice(pos..pos, block);
    let next = normalize_group_stack(remaining);
    write_layers(cx, comp.as_deref(), next)
}

/// Sets `group` on layers (by index) keeping their world poses
/// (`PM.moveToGroup` without its checks), through `groups::move_to_group`.
pub(crate) fn move_into_group(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    roots: &[usize],
    target: Option<usize>,
    _group_id: Option<&str>,
    time: f64,
) -> Result<(), CommandError> {
    let roots = roots.to_vec();
    cx.kernel(comp, |edit| {
        pm_edit::groups::move_to_group(edit, &roots, target, time)
    })
}

/// `orderLayers(PM, mode)`.
fn restack(cx: &mut Cx<'_>, layer_ids: &[String], to: Restack) -> Result<(), CommandError> {
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    let comp = cx.find(first)?.comp_id;
    let mut layers = cx.layers(comp.as_deref());
    let selected: HashSet<&str> = layer_ids.iter().map(String::as_str).collect();
    let movable: HashSet<String> = layers
        .iter()
        .filter(|l| {
            values::text(l, "id").is_some_and(|i| selected.contains(i)) && !values::flag(l, "lock")
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    if movable.is_empty() {
        return Ok(());
    }
    let step = |layers: &mut Vec<Value>, forward: bool| -> bool {
        let mut changed = false;
        let id = |l: &Value| values::text(l, "id").unwrap_or_default().to_owned();
        if forward {
            for i in 1..layers.len() {
                if !movable.contains(&id(&layers[i])) {
                    continue;
                }
                let prev = &layers[i - 1];
                if selected.contains(id(prev).as_str()) || values::flag(prev, "lock") {
                    continue;
                }
                layers.swap(i - 1, i);
                changed = true;
            }
        } else {
            for i in (0..layers.len().saturating_sub(1)).rev() {
                if !movable.contains(&id(&layers[i])) {
                    continue;
                }
                let next = &layers[i + 1];
                if selected.contains(id(next).as_str()) || values::flag(next, "lock") {
                    continue;
                }
                layers.swap(i, i + 1);
                changed = true;
            }
        }
        changed
    };
    match to {
        Restack::Forward => {
            step(&mut layers, true);
        }
        Restack::Backward => {
            step(&mut layers, false);
        }
        Restack::Front => while step(&mut layers, true) {},
        Restack::Back => while step(&mut layers, false) {},
    }
    write_layers(cx, comp.as_deref(), layers)
}

fn patch_label(p: &LayerPatch) -> &'static str {
    let fields = [
        p.name.is_some(),
        p.from.is_some(),
        p.duration.is_some(),
        p.visible.is_some(),
        p.locked.is_some(),
        p.shy.is_some(),
        p.solo.is_some(),
        p.blend.is_some(),
        p.motion_blur.is_some(),
        p.parent.is_some(),
        p.matte_source.is_some(),
        p.color.is_some(),
        p.collapsed.is_some(),
        p.three_d.is_some(),
        p.scale_linked.is_some(),
        p.separate_dimensions.is_some(),
        p.continuous_raster.is_some(),
        p.effect_space.is_some(),
    ];
    if fields.iter().filter(|f| **f).count() != 1 {
        return "Edit Layer";
    }
    match p {
        LayerPatch { name: Some(_), .. } => "Rename Layer",
        LayerPatch { from: Some(_), .. } => "Move Layer",
        LayerPatch {
            duration: Some(_), ..
        } => "Change Duration",
        LayerPatch {
            visible: Some(true),
            ..
        } => "Show Layer",
        LayerPatch {
            visible: Some(false),
            ..
        } => "Hide Layer",
        LayerPatch {
            locked: Some(true), ..
        } => "Lock Layer",
        LayerPatch {
            locked: Some(false),
            ..
        } => "Unlock Layer",
        LayerPatch { shy: Some(_), .. } => "Toggle Shy",
        LayerPatch { solo: Some(_), .. } => "Toggle Solo",
        LayerPatch { blend: Some(_), .. } => "Change Blend Mode",
        LayerPatch {
            motion_blur: Some(_),
            ..
        } => "Toggle Motion Blur",
        LayerPatch {
            parent: Some(_), ..
        } => "Set Parent",
        LayerPatch {
            matte_source: Some(_),
            ..
        } => "Set Track Matte",
        LayerPatch { color: Some(_), .. } => "Change Label Color",
        LayerPatch {
            collapsed: Some(_), ..
        } => "Toggle Layer",
        LayerPatch {
            three_d: Some(_), ..
        } => "Toggle 3D Layer",
        LayerPatch {
            scale_linked: Some(_),
            ..
        } => "Toggle Scale Link",
        LayerPatch {
            separate_dimensions: Some(_),
            ..
        } => "Separate Dimensions",
        LayerPatch {
            continuous_raster: Some(_),
            ..
        } => "Toggle Continuous Rasterization",
        _ => "Change Effect Space",
    }
}
