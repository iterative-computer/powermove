//! Layer records: fields and switches, creation, deletion, duplication,
//! paste, and stack order.
//!
//! Ports `setLayer`, `addLayer`, `deleteLayers`, `reorderLayer` and the
//! `delete_layers` lock policy of `legacy/core/editing.ts`, `PM.mkLayer`,
//! `PM.cloneLayer` and `PM.removeLayers` of `legacy/core/model.ts`, and the
//! stacking shortcuts of `ui/shortcuts.ts`.

use pm_model::{Map, Value};

use super::Cx;
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
    /// Cmd+D / Edit › Duplicate (`PM.cloneLayer`): each layer is copied
    /// above itself with fresh layer and key ids and a numbered name.
    /// `new_ids` pairs with `layer_ids` (same length). Label "Duplicate".
    Duplicate {
        layer_ids: Vec<String>,
        new_ids: Vec<String>,
    },
    /// Paste layers from the clipboard (layer records as the clipboard
    /// holds them): fresh key ids, ids from `new_ids` (same length), parents
    /// and groups kept only when they point inside the pasted set or at a
    /// layer of the target composition. Inserted at `index` (default: the
    /// top). With `time`, the pasted block is moved so its first layer starts
    /// there. Label "Paste".
    Paste {
        comp: Option<String>,
        layers: Vec<Value>,
        new_ids: Vec<String>,
        index: Option<usize>,
        time: Option<f64>,
    },
    /// `reorder_layer` (`reorderLayer`): moves the layer (with a group's
    /// members) to `index`, entering or leaving the destination's group.
    /// `Unlock the destination before moving layers`. Label "Reorder Layer".
    Reorder { layer_id: String, index: usize },
    /// Bring Forward / Send Backward / Bring to Front / Send to Back of the
    /// selected layers, keeping their relative order. Label "Arrange Layers".
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
            LayerCommand::Restack { .. } => "Arrange Layers",
            LayerCommand::TransformLayers { .. } => "Transform Layers",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            LayerCommand::Set { .. } => "set_layer",
            LayerCommand::Add(_) => "add_layer",
            LayerCommand::Delete { .. } => "delete_layers",
            LayerCommand::Duplicate { .. } => "duplicate_layers",
            LayerCommand::Paste { .. } => "paste_layers",
            LayerCommand::Reorder { .. } => "reorder_layer",
            LayerCommand::Restack { .. } => "restack_layers",
            LayerCommand::TransformLayers { .. } => "transform_layers",
        }))
    }
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
