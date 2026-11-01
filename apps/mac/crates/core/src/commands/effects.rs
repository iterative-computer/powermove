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

use super::Cx;
use crate::error::CommandError;

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
                } => "Toggle Effect",
                EffectPatch {
                    enabled: None,
                    open: None,
                    index: Some(_),
                    inputs: None,
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
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            EffectCommand::Add(_) => "add_effect",
            EffectCommand::Remove { .. } => "remove_effect",
            EffectCommand::Set { .. } => "set_effect",
            EffectCommand::Duplicate { .. } => "duplicate_effect",
            EffectCommand::Reset { .. } => "reset_effect",
            EffectCommand::SetTransition { .. } => "set_transition",
        }))
    }
}
