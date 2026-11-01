//! Typed editor commands: the whole edit vocabulary of the old app.
//!
//! A [`Command`] is a *request*. [`Command::plan`] validates it against the
//! transaction's scratch tree and turns it into engine operations
//! ([`pm_model::Op`]); a command that cannot be planned changes nothing.
//! Run commands through [`DocumentStore::execute`](crate::DocumentStore::execute)
//! (one command, one undo step), [`DocumentStore::transact`](crate::DocumentStore::transact)
//! (several commands, one undo step: later commands see earlier ones), or a
//! gesture ([`DocumentStore::begin_gesture`](crate::DocumentStore::begin_gesture))
//! for drags.
//!
//! # The catalog
//!
//! Commands are grouped by domain; each domain module owns its enum, its
//! docs and its planning code:
//!
//! | Variant | Module | Covers |
//! |---|---|---|
//! | [`Command::Layer`] | [`layers`] | `set_layer`, `add_layer`, `delete_layers`, `reorder_layer`, duplicate, paste, stacking, `transform_layers` |
//! | [`Command::Property`] | [`properties`] | `set_property` (keys at the playhead when animated), `set_expression`, stopwatch, reset |
//! | [`Command::Keyframe`] | [`keyframes`] | `replace_keyframes`, `set_easing`, `simplify_keyframes`, add/toggle/delete/move/edit/paste keys, interpolation, handles |
//! | [`Command::Timing`] | [`timing`] | strip move, trims, `[`/`]`, slip, split, stagger |
//! | [`Command::Group`] | [`groups`] | `group_layers`, `ungroup_layers`, `move_to_group` |
//! | [`Command::Comp`] | [`comps`] | `set_composition`, work area, `set_scene_parameter`, new/duplicate/delete composition, pre-compose |
//! | [`Command::Effect`] | [`effects`] | `add_effect`, `remove_effect`, `set_effect`, duplicate, reset, `set_transition` |
//! | [`Command::Mask`] | [`masks`] | masks and vector paths (pen tool results) |
//! | [`Command::Text`] | [`text`] | in-place text, paragraph boxes, split text, animators and styles |
//! | [`Command::Content`] | [`content`] | `set_content` |
//! | [`Command::Marker`] | [`markers`] | `add_marker`, `remove_marker`, move/rename |
//! | [`Command::Caption`] | [`captions`] | `add_captions`, `edit_captions` |
//! | [`Command::Scene3d`] | [`scene3d`] | Studio 3D scene data |
//! | [`Command::Media`] | [`media`] | replace source, project assets and folders |
//! | [`Command::Library`] | [`library`] | `create_section`, `update_section` |
//! | [`Command::Transform`] | [`transform`] | nudge, center anchor, align, distribute |
//!
//! Every variant is typed and documented (parameters, validation, undo
//! label, the old function it ports) even before it is implemented; an
//! unported one returns [`CommandError::NotImplemented`] and is listed with
//! its status in `docs/gpui/core-api.md`.
//!
//! The four spine commands ([`Command::SetStaticOpacity`],
//! [`Command::SetLayerName`], [`Command::SetLayerVisible`],
//! [`Command::SetLayerLocked`]) stay for existing callers; new code uses the
//! domain variants.
//!
//! # Conventions
//!
//! * Layers, keyframes, effects, masks and markers are addressed by **id**.
//! * Times are composition seconds unless a field says otherwise. Commands
//!   that depend on the playhead take it as `time` (the core has no
//!   playhead; the UI owns it).
//! * Ids of things a command creates are passed in (make them with
//!   [`new_id`]) so the caller can select what it made.
//! * A channel is the old app's property key: `opacity`, `position.x`,
//!   `scale.y`, `c.<content key>`, `<effectId>.<param>`, `<effectId>.$enabled`,
//!   `m.<maskId>.<param>`, `l.blend`, `l.mblur`, `l.matteMode`, `u.<uniform>`,
//!   `x.<extension param>`, `transitionIn.p.<param>` (`PM.findProp`).
//!   A leading `properties.` or `transform.` is ignored, as before.
//!
//! # Adding a command (core owner only)
//!
//! 1. Address layers by **id**; resolve the index inside `plan` from the
//!    tree it is given. Never cache an index across revisions.
//! 2. Apply the old app's lock policy ([`Cx::layer_mut_checked`],
//!    `policyCommand`).
//! 3. Return the smallest operation list: leaf patches for values,
//!    `Op::Order`/`Insert`/`Delete` for structure ([`Cx::replace`] diffs an
//!    edited copy for you). Nothing written means "already so"; the store
//!    then publishes nothing.
//! 4. Write no inverse: the store captures each operation's inverse from the
//!    scratch tree right before applying it.

use pm_model::{Node, Op};

use crate::error::CommandError;

mod cx;
mod spine;
#[allow(dead_code)] // helpers for the commands being ported
mod values;

pub mod captions;
pub mod comps;
pub mod content;
pub mod effects;
pub mod groups;
pub mod keyframes;
pub mod layers;
pub mod library;
pub mod markers;
pub mod masks;
pub mod media;
pub mod properties;
pub mod scene3d;
pub mod text;
pub mod timing;
pub mod transform;

pub use captions::CaptionCommand;
pub use comps::CompCommand;
pub use content::ContentCommand;
pub use cx::{Cx, new_id};
pub use effects::EffectCommand;
pub use groups::GroupCommand;
pub use keyframes::KeyframeCommand;
pub use layers::LayerCommand;
pub use library::LibraryCommand;
pub use markers::MarkerCommand;
pub use masks::MaskCommand;
pub use media::MediaCommand;
pub use properties::PropertyCommand;
pub use scene3d::Scene3dCommand;
pub use spine::check_lock;
pub use text::TextCommand;
pub use timing::TimingCommand;
pub use transform::TransformCommand;

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

    /// `isGeneratedOrigin`: agents, MCP and extensions.
    pub fn is_generated(self) -> bool {
        !self.is_human()
    }
}

/// An editor command. `#[non_exhaustive]`: match with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// Spine: the *static* value of a layer's opacity (0–100). Refuses an
    /// animated channel and values outside 0–100. New code uses
    /// [`PropertyCommand::Set`], which keys animated channels.
    SetStaticOpacity {
        layer_id: String,
        value: f64,
    },
    /// Spine: renames a layer (`set_layer {name}`): trimmed; a blank name
    /// keeps the current one.
    SetLayerName {
        layer_id: String,
        name: String,
    },
    /// Spine: the eye switch (`set_layer {visible}`).
    SetLayerVisible {
        layer_id: String,
        visible: bool,
    },
    /// Spine: the lock switch (`set_layer {locked}`); humans may unlock a
    /// locked layer, agents may not.
    SetLayerLocked {
        layer_id: String,
        locked: bool,
    },

    Layer(LayerCommand),
    Property(PropertyCommand),
    Keyframe(KeyframeCommand),
    Timing(TimingCommand),
    Group(GroupCommand),
    Comp(CompCommand),
    Effect(EffectCommand),
    Mask(MaskCommand),
    Text(TextCommand),
    Content(ContentCommand),
    Marker(MarkerCommand),
    Caption(CaptionCommand),
    Scene3d(Scene3dCommand),
    Media(MediaCommand),
    Library(LibraryCommand),
    Transform(TransformCommand),
}

macro_rules! from_domain {
    ($($variant:ident($ty:ty)),* $(,)?) => {
        $(impl From<$ty> for Command {
            fn from(c: $ty) -> Command {
                Command::$variant(c)
            }
        })*
    };
}

from_domain!(
    Layer(LayerCommand),
    Property(PropertyCommand),
    Keyframe(KeyframeCommand),
    Timing(TimingCommand),
    Group(GroupCommand),
    Comp(CompCommand),
    Effect(EffectCommand),
    Mask(MaskCommand),
    Text(TextCommand),
    Content(ContentCommand),
    Marker(MarkerCommand),
    Caption(CaptionCommand),
    Scene3d(Scene3dCommand),
    Media(MediaCommand),
    Library(LibraryCommand),
    Transform(TransformCommand),
);

impl Command {
    /// The undo-menu label (“Undo Change Opacity”).
    pub fn label(&self) -> String {
        match self {
            Command::SetStaticOpacity { .. }
            | Command::SetLayerName { .. }
            | Command::SetLayerVisible { .. }
            | Command::SetLayerLocked { .. } => spine::label(self),
            Command::Layer(c) => c.label(),
            Command::Property(c) => c.label(),
            Command::Keyframe(c) => c.label(),
            Command::Timing(c) => c.label(),
            Command::Group(c) => c.label(),
            Command::Comp(c) => c.label(),
            Command::Effect(c) => c.label(),
            Command::Mask(c) => c.label(),
            Command::Text(c) => c.label(),
            Command::Content(c) => c.label(),
            Command::Marker(c) => c.label(),
            Command::Caption(c) => c.label(),
            Command::Scene3d(c) => c.label(),
            Command::Media(c) => c.label(),
            Command::Library(c) => c.label(),
            Command::Transform(c) => c.label(),
        }
    }

    /// Validates against `root` (the scratch project tree) and returns the
    /// operations; empty when the document already says so.
    pub fn plan(&self, root: &Node, origin: Origin) -> Result<Vec<Op>, CommandError> {
        let mut cx = Cx::new(root, origin);
        match self {
            Command::SetStaticOpacity { .. }
            | Command::SetLayerName { .. }
            | Command::SetLayerVisible { .. }
            | Command::SetLayerLocked { .. } => return spine::plan(self, root, origin),
            Command::Layer(c) => c.plan(&mut cx)?,
            Command::Property(c) => c.plan(&mut cx)?,
            Command::Keyframe(c) => c.plan(&mut cx)?,
            Command::Timing(c) => c.plan(&mut cx)?,
            Command::Group(c) => c.plan(&mut cx)?,
            Command::Comp(c) => c.plan(&mut cx)?,
            Command::Effect(c) => c.plan(&mut cx)?,
            Command::Mask(c) => c.plan(&mut cx)?,
            Command::Text(c) => c.plan(&mut cx)?,
            Command::Content(c) => c.plan(&mut cx)?,
            Command::Marker(c) => c.plan(&mut cx)?,
            Command::Caption(c) => c.plan(&mut cx)?,
            Command::Scene3d(c) => c.plan(&mut cx)?,
            Command::Media(c) => c.plan(&mut cx)?,
            Command::Library(c) => c.plan(&mut cx)?,
            Command::Transform(c) => c.plan(&mut cx)?,
        }
        Ok(cx.finish())
    }
}
