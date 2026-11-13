//! Menu commands over the selected layers: plain plans over a publication, so
//! they run and test without a window. The kernel validates at execution.
use crate::dialogs::PrecomposeDraft;
use pm_kernel::commands::comps::PrecomposeMode;
use pm_kernel::commands::content::ContentCommand;
use pm_kernel::commands::groups::GroupCommand;
use pm_kernel::commands::keyframes::{KeyframeCommand, SimplifyScope};
use pm_kernel::commands::layers::{AddLayer, LayerPatch, ParentChange, Restack};
use pm_kernel::commands::precompose::{can_leave_attributes, next_name};
use pm_kernel::commands::properties::SetProperty;
use pm_kernel::commands::text::{SplitMode, TextCommand};
use pm_kernel::commands::transform::TransformCommand;
use pm_kernel::model::{LayerInfo, list_layers};
use pm_kernel::{Command, CompCommand, LayerCommand, PropertyCommand, Published, new_id};
use pm_model::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct Plan {
    pub label: String,
    pub commands: Vec<Command>,
    pub select: Option<Vec<String>>,
    /// These layers' keyframes are counted before and after, and the status
    /// line says both (Simplify Keyframes).
    pub count_keys: Option<Vec<String>>,
}
/// The keyframes on `ids`, however deep their channels sit.
pub fn key_count(published: &Published, ids: &[String]) -> usize {
    fn keys(value: &Value) -> usize {
        match value {
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| match value {
                    Value::Array(keys) if key == "kf" => keys.len(),
                    other => keys(other),
                })
                .sum(),
            Value::Array(items) => items.iter().map(keys).sum(),
            _ => 0,
        }
    }
    ids.iter()
        .filter_map(|id| layer(published, id))
        .map(|l| keys(&l))
        .sum()
}

fn layer(published: &Published, id: &str) -> Option<Value> {
    let root = published.snapshot.root()?;
    let loc = pm_kernel::model::find_layer(root, id)?;
    Some(pm_kernel::model::layer_at(root, &loc)?.to_value())
}
fn kind(layer: &Value) -> Option<&str> {
    layer.get("type").and_then(Value::as_str)
}
fn plain(label: &str, commands: Vec<Command>) -> Plan {
    Plan {
        label: label.into(),
        commands,
        select: None,
        count_keys: None,
    }
}

pub fn plan(name: &str, published: &Published, time: f64) -> Option<Plan> {
    let selected = published.selection.layers().to_vec();
    if selected.is_empty() {
        return None;
    }
    let of_type = |wanted: &str| -> Vec<String> {
        selected
            .iter()
            .filter(|id| layer(published, id).is_some_and(|l| kind(&l) == Some(wanted)))
            .cloned()
            .collect()
    };
    match name {
        "releasePrecomp" => {
            let layer_id = of_type("precomp").into_iter().next()?;
            Some(Plan {
                select: Some(vec![layer_id.clone()]),
                ..plain(
                    "Release Pre-comp",
                    vec![CompCommand::ReleasePrecomp { layer_id }.into()],
                )
            })
        }
        "toggleContinuousRaster" => {
            let precomps = of_type("precomp");
            if precomps.is_empty() {
                return None;
            }
            let all_on = precomps.iter().all(|id| {
                layer(published, id)
                    .and_then(|l| l.get("continuousRaster").and_then(Value::as_bool))
                    == Some(true)
            });
            let commands = precomps
                .into_iter()
                .map(|layer_id| {
                    LayerCommand::Set {
                        layer_id,
                        patch: LayerPatch {
                            continuous_raster: Some(!all_on),
                            time,
                            ..Default::default()
                        },
                    }
                    .into()
                })
                .collect();
            Some(plain("Continuously Rasterize", commands))
        }
        "centerAnchor" => Some(plain(
            "Center Anchor Point",
            vec![
                TransformCommand::CenterAnchor {
                    layer_ids: selected,
                    time,
                }
                .into(),
            ],
        )),
        "splitTextWords" | "splitTextCharacters" | "splitTextLines" => {
            let layer_ids = of_type("text");
            if layer_ids.is_empty() {
                return None;
            }
            let group_ids: Vec<String> = layer_ids.iter().map(|_| new_id("L")).collect();
            let mode = match name {
                "splitTextWords" => SplitMode::Words,
                "splitTextCharacters" => SplitMode::Characters,
                _ => SplitMode::Lines,
            };
            Some(Plan {
                select: Some(group_ids.clone()),
                ..plain(
                    "Split Text",
                    vec![
                        TextCommand::Split {
                            layer_ids,
                            group_ids,
                            mode,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
        _ => None,
    }
}

/// What Cut and Copy last took. Only the newest copy stays pasteable.
#[derive(Clone, Debug, Default)]
pub enum Clipboard {
    #[default]
    Empty,
    Layers(Vec<Value>),
    Keys(Vec<pm_panel_timeline::ClipboardTrack>),
}
/// Where a menu, palette or keyboard command runs. `None`: the session or
/// the shell owns it.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Plan(&'static str),
    Timeline { frames: Option<i32>, shift: bool },
    Copy { cut: bool },
    Paste,
}
pub fn route(name: &str, args: &[serde_json::Value]) -> Option<Route> {
    if let Some(planned) = PLANNED.iter().find(|p| **p == name) {
        return Some(Route::Plan(planned));
    }
    if timeline_owned(name) {
        return Some(Route::Timeline {
            frames: args.first().and_then(|a| a.as_i64()).map(|n| n as i32),
            shift: args.first().and_then(|a| a.as_bool()) == Some(true),
        });
    }
    match name {
        "copyLayers" | "contextCopy" => Some(Route::Copy { cut: false }),
        "cutLayers" | "contextCut" => Some(Route::Copy { cut: true }),
        "pasteLayers" | "contextPaste" => Some(Route::Paste),
        "contextSelectAll" => Some(Route::Plan("selectAll")),
        _ => None,
    }
}
/// A plan's one Undo step; a selection-only plan writes nothing.
pub fn transaction(plan: &Plan, expected_revision: Option<u64>) -> Option<pm_kernel::Call> {
    (!plan.commands.is_empty()).then(|| pm_kernel::Call::Transact {
        label: Some(plan.label.clone()),
        expected_revision,
        body: pm_kernel::WriteBody::Commands(plan.commands.clone()),
    })
}
/// Commands the Timeline model owns, with the same behaviour as its keyboard route.
fn timeline_owned(name: &str) -> bool {
    matches!(
        name,
        "gotoLayerIn"
            | "gotoLayerOut"
            | "gotoWorkIn"
            | "gotoWorkOut"
            | "workIn"
            | "workOut"
            | "moveLayerIn"
            | "moveLayerInToStart"
            | "moveLayerOut"
            | "moveLayerOutToEnd"
            | "trimIn"
            | "trimOut"
            | "split"
            | "nudgeKeyframes"
            | "easyEase"
            | "easyEaseIn"
            | "easyEaseOut"
            | "delete"
            | "deselect"
            | "graph"
            | "timeline.adjacentKeyframe:prev"
            | "timeline.adjacentKeyframe:next"
            | "prevVisibleEvent"
            | "nextVisibleEvent"
            | "timeline.revealAll"
    ) || name.starts_with("timeline.revealProperty:")
}
/// What a layer of the open composition is to the layer menu.
struct Facts {
    layers: Vec<LayerInfo>,
    /// The menu's layers, top of the stack first: the selection when the
    /// pressed layer is in it, else that layer alone.
    selected: Vec<LayerInfo>,
}
impl Facts {
    fn read(published: &Published, pressed: Option<&str>) -> Option<Self> {
        let layers = list_layers(published.snapshot.root()?, None);
        let chosen = published.selection.layers();
        let selected = layers
            .iter()
            .filter(|l| match pressed {
                Some(id) if !chosen.iter().any(|c| c == id) => l.id == id,
                _ => chosen.contains(&l.id),
            })
            .cloned()
            .collect();
        Some(Self { layers, selected })
    }
    fn ids(&self) -> Vec<String> {
        self.selected.iter().map(|l| l.id.clone()).collect()
    }
    /// No selected layer is locked or inside a locked group.
    fn editable(&self) -> bool {
        self.selected.iter().all(|l| !l.effectively_locked)
    }
    fn info(&self, id: &str) -> Option<&LayerInfo> {
        self.layers.iter().find(|l| l.id == id)
    }
    /// The selection with everything inside its groups (`PM.expandGroups`).
    fn expanded(&self) -> HashSet<&str> {
        let mut ids: HashSet<&str> = self.selected.iter().map(|l| l.id.as_str()).collect();
        loop {
            let before = ids.len();
            for l in &self.layers {
                if l.group.as_deref().is_some_and(|g| ids.contains(g)) {
                    ids.insert(&l.id);
                }
            }
            if ids.len() == before {
                return ids;
            }
        }
    }
    /// Unlocked groups the selection could move into.
    fn groups(&self) -> Vec<&LayerInfo> {
        let inside = self.expanded();
        self.layers
            .iter()
            .filter(|l| l.kind == "group" && !l.locked && !inside.contains(l.id.as_str()))
            .collect()
    }
    /// The outermost locked group around `id`.
    fn locked_group(&self, id: &str) -> Option<&LayerInfo> {
        let mut found = None;
        let mut at = self.info(id).and_then(|l| l.group.as_deref());
        for _ in 0..256 {
            let Some(group) = at.and_then(|g| self.info(g)) else {
                break;
            };
            if group.locked {
                found = Some(group);
            }
            at = group.group.as_deref();
        }
        found
    }
}
/// A kernel relation question about `layer` (`wouldCycle`, `validMatteSource`).
fn relation(
    mirror: &mut pm_kernel::QueryMirror,
    published: &Published,
    op: &str,
    layer: &str,
    (key, other): (&str, &str),
) -> bool {
    let Some(at) = published
        .snapshot
        .root()
        .and_then(|root| pm_kernel::model::find_layer(root, layer))
    else {
        return false;
    };
    let mut request = Map::new();
    request.insert("op", Value::String(op.into()));
    request.insert("layer", Value::Number(at.index as f64));
    request.insert("comp", Value::String(at.comp_id.unwrap_or_default().into()));
    request.insert(key, Value::String(other.into()));
    mirror
        .queries()
        .query(&Value::Object(request))
        .ok()
        .and_then(|v| v.as_bool())
        == Some(true)
}
/// The layers a parent menu offers every one of `children` (no cycles, no
/// audio, none of the children themselves).
fn parent_candidates<'a>(
    published: &Published,
    facts: &'a Facts,
    children: &[&LayerInfo],
) -> Vec<&'a LayerInfo> {
    let mut mirror = pm_kernel::QueryMirror::new(published);
    facts
        .layers
        .iter()
        .filter(|candidate| {
            candidate.kind != "audio"
                && children.iter().all(|child| {
                    child.id != candidate.id
                        && !relation(
                            &mut mirror,
                            published,
                            "wouldCycle",
                            &child.id,
                            ("parent", &candidate.id),
                        )
                })
        })
        .collect()
}
/// The upper and lower of exactly two selected layers, when the lower can
/// be the upper's track matte.
fn matte_pair(published: &Published, facts: &Facts) -> Option<(String, String)> {
    let [top, bottom] = facts.selected.as_slice() else {
        return None;
    };
    let mut mirror = pm_kernel::QueryMirror::new(published);
    (!matches!(
        bottom.kind.as_str(),
        "audio" | "null" | "adjustment" | "group"
    ) && relation(
        &mut mirror,
        published,
        "validMatteSource",
        &top.id,
        ("id", &bottom.id),
    ))
    .then(|| (top.id.clone(), bottom.id.clone()))
}
/// A video layer's embedded soundtrack: `Some(muted)` while it has one.
fn embedded_audio(layer: &Value) -> Option<bool> {
    let content = layer.get("d")?;
    (kind(layer) == Some("video")
        && content.get("embeddedAudio").and_then(Value::as_bool) == Some(true))
    .then(|| content.get("audioMuted").and_then(Value::as_bool) == Some(true))
}
/// The `on` switch as the viewer shows it at `time` (it can be animated).
fn shown(published: &Published, info: &LayerInfo, time: f64) -> bool {
    pm_kernel::QueryMirror::new(published)
        .queries()
        .evaluate(None, &info.id, "l.on", time)
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(info.visible)
}
/// The layer menu on a timeline row and in the viewer, as the shipped app's
/// (`layer-menu.ts`): the same rows in the same order, shown and enabled by
/// the same rules. Each row is a command with its arguments; `null` is a
/// separator and `children` a submenu. Preorder positions identify rows.
pub fn layer_menu(published: &Published, pressed: &str, time: f64) -> Vec<serde_json::Value> {
    use serde_json::json;
    let Some(facts) = Facts::read(published, Some(pressed)) else {
        return vec![];
    };
    let Some(target) = facts.info(pressed) else {
        return vec![];
    };
    let editable = facts.editable();
    let every = |test: &dyn Fn(&LayerInfo) -> bool| facts.selected.iter().all(test);
    let row = |label: &str, command: &str, args: serde_json::Value, enabled: bool| json!({"id":command,"label":label,"args":args,"disabled":!enabled});
    let plain = |label: &str, command: &str| row(label, command, json!([]), editable);
    let submenu = |label: &str, children: Vec<serde_json::Value>| json!({"id":"","label":label,"disabled":!editable,"children":children});
    let mut rows = vec![
        plain("Duplicate", "duplicate"),
        plain("Group", "groupLayers"),
        plain("Simplify Keyframes", "simplifyKeyframes"),
        plain("Pre-compose…", "precompose"),
    ];
    if facts.selected.len() == 2 {
        rows.push(row(
            "Use Bottom Layer as Mask",
            "useBottomLayerAsMask",
            json!([]),
            editable && matte_pair(published, &facts).is_some(),
        ));
    }
    if every(&|l| l.kind == "text") {
        rows.push(submenu(
            "Split Text into Layers",
            vec![
                plain("By Word", "splitTextWords"),
                plain("By Character", "splitTextCharacters"),
                plain("By Line", "splitTextLines"),
            ],
        ));
    }
    if facts.selected.iter().any(|l| l.kind == "group") {
        rows.push(plain("Ungroup", "ungroupLayers"));
    }
    let groups = facts.groups();
    if !groups.is_empty() || facts.selected.iter().any(|l| l.group.is_some()) {
        let mut children = vec![row(
            "Outside Groups",
            "moveToGroup",
            json!([null]),
            editable,
        )];
        children.extend(
            groups
                .iter()
                .map(|g| row(&g.name, "moveToGroup", json!([g.id]), editable)),
        );
        rows.push(submenu("Move to Group", children));
    }
    if every(&|l| l.kind != "audio") {
        let children: Vec<&LayerInfo> = facts.selected.iter().collect();
        let mut items = vec![json!({"id":"parentLayers","label":"None","args":[null],
            "disabled":!editable,"checked":every(&|l| l.parent.is_none())})];
        items.extend(
            parent_candidates(published, &facts, &children)
                .iter()
                .map(|c| {
                    json!({"id":"parentLayers","label":c.name,"args":[c.id],"disabled":!editable,
                "checked":every(&|l| l.parent.as_deref() == Some(c.id.as_str()))})
                }),
        );
        rows.push(submenu("Parent", items));
    }
    if every(&|l| l.kind != "group") {
        let inside = every(&|l| time > l.in_time && time < l.out_time);
        rows.push(submenu(
            "Timing",
            vec![
                row("Split at Playhead", "split", json!([]), editable && inside),
                row(
                    "Trim In to Playhead",
                    "trimIn",
                    json!([]),
                    editable && inside,
                ),
                row(
                    "Trim Out to Playhead",
                    "trimOut",
                    json!([]),
                    editable && inside,
                ),
                plain("Fit to Composition", "fitToComposition"),
            ],
        ));
    }
    if let [only] = facts.selected.as_slice()
        && let Some(muted) = layer(published, &only.id).as_ref().and_then(embedded_audio)
    {
        let label = if muted { "Unmute Audio" } else { "Mute Audio" };
        rows.push(plain(label, "toggleAudioMute"));
        rows.push(plain("Separate Audio", "separateAudio"));
    }
    rows.push(serde_json::Value::Null);
    let visible = shown(published, target, time);
    rows.push(row(
        if visible { "Hide" } else { "Show" },
        "toggleVisibility",
        json!([!visible]),
        editable,
    ));
    rows.push(row(
        if target.solo { "Unsolo" } else { "Solo" },
        "toggleSolo",
        json!([!target.solo]),
        editable,
    ));
    // Unlocking is never refused: it is how a locked layer becomes editable.
    rows.push(match facts.locked_group(pressed) {
        Some(group) => row("Unlock Group", "unlockGroup", json!([group.id]), true),
        None if target.locked => row("Unlock", "unlockSelectedLayers", json!([]), true),
        None => row("Lock", "lockSelectedLayers", json!([]), true),
    });
    rows.push(serde_json::Value::Null);
    rows.push(plain("Delete", "delete"));
    rows
}
/// The command and arguments of the menu row at preorder position `index`.
pub fn layer_menu_choice(
    rows: &[serde_json::Value],
    index: usize,
) -> Option<(String, Vec<serde_json::Value>)> {
    let row = crate::v4_native::menu_item(rows, index)?;
    let command = row.get("id")?.as_str().filter(|c| !c.is_empty())?;
    Some((
        command.to_owned(),
        row.get("args")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default(),
    ))
}
/// Commands [`command_plan`] answers.
pub const PLANNED: [&str; 36] = [
    "newSolid",
    "newShape",
    "newAdjustment",
    "newNull",
    "selectAll",
    "selectPreviousLayer",
    "selectNextLayer",
    "extendSelectionPreviousLayer",
    "extendSelectionNextLayer",
    "duplicate",
    "groupLayers",
    "ungroupLayers",
    "bringForward",
    "sendBackward",
    "bringToFront",
    "sendToBack",
    "lockSelectedLayers",
    "unlockAllLayers",
    "nudgeSelection",
    "releasePrecomp",
    "toggleContinuousRaster",
    "centerAnchor",
    "splitTextWords",
    "splitTextCharacters",
    "splitTextLines",
    "simplifyKeyframes",
    "useBottomLayerAsMask",
    "moveToGroup",
    "parentLayers",
    "fitToComposition",
    "toggleAudioMute",
    "separateAudio",
    "toggleVisibility",
    "toggleSolo",
    "unlockSelectedLayers",
    "unlockGroup",
];
/// Menu rows with no working path yet. They stay visible and disabled.
pub const UNAVAILABLE: [&str; 2] = ["help", "importFolder"];
fn selection_plan(ids: Vec<String>) -> Plan {
    Plan {
        select: Some(ids),
        ..plain("Select", vec![])
    }
}
/// A number of the open composition (`w`, `h`, `fps`, `dur`): the one being
/// edited, which is not the project's own while a pre-comp is open.
fn comp_number(published: &Published, key: &str) -> Option<f64> {
    let root = published.snapshot.root()?;
    let open = pm_kernel::model::open_comp_id(root);
    pm_kernel::model::comp_node(root, open.as_deref())
        .or_else(|| pm_kernel::model::comp_node(root, None))?
        .get(key)?
        .to_value()
        .as_f64()
}
fn new_layer(published: &Published, kind: &str, name: &str, centred: bool, time: f64) -> Plan {
    let label = if kind == "null" {
        "New Null Object".to_owned()
    } else {
        format!("New {name}")
    };
    let id = new_id("L");
    let mut properties = Map::new();
    if centred {
        let size = |key: &str| comp_number(published, key).unwrap_or(0.);
        properties.insert("position.x", Value::Number(size("w") / 2.));
        properties.insert("position.y", Value::Number(size("h") / 2.));
    }
    // From the playhead's frame to the composition's end, a second at least
    // (`addLayer` in shortcuts.ts).
    let (from, duration) = comp_number(published, "fps")
        .filter(|fps| *fps > 0.)
        .zip(comp_number(published, "dur"))
        .map(|(fps, dur)| {
            let from = (time * fps).round() / fps;
            (Some(from), Some((dur - from).max(1.)))
        })
        .unwrap_or((None, None));
    let add = AddLayer {
        id: id.clone(),
        layer_type: kind.into(),
        comp: None,
        name: Some(name.into()),
        from,
        duration,
        content: Map::new(),
        properties,
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time,
    };
    Plan {
        select: Some(vec![id]),
        ..plain(&label, vec![LayerCommand::Add(Box::new(add)).into()])
    }
}
/// `selectAdjacentLayer`: the stack's neighbour, or the far end with no selection.
fn adjacent(layers: &[LayerInfo], selected: &[String], up: bool, extend: bool) -> Option<Plan> {
    let rows = || {
        layers
            .iter()
            .enumerate()
            .filter(|(_, l)| selected.contains(&l.id))
            .map(|(i, _)| i)
    };
    let target = if up {
        match rows().min() {
            Some(first) => layers.get(first.checked_sub(1)?)?,
            None => layers.last()?,
        }
    } else {
        match rows().max() {
            Some(last) => layers.get(last + 1)?,
            None => layers.first()?,
        }
    };
    Some(selection_plan(if extend {
        layers
            .iter()
            .filter(|l| selected.contains(&l.id) || l.id == target.id)
            .map(|l| l.id.clone())
            .collect()
    } else {
        vec![target.id.clone()]
    }))
}
/// The selected layers that own their transform. A selected group or parent
/// above carries the rest, which must not receive the same move twice
/// (viewer.ts selectionTransformRoots).
pub fn transform_roots<'a>(layers: &'a [LayerInfo], selected: &[String]) -> HashSet<&'a str> {
    type Link = fn(&LayerInfo) -> Option<&String>;
    let chosen: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let info: HashMap<&str, &LayerInfo> = layers.iter().map(|l| (l.id.as_str(), l)).collect();
    let carried = |layer: &LayerInfo, next: Link| {
        let mut at = next(layer);
        for _ in 0..256 {
            let Some(id) = at else { return false };
            if chosen.contains(id.as_str()) {
                return true;
            }
            at = info.get(id.as_str()).and_then(|l| next(l));
        }
        false
    };
    layers
        .iter()
        .filter(|l| chosen.contains(l.id.as_str()))
        .filter(|l| !carried(l, |l| l.group.as_ref()) && !carried(l, |l| l.parent.as_ref()))
        .map(|l| l.id.as_str())
        .collect()
}
/// One keyboard step in composition pixels, mapped into each layer's parent.
fn nudge(
    published: &Published,
    layers: &[LayerInfo],
    selected: &[String],
    delta: [f64; 2],
    time: f64,
) -> Option<Plan> {
    let mut mirror = pm_kernel::QueryMirror::new(published);
    let ids: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let unlocked: Vec<String> = layers
        .iter()
        .filter(|l| !l.effectively_locked && ids.contains(l.id.as_str()))
        .map(|l| l.id.clone())
        .collect();
    let roots = transform_roots(layers, &unlocked);
    let mut commands = vec![];
    for layer in layers.iter().filter(|l| roots.contains(l.id.as_str())) {
        let q = mirror.queries();
        let (Ok(world), Ok(local)) = (
            q.world_matrix(None, &layer.id, time),
            q.local_matrix(None, &layer.id, time),
        ) else {
            continue;
        };
        let Some(step) = crate::gizmo::parent_delta(&world, &local, delta) else {
            continue;
        };
        for (channel, offset) in [("position.x", step[0]), ("position.y", step[1])] {
            if offset == 0. {
                continue;
            }
            let Some(value) = mirror
                .queries()
                .evaluate(None, &layer.id, channel, time)
                .ok()
                .and_then(|v| v.as_f64())
            else {
                continue;
            };
            let mut set = SetProperty::new(&layer.id, channel, Value::Number(value + offset), time);
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            commands.push(PropertyCommand::Set(set).into());
        }
    }
    (!commands.is_empty()).then(|| plain("Nudge Selection", commands))
}
/// Cmd+V layers: the block keeps its timing and starts at the playhead.
pub fn paste_plan(layers: Vec<Value>, time: f64) -> Plan {
    let new_ids: Vec<String> = layers.iter().map(|_| new_id("L")).collect();
    Plan {
        select: Some(new_ids.clone()),
        ..plain(
            "Paste",
            vec![
                LayerCommand::Paste {
                    comp: None,
                    layers,
                    new_ids,
                    index: None,
                    time: Some(time),
                }
                .into(),
            ],
        )
    }
}
/// Every layer command a menu, the palette or a key can run. `None` when there
/// is nothing to act on; a key then falls through to the focused control.
pub fn command_plan(
    name: &str,
    args: &[serde_json::Value],
    published: &Published,
    time: f64,
) -> Option<Plan> {
    let layers = list_layers(published.snapshot.root()?, None);
    let selected = published.selection.layers().to_vec();
    let any = !selected.is_empty();
    let restack = |to| {
        any.then(|| {
            plain(
                "Reorder Layers",
                vec![
                    LayerCommand::Restack {
                        layer_ids: selected.clone(),
                        to,
                    }
                    .into(),
                ],
            )
        })
    };
    match name {
        "newSolid" => Some(new_layer(published, "solid", "Solid", false, time)),
        "newShape" => Some(new_layer(published, "shape", "Shape", true, time)),
        "newAdjustment" => Some(new_layer(
            published,
            "adjustment",
            "Adjustment Layer",
            false,
            time,
        )),
        // A null is made at the composition's centre, as v1's command did.
        "newNull" => Some(new_layer(published, "null", "Null", true, time)),
        "selectAll" => Some(selection_plan(
            layers.iter().map(|l| l.id.clone()).collect(),
        )),
        "selectPreviousLayer" => adjacent(&layers, &selected, true, false),
        "selectNextLayer" => adjacent(&layers, &selected, false, false),
        "extendSelectionPreviousLayer" => adjacent(&layers, &selected, true, true),
        "extendSelectionNextLayer" => adjacent(&layers, &selected, false, true),
        "duplicate" => {
            if !any
                || layers
                    .iter()
                    .any(|l| selected.contains(&l.id) && l.effectively_locked)
            {
                return None;
            }
            let new_ids: Vec<String> = selected.iter().map(|_| new_id("L")).collect();
            Some(Plan {
                select: Some(new_ids.clone()),
                ..plain(
                    "Duplicate",
                    vec![
                        LayerCommand::Duplicate {
                            layer_ids: selected,
                            new_ids,
                        }
                        .into(),
                    ],
                )
            })
        }
        "groupLayers" => {
            if !any {
                return None;
            }
            let group_id = new_id("L");
            Some(Plan {
                select: Some(vec![group_id.clone()]),
                ..plain(
                    "Group Layers",
                    vec![
                        GroupCommand::Group {
                            layer_ids: selected,
                            group_id,
                            name: None,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
        "ungroupLayers" => {
            let group_ids: Vec<String> = layers
                .iter()
                .filter(|l| l.kind == "group" && selected.contains(&l.id))
                .map(|l| l.id.clone())
                .collect();
            (!group_ids.is_empty()).then(|| {
                plain(
                    "Ungroup",
                    vec![GroupCommand::Ungroup { group_ids, time }.into()],
                )
            })
        }
        "bringForward" => restack(Restack::Forward),
        "sendBackward" => restack(Restack::Backward),
        "bringToFront" => restack(Restack::Front),
        "sendToBack" => restack(Restack::Back),
        "lockSelectedLayers" | "unlockAllLayers" => {
            let lock = name == "lockSelectedLayers";
            let commands: Vec<Command> = layers
                .iter()
                .filter(|l| {
                    if lock {
                        selected.contains(&l.id) && !l.locked
                    } else {
                        l.locked
                    }
                })
                .map(|l| {
                    LayerCommand::Set {
                        layer_id: l.id.clone(),
                        patch: LayerPatch {
                            locked: Some(lock),
                            time,
                            ..Default::default()
                        },
                    }
                    .into()
                })
                .collect();
            let label = if lock { "Lock Layers" } else { "Unlock All" };
            (!commands.is_empty()).then(|| plain(label, commands))
        }
        "simplifyKeyframes"
        | "useBottomLayerAsMask"
        | "moveToGroup"
        | "parentLayers"
        | "fitToComposition"
        | "toggleAudioMute"
        | "separateAudio"
        | "toggleVisibility"
        | "toggleSolo"
        | "unlockSelectedLayers"
        | "unlockGroup" => layer_menu_plan(name, args, published, time),
        "nudgeSelection" => {
            let step = |i: usize| args.get(i).and_then(serde_json::Value::as_f64);
            let delta = [step(0)?, step(1)?];
            nudge(published, &layers, &selected, delta, time)
        }
        _ => plan(name, published, time),
    }
}
/// The layer menu's own commands, on the selection (the palette and the
/// agent run them too). A switch takes its new state as the first argument,
/// or flips what the selection shows.
fn layer_menu_plan(
    name: &str,
    args: &[serde_json::Value],
    published: &Published,
    time: f64,
) -> Option<Plan> {
    let facts = Facts::read(published, None)?;
    let ids = facts.ids();
    let first = facts.selected.first()?;
    if name == "unlockGroup" {
        let group = args.first().and_then(|a| a.as_str())?;
        facts
            .info(group)
            .filter(|g| g.kind == "group" && g.locked)?;
        return Some(plain(
            "Unlock Group",
            vec![set(group, lock_patch(false, time))],
        ));
    }
    if name == "unlockSelectedLayers" {
        let commands: Vec<Command> = facts
            .selected
            .iter()
            .filter(|l| l.locked)
            .map(|l| set(&l.id, lock_patch(false, time)))
            .collect();
        return (!commands.is_empty()).then(|| plain("Unlock Layers", commands));
    }
    if !facts.editable() {
        return None;
    }
    let each = |patch: &dyn Fn(&LayerInfo) -> LayerPatch| -> Vec<Command> {
        facts
            .selected
            .iter()
            .map(|l| set(&l.id, patch(l)))
            .collect()
    };
    let asked = args.first().and_then(|a| a.as_bool());
    match name {
        "simplifyKeyframes" => Some(Plan {
            count_keys: Some(ids.clone()),
            ..plain(
                "Simplify Keyframes",
                vec![
                    KeyframeCommand::Simplify {
                        scope: SimplifyScope::Layers(ids),
                    }
                    .into(),
                ],
            )
        }),
        "useBottomLayerAsMask" => {
            let (top, bottom) = matte_pair(published, &facts)?;
            let patch = LayerPatch {
                matte_source: Some(Some(bottom)),
                time,
                ..Default::default()
            };
            Some(plain("Use Bottom Layer as Mask", vec![set(&top, patch)]))
        }
        "moveToGroup" => {
            let group = match args.first()? {
                serde_json::Value::Null => None,
                id => Some(id.as_str()?.to_owned()),
            };
            if let Some(group) = &group {
                facts.groups().iter().find(|g| g.id == *group)?;
            } else if facts.selected.iter().all(|l| l.group.is_none()) {
                return None;
            }
            let label = if group.is_some() {
                "Move to Group"
            } else {
                "Move out of Group"
            };
            Some(plain(
                label,
                vec![
                    GroupCommand::MoveToGroup {
                        layer_ids: ids,
                        group,
                        time,
                    }
                    .into(),
                ],
            ))
        }
        "parentLayers" => {
            let parent = match args.first()? {
                serde_json::Value::Null => None,
                id => Some(id.as_str()?.to_owned()),
            };
            let children: Vec<&LayerInfo> = facts
                .selected
                .iter()
                .filter(|l| l.kind != "audio")
                .collect();
            if let Some(parent) = &parent {
                parent_candidates(published, &facts, &children)
                    .iter()
                    .find(|c| c.id == *parent)?;
            }
            let commands: Vec<Command> = children
                .iter()
                .filter(|l| l.parent != parent)
                .map(|l| {
                    let patch = LayerPatch {
                        parent: Some(ParentChange {
                            parent: parent.clone(),
                            time,
                        }),
                        time,
                        ..Default::default()
                    };
                    set(&l.id, patch)
                })
                .collect();
            let label = if parent.is_some() {
                "Parent Layers"
            } else {
                "Remove Parent"
            };
            (!commands.is_empty()).then(|| plain(label, commands))
        }
        "fitToComposition" => {
            let duration = comp_number(published, "dur")?;
            if facts.selected.iter().any(|l| l.kind == "group") {
                return None;
            }
            Some(plain(
                "Fit to Composition",
                each(&|_| LayerPatch {
                    from: Some(0.),
                    duration: Some(duration),
                    time,
                    ..Default::default()
                }),
            ))
        }
        "toggleAudioMute" | "separateAudio" => {
            if facts.selected.len() != 1 {
                return None;
            }
            let video = layer(published, &first.id)?;
            let muted = embedded_audio(&video)?;
            let content = video.get("d")?;
            let patch = |key: &str, on: bool| -> Command {
                let mut patch = Map::new();
                patch.insert(key, Value::Bool(on));
                ContentCommand::Set {
                    layer_id: first.id.clone(),
                    patch,
                    time,
                }
                .into()
            };
            if name == "toggleAudioMute" {
                let label = if muted { "Unmute Audio" } else { "Mute Audio" };
                return Some(plain(label, vec![patch("audioMuted", !muted)]));
            }
            // The soundtrack becomes its own layer above the picture, with
            // the level and mute it had (`separateVideoAudio`).
            let asset = content.get("asset").filter(|a| a.is_truthy())?.clone();
            let number = |value: Option<&Value>| {
                value
                    .and_then(|v| v.as_f64().or_else(|| v.get("v").and_then(Value::as_f64)))
                    .unwrap_or(0.)
            };
            let mut audio = Map::new();
            audio.insert("asset", asset);
            audio.insert("trim", Value::Number(number(content.get("trim")).max(0.)));
            audio.insert(
                "gain",
                match content.get("audioGain") {
                    Some(gain) if !gain.is_null() => gain.clone(),
                    _ => Value::Number(1.),
                },
            );
            audio.insert("fadeIn", Value::Number(0.));
            audio.insert("fadeOut", Value::Number(0.));
            let id = new_id("L");
            let add = AddLayer {
                id: id.clone(),
                layer_type: "audio".into(),
                comp: None,
                name: Some(format!("{} Audio", first.name)),
                from: Some(first.in_time),
                duration: Some(first.out_time - first.in_time),
                content: audio,
                properties: Map::new(),
                color: None,
                index: Some(first.index),
                parent: None,
                blend: None,
                motion_blur: None,
                visible: muted.then_some(false),
                shy: None,
                collapsed: None,
                time,
            };
            Some(Plan {
                select: Some(vec![id]),
                ..plain(
                    "Separate Audio",
                    vec![
                        patch("embeddedAudio", false),
                        LayerCommand::Add(Box::new(add)).into(),
                    ],
                )
            })
        }
        "toggleVisibility" => {
            // Shown again only when every selected layer is hidden.
            let show =
                asked.unwrap_or_else(|| facts.selected.iter().all(|l| !shown(published, l, time)));
            let label = if show { "Show Layers" } else { "Hide Layers" };
            Some(plain(
                label,
                each(&|_| LayerPatch {
                    visible: Some(show),
                    time,
                    ..Default::default()
                }),
            ))
        }
        "toggleSolo" => {
            let solo = asked.unwrap_or(!first.solo);
            Some(plain(
                "Solo Layers",
                each(&|_| LayerPatch {
                    solo: Some(solo),
                    time,
                    ..Default::default()
                }),
            ))
        }
        _ => None,
    }
}
fn set(layer: &str, patch: LayerPatch) -> Command {
    LayerCommand::Set {
        layer_id: layer.into(),
        patch,
    }
    .into()
}
fn lock_patch(locked: bool, time: f64) -> LayerPatch {
    LayerPatch {
        locked: Some(locked),
        time,
        ..Default::default()
    }
}
/// The commands that cannot run in this document state, for menus, the palette
/// and the toolbar. A row is live only when its command would do something.
pub fn disabled(
    published: Option<&Published>,
    layers: &[LayerInfo],
    clipboard: &Clipboard,
) -> BTreeSet<String> {
    let mut off: BTreeSet<String> = UNAVAILABLE.iter().map(|c| (*c).to_owned()).collect();
    let Some(published) = published else {
        off.extend(PLANNED.iter().map(|c| (*c).to_owned()));
        off.extend(DOCUMENT.iter().map(|c| (*c).to_owned()));
        off.extend(SELECTION.iter().map(|c| (*c).to_owned()));
        off.insert("text.editSelected".into());
        return off;
    };
    // Runs on every publication, so the selection is looked up, never scanned for.
    let ids: HashSet<&str> = published
        .selection
        .layers()
        .iter()
        .map(String::as_str)
        .collect();
    let selected: Vec<&LayerInfo> = layers
        .iter()
        .filter(|l| !ids.is_empty() && ids.contains(l.id.as_str()))
        .collect();
    let keys = !published.selection.keys().is_empty();
    let chosen = !selected.is_empty();
    let has = |kind: &str| selected.iter().any(|l| l.kind == kind);
    let mut set = |name: &str, live: bool| {
        if !live {
            off.insert(name.to_owned());
        }
    };
    for name in SELECTION {
        set(name, chosen);
    }
    // Return edits exactly one unlocked text layer in place.
    set(
        "text.editSelected",
        matches!(selected.as_slice(), [l] if l.kind == "text" && !l.effectively_locked),
    );
    for name in ["copyLayers", "cutLayers", "delete"] {
        set(name, chosen || keys);
    }
    for name in [
        "nudgeKeyframes",
        "easyEase",
        "easyEaseIn",
        "easyEaseOut",
        "keyframeValue",
        "keyframeVelocity",
        "scaleKeyframes",
    ] {
        set(name, keys);
    }
    for name in [
        "selectAll",
        "selectPreviousLayer",
        "selectNextLayer",
        "extendSelectionPreviousLayer",
        "extendSelectionNextLayer",
    ] {
        set(name, !layers.is_empty());
    }
    set(
        "nudgeSelection",
        selected.iter().any(|l| !l.effectively_locked),
    );
    set(
        "duplicate",
        chosen && selected.iter().all(|l| !l.effectively_locked),
    );
    set("ungroupLayers", has("group"));
    set("releasePrecomp", has("precomp"));
    set("toggleContinuousRaster", has("precomp"));
    for name in ["splitTextWords", "splitTextCharacters", "splitTextLines"] {
        set(name, has("text"));
    }
    let editable = chosen && selected.iter().all(|l| !l.effectively_locked);
    for name in ["simplifyKeyframes", "toggleVisibility", "toggleSolo"] {
        set(name, editable);
    }
    set("fitToComposition", editable && !has("group"));
    // One video layer that still carries its soundtrack.
    let soundtrack = matches!(selected.as_slice(), [l] if l.kind == "video"
        && layer(published, &l.id).as_ref().and_then(embedded_audio).is_some());
    for name in ["separateAudio", "toggleAudioMute"] {
        set(name, editable && soundtrack);
    }
    set("unlockSelectedLayers", selected.iter().any(|l| l.locked));
    set("lockSelectedLayers", selected.iter().any(|l| !l.locked));
    set("unlockAllLayers", layers.iter().any(|l| l.locked));
    set("pasteLayers", !matches!(clipboard, Clipboard::Empty));
    off
}
/// Rows that act on the selected layers.
const SELECTION: [&str; 16] = [
    "split",
    "trimIn",
    "trimOut",
    "moveLayerIn",
    "moveLayerOut",
    "moveLayerInToStart",
    "moveLayerOutToEnd",
    "gotoLayerIn",
    "gotoLayerOut",
    "groupLayers",
    "bringForward",
    "sendBackward",
    "bringToFront",
    "sendToBack",
    "nudgeSelection",
    "centerAnchor",
];
/// Rows that need a document, on top of the selection-based ones.
const DOCUMENT: [&str; 19] = [
    "importSequence",
    "delete",
    "copyLayers",
    "cutLayers",
    "pasteLayers",
    "deselect",
    "precompose",
    "nudgeKeyframes",
    "easyEase",
    "easyEaseIn",
    "easyEaseOut",
    "gotoWorkIn",
    "gotoWorkOut",
    "workIn",
    "workOut",
    "graph",
    "timeline.revealAll",
    "newComposition",
    "compositionSettings",
];

/// A command that acts on the project: asked for while one is still
/// arriving, it waits for it (`Controller::command`) instead of finding none.
pub fn waits_for_project(name: &str) -> bool {
    DOCUMENT.contains(&name) || name == "export" || crate::keyframe_sheets::Kind::of(name).is_some()
}

/// The sheet's starting state; Leave All Attributes needs one layer with a source.
pub fn precompose_draft(published: &Published) -> Option<PrecomposeDraft> {
    let selected = published.selection.layers();
    let first = layer(published, selected.first()?)?;
    let can_leave = selected.len() == 1 && kind(&first).is_some_and(can_leave_attributes);
    Some(PrecomposeDraft {
        name: next_name(published.snapshot.root()?, "Pre-comp"),
        layer_name: first.get("name").and_then(Value::as_str).map(str::to_owned),
        can_leave,
        leave: false,
        adjust_duration: false,
        open_after: false,
    })
}

pub fn precompose_plan(published: &Published, draft: &PrecomposeDraft, time: f64) -> Plan {
    let leave = draft.leave && draft.can_leave;
    let layer_ids = published.selection.layers().to_vec();
    let precomp_layer_id = new_id("L");
    let new_comp_id = new_id("C");
    Plan {
        label: "Pre-compose".into(),
        count_keys: None,
        select: if draft.open_after {
            // The kernel keeps a composition's selection per composition; the
            // new one has none yet.
            Some(vec![])
        } else if leave {
            Some(layer_ids.clone())
        } else {
            Some(vec![precomp_layer_id.clone()])
        },
        commands: std::iter::once(
            CompCommand::Precompose {
                layer_ids,
                new_comp_id: new_comp_id.clone(),
                precomp_layer_id,
                name: Some(draft.name.clone()),
                mode: if leave {
                    PrecomposeMode::LeaveAttributes
                } else {
                    PrecomposeMode::MoveAll
                },
                adjust_duration: draft.adjust_duration && !leave,
                time,
            }
            .into(),
        )
        // Inside the transaction, so Undo also leaves the composition it removes.
        .chain(draft.open_after.then(|| {
            CompCommand::Open {
                comp_id: new_comp_id,
            }
            .into()
        }))
        .collect(),
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
pub(crate) mod commands_tests;
#[cfg(test)]
#[path = "layer_commands_tests.rs"]
mod tests;
