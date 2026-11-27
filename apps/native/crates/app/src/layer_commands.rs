//! Menu commands over the selected layers: plain plans over a publication, so
//! they run and test without a window. The kernel validates at execution.
use crate::dialogs::PrecomposeDraft;
use pm_kernel::commands::comps::PrecomposeMode;
use pm_kernel::commands::content::ContentCommand;
use pm_kernel::commands::groups::GroupCommand;
use pm_kernel::commands::keyframes::{KeyframeCommand, SimplifyScope};
use pm_kernel::commands::layers::{AddLayer, LayerPatch, ParentChange, Restack};
use pm_kernel::commands::media::{CopiedMedia, MediaCommand};
use pm_kernel::commands::precompose::{can_leave_attributes, next_name};
use pm_kernel::commands::properties::SetProperty;
use pm_kernel::commands::text::{SplitMode, TextCommand};
use pm_kernel::commands::timing::{RetimeMode, TimeEdge, TimingCommand};
use pm_kernel::commands::transform::{AlignMode, Spread, TransformCommand};
use pm_kernel::model::{LayerInfo, list_layers};
use pm_kernel::{Command, CompCommand, LayerCommand, PropertyCommand, Published, new_id};
use pm_model::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
/// A ripple, roll or slide made while the timeline shows tracks keeps every
/// clip on the track it shows on (`TimingCommand::InTracks`); in the Layers
/// timeline it is After Effects' layer edit, as it is.
pub(crate) fn in_tracks(published: &Published, command: TimingCommand) -> TimingCommand {
    let mut view = pm_panel_timeline::TimelineModel::default();
    view.restore_ui(&published.ui_state);
    if view.mode != pm_panel_timeline::TimelineMode::Tracks {
        return command;
    }
    let index = pm_panel_timeline::TimelineIndex::from_snapshot(&published.snapshot);
    pm_panel_timeline::tracks::in_tracks(&index, &view.reveal, command)
}

/// Where a layer made in the viewer goes while the timeline shows tracks,
/// over `time` for `duration` (the kernel's default when unset): the lowest
/// track above every clip it overlaps, and the stack place directly above
/// the topmost of them, so the higher track is drawn on top (PM-512). As
/// (track, stack index); None in Layers mode.
pub(crate) fn place_on_top(
    published: &Published,
    kind: &str,
    time: f64,
    duration: Option<f64>,
) -> Option<(u32, usize)> {
    if !shows_tracks(published) {
        return None;
    }
    let fps = comp_number(published, "fps")
        .filter(|f| *f > 0.)
        .unwrap_or(30.);
    let from = (time * fps).round() / fps;
    let comp_end = comp_number(published, "dur").unwrap_or(0.);
    let end = from + duration.unwrap_or_else(|| (comp_end - from).max(0.1));
    let mut view = pm_panel_timeline::TimelineModel::default();
    view.restore_ui(&published.ui_state);
    let index = pm_panel_timeline::TimelineIndex::from_snapshot(&published.snapshot);
    let (track, at) = pm_panel_timeline::tracks::top_place(&index, &view.reveal, kind, from, end);
    Some((u32::try_from(track).ok()?, at))
}

/// Whether the timeline shows tracks (its published view).
pub(crate) fn shows_tracks(published: &Published) -> bool {
    let mut view = pm_panel_timeline::TimelineModel::default();
    view.restore_ui(&published.ui_state);
    view.mode == pm_panel_timeline::TimelineMode::Tracks
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
    Layers(Arc<CopiedLayers>),
    Keys(Vec<pm_panel_timeline::ClipboardTrack>),
}
static CLIPBOARD: Mutex<Clipboard> = Mutex::new(Clipboard::Empty);

pub fn shared_clipboard() -> Clipboard {
    CLIPBOARD.lock().unwrap().clone()
}

pub fn set_clipboard(value: Clipboard) {
    *CLIPBOARD.lock().unwrap() = value;
}

pub fn can_paste() -> bool {
    has_copy() || crate::platform::has_pasteboard_content()
}

/// `can_paste` for the menus' state, asked on every frame the window draws.
pub fn can_paste_per_frame() -> bool {
    has_copy() || crate::platform::has_pasteboard_content_per_frame()
}

fn has_copy() -> bool {
    !matches!(*CLIPBOARD.lock().unwrap(), Clipboard::Empty)
}

#[derive(Debug)]
pub struct CopiedLayers {
    document: u64,
    pub layers: Vec<Value>,
    assets: BTreeMap<String, CopiedAsset>,
    comps: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct CopiedAsset {
    record: Value,
    media: Option<CopiedMedia>,
    /// Archive originals by their source ids; the primary's own id when an
    /// original is the model itself.
    originals: Vec<(String, CopiedMedia)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Asset,
    Comp,
}

/// Every asset and composition a layer draws from, mutably: its media
/// (`asset`/`assetId`, `comp`/`compId` on the layer and under `d`) and,
/// anywhere under `d`, a 3D model source (`assetId`), Studio and extrusion
/// map sources (an asset id, or `{compositionId}`), and world and terrain
/// sources.
fn visit_sources(layer: &mut Value, visit: &mut dyn FnMut(Source, &mut Value)) {
    let Some(fields) = layer.as_object_mut() else { return };
    for (key, value) in fields.iter_mut() {
        match key.as_str() {
            Some("asset" | "assetId") => visit(Source::Asset, value),
            Some("comp" | "compId") => visit(Source::Comp, value),
            Some("d") => visit_data(value, None, visit),
            _ => {}
        }
    }
}

fn visit_data(node: &mut Value, parent: Option<&str>, visit: &mut dyn FnMut(Source, &mut Value)) {
    match node {
        Value::Array(items) => {
            for item in items {
                visit_data(item, parent, visit);
            }
        }
        Value::Object(fields) => {
            let top = parent.is_none();
            for (key, value) in fields.iter_mut() {
                let key = key.as_str().unwrap_or("");
                let text = value.as_str().is_some();
                match key {
                    "assetId" => visit(Source::Asset, value),
                    "asset" if top => visit(Source::Asset, value),
                    "comp" | "compId" if top => visit(Source::Comp, value),
                    "compositionId" => visit(Source::Comp, value),
                    "heightSource" if text => visit(Source::Asset, value),
                    "source" if text && parent == Some("world") => visit(Source::Asset, value),
                    "maps" => {
                        if let Some(maps) = value.as_object_mut() {
                            for (_, map) in maps.iter_mut() {
                                if map.as_str().is_some() {
                                    visit(Source::Asset, map);
                                } else {
                                    visit_data(map, Some("maps"), visit);
                                }
                            }
                        }
                    }
                    _ => visit_data(value, Some(key), visit),
                }
            }
        }
        _ => {}
    }
}

fn references(layer: &Value, kind: Source) -> Vec<String> {
    let mut ids = Vec::new();
    visit_sources(&mut layer.clone(), &mut |found, value| {
        if found == kind
            && let Some(id) = value.as_str()
        {
            ids.push(id.to_owned());
        }
    });
    ids
}

fn remap_fields(node: &mut Value, fields: &[&str], ids: &HashMap<String, String>) {
    let Some(node) = node.as_object_mut() else { return };
    for field in fields {
        if let Some(value) = node.get_mut(*field)
            && let Some(id) = value.as_str().and_then(|id| ids.get(id))
        {
            *value = Value::from(id.as_str());
        }
    }
}

fn remap_sources(layer: &mut Value, assets: &HashMap<String, String>, comps: &HashMap<String, String>) {
    visit_sources(layer, &mut |kind, value| {
        let ids = if kind == Source::Asset { assets } else { comps };
        if let Some(id) = value.as_str().and_then(|id| ids.get(id)) {
            *value = Value::from(id.as_str());
        }
    });
}

/// The archive originals an imported 3D model lists (`native3dSources`).
fn original_ids(asset: &Value) -> Vec<String> {
    asset
        .get("native3dSources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| source.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn copied_asset(
    published: &Published,
    id: &str,
    mut asset: Value,
    resolved: Option<&Path>,
) -> Option<CopiedAsset> {
    let fields = asset.as_object_mut()?;
    let media = published.media.get(id).filter(|entry| {
        !published.media.is_linked(&entry.id) && !published.media.is_proxy_only(id)
    });
    fields.remove("folder");
    fields.remove("originalVersion");
    if media.is_none() {
        let path = fields
            .get("sourcePath")
            .or_else(|| fields.get("path"))
            .and_then(Value::as_str)
            .map(Path::new);
        let absolute = path.filter(|path| path.is_absolute()).map(Path::to_owned)
            .or_else(|| resolved.map(Path::to_owned))
            .or_else(|| Some(published.path.as_deref()?.parent()?.join(path?)));
        if let Some(path) = absolute {
            fields.insert("sourcePath", Value::from(path.to_string_lossy().as_ref()));
        }
    }
    let originals = original_ids(&asset)
        .into_iter()
        .filter(|source| source != id)
        .filter_map(|source| {
            let entry = published.media.get(&source)?;
            Some((source, CopiedMedia(Arc::new(entry.clone()))))
        })
        .collect();
    Some(CopiedAsset {
        record: asset,
        media: media.cloned().map(|entry| CopiedMedia(Arc::new(entry))),
        originals,
    })
}

/// Capture the selected block in stack order and walk the referenced precomps once.
pub fn copy_layers(
    published: &Published,
    pressed: Option<&str>,
    cut: bool,
    originals: &BTreeMap<String, PathBuf>,
) -> Option<(CopiedLayers, Vec<String>)> {
    let facts = Facts::read(published, pressed)?;
    if facts.selected.is_empty() || cut && !facts.cuttable() {
        return None;
    }
    let expanded = facts.expanded();
    let root = published.snapshot.root()?;
    let open = published.snapshot.open_comp();
    let source_comp = pm_kernel::model::comp_node(root, open.as_deref())?;
    let layers: Vec<_> = pm_kernel::model::layers_of(source_comp)
        .filter(|layer| layer.get("id").and_then(crate::node_string).is_some_and(|id| expanded.contains(id)))
        .map(pm_model::Node::to_value)
        .collect();
    if layers.is_empty() {
        return None;
    }
    let mut clipboard = CopiedLayers {
        document: published.snapshot.doc_id,
        layers,
        assets: BTreeMap::new(),
        comps: BTreeMap::new(),
    };
    let mut pending = Vec::new();
    let mut assets = BTreeSet::new();
    let mut scan = |layers: &[Value], pending: &mut Vec<String>| {
        for layer in layers {
            assets.extend(references(layer, Source::Asset));
            pending.extend(references(layer, Source::Comp));
        }
    };
    scan(&clipboard.layers, &mut pending);
    let comps = root.get("comps");
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(comp) = comps.and_then(|comps| comps.get(&id)).map(pm_model::Node::to_value) else {
            continue;
        };
        if let Some(layers) = comp.get("layers").and_then(Value::as_array) {
            scan(layers, &mut pending);
        }
        clipboard.comps.insert(id, comp);
    }
    if let Some(records) = root.get("assets") {
        for id in assets {
            if let Some(asset) = records.get(&id).and_then(|node| {
                copied_asset(published, &id, node.to_value(), originals.get(&id).map(PathBuf::as_path))
            }) {
                clipboard.assets.insert(id, asset);
            }
        }
    }
    Some((clipboard, facts.ids()))
}

impl CopiedLayers {
    pub fn paste(&self, target: &Published, time: f64) -> Option<Plan> {
        let foreign = self.document != target.snapshot.doc_id;
        let mut plan = paste_plan(self.layers.clone(), time, foreign);
        if !foreign {
            return Some(plan);
        }
        let root = target.snapshot.root()?;
        let asset_ids = self.assets.keys().map(|id| {
            let taken = root.get("assets").is_some_and(|assets| assets.get(id).is_some())
                || target.media.get(id).is_some();
            (id.clone(), if taken { new_id("A") } else { id.clone() })
        }).collect::<HashMap<_, _>>();
        let comp_ids = self.comps.keys().map(|id| {
            let taken = root.get("comps").is_some_and(|comps| comps.get(id).is_some())
                || root.get("id").and_then(crate::node_string) == Some(id);
            (id.clone(), if taken { new_id("C") } else { id.clone() })
        }).collect::<HashMap<_, _>>();
        let taken = |id: &str| {
            root.get("assets").is_some_and(|assets| assets.get(id).is_some())
                || target.media.get(id).is_some()
        };
        let mut commands = Vec::new();
        for (id, copied) in &self.assets {
            let asset_id = asset_ids[id].clone();
            // An original keeps its id unless the target has it, or it was
            // the model's own id and the model moved.
            let source_ids: HashMap<String, String> = original_ids(&copied.record)
                .into_iter()
                .map(|source| {
                    let next = if source == *id {
                        asset_id.clone()
                    } else if taken(&source) || asset_ids.values().any(|new| *new == source) {
                        new_id("A")
                    } else {
                        source.clone()
                    };
                    (source, next)
                })
                .collect();
            let mut record = copied.record.clone();
            if let Some(sources) = record
                .as_object_mut()
                .and_then(|record| record.get_mut("native3dSources"))
                .and_then(Value::as_array_mut)
            {
                for source in sources {
                    remap_fields(source, &["id"], &source_ids);
                }
            }
            commands.push(MediaCommand::AddCopy {
                asset_id,
                asset: record,
                media: copied.media.clone(),
                originals: copied
                    .originals
                    .iter()
                    .map(|(source, media)| (source_ids[source].clone(), media.clone()))
                    .collect(),
            }.into());
        }
        let mut layer_ids = HashMap::new();
        for comp in self.comps.values() {
            if let Some(layers) = comp.get("layers").and_then(Value::as_array) {
                for layer in layers {
                    if let Some(id) = layer.get("id").and_then(Value::as_str)
                        && pm_kernel::model::find_layer(root, id).is_some()
                    {
                        layer_ids.entry(id.to_owned()).or_insert_with(|| new_id("L"));
                    }
                }
            }
        }
        for (id, comp) in &self.comps {
            let mut comp = comp.clone();
            let Some(fields) = comp.as_object_mut() else { continue };
            let comp_id = comp_ids[id].clone();
            fields.insert("id", Value::from(comp_id.as_str()));
            fields.remove("folder");
            if let Some(layers) = fields.get_mut("layers").and_then(Value::as_array_mut) {
                for layer in layers {
                    remap_sources(layer, &asset_ids, &comp_ids);
                    remap_fields(layer, &["id", "group", "parent", "matteSource"], &layer_ids);
                    if let Some(effects) = layer.as_object_mut().and_then(|layer| layer.get_mut("fx")).and_then(Value::as_array_mut) {
                        for effect in effects {
                            if let Some(inputs) = effect.as_object_mut().and_then(|effect| effect.get_mut("inputs")).and_then(Value::as_object_mut) {
                                for (_, input) in inputs.iter_mut() {
                                    remap_fields(input, &["layerId"], &layer_ids);
                                }
                            }
                        }
                    }
                }
            }
            commands.push(CompCommand::AddCopy { comp_id, composition: comp }.into());
        }
        if let Some(Command::Layer(LayerCommand::Paste { layers, .. })) = plan.commands.first_mut() {
            for layer in layers {
                remap_sources(layer, &asset_ids, &comp_ids);
            }
        }
        commands.append(&mut plan.commands);
        plan.commands = commands;
        Some(plan)
    }
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
    if name == "palette.selectLayer" {
        return Some(Route::Plan("palette.selectLayer"));
    }
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
    if plan.commands.is_empty() {
        return None;
    }
    let label = Some(plan.label.clone());
    let body = pm_kernel::WriteBody::Commands(plan.commands.clone());
    Some(match &plan.select {
        Some(ids) => pm_kernel::Call::TransactWithSelection {
            label,
            body,
            expected_revision,
            selection: pm_kernel::SelectionCall::Layers {
                ids: ids.clone(),
                add: false,
            },
        },
        None => pm_kernel::Call::Transact {
            label,
            body,
            expected_revision,
        },
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
            | "prevEdge"
            | "nextEdge"
            | "prevSelectedEvent"
            | "nextSelectedEvent"
            | "timeline.addKeyframe"
            | "timeline.addMarker"
            | "timeline.toggleHold"
            | "timeline.toggleSnapping"
            | "timeline.toggleMode"
            | "timeline.toggleShy"
            | "toggleShy"
            | "timeline.zoomIn"
            | "timeline.zoomOut"
            | "timeline.fit"
            | "resetWorkArea"
            | "graph.value"
            | "graph.speed"
            | "graph.fit"
            | "graph.merge"
            | "timeline.revealAll"
    ) || name.starts_with("timeline.revealProperty:")
        || name.starts_with("timeline.keyProperty:")
        || name.starts_with("timeline.tool:")
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
        let mut chosen: HashSet<&str> = published
            .selection
            .layers()
            .iter()
            .map(String::as_str)
            .collect();
        if let Some(id) = pressed
            && !chosen.contains(id)
        {
            chosen = HashSet::from([id]);
        }
        let selected = layers
            .iter()
            .filter(|l| chosen.contains(l.id.as_str()))
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
    fn cuttable(&self) -> bool {
        let expanded = self.expanded();
        self.layers.iter().all(|l| !expanded.contains(l.id.as_str()) || !l.effectively_locked)
    }
    fn info(&self, id: &str) -> Option<&LayerInfo> {
        self.layers.iter().find(|l| l.id == id)
    }
    /// The selection with everything inside its groups (`PM.expandGroups`).
    fn expanded(&self) -> HashSet<&str> {
        let mut members: HashMap<&str, Vec<&str>> = HashMap::new();
        for layer in &self.layers {
            if let Some(group) = layer.group.as_deref() {
                members.entry(group).or_default().push(&layer.id);
            }
        }
        let mut pending: Vec<&str> = self.selected.iter().map(|l| l.id.as_str()).collect();
        let mut ids = HashSet::new();
        while let Some(id) = pending.pop() {
            if ids.insert(id)
                && let Some(children) = members.get(id)
            {
                pending.extend(children);
            }
        }
        ids
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
/// The rows a pop-up lists when its menu opens (the kit's `HostRows`):
/// `layer-parents`, every layer `layer` (with the selection it is part of)
/// can be parented to, as Properties' Parent offers; `layer-mattes`, every
/// layer that can be its track matte. `None` first.
pub fn host_rows(
    published: &Published,
    source: &serde_json::Value,
) -> Option<Vec<serde_json::Value>> {
    use serde_json::json;
    let layer = source.get("layer")?.as_str()?;
    let root = published.snapshot.root()?;
    // By stack number and name, as After Effects lists them: two layers
    // named alike ("Rect", "Rect") are told apart (PM-502).
    let label = |l: &LayerInfo| {
        let name = if l.name.is_empty() { &l.id } else { &l.name };
        format!("{}. {name}", l.index + 1)
    };
    let mut rows = vec![json!({"v": null, "label": "None"})];
    match source.get("source")?.as_str()? {
        "layer-parents" => {
            let facts = Facts::read(published, Some(layer))?;
            let children: Vec<_> = facts
                .selected
                .iter()
                .filter(|l| l.kind != "audio")
                .collect();
            for candidate in parent_candidates(published, &facts, &children) {
                rows.push(json!({"v": candidate.id, "label": label(candidate)}));
            }
        }
        "layer-mattes" => {
            let loc = pm_kernel::model::find_layer(root, layer)?;
            let mut mirror = pm_kernel::QueryMirror::new(published);
            for candidate in &list_layers(root, None) {
                if relation_at(&mut mirror, &loc, "validMatteSource", ("id", &candidate.id)) {
                    rows.push(json!({"v": candidate.id, "label": label(candidate)}));
                }
            }
        }
        _ => return None,
    }
    Some(rows)
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
    relation_at(mirror, &at, op, (key, other))
}

fn relation_at(
    mirror: &mut pm_kernel::QueryMirror,
    at: &pm_kernel::model::LayerLoc,
    op: &str,
    (key, other): (&str, &str),
) -> bool {
    let mut request = Map::new();
    request.insert("op", Value::String(op.into()));
    request.insert("layer", Value::Number(at.index as f64));
    request.insert(
        "comp",
        Value::String(at.comp_id.as_deref().unwrap_or_default().into()),
    );
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
    let layers = &facts.layers;
    let by_id: HashMap<_, _> = layers
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id.as_str(), i))
        .collect();
    // Canonical compositions have unique string IDs and relation fields.
    // Imported legacy data outside that shape keeps the kernel's lookup rules.
    let plain_relations = published
        .snapshot
        .root()
        .and_then(|root| {
            pm_kernel::model::comp_node(root, published.snapshot.open_comp().as_deref())
        })
        .is_some_and(|comp| {
            pm_kernel::model::layers_of(comp)
                .enumerate()
                .all(|(i, node)| {
                    layers.get(i).is_some_and(|row| row.index == i)
                        && ["parent", "group"].iter().all(|key| {
                            node.get(key).is_none_or(|value| {
                                matches!(value.kind(), pm_model::NodeKind::Null)
                                    || crate::node_string(value).is_some()
                            })
                        })
                })
        });
    if by_id.len() != layers.len() || !plain_relations {
        let mut mirror = pm_kernel::QueryMirror::new(published);
        return layers
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
            .collect();
    }

    let mut groups = vec![None; layers.len()];
    let mut members = vec![Vec::new(); layers.len()];
    let mut followers = vec![Vec::new(); layers.len()];
    let lookup = |id: Option<&str>| {
        id.filter(|id| !id.is_empty())
            .and_then(|id| by_id.get(id).copied())
    };
    for (i, layer) in layers.iter().enumerate() {
        if let Some(parent) = lookup(layer.parent.as_deref()) {
            followers[parent].push(i);
        }
        if let Some(group) = lookup(layer.group.as_deref()).filter(|&g| layers[g].kind == "group") {
            groups[i] = Some(group);
            members[group].push(i);
        }
    }
    // wouldCycle rejects the child's enclosing groups outright. It also
    // rejects a parent chain reaching a selected child or one of its members.
    let mut enclosing = vec![false; layers.len()];
    let mut targets = vec![false; layers.len()];
    let mut queue = std::collections::VecDeque::new();
    for child in children {
        let i = by_id[child.id.as_str()];
        targets[i] = true;
        queue.push_back((i, 0));
        let mut next = groups[i];
        while let Some(group) = next {
            if std::mem::replace(&mut enclosing[group], true) {
                break;
            }
            next = groups[group];
        }
    }
    // Take the union of the selected groups' subtrees before walking parents,
    // so the breadth-first traversal starts with all zero-distance targets.
    let mut pending: Vec<_> = children
        .iter()
        .filter(|l| l.kind == "group")
        .map(|l| by_id[l.id.as_str()])
        .collect();
    let mut visited = vec![false; layers.len()];
    while let Some(i) = pending.pop() {
        if std::mem::replace(&mut visited[i], true) {
            continue;
        }
        pending.extend(members[i].iter().copied());
        if !std::mem::replace(&mut targets[i], true) {
            queue.push_back((i, 0));
        }
    }
    while let Some((i, depth)) = queue.pop_front() {
        // The kernel inspects at most 256 nodes including the candidate.
        if depth == 255 {
            continue;
        }
        for &follower in &followers[i] {
            if !std::mem::replace(&mut targets[follower], true) {
                queue.push_back((follower, depth + 1));
            }
        }
    }
    layers
        .iter()
        .enumerate()
        .filter_map(|(i, layer)| {
            (layer.kind != "audio" && !targets[i] && !enclosing[i]).then_some(layer)
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
pub fn layer_menu(published: &Published, pressed: &str, time: f64, can_paste: bool) -> Vec<serde_json::Value> {
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
    // v1.3.2's layer menu, its words and order: the most-used edits, then
    // where the layer sits, then its switches, and Delete last on its own.
    // A title only for a multiple selection, to say what the commands act on.
    let mut rows = Vec::new();
    if facts.selected.len() > 1 {
        rows.push(json!({"header":true,"label":format!("{} Layers", facts.selected.len())}));
    }
    // Rename… leads, for one layer (↩ opens the same field).
    if facts.selected.len() == 1 {
        rows.push(plain("Rename…", "renameLayer"));
    }
    rows.extend([
        row("Cut", "cutLayers", json!([pressed]), facts.cuttable()),
        row("Copy", "copyLayers", json!([pressed]), true),
        row("Paste", "pasteLayers", json!([]), can_paste),
        serde_json::Value::Null,
        plain("Save Animation Preset…", "saveAnimationPreset"),
        plain("Duplicate", "duplicate"),
        plain("Group Layers", "groupLayers"),
    ]);
    if facts.selected.iter().any(|l| l.kind == "group") {
        rows.push(plain("Ungroup Layers", "ungroupLayers"));
    }
    rows.push(plain("Pre-compose…", "precompose"));
    // Un-precompose: the pre-comp becomes a group of its layers.
    if let [only] = facts.selected.as_slice()
        && only.kind == "precomp"
    {
        rows.push(plain("Release Pre-comp", "releasePrecomp"));
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
    if facts.selected.len() == 2 {
        rows.push(row(
            "Use Bottom Layer as Mask",
            "useBottomLayerAsMask",
            json!([]),
            editable && matte_pair(published, &facts).is_some(),
        ));
    }
    rows.push(serde_json::Value::Null);
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
        let common_parent = target
            .parent
            .as_deref()
            .filter(|parent| every(&|l| l.parent.as_deref() == Some(*parent)));
        items.extend(
            parent_candidates(published, &facts, &children)
                .iter()
                .map(|c| {
                    json!({"id":"parentLayers","label":c.name,"args":[c.id],"disabled":!editable,
                "checked":common_parent == Some(c.id.as_str())})
                }),
        );
        rows.push(submenu("Link Layers", items));
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
    // Shy has no row button: the row menu and the Layer menu keep it.
    let shy = layer(published, &target.id).and_then(|l| l.get("shy").and_then(Value::as_bool))
        == Some(true);
    rows.push(row(
        if shy { "Unshy" } else { "Shy" },
        "toggleShy",
        json!([]),
        editable,
    ));
    // Unlocking is never refused: it is how a locked layer becomes editable.
    rows.push(match facts.locked_group(pressed) {
        Some(group) => row("Unlock Group", "unlockGroup", json!([group.id]), true),
        None if target.locked => row("Unlock", "unlockSelectedLayers", json!([]), true),
        None => row("Lock", "lockSelectedLayers", json!([]), true),
    });
    // v1's 3D Layer: the same switch as the timeline row's; audio and
    // adjustment layers have no depth.
    if every(&|l| !matches!(l.kind.as_str(), "audio" | "adjustment")) {
        let on = facts.selected.iter().all(|l| {
            layer(published, &l.id).and_then(|v| v.get("threeD").and_then(Value::as_bool))
                == Some(true)
        });
        let mut depth = row("3D Layer", "toggleThreeD", json!([!on]), editable);
        depth["checked"] = json!(on);
        rows.push(depth);
    }
    if every(&|l| l.kind == "precomp") {
        let on = facts.selected.iter().all(|l| {
            layer(published, &l.id)
                .and_then(|v| v.get("continuousRaster").and_then(Value::as_bool))
                == Some(true)
        });
        let mut raster = plain("Continuously Rasterize", "toggleContinuousRaster");
        raster["checked"] = json!(on);
        rows.push(raster);
    }
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
pub const PLANNED: [&str; 59] = [
    "addEdgeFade",
    "editTransition",
    "addTransition",
    "removeTransition",
    "addDefaultTransition",
    "rippleDelete",
    "nudgeLayerTime",
    "rippleTrimIn",
    "rippleTrimOut",
    "extendEdit",
    "slipLayers",
    "slideLayer",
    "liftWorkArea",
    "extractWorkArea",
    "timeRemapLayers",
    "freezeFrame",
    "reverseLayers",
    "alignLayers",
    "distributeLayers",
    "newSolid",
    "new3d",
    "newCamera",
    "newShape",
    "newText",
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
    "toggleThreeD",
    "unlockSelectedLayers",
    "unlockGroup",
];
/// Menu rows with no working path yet. They stay visible and disabled.
pub const UNAVAILABLE: [&str; 1] = ["help"];
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
/// The Text tool follows v1 in Layers; Tracks titles use five seconds.
/// New Text also uses this in Tracks, keeping its v1 menu rule in Layers.
pub(crate) fn text_layer_duration(published: &Published, time: f64) -> Option<f64> {
    let fps = comp_number(published, "fps").unwrap_or(30.).max(1.);
    let from = (time * fps).round() / fps;
    let remaining = (comp_number(published, "dur")? - from).max(1. / fps);
    Some(if published.ui_state.timeline.get("mode").and_then(Value::as_str) == Some("tracks") { remaining.min(5.) } else { remaining })
}
fn new_layer(published: &Published, kind: &str, name: &str, centred: bool, time: f64) -> Plan {
    let label = match kind {
        "null" => "New Null Object".to_owned(),
        "text" => "New Text".to_owned(),
        _ => format!("New {name}"),
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
    let duration = if kind == "text" && published.ui_state.timeline.get("mode").and_then(Value::as_str) == Some("tracks") { text_layer_duration(published, time).or(duration) } else { duration };
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
        track: None,
        time,
    };
    let mut commands: Vec<Command> = vec![LayerCommand::Add(Box::new(add)).into()];
    if let Some(group) = pm_panel_timeline::focused_group_for_published(published) {
        commands.push(
            pm_kernel::GroupCommand::MoveToGroup {
                layer_ids: vec![id.clone()],
                group: Some(group),
                time,
            }
            .into(),
        );
    }
    Plan {
        select: Some(vec![id]),
        ..plain(&label, commands)
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
/// Scene units per composition pixel of a keyboard nudge on a 3D layer.
const STUDIO_NUDGE: f64 = 0.1;
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
        let studio = crate::gizmo::is_studio(published, &layer.id);
        let step = if studio {
            // A 3D layer moves in scene units, up is up: 0.1 per arrow and
            // 1 with ⇧ (whose step is ten pixels), the ⌃-snap unit (§5).
            Some([delta[0] * STUDIO_NUDGE, -delta[1] * STUDIO_NUDGE])
        } else {
            let q = mirror.queries();
            let (Ok(world), Ok(local)) = (
                q.world_matrix(None, &layer.id, time),
                q.local_matrix(None, &layer.id, time),
            ) else {
                continue;
            };
            crate::gizmo::parent_delta(&world, &local, delta)
        };
        let Some(step) = step else {
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
            let next = if studio {
                ((value + offset) * 1000.).round() / 1000.
            } else {
                value + offset
            };
            let mut set = SetProperty::new(&layer.id, channel, Value::Number(next), time);
            set.preserve_hand_edits = false;
            set.mark_intent = true;
            commands.push(PropertyCommand::Set(set).into());
        }
    }
    (!commands.is_empty()).then(|| plain("Nudge Selection", commands))
}
/// Shift-Delete: the selected layers go, and every later layer closes up by
/// the time they took. A layer moves by the removed spans that end at or
/// before its start; one that overlaps a removed span stays where it is, and
/// so does a locked one. A group that moves carries its members.
fn ripple_delete(layers: &[LayerInfo], selected: &[String], time: f64) -> Option<Plan> {
    let chosen: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let info: HashMap<&str, &LayerInfo> = layers.iter().map(|l| (l.id.as_str(), l)).collect();
    let picked: Vec<&LayerInfo> = layers
        .iter()
        .filter(|l| chosen.contains(l.id.as_str()))
        .collect();
    if picked.is_empty() || picked.iter().any(|l| l.effectively_locked) {
        return None;
    }
    let mut spans: Vec<(f64, f64)> = picked
        .iter()
        .map(|l| (l.in_time, l.out_time))
        .filter(|(start, end)| end > start)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut removed: Vec<(f64, f64)> = vec![];
    for (start, end) in spans {
        match removed.last_mut() {
            Some(last) if start <= last.1 + 1e-6 => last.1 = last.1.max(end),
            _ => removed.push((start, end)),
        }
    }
    let shift = |from: f64| -> f64 {
        removed
            .iter()
            .filter(|(_, end)| from >= end - 1e-6)
            .map(|(start, end)| end - start)
            .sum()
    };
    // Whether a group above the layer is `wanted` (selected, or moving).
    let above = |layer: &LayerInfo, wanted: &dyn Fn(&LayerInfo) -> bool| {
        let mut at = layer.group.as_deref();
        for _ in 0..256 {
            let Some(group) = at.and_then(|id| info.get(id)) else {
                return false;
            };
            if wanted(group) {
                return true;
            }
            at = group.group.as_deref();
        }
        false
    };
    let gone = |l: &LayerInfo| chosen.contains(l.id.as_str());
    let moves = |l: &LayerInfo| !gone(l) && !l.effectively_locked && shift(l.in_time) > 0.;
    let mut by_delta: Vec<(f64, Vec<String>)> = vec![];
    for layer in layers {
        if gone(layer) || above(layer, &gone) || !moves(layer) || above(layer, &moves) {
            continue;
        }
        let delta = -shift(layer.in_time);
        match by_delta.iter_mut().find(|(d, _)| (*d - delta).abs() < 1e-9) {
            Some((_, ids)) => ids.push(layer.id.clone()),
            None => by_delta.push((delta, vec![layer.id.clone()])),
        }
    }
    let mut commands: Vec<Command> = vec![
        LayerCommand::DeleteAt {
            layer_ids: picked.iter().map(|l| l.id.clone()).collect(),
            time,
        }
        .into(),
    ];
    commands.extend(
        by_delta
            .into_iter()
            .map(|(delta, layer_ids)| TimingCommand::Move { layer_ids, delta }.into()),
    );
    Some(plain("Ripple Delete", commands))
}
/// The selected layers `keep` admits that no lock holds, in stack order.
fn unlocked(
    layers: &[LayerInfo],
    selected: &[String],
    keep: impl Fn(&LayerInfo) -> bool,
) -> Vec<String> {
    layers
        .iter()
        .filter(|l| selected.contains(&l.id) && !l.effectively_locked && keep(l))
        .map(|l| l.id.clone())
        .collect()
}
/// The open composition's work area (`work`), else all of it.
fn work_area(published: &Published) -> Option<(f64, f64)> {
    let root = published.snapshot.root()?;
    let open = pm_kernel::model::open_comp_id(root);
    let comp = pm_kernel::model::comp_node(root, open.as_deref())
        .or_else(|| pm_kernel::model::comp_node(root, None))?
        .to_value();
    let work = comp.get("work").and_then(Value::as_array);
    let at = |i: usize| work.and_then(|w| w.get(i)).and_then(Value::as_f64);
    Some((at(0).unwrap_or(0.), at(1).or(comp.get("dur")?.as_f64())?))
}
/// Cmd+V layers: the block keeps its timing and starts at the playhead.
pub fn paste_plan(layers: Vec<Value>, time: f64, foreign: bool) -> Plan {
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
                    foreign,
                }
                .into(),
            ],
        )
    }
}
/// Every layer command a menu, the palette or a key can run. `None` when there
/// is nothing to act on; a key then falls through to the focused control.
/// Why `name` had nothing to change, in plain words, when the person would
/// otherwise wonder: Slip moves a clip's media inside it, so a selection with
/// no video or audio clip has nothing to slip.
pub fn nothing_reason(name: &str, published: &Published) -> Option<&'static str> {
    match name {
        "slipLayers" => {
            let root = published.snapshot.root()?;
            let layers = list_layers(root, None);
            let selected = published.selection.layers();
            let media = layers
                .iter()
                .any(|l| selected.contains(&l.id) && matches!(l.kind.as_str(), "video" | "audio"));
            (!selected.is_empty() && !media)
                .then_some("Slip moves a clip's video or audio; the selected layers have none")
        }
        _ => None,
    }
}

pub fn command_plan(
    name: &str,
    args: &[serde_json::Value],
    published: &Published,
    time: f64,
) -> Option<Plan> {
    let layers = list_layers(published.snapshot.root()?, None);
    let selected = published.selection.layers().to_vec();
    if name == "palette.selectLayer" {
        let id = args.first()?.as_str()?;
        return layers
            .iter()
            .any(|l| l.id == id)
            .then(|| selection_plan(vec![id.to_owned()]));
    }
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
        "editTransition" => {
            use pm_kernel::commands::{
                effects::{Edge, EffectCommand},
                transition_edges,
            };
            let target = args.first()?.as_str()?;
            let side = args.get(1)?.as_str()?;
            let clip = layer(published, target)?;
            let edge = if side == "in" {
                Edge::In
            } else if side == "out" {
                Edge::Out
            } else {
                return None;
            };
            let field = if edge == Edge::In {
                "transitionIn"
            } else {
                "transitionOut"
            };
            let transition = clip.get(field)?;
            let mut spec = transition_edges::builtin_spec(transition.get("type")?.as_str()?)?;
            spec.duration = Some(args.get(2)?.as_f64()?);
            let command = if let Some(peer) = transition.get("paired").and_then(Value::as_str) {
                let (left, right) = if edge == Edge::In {
                    (peer.into(), target.into())
                } else {
                    (target.into(), peer.into())
                };
                EffectCommand::SetPairedTransition {
                    left,
                    right,
                    transition: Some(Box::new(spec)),
                    offset: args.get(3).and_then(serde_json::Value::as_f64),
                }
            } else {
                EffectCommand::SetTransition {
                    layer_id: target.into(),
                    edge,
                    transition: Some(Box::new(spec)),
                }
            };
            Some(plain("Edit Transition", vec![command.into()]))
        }
        "addTransition" | "removeTransition" | "addEdgeFade" => {
            use pm_kernel::commands::{effects::Edge, transition_edges};
            let kind = args.first()?.as_str()?;
            let target = args.get(1)?.as_str()?;
            let edge = match args.get(2)?.as_str()? {
                "in" => Edge::In,
                "out" => Edge::Out,
                _ => return None,
            };
            let spec = if name == "removeTransition" {
                None
            } else {
                Some(transition_edges::builtin_spec(kind)?)
            };
            let mut view = pm_panel_timeline::TimelineModel::default();
            view.restore_ui(&published.ui_state);
            let commands = transition_edges::plan(
                published.snapshot.root()?,
                target,
                edge,
                spec,
                view.mode == pm_panel_timeline::TimelineMode::Tracks && name != "addEdgeFade",
            )
            .ok()?;
            Some(plain(
                if name == "removeTransition" {
                    "Remove Transition"
                } else {
                    "Add Transition"
                },
                commands,
            ))
        }
        "addDefaultTransition" => {
            use pm_kernel::commands::effects::{Edge, TransitionDefinition, TransitionSpec};
            let clip = layers.iter().find(|clip| {
                selected.contains(&clip.id) && clip.kind != "audio" && !clip.effectively_locked
            })?;
            let edge = if (time - clip.in_time).abs() <= (time - clip.out_time).abs() {
                Edge::In
            } else {
                Edge::Out
            };
            let mut view = pm_panel_timeline::TimelineModel::default();
            view.restore_ui(&published.ui_state);
            let commands = pm_kernel::commands::transition_edges::plan(
                published.snapshot.root()?,
                &clip.id,
                edge,
                Some(TransitionSpec {
                    definition: TransitionDefinition {
                        transition_type: "crossfade".into(),
                        params: vec![],
                    },
                    duration: Some(0.5),
                    params: Map::new(),
                }),
                view.mode == pm_panel_timeline::TimelineMode::Tracks,
            )
            .ok()?;
            Some(plain("Add Default Transition", commands))
        }

        "newCamera" => command_plan("new3d", &[serde_json::json!("camera")], published, time),
        "new3d" => {
            // Layer ▸ Add 3D Layer: one Undo step named for what it adds, then the
            // new layer is selected so Properties shows it.
            let kind = args.first().and_then(serde_json::Value::as_str)?;
            let id = new_id("L");
            let (label, commands, select) =
                pm_panel_studio3d::properties::add_layer(published, time, kind, &id).ok()?;
            Some(Plan {
                label,
                commands,
                select: Some(vec![select]),
                count_keys: None,
            })
        }
        "newSolid" => Some(new_layer(published, "solid", "Solid", false, time)),
        "newShape" => Some(new_layer(published, "shape", "Shape", true, time)),
        // v1's "New text layer" (the empty composition's Text button): a
        // text named Headline at the centre, with the default words.
        "newText" => Some(new_layer(published, "text", "Headline", true, time)),
        "newAdjustment" => Some(new_layer(
            published,
            "adjustment",
            "Adjustment Layer",
            false,
            time,
        )),
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
        | "toggleThreeD"
        | "unlockSelectedLayers"
        | "unlockGroup" => layer_menu_plan(name, args, published, time),
        "rippleDelete" => ripple_delete(&layers, &selected, time),
        // The Properties alignment strip, for the palette and agents: the
        // selection's anchors meet the target edge or centre.
        "alignLayers" => {
            let mode = match args.first().and_then(serde_json::Value::as_str)? {
                "left" => AlignMode::Left,
                "hcenter" => AlignMode::HorizontalCenter,
                "right" => AlignMode::Right,
                "top" => AlignMode::Top,
                "vcenter" => AlignMode::VerticalCenter,
                "bottom" => AlignMode::Bottom,
                _ => return None,
            };
            any.then(|| {
                plain(
                    mode.label(),
                    vec![
                        TransformCommand::Align {
                            layer_ids: selected,
                            mode,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
        // Evenly spaced anchors; the outermost stay.
        "distributeLayers" => {
            let spread = match args.first().and_then(serde_json::Value::as_str)? {
                "horizontal" => Spread::HorizontalSpace,
                "vertical" => Spread::VerticalSpace,
                _ => return None,
            };
            (selected.len() >= 3).then(|| {
                plain(
                    spread.label(),
                    vec![
                        TransformCommand::Distribute {
                            layer_ids: selected,
                            spread,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
        // Option-Page Up / Page Down (After Effects): the selected layers
        // move in time by whole frames.
        "nudgeLayerTime" => {
            let frames = args.first().and_then(serde_json::Value::as_f64)?;
            let fps = comp_number(published, "fps").filter(|fps| *fps > 0.)?;
            let layer_ids: Vec<String> = layers
                .iter()
                .filter(|l| selected.contains(&l.id) && !l.effectively_locked)
                .map(|l| l.id.clone())
                .collect();
            (!layer_ids.is_empty() && frames != 0.).then(|| {
                plain(
                    "Move Layers",
                    vec![
                        TimingCommand::Move {
                            layer_ids,
                            delta: frames / fps,
                        }
                        .into(),
                    ],
                )
            })
        }
        // Premiere's Q and W: the selected layers under the playhead.
        "rippleTrimIn" | "rippleTrimOut" => {
            let layer_ids = unlocked(&layers, &selected, |l| {
                l.in_time < time - 1e-6 && time < l.out_time - 1e-6
            });
            let edge = if name == "rippleTrimIn" {
                TimeEdge::In
            } else {
                TimeEdge::Out
            };
            (!layer_ids.is_empty()).then(|| {
                plain(
                    "Ripple Trim",
                    vec![
                        in_tracks(
                            published,
                            TimingCommand::RippleTrim {
                                layer_ids,
                                edge,
                                time,
                            },
                        )
                        .into(),
                    ],
                )
            })
        }
        // Extend Edit (Premiere E, Final Cut Shift-X): the selected layer's
        // nearer edit rolls to the playhead.
        "extendEdit" => {
            let id = unlocked(&layers, &selected, |l| l.kind != "group")
                .into_iter()
                .next()?;
            let info = layers.iter().find(|l| l.id == id)?;
            let edge = if (time - info.in_time).abs() <= (time - info.out_time).abs() {
                TimeEdge::In
            } else {
                TimeEdge::Out
            };
            Some(plain(
                "Extend Edit",
                vec![
                    in_tracks(
                        published,
                        TimingCommand::Roll {
                            layer_id: id,
                            edge,
                            time,
                        },
                    )
                    .into(),
                ],
            ))
        }
        // Slip by frames (Premiere Option-Command-Arrow): a negative count
        // moves the content left, showing later source frames.
        "slipLayers" => {
            let frames = args.first().and_then(serde_json::Value::as_f64)?;
            let fps = comp_number(published, "fps").filter(|fps| *fps > 0.)?;
            let commands: Vec<Command> = unlocked(&layers, &selected, |l| {
                matches!(l.kind.as_str(), "video" | "audio")
            })
            .into_iter()
            .filter_map(|layer_id| {
                let l = layer(published, &layer_id)?;
                let content = |key: &str, fallback: f64| {
                    l.get("d")
                        .and_then(|d| d.get(key))
                        .and_then(|v| v.as_f64().or_else(|| v.get("v")?.as_f64()))
                        .unwrap_or(fallback)
                };
                let rate = if l.get("type").and_then(Value::as_str) == Some("video") {
                    content("speed", 1.).max(0.0001)
                } else {
                    1.
                };
                let source_in = (content("trim", 0.) - frames / fps * rate).max(0.);
                Some(
                    TimingCommand::Slip {
                        layer_id,
                        source_in,
                    }
                    .into(),
                )
            })
            .collect();
            (!commands.is_empty() && frames != 0.).then(|| plain("Slip Layers", commands))
        }
        "slideLayer" => {
            let frames = args.first().and_then(serde_json::Value::as_f64)?;
            let fps = comp_number(published, "fps").filter(|fps| *fps > 0.)?;
            let layer_id = unlocked(&layers, &selected, |l| l.kind != "group")
                .into_iter()
                .next()?;
            (frames != 0.).then(|| {
                plain(
                    "Slide Layer",
                    vec![
                        in_tracks(
                            published,
                            TimingCommand::Slide {
                                layer_id,
                                delta: frames / fps,
                            },
                        )
                        .into(),
                    ],
                )
            })
        }
        // After Effects' Edit ▸ Lift / Extract Work Area: the selected
        // layers, or every layer when none is selected.
        "liftWorkArea" | "extractWorkArea" => {
            let (start, end) = work_area(published)?;
            let layer_ids: Vec<String> = if any {
                selected
            } else {
                layers.iter().map(|l| l.id.clone()).collect()
            };
            let close = name == "extractWorkArea";
            let label = if close {
                "Extract Work Area"
            } else {
                "Lift Work Area"
            };
            Some(plain(
                label,
                vec![
                    TimingCommand::RemoveRange {
                        layer_ids,
                        start,
                        end,
                        close,
                    }
                    .into(),
                ],
            ))
        }
        "timeRemapLayers" | "freezeFrame" | "reverseLayers" => {
            let (mode, label) = match name {
                "timeRemapLayers" => (RetimeMode::Remap, "Enable Time Remapping"),
                "freezeFrame" => (RetimeMode::Freeze, "Freeze Frame"),
                _ => (RetimeMode::Reverse, "Time-Reverse Layer"),
            };
            let layer_ids = unlocked(&layers, &selected, |l| {
                matches!(l.kind.as_str(), "video" | "precomp")
            });
            (!layer_ids.is_empty()).then(|| {
                plain(
                    label,
                    vec![
                        TimingCommand::Retime {
                            layer_ids,
                            mode,
                            time,
                        }
                        .into(),
                    ],
                )
            })
        }
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
                track: None,
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
        "toggleThreeD" => {
            // On for every selected layer unless all are 3D already.
            let all = facts.selected.iter().all(|l| {
                layer(published, &l.id).and_then(|v| v.get("threeD").and_then(Value::as_bool))
                    == Some(true)
            });
            let depth = asked.unwrap_or(!all);
            Some(plain(
                "3D Layer",
                each(&|_| LayerPatch {
                    three_d: Some(depth),
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
/// Facts rebuilt only when the rows are replaced, including after a worker read.
#[derive(Default)]
pub(crate) struct CommandRows {
    positions: HashMap<String, usize>,
    has_locked: bool,
}

impl CommandRows {
    pub(crate) fn new(layers: &[LayerInfo]) -> Self {
        Self {
            positions: layers
                .iter()
                .enumerate()
                .map(|(i, l)| (l.id.clone(), i))
                .collect(),
            has_locked: layers.iter().any(|l| l.locked),
        }
    }
}

#[cfg(test)]
pub fn disabled(published: Option<&Published>, layers: &[LayerInfo]) -> BTreeSet<String> {
    disabled_indexed(
        published,
        layers,
        &CommandRows::new(layers),
        published.is_some_and(pm_panel_studio3d::viewport::supports_navigation),
    )
}

/// The commands that cannot run in this document state, for menus, the palette
/// and the toolbar. A row is live only when its command would do something.
pub(crate) fn disabled_indexed(
    published: Option<&Published>,
    layers: &[LayerInfo],
    index: &CommandRows,
    studio: bool,
) -> BTreeSet<String> {
    let mut off: BTreeSet<String> = UNAVAILABLE.iter().map(|c| (*c).to_owned()).collect();
    // The controller enables Paste from the live app and system clipboards.
    off.insert("pasteLayers".into());
    let Some(published) = published else {
        off.extend(PLANNED.iter().map(|c| (*c).to_owned()));
        off.extend(DOCUMENT.iter().map(|c| (*c).to_owned()));
        off.extend(SELECTION.iter().map(|c| (*c).to_owned()));
        off.insert("studio.view".into());
        off.insert("text.editSelected".into());
        // Nothing to export without a project (v1's File ▸ Export, dimmed
        // over Home): its row and its keys are off (PM-472).
        off.insert("export".into());
        return off;
    };
    let mut seen = HashSet::new();
    let selected: Vec<&LayerInfo> = published
        .selection
        .layers()
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| index.positions.get(id).and_then(|&i| layers.get(i)))
        .collect();
    let keys = !published.selection.keys().is_empty();
    let chosen = !selected.is_empty();
    let has = |kind: &str| selected.iter().any(|l| l.kind == kind);
    let mut set = |name: &str, live: bool| {
        if !live {
            off.insert(name.to_owned());
        }
    };
    set(
        "addDefaultTransition",
        selected
            .iter()
            .any(|clip| clip.kind != "audio" && !clip.effectively_locked),
    );
    set("studio.view", studio);
    for name in SELECTION {
        set(name, chosen);
    }
    // Return edits exactly one unlocked text layer in place.
    set(
        "text.editSelected",
        matches!(selected.as_slice(), [l] if l.kind == "text" && !l.effectively_locked),
    );
    for name in ["copyLayers", "delete"] {
        set(name, chosen || keys);
    }
    set("cutLayers", (chosen || keys) && selected.iter().all(|l| !l.effectively_locked));
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
    let editable = chosen && selected.iter().all(|l| !l.effectively_locked);
    for name in ["simplifyKeyframes", "toggleVisibility", "toggleSolo"] {
        set(name, editable);
    }
    set("rippleDelete", editable);
    let free = |kinds: &[&str]| {
        selected.iter().any(|l| {
            !l.effectively_locked && (kinds.is_empty() || kinds.contains(&l.kind.as_str()))
        })
    };
    for name in ["rippleTrimIn", "rippleTrimOut"] {
        set(name, free(&[]));
    }
    for name in ["extendEdit", "slideLayer", "addDefaultTransition"] {
        set(
            name,
            selected
                .iter()
                .any(|l| !l.effectively_locked && l.kind != "group"),
        );
    }
    // Enabled for any unlocked layer: with no video or audio clip selected,
    // Slip says why in the status line instead of doing nothing (a disabled
    // command's key was silent).
    set(
        "slipLayers",
        selected.iter().any(|l| !l.effectively_locked && l.kind != "group"),
    );
    set("speedDuration", free(&["video", "audio"]));
    for name in ["timeRemapLayers", "freezeFrame", "reverseLayers"] {
        set(name, free(&["video", "precomp"]));
    }
    for name in ["liftWorkArea", "extractWorkArea"] {
        set(name, !layers.is_empty());
    }
    set("alignLayers", editable);
    set("distributeLayers", editable && selected.len() >= 3);
    set(
        "nudgeLayerTime",
        selected.iter().any(|l| !l.effectively_locked),
    );
    set("timeline.toggleHold", keys);
    for name in ["prevSelectedEvent", "nextSelectedEvent"] {
        set(name, chosen);
    }
    for name in ["timeline.addKeyframe", "timeline.keyProperty:p"] {
        set(name, selected.iter().any(|l| !l.effectively_locked));
    }
    set("fitToComposition", editable && !has("group"));
    // One video layer that still carries its soundtrack.
    let soundtrack = matches!(selected.as_slice(), [l] if l.kind == "video"
    && published.snapshot.root().and_then(|root| {
        let comp = pm_kernel::model::comp_node(root, published.snapshot.open_comp().as_deref())?;
        comp.get("layers").and_then(|stack| stack.at(l.index))
            .filter(|node| node.get("id").and_then(crate::node_string) == Some(l.id.as_str()))
            .or_else(|| pm_kernel::model::find_layer(root, &l.id)
                .and_then(|loc| pm_kernel::model::layer_at(root, &loc)))
    }).is_some_and(|node| {
        node.get("type").and_then(crate::node_string) == Some("video")
            && matches!(node.get("d").and_then(|d| d.get("embeddedAudio")).map(pm_model::Node::kind),
                Some(pm_model::NodeKind::Bool(true)))
    }));
    for name in ["separateAudio", "toggleAudioMute"] {
        set(name, editable && soundtrack);
    }
    set("unlockSelectedLayers", selected.iter().any(|l| l.locked));
    set("lockSelectedLayers", selected.iter().any(|l| !l.locked));
    set("unlockAllLayers", index.has_locked);
    off
}
/// Rows that act on the selected layers.
const SELECTION: &[&str] = &[
    "addDefaultTransition",
    "rippleDelete",
    "rippleTrimIn",
    "rippleTrimOut",
    "extendEdit",
    "slipLayers",
    "slideLayer",
    "timeRemapLayers",
    "freezeFrame",
    "reverseLayers",
    "nudgeLayerTime",
    "toggleShy",
    "alignLayers",
    "distributeLayers",
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
const DOCUMENT: &[&str] = &[
    "timeline.tool:selection",
    "timeline.tool:split",
    "timeline.tool:ripple",
    "timeline.tool:roll",
    "timeline.tool:slip",
    "timeline.tool:slide",
    "liftWorkArea",
    "extractWorkArea",
    "importFolder",
    "projectSettings",
    "fitView",
    "prevEdge",
    "nextEdge",
    "timeline.addMarker",
    "timeline.toggleHold",
    "timeline.toggleSnapping",
    "timeline.toggleMode",
    "timeline.toggleShy",
    "timeline.zoomIn",
    "timeline.zoomOut",
    "timeline.fit",
    "resetWorkArea",
    "graph.value",
    "graph.speed",
    "graph.fit",
    "graph.merge",
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
    "speedDuration",
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
        size_to_selection: false,
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
                size_to_selection: draft.size_to_selection && !leave,
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

#[cfg(test)]
mod parent_filter_tests {
    use super::*;

    #[test]
    fn parent_filter_matches_kernel_on_639_layers() {
        let mut stack = Vec::new();
        for i in 0..639 {
            let id = format!("l{i}");
            let parent = match i {
                0 | 1 | 2 | 620.. => None,
                3 => Some("l2".to_owned()),
                4 => Some("l3".to_owned()),
                5..=300 => Some(format!("l{}", i - 1)),
                _ => None,
            };
            let group = match i {
                1 | 2 => Some("l0"),
                301 => Some("l1"),
                302 => Some("l301"),
                _ => None,
            };
            let kind = match i {
                0 | 1 | 301 => "group",
                305 => "audio",
                _ => "solid",
            };
            stack.push(serde_json::json!({
                "id": id, "name": id, "type": kind, "parent": parent, "group": group,
                "from": 0, "dur": 5, "lock": i == 0 || i == 306,
            }));
        }
        use pm_kernel::{KernelActor, KernelConfig, OpenSource};
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(
            &serde_json::json!({
                "id": "project", "compId": "c", "w": 64, "h": 64, "fps": 30, "dur": 5,
                "layers": stack,
            })
            .to_string(),
        )
        .unwrap();
        kernel
            .open("test", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let published = kernel.published("test").unwrap();
        let facts = Facts::read(&published, Some("l1")).unwrap();
        assert_eq!(facts.layers.len(), 639);
        let children = [facts.info("l1").unwrap(), facts.info("l2").unwrap()];
        assert!(children.iter().all(|l| l.effectively_locked));
        let mut mirror = pm_kernel::QueryMirror::new(&published);
        let expected: Vec<_> = facts
            .layers
            .iter()
            .filter(|candidate| {
                candidate.kind != "audio"
                    && children.iter().all(|child| {
                        candidate.id != child.id
                            && !relation(
                                &mut mirror,
                                &published,
                                "wouldCycle",
                                &child.id,
                                ("parent", &candidate.id),
                            )
                    })
            })
            .map(|row| row.id.as_str())
            .collect();
        assert!(!expected.contains(&"l257"));
        assert!(expected.contains(&"l258"));
        assert!(!expected.contains(&"l302"));
        assert!(expected.contains(&"l306"));

        let mut times = Vec::new();
        for _ in 0..21 {
            let start = std::time::Instant::now();
            let candidates = parent_candidates(&published, &facts, &children);
            times.push(start.elapsed());
            assert_eq!(
                candidates
                    .iter()
                    .map(|row| row.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        times.sort_unstable();
        let median = times[times.len() / 2];
        eprintln!(
            "639-layer parent cycle filtering median: {:.3} ms",
            median.as_secs_f64() * 1e3
        );
        assert!(median < std::time::Duration::from_millis(1), "{median:?}");
    }
}

/// Palette scope follows the open composition, including nested compositions.
pub fn palette_layers(published: &Published) -> Vec<(String, String)> {
    published
        .snapshot
        .root()
        .map(|root| pm_kernel::model::layer_names(root, None))
        .unwrap_or_default()
}

/// Last registered provider wins, just as in the Effects browser.
pub fn palette_effects(state: &pm_kernel::state::StateSnapshot) -> Vec<(String, String)> {
    let mut effects = std::collections::BTreeMap::new();
    for entry in state.registries.get("effects").into_iter().flatten() {
        effects.insert(
            entry.id.clone(),
            entry
                .definition
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(&entry.id)
                .to_owned(),
        );
    }
    effects.into_iter().collect()
}
/// All selected layers receive the effect together in one undo step.
pub fn palette_effect_plan(
    published: &Published,
    label: &str,
    mut add: impl FnMut(&str) -> Result<Vec<Command>, String>,
) -> Result<Option<Plan>, String> {
    let layers = palette_layers(published);
    let targets: Vec<_> = published
        .selection
        .layers()
        .iter()
        .filter(|id| layers.iter().any(|(layer, _)| layer == *id))
        .collect();
    if targets.is_empty() {
        return Ok(None);
    }
    let mut commands = Vec::new();
    for id in targets {
        commands.extend(add(id)?);
    }
    Ok(Some(plain(&format!("Add {label}"), commands)))
}

/// Source-handle adjustments are reported after the actor accepts the edit.
pub fn transition_requests(plan: &Plan) -> Vec<(String, f64)> {
    plan.commands
        .iter()
        .filter_map(|command| match command {
            Command::Effect(pm_kernel::EffectCommand::SetPairedTransition {
                left,
                transition: Some(spec),
                ..
            }) => Some((left.clone(), spec.duration.unwrap_or(0.5))),
            _ => None,
        })
        .collect()
}
pub fn transition_notice(published: &Published, requests: &[(String, f64)]) -> Option<String> {
    for (id, requested) in requests {
        let clip = layer(published, id)?;
        let transition = clip.get("transitionOut")?;
        let root = published.snapshot.root()?;
        let loc = pm_kernel::model::find_layer(root, id)?;
        let node = pm_kernel::model::layer_at(root, &loc)?;
        let fitted = pm_kernel::commands::transition_edges::fitted_edge(
            root,
            node,
            pm_kernel::commands::effects::Edge::Out,
        )?;
        let duration = fitted.duration;
        let offset = fitted.offset;
        // Shortening scales both span and offset. Only a change in cut
        // alignment (such as a zero handle forcing the span to one side)
        // is a shift; a zero-width plain cut has no alignment to report.
        let intended_duration = transition.get("dur")?.as_f64()?;
        let intended_offset = transition.get("offset")?.as_f64()?;
        let shifted =
            duration > 0. && (offset / duration - intended_offset / intended_duration).abs() > 1e-6;
        let shortened = duration < *requested - 1e-6;
        if shifted || shortened {
            return Some(format!(
                "Transition {}{} to fit available source media ({duration:.2} s)",
                if shortened { "shortened" } else { "" },
                if shifted {
                    if shortened { " and shifted" } else { "shifted" }
                } else {
                    ""
                }
            ));
        }
    }
    None
}

/// Cached cut admission for the Timeline menu. Only the chosen edge changes
/// with the playhead; project walks happen on a document/selection change.
#[derive(Default)]
pub struct TransitionMenuFacts {
    stamp: Option<(u64, u64)>,
    composition: Option<String>,
    selected: Vec<String>,
    span: Option<(f64, f64)>,
    reasons: [Option<&'static str>; 2],
}
impl TransitionMenuFacts {
    pub fn refusal(
        &mut self,
        published: &Published,
        layers: &[LayerInfo],
        time: f64,
    ) -> Option<&'static str> {
        use pm_kernel::commands::{effects::Edge, transition_edges};
        if published
            .ui_state
            .timeline
            .get("mode")
            .and_then(Value::as_str)
            != Some("tracks")
        {
            return None;
        }
        let root = published.snapshot.root()?;
        let composition = pm_kernel::model::open_comp_id(root);
        let stamp = (published.snapshot.doc_id, published.revision());
        let selected = published.selection.layers();
        if self.stamp != Some(stamp) || self.composition != composition || self.selected != selected
        {
            self.stamp = Some(stamp);
            self.composition = composition;
            self.selected = selected.to_vec();
            self.span = None;
            self.reasons = [None, None];
            if let Some(clip) = layers.iter().find(|clip| {
                selected.contains(&clip.id) && clip.kind != "audio" && !clip.effectively_locked
            }) {
                self.span = Some((clip.in_time, clip.out_time));
                for (i, edge) in [Edge::In, Edge::Out].into_iter().enumerate() {
                    if let Ok(commands) = transition_edges::plan(
                        root,
                        &clip.id,
                        edge,
                        transition_edges::builtin_spec("crossfade"),
                        true,
                    ) {
                        self.reasons[i] = transition_edges::refusal_for_commands(root, &commands);
                    }
                }
            }
        }
        let (start, end) = self.span?;
        self.reasons[usize::from((time - start).abs() > (time - end).abs())]
    }
}
