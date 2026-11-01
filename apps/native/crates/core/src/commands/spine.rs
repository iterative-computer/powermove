//! The four spine commands (the first lane), kept for existing callers:
//! [`Command::SetStaticOpacity`], [`Command::SetLayerName`],
//! [`Command::SetLayerVisible`], [`Command::SetLayerLocked`]. New code uses
//! the domain commands.
//!
//! Ported from `set_property` (static path) and `set_layer` of
//! `main:apps/desktop/src/renderer/src/legacy/core/editing.ts`, with
//! `policyCommand`'s lock and origin rules.

use pm_model::patch::{Patch, PathPart};
use pm_model::{JsString, Node, NodeKind, Op, Value};

use super::{Command, Origin};
use crate::error::CommandError;
use crate::model::{self, LayerLoc, str_of};

pub(super) fn label(command: &Command) -> String {
    let this = command;
    match this {
        Command::SetStaticOpacity { .. } => "Change Opacity",
        Command::SetLayerName { .. } => "Rename Layer",
        Command::SetLayerVisible { visible: true, .. } => "Show Layer",
        Command::SetLayerVisible { visible: false, .. } => "Hide Layer",
        Command::SetLayerLocked { locked: true, .. } => "Lock Layer",
        Command::SetLayerLocked { locked: false, .. } => "Unlock Layer",
        _ => "Edit",
    }
    .to_owned()
}

pub(super) fn layer_id(command: &Command) -> &str {
    match command {
        Command::SetStaticOpacity { layer_id, .. }
        | Command::SetLayerName { layer_id, .. }
        | Command::SetLayerVisible { layer_id, .. }
        | Command::SetLayerLocked { layer_id, .. } => layer_id,
        _ => "",
    }
}

pub(super) fn plan(this: &Command, root: &Node, origin: Origin) -> Result<Vec<Op>, CommandError> {
    let id = layer_id(this);
    let loc =
        model::find_layer(root, id).ok_or_else(|| CommandError::LayerNotFound(id.to_owned()))?;
    let layer =
        model::layer_at(root, &loc).ok_or_else(|| CommandError::LayerNotFound(id.to_owned()))?;
    let unlock_only =
        origin.is_human() && matches!(this, Command::SetLayerLocked { locked: false, .. });
    if !unlock_only {
        check_lock(root, &loc, layer, id)?;
    }
    let at = |tail: &[&str]| -> Vec<PathPart> {
        let mut path = loc.path();
        path.extend(tail.iter().map(|k| PathPart::from(*k)));
        path
    };
    let name = || layer_name(layer, id);
    match this {
        Command::SetStaticOpacity { value, .. } => {
            let channel = layer
                .get("p")
                .and_then(|p| p.get("opacity"))
                .filter(|c| matches!(c.kind(), NodeKind::Object(_)))
                .ok_or_else(|| CommandError::PropertyNotFound {
                    layer_id: id.to_owned(),
                    name: name(),
                    channel: "opacity".into(),
                })?;
            if !origin.is_human() && has_intent(layer, "opacity") {
                return Err(CommandError::PreservedHandEdit {
                    layer_id: id.to_owned(),
                    channel: "opacity".into(),
                });
            }
            reject_animation(channel, id, "opacity")?;
            if !value.is_finite() {
                return Err(CommandError::InvalidValue {
                    what: "opacity",
                    reason: "opacity must be a finite number".into(),
                });
            }
            if !(0.0..=100.0).contains(value) {
                return Err(CommandError::InvalidValue {
                    what: "opacity",
                    reason: format!("{value} is outside 0–100"),
                });
            }
            if channel
                .get("v")
                .and_then(model::number_of)
                .map(f64::to_bits)
                == Some(value.to_bits())
            {
                return Ok(Vec::new());
            }
            Ok(vec![set(at(&["p", "opacity", "v"]), Value::Number(*value))])
        }
        Command::SetLayerName { name: new, .. } => {
            let trimmed = js_trim(new);
            if trimmed.is_empty() || layer.get("name").and_then(str_of) == Some(trimmed) {
                return Ok(Vec::new());
            }
            Ok(vec![set(
                at(&["name"]),
                Value::String(JsString::from(trimmed)),
            )])
        }
        Command::SetLayerVisible { visible, .. } => {
            if layer.get("on").and_then(model::bool_of) == Some(*visible) {
                return Ok(Vec::new());
            }
            Ok(vec![set(at(&["on"]), Value::Bool(*visible))])
        }
        Command::SetLayerLocked { locked, .. } => {
            if layer.get("lock").and_then(model::bool_of) == Some(*locked) {
                return Ok(Vec::new());
            }
            Ok(vec![set(at(&["lock"]), Value::Bool(*locked))])
        }
        _ => Ok(Vec::new()),
    }
}

fn set(path: Vec<PathPart>, value: Value) -> Op {
    Op::Leaf(Patch::set(path, value))
}

/// `String.prototype.trim` (JavaScript's whitespace set, which includes the
/// BOM and line separators beyond Rust's `char::is_whitespace`).
fn js_trim(s: &str) -> &str {
    let ws = |c: char| c.is_whitespace() || c == '\u{feff}';
    s.trim_matches(ws)
}

fn layer_name(layer: &Node, id: &str) -> String {
    layer
        .get("name")
        .and_then(model::string_of)
        .map(JsString::to_string_lossy)
        .unwrap_or_else(|| id.to_owned())
}

/// `lockedIntent(layer, channel)`: a marker on the channel or its root.
fn has_intent(layer: &Node, channel: &str) -> bool {
    let Some(intent) = layer.get("locked_intent") else {
        return false;
    };
    let root = channel.split('.').next().unwrap_or(channel);
    let truthy = |n: Option<&Node>| {
        n.is_some_and(|n| match n.kind() {
            NodeKind::Null => false,
            NodeKind::Bool(b) => *b,
            NodeKind::Number(x) => *x != 0.0 && !x.is_nan(),
            NodeKind::String(s) => !s.as_bytes().is_empty(),
            _ => true,
        })
    };
    truthy(intent.get(channel)) || truthy(intent.get(root))
}

/// A channel with keyframes or a non-empty expression is not static.
fn reject_animation(
    channel: &Node,
    layer_id: &str,
    name: &'static str,
) -> Result<(), CommandError> {
    let has_keys = matches!(
        channel.get("kf").map(Node::kind),
        Some(NodeKind::Array(items)) if !items.is_empty()
    );
    let has_expr = matches!(
        channel.get("expr").map(Node::kind),
        Some(NodeKind::String(s)) if !s.as_bytes().is_empty()
    );
    if has_keys || has_expr {
        return Err(CommandError::Animated {
            layer_id: layer_id.to_owned(),
            channel: name.to_owned(),
        });
    }
    Ok(())
}

/// The old lock rule: the layer, or any group above it, being locked refuses
/// the edit (`lockedMessage`).
pub fn check_lock(
    root: &Node,
    loc: &LayerLoc,
    layer: &Node,
    layer_id: &str,
) -> Result<(), CommandError> {
    if model::is_locked(root, loc) {
        Err(CommandError::Locked {
            layer_id: layer_id.to_owned(),
            name: layer_name(layer, layer_id),
        })
    } else {
        Ok(())
    }
}
