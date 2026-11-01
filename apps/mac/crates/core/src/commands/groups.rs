//! Layer groups.
//!
//! Ports `PM.groupLayers`, `PM.ungroupLayers`, `PM.moveToGroup` and
//! `PM.normalizeGroupStack` of `legacy/core/layer-groups.ts`, with the
//! geometry from the engine's `pm-edit` kernels (`groups::group_placement`,
//! `groups::move_to_group`, `groups::ungroup`), and the group lock policy of
//! `policyCommand`.

use std::collections::HashSet;

use pm_eval::bounds::NoHost;
use pm_eval::text_geom::TextMeasure;
use pm_model::{Map, Value};
use pm_text::compat::{GlyphLayout, TextContent};

use super::{Cx, layers, values};
use crate::error::CommandError;

/// Group commands. A locked target refuses the whole command.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GroupCommand {
    /// `group_layers` (Cmd+G): a new group layer `group_id` named `name`
    /// (default "Group") above the topmost selected layer, its pivot at the
    /// members' bounds center; the layers keep their world poses. Layers in
    /// different groups refuse (`Select layers in the same group`). Label
    /// "Group Layers".
    Group {
        layer_ids: Vec<String>,
        group_id: String,
        name: Option<String>,
        time: f64,
    },
    /// `ungroup_layers` (Cmd+Shift+G): dissolves the groups, moving members
    /// to the group's own parent group with world pose, aim, opacity and
    /// visibility preserved (animated groups are baked to keys). Label
    /// "Ungroup".
    Ungroup { group_ids: Vec<String>, time: f64 },
    /// `move_to_group`: moves layers into `group` (or out of every group
    /// with `None`) keeping their world poses. Label "Move to Group".
    MoveToGroup {
        layer_ids: Vec<String>,
        group: Option<String>,
        time: f64,
    },
}

impl GroupCommand {
    pub fn label(&self) -> String {
        match self {
            GroupCommand::Group { .. } => "Group Layers",
            GroupCommand::Ungroup { .. } => "Ungroup",
            GroupCommand::MoveToGroup { .. } => "Move to Group",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        match self {
            GroupCommand::Group {
                layer_ids,
                group_id,
                name,
                time,
            } => group(cx, layer_ids, group_id, name.as_deref(), *time),
            GroupCommand::Ungroup { group_ids, time } => ungroup(cx, group_ids, *time),
            GroupCommand::MoveToGroup {
                layer_ids,
                group,
                time,
            } => move_to_group(cx, layer_ids, group.as_deref(), *time),
        }
    }
}

/// `policyCommand`: a locked target (or one in a locked group) refuses.
fn check_targets(cx: &Cx<'_>, ids: &[String]) -> Result<Option<String>, CommandError> {
    let mut comp = None;
    for id in ids {
        if let Ok(loc) = cx.find(id) {
            cx.check_lock(&loc, id)?;
            comp.get_or_insert(loc.comp_id);
        }
    }
    Ok(comp.flatten().or_else(|| cx.comp_id(None)))
}

fn index_of(layers: &[Value], id: &str) -> Option<usize> {
    layers
        .iter()
        .position(|l| values::text(l, "id") == Some(id))
}

/// A text measure for kernels that must not measure text (the core has no
/// fonts yet): text layers are kept out of its way by the callers.
struct NoFonts;

impl TextMeasure for NoFonts {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<pm_eval::text_geom::MeasuredLine> {
        Vec::new()
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        GlyphLayout::default()
    }
    fn spaced_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
}

/// `PM.groupLayers(ids, name)`.
fn group(
    cx: &mut Cx<'_>,
    ids: &[String],
    group_id: &str,
    name: Option<&str>,
    time: f64,
) -> Result<(), CommandError> {
    let comp = check_targets(cx, ids)?;
    let layers = cx.layers(comp.as_deref());
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let selected: Vec<&Value> = layers
        .iter()
        .filter(|l| values::text(l, "id").is_some_and(|i| wanted.contains(i)))
        .collect();
    if selected.is_empty() {
        return Err(CommandError::Invalid("Select layers to group".into()));
    }
    if wanted.len() != selected.len() {
        return Err(CommandError::Invalid("Layer not found".into()));
    }
    let roots: Vec<String> = selected
        .iter()
        .filter(|l| {
            !layers::ancestors(&layers, l)
                .iter()
                .any(|g| values::text(g, "id").is_some_and(|i| wanted.contains(i)))
        })
        .filter_map(|l| values::text(l, "id").map(str::to_owned))
        .collect();
    let group_of = |id: &str| {
        layers
            .iter()
            .find(|l| values::text(l, "id") == Some(id))
            .and_then(|l| values::text(l, "group"))
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
    };
    let parent = group_of(&roots[0]);
    if roots.iter().any(|r| group_of(r) != parent) {
        return Err(CommandError::Invalid(
            "Select layers in the same group".into(),
        ));
    }
    if cx.find(group_id).is_ok() {
        return Err(CommandError::Invalid(format!(
            "Layer id already exists: {group_id}"
        )));
    }
    let comp_node = cx.comp_node(comp.as_deref())?;
    let dim = |k: &str, d: f64| {
        comp_node
            .get(k)
            .and_then(crate::model::number_of)
            .unwrap_or(d)
    };
    let dims = (dim("w", 1920.0), dim("h", 1080.0), dim("dur", 5.0));
    let label = name
        .map(values::js_trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("Group");
    let mut g = layers::make_layer(
        "group",
        group_id,
        Some(label),
        dims,
        &Map::new(),
        &Map::new(),
    )?;
    {
        let m = values::obj(&mut g);
        m.insert(
            "group",
            parent.as_deref().map(values::s).unwrap_or(Value::Null),
        );
        m.insert("collapsed", Value::Bool(false));
        m.insert("from", Value::Number(0.0));
        m.insert("dur", Value::Number(cx.duration(comp.as_deref())));
    }
    let at = index_of(&layers, &roots[0]).unwrap_or(0);
    let mut next = layers.clone();
    next.insert(at, g);
    for l in next.iter_mut() {
        if values::text(l, "id").is_some_and(|i| roots.iter().any(|r| r == i)) {
            values::obj(l).insert("group", values::s(group_id));
        }
    }
    let next = layers::normalize_group_stack(next);
    layers::write_layers(cx, comp.as_deref(), next)?;
    // The pivot at the members' bounds center (an identity transform).
    let members: HashSet<&str> = [group_id].into_iter().collect();
    let now = cx.layers(comp.as_deref());
    let has_text = layers::expand_groups(&now, &members).iter().any(|id| {
        now.iter()
            .find(|l| values::text(l, "id") == Some(id.as_str()))
            .is_some_and(|l| values::text(l, "type") == Some("text"))
    });
    if !has_text && let Some(index) = index_of(&now, group_id) {
        let time = values::finite(time, "time")?;
        cx.kernel(comp.as_deref(), |edit| {
            pm_edit::groups::group_placement(edit, index, time, &mut NoFonts, &mut NoHost)
                .map(|_| ())
        })?;
    }
    Ok(())
}

/// `PM.ungroupLayers(ids)`: the checks here, the rebasing and baking in the
/// engine's `groups::ungroup` kernel.
fn ungroup(cx: &mut Cx<'_>, ids: &[String], time: f64) -> Result<(), CommandError> {
    let comp = check_targets(cx, ids)?;
    let layers = cx.layers(comp.as_deref());
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let groups: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            values::text(l, "id").is_some_and(|i| wanted.contains(i))
                && values::text(l, "type") == Some("group")
        })
        .map(|(i, _)| i)
        .collect();
    if groups.is_empty() {
        return Err(CommandError::Invalid("Select a group to ungroup".into()));
    }
    if layers.iter().any(|l| {
        values::flag(l, "lock")
            && layers::ancestors(&layers, l)
                .iter()
                .any(|g| values::text(g, "id").is_some_and(|i| wanted.contains(i)))
    }) {
        return Err(CommandError::Invalid(
            "Unlock the group contents before ungrouping".into(),
        ));
    }
    if groups.iter().any(|&g| values::flag(&layers[g], "solo")) {
        return Err(CommandError::Invalid(
            "Turn off Solo before ungrouping this group.".into(),
        ));
    }
    if let Some(&styled) = groups.iter().find(|&&g| has_visual_style(&layers[g])) {
        return Err(CommandError::Invalid(format!(
            "Remove visual compositing from “{}” before ungrouping",
            values::layer_name(&layers[styled], "")
        )));
    }
    if groups.iter().any(|&g| {
        let on = layers[g].get("on");
        !constant(on, &Value::Bool(true)) && !constant(on, &Value::Bool(false))
    }) {
        return Err(CommandError::Invalid(
            "Pre-compose this group to keep its animated visibility before ungrouping.".into(),
        ));
    }
    let time = values::finite(time, "time")?;
    cx.kernel(comp.as_deref(), |edit| {
        pm_edit::groups::ungroup(edit, &groups, time).map(|_| ())
    })?;
    // The kernel moves the members; the group itself goes (or becomes a null
    // when outside layers use it as a transform parent).
    let now = cx.layers(comp.as_deref());
    let mut next = Vec::with_capacity(now.len());
    for l in now {
        let id = values::text(&l, "id").unwrap_or_default().to_owned();
        if wanted.contains(id.as_str()) && values::text(&l, "type") == Some("group") {
            let parented = next
                .iter()
                .chain(cx.layers(comp.as_deref()).iter())
                .any(|o: &Value| values::text(o, "parent") == Some(id.as_str()));
            if parented {
                let mut l = l;
                let m = values::obj(&mut l);
                m.insert("type", values::s("null"));
                m.insert("solo", Value::Bool(false));
                next.push(l);
            }
            continue;
        }
        next.push(l);
    }
    let next = layers::normalize_group_stack(next);
    layers::write_layers(cx, comp.as_deref(), next)
}

/// `hasVisualStyle(layer)`.
fn has_visual_style(l: &Value) -> bool {
    let non_empty = |k: &str| {
        l.get(k)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    non_empty("fx")
        || non_empty("masks")
        || l.get("matteSource").is_some_and(Value::is_truthy)
        || l.get("transitionIn").is_some_and(Value::is_truthy)
        || l.get("transitionOut").is_some_and(Value::is_truthy)
        || l.get("blend").is_some_and(|b| {
            b.as_object().is_some() || (b.is_truthy() && b.as_str() != Some("normal"))
        })
        || l.get("mblur")
            .is_some_and(|b| b.as_object().is_some() || b.as_bool() == Some(true))
}

/// `constantProperty(prop, value)` for switches.
fn constant(prop: Option<&Value>, value: &Value) -> bool {
    match prop {
        Some(p) if p.as_object().is_some() => {
            if p.get("expr").is_some_and(Value::is_truthy) {
                return false;
            }
            let keys = p.get("kf").and_then(Value::as_array);
            match keys {
                Some(k) if !k.is_empty() => k.iter().all(|key| key.get("v") == Some(value)),
                _ => p.get("v") == Some(value),
            }
        }
        Some(p) => p == value,
        // `undefined === true` is false.
        None => false,
    }
}

/// `PM.moveToGroup(ids, target)`.
fn move_to_group(
    cx: &mut Cx<'_>,
    ids: &[String],
    target: Option<&str>,
    time: f64,
) -> Result<(), CommandError> {
    let comp = check_targets(cx, ids)?;
    let layers = cx.layers(comp.as_deref());
    let group = match target {
        Some(t) => Some(
            layers
                .iter()
                .find(|l| {
                    values::text(l, "id") == Some(t) && values::text(l, "type") == Some("group")
                })
                .ok_or_else(|| CommandError::Invalid("Group not found".into()))?,
        ),
        None => None,
    };
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let selected: Vec<&Value> = layers
        .iter()
        .filter(|l| values::text(l, "id").is_some_and(|i| wanted.contains(i)))
        .collect();
    if selected.is_empty() || wanted.len() != selected.len() {
        return Err(CommandError::Invalid("Layer not found".into()));
    }
    let locked = |l: &Value| {
        values::flag(l, "lock")
            || layers::ancestors(&layers, l)
                .iter()
                .any(|g| values::flag(g, "lock"))
    };
    if group.is_some_and(locked) || selected.iter().any(|l| locked(l)) {
        return Err(CommandError::Invalid("Unlock layers before moving".into()));
    }
    if let Some(g) = group
        && (wanted.contains(values::text(g, "id").unwrap_or_default())
            || layers::ancestors(&layers, g)
                .iter()
                .any(|a| values::text(a, "id").is_some_and(|i| wanted.contains(i))))
    {
        return Err(CommandError::Invalid(
            "Grouping would create a cycle".into(),
        ));
    }
    let roots: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            values::text(l, "id").is_some_and(|i| wanted.contains(i))
                && !layers::ancestors(&layers, l)
                    .iter()
                    .any(|g| values::text(g, "id").is_some_and(|i| wanted.contains(i)))
        })
        .map(|(i, _)| i)
        .collect();
    let target_index = target.and_then(|t| index_of(&layers, t));
    layers::move_into_group(
        cx,
        comp.as_deref(),
        &roots,
        target_index,
        target,
        values::finite(time, "time")?,
    )?;
    let next = layers::normalize_group_stack(cx.layers(comp.as_deref()));
    layers::write_layers(cx, comp.as_deref(), next)
}
