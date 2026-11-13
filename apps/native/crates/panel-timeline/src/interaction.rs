use crate::snap::{Anchor, DirectionalSnap, Edge, Target};
use crate::{GraphDisplay, GraphMode, PropertyFacts, TimelineIndex, TimelineMode, TimelineModel};
use pm_kernel::commands::{
    comps::CompPatch,
    keyframes::{CopiedKey, EaseSide, EaseSpec, HandleValue},
    layers::{LayerPatch, ParentChange},
    properties::SetProperty,
};
use pm_kernel::{
    Call, Command, CompCommand, HistoryCall, KeyframeCommand, LayerCommand, MarkerCommand,
    PropertyCommand, SelectionCall, TimingCommand, WriteBody, new_id,
};
use pm_model::Value;
use std::sync::Arc;

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
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ModelError {}

/// Effects the application must execute on its transport/clipboard/extension hosts.
/// Returning an effect does not claim that host work has succeeded.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEffect {
    Seek(f64),
    TogglePlayback,
    FitCurves(Vec<String>),
    MergeCurves,
    FrameComposition,
    OpenComposition(String),
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
            "prevVisibleEvent" | "nextVisibleEvent" => {
                let mut events = vec![0., duration, self.index.work[0], self.index.work[1]];
                events.extend(self.index.markers.iter().map(|m| m.time));
                for l in &self.index.layers {
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
            "trimIn" => edit(
                TimingCommand::TrimIn {
                    layer_ids: layers,
                    time,
                }
                .into(),
            ),
            "trimOut" => edit(
                TimingCommand::TrimOut {
                    layer_ids: layers,
                    time,
                }
                .into(),
            ),
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
            "delete" if self.key_selection_active => {
                edit(KeyframeCommand::Delete { key_ids: keys }.into())
            }
            "delete" => edit(LayerCommand::Delete { layer_ids: layers }.into()),
            "deselect" => {
                self.key_selection_active = false;
                Plan {
                    calls: vec![SelectionCall::Clear.into()],
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
            "graph.fit" => Plan {
                effects: vec![HostEffect::FitCurves(keys)],
                ..Plan::default()
            },
            "graph.merge" => Plan {
                effects: vec![HostEffect::MergeCurves],
                ..Plan::default()
            },
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
        let plan = self.property_diamond(&targets, time)?;
        self.show_channels(&targets);
        Ok(plan)
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
        self.show_channels(&targets);
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
                    .filter(|c| wanted(&l.id, &c.path, !c.keys.is_empty()))
                    .map(|c| (l.id.clone(), c.path.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    /// Opens these layers' rows on these channels (with what they showed).
    fn show_channels(&mut self, targets: &[(String, String)]) {
        for (layer, path) in targets {
            let Some(l) = self.index.layer(layer) else {
                continue;
            };
            let shown = self.reveal.layers.entry(layer.clone()).or_insert_with(|| {
                crate::reveal::Disclosure {
                    collapsed: l.collapsed,
                    group_collapsed: l.group_collapsed,
                    paths: None,
                }
            });
            if shown.collapsed {
                // Closed, it showed nothing: open on what gets a key.
                shown.collapsed = false;
                shown.paths = Some(vec![]);
            }
            // `None` lists the animated channels, which these now are.
            if let Some(paths) = &mut shown.paths
                && !paths.iter().any(|p| p == "*" || p == path)
            {
                paths.push(path.clone());
            }
        }
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
    pub fn switch(&self, layer: &str, switch: LayerSwitch) -> Result<Plan, ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        let patch = match switch {
            LayerSwitch::Visible => LayerPatch {
                visible: Some(!l.visible),
                ..Default::default()
            },
            LayerSwitch::Locked => LayerPatch {
                locked: Some(!l.locked),
                ..Default::default()
            },
            LayerSwitch::Solo => LayerPatch {
                solo: Some(!l.solo),
                ..Default::default()
            },
            LayerSwitch::ThreeD => LayerPatch {
                three_d: Some(!l.three_d),
                ..Default::default()
            },
        };
        Ok(Plan::edit(
            vec![
                LayerCommand::Set {
                    layer_id: layer.into(),
                    patch,
                }
                .into(),
            ],
            self.index.revision,
        ))
    }
    pub fn parent(&self, ids: &[String], parent: Option<String>, time: f64) -> Plan {
        Plan::edit(
            ids.iter()
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
                .collect(),
            self.index.revision,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub enum LayerSwitch {
    Visible,
    Locked,
    Solo,
    ThreeD,
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
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DragSample {
    pub time_delta: f64,
    pub value_delta: f64,
    pub time_scale: f64,
    pub value_scale: f64,
    pub handle_time: f64,
    pub handle_value: f64,
    pub modifiers: Modifiers,
    pub acquire_tolerance: f64,
    pub release_tolerance: f64,
    pub playhead: f64,
}
#[derive(Clone, Debug)]
pub struct Drag {
    pub kind: DragKind,
    pub start: Arc<TimelineIndex>,
    pub snap: DirectionalSnap,
    fixed: Option<Vec<Target>>,
    anchors: Vec<Anchor>,
    pub key_snap: Option<crate::snap::GroupLock>,
    track_snap: Option<bool>,
    pub property_values: std::collections::HashMap<(String, String), f64>,
    property_keys: std::collections::HashMap<(String, String), Vec<(String, f64)>>,
}
impl Drag {
    /// Captures identities and baselines once; never keeps a writer lease itself.
    pub fn begin(model: &TimelineModel, mut kind: DragKind) -> Result<Self, ModelError> {
        let index = &model.index;
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
                for id in ids.iter() {
                    if index.layer(id).unwrap().effectively_locked {
                        return Err(ModelError::Locked(id.clone()));
                    }
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
            _ => {}
        }
        let mut anchors = Vec::new();
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
                for id in ids {
                    let l = index.layer(id).unwrap();
                    if l.kind != "group" {
                        let is_in = matches!(kind, DragKind::TrimIn { .. });
                        anchors.push(Anchor {
                            time: if is_in { l.from } else { l.from + l.duration },
                            edge: Some(if is_in { Edge::In } else { Edge::Out }),
                        });
                    }
                }
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
        let mut property_keys = std::collections::HashMap::new();
        if let DragKind::Property { targets, time } = &kind {
            for (l, c) in targets {
                let layer = index.layer(l).unwrap();
                let channel = index.channel(l, c).unwrap();
                if !channel
                    .keys
                    .iter()
                    .any(|k| (layer.from + k.time - time).abs() < 0.5 / index.fps)
                {
                    let keys: Vec<_> = channel
                        .keys
                        .iter()
                        .filter(|k| model.selection.keys().contains(&k.id))
                        .filter_map(|k| k.value.as_f64().map(|v| (k.id.clone(), v)))
                        .collect();
                    if !keys.is_empty() {
                        property_keys.insert((l.clone(), c.clone()), keys);
                    }
                }
            }
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
        Ok(Self {
            kind,
            start: index.clone(),
            snap: DirectionalSnap::default(),
            fixed: None,
            anchors,
            key_snap: None,
            track_snap,
            property_values: Default::default(),
            property_keys,
        })
    }
    pub fn label(&self) -> &'static str {
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
            }
            DragKind::Layers { .. } => "Move clip",
            DragKind::DuplicateLayers { .. } => "Duplicate Layers",
            DragKind::TrimIn { .. } | DragKind::TrimOut { .. } => "Trim clip",
            DragKind::QuickOffsetLayers { .. } => "Quick offset layers",
            DragKind::Keys { .. } => "Move keyframe",
            DragKind::GraphKeys { .. } => "Edit curve",
            DragKind::StretchKeys { .. } => "Stretch keyframes",
            DragKind::TransformKeys { .. } => "Scale keyframes",
            DragKind::Handle { .. } => "Adjust Handle",
            DragKind::Property { .. } => "Change Property",
            DragKind::WorkMove => "Move work area",
            _ => "Work area",
        }
    }
    pub fn commands(&mut self, sample: DragSample) -> Result<Vec<Command>, ModelError> {
        finite(sample.time_delta)?;
        finite(sample.value_delta)?;
        finite(sample.playhead)?;
        let snapping = self
            .track_snap
            .map_or(sample.modifiers.shift, |on| on != sample.modifiers.shift);
        if snapping && self.fixed.is_none() {
            self.fixed = Some(snap_targets(
                &self.start,
                self.moving_layers(),
                self.moving_keys(),
            ));
        }
        let mut targets = self.fixed.clone().unwrap_or_default();
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
            DragKind::ProEdit(command) => {
                use pm_kernel::commands::pro_edit::ProEditCommand as Pro;
                let mut command = command.clone();
                match &mut command {
                    Pro::Move { delta, .. }
                    | Pro::Trim { delta, .. }
                    | Pro::Slip { delta, .. }
                    | Pro::RippleTrim { delta, .. }
                    | Pro::Roll { delta, .. }
                    | Pro::Slide { delta, .. } => *delta = sample.time_delta,
                    _ => return Err(ModelError::InvalidGesture),
                }
                out.push(command.into());
            }

            DragKind::Layers { ids } => {
                for id in ids {
                    let l = self.start.layer(id).unwrap();
                    let from = if self.snap.lock.is_some() {
                        l.from + delta
                    } else {
                        frame(l.from + sample.time_delta, self.start.fps)
                    };
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
            // The copies exist only in this gesture's writes: each step
            // makes them again from the start and places them.
            DragKind::DuplicateLayers { ids, new_ids } => {
                let first = self.start.layer(&ids[0]).unwrap();
                let step = if self.snap.lock.is_some() {
                    delta
                } else {
                    frame(first.from + sample.time_delta, self.start.fps) - first.from
                };
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
                for id in ids {
                    let l = self.start.layer(id).unwrap();
                    if l.kind == "group" {
                        continue;
                    }
                    out.push(
                        if matches!(self.kind, DragKind::TrimIn { .. }) {
                            TimingCommand::TrimIn {
                                layer_ids: vec![id.clone()],
                                time: l.from + delta,
                            }
                        } else {
                            TimingCommand::TrimOut {
                                layer_ids: vec![id.clone()],
                                time: l.from + l.duration + delta,
                            }
                        }
                        .into(),
                    );
                }
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
            DragKind::GraphKeys { ids } => out.push(
                KeyframeCommand::GraphMove {
                    key_ids: ids.clone(),
                    delta,
                    value_delta: sample.value_delta,
                }
                .into(),
            ),
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
                out.push(
                    KeyframeCommand::Transform {
                        key_ids: ids.clone(),
                        anchor_time: *anchor_time,
                        time_scale: sample.time_scale,
                        anchor_value: *anchor_value,
                        value_scale: sample.value_scale,
                    }
                    .into(),
                );
            }
            DragKind::Handle { id, side } => {
                finite(sample.handle_time)?;
                finite(sample.handle_value)?;
                out.push(
                    KeyframeCommand::SetHandle {
                        key_id: id.clone(),
                        side: *side,
                        value: HandleValue::Point {
                            t: sample.handle_time,
                            v: sample.handle_value,
                        },
                        split: sample.modifiers.alt,
                    }
                    .into(),
                );
            }
            DragKind::Property { targets, time } => {
                for (l, c) in targets {
                    if let Some(keys) = self.property_keys.get(&(l.clone(), c.clone())) {
                        for (id, start) in keys {
                            out.push(
                                KeyframeCommand::Edit {
                                    key_id: id.clone(),
                                    time: None,
                                    value: Some(Value::Number(start + sample.value_delta)),
                                }
                                .into(),
                            );
                        }
                        continue;
                    }
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
                    out.push(
                        PropertyCommand::Set(SetProperty::new(l, c, Value::Number(value), *time))
                            .into(),
                    );
                }
            }
            DragKind::WorkIn | DragKind::WorkOut | DragKind::WorkMove => {
                let [a, b] = self.start.work;
                let frame = 1. / self.start.fps;
                let duration = self.start.duration;
                let (a, b, duration) = match self.kind {
                    DragKind::WorkIn => ((a + delta).max(0.).min(b - frame), b, duration),
                    DragKind::WorkOut => {
                        let end = (b + delta).max(a + frame);
                        (a, end, duration.max(end))
                    }
                    _ => {
                        let d = delta.max(-a).min(duration - b);
                        (a + d, b + d, duration)
                    }
                };
                out.push(
                    CompCommand::Set {
                        patch: Box::new(CompPatch {
                            duration: Some(duration),
                            work_area: Some((a, b)),
                            ..Default::default()
                        }),
                    }
                    .into(),
                );
            }
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
        if l.duration > 0. {
            out.extend([Target::at(l.from), Target::at(l.from + l.duration)]);
        }
        if moving_layers.is_empty() {
            for c in &l.channels {
                for k in &c.keys {
                    if !moving_keys.contains(&k.id) {
                        let time = l.from + k.time;
                        if time >= 0. && time <= index.duration {
                            out.push(Target::at(time));
                        }
                    }
                }
            }
        }
    }
    out
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
    pub id: &'static str,
    pub enabled: bool,
    pub checked: bool,
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
        let mut add = |id, enabled, checked| {
            items.push(MenuItem {
                id,
                enabled,
                checked,
            })
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
                for id in ["work.start", "work.end", "work.reset", "mode.toggle"] {
                    add(id, true, false);
                }
            }
            Context::Keys { ids } => {
                let keys = self.context_keys(ids);
                let editable = !keys.is_empty()
                    && keys.iter().all(|id| {
                        self.index
                            .key(id)
                            .is_some_and(|(l, _, _)| !l.effectively_locked)
                    });
                for id in [
                    "keys.value",
                    "keys.velocity",
                    "keys.scale",
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
                    "keys.hold",
                    "keys.delete",
                ] {
                    add(id, editable, false);
                }
                add("keys.copy", !keys.is_empty(), false);
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
            }
            Context::Layer { .. } => add("layer.shared-menu", true, false),
            Context::Graph { series } => {
                add("graph.value", true, self.graph_mode == GraphMode::Value);
                add("graph.speed", true, self.graph_mode == GraphMode::Speed);
                add("graph.fit", *series > 0, false);
                add("graph.merge", *series >= 2, false);
                add(
                    "graph.combined",
                    true,
                    self.graph_display == GraphDisplay::Combined,
                );
            }
            Context::Empty => {
                for id in ["work.start", "work.end", "work.reset", "mode.toggle"] {
                    add(id, true, false);
                }
            }
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
                let hold = !keys.iter().all(|id| {
                    self.index
                        .key(id)
                        .is_some_and(|(_, _, k)| crate::snapshot::boolean(k.stored.get("hold")))
                });
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
        "marker.add" => "Add Marker",
        "marker.remove" => "Remove Marker",
        "work.start" => "Set Work Area Start",
        "work.end" => "Set Work Area End",
        "work.reset" => "Reset Work Area",
        "mode.toggle" => "Toggle Layers and Tracks",
        "keys.value" => "Edit Value…",
        "keys.velocity" => "Keyframe Velocity…",
        "keys.scale" => "Scale Keyframes…",
        "keys.hold" => "Toggle Hold Keyframe",
        "keys.delete" => "Delete Keyframes",
        "keys.copy" => "Copy Keyframes",
        "ease.linear" => "Linear",
        "ease.easeIn" => "Ease In",
        "ease.easeOut" => "Ease Out",
        "ease.easeInOut" => "Ease In and Out",
        "ease.swish" => "Swish",
        "ease.power" => "Power",
        "ease.expoOut" => "Expo Out",
        "ease.backOut" => "Back Out",
        "ease.snap" => "Snap",
        "ease.glide" => "Glide",
        "track.add" => "Add Keyframe",
        "track.paste" => "Paste Keyframes",
        "track.clear" => "Remove All Keyframes",
        "layer.shared-menu" => "Layer Options…",
        "graph.value" => "Value Graph",
        "graph.speed" => "Speed Graph",
        "graph.fit" => "Fit Curves",
        "graph.merge" => "Merge Curves",
        "graph.combined" => "Show Dope Sheet",
        _ => "",
    }
}
