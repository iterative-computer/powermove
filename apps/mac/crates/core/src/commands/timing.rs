//! Layer timing: strip moves, trims, `[`/`]`, slip, split and stagger.
//!
//! Ports the timeline's strip gestures (`extensions/timeline/timeline.ts`),
//! its split command (`extensions/timeline/index.ts`: Cmd+Shift+D / Cmd+B),
//! the layer-timing shortcuts of `extensions/keymap-default/keymap.ts`, the
//! quick stagger (Cmd+Option drag), and the agent's video edits
//! (`legacy/assistant/video-editing.ts`: split, trim, slip).
//!
//! Key times are layer-local, so moving a layer moves its animation with it.
//! Group strips act on their members; caption cues stay put in composition
//! time. There is no ripple edit in the old app (the parity inventory found
//! none), so none is offered.

use super::Cx;
use crate::error::CommandError;

/// Timing commands. Every variant refuses locked layers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TimingCommand {
    /// Strip drag: `from += delta` for each layer (and a group's members).
    /// Label "Move Layers".
    Move { layer_ids: Vec<String>, delta: f64 },
    /// Drag the In edge / Option+`[`: the layer starts at `time` and keeps
    /// its end; media keeps showing the same source frames (`d.trim` moves
    /// with the edge; keys stay at their composition times). At least one
    /// frame remains. Label "Trim Layer".
    TrimIn { layer_ids: Vec<String>, time: f64 },
    /// Drag the Out edge / Option+`]`: the layer ends at `time`. At least
    /// one frame remains. Label "Trim Layer".
    TrimOut { layer_ids: Vec<String>, time: f64 },
    /// `[`: move the layer so its In point is at `time` (Option+Home: the
    /// composition start). Label "Move Layer In".
    MoveInTo { layer_ids: Vec<String>, time: f64 },
    /// `]`: move the layer so its Out point is at `time` (Option+End: the
    /// composition end). Label "Move Layer Out".
    MoveOutTo { layer_ids: Vec<String>, time: f64 },
    /// Slip: change which source frames a media or pre-comp layer shows
    /// (`d.trim -= delta`), keeping the strip and its animation where they
    /// are; refuses to expose frames before the source start. Label "Slip
    /// Layer".
    Slip { layer_ids: Vec<String>, delta: f64 },
    /// Split at `time` (Cmd+Shift+D): each layer that spans `time` becomes
    /// two; the right part gets the id from `new_ids` (same length; the
    /// caller selects those), the source time is preserved, keys are split
    /// between the halves, caption cues stay in place. Label "Split Layer".
    Split {
        layer_ids: Vec<String>,
        time: f64,
        new_ids: Vec<String>,
    },
    /// Quick stagger (Cmd+Option drag): the i-th layer (in the given order)
    /// starts `i * step` after the first, at subframe precision. Label
    /// "Stagger Layers".
    Stagger { layer_ids: Vec<String>, step: f64 },
}

impl TimingCommand {
    pub fn label(&self) -> String {
        match self {
            TimingCommand::Move { .. } => "Move Layers",
            TimingCommand::TrimIn { .. } | TimingCommand::TrimOut { .. } => "Trim Layer",
            TimingCommand::MoveInTo { .. } => "Move Layer In",
            TimingCommand::MoveOutTo { .. } => "Move Layer Out",
            TimingCommand::Slip { .. } => "Slip Layer",
            TimingCommand::Split { .. } => "Split Layer",
            TimingCommand::Stagger { .. } => "Stagger Layers",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            TimingCommand::Move { .. } => "move_layers",
            TimingCommand::TrimIn { .. } => "trim_in",
            TimingCommand::TrimOut { .. } => "trim_out",
            TimingCommand::MoveInTo { .. } => "move_in_to",
            TimingCommand::MoveOutTo { .. } => "move_out_to",
            TimingCommand::Slip { .. } => "slip",
            TimingCommand::Split { .. } => "split",
            TimingCommand::Stagger { .. } => "stagger",
        }))
    }
}
