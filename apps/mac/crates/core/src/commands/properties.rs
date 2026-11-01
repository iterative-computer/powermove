//! Property channels (`{ v, kf, expr }`): values, expressions, the
//! stopwatch, and reset.
//!
//! Ports `setProperty` and `setExpression` of `legacy/core/editing.ts`,
//! `PM.toggleStopwatch` of `legacy/core/anim.ts`, and the inspector's
//! channel-row actions (`extensions/inspector/ChannelRow.svelte`).

use pm_model::Value;

use super::Cx;
use super::keyframes::EaseSpec;
use crate::error::CommandError;

/// How `set_property` writes (`mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyMode {
    /// Key at `time` when the channel already has keys, else the static
    /// value (the inspector and canvas).
    #[default]
    Auto,
    /// Always key at `time` (starts animation on a static channel).
    Keyframe,
    /// Always the static value `v`, even on an animated channel (its keys
    /// keep winning; the old `mode: 'static'` behaved the same).
    Static,
}

/// `set_property`.
#[derive(Clone, Debug, PartialEq)]
pub struct SetProperty {
    pub layer_id: String,
    /// The channel key (see the module docs of [`super`]).
    pub channel: String,
    /// The new value. A numeric channel needs a finite number
    /// (`<channel> must be a finite number`); colors are `#RRGGBB[AA]`
    /// strings, switches booleans, menus strings.
    pub value: Value,
    /// The playhead (composition seconds): where a key is written.
    pub time: f64,
    pub mode: KeyMode,
    /// Ease of a key this command creates (default linear); with an
    /// explicit ease an existing key's ease is replaced too.
    pub ease: Option<EaseSpec>,
    /// Hold flag of the written key.
    pub hold: Option<bool>,
    /// `preserveHandEdits` (default true): refuse a channel with a
    /// hand-edit marker. Generated origins always preserve.
    pub preserve_hand_edits: bool,
    /// `markIntent`: record a hand-edit marker (`locked_intent[channel]`)
    /// so agents leave the value alone.
    pub mark_intent: bool,
}

impl SetProperty {
    /// An interface edit with the defaults (`mode: auto`, linear, preserve).
    pub fn new(
        layer_id: impl Into<String>,
        channel: impl Into<String>,
        value: impl Into<Value>,
        time: f64,
    ) -> SetProperty {
        SetProperty {
            layer_id: layer_id.into(),
            channel: channel.into(),
            value: value.into(),
            time,
            mode: KeyMode::Auto,
            ease: None,
            hold: None,
            preserve_hand_edits: true,
            mark_intent: false,
        }
    }
}

/// Property commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyCommand {
    /// `set_property` (`setProperty`): the static value, or a key at `time`
    /// (layer-local, snapped to a frame; an existing key within half a frame
    /// is updated). Changing a video's constant `c.speed` rescales its
    /// duration. Label "Change <Channel label>" (e.g. "Change Opacity").
    Set(SetProperty),
    /// `set_expression` (`setExpression`): `None` or empty removes it.
    /// Refuses an expression the expression engine rejects. Generated
    /// origins may not overwrite a hand-edited 3D channel. Label "Set
    /// Expression" / "Remove Expression".
    SetExpression {
        layer_id: String,
        channel: String,
        expression: Option<String>,
    },
    /// The stopwatch (`PM.toggleStopwatch`): animated → static at the value
    /// shown at `time` (keys removed); static → one key at `time` holding
    /// the static value. Label "Enable Animation" / "Remove Animation".
    ToggleStopwatch {
        layer_id: String,
        channel: String,
        time: f64,
    },
    /// Reset (channel-row menu): back to the channel's default value (keys
    /// and expression removed). Needs the channel schema defaults
    /// (`PM.CH`, effect definitions); not ported.
    Reset {
        layer_id: String,
        channel: String,
        time: f64,
    },
}

impl PropertyCommand {
    pub fn label(&self) -> String {
        match self {
            PropertyCommand::Set(c) => format!("Change {}", channel_label(&c.channel)),
            PropertyCommand::SetExpression {
                expression: Some(e),
                ..
            } if !e.is_empty() => "Set Expression".into(),
            PropertyCommand::SetExpression { .. } => "Remove Expression".into(),
            PropertyCommand::ToggleStopwatch { .. } => "Toggle Animation".into(),
            PropertyCommand::Reset { .. } => "Reset Property".into(),
        }
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            PropertyCommand::Set(_) => "set_property",
            PropertyCommand::SetExpression { .. } => "set_expression",
            PropertyCommand::ToggleStopwatch { .. } => "toggle_stopwatch",
            PropertyCommand::Reset { .. } => "reset_property",
        }))
    }
}

/// `PM.CH[key].label` for transform channels, else the key's last part.
pub fn channel_label(channel: &str) -> String {
    let key = super::values::channel_key(channel);
    let known = match key {
        "anchor.x" => "Anchor X",
        "anchor.y" => "Anchor Y",
        "anchor.z" => "Anchor Z",
        "position.x" => "Position X",
        "position.y" => "Position Y",
        "position.z" => "Position Z",
        "scale.x" => "Scale X",
        "scale.y" => "Scale Y",
        "scale.z" => "Scale Z",
        "rotation" => "Rotation",
        "rotation.x" => "Rotation X",
        "rotation.y" => "Rotation Y",
        "opacity" => "Opacity",
        "skew" => "Skew",
        "perspective" => "Perspective",
        "l.blend" => "Blend mode",
        "l.mblur" => "Motion blur",
        "l.on" => "Visibility",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let last = key.rsplit('.').next().unwrap_or(key);
    let last = if last == "$enabled" { "Enabled" } else { last };
    let mut chars = last.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "Property".into(),
    }
}
