//! Typed editor commands.
//!
//! A [`Command`] is a *request*. [`Command::plan`] validates it against the
//! transaction's scratch tree and turns it into engine operations
//! ([`pm_model::Op`]); a command that cannot be planned changes nothing.
//! Run commands through [`DocumentStore::execute`](crate::DocumentStore::execute)
//! (one command, one undo step), [`DocumentStore::transact`](crate::DocumentStore::transact)
//! (several commands, one undo step), or a gesture
//! ([`DocumentStore::begin_gesture`](crate::DocumentStore::begin_gesture)).
//!
//! # Adding a command (core owner only)
//!
//! 1. Address layers by **id**; resolve the index inside `plan` from the
//!    tree it is given. Never cache an index across revisions.
//! 2. Apply the old app's lock policy ([`check_lock`], `policyCommand`).
//! 3. Return the smallest `Vec<Op>`: leaf [`Patch`]es for values,
//!    `Op::Order`/`Insert`/`Delete` for structure. An empty list means
//!    "already so"; the store then publishes nothing.
//! 4. Write no inverse: the store captures each operation's inverse from the
//!    scratch tree right before applying it.
//!
//! # Ported from the old app
//!
//! `set_property` (static path) and `set_layer` of
//! `main:apps/desktop/src/renderer/src/legacy/core/editing.ts`, with
//! `policyCommand`'s lock and origin rules. Differences are listed on each
//! variant.

use pm_model::patch::{Patch, PathPart};
use pm_model::{JsString, Node, NodeKind, Op, Value};

use crate::error::CommandError;
use crate::model::{self, LayerLoc, str_of};

/// Who is asking (`ORIGIN_TRUST` in `editing.ts`). Human origins may unlock a
/// locked layer with a bare unlock and overwrite hand-edit markers;
/// generated ones may not, so an agent's `[unlock, edit]` batch cannot
/// defeat a lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Inspector, timeline, canvas, menus, command palette, library, import.
    #[default]
    Interface,
    /// The in-app agent.
    Agent,
    /// The external MCP server.
    Mcp,
    /// Extension scripts and generated panels.
    Extension,
}

impl Origin {
    /// A trusted human gesture.
    pub fn is_human(self) -> bool {
        matches!(self, Origin::Interface)
    }
}

/// An editor command. `#[non_exhaustive]`: match with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// The *static* value of a layer's opacity
    /// (`comps/<comp>/layers/<index>/p/opacity/v`, percent).
    ///
    /// Boundary doc section 6 step 6: validates existence, lock state, a
    /// finite value in 0–100, and the absence of keyframes and expression.
    ///
    /// Differences from `set_property`: an animated channel is refused
    /// instead of keyed at the playhead (the keyframe lane owns that); a
    /// value outside 0–100 is refused (the old command accepted it and the
    /// inspector clamped); a hand-edit marker (`locked_intent.opacity`)
    /// blocks every non-[`Origin::Interface`] origin (the old per-command
    /// `preserveHandEdits` flag is not exposed yet); `markIntent` is not
    /// written.
    SetStaticOpacity { layer_id: String, value: f64 },
    /// Renames a layer (`set_layer {name}`): trimmed; a blank name keeps the
    /// current one, as in the old app.
    SetLayerName { layer_id: String, name: String },
    /// The eye switch (`set_layer {visible}`): writes `on` as a plain
    /// boolean, replacing a channel if `on` was one, as the old app did.
    SetLayerVisible { layer_id: String, visible: bool },
    /// The lock switch (`set_layer {locked}`). A bare unlock is allowed on
    /// a locked layer for human origins only (`unlockOnly`).
    SetLayerLocked { layer_id: String, locked: bool },
}

impl Command {
    /// The undo-menu label (“Undo Change Opacity”).
    pub fn label(&self) -> String {
        match self {
            Command::SetStaticOpacity { .. } => "Change Opacity",
            Command::SetLayerName { .. } => "Rename Layer",
            Command::SetLayerVisible { visible: true, .. } => "Show Layer",
            Command::SetLayerVisible { visible: false, .. } => "Hide Layer",
            Command::SetLayerLocked { locked: true, .. } => "Lock Layer",
            Command::SetLayerLocked { locked: false, .. } => "Unlock Layer",
        }
        .to_owned()
    }

    /// The layer the command targets.
    pub fn layer_id(&self) -> &str {
        match self {
            Command::SetStaticOpacity { layer_id, .. }
            | Command::SetLayerName { layer_id, .. }
            | Command::SetLayerVisible { layer_id, .. }
            | Command::SetLayerLocked { layer_id, .. } => layer_id,
        }
    }

    /// Validates against `root` (the scratch project tree) and returns the
    /// operations; empty when the document already says so.
    pub fn plan(&self, root: &Node, origin: Origin) -> Result<Vec<Op>, CommandError> {
        let id = self.layer_id();
        let loc = model::find_layer(root, id)
            .ok_or_else(|| CommandError::LayerNotFound(id.to_owned()))?;
        let layer = model::layer_at(root, &loc)
            .ok_or_else(|| CommandError::LayerNotFound(id.to_owned()))?;
        let unlock_only =
            origin.is_human() && matches!(self, Command::SetLayerLocked { locked: false, .. });
        if !unlock_only {
            check_lock(root, &loc, layer, id)?;
        }
        let at = |tail: &[&str]| -> Vec<PathPart> {
            let mut path = loc.path();
            path.extend(tail.iter().map(|k| PathPart::from(*k)));
            path
        };
        let name = || layer_name(layer, id);
        match self {
            Command::SetStaticOpacity { value, .. } => {
                let channel = layer
                    .get("p")
                    .and_then(|p| p.get("opacity"))
                    .filter(|c| matches!(c.kind(), NodeKind::Object(_)))
                    .ok_or_else(|| CommandError::PropertyNotFound {
                        layer_id: id.to_owned(),
                        name: name(),
                        channel: "opacity",
                    })?;
                if !origin.is_human() && has_intent(layer, "opacity") {
                    return Err(CommandError::PreservedHandEdit {
                        layer_id: id.to_owned(),
                        channel: "opacity",
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
                if channel.get("v").and_then(model::number_of).map(f64::to_bits)
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
                Ok(vec![set(at(&["name"]), Value::String(JsString::from(trimmed)))])
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
        }
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
fn reject_animation(channel: &Node, layer_id: &str, name: &'static str) -> Result<(), CommandError> {
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
            channel: name,
        });
    }
    Ok(())
}

/// The old lock rule: the layer, or any group above it, being locked refuses
/// the edit (`lockedMessage`).
pub fn check_lock(root: &Node, loc: &LayerLoc, layer: &Node, layer_id: &str) -> Result<(), CommandError> {
    if model::is_locked(root, loc) {
        Err(CommandError::Locked {
            layer_id: layer_id.to_owned(),
            name: layer_name(layer, layer_id),
        })
    } else {
        Ok(())
    }
}
