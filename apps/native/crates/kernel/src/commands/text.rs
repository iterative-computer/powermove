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
use super::{Cx, groups, layers, values};
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
    /// Split Text (`splitTextLayers`): the source's layout at `time` becomes
    /// one layer per character, word or line, grouped with the hidden source
    /// as `group_ids[i]`, each piece keeping its glyphs' position. One Undo.
    /// A source without pieces is left alone. Needs the host's fonts.
    Split {
        layer_ids: Vec<String>,
        group_ids: Vec<String>,
        mode: SplitMode,
        time: f64,
    },
    /// Append a text animator to `d.animators` (`ta.<id>` channels).
    AddAnimator {
        layer_id: String,
        animator_id: String,
        animator: Value,
    },
    /// Remove a text animator (`Text animator not found: x`).
    RemoveAnimator {
        layer_id: String,
        animator_id: String,
    },
    /// Add, replace or (`None`) remove a style range in `d.styles`
    /// (`ts.<id>`: start, end, and overrides).
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
            TextCommand::Split {
                layer_ids,
                group_ids,
                mode,
                time,
            } => {
                for id in layer_ids {
                    text_layer(cx, id)?;
                }
                split(cx, layer_ids, group_ids, *mode, *time)
            }
            TextCommand::AddAnimator {
                layer_id,
                animator_id,
                animator,
            } => {
                text_layer(cx, layer_id)?;
                if animator.as_object().is_none() {
                    return Err(CommandError::Invalid(
                        "A text animator must be an object".into(),
                    ));
                }
                let mut entry = animator.clone();
                values::obj(&mut entry).insert("id", values::s(animator_id));
                edit_list(cx, layer_id, "animators", |list| {
                    list.push(entry);
                    Ok(())
                })
            }
            TextCommand::RemoveAnimator {
                layer_id,
                animator_id,
            } => {
                text_layer(cx, layer_id)?;
                edit_list(cx, layer_id, "animators", |list| {
                    let count = list.len();
                    list.retain(|a| values::text(a, "id") != Some(animator_id.as_str()));
                    if list.len() == count {
                        return Err(CommandError::Invalid(format!(
                            "Text animator not found: {animator_id}"
                        )));
                    }
                    Ok(())
                })
            }
            TextCommand::SetStyleRange {
                layer_id,
                style_id,
                style,
            } => {
                text_layer(cx, layer_id)?;
                edit_list(cx, layer_id, "styles", |list| {
                    let at = list
                        .iter()
                        .position(|s| values::text(s, "id") == Some(style_id.as_str()));
                    match (style, at) {
                        (Some(style), at) => {
                            let mut entry = style.clone();
                            if entry.as_object().is_none() {
                                return Err(CommandError::Invalid(
                                    "A text style must be an object".into(),
                                ));
                            }
                            values::obj(&mut entry).insert("id", values::s(style_id));
                            match at {
                                Some(at) => list[at] = entry,
                                None => list.push(entry),
                            }
                        }
                        (None, Some(at)) => {
                            list.remove(at);
                        }
                        (None, None) => {}
                    }
                    Ok(())
                })
            }
        }
    }
}

/// Rewrites `d.<field>`, an array of structured text objects.
fn edit_list(
    cx: &mut Cx<'_>,
    layer_id: &str,
    field: &str,
    edit: impl FnOnce(&mut Vec<Value>) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let (loc, layer) = cx.layer(layer_id)?;
    let mut list = layer
        .get("d")
        .and_then(|d| d.get(field))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    edit(&mut list)?;
    let mut path = loc.path();
    path.extend(["d".into(), field.into()]);
    cx.set(path, Value::Array(list))
}

/// `splitTextLayers`: the engine kernel makes the pieces; each source's stack
/// slot takes them, the source hides, and one group holds them all.
fn split(
    cx: &mut Cx<'_>,
    layer_ids: &[String],
    group_ids: &[String],
    mode: SplitMode,
    time: f64,
) -> Result<(), CommandError> {
    let time = values::finite(time, "time")?;
    let Some(first) = layer_ids.first() else {
        return Ok(());
    };
    if group_ids.len() != layer_ids.len() {
        return Err(CommandError::Invalid(
            "Split text needs one group id per layer".into(),
        ));
    }
    let comp = cx.find(first)?.comp_id;
    let mut indexes = Vec::with_capacity(layer_ids.len());
    for id in layer_ids {
        let loc = cx.find(id)?;
        if loc.comp_id != comp {
            return Err(CommandError::Invalid(
                "Split text layers must share a composition".into(),
            ));
        }
        indexes.push(loc.index);
    }
    let mut measure = cx.measure(true)?;
    let kernel_mode = match mode {
        SplitMode::Characters => pm_edit::split_text::Mode::Characters,
        SplitMode::Words => pm_edit::split_text::Mode::Words,
        SplitMode::Lines => pm_edit::split_text::Mode::Lines,
    };
    let plans = cx.kernel(comp.as_deref(), |edit| {
        pm_edit::split_text::split_text(edit, &indexes, kernel_mode, time, &mut *measure)
    })?;
    cx.check_fonts()?;
    if plans.iter().all(|plan| plan.children.is_empty()) {
        return Ok(());
    }
    let mut stack = cx.layers(comp.as_deref());
    let mut grouped = Vec::new();
    for ((plan, id), group_id) in plans.iter().zip(layer_ids).zip(group_ids) {
        if plan.children.is_empty() {
            continue;
        }
        let index = stack
            .iter()
            .position(|l| values::text(l, "id") == Some(id.as_str()))
            .ok_or_else(|| CommandError::Invalid(format!("Layer not found: {id}")))?;
        let source = values::obj(&mut stack[index]);
        source.insert("on", values::property(Value::Bool(false)));
        source.insert("solo", Value::Bool(false));
        let name = values::text(&stack[index], "name").map(str::to_owned);
        let mut members: Vec<String> = plan
            .children
            .iter()
            .filter_map(|child| values::text(child, "id").map(str::to_owned))
            .collect();
        members.push(id.clone());
        grouped.push((members, group_id, name));
        stack.splice(index..index, plan.children.iter().cloned());
    }
    layers::write_layers(cx, comp.as_deref(), stack)?;
    for (members, group_id, name) in grouped {
        groups::group(cx, &members, group_id, name.as_deref(), time)?;
    }
    Ok(())
}
