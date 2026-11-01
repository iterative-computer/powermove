//! Masks and vector paths.
//!
//! Ports `PM.mkMask` (`legacy/core/model.ts`), the inspector's mask rows,
//! and the pen tool's edits (`extensions/viewer/path-editing.ts`): the
//! viewer hit-tests and maps the pointer into the path's space; the core
//! creates paths and vertices and the property commands move them.

use pm_model::{Map, Value};

use super::properties::{self, SetProperty};
use super::{Cx, values};
use crate::error::CommandError;

/// Parametric mask shapes (`PM.MASK_SHAPES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskShape {
    Rect,
    Ellipse,
}

/// The editable mask fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaskPatch {
    /// `add`, `subtract`, `intersect`, `difference` (keys `m.<id>.mode` when
    /// animated).
    pub mode: Option<String>,
    /// Keys `m.<id>.on` when animated.
    pub on: Option<bool>,
    pub inverted: Option<bool>,
    pub name: Option<String>,
    /// The playhead, for animated fields.
    pub time: f64,
}

/// Mask and pen commands. Locked layers (or locked groups above) refuse;
/// audio layers have no masks. Dragging a vertex or tangent is a
/// `PropertyCommand::Set` on its channel (`g.<pathId>.v.<vertexId>.x`,
/// `.inX`, `.outY`, … or `mp.<maskId>.v.…`), keyed at the playhead when
/// animated, as the pen tool's `write` did.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MaskCommand {
    /// A new parametric mask `mask_id` (`PM.mkMask`: centered square half
    /// the composition's shorter side, feather 24, mode `add`). Label "Add
    /// Mask".
    Add {
        layer_id: String,
        mask_id: String,
        shape: MaskShape,
    },
    /// Remove masks by id. Label "Remove Mask".
    Remove {
        layer_id: String,
        mask_ids: Vec<String>,
    },
    /// Mode, on, inverted, name. Label "Edit Mask".
    Set {
        layer_id: String,
        mask_id: String,
        patch: MaskPatch,
    },
    /// Reorder a mask. Label "Reorder Mask".
    Reorder {
        layer_id: String,
        mask_id: String,
        index: usize,
    },
    /// The pen's first click (`makeVectorPath`): a new open path `path_id`
    /// on a shape layer (`d.paths`), or on another layer a new mask
    /// `mask_id` (feather 0) holding it. Label "Draw path vertex" (with the
    /// first vertex in the same transaction).
    AddPath {
        layer_id: String,
        path_id: String,
        mask_id: Option<String>,
    },
    /// A pen click (`makeVertex`): vertex `vertex_id` at (x, y) in the
    /// path's space, no tangents, appended. Label "Draw path vertex".
    AddVertex {
        layer_id: String,
        path_id: String,
        vertex_id: String,
        x: f64,
        y: f64,
    },
    /// Cmd-click a vertex. Label "Delete path vertex".
    DeleteVertex {
        layer_id: String,
        path_id: String,
        vertex_id: String,
    },
    /// Double-click / click the first vertex: `closed` on. Label "Close
    /// path".
    ClosePath { layer_id: String, path_id: String },
}

impl MaskCommand {
    pub fn label(&self) -> String {
        match self {
            MaskCommand::Add { .. } => "Add Mask",
            MaskCommand::Remove { .. } => "Remove Mask",
            MaskCommand::Set { .. } => "Edit Mask",
            MaskCommand::Reorder { .. } => "Reorder Mask",
            MaskCommand::AddPath { .. } | MaskCommand::AddVertex { .. } => "Draw path vertex",
            MaskCommand::DeleteVertex { .. } => "Delete path vertex",
            MaskCommand::ClosePath { .. } => "Close path",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let layer_id = match self {
            MaskCommand::Add { layer_id, .. }
            | MaskCommand::Remove { layer_id, .. }
            | MaskCommand::Set { layer_id, .. }
            | MaskCommand::Reorder { layer_id, .. }
            | MaskCommand::AddPath { layer_id, .. }
            | MaskCommand::AddVertex { layer_id, .. }
            | MaskCommand::DeleteVertex { layer_id, .. }
            | MaskCommand::ClosePath { layer_id, .. } => layer_id.clone(),
        };
        let (loc, mut layer) = cx.layer_mut_checked(&layer_id)?;
        if values::text(&layer, "type") == Some("audio") {
            return Err(CommandError::Invalid(
                "Audio layers do not support masks".into(),
            ));
        }
        let comp = cx.comp_node(loc.comp_id.as_deref())?;
        let side = comp
            .get("w")
            .and_then(crate::model::number_of)
            .unwrap_or(1920.0)
            .min(
                comp.get("h")
                    .and_then(crate::model::number_of)
                    .unwrap_or(1080.0),
            );
        let not_found = || CommandError::Invalid("Mask not found".into());
        match self {
            MaskCommand::Add { mask_id, shape, .. } => {
                let m = make_mask(mask_id, *shape, side);
                masks(&mut layer).push(m);
            }
            MaskCommand::Remove { mask_ids, .. } => {
                masks(&mut layer).retain(|m| {
                    !values::text(m, "id").is_some_and(|i| mask_ids.iter().any(|x| x == i))
                });
            }
            MaskCommand::Set { mask_id, patch, .. } => {
                let i = mask_index(&layer, mask_id).ok_or_else(not_found)?;
                if let Some(mode) = &patch.mode
                    && !["add", "subtract", "intersect", "difference"].contains(&mode.as_str())
                {
                    return Err(CommandError::Invalid(format!("Unknown mask mode: {mode}")));
                }
                let mut keyed = Vec::new();
                {
                    let m = values::obj(&mut masks(&mut layer)[i]);
                    for (field, v) in [
                        ("mode", patch.mode.as_deref().map(values::s)),
                        ("on", patch.on.map(Value::Bool)),
                    ] {
                        let Some(v) = v else { continue };
                        if m.get(field).is_some_and(values::is_property) {
                            keyed.push((field, v));
                        } else {
                            m.insert(field, v);
                        }
                    }
                    if let Some(inv) = patch.inverted {
                        m.insert("inverted", Value::Bool(inv));
                    }
                    if let Some(name) = &patch.name {
                        m.insert("name", values::s(name));
                    }
                }
                cx.put_layer(&loc, layer)?;
                for (field, v) in keyed {
                    let mut c = SetProperty::new(
                        layer_id.clone(),
                        format!("m.{mask_id}.{field}"),
                        v,
                        patch.time,
                    );
                    c.preserve_hand_edits = false;
                    properties::set_property(cx, &c)?;
                }
                return Ok(());
            }
            MaskCommand::Reorder { mask_id, index, .. } => {
                let i = mask_index(&layer, mask_id).ok_or_else(not_found)?;
                let list = masks(&mut layer);
                let to = (*index).min(list.len() - 1);
                let m = list.remove(i);
                list.insert(to, m);
            }
            MaskCommand::AddPath {
                path_id, mask_id, ..
            } => {
                let path = make_vector_path(
                    path_id,
                    if values::text(&layer, "type") == Some("shape") {
                        "Path"
                    } else {
                        "Mask path"
                    },
                );
                if values::text(&layer, "type") == Some("shape") {
                    let d = values::obj(&mut layer);
                    if d.get("d").and_then(Value::as_object).is_none() {
                        d.insert("d", Value::Object(Map::new()));
                    }
                    let d = d.get_mut("d").and_then(Value::as_object_mut).expect("made");
                    if d.get("paths").and_then(Value::as_array).is_none() {
                        d.insert("paths", Value::Array(Vec::new()));
                    }
                    d.get_mut("paths")
                        .and_then(Value::as_array_mut)
                        .expect("made")
                        .push(path);
                } else {
                    let id = mask_id.clone().unwrap_or_else(|| super::new_id("K"));
                    let mut m = make_mask(&id, MaskShape::Rect, side);
                    let mm = values::obj(&mut m);
                    mm.insert("path", path);
                    if let Some(f) = mm
                        .get_mut("p")
                        .and_then(|p| p.as_object_mut())
                        .and_then(|p| p.get_mut("feather"))
                    {
                        values::obj(f).insert("v", Value::Number(0.0));
                    }
                    masks(&mut layer).push(m);
                }
            }
            MaskCommand::AddVertex {
                path_id,
                vertex_id,
                x,
                y,
                ..
            } => {
                let path = vector_path(&mut layer, path_id)
                    .ok_or_else(|| CommandError::Invalid("Path not found".into()))?;
                let mut p = Map::new();
                for (k, v) in [
                    ("x", *x),
                    ("y", *y),
                    ("inX", 0.0),
                    ("inY", 0.0),
                    ("outX", 0.0),
                    ("outY", 0.0),
                ] {
                    p.insert(
                        k,
                        values::property(Value::Number(values::finite(v, "vertex")?)),
                    );
                }
                let mut vx = Map::new();
                vx.insert("id", values::s(vertex_id));
                vx.insert("p", Value::Object(p));
                let pm = values::obj(path);
                if pm.get("vertices").and_then(Value::as_array).is_none() {
                    pm.insert("vertices", Value::Array(Vec::new()));
                }
                pm.get_mut("vertices")
                    .and_then(Value::as_array_mut)
                    .expect("made")
                    .push(Value::Object(vx));
            }
            MaskCommand::DeleteVertex {
                path_id, vertex_id, ..
            } => {
                let path = vector_path(&mut layer, path_id)
                    .ok_or_else(|| CommandError::Invalid("Path not found".into()))?;
                if let Some(vs) = path
                    .as_object_mut()
                    .and_then(|p| p.get_mut("vertices"))
                    .and_then(Value::as_array_mut)
                {
                    vs.retain(|v| values::text(v, "id") != Some(vertex_id.as_str()));
                }
            }
            MaskCommand::ClosePath { path_id, .. } => {
                let path = vector_path(&mut layer, path_id)
                    .ok_or_else(|| CommandError::Invalid("Path not found".into()))?;
                if let Some(closed) = path
                    .as_object_mut()
                    .and_then(|p| p.get_mut("p"))
                    .and_then(Value::as_object_mut)
                    .and_then(|p| p.get_mut("closed"))
                {
                    values::obj(closed).insert("v", Value::Bool(true));
                }
            }
        }
        cx.put_layer(&loc, layer)
    }
}

fn masks(layer: &mut Value) -> &mut Vec<Value> {
    let m = values::obj(layer);
    if m.get("masks").and_then(Value::as_array).is_none() {
        m.insert("masks", Value::Array(Vec::new()));
    }
    m.get_mut("masks")
        .and_then(Value::as_array_mut)
        .expect("made")
}

fn mask_index(layer: &Value, id: &str) -> Option<usize> {
    layer
        .get("masks")?
        .as_array()?
        .iter()
        .position(|m| values::text(m, "id") == Some(id))
}

/// `PM.mkMask(shape, comp)`.
fn make_mask(id: &str, shape: MaskShape, side: f64) -> Value {
    let size = values::js_round(side * 0.5);
    let mut p = Map::new();
    for (k, v) in [
        ("x", 0.0),
        ("y", 0.0),
        ("w", size),
        ("h", size),
        ("rotation", 0.0),
        ("feather", 24.0),
    ] {
        p.insert(k, values::property(Value::Number(v)));
    }
    let mut m = Map::new();
    m.insert("id", values::s(id));
    m.insert(
        "shape",
        values::s(match shape {
            MaskShape::Rect => "rect",
            MaskShape::Ellipse => "ellipse",
        }),
    );
    m.insert("mode", values::s("add"));
    m.insert("on", Value::Bool(true));
    m.insert("p", Value::Object(p));
    Value::Object(m)
}

/// `makeVectorPath(api, name)`.
fn make_vector_path(id: &str, name: &str) -> Value {
    let mut p = Map::new();
    let nums = [
        ("x", 0.0),
        ("y", 0.0),
        ("rotation", 0.0),
        ("scaleX", 100.0),
        ("scaleY", 100.0),
        ("fillOpacity", 100.0),
        ("strokeWidth", 2.0),
        ("strokeOpacity", 100.0),
        ("trimStart", 0.0),
        ("trimEnd", 100.0),
        ("trimOffset", 0.0),
        ("copies", 1.0),
        ("repeatX", 30.0),
        ("repeatY", 0.0),
        ("repeatRotation", 0.0),
    ];
    for (k, v) in nums {
        p.insert(k, values::property(Value::Number(v)));
    }
    p.insert("closed", values::property(Value::Bool(false)));
    p.insert("fill", values::property(values::s("#E8E2CF")));
    p.insert("fillEnabled", values::property(Value::Bool(true)));
    p.insert("stroke", values::property(values::s("#ffffff")));
    let mut m = Map::new();
    m.insert("id", values::s(id));
    m.insert("name", values::s(name));
    m.insert("parent", Value::Null);
    m.insert("vertices", Value::Array(Vec::new()));
    m.insert("p", Value::Object(p));
    Value::Object(m)
}

/// A vector path on the layer: `d.paths[i]` or a mask's `path`.
fn vector_path<'v>(layer: &'v mut Value, path_id: &str) -> Option<&'v mut Value> {
    let in_paths = layer
        .get("d")
        .and_then(|d| d.get("paths"))
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .position(|p| values::text(p, "id") == Some(path_id))
        });
    if let Some(i) = in_paths {
        return layer
            .as_object_mut()?
            .get_mut("d")?
            .as_object_mut()?
            .get_mut("paths")?
            .as_array_mut()?
            .get_mut(i);
    }
    let mi = layer.get("masks")?.as_array()?.iter().position(|m| {
        m.get("path")
            .and_then(|p| p.get("id"))
            .and_then(Value::as_str)
            == Some(path_id)
    })?;
    layer
        .as_object_mut()?
        .get_mut("masks")?
        .as_array_mut()?
        .get_mut(mi)?
        .as_object_mut()?
        .get_mut("path")
}
