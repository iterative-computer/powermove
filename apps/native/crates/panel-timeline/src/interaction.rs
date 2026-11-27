use crate::snap::{Anchor, DirectionalSnap, Edge, Target};
use crate::{GraphDisplay, GraphMode, PropertyFacts, TimelineIndex, TimelineMode, TimelineModel};
use pm_kernel::commands::{
    comps::CompPatch,
    keyframes::{CopiedKey, EaseSide, EaseSpec, HandleValue, TemporalEase},
    layers::{LayerPatch, ParentChange},
    properties::SetProperty,
};
use pm_kernel::{
    Call, Command, CompCommand, HistoryCall, KeyframeCommand, LayerCommand, MarkerCommand,
    PropertyCommand, SelectionCall, TimingCommand, WriteBody, new_id,
};
use pm_model::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub control: bool,
}
impl Modifiers {
    pub fn command(self) -> bool {
        self.meta || self.control
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    UnknownId(String),
    UnknownCommand(String),
    HiddenRow,
    InvalidNumber,
    Locked(String),
    InvalidGesture,
    MissingHost(&'static str),
    KernelPlan(String),
}
/// A sentence for the status line: never a variant name or an internal id.
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownId(_) => f.write_str("That item is no longer in the timeline."),
            Self::UnknownCommand(_) | Self::MissingHost(_) => {
                f.write_str("The timeline can’t do that here.")
            }
            Self::HiddenRow => f.write_str("That row is hidden."),
            Self::InvalidNumber => f.write_str("That isn’t a number the timeline can use."),
            Self::Locked(_) => f.write_str("That layer is locked."),
            Self::InvalidGesture => f.write_str("That timeline edit can’t be made."),
            Self::KernelPlan(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for ModelError {}

/// Effects the application must execute on its transport/clipboard/extension hosts.
/// Returning an effect does not claim that host work has succeeded.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEffect {
    /// One of the app's own commands, by name (the empty composition's
    /// Text, Shape, Solid, Null and Import… buttons).
    Run(String),
    Seek(f64),
    Status(String),
    LockedStrip {
        layer: String,
        document: u64,
        composition: Option<String>,
        trim: bool,
    },
    TogglePlayback,
    /// Fit Selected Curves: show this span of composition time.
    FitCurves(f64, f64),
    FrameComposition,
    OpenComposition(String),
    FocusGroup(Option<String>),
    ShowLayerMenu(String),
    EditKeyValues(Vec<String>),
    EditVelocity(Vec<String>),
    ScaleKeys(Vec<String>),
    CopyKeys(Vec<ClipboardTrack>),
    Import {
        time: f64,
        before_layer: Option<String>,
        as_folder: bool,
    },
    GatherMenu {
        kind: &'static str,
        layer_id: Option<String>,
        time: f64,
    },
}
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub calls: Vec<Call>,
    pub effects: Vec<HostEffect>,
    pub expected_document: Option<u64>,
    pub required_origin: Option<pm_kernel::Origin>,
}
impl Plan {
    pub fn edit(commands: Vec<Command>, revision: u64) -> Self {
        Self {
            calls: vec![Call::Transact {
                label: None,
                body: WriteBody::Commands(commands),
                expected_revision: Some(revision),
            }],
            effects: Vec::new(),
            ..Self::default()
        }
    }
    pub(crate) fn seek(time: f64) -> Self {
        Self {
            calls: Vec::new(),
            effects: vec![HostEffect::Seek(time)],
            ..Self::default()
        }
    }
}

/// No platform keycodes: shell translates keys and uses its ordered keymap resolver.
#[derive(Clone, Debug)]
pub struct KeyInput<'a> {
    pub key: &'a str,
    pub modifiers: Modifiers,
    pub repeat: bool,
    pub in_field: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub command: &'static str,
    pub frames: Option<i32>,
    pub shift_reveal: bool,
}
/// Tag keymap-default/keymap.ts:65–176. Only timeline-owned/editor timeline bindings.
/// Other editor shortcuts decline to the shell rather than stealing their keys.
pub fn resolve_key(input: &KeyInput<'_>) -> Option<Shortcut> {
    if input.in_field {
        return None;
    }
    let m = input.modifiers;
    let mut frames = None;
    let mut shift_reveal = false;
    let command = match (input.key, m.command(), m.alt, m.shift) {
        ("space", _, _, _) => "play",
        ("home", false, false, false) => "gotoStart",
        ("end", false, false, false) => "gotoEnd",
        ("home", false, false, true) => "gotoWorkIn",
        ("end", false, false, true) => "gotoWorkOut",
        ("pageup", false, false, false) => "prevFrame",
        ("pagedown", false, false, false) => "nextFrame",
        ("pageup", false, false, true) => {
            frames = Some(-10);
            "stepFrames"
        }
        ("pagedown", false, false, true) => {
            frames = Some(10);
            "stepFrames"
        }
        ("left", true, false, false) if m.control && !m.meta => "timeline.adjacentKeyframe:prev",
        ("right", true, false, false) if m.control && !m.meta => "timeline.adjacentKeyframe:next",
        ("left", true, false, false) => "prevFrame",
        ("right", true, false, false) => "nextFrame",
        ("left", true, false, true) => {
            frames = Some(-10);
            "stepFrames"
        }
        ("right", true, false, true) => {
            frames = Some(10);
            "stepFrames"
        }
        ("left", false, true, shift) => {
            frames = Some(if shift { -10 } else { -1 });
            "nudgeKeyframes"
        }
        ("right", false, true, shift) => {
            frames = Some(if shift { 10 } else { 1 });
            "nudgeKeyframes"
        }
        ("j", false, false, false) => "prevVisibleEvent",
        ("k", false, false, false) => "nextVisibleEvent",
        ("j", false, false, true) => "timeline.adjacentKeyframe:prev",
        ("k", false, false, true) => "timeline.adjacentKeyframe:next",
        ("i", false, false, false) => "gotoLayerIn",
        ("o", false, false, false) => "gotoLayerOut",
        ("c", false, false, false) => "timeline.tool:split",
        ("v", false, false, false) => "timeline.tool:selection",
        ("[", false, false, false) => "moveLayerIn",
        ("]", false, false, false) => "moveLayerOut",
        ("[", false, true, false) => "trimIn",
        ("]", false, true, false) => "trimOut",
        ("home", false, true, false) => "moveLayerInToStart",
        ("end", false, true, false) => "moveLayerOutToEnd",
        ("b", false, false, _) => "workIn",
        ("n", false, false, _) => "workOut",
        ("b", true, false, false) | ("d", true, false, true) => "split",
        ("f3", false, false, true) => "graph",
        ("f9", false, false, false) => "easyEase",
        ("f9", false, false, true) => "easyEaseIn",
        ("f9", true, false, true) => "easyEaseOut",
        ("`", true, false, false) => "timeline.revealAll",
        ("p" | "s" | "r" | "t" | "a" | "u" | "m" | "f" | "e" | "l", false, false, shift) => {
            shift_reveal = shift;
            match input.key {
                "p" => "timeline.revealProperty:p",
                "s" => "timeline.revealProperty:s",
                "r" => "timeline.revealProperty:r",
                "t" => "timeline.revealProperty:t",
                "a" => "timeline.revealProperty:a",
                "u" => "timeline.revealProperty:u",
                "m" => "timeline.revealProperty:m",
                "f" => "timeline.revealProperty:f",
                "e" => "timeline.revealProperty:e",
                _ => "timeline.revealProperty:l",
            }
        }
        ("backspace" | "delete", false, false, false) => "delete",
        ("escape", false, false, false) => "deselect",
        _ => return None,
    };
    if input.repeat
        && !matches!(
            command,
            "prevFrame" | "nextFrame" | "stepFrames" | "nudgeKeyframes"
        )
    {
        return None;
    }
    Some(Shortcut {
        command,
        frames,
        shift_reveal,
    })
}

impl TimelineModel {
    /// Identical entrypoint for shell keybindings, palette, pointer actions and agents.
    pub fn command(
        &mut self,
        id: &str,
        time: f64,
        frames: Option<i32>,
        shift: bool,
        millis: u64,
        facts: &PropertyFacts,
    ) -> Result<Plan, ModelError> {
        finite(time)?;
        let layers = self.selection.layers().to_vec();
        let keys = self.selection.keys().to_vec();
        let edit = |c: Command| Plan::edit(vec![c], self.index.revision);
        if let Some(key) = id
            .strip_prefix("timeline.revealProperty:")
            .and_then(|s| if s.len() == 1 { s.chars().next() } else { None })
        {
            self.reveal_property(key, shift, millis, facts)?;
            return Ok(Plan::default());
        }
        if let Some(property) = id.strip_prefix("timeline.keyProperty:") {
            return self.key_property(property, time);
        }
        let duration = self.index.duration;
        Ok(match id {
            "timeline.addKeyframe" => self.add_keyframes(time)?,
            "play" => Plan {
                effects: vec![HostEffect::TogglePlayback],
                ..Plan::default()
            },
            "gotoStart" => Plan::seek(0.),
            "gotoEnd" => Plan::seek(duration),
            "gotoWorkIn" => Plan::seek(self.index.work[0]),
            "gotoWorkOut" => Plan::seek(self.index.work[1]),
            "prevFrame" | "nextFrame" | "stepFrames" => {
                let n = if id == "prevFrame" {
                    -1
                } else if id == "nextFrame" {
                    1
                } else {
                    frames.unwrap_or(1)
                };
                Plan::seek(
                    (frame(time, self.index.fps) + f64::from(n) / self.index.fps)
                        .clamp(0., duration),
                )
            }
            "timeline.adjacentKeyframe:prev" | "timeline.adjacentKeyframe:next" => self
                .adjacent_key(if id.ends_with("prev") { -1 } else { 1 }, time, None)
                .map_or_else(Plan::default, Plan::seek),
            // The stops After Effects counts as visible: the composition's
            // ends, the work area's, markers, layer ends and keyframes.
            // Edges are the composition's ends, layer ends and keyframes;
            // selected events are the selected layers' ends and keyframes.
            "prevVisibleEvent" | "nextVisibleEvent" | "prevEdge" | "nextEdge"
            | "prevSelectedEvent" | "nextSelectedEvent" => {
                let selected = id.ends_with("SelectedEvent");
                let mut events = if selected { vec![] } else { vec![0., duration] };
                if id.ends_with("VisibleEvent") {
                    events.extend(self.index.work);
                    events.extend(self.index.markers.iter().map(|m| m.time));
                }
                for l in &self.index.layers {
                    if selected && !layers.contains(&l.id) {
                        continue;
                    }
                    events.extend([l.from, l.from + l.duration]);
                    for c in &l.channels {
                        events.extend(c.keys.iter().map(|k| l.from + k.time));
                    }
                }
                events.retain(|t| t.is_finite() && *t >= 0. && *t <= duration);
                for e in &mut events {
                    *e = (*e * 1e6).round() / 1e6;
                }
                events.sort_by(f64::total_cmp);
                let next = if id.starts_with("next") {
                    events.into_iter().find(|t| *t > time + 1e-5)
                } else {
                    events.into_iter().rev().find(|t| *t < time - 1e-5)
                };
                next.map_or_else(Plan::default, Plan::seek)
            }
            "gotoLayerIn" | "gotoLayerOut" => {
                let selected: Vec<_> = self
                    .index
                    .layers
                    .iter()
                    .filter(|l| layers.contains(&l.id))
                    .collect();
                let target = if id.ends_with("In") {
                    selected.iter().map(|l| l.from).reduce(f64::min)
                } else {
                    selected
                        .iter()
                        .map(|l| l.from + l.duration)
                        .reduce(f64::max)
                };
                target.map_or_else(Plan::default, |t| Plan::seek(t.clamp(0., duration)))
            }
            "workIn" => edit(
                CompCommand::SetWorkArea {
                    comp: None,
                    start: Some(time),
                    end: None,
                }
                .into(),
            ),
            "workOut" => edit(
                CompCommand::SetWorkArea {
                    comp: None,
                    start: None,
                    end: Some(time),
                }
                .into(),
            ),
            "resetWorkArea" => edit(CompCommand::ResetWorkArea { comp: None }.into()),
            "moveLayerIn" | "moveLayerInToStart" => edit(
                TimingCommand::MoveInTo {
                    layer_ids: layers,
                    time: if id.ends_with("Start") { 0. } else { time },
                }
                .into(),
            ),
            "moveLayerOut" | "moveLayerOutToEnd" => edit(
                TimingCommand::MoveOutTo {
                    layer_ids: layers,
                    time: if id.ends_with("End") { duration } else { time },
                }
                .into(),
            ),
            "trimIn" | "trimOut" => {
                let is_in = id == "trimIn";
                let selected: HashSet<_> = layers.iter().collect();
                let min_duration = 1. / self.index.fps;
                let mut layer_ids = Vec::new();
                for id in self.index.expanded_ids(&layers) {
                    let layer = self.index.layer(&id).unwrap();
                    if layer.effectively_locked {
                        continue;
                    }
                    let (from, duration) = self.index.strip_span(layer);
                    let valid = if is_in {
                        time <= from + duration - min_duration
                    } else {
                        time >= from + min_duration
                    };
                    if valid {
                        layer_ids.push(id);
                    } else if selected.contains(&id) {
                        return Err(ModelError::KernelPlan(
                            "Trim refused: the playhead must leave at least one frame in the clip.".into(),
                        ));
                    } else {
                        // A member that cannot keep a frame inside the new
                        // edge would go on drawing outside its group.
                        return Err(ModelError::KernelPlan(
                            "Trim refused: a layer in the group would stay outside the new edge.".into(),
                        ));
                    }
                }
                if layer_ids.is_empty() {
                    Plan::default()
                } else if is_in {
                    edit(TimingCommand::TrimIn { layer_ids, time }.into())
                } else {
                    edit(TimingCommand::TrimOut { layer_ids, time }.into())
                }
            }
            "split" => {
                let new_ids: Vec<_> = layers.iter().map(|_| new_id("L")).collect();
                let tails: Vec<_> = layers
                    .iter()
                    .zip(&new_ids)
                    .filter(|(id, _)| {
                        self.index.layer(id).is_some_and(|l| {
                            time > l.from && time < l.from + l.duration && !l.effectively_locked
                        })
                    })
                    .map(|(_, id)| id.clone())
                    .collect();
                let mut plan = edit(
                    TimingCommand::Split {
                        layer_ids: layers,
                        time,
                        new_ids,
                    }
                    .into(),
                );
                if !tails.is_empty() {
                    plan.calls.push(
                        SelectionCall::Layers {
                            ids: tails,
                            add: false,
                        }
                        .into(),
                    );
                }
                plan
            }
            "nudgeKeyframes" => edit(
                KeyframeCommand::Move {
                    key_ids: keys,
                    delta: f64::from(frames.unwrap_or(1)) / self.index.fps,
                }
                .into(),
            ),
            "easyEase" | "easyEaseIn" | "easyEaseOut" => edit(
                KeyframeCommand::EasyEase {
                    key_ids: keys,
                    side: if id.ends_with("In") {
                        EaseSide::In
                    } else if id.ends_with("Out") {
                        EaseSide::Out
                    } else {
                        EaseSide::Both
                    },
                }
                .into(),
            ),
            // A selected transition goes, not its clip.
            "delete"
                if let Some((command, audio)) =
                    self.transition.as_ref().and_then(|(l, edge)| {
                        let layer = self.index.layer(l)?;
                        Some((
                            transition_command(layer, *edge, None, time)?,
                            layer.kind == "audio",
                        ))
                    }) =>
            {
                Plan {
                    calls: vec![Call::Transact {
                        label: Some(
                            if audio {
                                "Remove Fade"
                            } else {
                                "Remove Transition"
                            }
                            .into(),
                        ),
                        body: WriteBody::Commands(vec![command]),
                        expected_revision: Some(self.index.revision),
                    }],
                    ..Plan::default()
                }
            }
            "delete" if self.key_selection_active || !keys.is_empty() => {
                self.key_selection_active = true;
                if keys.is_empty() {
                    return Ok(Plan::default());
                }
                edit(KeyframeCommand::Delete { key_ids: keys }.into())
            }
            "delete" => {
                let roots: Vec<_> = self
                    .index
                    .layers
                    .iter()
                    .filter(|l| {
                        if !layers.contains(&l.id) {
                            return false;
                        }
                        let mut next = l.group.as_deref();
                        let mut seen = std::collections::HashSet::new();
                        while let Some(id) = next {
                            if !seen.insert(id) {
                                break;
                            }
                            if layers.iter().any(|selected| selected == id) {
                                return false;
                            }
                            next = self.index.layer(id).and_then(|g| g.group.as_deref());
                        }
                        true
                    })
                    .collect();
                let skipped: Vec<_> = roots
                    .into_iter()
                    .filter(|l| {
                        self.index
                            .expanded_ids(&[l.id.clone()])
                            .iter()
                            .filter_map(|id| self.index.layer(id))
                            .any(|child| child.effectively_locked)
                    })
                    .collect();
                let locked = skipped.len();
                let groups = skipped.iter().all(|l| l.kind == "group");
                let mut plan = if layers.iter().any(|id| self.index.layer(id).is_some()) {
                    edit(
                        LayerCommand::DeleteAt {
                            layer_ids: layers,
                            time,
                        }
                        .into(),
                    )
                } else {
                    Plan::default()
                };
                if locked > 0 {
                    plan.effects.push(HostEffect::Status(format!(
                        "Skipped {locked} locked {}",
                        match (groups, locked) {
                            (true, 1) => "group",
                            (true, _) => "groups",
                            (false, 1) => "layer",
                            (false, _) => "layers",
                        }
                    )));
                }
                plan
            }
            "deselect" => {
                self.edit_tool = crate::EditTool::Selection;
                self.key_selection_active = false;
                Plan {
                    calls: vec![SelectionCall::Clear.user().into()],
                    ..Plan::default()
                }
            }
            "undo" | "redo" => Plan {
                calls: vec![
                    if id == "undo" {
                        HistoryCall::Undo
                    } else {
                        HistoryCall::Redo
                    }
                    .into(),
                ],
                ..Plan::default()
            },
            "timeline.addMarker" => self.add_marker(time)?,
            // Toggle Hold Keyframe (After Effects): hold every selected key,
            // or release them when all of them hold.
            "timeline.toggleHold" => {
                if keys.is_empty() {
                    return Ok(Plan::default());
                }
                let hold = !self.all_held(&keys);
                edit(
                    KeyframeCommand::SetHold {
                        key_ids: keys,
                        hold,
                    }
                    .into(),
                )
            }
            tool if tool.starts_with("timeline.tool:") => {
                let name = &tool["timeline.tool:".len()..];
                self.edit_tool = crate::EditTool::ALL
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(t, _)| *t)
                    .ok_or(ModelError::MissingHost("unknown edit tool"))?;
                Plan::default()
            }
            "timeline.toggleSnapping" => {
                self.track_snap_disabled = !self.track_snap_disabled;
                Plan::default()
            }
            "timeline.removeMarker" => {
                if let Some(marker) = self.index.marker_at(time) {
                    let id = marker
                        .id
                        .clone()
                        .ok_or(ModelError::MissingHost("legacy marker without id"))?;
                    edit(MarkerCommand::RemoveById { comp: None, id }.into())
                } else {
                    Plan::default()
                }
            }
            "timeline.revealAll" | "toggleLayerStrips" => {
                self.reveal_property(
                    if id == "toggleLayerStrips" { 'm' } else { '*' },
                    false,
                    millis,
                    facts,
                )?;
                Plan::default()
            }
            // Hide Shy Layers (After Effects' master shy switch).
            "timeline.toggleShy" => {
                self.hide_shy = !self.hide_shy;
                self.rebuild_rows();
                Plan::default()
            }
            // The Shy switch of the selected layers: all on unless all are.
            "toggleShy" => {
                let selected: Vec<_> = layers
                    .iter()
                    .filter_map(|id| self.index.layer(id))
                    .filter(|l| !l.effectively_locked)
                    .collect();
                let shy = !selected.iter().all(|l| l.shy);
                if selected.is_empty() {
                    Plan::default()
                } else {
                    Plan::edit(
                        selected
                            .iter()
                            .map(|l| {
                                LayerCommand::Set {
                                    layer_id: l.id.clone(),
                                    patch: LayerPatch {
                                        shy: Some(shy),
                                        ..Default::default()
                                    },
                                }
                                .into()
                            })
                            .collect(),
                        self.index.revision,
                    )
                }
            }
            "timeline.mode:layers" => {
                self.mode = TimelineMode::Layers;
                Plan::default()
            }
            "timeline.mode:tracks" => {
                self.mode = TimelineMode::Tracks;
                Plan::default()
            }
            "timeline.toggleMode" => {
                self.mode = if self.mode == TimelineMode::Layers {
                    TimelineMode::Tracks
                } else {
                    TimelineMode::Layers
                };
                Plan::default()
            }
            "graph" => {
                self.graph_display = if self.graph_display == GraphDisplay::DopeSheet {
                    GraphDisplay::Graph
                } else {
                    GraphDisplay::DopeSheet
                };
                Plan::default()
            }
            "graph.value" => {
                self.graph_mode = GraphMode::Value;
                Plan::default()
            }
            "graph.speed" => {
                self.graph_mode = GraphMode::Speed;
                Plan::default()
            }
            "graph.combined" => {
                self.graph_display = if self.graph_display == GraphDisplay::Combined {
                    GraphDisplay::Graph
                } else {
                    GraphDisplay::Combined
                };
                Plan::default()
            }
            // The timeline zooms to the keys of the curves in the graph.
            "graph.fit" => Plan {
                effects: self
                    .graph_span()
                    .map(|(first, last)| HostEffect::FitCurves(first, last))
                    .into_iter()
                    .collect(),
                ..Plan::default()
            },
            "graph.merge" => {
                self.graph_merge = !self.graph_merge;
                Plan::default()
            }
            "frameView" => Plan {
                effects: vec![HostEffect::FrameComposition],
                ..Plan::default()
            },
            _ => return Err(ModelError::UnknownCommand(id.into())),
        })
    }
    /// Option-Shift-P, S, R, T or A (After Effects): a keyframe at the
    /// playhead on that transform property of the selected layers, or its
    /// removal when every one of its channels has a key there. The property
    /// shows in the timeline afterwards.
    pub fn key_property(&mut self, property: &str, time: f64) -> Result<Plan, ModelError> {
        let wanted = |path: &str| match property {
            "p" => path.starts_with("position."),
            "s" => path.starts_with("scale."),
            "r" => path == "rotation" || path.starts_with("rotation."),
            "t" => path == "opacity",
            "a" => path.starts_with("anchor."),
            _ => false,
        };
        if !["p", "s", "r", "t", "a"].contains(&property) {
            return Err(ModelError::UnknownCommand(property.into()));
        }
        let targets = self.channels_of_selection(|_, path, _| wanted(path));
        if targets.is_empty() {
            return Ok(Plan::default());
        }
        self.property_diamond(&targets, time)
    }
    /// Add Keyframe: a key at the playhead on the selected property (the
    /// properties of the selected keys, else every animated property) of the
    /// selected layers. Channels that have a key there are left alone.
    pub fn add_keyframes(&mut self, time: f64) -> Result<Plan, ModelError> {
        finite(time)?;
        let property = self.selection.channel().map(str::to_owned);
        let keyed: Vec<(String, String)> = self
            .selection
            .keys()
            .iter()
            .filter_map(|id| self.index.key(id))
            .map(|(l, c, _)| (l.id.clone(), c.path.clone()))
            .collect();
        let targets = self.channels_of_selection(|layer, path, animated| match &property {
            Some(chosen) => path == chosen || path.starts_with(&format!("{chosen}.")),
            None if !keyed.is_empty() => keyed.iter().any(|(l, c)| l == layer && c == path),
            None => animated,
        });
        let half = 0.5 / self.index.fps;
        let commands: Vec<Command> = targets
            .iter()
            .filter(|(l, c)| {
                let from = self.index.layer(l).map_or(0., |l| l.from);
                !self
                    .index
                    .channel(l, c)
                    .is_some_and(|c| c.keys.iter().any(|k| (from + k.time - time).abs() < half))
            })
            .map(|(l, c)| {
                KeyframeCommand::Add {
                    layer_id: l.clone(),
                    channel: c.clone(),
                    time,
                    key_id: new_id("K"),
                    value: None,
                }
                .into()
            })
            .collect();
        if commands.is_empty() {
            return Ok(Plan::default());
        }
        Ok(Plan::edit(commands, self.index.revision))
    }
    /// `(layer, channel)` of the selected, unlocked layers whose channel
    /// `wanted(layer, path, has keys)` accepts.
    fn channels_of_selection(
        &self,
        wanted: impl Fn(&str, &str, bool) -> bool,
    ) -> Vec<(String, String)> {
        self.index
            .layers
            .iter()
            .filter(|l| self.selection.contains(&l.id) && !l.effectively_locked)
            .flat_map(|l| {
                l.channels
                    .iter()
                    .filter(|c| {
                        l.channel_available(&c.path) && wanted(&l.id, &c.path, !c.keys.is_empty())
                    })
                    .map(|c| (l.id.clone(), c.path.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    /// Every one of these keys holds its value to the next key: the legacy
    /// `hold` flag, or the outgoing interpolation the kernel writes now.
    /// The named ease each key has (`None`: hold, custom or flat), read as
    /// the kernel names what Set Easing wrote.
    fn key_presets(&self, keys: &[String]) -> Vec<Option<&'static str>> {
        let mut tracks: std::collections::HashMap<(String, String), Vec<pm_model::Value>> =
            std::collections::HashMap::new();
        keys.iter()
            .map(|id| {
                let (layer, channel, key) = self.index.key(id)?;
                let at = channel.keys.iter().position(|k| k.id == key.id)?;
                let stored = tracks
                    .entry((layer.id.clone(), channel.path.clone()))
                    .or_insert_with(|| channel.keys.iter().map(|k| k.stored.to_value()).collect());
                pm_kernel::commands::keyframes::ease_preset(stored, at)
            })
            .collect()
    }
    fn all_held(&self, keys: &[String]) -> bool {
        keys.iter().all(|id| {
            self.index.key(id).is_some_and(|(_, _, k)| {
                crate::snapshot::boolean(k.stored.get("hold"))
                    || k.stored
                        .get("outInterp")
                        .is_some_and(|n| n.to_value().as_str() == Some("hold"))
            })
        })
    }
    pub fn add_marker(&self, time: f64) -> Result<Plan, ModelError> {
        finite(time)?;
        let time = frame(time, self.index.fps).clamp(0., self.index.duration);
        if self.index.marker_at(time).is_some() {
            return Ok(Plan::default());
        }
        Ok(Plan::edit(
            vec![
                MarkerCommand::Add {
                    comp: None,
                    id: new_id("M"),
                    time,
                    name: None,
                }
                .into(),
            ],
            self.index.revision,
        ))
    }
    /// Context menu identity is resolved from the current publication, never a cached index.
    pub fn context_keys(&self, hit: &[String]) -> Vec<String> {
        if hit.iter().any(|id| self.selection.keys().contains(id)) {
            self.selection.keys().to_vec()
        } else {
            hit.to_vec()
        }
    }
    pub fn copy_keys(&self, ids: &[String]) -> Vec<ClipboardTrack> {
        let first = ids
            .iter()
            .filter_map(|id| self.index.key(id).map(|(l, _, k)| l.from + k.time))
            .reduce(f64::min)
            .unwrap_or(0.);
        let mut tracks: Vec<ClipboardTrack> = Vec::new();
        for id in ids {
            if let Some((l, c, k)) = self.index.key(id) {
                let pos = tracks
                    .iter()
                    .position(|t| t.layer_id == l.id && t.channel == c.path)
                    .unwrap_or_else(|| {
                        tracks.push(ClipboardTrack {
                            layer_id: l.id.clone(),
                            channel: c.path.clone(),
                            keys: Vec::new(),
                        });
                        tracks.len() - 1
                    });
                tracks[pos].keys.push(CopiedKey {
                    at: l.from + k.time - first,
                    key: k.stored.to_value(),
                });
            }
        }
        tracks
    }
    pub fn paste_keys(
        &self,
        layer: &str,
        channel: &str,
        time: f64,
        copied: &[CopiedKey],
    ) -> Result<Plan, ModelError> {
        self.require_channel(layer, channel)?;
        finite(time)?;
        let ids: Vec<_> = copied.iter().map(|_| new_id("K")).collect();
        let selected: Vec<_> = copied
            .iter()
            .zip(&ids)
            .filter(|(k, _)| {
                let t = frame(time + k.at, self.index.fps);
                t >= 0. && t <= self.index.duration
            })
            .map(|(_, id)| id.clone())
            .collect();
        let mut p = Plan::edit(
            vec![
                KeyframeCommand::Paste {
                    layer_id: layer.into(),
                    channel: channel.into(),
                    time,
                    keys: copied.to_vec(),
                    new_ids: ids,
                }
                .into(),
            ],
            self.index.revision,
        );
        p.calls.push(SelectionCall::Keys(selected).into());
        Ok(p)
    }
    /// v1's separateDimensionsItem: the selection when the row's layer is
    /// in it, else that layer; locked layers stay as they are.
    fn separate_targets(&self, layer: &str) -> Vec<String> {
        let selected = self.selection.layers();
        let ids: Vec<String> = if selected.iter().any(|id| id == layer) {
            selected.to_vec()
        } else {
            vec![layer.to_owned()]
        };
        ids.into_iter()
            .filter(|id| self.index.layer(id).is_some_and(|l| !l.effectively_locked))
            .collect()
    }
    pub fn require_channel(&self, layer: &str, channel: &str) -> Result<(), ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        if l.effectively_locked {
            return Err(ModelError::Locked(layer.into()));
        }
        self.index
            .channel(layer, channel)
            .ok_or_else(|| ModelError::UnknownId(channel.into()))?;
        Ok(())
    }
    /// Current boolean value of `switch` on `layer`.
    pub fn switch_value(&self, layer: &str, switch: LayerSwitch) -> Option<bool> {
        let l = self.index.layer(layer)?;
        Some(match switch {
            LayerSwitch::Visible => l.visible,
            LayerSwitch::Locked => l.locked,
            LayerSwitch::Solo => l.solo,
            LayerSwitch::ThreeD => l.three_d,
            LayerSwitch::Shy => l.shy,
        })
    }

    /// Build a patch setting `switch` to `value`.
    pub fn switch_patch(switch: LayerSwitch, value: bool) -> LayerPatch {
        match switch {
            LayerSwitch::Visible => LayerPatch {
                visible: Some(value),
                ..Default::default()
            },
            LayerSwitch::Locked => LayerPatch {
                locked: Some(value),
                ..Default::default()
            },
            LayerSwitch::Solo => LayerPatch {
                solo: Some(value),
                ..Default::default()
            },
            LayerSwitch::ThreeD => LayerPatch {
                three_d: Some(value),
                ..Default::default()
            },
            LayerSwitch::Shy => LayerPatch {
                shy: Some(value),
                ..Default::default()
            },
        }
    }

    /// Whether `layer` can accept `switch` in a bulk / multi-selection edit.
    pub fn switch_eligible(&self, layer: &str, switch: LayerSwitch) -> bool {
        let Some(l) = self.index.layer(layer) else {
            return false;
        };
        match switch {
            LayerSwitch::Locked => true,
            LayerSwitch::ThreeD => !l.effectively_locked && l.can_be_3d(),
            LayerSwitch::Visible | LayerSwitch::Solo | LayerSwitch::Shy => !l.effectively_locked,
        }
    }

    /// Set `switch` to `value` on every eligible layer in `layers` as one undoable edit.
    pub fn set_switch(
        &self,
        layers: &[String],
        switch: LayerSwitch,
        value: bool,
    ) -> Result<Plan, ModelError> {
        let multi = layers.len() > 1;
        let patch = Self::switch_patch(switch, value);
        let mut commands = Vec::new();
        for id in layers {
            let l = self
                .index
                .layer(id)
                .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
            if multi && !self.switch_eligible(id, switch) {
                continue;
            }
            if multi && self.switch_value(id, switch) == Some(value) {
                continue;
            }
            commands.push(
                LayerCommand::Set {
                    layer_id: l.id.clone(),
                    patch: patch.clone(),
                }
                .into(),
            );
        }
        if commands.is_empty() {
            return Ok(Plan::default());
        }
        Ok(Plan::edit(commands, self.index.revision))
    }

    pub fn switch(&self, layer: &str, switch: LayerSwitch) -> Result<Plan, ModelError> {
        let current = self
            .switch_value(layer, switch)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        let target = !current;
        let selected = self.selection.layers();
        let ids: Vec<String> = if selected.len() > 1 && selected.iter().any(|id| id == layer) {
            selected.to_vec()
        } else {
            vec![layer.to_owned()]
        };
        self.set_switch(&ids, switch, target)
    }
    pub fn parent(&self, ids: &[String], parent: Option<String>, time: f64) -> Plan {
        let commands = ids
            .iter()
            .map(|id| {
                LayerCommand::Set {
                    layer_id: id.clone(),
                    patch: LayerPatch {
                        parent: Some(ParentChange {
                            parent: parent.clone(),
                            time,
                        }),
                        ..Default::default()
                    },
                }
                .into()
            })
            .collect();
        Plan {
            calls: vec![Call::Transact {
                label: Some("Set Parent".into()),
                body: WriteBody::Commands(commands),
                expected_revision: None,
            }],
            effects: Vec::new(),
            ..Plan::default()
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerSwitch {
    Visible,
    Locked,
    Solo,
    ThreeD,
    Shy,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardTrack {
    pub layer_id: String,
    pub channel: String,
    pub keys: Vec<CopiedKey>,
}

#[derive(Clone, Debug)]
pub enum Draft {
    Name {
        layer_id: String,
        text: String,
    },
    Property {
        targets: Vec<(String, String)>,
        value: Value,
        time: f64,
    },
    Keys {
        ids: Vec<String>,
        time: Option<f64>,
        value: Option<Value>,
    },
}
impl Draft {
    /// Enter/blur commit; Escape discards. Field focus routing belongs to the shell.
    pub fn finish(self, commit: bool, revision: u64) -> Plan {
        if !commit {
            return Plan::default();
        }
        let commands = match self {
            Self::Name { layer_id, text } => {
                if text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![Command::SetLayerName {
                        layer_id,
                        name: text,
                    }]
                }
            }
            Self::Property {
                targets,
                value,
                time,
            } => targets
                .into_iter()
                .map(|(layer_id, channel)| {
                    PropertyCommand::Set(SetProperty::new(layer_id, channel, value.clone(), time))
                        .into()
                })
                .collect(),
            Self::Keys { ids, time, value } => ids
                .into_iter()
                .map(|key_id| {
                    KeyframeCommand::Edit {
                        key_id,
                        time,
                        value: value.clone(),
                    }
                    .into()
                })
                .collect(),
        };
        if commands.is_empty() {
            Plan::default()
        } else {
            Plan::edit(commands, revision)
        }
    }
}

#[derive(Clone, Debug)]
pub enum DragKind {
    /// Scoped pro-media edits use the same actor replacement gesture lifecycle.
    ProEdit(pm_kernel::commands::pro_edit::ProEditCommand),
    /// The Ripple, Rolling and Slide tools in Layers mode: the layer-level
    /// edits the keys run, with the edge time (or slide) following the drag.
    Timing(pm_kernel::TimingCommand),
    Layers {
        ids: Vec<String>,
    },
    /// Option-drag: copies of `ids` (`new_ids`, in the same order) follow the
    /// pointer and the originals stay. One Undo step.
    DuplicateLayers {
        ids: Vec<String>,
        new_ids: Vec<String>,
    },
    TrimIn {
        ids: Vec<String>,
    },
    TrimOut {
        ids: Vec<String>,
    },
    QuickOffsetKeys {
        groups: Vec<Vec<String>>,
        time: f64,
    },
    TextStagger {
        band: crate::stagger::Band,
        end: bool,
        time: f64,
    },
    QuickOffsetLayers {
        ids: Vec<String>,
    },
    Keys {
        ids: Vec<String>,
    },
    GraphKeys {
        ids: Vec<String>,
    },
    StretchKeys {
        ids: Vec<String>,
        anchor_time: f64,
        grabbed_time: f64,
    },
    TransformKeys {
        ids: Vec<String>,
        anchor_time: f64,
        anchor_value: f64,
    },
    Handle {
        id: String,
        side: EaseSide,
    },
    Property {
        targets: Vec<(String, String)>,
        time: f64,
    },
    WorkIn,
    WorkOut,
    WorkMove,
    /// An audio clip's level line: `start` is the gain's place on the fader
    /// law (0 floor, 1 top) when the drag began; the sample's `value_delta`
    /// moves it (a share of the clip's height).
    Gain {
        layer: String,
        start: f64,
    },
    /// A transition's wedge (an audio clip's fade): its duration follows
    /// the drag, from the clip's end to the pointer.
    Transition {
        layer: String,
        edge: crate::render::Edge,
    },
}
impl DragKind {
    pub fn is_strip_trim(&self) -> bool {
        matches!(self, Self::TrimIn { .. } | Self::TrimOut { .. })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DragSample {
    pub time_delta: f64,
    pub value_delta: f64,
    pub time_scale: f64,
    pub value_scale: f64,
    /// A transform drag's fixed point `(time, value)` when it is not the
    /// one the drag began with (Cmd scales about the box's centre, v1).
    pub anchor: Option<(f64, f64)>,
    pub handle_time: f64,
    pub handle_value: f64,
    pub modifiers: Modifiers,
    pub acquire_tolerance: f64,
    pub release_tolerance: f64,
    pub playhead: f64,
}
#[derive(Clone, Debug)]
pub struct Drag {
    work_duration: f64,
    /// Tracks mode: the lanes shown when the drag began, written as tracks
    /// ahead of a pro edit so it keeps every clip on its track.
    lanes: Option<Vec<(String, u32)>>,
    pub kind: DragKind,
    pub start: Arc<TimelineIndex>,
    pub snap: DirectionalSnap,
    fixed: Option<Vec<Target>>,
    anchors: Vec<Anchor>,
    pub key_snap: Option<crate::snap::GroupLock>,
    track_snap: Option<bool>,
    pub property_values: std::collections::HashMap<(String, String), f64>,

    /// A key drag in the Speed Graph: each key side's speed and influence at
    /// the press. A vertical drag there changes speeds (as v1 does), never
    /// the keys' values.
    speed_eases: Option<Vec<(String, EaseSide, TemporalEase)>>,
    /// A handle drag: each target key's id (expanded across linked scale /
    /// combined position axes), its layer-local time, value, dragged-side
    /// neighbour span `dt`, opposite side's ease, and normalized chord
    /// directions `(side_dir, opp_dir)` for combined Speed Graph handle drags.
    handle_keys: Vec<(String, f64, f64, Option<f64>, Option<TemporalEase>, f64, f64)>,
    /// True when the active `DragKind::Handle` is in Speed Graph space
    /// (explicit Speed Graph mode or combined Position Speed curve).
    handle_speed: bool,
    /// Locked layers a move or trim left where they were (After Effects skips
    /// locked layers in a mixed selection instead of refusing the gesture).
    skipped_locked: usize,
    trim_limit: f64,
}
/// Each side's ease of the keys `ids` as the graph shows it: the speed (value
/// units per second) and influence (percent) of its direction handle, so a
/// linear side reads its real chord speed, not a stored zero.
fn key_eases(index: &TimelineIndex, ids: &[String]) -> Vec<(String, EaseSide, TemporalEase)> {
    let mut out = vec![];
    for id in ids {
        let Some((_, channel, _)) = index.key(id) else {
            continue;
        };
        let Some(i) = channel.keys.iter().position(|k| &k.id == id) else {
            continue;
        };
        let stored: Vec<_> = channel.keys.iter().map(|k| k.stored.to_value()).collect();
        let points = pm_kernel::commands::keyframes::handle_points(&stored);
        let key = &channel.keys[i];
        let Some(value) = key.value.as_f64() else {
            continue;
        };
        for (side, neighbour) in [
            (EaseSide::In, i.checked_sub(1)),
            (
                EaseSide::Out,
                Some(i + 1).filter(|n| *n < channel.keys.len()),
            ),
        ] {
            let slot = if side == EaseSide::In { 0 } else { 1 };
            let (Some((t, v)), Some(n)) = (points.get(i).and_then(|p| p[slot]), neighbour) else {
                continue;
            };
            let reach = t - key.time;
            let span = (channel.keys[n].time - key.time).abs();
            if reach.abs() < 1e-9 || span < 1e-9 {
                continue;
            }
            out.push((
                id.clone(),
                side,
                TemporalEase {
                    speed: (v - value) / reach,
                    influence: (reach.abs() / span * 100.).clamp(0.1, 100.),
                },
            ));
        }
    }
    out
}
impl Drag {
    /// Tracks mode: the command writing the lanes shown when the drag began,
    /// which goes ahead of each step of a pro edit.
    pub fn pin_command(&self) -> Option<Command> {
        self.lanes.as_ref().map(|lanes| {
            pm_kernel::TimingCommand::PinTracks {
                lanes: lanes.clone(),
            }
            .into()
        })
    }
    /// Captures identities and baselines once; never keeps a writer lease itself.
    pub fn begin(model: &TimelineModel, mut kind: DragKind) -> Result<Self, ModelError> {
        let index = &model.index;
        let mut skipped_locked = 0;
        match &mut kind {
            DragKind::ProEdit(command) => {
                use pm_kernel::commands::pro_edit::ProEditCommand as Pro;
                if !matches!(
                    command,
                    Pro::Move { .. }
                        | Pro::Trim { .. }
                        | Pro::Slip { .. }
                        | Pro::RippleTrim { .. }
                        | Pro::Roll { .. }
                        | Pro::Slide { .. }
                ) {
                    return Err(ModelError::InvalidGesture);
                }
            }
            DragKind::Timing(command) => {
                use pm_kernel::TimingCommand as T;
                if !matches!(
                    command,
                    T::RippleTrim { .. } | T::Roll { .. } | T::Slide { .. }
                ) {
                    return Err(ModelError::InvalidGesture);
                }
            }
            DragKind::Layers { ids }
            | DragKind::TrimIn { ids }
            | DragKind::TrimOut { ids }
            | DragKind::QuickOffsetLayers { ids } => {
                for id in ids.iter() {
                    index
                        .layer(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                }
                *ids = index.expanded_ids(ids);
                // A mixed selection moves or trims its unlocked layers; the
                // locked ones stay put. Only a selection with nothing
                // unlocked left refuses the gesture.
                let first_locked = ids
                    .iter()
                    .find(|id| index.layer(id).unwrap().effectively_locked)
                    .cloned();
                if let Some(locked) = first_locked {
                    let before = ids.len();
                    ids.retain(|id| !index.layer(id).unwrap().effectively_locked);
                    if ids.is_empty() {
                        return Err(ModelError::Locked(locked));
                    }
                    skipped_locked = before - ids.len();
                }
            }
            DragKind::Transition { layer, edge } => {
                let l = index
                    .layer(layer)
                    .ok_or_else(|| ModelError::UnknownId(layer.clone()))?;
                if l.effectively_locked {
                    return Err(ModelError::Locked(layer.clone()));
                }
                if l.transitions[*edge as usize]
                    .as_ref()
                    .is_none_or(|t| t.missing)
                {
                    return Err(ModelError::InvalidGesture);
                }
            }
            DragKind::DuplicateLayers { ids, new_ids } => {
                if ids.is_empty() || ids.len() != new_ids.len() {
                    return Err(ModelError::InvalidGesture);
                }
                for id in ids.iter() {
                    let layer = index
                        .layer(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                    if layer.effectively_locked {
                        return Err(ModelError::Locked(id.clone()));
                    }
                }
            }
            DragKind::Keys { ids }
            | DragKind::GraphKeys { ids }
            | DragKind::StretchKeys { ids, .. }
            | DragKind::TransformKeys { ids, .. } => {
                for id in ids {
                    let (l, _, _) = index
                        .key(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                    if l.effectively_locked {
                        return Err(ModelError::Locked(l.id.clone()));
                    }
                }
            }
            DragKind::TextStagger { band, time, .. } => {
                finite(*time)?;
                for key in ["delay", "duration", "stagger"] {
                    let path = format!("ta.{}.{key}", band.animator);
                    if key == "duration" || index.channel(&band.layer, &path).is_some() {
                        model.require_channel(&band.layer, &path)?;
                    }
                }
            }
            DragKind::QuickOffsetKeys { groups, time } => {
                finite(*time)?;
                if groups.len() < 2 {
                    return Err(ModelError::InvalidGesture);
                }
                for id in groups.iter().flatten() {
                    let (l, _, _) = index
                        .key(id)
                        .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                    if l.effectively_locked {
                        return Err(ModelError::Locked(l.id.clone()));
                    }
                }
            }
            DragKind::Handle { id, .. } => {
                let (l, _, _) = index
                    .key(id)
                    .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                if l.effectively_locked {
                    return Err(ModelError::Locked(l.id.clone()));
                }
            }
            DragKind::Property { targets, time } => {
                finite(*time)?;
                for (l, c) in targets {
                    model.require_channel(l, c)?;
                }
            }
            DragKind::Gain { layer, start } => {
                finite(*start)?;
                let l = index
                    .layer(layer)
                    .ok_or_else(|| ModelError::UnknownId(layer.clone()))?;
                if l.effectively_locked {
                    return Err(ModelError::Locked(l.id.clone()));
                }
                if l.kind != "audio" {
                    return Err(ModelError::UnknownId(layer.clone()));
                }
            }
            _ => {}
        }
        let mut anchors = Vec::new();
        let mut trim_limit = 0.;
        match &kind {
            DragKind::Layers { ids } | DragKind::DuplicateLayers { ids, .. } => {
                for id in ids {
                    let l = index.layer(id).unwrap();
                    if l.kind != "group" {
                        anchors.extend([
                            Anchor {
                                time: l.from,
                                edge: Some(Edge::In),
                            },
                            Anchor {
                                time: l.from + l.duration,
                                edge: Some(Edge::Out),
                            },
                        ]);
                    }
                }
            }
            DragKind::TrimIn { ids } | DragKind::TrimOut { ids } => {
                let is_in = matches!(kind, DragKind::TrimIn { .. });
                trim_limit = if is_in {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                };
                let frame = 1. / index.fps;
                for id in ids {
                    let l = index.layer(id).unwrap();
                    let (from, duration) = index.strip_span(l);
                    anchors.push(Anchor {
                        time: if is_in { from } else { from + duration },
                        edge: Some(if is_in { Edge::In } else { Edge::Out }),
                    });
                    if l.kind == "group" {
                        // Capture the limit from the starting geometry once.
                        // Each unlocked member keeps at least one frame.
                        for member in index.expanded_ids(std::slice::from_ref(id)) {
                            let child = index.layer(&member).unwrap();
                            if child.id == *id || child.effectively_locked {
                                continue;
                            }
                            let (start, length) = index.strip_span(child);
                            trim_limit = if is_in {
                                trim_limit.min(start + length - frame - from)
                            } else {
                                trim_limit.max(start + frame - (from + duration))
                            };
                        }
                    }
                }
            }
            DragKind::Transition { layer, edge } => {
                let l = index.layer(layer).unwrap();
                let seconds = l.transitions[*edge as usize].as_ref().unwrap().duration;
                anchors.push(Anchor {
                    time: match edge {
                        crate::render::Edge::In => l.from + seconds,
                        crate::render::Edge::Out => l.from + l.duration - seconds,
                    },
                    edge: None,
                });
            }
            DragKind::Keys { ids } | DragKind::GraphKeys { ids } => {
                for id in ids {
                    let (l, _, k) = index.key(id).unwrap();
                    anchors.push(Anchor {
                        time: l.from + k.time,
                        edge: None,
                    });
                }
            }
            _ => {}
        }
        let track_snap = if model.mode == TimelineMode::Tracks
            && matches!(
                kind,
                DragKind::Layers { .. }
                    | DragKind::DuplicateLayers { .. }
                    | DragKind::TrimIn { .. }
                    | DragKind::TrimOut { .. }
            ) {
            Some(!model.track_snap_disabled)
        } else {
            None
        };
        let speed_eases = match &kind {
            DragKind::GraphKeys { ids }
                if model.graph_mode == GraphMode::Speed
                    || ids.iter().all(|id| {
                        index.key(id).is_some_and(|(l, c, _)| {
                            model.is_combined_position_speed(&l.id, &c.path)
                        })
                    }) =>
            {
                Some(key_eases(index, ids))
            }
            _ => None,
        };
        let (handle_keys, handle_speed) = match &kind {
            DragKind::Handle { id, side } => {
                let other = if *side == EaseSide::In {
                    EaseSide::Out
                } else {
                    EaseSide::In
                };
                let is_speed = model.graph_mode == GraphMode::Speed
                    || index.key(id).is_some_and(|(l, c, _)| {
                        model.is_combined_position_speed(&l.id, &c.path)
                    });
                let expanded = model.expand_graph_keys(std::slice::from_ref(id));
                // Compute per-axis chord deltas on the dragged side and opposite
                // side so a multi-axis Speed handle preserves the exact motion
                // path direction.
                let mut raw: Vec<(String, f64, f64, Option<f64>, Option<TemporalEase>, f64, f64)> =
                    Vec::new();
                for target_id in expanded {
                    let Some((_, channel, key)) = index.key(&target_id) else {
                        continue;
                    };
                    let Some(value) = key.value.as_f64() else {
                        continue;
                    };
                    let Some(idx) = channel.keys.iter().position(|k| k.id == target_id) else {
                        continue;
                    };
                    let neighbour_for = |s: EaseSide| -> Option<(f64, f64)> {
                        let n = match s {
                            EaseSide::In => idx.checked_sub(1).map(|i| &channel.keys[i]),
                            _ => channel.keys.get(idx + 1),
                        }?;
                        let dt = (n.time - key.time).abs();
                        let dv = match s {
                            EaseSide::In => value - n.value.as_f64()?,
                            _ => n.value.as_f64()? - value,
                        };
                        (dt > 1e-6).then_some((dt, dv / dt))
                    };
                    let side_info = neighbour_for(*side);
                    let opp_info = neighbour_for(other);
                    let opposite = key_eases(index, std::slice::from_ref(&target_id))
                        .into_iter()
                        .find(|(_, s, _)| *s == other)
                        .map(|(_, _, e)| e);
                    raw.push((
                        target_id,
                        key.time,
                        value,
                        side_info.map(|(dt, _)| dt),
                        opposite,
                        side_info.map_or(0., |(_, slope)| slope),
                        opp_info.map_or(0., |(_, slope)| slope),
                    ));
                }
                if is_speed && raw.len() > 1 {
                    let side_norm = raw.iter().map(|r| r.5 * r.5).sum::<f64>().sqrt();
                    let opp_norm = raw.iter().map(|r| r.6 * r.6).sum::<f64>().sqrt();
                    for r in &mut raw {
                        r.5 = if side_norm > 1e-6 {
                            r.5 / side_norm
                        } else if opp_norm > 1e-6 {
                            r.6 / opp_norm
                        } else {
                            0.
                        };
                        r.6 = if opp_norm > 1e-6 {
                            r.6 / opp_norm
                        } else {
                            r.5
                        };
                    }
                }
                (raw, is_speed)
            }
            _ => (Vec::new(), false),
        };
        let lanes = (model.mode == crate::TimelineMode::Tracks
            && matches!(
                kind,
                DragKind::ProEdit(_)
                    | DragKind::Timing(_)
                    | DragKind::TrimIn { .. }
                    | DragKind::TrimOut { .. }
            ))
        .then(|| crate::tracks::relative_lanes(index, &model.reveal));
        Ok(Self {
            lanes,
            handle_keys,
            handle_speed,
            skipped_locked,
            trim_limit,
            speed_eases,
            kind,
            start: index.clone(),
            work_duration: index.duration,
            snap: DirectionalSnap::default(),
            fixed: None,
            anchors,
            key_snap: None,
            track_snap,
            property_values: Default::default(),
        })
    }
    /// The gesture's undo name: v1's words (timeline.ts), in Title Case as
    /// the Edit menu shows every action.
    pub fn label(&self) -> String {
        let many = |n: usize, one: &str, more: &str| if n > 1 { more } else { one }.to_owned();
        match self.kind {
            DragKind::ProEdit(ref command) => {
                use pm_kernel::commands::pro_edit::ProEditCommand as Pro;
                match command {
                    Pro::Move { .. } => "Move Linked Clips",
                    Pro::Trim { .. } => "Trim Clips",
                    Pro::Slip { .. } => "Slip Edit",
                    Pro::RippleTrim { .. } => "Ripple Trim",
                    Pro::Roll { .. } => "Roll Edit",
                    Pro::Slide { .. } => "Slide Edit",
                    _ => "Edit Clips",
                }
                .into()
            }
            DragKind::Timing(ref command) => match command {
                pm_kernel::TimingCommand::RippleTrim { .. } => "Ripple Trim",
                pm_kernel::TimingCommand::Roll { .. } => "Roll Edit",
                _ => "Slide Layer",
            }
            .into(),
            DragKind::Layers { .. } => "Move Clip".into(),
            DragKind::DuplicateLayers { .. } => "Duplicate Layers".into(),
            DragKind::TrimIn { .. } | DragKind::TrimOut { .. } => "Trim Clip".into(),
            DragKind::TextStagger { end, .. } => if end {
                "Stretch Text Animation"
            } else {
                "Move Text Animation"
            }
            .into(),
            DragKind::QuickOffsetLayers { .. } => "Quick Offset Layers".into(),
            DragKind::QuickOffsetKeys { .. } => "Quick Offset Keyframes".into(),
            DragKind::Keys { ref ids } => many(ids.len(), "Move Keyframe", "Move Keyframes"),
            DragKind::GraphKeys { ref ids } => many(ids.len(), "Edit Curve", "Edit Keyframes"),
            DragKind::StretchKeys { .. } => "Stretch Keyframes".into(),
            DragKind::TransformKeys { .. } => "Scale Keyframes".into(),
            DragKind::Handle { .. } => "Adjust Easing".into(),
            // v1: "Edit <property>" (the row's name).
            DragKind::Property { ref targets, .. } => match targets.as_slice() {
                [(_, channel)] => format!("Edit {}", crate::property_label(channel)),
                _ => "Edit Properties".into(),
            },
            DragKind::WorkMove => "Move Work Area".into(),
            // The Mixer names an audio layer's level edit the same.
            DragKind::Gain { .. } => "Gain".into(),
            DragKind::Transition { .. } => "Transition Duration".into(),
            _ => "Work Area".into(),
        }
    }
    pub fn is_work_area(&self) -> bool {
        matches!(
            self.kind,
            DragKind::WorkIn | DragKind::WorkOut | DragKind::WorkMove
        )
    }
    /// Electron draws a guide for strip latches only. Key proximity locks have
    /// no guide, and work-area edits remain frame-only.
    pub fn snap_target(&self) -> Option<f64> {
        self.snap.lock.map(|lock| lock.target)
    }
    /// Trimming can clamp at a legal source/minimum-duration boundary. Only
    /// show the latch if an actual resulting edge still rests on its target.
    pub fn snap_target_in(&self, index: &TimelineIndex) -> Option<f64> {
        let target = self.snap_target()?;
        match &self.kind {
            DragKind::TrimIn { ids } | DragKind::TrimOut { ids } => {
                let is_in = matches!(self.kind, DragKind::TrimIn { .. });
                ids.iter()
                    .filter_map(|id| index.layer(id))
                    .any(|l| {
                        let edge = if is_in { l.from } else { l.from + l.duration };
                        (edge - target).abs() < 1e-9
                    })
                    .then_some(target)
            }
            _ => Some(target),
        }
    }
    pub fn commands(&mut self, sample: DragSample) -> Result<Vec<Command>, ModelError> {
        finite(sample.time_delta)?;
        finite(sample.value_delta)?;
        finite(sample.playhead)?;
        let is_key = matches!(
            self.kind,
            DragKind::Keys { .. } | DragKind::GraphKeys { .. }
        );
        let snapping = !self.is_work_area()
            && !(matches!(self.kind, DragKind::GraphKeys { .. }) && sample.time_delta == 0.)
            && self
                .track_snap
                .map_or(sample.modifiers.shift, |on| on != sample.modifiers.shift);
        if snapping && self.fixed.is_none() {
            self.fixed = Some(snap_targets(
                &self.start,
                self.moving_layers(),
                self.moving_keys(),
                is_key,
            ));
        }
        let mut targets = if snapping {
            self.fixed.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
        if is_key && snapping {
            // Electron keys prefer the playhead on equal distances. They do not
            // offer the strip-only exclusive out-edge at playhead + one frame.
            targets.insert(0, Target::at(sample.playhead));
        } else if snapping {
            targets.push(Target {
                time: sample.playhead,
                edge: None,
                weight: 1.5,
            });
            targets.push(Target {
                time: sample.playhead + 1. / self.start.fps,
                edge: Some(Edge::Out),
                weight: 1.5,
            });
        }
        let resolved = if matches!(
            self.kind,
            DragKind::Keys { .. } | DragKind::GraphKeys { .. }
        ) {
            if snapping {
                let anchors: Vec<_> = self.anchors.iter().map(|a| a.time).collect();
                let times: Vec<_> = targets
                    .iter()
                    .map(|t| (t.time * 1e6 + 0.5).floor() / 1e6)
                    .collect();
                let (delta, lock) = crate::snap::keyframe_group(
                    &anchors,
                    sample.time_delta,
                    &times,
                    sample.acquire_tolerance,
                    self.key_snap,
                    sample.release_tolerance,
                );
                self.key_snap = lock;
                delta
            } else {
                self.key_snap = None;
                sample.time_delta
            }
        } else {
            self.snap.resolve(
                &self.anchors,
                sample.time_delta,
                &targets,
                sample.release_tolerance,
                snapping,
            )
        };
        let delta = if self.snap.lock.is_some() {
            resolved
        } else {
            frame(resolved, self.start.fps)
        };
        let mut out = Vec::new();
        match &self.kind {
            DragKind::TextStagger { band, end, time } => {
                out.extend(band.commands(*end, sample.time_delta, self.start.fps, *time))
            }
            DragKind::QuickOffsetKeys { groups, time } => {
                let mut model = TimelineModel::default();
                model.index = self.start.clone();
                let plan = model.quick_offset_keys(groups, sample.time_delta, *time)?;
                for call in plan.calls {
                    if let pm_kernel::Call::Transact {
                        body: pm_kernel::WriteBody::Commands(commands),
                        ..
                    } = call
                    {
                        out.extend(commands);
                    }
                }
            }
            DragKind::ProEdit(command) => {
                use pm_kernel::commands::pro_edit::ProEditCommand as Pro;
                let mut command = command.clone();
                match &mut command {
                    Pro::Move { delta, .. }
                    | Pro::Trim { delta, .. }
                    | Pro::RippleTrim { delta, .. }
                    | Pro::Roll { delta, .. }
                    | Pro::Slide { delta, .. } => *delta = sample.time_delta,
                    // The content follows the pointer, as in Premiere, Final
                    // Cut and After Effects: dragging right shows earlier
                    // source frames (the kernel's delta is the source's).
                    Pro::Slip { delta, .. } => *delta = -sample.time_delta,
                    _ => return Err(ModelError::InvalidGesture),
                }
                out.push(command.into());
            }
            DragKind::Timing(command) => {
                use pm_kernel::TimingCommand as T;
                let mut command = command.clone();
                match &mut command {
                    // The stored time is the edge where the drag began.
                    T::RippleTrim { time, .. } | T::Roll { time, .. } => *time += delta,
                    T::Slide { delta: by, .. } => *by = delta,
                    _ => return Err(ModelError::InvalidGesture),
                }
                if delta != 0. {
                    out.push(command.into());
                }
            }

            DragKind::Layers { ids } => {
                let earliest = ids
                    .iter()
                    .filter_map(|id| self.start.layer(id))
                    .filter(|l| l.kind != "group")
                    .map(|l| l.from)
                    .reduce(f64::min)
                    .unwrap_or(0.);
                let clamp = |d: f64| {
                    if self.track_snap.is_some() {
                        d.max(-earliest)
                    } else {
                        d
                    }
                };
                for id in ids {
                    let l = self.start.layer(id).unwrap();
                    let (start, duration) = self.start.strip_span(l);
                    let moved = if self.snap.lock.is_some() {
                        start + clamp(delta)
                    } else {
                        let rounded = frame(start + sample.time_delta, self.start.fps);
                        if self.track_snap.is_some() {
                            start + clamp(rounded - start)
                        } else {
                            rounded
                        }
                    };
                    let from = if l.kind == "group" && l.duration > 0. {
                        l.from + (moved - start)
                    } else {
                        moved
                    };
                    out.push(
                        LayerCommand::Set {
                            layer_id: id.clone(),
                            patch: LayerPatch {
                                from: Some(from),
                                duration: (l.kind == "group" && l.duration == 0. && duration > 0.)
                                    .then_some(duration),
                                ..Default::default()
                            },
                        }
                        .into(),
                    );
                }
            }
            // The copies exist only in this gesture's writes: each step
            // makes them again from the start and places them.
            DragKind::DuplicateLayers { ids, new_ids } => {
                let first = self.start.layer(&ids[0]).unwrap();
                let step = if self.snap.lock.is_some() {
                    delta
                } else {
                    frame(first.from + sample.time_delta, self.start.fps) - first.from
                };
                let step = if self.track_snap.is_some() {
                    let earliest = self.start.expanded_ids(ids).iter().filter_map(|id| self.start.layer(id)).filter(|l| l.kind != "group").map(|l| l.from).reduce(f64::min).unwrap_or(0.);
                    step.max(-earliest)
                } else { step };
                out.push(
                    LayerCommand::Duplicate {
                        layer_ids: ids.clone(),
                        new_ids: new_ids.clone(),
                    }
                    .into(),
                );
                out.push(
                    TimingCommand::Move {
                        layer_ids: new_ids.clone(),
                        delta: step,
                    }
                    .into(),
                );
            }
            DragKind::TrimIn { ids } | DragKind::TrimOut { ids } => {
                let is_in = matches!(self.kind, DragKind::TrimIn { .. });
                let min_duration = 1. / self.start.fps;
                let earliest = if self.track_snap.is_some() {
                    0.
                } else {
                    f64::NEG_INFINITY
                };
                let delta = if is_in {
                    delta.min(self.trim_limit)
                } else {
                    delta.max(self.trim_limit)
                };
                let mut legacy_groups = Vec::new();
                let mut batch_by_time = HashMap::new();
                let mut batches: Vec<(f64, Vec<String>)> = Vec::new();
                for id in ids {
                    let l = self.start.layer(id).unwrap();
                    let (from, duration) = self.start.strip_span(l);
                    let end = from + duration;
                    let time = if is_in {
                        (from + delta).min(end - min_duration).max(earliest)
                    } else {
                        (end + delta).max(from + min_duration)
                    };
                    // A clip entirely before zero has no legal In target at zero.
                    if is_in && time >= end {
                        continue;
                    }
                    // A group's In/Out comes from its original members. Run
                    // group trims before trimming any member so their starting
                    // span matches `self.start.strip_span`.
                    if l.kind == "group" {
                        legacy_groups.push((time, vec![id.clone()]));
                        continue;
                    }
                    // Normalize signed zero and retain the first-seen order.
                    let key = if time == 0. { 0 } else { time.to_bits() };
                    let index = *batch_by_time.entry(key).or_insert_with(|| {
                        batches.push((time, Vec::new()));
                        batches.len() - 1
                    });
                    batches[index].1.push(id.clone());
                }
                out.extend(legacy_groups.into_iter().chain(batches).map(|(time, layer_ids)| {
                    if is_in {
                        TimingCommand::TrimIn { layer_ids, time }.into()
                    } else {
                        TimingCommand::TrimOut { layer_ids, time }.into()
                    }
                }));
            }
            DragKind::QuickOffsetLayers { ids } => {
                let roots: Vec<_> = ids
                    .iter()
                    .filter(|id| {
                        !self
                            .start
                            .layer(id)
                            .unwrap()
                            .group
                            .as_ref()
                            .is_some_and(|g| ids.contains(g))
                    })
                    .collect();
                let count = roots.len();
                for id in ids {
                    let mut root = id;
                    let mut seen = std::collections::HashSet::new();
                    while let Some(g) = self
                        .start
                        .layer(root)
                        .and_then(|l| l.group.as_ref())
                        .filter(|g| ids.contains(g))
                    {
                        if !seen.insert(root) {
                            break;
                        }
                        root = g;
                    }
                    let rank = roots.iter().position(|r| *r == root).unwrap_or(0);
                    let delta = if count > 1 {
                        sample.time_delta * rank as f64 / (count - 1) as f64
                    } else {
                        0.
                    };
                    let from = ((self.start.layer(id).unwrap().from + delta) * 1e6).round() / 1e6;
                    out.push(
                        LayerCommand::Set {
                            layer_id: id.clone(),
                            patch: LayerPatch {
                                from: Some(from),
                                ..Default::default()
                            },
                        }
                        .into(),
                    );
                }
            }
            DragKind::Keys { ids } => out.push(
                KeyframeCommand::Move {
                    key_ids: ids.clone(),
                    delta,
                }
                .into(),
            ),
            DragKind::GraphKeys { ids } => {
                // In the Speed Graph the vertical axis is speed: the keys
                // keep their values and each side's speed moves instead.
                let speeds = self.speed_eases.as_ref();
                out.push(
                    KeyframeCommand::GraphMove {
                        key_ids: ids.clone(),
                        delta,
                        value_delta: if speeds.is_some() {
                            0.
                        } else {
                            sample.value_delta
                        },
                    }
                    .into(),
                );
                for (id, side, ease) in speeds.into_iter().flatten() {
                    out.push(
                        KeyframeCommand::SetHandle {
                            key_id: id.clone(),
                            side: *side,
                            value: HandleValue::Ease(TemporalEase {
                                speed: ease.speed + sample.value_delta,
                                influence: ease.influence,
                            }),
                            split: false,
                        }
                        .into(),
                    );
                }
            }
            DragKind::StretchKeys {
                ids,
                anchor_time,
                grabbed_time,
            } => {
                let span = grabbed_time - anchor_time;
                if span.abs() < 1e-9 {
                    return Err(ModelError::InvalidGesture);
                }
                out.push(
                    KeyframeCommand::Transform {
                        key_ids: ids.clone(),
                        anchor_time: *anchor_time,
                        time_scale: (span + sample.time_delta) / span,
                        anchor_value: 0.,
                        value_scale: 1.,
                    }
                    .into(),
                );
            }
            DragKind::TransformKeys {
                ids,
                anchor_time,
                anchor_value,
            } => {
                finite(sample.time_scale)?;
                finite(sample.value_scale)?;
                let (anchor_time, anchor_value) =
                    sample.anchor.unwrap_or((*anchor_time, *anchor_value));
                finite(anchor_time)?;
                finite(anchor_value)?;
                out.push(
                    KeyframeCommand::Transform {
                        key_ids: ids.clone(),
                        anchor_time,
                        time_scale: sample.time_scale,
                        anchor_value,
                        value_scale: sample.value_scale,
                    }
                    .into(),
                );
            }
            DragKind::Handle { id, side } => {
                finite(sample.handle_time)?;
                finite(sample.handle_value)?;
                let (t, raw_v) = (sample.handle_time, sample.handle_value);
                let primary_value = self
                    .handle_keys
                    .first()
                    .map(|(_, _, kv, _, _, _, _)| *kv);
                let multi_speed = self.handle_speed && self.handle_keys.len() > 1;
                let targets: Vec<_> = if self.handle_keys.is_empty() {
                    vec![(id.clone(), 0., raw_v, None, None, 1., 1.)]
                } else {
                    self.handle_keys.clone()
                };
                for (target_id, key_time, key_value, neighbour_dt, opposite, side_dir, opp_dir) in
                    targets
                {
                    if self.handle_speed {
                        let Some(dt) = neighbour_dt.filter(|dt| *dt > 1e-6) else {
                            continue;
                        };
                        let reach = match side {
                            EaseSide::In => key_time - t,
                            _ => t - key_time,
                        };
                        let influence = (reach.max(0.) / dt * 100.).clamp(0.1, 100.);
                        let target_speed = if sample.modifiers.shift {
                            0.
                        } else if multi_speed {
                            raw_v.max(0.) * side_dir
                        } else {
                            raw_v
                        };
                        out.push(
                            KeyframeCommand::SetHandle {
                                key_id: target_id.clone(),
                                side: *side,
                                value: HandleValue::Ease(TemporalEase {
                                    speed: target_speed,
                                    influence,
                                }),
                                split: sample.modifiers.alt,
                            }
                            .into(),
                        );
                        if let Some(opposite) = opposite
                            && !sample.modifiers.alt
                        {
                            let opp_speed = if sample.modifiers.shift {
                                0.
                            } else if multi_speed {
                                raw_v.max(0.) * opp_dir
                            } else {
                                raw_v
                            };
                            out.push(
                                KeyframeCommand::SetHandle {
                                    key_id: target_id,
                                    side: if *side == EaseSide::In {
                                        EaseSide::Out
                                    } else {
                                        EaseSide::In
                                    },
                                    value: HandleValue::Ease(TemporalEase {
                                        speed: opp_speed,
                                        influence: opposite.influence,
                                    }),
                                    split: false,
                                }
                                .into(),
                            );
                        }
                        continue;
                    }
                    // For linked scale axes with a different base value, apply
                    // the same value offset from the key rather than snapping
                    // both curves to one absolute y.
                    let mut v = match primary_value {
                        Some(base) if target_id != *id => key_value + (raw_v - base),
                        _ => raw_v,
                    };
                    // ⇧ lays the handle flat (v1's `moveBezierHandle` flat side).
                    if sample.modifiers.shift {
                        v = key_value;
                    }
                    out.push(
                        KeyframeCommand::SetHandle {
                            key_id: target_id.clone(),
                            side: *side,
                            value: HandleValue::Point { t, v },
                            split: sample.modifiers.alt,
                        }
                        .into(),
                    );
                    // Unless ⌥ splits them, the opposite handle stays on the
                    // exact same clamped tangent speed as the dragged handle,
                    // keeping its own length (`mirroredBezierHandlePoint`).
                    if let (Some(dt), Some(opposite)) = (neighbour_dt, opposite)
                        && !sample.modifiers.alt
                        && dt > 1e-6
                    {
                        let reach = match side {
                            EaseSide::In => key_time - t,
                            _ => t - key_time,
                        };
                        let influence = (reach.max(0.) / dt * 100.).clamp(0.1, 100.);
                        let span = influence / 100. * dt;
                        let rise = match side {
                            EaseSide::In => key_value - v,
                            _ => v - key_value,
                        };
                        out.push(
                            KeyframeCommand::SetHandle {
                                key_id: target_id,
                                side: if *side == EaseSide::In {
                                    EaseSide::Out
                                } else {
                                    EaseSide::In
                                },
                                value: HandleValue::Ease(TemporalEase {
                                    speed: rise / span,
                                    influence: opposite.influence,
                                }),
                                split: false,
                            }
                            .into(),
                        );
                    }
                }
            }
            DragKind::Property { targets, time } => {
                for (l, c) in targets {
                    let channel = self.start.channel(l, c).unwrap();
                    let baseline = self
                        .property_values
                        .get(&(l.clone(), c.clone()))
                        .copied()
                        .or_else(|| {
                            if channel.keys.is_empty() && channel.expression.is_none() {
                                channel.value.as_f64()
                            } else {
                                None
                            }
                        })
                        .ok_or(ModelError::MissingHost("evaluated property baseline"))?;
                    let value = baseline + sample.value_delta;
                    if matches!(c.as_str(), "scale.x" | "scale.y" | "scale.z")
                        && targets
                            .iter()
                            .filter(|(layer, path)| {
                                layer == l
                                    && matches!(path.as_str(), "scale.x" | "scale.y" | "scale.z")
                            })
                            .count()
                            == 1
                    {
                        let three_d = self.start.layer(l).is_some_and(|layer| layer.three_d);
                        let mut peers = Vec::new();
                        for paired in ["scale.x", "scale.y", "scale.z"] {
                            if paired == c
                                || (paired == "scale.z" && !three_d)
                                || self.start.channel(l, paired).is_none()
                            {
                                continue;
                            }
                            let other = self
                                .property_values
                                .get(&(l.clone(), paired.into()))
                                .copied()
                                .or_else(|| self.start.channel(l, paired)?.value.as_f64())
                                .ok_or(ModelError::InvalidNumber)?;
                            peers.push((paired, other));
                        }
                        let mut model = TimelineModel::default();
                        model.index = self.start.clone();
                        for call in model
                            .scale_values(l, c, value, *time, baseline, &peers)?
                            .calls
                        {
                            if let Call::Transact {
                                body: WriteBody::Commands(commands),
                                ..
                            } = call
                            {
                                out.extend(commands);
                            }
                        }
                        continue;
                    }
                    out.push(
                        PropertyCommand::Set(SetProperty::new(l, c, Value::Number(value), *time))
                            .into(),
                    );
                }
            }
            DragKind::Transition { layer, edge } => {
                let l = self
                    .start
                    .layer(layer)
                    .ok_or(ModelError::UnknownId(layer.clone()))?;
                let seconds = l.transitions[*edge as usize]
                    .as_ref()
                    .map_or(0., |t| t.duration);
                let seconds = match edge {
                    crate::render::Edge::In => seconds + delta,
                    crate::render::Edge::Out => seconds - delta,
                };
                out.extend(transition_command(l, *edge, Some(seconds), sample.playhead));
            }
            DragKind::WorkIn | DragKind::WorkOut | DragKind::WorkMove => {
                let [a, b] = self.start.work;
                let frame = 1. / self.start.fps;
                let duration = self.start.duration;
                let (a, b) = match self.kind {
                    DragKind::WorkIn => (
                        (self::frame(a + sample.time_delta, self.start.fps))
                            .max(0.)
                            .min(b - frame),
                        b,
                    ),
                    DragKind::WorkOut => {
                        let end = self::frame(b + sample.time_delta, self.start.fps).max(a + frame);
                        self.work_duration = self.work_duration.max(end);
                        (a, end)
                    }
                    _ => {
                        let d = delta.max(-a).min(duration - b);
                        (a + d, b + d)
                    }
                };
                out.push(
                    CompCommand::Set {
                        patch: Box::new(CompPatch {
                            duration: matches!(self.kind, DragKind::WorkOut)
                                .then_some(self.work_duration),
                            work_area: Some((a, b)),
                            ..Default::default()
                        }),
                    }
                    .into(),
                );
            }
            DragKind::Gain { layer, start } => {
                let at = (start + sample.value_delta).clamp(0., 1.);
                let gain = crate::level::db_gain(crate::level::fader_db(at));
                let mut patch = pm_model::Map::new();
                patch.insert("gain", Value::Number(gain));
                out.push(
                    pm_kernel::ContentCommand::Set {
                        layer_id: layer.clone(),
                        patch,
                        time: sample.playhead,
                    }
                    .into(),
                );
            }
        }
        // Tracks mode: the lanes shown when the drag began go first, so the
        // edit keeps every clip on its track (an edge trim that freed room on
        // a lower lane let an untouched clip drop into it).
        if !out.is_empty()
            && let Some(pin) = self.pin_command()
        {
            out.insert(0, pin);
        }
        Ok(out)
    }
    fn moving_layers(&self) -> &[String] {
        match &self.kind {
            DragKind::Layers { ids }
            | DragKind::TrimIn { ids }
            | DragKind::TrimOut { ids }
            | DragKind::QuickOffsetLayers { ids } => ids,
            _ => &[],
        }
    }
    /// The one-line note for locked layers this gesture left alone.
    pub fn locked_note(&self) -> Option<HostEffect> {
        let n = self.skipped_locked;
        if n == 0 {
            return None;
        }
        let verb = match self.kind {
            DragKind::TrimIn { .. } | DragKind::TrimOut { .. } => "trim",
            _ => "move",
        };
        Some(HostEffect::Status(format!(
            "{n} locked {} didn't {verb}",
            if n == 1 { "layer" } else { "layers" }
        )))
    }
    /// The copies an Option-drag made, to select when it ends.
    pub fn copies(&self) -> Option<&[String]> {
        match &self.kind {
            DragKind::DuplicateLayers { new_ids, .. } => Some(new_ids),
            _ => None,
        }
    }
    fn moving_keys(&self) -> &[String] {
        match &self.kind {
            DragKind::Keys { ids }
            | DragKind::GraphKeys { ids }
            | DragKind::StretchKeys { ids, .. }
            | DragKind::TransformKeys { ids, .. } => ids,
            _ => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scrub {
    index: Arc<TimelineIndex>,
    snap: DirectionalSnap,
    targets: Option<Vec<Target>>,
    pub raw_time: f64,
}
impl Scrub {
    pub fn begin(index: Arc<TimelineIndex>, time: f64) -> Result<Self, ModelError> {
        finite(time)?;
        Ok(Self {
            index,
            snap: DirectionalSnap::new(time),
            targets: None,
            raw_time: time,
        })
    }
    pub fn update(
        &mut self,
        time: f64,
        shift: bool,
        tolerance: f64,
    ) -> Result<HostEffect, ModelError> {
        finite(time)?;
        self.raw_time = time;
        let raw = time.clamp(0., self.index.duration);
        if shift && self.targets.is_none() {
            self.targets = Some(
                playhead_targets(&self.index)
                    .into_iter()
                    .map(Target::at)
                    .collect(),
            );
        }
        let delta = self.snap.resolve(
            &[Anchor {
                time: 0.,
                edge: None,
            }],
            raw,
            self.targets.as_deref().unwrap_or_default(),
            tolerance,
            shift,
        );
        Ok(HostEffect::Seek(delta))
    }
    pub fn snap_target(&self) -> Option<f64> {
        self.snap.lock.map(|lock| lock.target)
    }
    pub fn targets_loaded(&self) -> bool {
        self.targets.is_some()
    }
    /// Modifier changes at rest re-evaluate the last pointer sample.
    pub fn modifiers(&mut self, shift: bool, tolerance: f64) -> Result<HostEffect, ModelError> {
        self.update(self.raw_time, shift, tolerance)
    }
}
fn snap_targets(
    index: &TimelineIndex,
    moving_layers: &[String],
    moving_keys: &[String],
    keys: bool,
) -> Vec<Target> {
    let mut out: Vec<_> = [0., index.duration, index.work[0], index.work[1]]
        .into_iter()
        .map(Target::at)
        .collect();
    out.extend(index.markers.iter().map(|m| Target::at(m.time)));
    for l in &index.layers {
        if moving_layers.contains(&l.id) {
            continue;
        }
        if keys || l.duration > 0. {
            out.extend([Target::at(l.from), Target::at(l.from + l.duration)]);
        }
        if keys {
            for c in &l.channels {
                for k in &c.keys {
                    if !moving_keys.contains(&k.id) {
                        let time = l.from + k.time;
                        out.push(Target::at(time));
                    }
                }
            }
        }
    }
    out
}
/// The command setting `layer`'s transition at `edge` to `seconds` (kept
/// within 0.02 s and the clip, as the kernel allows), or removing it with
/// `None`. An audio clip's fade stands in for its transition. `None` when
/// there is no transition there to edit.
pub fn transition_command(
    layer: &crate::Layer,
    edge: crate::render::Edge,
    seconds: Option<f64>,
    time: f64,
) -> Option<Command> {
    let t = layer.transitions[edge as usize].as_ref()?;
    let seconds = seconds.map(|s| s.clamp(0.02, layer.duration.clamp(0.02, 600.)));
    let pm_edge = match edge {
        crate::render::Edge::In => pm_kernel::commands::effects::Edge::In,
        crate::render::Edge::Out => pm_kernel::commands::effects::Edge::Out,
    };
    Some(if layer.kind == "audio" {
        let mut patch = pm_model::Map::new();
        patch.insert(
            if edge == crate::render::Edge::In {
                "fadeIn"
            } else {
                "fadeOut"
            },
            Value::Number(seconds.unwrap_or(0.)),
        );
        pm_kernel::ContentCommand::Set {
            layer_id: layer.id.clone(),
            patch,
            time,
        }
        .into()
    } else {
        match seconds {
            Some(seconds) => pm_kernel::commands::effects::retime_transition(
                &layer.id, pm_edge, &t.kind, &t.params, seconds,
            ),
            None => pm_kernel::commands::effects::EffectCommand::SetTransition {
                layer_id: layer.id.clone(),
                edge: pm_edge,
                transition: None,
            },
        }
        .into()
    })
}
pub fn frame(time: f64, fps: f64) -> f64 {
    let result = (time * fps + 0.5).floor() / fps;
    if result == 0. { 0. } else { result }
}
pub(crate) fn finite(value: f64) -> Result<(), ModelError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ModelError::InvalidNumber)
    }
}

/// Semantic context rows; no labels, menu layout or styling. Extension rows append.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// The action (`menu_action`); [`MenuItem::SEPARATOR`] and
    /// [`MenuItem::HEADER`] are rows that are no action.
    pub id: &'static str,
    /// What the row says (a count when it acts on several keys).
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}
impl MenuItem {
    pub const SEPARATOR: &'static str = "-";
    /// A section's title (the keyframe menu's "Keyframe" / "3 Keyframes").
    pub const HEADER: &'static str = "header";
    /// Whether choosing the row does something.
    pub fn is_action(&self) -> bool {
        self.id != Self::SEPARATOR && self.id != Self::HEADER
    }
}
#[derive(Clone, Debug)]
pub enum Context {
    Ruler { time: f64 },
    Marker { id: String },
    Keys { ids: Vec<String> },
    Track { layer: String, channel: String },
    Layer { id: String },
    Graph { series: usize },
    Empty,
}
impl TimelineModel {
    pub fn menu(&self, context: &Context) -> Vec<MenuItem> {
        let mut items = Vec::new();
        let mut row = |id: &'static str, label: String, enabled, checked| {
            items.push(MenuItem {
                id,
                label,
                enabled,
                checked,
            })
        };
        let mut add = |id, enabled, checked| row(id, menu_label(id).to_owned(), enabled, checked);
        // v1 (timeline.ts onCtx): the work area's three, then the switch
        // naming the timeline it goes to.
        let work_and_mode = |add: &mut dyn FnMut(&'static str, bool, bool)| {
            for id in ["work.start", "work.end", "work.reset"] {
                add(id, true, false);
            }
            add(MenuItem::SEPARATOR, false, false);
            add(
                if self.mode == TimelineMode::Layers {
                    "mode.tracks"
                } else {
                    "mode.layers"
                },
                true,
                false,
            );
        };
        match context {
            Context::Marker { id } => add(
                "marker.remove",
                self.index.markers.iter().any(|m| m.id.as_ref() == Some(id)),
                false,
            ),
            Context::Ruler { time } => {
                add(
                    "marker.add",
                    self.index
                        .marker_at(frame(*time, self.index.fps).clamp(0., self.index.duration))
                        .is_none(),
                    false,
                );
                add(MenuItem::SEPARATOR, false, false);
                work_and_mode(&mut add);
            }
            Context::Keys { ids } => {
                let keys = self.context_keys(ids);
                let editable = !keys.is_empty()
                    && keys.iter().all(|id| {
                        self.index
                            .key(id)
                            .is_some_and(|(l, _, _)| !l.effectively_locked)
                    });
                // v1's pushKeyframeMenu: a header naming what it acts on,
                // the dialogs, the eases (the one every key has checked),
                // hold, then Copy and Delete.
                let many = keys.len() > 1;
                let count = keys.len();
                drop(add);
                row(
                    MenuItem::HEADER,
                    if many { format!("{count} Keyframes") } else { "Keyframe".into() },
                    false,
                    false,
                );
                for id in ["keys.value", "keys.velocity", "keys.scale"] {
                    row(id, menu_label(id).into(), editable, false);
                }
                let presets = self.key_presets(&keys);
                for id in [
                    "ease.linear",
                    "ease.easeIn",
                    "ease.easeOut",
                    "ease.easeInOut",
                    "ease.swish",
                    "ease.power",
                    "ease.expoOut",
                    "ease.backOut",
                    "ease.snap",
                    "ease.glide",
                ] {
                    let on = !presets.is_empty() && presets.iter().all(|p| *p == Some(&id[5..]));
                    row(id, menu_label(id).into(), editable, on);
                }
                let hold = if self.all_held(&keys) {
                    "Remove Hold"
                } else if many {
                    "Hold Keyframes"
                } else {
                    "Toggle Hold"
                };
                row("keys.hold", hold.into(), editable, false);
                row(MenuItem::SEPARATOR, String::new(), false, false);
                let counted = |verb: &str| {
                    if many {
                        format!("{verb} {count} Keyframes")
                    } else {
                        format!("{verb} Keyframe")
                    }
                };
                row("keys.copy", counted("Copy"), !keys.is_empty(), false);
                row("keys.delete", counted("Delete"), editable, false);
                return items;
            }
            Context::Track { layer, channel } => {
                let editable = self.require_channel(layer, channel).is_ok();
                add("track.add", editable, false);
                add("track.paste", editable, false);
                add(
                    "track.clear",
                    editable
                        && self
                            .index
                            .channel(layer, channel)
                            .is_some_and(|c| !c.keys.is_empty()),
                    false,
                );
                // v1's separateDimensionsItem: on a Position row, checked
                // while its layer's dimensions are separate.
                if channel == "position" || channel.starts_with("position.") {
                    let separated = self
                        .index
                        .layer(layer)
                        .is_some_and(|l| l.separate_dimensions);
                    add(
                        "track.separate",
                        !self.separate_targets(layer).is_empty(),
                        separated,
                    );
                }
            }
            Context::Layer { .. } => add("layer.shared-menu", true, false),
            Context::Graph { series } => {
                add("graph.value", true, self.graph_mode == GraphMode::Value);
                add("graph.speed", true, self.graph_mode == GraphMode::Speed);
                // v1's order: Merge, then Fit.
                add("graph.merge", *series >= 2, self.graph_merge);
                add("graph.fit", *series > 0, false);
                add(
                    "graph.combined",
                    true,
                    self.graph_display == GraphDisplay::Combined,
                );
            }
            Context::Empty => work_and_mode(&mut add),
        }
        items
    }
    pub fn menu_action(
        &mut self,
        context: &Context,
        action: &str,
        time: f64,
    ) -> Result<Plan, ModelError> {
        if !self
            .menu(context)
            .iter()
            .any(|item| item.id == action && item.enabled)
        {
            return Err(ModelError::UnknownCommand(action.into()));
        }
        let edit = |c: Command| Plan::edit(vec![c], self.index.revision);
        let keys = if let Context::Keys { ids } = context {
            self.context_keys(ids)
        } else {
            Vec::new()
        };
        Ok(match action {
            "marker.add" => self.add_marker(if let Context::Ruler { time } = context {
                *time
            } else {
                time
            })?,
            "marker.remove" => {
                if let Context::Marker { id } = context {
                    edit(
                        MarkerCommand::RemoveById {
                            comp: None,
                            id: id.clone(),
                        }
                        .into(),
                    )
                } else {
                    return Err(ModelError::InvalidGesture);
                }
            }
            "work.start" => edit(
                CompCommand::SetWorkArea {
                    comp: None,
                    start: Some(time),
                    end: None,
                }
                .into(),
            ),
            "work.end" => edit(
                CompCommand::SetWorkArea {
                    comp: None,
                    start: None,
                    end: Some(time),
                }
                .into(),
            ),
            "work.reset" => edit(CompCommand::ResetWorkArea { comp: None }.into()),
            "mode.tracks" | "mode.layers" => {
                self.mode = if action == "mode.tracks" {
                    TimelineMode::Tracks
                } else {
                    TimelineMode::Layers
                };
                Plan::default()
            }
            "mode.toggle" => {
                self.mode = if self.mode == TimelineMode::Layers {
                    TimelineMode::Tracks
                } else {
                    TimelineMode::Layers
                };
                Plan::default()
            }
            "keys.delete" => edit(KeyframeCommand::Delete { key_ids: keys }.into()),
            "keys.hold" => {
                let hold = !self.all_held(&keys);
                edit(
                    KeyframeCommand::SetHold {
                        key_ids: keys,
                        hold,
                    }
                    .into(),
                )
            }
            "keys.copy" => Plan {
                effects: vec![HostEffect::CopyKeys(self.copy_keys(&keys))],
                ..Plan::default()
            },
            "keys.value" => Plan {
                effects: vec![HostEffect::EditKeyValues(keys)],
                ..Plan::default()
            },
            "keys.velocity" => Plan {
                effects: vec![HostEffect::EditVelocity(keys)],
                ..Plan::default()
            },
            "keys.scale" => Plan {
                effects: vec![HostEffect::ScaleKeys(keys)],
                ..Plan::default()
            },
            "track.add" | "track.clear" => {
                let Context::Track { layer, channel } = context else {
                    return Err(ModelError::InvalidGesture);
                };
                if action == "track.add" {
                    edit(
                        KeyframeCommand::Add {
                            layer_id: layer.clone(),
                            channel: channel.clone(),
                            time,
                            key_id: new_id("K"),
                            value: None,
                        }
                        .into(),
                    )
                } else {
                    edit(
                        KeyframeCommand::ClearAll {
                            layer_id: layer.clone(),
                            channel: channel.clone(),
                            time,
                        }
                        .into(),
                    )
                }
            }
            "track.paste" => {
                return Err(ModelError::MissingHost("clipboard payload: use paste_keys"));
            }
            "track.separate" => {
                let Context::Track { layer, .. } = context else {
                    return Err(ModelError::InvalidGesture);
                };
                let separated = !self
                    .index
                    .layer(layer)
                    .is_some_and(|l| l.separate_dimensions);
                let commands = self
                    .separate_targets(layer)
                    .into_iter()
                    .map(|layer_id| {
                        pm_kernel::commands::layers::LayerCommand::Set {
                            layer_id,
                            patch: pm_kernel::commands::layers::LayerPatch {
                                separate_dimensions: Some(separated),
                                ..Default::default()
                            },
                        }
                        .into()
                    })
                    .collect();
                let mut plan = Plan::edit(commands, self.index.revision);
                if let Some(pm_kernel::Call::Transact { label, .. }) = plan.calls.first_mut() {
                    *label = Some(
                        if separated {
                            "Separate Dimensions"
                        } else {
                            "Join Dimensions"
                        }
                        .into(),
                    );
                }
                plan
            }
            "layer.shared-menu" => {
                if let Context::Layer { id } = context {
                    Plan {
                        effects: vec![HostEffect::ShowLayerMenu(id.clone())],
                        ..Plan::default()
                    }
                } else {
                    return Err(ModelError::InvalidGesture);
                }
            }
            id if id.starts_with("ease.") => edit(
                KeyframeCommand::SetEasing {
                    key_ids: keys,
                    curve: EaseSpec::Preset(id[5..].into()),
                }
                .into(),
            ),
            id if id.starts_with("graph.") => {
                return self.command(id, time, None, false, 0, &PropertyFacts::default());
            }
            _ => return Err(ModelError::UnknownCommand(action.into())),
        })
    }
}

/// timeline.ts:326–373: first/last visible frames, never exclusive strip out.
pub fn first_frame(from: f64, fps: f64) -> f64 {
    let rate = fps.max(1.);
    (from * rate - 1e-6).ceil() / rate
}
pub fn last_frame(from: f64, duration: f64, fps: f64) -> f64 {
    let rate = fps.max(1.);
    first_frame(from, rate).max(((from + duration) * rate - 1e-6).ceil() / rate - 1. / rate)
}
pub fn playhead_targets(index: &TimelineIndex) -> Vec<f64> {
    let mut times = vec![0., last_frame(0., index.duration, index.fps)];
    times.extend(index.work.iter().map(|t| frame(*t, index.fps)));
    times.extend(index.markers.iter().map(|m| frame(m.time, index.fps)));
    for l in &index.layers {
        if l.duration <= 0. {
            continue;
        }
        times.extend([
            first_frame(l.from, index.fps),
            last_frame(l.from, l.duration, index.fps),
        ]);
        for c in &l.channels {
            times.extend(c.keys.iter().map(|k| frame(l.from + k.time, index.fps)));
        }
    }
    times.retain(|t| t.is_finite());
    times.sort_by(f64::total_cmp);
    times.dedup();
    times
}

/// Menu row titles, shared by every surface that shows the model's menus.
pub fn menu_label(id: &str) -> &'static str {
    match id {
        "marker.add" => "Add Marker Here",
        "marker.remove" => "Remove Marker",
        "work.start" => "Set Work Area Start",
        "work.end" => "Set Work Area End",
        "work.reset" => "Reset Work Area",
        "mode.toggle" => "Toggle Layers and Tracks",
        "mode.tracks" => "Switch to Track Timeline",
        "mode.layers" => "Switch to Layer Timeline",
        "keys.value" => "Edit Value…",
        "keys.velocity" => "Keyframe Velocity…",
        "keys.scale" => "Scale Keyframes…",
        "keys.hold" => "Toggle Hold Keyframe",
        "keys.delete" => "Delete Keyframes",
        "keys.copy" => "Copy Keyframes",
        "ease.linear" => "Linear",
        "ease.easeIn" => "Ease In",
        "ease.easeOut" => "Ease Out",
        "ease.easeInOut" => "Easy Ease",
        "ease.swish" => "Swish",
        "ease.power" => "Power",
        "ease.expoOut" => "Exponential Out",
        "ease.backOut" => "Overshoot",
        "ease.snap" => "Snap",
        "ease.glide" => "Glide",
        "track.add" => "Add Keyframe Here",
        "track.paste" => "Paste Keyframes Here",
        "track.clear" => "Clear All Keyframes",
        "track.separate" => "Separate Dimensions",
        "layer.shared-menu" => "Layer Options…",
        "graph.value" => "Value Graph",
        "graph.speed" => "Speed Graph",
        "graph.fit" => "Fit Selected Curves",
        "graph.merge" => "Merge Selected Curves",
        "graph.combined" => "Show Layer Bars with Graph",
        _ => "",
    }
}

impl TimelineModel {
    /// Pointer cuts share the playhead split kernel command and one transaction.
    pub fn split_at(&self, hit: &str, time: f64, all: bool) -> Result<Plan, ModelError> {
        let Some(hit) = self.index.layer_ordinal(hit).filter(|_| time.is_finite()) else {
            return Ok(Plan::default());
        };
        let candidates = if all {
            &self.index.layers[..]
        } else {
            &self.index.layers[hit..=hit]
        };
        let margin = 1. / self.index.fps.max(1.);
        let layers: Vec<_> = candidates
            .iter()
            .filter(|l| {
                !l.effectively_locked
                    && time - l.from > margin + 1e-9
                    && l.from + l.duration - time > margin + 1e-9
            })
            .collect();
        if layers.is_empty() {
            return Ok(Plan::default());
        }
        let label = if all {
            format!("Split {} layers", layers.len())
        } else {
            format!("Split {}", layers[0].name.to_string_lossy())
        };
        let ids = layers.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
        let tails = ids.iter().map(|_| new_id("L")).collect::<Vec<_>>();
        let mut commands = Vec::new();
        if self.mode == crate::TimelineMode::Tracks {
            // Freeze the displayed lanes before Split creates its new pieces.
            // Otherwise the automatic layout can pack them into another track.
            commands.push(TimingCommand::PinTracks {
                lanes: crate::tracks::relative_lanes(&self.index, &self.reveal),
            }.into());
        }
        commands.push(TimingCommand::Split {
            layer_ids: ids,
            time,
            new_ids: tails,
        }.into());
        let mut plan = Plan::edit(commands, self.index.revision);
        if let Some(Call::Transact { label: slot, .. }) = plan.calls.first_mut() {
            *slot = Some(label);
        }
        Ok(plan)
    }
    /// Snap to actual cut boundaries, keys, markers and the stationary playhead.
    pub fn split_time(&self, raw: f64, playhead: f64, radius: f64) -> f64 {
        let rounded = frame(raw, self.index.fps);
        if self.track_snap_disabled {
            return rounded;
        }
        let cuts = self.index.split_targets();
        let next = cuts.partition_point(|(time, _)| *time < raw);
        let previous = next.checked_sub(1).and_then(|i| cuts.get(i).copied());
        [Some((playhead, 0)), previous, cuts.get(next).copied()]
            .into_iter()
            .flatten()
            .filter(|(t, _)| t.is_finite() && (*t - raw).abs() <= radius)
            .min_by(|(a, a_order), (b, b_order)| {
                (a - raw)
                    .abs()
                    .total_cmp(&(b - raw).abs())
                    .then(a_order.cmp(b_order))
            })
            .map_or(rounded, |(time, _)| time)
    }
}
