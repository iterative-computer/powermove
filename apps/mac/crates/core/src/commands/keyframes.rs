//! Keyframes: replacing tracks, adding, toggling, deleting, moving, typed
//! edits, paste, easing, interpolation and Bézier handles.
//!
//! Ports `replaceKeyframes` and `setEasing` of `legacy/core/editing.ts`,
//! `PM.setKeyOn`/`setKeysOn`/`removeKey`/`applyEaseTo` of
//! `legacy/core/anim.ts`, the interpolation commands of
//! `core/anim/temporal-ease.ts` (`easyEase`, `setInterpolation`, `setHold`,
//! `applyUnitCurve`, `applyContinuity`), the timeline's key drags and
//! keyframe value dialog (`extensions/timeline/timeline.ts`,
//! `keyframe-value-dialog.ts`), and `legacy/core/keyframe-simplification.ts`.
//!
//! Keys are written in the native (After Effects) form the app adopts on
//! first evaluation: `{ t, v, i, inInterp, outInterp, inEase, outEase,
//! autoBezier, continuous }` for numeric tracks; non-numeric tracks (colors,
//! strings, switches) keep the legacy `{ t, v, i, eo, ei, hold }` handles, as
//! the old app did. Key times are layer-local seconds.

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// An easing preset name (`PM.Ease.PRESETS`: `linear`, `ease`, `easeIn`,
/// `easeOut`, `easeInOut`, `quadIn` … `glide`) or a unit-square cubic
/// `[x1, y1, x2, y2]` (x clamped to 0–1, y to −4–4).
#[derive(Clone, Debug, PartialEq)]
pub enum EaseSpec {
    Preset(String),
    Curve([f64; 4]),
}

impl EaseSpec {
    pub fn linear() -> EaseSpec {
        EaseSpec::Preset("linear".into())
    }
}

/// Which side of a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EaseSide {
    In,
    Out,
    #[default]
    Both,
}

/// AE's Keyframe Interpolation choices (`setInterpolation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Bezier,
    Hold,
    /// Continuous Bézier (joined handles).
    Continuous,
    /// Auto Bézier.
    Auto,
}

/// A side's speed (value units per second) and influence (percent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemporalEase {
    pub speed: f64,
    pub influence: f64,
}

/// One key of `replace_keyframes` or a paste.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeSpec {
    /// Layer-local seconds (negative becomes 0).
    pub time: f64,
    pub value: Value,
    /// Named ease (default linear); ignored when `temporal` is given.
    pub ease: Option<EaseSpec>,
    pub hold: bool,
    /// Keep this key id (native edits keep identities); unique per track and
    /// not used by another channel.
    pub id: Option<String>,
    /// Native ease fields (`inInterp`, `outEase`, …) copied as given; keeps
    /// the exact subframe time.
    pub temporal: Option<Value>,
    /// `Some(None)` removes the spring, `Some(Some(v))` sets it.
    pub spring: Option<Option<Value>>,
    /// Unit handles `[eo0, eo1, ei0, ei1]` instead of `ease`.
    pub handles: Option<[f64; 4]>,
}

impl KeyframeSpec {
    pub fn new(time: f64, value: impl Into<Value>) -> KeyframeSpec {
        KeyframeSpec {
            time,
            value: value.into(),
            ease: None,
            hold: false,
            id: None,
            temporal: None,
            spring: None,
            handles: None,
        }
    }
}

/// `replace_keyframes`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplaceKeyframes {
    pub layer_id: String,
    pub channel: String,
    pub keyframes: Vec<KeyframeSpec>,
    /// `replace` (default true): drop the existing keys first. With no keys
    /// left the channel keeps the value shown at `time`.
    pub replace: bool,
    /// `Some(None)` removes the expression, `Some(Some(e))` sets it.
    pub expression: Option<Option<String>>,
    pub preserve_hand_edits: bool,
    /// The playhead, for the value kept when the last key goes.
    pub time: f64,
}

/// What `simplify_keyframes` works on.
#[derive(Clone, Debug, PartialEq)]
pub enum SimplifyScope {
    /// Every animated property of the project.
    All,
    /// Every animated property of these layers.
    Layers(Vec<String>),
    /// These `(layer id, channel)` pairs.
    Properties(Vec<(String, String)>),
}

/// Keyframe commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum KeyframeCommand {
    /// `replace_keyframes` (`replaceKeyframes`). Label "Edit Keyframes".
    Replace(Box<ReplaceKeyframes>),
    /// `set_easing` (`setEasing`): applies `curve` to the keys (outgoing
    /// handle from `x1,y1`, incoming from `x2,y2`, hold off). Ids not found
    /// are ignored; none found: `The selected keyframes are no longer
    /// available`. At most 1000 ids. Label "Set Easing".
    SetEasing {
        key_ids: Vec<String>,
        curve: EaseSpec,
    },
    /// `simplify_keyframes` (`simplifyProperties`): removes keys the curve
    /// does not need within tolerance. Not ported.
    Simplify { scope: SimplifyScope },
    /// Add Keyframe at Playhead / the diamond on a channel without a key at
    /// `time`: a key holding the channel's current value (`value`, or the
    /// static value when `None` on an unanimated channel). An existing key
    /// within half a frame is left alone. Label "Add Keyframe".
    Add {
        layer_id: String,
        channel: String,
        time: f64,
        key_id: String,
        value: Option<Value>,
    },
    /// The diamond: removes the key at `time` (within half a frame) or adds
    /// one as [`KeyframeCommand::Add`] does. Removing the last key keeps its
    /// value. Label "Toggle Keyframe".
    Toggle {
        layer_id: String,
        channel: String,
        time: f64,
        key_id: String,
        value: Option<Value>,
    },
    /// Delete selected keys (`PM.removeKey`); a channel losing its last key
    /// keeps that key's value as its static value. Missing ids are ignored.
    /// Label "Delete Keyframes".
    Delete { key_ids: Vec<String> },
    /// Clear All Keyframes / Remove Animation: the channel becomes static at
    /// `value` (the value shown at the playhead; required when the channel
    /// has more than one key) and loses its keys. Label "Clear Keyframes".
    ClearAll {
        layer_id: String,
        channel: String,
        value: Option<Value>,
    },
    /// Key drag and Option+←/→ nudge: shifts the keys by `delta` seconds
    /// (each channel's moved keys together; a moved key landing within half
    /// a frame of a key that did not move replaces it, as in the timeline;
    /// times stay ≥ 0). Label "Move Keyframes".
    Move { key_ids: Vec<String>, delta: f64 },
    /// The keyframe value dialog: a key's time (composition seconds, snapped
    /// to a frame) and/or value. A time colliding with a neighbouring key is
    /// refused (`Another keyframe already exists at that time`). Label "Edit
    /// Keyframe".
    Edit {
        key_id: String,
        time: Option<f64>,
        value: Option<Value>,
    },
    /// Paste Keyframes Here: `keys` (times relative to the first copied
    /// key) placed from `time` (composition seconds) on the channel, fresh
    /// ids from `new_ids` (same length). Label "Paste Keyframes".
    Paste {
        layer_id: String,
        channel: String,
        time: f64,
        keys: Vec<KeyframeSpec>,
        new_ids: Vec<String>,
    },
    /// F9 / Shift+F9 / Cmd+Shift+F9 (`easyEase`): Bézier at 33.3% influence,
    /// flat tangent, on `side`. Label "Easy Ease".
    EasyEase {
        key_ids: Vec<String>,
        side: EaseSide,
    },
    /// Keyframe Interpolation (`setInterpolation`). Label "Change
    /// Interpolation".
    SetInterpolation {
        key_ids: Vec<String>,
        side: EaseSide,
        interpolation: Interpolation,
    },
    /// Toggle Hold Keyframe (`setHold`). Label "Toggle Hold Keyframe".
    SetHold { key_ids: Vec<String>, hold: bool },
    /// A Bézier handle (graph editor drag, easing panel numeric entry): the
    /// side's speed and influence. A continuous key mirrors the speed to its
    /// other side unless `split` (Option-drag) breaks it. Label "Adjust
    /// Handle".
    SetHandle {
        key_id: String,
        side: EaseSide,
        ease: TemporalEase,
        split: bool,
    },
    /// Spring easing of the outgoing segment (`spring`: `{ stiffness,
    /// damping, mass, ... }`; `None` removes). Label "Set Spring".
    SetSpring {
        key_ids: Vec<String>,
        spring: Option<Value>,
    },
    /// The graph editor's transform box: scale key times about
    /// `time_pivot` and values about `value_pivot`. Not ported.
    Transform {
        key_ids: Vec<String>,
        time_pivot: f64,
        time_scale: f64,
        value_pivot: f64,
        value_scale: f64,
    },
}

impl KeyframeCommand {
    pub fn label(&self) -> String {
        match self {
            KeyframeCommand::Replace(_) => "Edit Keyframes",
            KeyframeCommand::SetEasing { .. } => "Set Easing",
            KeyframeCommand::Simplify { .. } => "Simplify Keyframes",
            KeyframeCommand::Add { .. } => "Add Keyframe",
            KeyframeCommand::Toggle { .. } => "Toggle Keyframe",
            KeyframeCommand::Delete { .. } => "Delete Keyframes",
            KeyframeCommand::ClearAll { .. } => "Clear Keyframes",
            KeyframeCommand::Move { .. } => "Move Keyframes",
            KeyframeCommand::Edit { .. } => "Edit Keyframe",
            KeyframeCommand::Paste { .. } => "Paste Keyframes",
            KeyframeCommand::EasyEase { .. } => "Easy Ease",
            KeyframeCommand::SetInterpolation { .. } => "Change Interpolation",
            KeyframeCommand::SetHold { .. } => "Toggle Hold Keyframe",
            KeyframeCommand::SetHandle { .. } => "Adjust Handle",
            KeyframeCommand::SetSpring { .. } => "Set Spring",
            KeyframeCommand::Transform { .. } => "Transform Keyframes",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            KeyframeCommand::Replace(_) => "replace_keyframes",
            KeyframeCommand::SetEasing { .. } => "set_easing",
            KeyframeCommand::Simplify { .. } => "simplify_keyframes",
            KeyframeCommand::Add { .. } => "add_keyframe",
            KeyframeCommand::Toggle { .. } => "toggle_keyframe",
            KeyframeCommand::Delete { .. } => "delete_keyframes",
            KeyframeCommand::ClearAll { .. } => "clear_keyframes",
            KeyframeCommand::Move { .. } => "move_keyframes",
            KeyframeCommand::Edit { .. } => "edit_keyframe",
            KeyframeCommand::Paste { .. } => "paste_keyframes",
            KeyframeCommand::EasyEase { .. } => "easy_ease",
            KeyframeCommand::SetInterpolation { .. } => "set_interpolation",
            KeyframeCommand::SetHold { .. } => "set_hold",
            KeyframeCommand::SetHandle { .. } => "set_handle",
            KeyframeCommand::SetSpring { .. } => "set_spring",
            KeyframeCommand::Transform { .. } => "transform_keyframes",
        }))
    }
}
