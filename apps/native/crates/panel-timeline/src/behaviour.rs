//! Remaining semantic adapters. Platform hit tests produce these intents by id.
use crate::{
    HostEffect, ModelError, Plan, TimelineModel,
    interaction::{finite, frame},
};
use pm_kernel::commands::{
    captions::CaptionEdit,
    keyframes::{KeyframeSpec, ReplaceKeyframes},
    layers::LayerPatch,
    properties::SetProperty,
};
use pm_kernel::{
    Command, ContentCommand, GroupCommand, KeyframeCommand, LayerCommand, PropertyCommand,
};
use pm_model::{Map, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub struct NumericMetadata {
    pub step: f64,
    pub min: f64,
    pub max: f64,
}
impl Default for NumericMetadata {
    fn default() -> Self {
        Self {
            step: 0.1,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
        }
    }
}
/// property-values.ts:55–60. `delta` is a logical field drag delta from the host.
pub fn dragged_value(
    start: f64,
    delta: f64,
    meta: NumericMetadata,
    fine: bool,
    fast: bool,
) -> Result<f64, ModelError> {
    finite(start)?;
    finite(delta)?;
    finite(meta.step)?;
    if meta.min.is_nan() || meta.max.is_nan() || meta.min > meta.max {
        return Err(ModelError::InvalidNumber);
    }
    let result = start
        + delta
            * meta.step
            * if fine {
                0.1
            } else if fast {
                10.
            } else {
                1.
            };
    finite(result)?;
    Ok(((result * 1e6 + 0.5).floor() / 1e6).clamp(meta.min, meta.max))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeViewport {
    pub start: f64,
    pub end: f64,
    pub first_row: usize,
}
impl TimeViewport {
    pub fn new(start: f64, end: f64) -> Result<Self, ModelError> {
        finite(start)?;
        finite(end)?;
        if end <= start {
            return Err(ModelError::InvalidNumber);
        }
        Ok(Self {
            start,
            end,
            first_row: 0,
        })
    }
    pub fn pan(&mut self, delta: f64, minimum: f64) -> Result<(), ModelError> {
        finite(delta)?;
        finite(minimum)?;
        let width = self.end - self.start;
        self.start = (self.start + delta).max(minimum);
        self.end = self.start + width;
        Ok(())
    }
    /// No zoom limits or physical sizes: the Opus view supplies its legal range.
    pub fn zoom(&mut self, anchor: f64, factor: f64, minimum: f64) -> Result<(), ModelError> {
        finite(anchor)?;
        finite(factor)?;
        finite(minimum)?;
        if factor <= 0. {
            return Err(ModelError::InvalidNumber);
        }
        let start = anchor + (self.start - anchor) / factor;
        let end = anchor + (self.end - anchor) / factor;
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Err(ModelError::InvalidNumber);
        }
        let shift = (minimum - start).max(0.);
        self.start = start + shift;
        self.end = end + shift;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionTab {
    pub id: String,
    pub active: bool,
    pub closable: bool,
}
pub fn composition_tabs(ids: &[String], active: &str) -> Vec<CompositionTab> {
    ids.iter()
        .map(|id| CompositionTab {
            id: id.clone(),
            active: id == active,
            closable: ids.len() > 1,
        })
        .collect()
}

impl TimelineModel {
    /// Linked Scale row: distinct stored keys stay distinct at unequal legacy times.
    pub fn scale_members(&self, layer: &str, time: f64) -> Vec<String> {
        ["scale.x", "scale.y"]
            .into_iter()
            .filter_map(|p| self.index.channel(layer, p))
            .flat_map(|c| c.keys.iter())
            .filter(|k| k.time == time)
            .map(|k| k.id.clone())
            .collect()
    }
    pub fn expand_scale_keys(&self, ids: &[String]) -> Vec<String> {
        let mut result = ids.to_vec();
        for id in ids {
            if let Some((l, c, k)) = self.index.key(id)
                && (c.path == "scale.x"
                    || c.path == "scale.y"
                    || (!l.separate_dimensions && c.path.starts_with("position.")))
            {
                let path = if c.path.starts_with("position.") {
                    "position"
                } else {
                    "scale"
                };
                for id in self
                    .track_channels(&l.id, path)
                    .into_iter()
                    .flat_map(|c| &c.keys)
                    .filter(|other| other.time == k.time)
                    .map(|k| k.id.clone())
                {
                    if !result.contains(&id) {
                        result.push(id);
                    }
                }
            }
        }
        result
    }
    /// timeline.ts:201–218 removes at all targets only when every axis has a key.
    pub fn property_diamond(
        &self,
        targets: &[(String, String)],
        time: f64,
    ) -> Result<Plan, ModelError> {
        finite(time)?;
        for (l, c) in targets {
            self.require_channel(l, c)?;
        }
        let all = targets.iter().all(|(l, c)| {
            self.index.channel(l, c).unwrap().keys.iter().any(|k| {
                (self.index.layer(l).unwrap().from + k.time - time).abs() < 0.5 / self.index.fps
            })
        });
        let mut commands = Vec::new();
        for (l, c) in targets {
            let at = self.index.channel(l, c).unwrap().keys.iter().find(|k| {
                (self.index.layer(l).unwrap().from + k.time - time).abs() < 0.5 / self.index.fps
            });
            if all {
                if let Some(k) = at {
                    commands.push(
                        KeyframeCommand::Delete {
                            key_ids: vec![k.id.clone()],
                        }
                        .into(),
                    );
                }
            } else if at.is_none() {
                commands.push(
                    KeyframeCommand::Add {
                        layer_id: l.clone(),
                        channel: c.clone(),
                        time,
                        key_id: pm_kernel::new_id("K"),
                        value: None,
                    }
                    .into(),
                );
            }
        }
        Ok(Plan::edit(commands, self.index.revision))
    }
    pub fn stopwatch(&self, layer: &str, channel: &str, time: f64) -> Result<Plan, ModelError> {
        self.require_channel(layer, channel)?;
        finite(time)?;
        Ok(Plan::edit(
            vec![
                PropertyCommand::ToggleStopwatch {
                    layer_id: layer.into(),
                    channel: channel.into(),
                    time,
                }
                .into(),
            ],
            self.index.revision,
        ))
    }
    pub fn scale_link(&self, layer: &str, linked: bool) -> Plan {
        Plan::edit(
            vec![
                LayerCommand::Set {
                    layer_id: layer.into(),
                    patch: LayerPatch {
                        scale_linked: Some(linked),
                        ..Default::default()
                    },
                }
                .into(),
            ],
            self.index.revision,
        )
    }
    pub fn mute(&self, layer: &str, time: f64) -> Result<Plan, ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        let mut patch = Map::new();
        patch.insert("audioMuted", Value::Bool(!l.audio_muted));
        Ok(Plan::edit(
            vec![
                ContentCommand::Set {
                    layer_id: layer.into(),
                    patch,
                    time,
                }
                .into(),
            ],
            self.index.revision,
        ))
    }
    /// Reorder is addressed by destination ID; resolve index in this revision and
    /// fence the transaction. Group pose/cycle and lock validation stays in kernel.
    pub fn reorder(
        &self,
        ids: &[String],
        target: &str,
        after: bool,
        time: f64,
    ) -> Result<Plan, ModelError> {
        self.drop_layers(
            ids,
            target,
            if after {
                DropMode::After
            } else {
                DropMode::Before
            },
            time,
            pm_kernel::Origin::Interface,
        )
    }
    pub fn group_into(&self, ids: Vec<String>, group: Option<String>, time: f64) -> Plan {
        Plan::edit(
            vec![
                GroupCommand::MoveToGroup {
                    layer_ids: ids,
                    group,
                    time,
                }
                .into(),
            ],
            self.index.revision,
        )
    }
    pub fn import_drop(
        &self,
        time: f64,
        before_layer: Option<String>,
        alt: bool,
    ) -> Result<HostEffect, ModelError> {
        finite(time)?;
        if let Some(id) = &before_layer {
            self.index
                .layer(id)
                .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
        }
        Ok(HostEffect::Import {
            time: frame(time, self.index.fps).max(0.),
            before_layer,
            as_folder: alt,
        })
    }
    /// Quick offset preserves exact subframes, relative timing within each layer,
    /// and every unknown key field. A group index is supplied per selected key.
    /// timeline.ts:222–270; keyframe replacement still validates in the kernel.
    pub fn quick_offset_keys(
        &self,
        groups: &[Vec<String>],
        requested: f64,
        time: f64,
    ) -> Result<Plan, ModelError> {
        finite(requested)?;
        finite(time)?;
        if groups.len() < 2 {
            return Ok(Plan::default());
        }
        let mut total = requested;
        let mut selected = std::collections::HashMap::new();
        for (i, group) in groups.iter().enumerate() {
            for id in group {
                let (l, _, k) = self
                    .index
                    .key(id)
                    .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
                if l.effectively_locked {
                    return Err(ModelError::Locked(l.id.clone()));
                }
                let factor = i as f64 / (groups.len() - 1) as f64;
                if factor > 0. {
                    let composition = l.from + k.time;
                    total = total
                        .max((composition.min(0.) - composition) / factor)
                        .min((composition.max(self.index.duration) - composition) / factor);
                }
                if selected.insert(id.clone(), factor).is_some() {
                    return Err(ModelError::InvalidGesture);
                }
            }
        }
        let mut commands = Vec::new();
        for l in &self.index.layers {
            for c in &l.channels {
                if !c.keys.iter().any(|k| selected.contains_key(&k.id)) {
                    continue;
                }
                let destinations: Vec<_> = c
                    .keys
                    .iter()
                    .filter_map(|k| selected.get(&k.id).map(|factor| k.time + factor * total))
                    .collect();
                let keys: Vec<_> = c
                    .keys
                    .iter()
                    .filter(|k| {
                        selected.contains_key(&k.id)
                            || !destinations.iter().any(|t| (*t - k.time).abs() < 1e-9)
                    })
                    .map(|k| {
                        let mut spec = KeyframeSpec::new(
                            k.time + selected.get(&k.id).copied().unwrap_or(0.) * total,
                            k.value.clone(),
                        );
                        spec.id = Some(k.id.clone());
                        spec.temporal = Some(k.stored.clone());
                        spec
                    })
                    .collect();
                commands.push(
                    KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
                        layer_id: l.id.clone(),
                        channel: c.path.clone(),
                        keyframes: keys,
                        replace: true,
                        expression: None,
                        preserve_hand_edits: false,
                        time,
                    }))
                    .into(),
                );
            }
        }
        Ok(Plan::edit(commands, self.index.revision))
    }
    /// Produces the typed caption request.
    pub fn caption_edit(&self, layer: &str, edit: CaptionEdit) -> Result<Plan, ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        if l.kind != "captions" {
            return Err(ModelError::UnknownId(layer.into()));
        }
        if l.effectively_locked {
            return Err(ModelError::Locked(layer.into()));
        }
        Ok(Plan::edit(
            vec![
                pm_kernel::CaptionCommand::Edit {
                    layer_id: layer.into(),
                    edit,
                }
                .into(),
            ],
            self.index.revision,
        ))
    }
}

/// Pure text-animator drag arithmetic, text-stagger.ts:103–113.
#[derive(Clone, Copy, Debug)]
pub struct TextStagger {
    pub delay: f64,
    pub duration: f64,
    pub stagger: f64,
    pub cascade: f64,
}
impl TextStagger {
    pub fn commands(
        self,
        layer: &str,
        animator: &str,
        stretch: bool,
        delta: f64,
        fps: f64,
        time: f64,
    ) -> Result<Vec<Command>, ModelError> {
        for v in [
            self.delay,
            self.duration,
            self.stagger,
            self.cascade,
            delta,
            fps,
            time,
        ] {
            finite(v)?;
        }
        if fps <= 0. {
            return Err(ModelError::InvalidNumber);
        }
        let values = if !stretch {
            vec![("delay", frame(self.delay + delta, fps))]
        } else {
            let span = self.duration + self.cascade;
            if span <= 0. {
                vec![("duration", frame(delta, fps).max(0.))]
            } else {
                let scale = frame(span + delta, fps).max(1. / fps) / span;
                vec![
                    (
                        "duration",
                        (self.duration * scale * 1e3 + 0.5).floor() / 1e3,
                    ),
                    ("stagger", (self.stagger * scale * 1e3 + 0.5).floor() / 1e3),
                ]
            }
        };
        Ok(values
            .into_iter()
            .map(|(key, value)| {
                let mut set = SetProperty::new(layer, format!("ta.{animator}.{key}"), value, time);
                set.preserve_hand_edits = false;
                PropertyCommand::Set(set).into()
            })
            .collect())
    }
}

/// Independent cue selection, caption-track.ts:157–171. Modifier clicks toggle
/// cue membership without starting a drag; choosing a layer clears cue selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueSelection {
    pub layer: Option<String>,
    pub ids: Vec<String>,
}
impl CueSelection {
    pub fn click(&mut self, layer: &str, id: &str, additive: bool) -> bool {
        if self.layer.as_deref() != Some(layer) {
            self.ids.clear();
            self.layer = Some(layer.into());
        }
        let selected = self.ids.iter().any(|i| i == id);
        if additive {
            if selected {
                self.ids.retain(|i| i != id);
            } else {
                self.ids.push(id.into());
            }
            false
        } else {
            if !selected {
                self.ids = vec![id.into()];
            }
            true
        }
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Strict identity filtering for semantic marquee/hit results.
pub fn unique_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub enum TrackHeaderAction {
    Select,
    Lock,
    Visible,
    Disclosure,
}
impl TimelineModel {
    /// Tracks use modifier toggles, unlike Layers' Shift range (track-view.ts:665).
    pub fn select_track_layer(
        &mut self,
        id: &str,
        modifiers: crate::Modifiers,
    ) -> Result<(Vec<pm_kernel::Call>, bool), ModelError> {
        self.index
            .layer(id)
            .ok_or_else(|| ModelError::UnknownId(id.into()))?;
        self.key_selection_active = false;
        let mut ids = self.selection.layers().to_vec();
        let selected = ids.iter().any(|i| i == id);
        let drag = if modifiers.shift || modifiers.command() {
            if selected {
                ids.retain(|i| i != id);
            } else {
                ids.push(id.into());
            }
            !selected
        } else {
            if !selected {
                ids = vec![id.into()];
            }
            true
        };
        Ok((
            vec![pm_kernel::SelectionCall::Layers { ids, add: false }.into()],
            drag,
        ))
    }
    pub fn track_header(
        &mut self,
        area: crate::tracks::Area,
        lane: usize,
        action: TrackHeaderAction,
    ) -> Plan {
        let layout = crate::tracks::layout(&self.index, &self.reveal);
        let ids: Vec<_> = layout
            .items
            .iter()
            .filter(|i| i.area == area && i.lane == lane)
            .map(|i| i.id.clone())
            .collect();
        if ids.is_empty() {
            return Plan::default();
        }
        match action {
            TrackHeaderAction::Select => {
                self.key_selection_active = false;
                Plan {
                    calls: vec![pm_kernel::SelectionCall::Layers { ids, add: false }.into()],
                    ..Default::default()
                }
            }
            TrackHeaderAction::Lock | TrackHeaderAction::Visible => {
                let state = if matches!(action, TrackHeaderAction::Lock) {
                    ids.iter().all(|id| self.index.layer(id).unwrap().locked)
                } else {
                    ids.iter().any(|id| self.index.layer(id).unwrap().visible)
                };
                let commands = ids
                    .into_iter()
                    .map(|id| {
                        LayerCommand::Set {
                            layer_id: id,
                            patch: if matches!(action, TrackHeaderAction::Lock) {
                                LayerPatch {
                                    locked: Some(!state),
                                    ..Default::default()
                                }
                            } else {
                                LayerPatch {
                                    visible: Some(!state),
                                    ..Default::default()
                                }
                            },
                        }
                        .into()
                    })
                    .collect();
                Plan::edit(commands, self.index.revision)
            }
            TrackHeaderAction::Disclosure => {
                let ids: Vec<_> = ids
                    .into_iter()
                    .filter(|id| {
                        self.index.layer(id).unwrap().kind != "group"
                            && self
                                .index
                                .layer(id)
                                .unwrap()
                                .channels
                                .iter()
                                .any(|c| !c.keys.is_empty())
                    })
                    .collect();
                let open = !ids.iter().any(|id| {
                    !self
                        .reveal
                        .layers
                        .get(id)
                        .map(|d| d.collapsed)
                        .unwrap_or(self.index.layer(id).unwrap().collapsed)
                });
                for id in ids {
                    self.reveal.layers.entry(id).or_default().collapsed = !open;
                }
                self.rebuild_rows();
                Plan::default()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropMode {
    Before,
    Inside,
    After,
}
impl TimelineModel {
    /// timeline.ts:3467–3492. Plan command-validated membership changes then an
    /// ID permutation on a scratch snapshot. This never writes a live document.
    pub fn drop_layers(
        &self,
        ids: &[String],
        target: &str,
        mode: DropMode,
        time: f64,
        origin: pm_kernel::Origin,
    ) -> Result<Plan, ModelError> {
        use pm_kernel::{Call, WriteBody, model};
        use pm_model::{Document, JsString, Op};
        finite(time)?;
        if ids.is_empty() {
            return Ok(Plan::default());
        }
        let target_layer = self
            .index
            .layer(target)
            .ok_or_else(|| ModelError::UnknownId(target.into()))?;
        if target_layer.effectively_locked {
            return Err(ModelError::Locked(target.into()));
        }
        if mode == DropMode::Inside && target_layer.kind != "group" {
            return Err(ModelError::InvalidGesture);
        }
        let expanded = self.index.expanded_ids(ids);
        let selected: HashSet<_> = expanded.iter().cloned().collect();
        if selected.contains(target) {
            return Err(ModelError::InvalidGesture);
        }
        for id in ids {
            self.index
                .layer(id)
                .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
        }
        for id in &expanded {
            if self.index.layer(id).unwrap().effectively_locked {
                return Err(ModelError::Locked(id.clone()));
            }
        }
        let roots: Vec<_> = self
            .index
            .layers
            .iter()
            .filter(|l| {
                ids.contains(&l.id) && !l.group.as_ref().is_some_and(|g| selected.contains(g))
            })
            .map(|l| l.id.clone())
            .collect();
        let group = if mode == DropMode::Inside {
            Some(target.into())
        } else {
            target_layer.group.clone()
        };
        let source = self
            .index
            .source
            .as_ref()
            .ok_or(ModelError::InvalidGesture)?;
        let mut scratch = Document::from_root(source.root().cloned(), source.revision);
        let mut ops = Vec::new();
        let changing: Vec<_> = roots
            .into_iter()
            .filter(|id| self.index.layer(id).unwrap().group != group)
            .collect();
        if !changing.is_empty() {
            let command: Command = GroupCommand::MoveToGroup {
                layer_ids: changing,
                group,
                time,
            }
            .into();
            let next = command
                .plan(scratch.root().unwrap(), origin)
                .map_err(|e| ModelError::KernelPlan(e.to_string()))?;
            scratch
                .apply_ops(&next)
                .map_err(|e| ModelError::KernelPlan(format!("{e:?}")))?;
            ops.extend(next);
        }
        let mut remaining: Vec<_> = self
            .index
            .layers
            .iter()
            .filter(|l| !selected.contains(&l.id))
            .map(|l| l.id.clone())
            .collect();
        let mut at = remaining
            .iter()
            .position(|id| id == target)
            .ok_or(ModelError::InvalidGesture)?;
        if mode != DropMode::Before {
            at += 1;
            if mode == DropMode::After {
                let descendants: HashSet<_> = self
                    .index
                    .expanded_ids(&[target.into()])
                    .into_iter()
                    .collect();
                while at < remaining.len() && descendants.contains(&remaining[at]) {
                    at += 1;
                }
            }
        }
        remaining.splice(at..at, expanded);
        let mut path = model::comp_path(self.index.comp_id.as_deref());
        path.push("layers".into());
        ops.push(Op::Order {
            path,
            ids: remaining
                .iter()
                .map(|id| JsString::from(id.as_str()))
                .collect(),
        });
        Ok(Plan {
            calls: vec![Call::Transact {
                label: Some("Move layers".into()),
                body: WriteBody::Ops(ops),
                expected_revision: Some(self.index.revision),
            }],
            expected_document: Some(self.index.doc_id),
            required_origin: Some(origin),
            ..Default::default()
        })
    }
    /// Track stack and preferences commit together using the kernel's native
    /// operation transaction. Target checks, origin and revision are fenced;
    /// document/history/journal authority stays in pm-kernel.
    pub fn move_tracks(
        &self,
        ids: &[String],
        delta: f64,
        lanes: crate::tracks::LaneDelta,
        origin: pm_kernel::Origin,
    ) -> Result<Plan, ModelError> {
        use pm_kernel::{Call, WriteBody, model};
        use pm_model::{Document, JsString, Op, Patch};
        if ids.is_empty() {
            return Ok(Plan::default());
        }
        let planned = crate::tracks::plan_move(&self.index, &self.reveal, ids, delta, lanes)?;
        let source = self
            .index
            .source
            .as_ref()
            .ok_or(ModelError::InvalidGesture)?;
        let mut scratch = Document::from_root(source.root().cloned(), source.revision);
        let mut ops = Vec::new();
        for l in &planned {
            let before = self.index.layer(&l.id).unwrap();
            if l.from != before.from {
                let cmd: Command = LayerCommand::Set {
                    layer_id: l.id.clone(),
                    patch: LayerPatch {
                        from: Some(l.from),
                        ..Default::default()
                    },
                }
                .into();
                let next = cmd
                    .plan(scratch.root().unwrap(), origin)
                    .map_err(|e| ModelError::KernelPlan(e.to_string()))?;
                scratch
                    .apply_ops(&next)
                    .map_err(|e| ModelError::KernelPlan(format!("{e:?}")))?;
                ops.extend(next);
            }
            if l.track != before.preferred_track {
                // Resting locked tracks cannot be silently rewritten either.
                if before.effectively_locked {
                    return Err(ModelError::Locked(l.id.clone()));
                }
                let loc = model::find_layer(scratch.root().unwrap(), &l.id)
                    .ok_or_else(|| ModelError::UnknownId(l.id.clone()))?;
                let mut path = loc.path();
                path.push("track".into());
                ops.push(Op::Leaf(Patch::set(path, Value::Number(l.track as f64))));
            }
        }
        let mut path = model::comp_path(self.index.comp_id.as_deref());
        path.push("layers".into());
        ops.push(Op::Order {
            path,
            ids: planned
                .iter()
                .map(|l| JsString::from(l.id.as_str()))
                .collect(),
        });
        Ok(Plan {
            calls: vec![Call::Transact {
                label: Some("Move clips".into()),
                body: WriteBody::Ops(ops),
                expected_revision: Some(self.index.revision),
            }],
            expected_document: Some(self.index.doc_id),
            required_origin: Some(origin),
            ..Default::default()
        })
    }
}

impl TimelineModel {
    /// timeline.ts:161–199: property commands retarget by effect/mask ordinal,
    /// compatible type and exact ownership. Unknown paths are never guessed.
    pub fn property_targets(
        &self,
        primary: &str,
        paths: &[String],
    ) -> Result<Vec<(String, String)>, ModelError> {
        let source = self
            .index
            .layer(primary)
            .ok_or_else(|| ModelError::UnknownId(primary.into()))?;
        let selected = self.selection.layers();
        let candidates: Vec<_> = if selected.iter().any(|id| id == primary) {
            self.index
                .layers
                .iter()
                .filter(|l| selected.contains(&l.id))
                .collect()
        } else {
            vec![source]
        };
        let mut result = Vec::new();
        for l in candidates {
            if l.effectively_locked {
                continue;
            }
            let mut parent = l.group.as_ref();
            let mut seen = HashSet::new();
            let mut implied = false;
            while let Some(id) = parent {
                if !seen.insert(id) {
                    break;
                }
                if selected.contains(id) {
                    implied = true;
                    break;
                }
                parent = self.index.layer(id).and_then(|l| l.group.as_ref());
            }
            if implied {
                continue;
            }
            for path in paths {
                let mapped = if path.starts_with("c.") {
                    Some(path.clone())
                } else if path.starts_with("l.") {
                    if l.kind != "audio" || path == "l.on" {
                        Some(path.clone())
                    } else {
                        None
                    }
                } else if path.starts_with("u.") || path.starts_with("x.") {
                    if source.kind == l.kind
                        && (source.kind != "extension" || source.definition == l.definition)
                    {
                        Some(path.clone())
                    } else {
                        None
                    }
                } else if ["g.", "mp.", "ta.", "ts."]
                    .iter()
                    .any(|p| path.starts_with(p))
                {
                    if source.id == l.id {
                        Some(path.clone())
                    } else {
                        None
                    }
                } else if let Some(rest) = path.strip_prefix("m.") {
                    rest.split_once('.').and_then(|(id, suffix)| {
                        source
                            .masks
                            .iter()
                            .position(|m| m == id)
                            .and_then(|i| l.masks.get(i))
                            .map(|id| format!("m.{id}.{suffix}"))
                    })
                } else if let Some((id, suffix)) = path.split_once('.') {
                    source
                        .effects
                        .iter()
                        .position(|fx| fx.0 == id)
                        .and_then(|i| {
                            l.effects
                                .get(i)
                                .filter(|fx| fx.1 == source.effects[i].1)
                                .map(|fx| format!("{}.{suffix}", fx.0))
                        })
                        .or_else(|| self.index.channel(&l.id, path).map(|_| path.clone()))
                } else {
                    Some(path.clone())
                };
                if let Some(path) = mapped
                    && self.index.channel(&l.id, &path).is_some()
                {
                    result.push((l.id.clone(), path));
                }
            }
        }
        Ok(result)
    }
    /// Shared inspector scale binding semantics, controls/binding.ts:39–67.
    /// The evaluated baselines are explicit inputs; no static-value approximation.
    pub fn scale_value(
        &self,
        layer: &str,
        axis: &str,
        value: f64,
        time: f64,
        primary: f64,
        other: f64,
    ) -> Result<Plan, ModelError> {
        for n in [value, time, primary, other] {
            finite(n)?;
        }
        if axis != "scale.x" && axis != "scale.y" {
            return Err(ModelError::UnknownId(axis.into()));
        }
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        self.require_channel(layer, axis)?;
        let paired = if axis == "scale.x" {
            "scale.y"
        } else {
            "scale.x"
        };
        let animated = [axis, paired]
            .iter()
            .filter_map(|p| self.index.channel(layer, p))
            .any(|c| !c.keys.is_empty());
        let mut set = SetProperty::new(layer, axis, value, time);
        set.preserve_hand_edits = false;
        set.mark_intent = true;
        if animated {
            set.mode = pm_kernel::commands::properties::KeyMode::Keyframe;
        }
        let mut commands = vec![PropertyCommand::Set(set.clone()).into()];
        if l.scale_linked || animated {
            set.channel = paired.into();
            set.value = Value::Number(if l.scale_linked {
                value
                    * if primary.abs() > 1e-8 {
                        other / primary
                    } else {
                        1.
                    }
            } else {
                other
            });
            commands.push(PropertyCommand::Set(set).into());
        }
        Ok(Plan::edit(commands, self.index.revision))
    }
}
