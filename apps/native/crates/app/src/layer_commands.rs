//! Menu commands over the selected layers: plain plans over a publication, so
//! they run and test without a window. The kernel validates at execution.
use crate::dialogs::PrecomposeDraft;
use pm_kernel::commands::comps::PrecomposeMode;
use pm_kernel::commands::groups::GroupCommand;
use pm_kernel::commands::layers::{AddLayer, LayerPatch, Restack};
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
            | "timeline.revealAll"
    ) || name.starts_with("timeline.revealProperty:")
}
/// The shared layer menu, on timeline rows and in the viewer. Rows run ordinary
/// commands on the selection; an empty row is a separator.
pub const LAYER_MENU: [(&str, &str); 8] = [
    ("duplicate", "Duplicate"),
    ("split", "Split at Playhead"),
    ("groupLayers", "Group"),
    ("bringForward", "Bring Forward"),
    ("sendBackward", "Send Backward"),
    ("lockSelectedLayers", "Lock"),
    ("", ""),
    ("delete", "Delete"),
];
pub fn layer_menu_rows() -> Vec<serde_json::Value> {
    LAYER_MENU
        .iter()
        .map(|(id, label)| {
            if id.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!({"id":id,"label":label})
            }
        })
        .collect()
}
/// Commands [`command_plan`] answers.
pub const PLANNED: [&str; 24] = [
    "newSolid",
    "newShape",
    "newAdjustment",
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
];
/// Menu rows with no working path yet. They stay visible and disabled.
pub const UNAVAILABLE: [&str; 2] = ["help", "importFolder"];
fn selection_plan(ids: Vec<String>) -> Plan {
    Plan {
        label: "Select".into(),
        commands: vec![],
        select: Some(ids),
    }
}
fn new_layer(published: &Published, kind: &str, name: &str, centred: bool, time: f64) -> Plan {
    let id = new_id("L");
    let mut properties = Map::new();
    if centred {
        let size = |key: &str| {
            published
                .snapshot
                .root()
                .and_then(|root| pm_kernel::model::comp_node(root, None))
                .and_then(|comp| comp.get(key))
                .and_then(|value| value.to_value().as_f64())
                .unwrap_or(0.)
        };
        properties.insert("position.x", Value::Number(size("w") / 2.));
        properties.insert("position.y", Value::Number(size("h") / 2.));
    }
    let add = AddLayer {
        id: id.clone(),
        layer_type: kind.into(),
        comp: None,
        name: Some(name.into()),
        from: None,
        duration: None,
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
        ..plain(
            &format!("New {name}"),
            vec![LayerCommand::Add(Box::new(add)).into()],
        )
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
        "nudgeSelection" => {
            let step = |i: usize| args.get(i).and_then(serde_json::Value::as_f64);
            let delta = [step(0)?, step(1)?];
            nudge(published, &layers, &selected, delta, time)
        }
        _ => plan(name, published, time),
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
    set("interpretFootage", has("video") || has("image"));
    set("releasePrecomp", has("precomp"));
    set("toggleContinuousRaster", has("precomp"));
    for name in ["splitTextWords", "splitTextCharacters", "splitTextLines"] {
        set(name, has("text"));
    }
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
const DOCUMENT: [&str; 20] = [
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
    "interpretFootage",
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
mod commands_tests;
#[cfg(test)]
#[path = "layer_commands_tests.rs"]
mod tests;
