//! Text layers: in-place editing, paragraph boxes, split text, animators
//! and style ranges.
//!
//! Ports the in-place editor's commit (`extensions/viewer/text-editor.ts`),
//! the paragraph resize (`viewer.ts`: width/height without rewriting Size or
//! Scale), `legacy/core/split-text.ts` (with the engine's `split_text`
//! kernel), and the structured text channels (`ta.` animators, `ts.` style
//! ranges) of `legacy/core/text-animation.ts`.

use pm_model::{Map, Value};

use super::content::ContentCommand;
use super::properties::{self, SetProperty};
use super::{Cx, values};
use crate::error::CommandError;

/// How Split Text divides a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    Characters,
    Words,
    Lines,
}

/// Text commands. Locked layers refuse; non-text layers refuse
/// (`Not a text layer`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextCommand {
    /// Commit an in-place edit: the layer's `d.text` (keyed at `time` when
    /// `c.text` is animated). Label "Edit Text".
    SetText {
        layer_id: String,
        text: String,
        time: f64,
    },
    /// Paragraph text box (`boxWidth`/`boxHeight` channels, 0 = automatic),
    /// keyed at `time` when animated; Size and Scale are not touched. Label
    /// "Resize Text Box".
    SetBox {
        layer_id: String,
        width: Option<f64>,
        height: Option<f64>,
        time: f64,
    },
    /// Split Text: one layer per character, word or line, in place of the
    /// original, each keeping its glyphs' position. Not ported (needs the
    /// text layout and `pm_edit::split_text`).
    Split {
        layer_id: String,
        mode: SplitMode,
        time: f64,
    },
    /// A text animator (`ta.<id>`) with its selector and properties. Not
    /// ported.
    AddAnimator {
        layer_id: String,
        animator_id: String,
        animator: Value,
    },
    /// Remove a text animator. Not ported.
    RemoveAnimator {
        layer_id: String,
        animator_id: String,
    },
    /// A style range (`ts.<id>`: start, end, and overrides). Not ported.
    SetStyleRange {
        layer_id: String,
        style_id: String,
        style: Option<Value>,
    },
}

impl TextCommand {
    pub fn label(&self) -> String {
        match self {
            TextCommand::SetText { .. } => "Edit Text",
            TextCommand::SetBox { .. } => "Resize Text Box",
            TextCommand::Split { .. } => "Split Text",
            TextCommand::AddAnimator { .. } => "Add Text Animator",
            TextCommand::RemoveAnimator { .. } => "Remove Text Animator",
            TextCommand::SetStyleRange { .. } => "Style Text",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let text_layer = |cx: &Cx<'_>, id: &str| -> Result<(), CommandError> {
            let (_, l) = cx.layer_mut_checked(id)?;
            if values::text(&l, "type") != Some("text") {
                return Err(CommandError::Invalid("Not a text layer".into()));
            }
            Ok(())
        };
        match self {
            TextCommand::SetText {
                layer_id,
                text,
                time,
            } => {
                text_layer(cx, layer_id)?;
                let mut patch = Map::new();
                patch.insert("text", values::s(text));
                ContentCommand::Set {
                    layer_id: layer_id.clone(),
                    patch,
                    time: *time,
                }
                .plan(cx)
            }
            TextCommand::SetBox {
                layer_id,
                width,
                height,
                time,
            } => {
                text_layer(cx, layer_id)?;
                for (key, v) in [("boxWidth", width), ("boxHeight", height)] {
                    let Some(v) = v else { continue };
                    let v = values::finite(*v, key)?.max(0.0);
                    let mut c = SetProperty::new(layer_id.clone(), format!("c.{key}"), v, *time);
                    c.preserve_hand_edits = false;
                    properties::set_property(cx, &c)?;
                }
                Ok(())
            }
            TextCommand::Split { .. } => Err(CommandError::NotImplemented("split_text")),
            TextCommand::AddAnimator { .. } => {
                Err(CommandError::NotImplemented("add_text_animator"))
            }
            TextCommand::RemoveAnimator { .. } => {
                Err(CommandError::NotImplemented("remove_text_animator"))
            }
            TextCommand::SetStyleRange { .. } => {
                Err(CommandError::NotImplemented("set_text_style"))
            }
        }
    }
}
