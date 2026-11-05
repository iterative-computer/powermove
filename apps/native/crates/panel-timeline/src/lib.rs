//! Headless timeline model and native paint plans. No platform dependencies or JS.
//! Document writes are requests to the kernel; see docs/native/panels/timeline.md.
#![forbid(unsafe_code)]

mod behaviour;
mod client;
pub mod graph_render;
mod interaction;
mod marquee;
pub mod render;
mod reveal;
mod session;
pub mod snap;
mod snapshot;
pub mod tracks;
pub mod v4;
mod v4_actions;

pub use behaviour::*;
pub use client::{ClientError, TimelineClient};
pub use interaction::*;
pub use marquee::*;
pub use reveal::{Disclosure, PropertyFacts, RevealState};
pub use snapshot::{Channel, Key, Layer, Marker, Row, RowId, TimelineIndex, Window};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pm_kernel::{Call, Caller, Command, Published, Request, Selection, SelectionCall, WriteBody};

/// Presentation facts only. DESIGN-TODO(opus): render these through the frozen v4 API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimelineMode {
    #[default]
    Layers,
    Tracks,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphMode {
    #[default]
    Value,
    Speed,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphDisplay {
    #[default]
    DopeSheet,
    Combined,
    Graph,
}

#[derive(Clone, Debug, Default)]
pub struct TimelineModel {
    pub index: Arc<TimelineIndex>,
    pub selection: Selection,
    pub mode: TimelineMode,
    pub graph_mode: GraphMode,
    pub graph_display: GraphDisplay,
    /// Curve focus survives layer/strip clicks (timeline.ts:3110,3704).
    pub graph_focus: Option<(String, String)>,
    pub reveal: RevealState,
    pub key_selection_active: bool,
    pub track_snap_disabled: bool,
    pub search: String,
    pub property_labels: HashMap<(String, String), String>,
    pub(crate) anchor: Option<String>,
    rows: Vec<Row>,
    row_positions: HashMap<RowId, usize>,
    pub rebuilds: u64,
}

impl TimelineModel {
    /// Consume immutable publication; selection/transport-only changes never scan rows.
    pub fn sync(&mut self, published: &Published) {
        let new_document = self.index.doc_id != published.snapshot.doc_id;
        let changed = self.index.doc_id != published.snapshot.doc_id
            || self.index.revision != published.revision();
        if new_document {
            *self = Self::default();
        }
        let selection_changed = self.selection.layers() != published.selection.layers();
        self.selection = published.selection.clone();
        if changed {
            self.index = Arc::new(TimelineIndex::from_snapshot(&published.snapshot));
            self.rebuild_rows();
            if new_document {
                self.restore_ui(&published.ui_state);
            }
        }
        if selection_changed {
            let mut seen = HashSet::new();
            let mut opened = false;
            for id in self.selection.layers() {
                let mut at = self.index.layer(id).and_then(|l| l.group.clone());
                while let Some(id) = at {
                    if !seen.insert(id.clone()) {
                        break;
                    }
                    let base = self.index.layer(&id);
                    let d = self
                        .reveal
                        .layers
                        .entry(id.clone())
                        .or_insert_with(|| Disclosure {
                            collapsed: base.map(|l| l.collapsed).unwrap_or(true),
                            group_collapsed: base.map(|l| l.group_collapsed).unwrap_or(true),
                            paths: None,
                        });
                    opened |= d.group_collapsed;
                    d.group_collapsed = false;
                    at = base.and_then(|l| l.group.clone());
                }
            }
            if opened {
                self.rebuild_rows();
            }
        }
    }

    pub fn rebuild_rows(&mut self) {
        self.rows.clear();
        // Index parent links once; avoid an O(depth * layers) ancestor walk.
        let children = self.index.children();
        let mut stack: Vec<(usize, usize)> = self
            .index
            .roots(&children)
            .into_iter()
            .rev()
            .map(|i| (i, 0))
            .collect();
        let mut seen = HashSet::new();
        while let Some((i, depth)) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let layer = &self.index.layers[i];
            let disclosure = self.reveal.layers.get(&layer.id);
            let collapsed = disclosure.map(|d| d.collapsed).unwrap_or(layer.collapsed);
            let mut properties = Vec::new();
            let has_scale = layer.channels.iter().any(|c| c.path == "scale.x")
                && layer.channels.iter().any(|c| c.path == "scale.y");
            let position = self.track_channels(&layer.id, "position");
            let joined_position = !layer.separate_dimensions && position.len() >= 2;
            let first_position = position.first().map(|c| c.path.as_str());
            for channel in &layer.channels {
                if !layer.three_d
                    && [
                        "perspective",
                        "position.z",
                        "anchor.z",
                        "scale.z",
                        "rotation.x",
                        "rotation.y",
                        "orientation.x",
                        "orientation.y",
                        "orientation.z",
                    ]
                    .contains(&channel.path.as_str())
                {
                    continue;
                }
                if has_scale && channel.path == "scale.y" {
                    continue;
                }
                if joined_position
                    && channel.path.starts_with("position.")
                    && Some(channel.path.as_str()) != first_position
                {
                    continue;
                }
                let visible = (joined_position
                    && Some(channel.path.as_str()) == first_position
                    && position.iter().any(|c| self.reveal.visible(&layer.id, c)))
                    || self.reveal.visible(&layer.id, channel)
                    || (has_scale
                        && channel.path == "scale.x"
                        && layer
                            .channels
                            .iter()
                            .find(|c| c.path == "scale.y")
                            .is_some_and(|c| self.reveal.visible(&layer.id, c)));
                if visible {
                    properties.push(
                        if joined_position && Some(channel.path.as_str()) == first_position {
                            "position".to_owned()
                        } else if has_scale && channel.path == "scale.x" {
                            "scale".to_owned()
                        } else {
                            channel.path.clone()
                        },
                    );
                }
            }
            let query = self.search.trim().to_lowercase();
            let matching: Vec<_> = properties
                .iter()
                .filter(|path| {
                    self.property_labels
                        .get(&(layer.id.clone(), (*path).clone()))
                        .unwrap_or(path)
                        .to_lowercase()
                        .contains(&query)
                })
                .cloned()
                .collect();
            let include = query.is_empty()
                || layer.name.to_string_lossy().to_lowercase().contains(&query)
                || !matching.is_empty();
            if include {
                self.rows.push(Row {
                    id: RowId::Layer(layer.id.clone()),
                    depth,
                });
                if !collapsed || !query.is_empty() {
                    for path in if !query.is_empty() && !matching.is_empty() {
                        matching
                    } else {
                        properties
                    } {
                        self.rows.push(Row {
                            id: RowId::Property {
                                layer_id: layer.id.clone(),
                                channel: path,
                            },
                            depth: depth + 1,
                        });
                    }
                }
            }
            let group_closed = disclosure
                .map(|d| d.group_collapsed)
                .unwrap_or(layer.group_collapsed);
            if (!group_closed || !self.search.is_empty())
                && let Some(kids) = children.get(&Some(layer.id.clone()))
            {
                stack.extend(kids.iter().rev().map(|i| (*i, depth + 1)));
            }
        }
        self.row_positions = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        self.rebuilds += 1;
    }

    /// Ordinal virtualisation: O(window), no pixel/row-height choices. Empty/out-of-range safe.
    pub fn window(&self, start: usize, count: usize, overscan: usize) -> Window<'_> {
        let start = start.min(self.rows.len());
        let first = start.saturating_sub(overscan);
        let end = start
            .saturating_add(count)
            .saturating_add(overscan)
            .min(self.rows.len());
        Window {
            total: self.rows.len(),
            first,
            rows: &self.rows[first..end],
        }
    }
    pub fn set_search(&mut self, query: impl Into<String>) {
        self.search = query.into();
        self.rebuild_rows();
    }
    pub fn track_channels(&self, layer: &str, path: &str) -> Vec<&Channel> {
        if path == "position" {
            ["position.x", "position.y", "position.z"]
                .iter()
                .filter(|p| {
                    **p != "position.z" || self.index.layer(layer).is_some_and(|l| l.three_d)
                })
                .filter_map(|p| self.index.channel(layer, p))
                .collect()
        } else if path == "scale" {
            ["scale.x", "scale.y"]
                .iter()
                .filter_map(|c| self.index.channel(layer, c))
                .collect()
        } else {
            self.index.channel(layer, path).into_iter().collect()
        }
    }
    pub fn row_position(&self, id: &RowId) -> Option<usize> {
        self.row_positions.get(id).copied()
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// timeline.ts:3110–3135. Shift ranges only visible layer rows and keeps its anchor.
    pub fn select_layer(&mut self, id: &str, mods: Modifiers) -> Result<Vec<Call>, ModelError> {
        self.index
            .layer(id)
            .ok_or_else(|| ModelError::UnknownId(id.into()))?;
        let ids: Vec<_> = self
            .rows
            .iter()
            .filter_map(|r| match &r.id {
                RowId::Layer(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        let current = self.selection.layers();
        let mut next = current.to_vec();
        if mods.shift {
            if self
                .anchor
                .as_ref()
                .is_none_or(|a| !ids.contains(a) || !current.contains(a))
            {
                self.anchor = current
                    .iter()
                    .find(|id| ids.contains(id))
                    .cloned()
                    .or_else(|| Some(id.into()));
            }
            let a = ids
                .iter()
                .position(|x| Some(x) == self.anchor.as_ref())
                .ok_or(ModelError::HiddenRow)?;
            let b = ids
                .iter()
                .position(|x| x == id)
                .ok_or(ModelError::HiddenRow)?;
            let range = &ids[a.min(b)..=a.max(b)];
            if mods.command() {
                for id in range {
                    if !next.contains(id) {
                        next.push(id.clone());
                    }
                }
            } else {
                next = range.to_vec();
            }
        } else {
            self.anchor = Some(id.into());
            if mods.command() {
                if next.iter().any(|x| x == id) {
                    next.retain(|x| x != id);
                } else {
                    next.push(id.into());
                }
            } else if !next.iter().any(|x| x == id) {
                next = vec![id.into()];
            }
        }
        self.key_selection_active = false;
        Ok(vec![
            SelectionCall::Layers {
                ids: next,
                add: false,
            }
            .into(),
        ])
    }

    /// A key click stages an additive drag selection; pointer-up may toggle the hit off.
    /// timeline.ts:3704 and graph-selection.ts:selectionAfterKeyGesture.
    pub fn select_keys(
        &mut self,
        layer: &str,
        channel: &str,
        hit: &[String],
        additive: bool,
        dragging: bool,
    ) -> Result<Vec<Call>, ModelError> {
        let rows = self.track_channels(layer, channel);
        if rows.is_empty() {
            return Err(ModelError::UnknownId(channel.into()));
        }
        if hit
            .iter()
            .any(|id| !rows.iter().any(|row| row.keys.iter().any(|k| &k.id == id)))
        {
            return Err(ModelError::UnknownId("key".into()));
        }
        let expanded = self.expand_scale_keys(hit);
        let hit = expanded.as_slice();
        let before = self.selection.keys();
        let all = hit.iter().any(|id| before.contains(id));
        let mut next = if additive || all {
            before.to_vec()
        } else {
            Vec::new()
        };
        if additive && !dragging && all {
            next.retain(|id| !hit.contains(id));
        } else {
            for id in hit {
                if !next.contains(id) {
                    next.push(id.clone());
                }
            }
        }
        self.graph_focus = Some((layer.into(), channel.into()));
        self.key_selection_active = true;
        let mut calls = Vec::new();
        if additive || !self.selection.contains(layer) {
            calls.push(
                SelectionCall::Layers {
                    ids: vec![layer.into()],
                    add: additive,
                }
                .into(),
            );
        }
        calls.push(SelectionCall::Channel(Some(channel.into())).into());
        calls.push(SelectionCall::Keys(next).into());
        Ok(calls)
    }

    /// Semantic marquee: caller supplies intersected IDs after hit testing. Never seeks.
    pub fn marquee(&mut self, layers: Vec<String>, keys: Vec<String>, additive: bool) -> Vec<Call> {
        self.key_selection_active = !keys.is_empty();
        let mut selected_keys = if additive {
            self.selection.keys().to_vec()
        } else {
            Vec::new()
        };
        for key in unique_ids(keys) {
            if additive && selected_keys.contains(&key) {
                selected_keys.retain(|id| id != &key);
            } else if !selected_keys.contains(&key) {
                selected_keys.push(key);
            }
        }
        vec![
            SelectionCall::Layers {
                ids: layers,
                add: additive,
            }
            .into(),
            SelectionCall::Keys(selected_keys).into(),
        ]
    }

    pub fn toggle_disclosure(&mut self, id: &str) -> Result<(), ModelError> {
        let l = self
            .index
            .layer(id)
            .ok_or_else(|| ModelError::UnknownId(id.into()))?;
        let d = self
            .reveal
            .layers
            .entry(id.into())
            .or_insert_with(|| Disclosure {
                collapsed: l.collapsed,
                group_collapsed: l.group_collapsed,
                paths: None,
            });
        if l.kind == "group" {
            d.group_collapsed = !d.group_collapsed;
        } else {
            d.collapsed = !d.collapsed;
        }
        self.rebuild_rows();
        Ok(())
    }

    pub fn adjacent_key(&self, direction: i8, time: f64, row: Option<(&str, &str)>) -> Option<f64> {
        let mut times = Vec::new();
        for layer in &self.index.layers {
            if row.is_none()
                && !self.selection.layers().is_empty()
                && !self.selection.contains(&layer.id)
            {
                continue;
            }
            for channel in &layer.channels {
                if let Some((l, c)) = row {
                    if layer.id != l
                        || (channel.path != c
                            && !self
                                .track_channels(l, c)
                                .iter()
                                .any(|a| a.path == channel.path))
                    {
                        continue;
                    }
                } else if let Some(c) = self.selection.channel()
                    && channel.path != c
                    && !self
                        .track_channels(&layer.id, c)
                        .iter()
                        .any(|a| a.path == channel.path)
                {
                    continue;
                }
                times.extend(
                    channel
                        .keys
                        .iter()
                        .map(|key| layer.from + key.time)
                        .filter(|t| t.is_finite() && *t >= 0. && *t <= self.index.duration),
                );
            }
        }
        times.sort_by(f64::total_cmp);
        if direction > 0 {
            times.into_iter().find(|t| *t > time + 1e-5)
        } else {
            times.into_iter().rev().find(|t| *t < time - 1e-5)
        }
    }

    pub fn transact(&self, caller: Caller, commands: Vec<Command>, txid: u64) -> Request {
        Request::new(
            caller,
            Call::Transact {
                label: None,
                body: WriteBody::Commands(commands),
                expected_revision: Some(self.index.revision),
            },
        )
        .with_txid(txid)
        .with_document(self.index.doc_id)
    }
}
