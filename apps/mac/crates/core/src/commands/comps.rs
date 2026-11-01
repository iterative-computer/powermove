//! Compositions and project-level settings.
//!
//! Ports `setComposition` and `setSceneParameter` of
//! `legacy/core/editing.ts`, the work-area commands of the timeline (B, N,
//! the ruler menu), `PM.Comps` (`legacy/core/compositions.ts`: create,
//! duplicate, delete, pre-compose) and the composition settings sheet
//! (`legacy/ui/project-settings.ts`).

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// The editable fields of `set_composition` (`None` leaves a field alone).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompPatch {
    /// Trimmed; blank keeps the old name.
    pub name: Option<String>,
    /// `w`, rounded, at least 16.
    pub width: Option<f64>,
    /// `h`, rounded, at least 16.
    pub height: Option<f64>,
    /// Rounded, 1–240.
    pub fps: Option<f64>,
    /// At least 0.1 s; resets the work area to the whole composition unless
    /// `work_area` is also given.
    pub duration: Option<f64>,
    /// `bg` (`#RRGGBB`); also replaces `backgroundFill` with a solid fill.
    pub background: Option<String>,
    /// `backgroundFill` (`PM.normalizeFill`); `bg` follows its first stop.
    pub background_fill: Option<Value>,
    /// 0–2.
    pub shutter: Option<f64>,
    /// `[start, end]`, start ≥ 0, end ≤ duration, end after start.
    pub work_area: Option<(f64, f64)>,
    /// 8, 16 or 32.
    pub color_depth: Option<u32>,
    /// Master gain, 0–4.
    pub audio_gain: Option<f64>,
}

/// How Pre-compose treats the selection (`PM.Comps.precompose`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrecomposeMode {
    /// Move all attributes into the new composition.
    MoveAll,
    /// Leave all attributes (one layer: its transform and effects stay on
    /// the pre-comp layer).
    LeaveAttributes,
}

/// A new composition's settings (defaults from the open composition).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewComp {
    pub name: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub fps: Option<f64>,
    pub duration: Option<f64>,
    pub background: Option<String>,
}

/// Composition commands. `comp: None` is the open composition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompCommand {
    /// `set_composition` (`setComposition`). Label "Composition Settings".
    Set {
        comp: Option<String>,
        patch: Box<CompPatch>,
    },
    /// B / N / the ruler menu: one or both work-area edges (composition
    /// seconds), clamped to the composition and kept at least one frame
    /// apart. Label "Set Work Area".
    SetWorkArea {
        comp: Option<String>,
        start: Option<f64>,
        end: Option<f64>,
    },
    /// Reset Work Area to the entire composition. Label "Reset Work Area".
    ResetWorkArea { comp: Option<String> },
    /// `set_scene_parameter` (`setSceneParameter`): a project parameter
    /// (`params[name]`) shown by generated controls. Label "Set Parameter".
    SetSceneParameter {
        name: String,
        label: Option<String>,
        control: Option<String>,
        value: Value,
        min: Option<Value>,
        max: Option<Value>,
        options: Option<Value>,
    },
    /// New Composition (Cmd+N): an empty composition `comp_id`. Opening it is
    /// the shell's (the core does not switch compositions yet). Label "New
    /// Composition".
    Create { comp_id: String, settings: NewComp },
    /// Duplicate Composition: a copy under `new_id` with fresh layer and key
    /// ids. Label "Duplicate Composition".
    Duplicate { comp_id: String, new_id: String },
    /// Delete a composition that no pre-comp layer uses and that is not
    /// open (`This composition is used by …`). Label "Delete Composition".
    Delete { comp_id: String },
    /// Pre-compose (Cmd+Shift+C): moves the layers into a new composition
    /// `new_comp_id` and replaces them with one pre-comp layer
    /// `precomp_layer_id`, keeping every rendered pixel. Not ported (needs
    /// `PM.Comps.precompose` and the `comp_bounds` kernel).
    Precompose {
        layer_ids: Vec<String>,
        new_comp_id: String,
        precomp_layer_id: String,
        name: Option<String>,
        mode: PrecomposeMode,
        time: f64,
    },
}

impl CompCommand {
    pub fn label(&self) -> String {
        match self {
            CompCommand::Set { .. } => "Composition Settings",
            CompCommand::SetWorkArea { .. } => "Set Work Area",
            CompCommand::ResetWorkArea { .. } => "Reset Work Area",
            CompCommand::SetSceneParameter { .. } => "Set Parameter",
            CompCommand::Create { .. } => "New Composition",
            CompCommand::Duplicate { .. } => "Duplicate Composition",
            CompCommand::Delete { .. } => "Delete Composition",
            CompCommand::Precompose { .. } => "Pre-compose",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            CompCommand::Set { .. } => "set_composition",
            CompCommand::SetWorkArea { .. } => "set_work_area",
            CompCommand::ResetWorkArea { .. } => "reset_work_area",
            CompCommand::SetSceneParameter { .. } => "set_scene_parameter",
            CompCommand::Create { .. } => "new_composition",
            CompCommand::Duplicate { .. } => "duplicate_composition",
            CompCommand::Delete { .. } => "delete_composition",
            CompCommand::Precompose { .. } => "precompose",
        }))
    }
}
