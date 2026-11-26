//! Headless timeline model and native paint plans. No platform dependencies or JS.
//! Document writes are requests to the kernel; see docs/native/panels/timeline.md.
#![forbid(unsafe_code)]

pub mod agent_ink;
mod behaviour;
mod client;
pub mod footer;
pub mod graph_render;
mod interaction;
mod marquee;
pub mod render;
mod reveal;
mod session;
pub mod snap;
pub mod level;
pub mod waveform;
mod snapshot;
pub mod stagger;
pub mod tracks;
pub mod v4;
mod v4_actions;

pub use behaviour::*;
pub use client::{ClientError, KeyMovePreview, LayerPreviewFrame, TimelineClient};
pub use interaction::*;
pub use marquee::*;
pub use reveal::{Disclosure, PropertyFacts, RevealState, RevealTarget};
pub use snapshot::{Channel, Key, Layer, Marker, Row, RowId, TimelineIndex, Transition, Window};

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
/// What a drag on a clip does (Premiere's B N Y U, Final Cut's T): a trim
/// or move with Selection; the pro edits otherwise, in both timeline modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditTool {
    #[default]
    Selection,
    /// Click to split at the pointer; Shift splits all overlapping layers.
    Split,
    /// An edge drag ripples: what follows on the track moves with it.
    Ripple,
    /// An edge drag rolls the edit between two clips.
    Roll,
    /// A drag changes which source frames the clip shows.
    Slip,
    /// A drag moves the clip between its neighbours.
    Slide,
}
impl EditTool {
    pub const ALL: [(Self, &'static str); 6] = [
        (Self::Selection, "selection"),
        (Self::Split, "split"),
        (Self::Ripple, "ripple"),
        (Self::Roll, "roll"),
        (Self::Slip, "slip"),
        (Self::Slide, "slide"),
    ];
    /// The tool's command (`timeline.tool:ripple`).
    pub fn command(self) -> String {
        let name = Self::ALL
            .iter()
            .find(|(t, _)| *t == self)
            .map_or("", |(_, n)| n);
        format!("timeline.tool:{name}")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphDisplay {
    #[default]
    DopeSheet,
    Combined,
    Graph,
}

#[derive(Debug)]
struct CachedTrackLayout {
    index: Arc<TimelineIndex>,
    held: HashMap<String, reveal::Held>,
    focused_group: Option<String>,
    layout: Arc<tracks::Layout>,
}

/// The focused group id stored in the timeline's persisted UI state, if any.
pub fn focused_group_from_ui(ui: &pm_kernel::state::UiState) -> Option<&str> {
    ui.timeline
        .get("nativeModel")
        .and_then(|v| v.get("focusedGroup"))
        .and_then(pm_model::Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Validates the focused group stored in `published.ui_state` against the active
/// composition in `published.snapshot`.
pub fn focused_group_for_published(published: &Published) -> Option<String> {
    let id = focused_group_from_ui(&published.ui_state)?;
    let root = published.snapshot.root()?;
    let layers = pm_kernel::model::list_layers(root, None);
    layers
        .iter()
        .any(|l| l.id == id && l.kind == "group")
        .then(|| id.to_owned())
}

#[derive(Clone, Debug, Default)]
pub struct TimelineModel {
    pub index: Arc<TimelineIndex>,
    pub selection: Selection,
    /// Focused group layer id (double-clicked to edit like a composition).
    pub focused_group: Option<String>,
    /// Transient value field; never persisted as a reveal request.
    pub editing_property: Option<(String, String)>,
    pub mode: TimelineMode,
    pub graph_mode: GraphMode,
    pub graph_display: GraphDisplay,
    /// Curve focus survives layer/strip clicks (timeline.ts:3110,3704).
    pub graph_focus: Option<(String, String)>,
    /// Merge Selected Curves: the graph's curves drawn as the one motion
    /// they describe together. A view of the same keys, not saved.
    pub graph_merge: bool,
    /// The curves the graph showed for the last key selection (v1's
    /// `graphShown`): deselecting every key keeps them on screen.
    pub graph_shown: Vec<(String, String)>,
    pub reveal: RevealState,
    pub key_selection_active: bool,
    pub track_snap_disabled: bool,
    /// The timeline's edit tool; a session choice, not saved.
    pub edit_tool: EditTool,
    /// Hide Shy Layers: shy layers (and what a shy group holds) have no row.
    /// They still draw, play and export.
    pub hide_shy: bool,
    /// The timeline panel does not hold the keyboard: selected rows take
    /// the neutral selection fill, not the accent (PM-367).
    pub panel_blurred: bool,
    pub search: String,
    /// The transition selected on a clip's edge (its layer is selected
    /// too): Delete removes it. A view choice, not saved.
    pub transition: Option<(String, crate::render::Edge)>,
    pub property_labels: HashMap<(String, String), String>,
    pub(crate) anchor: Option<String>,
    rows: Arc<Vec<Row>>,
    row_positions: Arc<HashMap<RowId, usize>>,
    track_layout: Arc<std::sync::Mutex<Option<CachedTrackLayout>>>,
    pub rebuilds: u64,
    /// Monotonic request, consumed once by the native scroll host.
    pub reveal_serial: u64,
    pub reveal_target: Option<RevealTarget>,
    /// The published timeline UI last restored. The native tracks and the
    /// outline each hold a model; a disclosure one publishes reaches the other.
    ui_seen: Option<pm_model::json::Value>,
    history_applied: usize,
    history_entries: usize,
}

impl TimelineModel {
    /// Lane packing depends on the immutable index and the held lane assignments.
    /// Model clones used by the canvas and outline share the packed result.
    pub fn track_layout(&self) -> Arc<tracks::Layout> {
        let focus = self.active_focused_group().map(str::to_owned);
        let mut cached = self.track_layout.lock().unwrap();
        if let Some(entry) = &*cached
            && Arc::ptr_eq(&entry.index, &self.index)
            && entry.held == self.reveal.held
            && entry.focused_group == focus
        {
            return entry.layout.clone();
        }
        let layout = Arc::new(tracks::layout_focused(
            &self.index,
            &self.reveal,
            focus.as_deref(),
        ));
        *cached = Some(CachedTrackLayout {
            index: self.index.clone(),
            held: self.reveal.held.clone(),
            focused_group: focus,
            layout: layout.clone(),
        });
        layout
    }

    /// The currently focused group layer id, if it still exists as a group in
    /// the active composition.
    pub fn active_focused_group(&self) -> Option<&str> {
        let id = self.focused_group.as_deref()?;
        self.index
            .layer(id)
            .is_some_and(|l| l.kind == "group")
            .then_some(id)
    }

    /// Outermost-to-innermost chain of focused groups (empty when viewing the
    /// whole composition).
    pub fn focused_group_chain(&self) -> Vec<&Layer> {
        let Some(mut cur) = self.active_focused_group() else {
            return Vec::new();
        };
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        while seen.insert(cur) {
            let Some(layer) = self.index.layer(cur) else {
                break;
            };
            if layer.kind != "group" {
                break;
            }
            chain.push(layer);
            match layer.group.as_deref() {
                Some(parent) => cur = parent,
                None => break,
            }
        }
        chain.reverse();
        chain
    }

    /// Focus a group layer like a composition (`None` returns to the active
    /// composition root).
    pub fn set_focused_group(&mut self, group: Option<String>) -> Result<(), ModelError> {
        if let Some(id) = group.as_deref()
            && !self.index.layer(id).is_some_and(|l| l.kind == "group")
        {
            return Err(ModelError::UnknownId(id.into()));
        }
        self.focused_group = group;
        self.rebuild_rows();
        Ok(())
    }

    pub fn follows_publication(&self, published: &Published) -> bool {
        self.follows_snapshot(&published.snapshot)
            && self.ui_seen.as_ref().is_some_and(|seen| {
                crate::session::same_model_ui(seen, &published.ui_state.timeline)
            })
            && self.selection == published.selection
    }

    fn follows_snapshot(&self, snapshot: &pm_kernel::Snapshot) -> bool {
        self.index.source.as_ref().is_some_and(|s| {
            s.doc_id == snapshot.doc_id
                && s.revision == snapshot.revision
                && s.document.edition() == snapshot.document.edition()
        })
    }

    /// Consume immutable publication; selection/transport-only changes never scan rows.
    pub fn sync(&mut self, published: &Published) {
        let new_document = self.index.doc_id != published.snapshot.doc_id;
        let changed = !self.follows_snapshot(&published.snapshot);
        let same_composition =
            !new_document && self.index.comp_id == published.snapshot.open_comp();
        if new_document {
            *self = Self::default();
        }
        let selection_changed = self.selection.layers() != published.selection.layers();
        let keys_changed = self.selection.keys() != published.selection.keys()
            || self.selection.key_layers() != published.selection.key_layers();
        let selection_step = !changed
            && self.history_applied != published.history.applied
            && self.history_entries == published.history.entries;
        self.history_applied = published.history.applied;
        self.history_entries = published.history.entries;
        self.selection = published.selection.clone();
        let previous = (changed && same_composition).then(|| self.index.clone());
        let agent_edit = changed
            && published.changes.last().is_some_and(|change| {
                if change.revision != published.revision() {
                    return false;
                }
                use pm_kernel::{ChangeCause, commands::Origin};
                matches!(
                    change.cause,
                    ChangeCause::Write {
                        origin: Origin::Agent | Origin::Mcp,
                        ..
                    } | ChangeCause::Edit {
                        origin: Origin::Agent | Origin::Mcp,
                        ..
                    }
                )
            });
        let entry = published.entry.as_ref().map(|e| e.id);
        if entry != self.reveal.entry || entry.is_none() {
            self.reveal.entry = entry;
            self.reveal.revealed_in_entry.clear();
        }
        if changed {
            // Tracks mode: the lanes shown now hold through whatever changed
            // the document (the timeline, a menu, Properties, an agent).
            if self.mode == TimelineMode::Tracks && same_composition {
                self.reveal.held = crate::tracks::shown(&self.index, &self.reveal);
            } else {
                self.reveal.held.clear();
            }
            if !same_composition {
                self.focused_group = None;
            }
            self.index = TimelineIndex::shared(&published.snapshot, &self.index);
            if self.focused_group.is_some() && self.active_focused_group().is_none() {
                self.focused_group = None;
            }
            self.rebuild_rows();
        }
        // A transition stays selected until a selection leaves out its layer,
        // or the layer no longer has it.
        if let Some((layer, edge)) = &self.transition
            && (selection_changed && !self.selection.contains(layer)
                || self
                    .index
                    .layer(layer)
                    .is_none_or(|l| l.transitions[*edge as usize].is_none()))
        {
            self.transition = None;
        }
        if self.ui_seen.as_ref().is_none_or(|seen| {
            !crate::session::same_model_ui(seen, &published.ui_state.timeline)
        }) {
            self.ui_seen = Some(published.ui_state.timeline.clone());
            self.restore_ui(&published.ui_state);
        }
        // Deleting the last selected key keeps key mode: a second Delete
        // must not remove its layer. Explicit selection changes and history
        // navigation still restore the mode from their selected keys.
        if selection_step || (keys_changed && (!changed || !self.selection.keys().is_empty())) {
            self.key_selection_active = !self.selection.keys().is_empty();
        }
        if let Some(previous) = previous {
            let mut added = Vec::new();
            let mut removed = Vec::new();
            let mut closed_empty_strip = false;
            for layer in &self.index.layers {
                let Some(old_layer) = previous.layer(&layer.id) else {
                    continue;
                };
                if std::ptr::eq(old_layer, layer.as_ref()) {
                    continue;
                }
                // An older document can have an empty, open disclosure.
                // Adding a ready-animated effect or enabling a saved 3D
                // channel must not turn that into a visible strip.
                if !old_layer.has_keyframed_channels() && layer.has_keyframed_channels() {
                    let d = self.reveal.layers.entry(layer.id.clone()).or_insert_with(|| Disclosure {
                        collapsed: layer.collapsed,
                        group_collapsed: layer.group_collapsed,
                        paths: None,
                    });
                    if d.paths.is_none() && !d.collapsed {
                        d.collapsed = true;
                        closed_empty_strip = true;
                    }
                }
                // Reparenting and timing edits can generate keys to preserve
                // the layer's pose or the clip's boundary values.
                let structural_edit = old_layer.parent != layer.parent
                    || old_layer.group != layer.group
                    || old_layer.from != layer.from
                    || old_layer.duration != layer.duration;
                for channel in &layer.channels {
                    // New effect and mask channels may arrive with saved keys.
                    // Reveal only keys added to a channel already on this layer.
                    let Some(old) = previous.channel(&layer.id, &channel.path) else {
                        continue;
                    };
                    if channel.keys.is_empty() {
                        if !old.keys.is_empty() {
                            removed.push((layer.id.clone(), channel.path.clone()));
                        }
                        continue;
                    }
                    // A switch can insert a key on an already animated flag.
                    if structural_edit
                        || channel.path.starts_with("l.")
                        || channel.path.ends_with(".$enabled")
                        || channel.path == "c.audioMuted"
                        || !layer.channel_available(&channel.path)
                    {
                        continue;
                    }
                    let old_keys: HashSet<_> = old.keys.iter().map(|key| key.id.as_str()).collect();
                    if channel.keys.iter().any(|key| !old_keys.contains(key.id.as_str())) {
                        added.push((layer.id.clone(), channel.path.clone()));
                    }
                }
            }
            let disclosure_changed = closed_empty_strip || !added.is_empty() || !removed.is_empty();
            // Keep the final diamond's row under the pointer without opening
            // anything on deletion. A later animated-only refresh replaces it.
            for (layer, path) in removed {
                let l = self.index.layer(&layer).unwrap();
                let d = self
                    .reveal
                    .layers
                    .entry(layer.clone())
                    .or_insert_with(|| Disclosure {
                        collapsed: l.collapsed,
                        group_collapsed: l.group_collapsed,
                        paths: None,
                    });
                let paths = d.paths.get_or_insert_with(|| {
                    l.channels
                        .iter()
                        .filter(|c| !c.keys.is_empty())
                        .map(|c| c.path.clone())
                        .collect()
                });
                if !paths.contains(&path) {
                    self.reveal.retained_empty.insert((layer, path.clone()));
                    paths.push(path);
                }
            }
            for (layer, path) in added {
                self.reveal.retained_empty.remove(&(layer.clone(), path.clone()));
                // An agent can animate layers without taking over the user's
                // outline or changing which descendants are visible.
                if agent_edit && self.has_closed_group(&layer) {
                    continue;
                }
                if entry.is_some() && !self.reveal.revealed_in_entry.insert((layer.clone(), path.clone())) {
                    continue;
                }
                if self
                    .reveal
                    .layers
                    .get(&layer)
                    .is_some_and(|d| d.paths.is_some())
                {
                    let _ = self.include_paths(&layer, std::slice::from_ref(&path));
                } else {
                    let _ = self.open_property_rows(&layer, None);
                }
                self.reveal_serial += 1;
                self.reveal_target = Some(RevealTarget::Property { layer, path });
            }
            if disclosure_changed {
                self.rebuild_rows();
            }
        }
        if selection_changed && !agent_edit {
            let selected = self.selection.layers().to_vec();
            self.expand_ancestors(&selected);
        }
    }

    fn expand_ancestors(&mut self, layers: &[String]) {
        let mut seen = HashSet::new();
        let mut opened = false;
        for layer in layers {
            let mut at = self.index.layer(layer).and_then(|l| l.group.as_deref());
            while let Some(id) = at {
                let Some(group) = self.index.layer(id).filter(|_| seen.insert(id)) else {
                    break;
                };
                let d = self.reveal.layers.entry(id.to_owned()).or_insert_with(|| Disclosure {
                    collapsed: group.collapsed,
                    group_collapsed: group.group_collapsed,
                    paths: None,
                });
                opened |= d.group_collapsed;
                d.group_collapsed = false;
                at = group.group.as_deref();
            }
        }
        if opened {
            self.rebuild_rows();
        }
    }

    /// Open the group chain even if the palette chose an already-selected layer.
    pub fn reveal_layer(&mut self, layer: String) {
        // Tracks show lanes and clips. The named layer has its own row in Layers.
        self.mode = TimelineMode::Layers;
        self.expand_ancestors(std::slice::from_ref(&layer));
        self.reveal_serial += 1;
        self.reveal_target = Some(RevealTarget::Layer(layer));
    }

    fn has_closed_group(&self, layer: &str) -> bool {
        let mut ancestor = self.index.layer(layer).and_then(|l| l.group.as_deref());
        let mut seen = HashSet::new();
        while let Some(id) = ancestor {
            if !seen.insert(id) {
                break;
            }
            let parent = self.index.layer(id);
            if self
                .reveal
                .layers
                .get(id)
                .map(|d| d.group_collapsed)
                .unwrap_or_else(|| parent.is_some_and(|group| group.group_collapsed))
            {
                return true;
            }
            ancestor = parent.and_then(|group| group.group.as_deref());
        }
        false
    }

    pub fn rebuild_rows(&mut self) {
        self.reveal.retained_empty.retain(|(layer, path)| {
            let collapsed = self.reveal.layers.get(layer).map(|d| d.collapsed)
                .unwrap_or_else(|| self.index.layer(layer).is_none_or(|l| l.collapsed));
            if collapsed {
                if let Some(paths) = self.reveal.layers.get_mut(layer).and_then(|d| d.paths.as_mut()) {
                    paths.retain(|p| p != path);
                }
            }
            !collapsed
        });
        let mut rows = Vec::new();
        // Index parent links once; avoid an O(depth * layers) ancestor walk.
        let children = self.index.children();
        let roots = if let Some(gid) = self.active_focused_group() {
            children
                .get(&Some(gid.to_owned()))
                .cloned()
                .unwrap_or_default()
        } else {
            self.index.roots(&children)
        };
        let mut stack: Vec<(usize, usize)> =
            roots.into_iter().rev().map(|i| (i, 0)).collect();
        let mut seen = HashSet::new();
        let query = self.search.trim().to_lowercase();
        while let Some((i, depth)) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let layer = &self.index.layers[i];
            if self.hide_shy && layer.shy {
                continue;
            }
            let disclosure = self.reveal.layers.get(&layer.id);
            let collapsed = disclosure.map(|d| d.collapsed).unwrap_or(layer.collapsed);
            let group_closed = disclosure
                .map(|d| d.group_collapsed)
                .unwrap_or(layer.group_collapsed);
            let kids = (!group_closed || !self.search.is_empty())
                .then(|| children.get(&Some(layer.id.clone())))
                .flatten();
            // A closed layer with no search shows its row alone.
            if collapsed && query.is_empty() {
                rows.push(Row {
                    id: RowId::Layer(layer.id.clone()),
                    depth,
                });
                if let Some(kids) = kids {
                    stack.extend(kids.iter().rev().map(|i| (*i, depth + 1)));
                }
                continue;
            }
            let mut properties = Vec::new();
            let scale = self.track_channels(&layer.id, "scale");
            let joined_scale = scale.len() >= 2;
            let first_scale = scale.first().map(|c| c.path.as_str());
            let position = self.track_channels(&layer.id, "position");
            let joined_position = !layer.separate_dimensions && position.len() >= 2;
            let first_position = position.first().map(|c| c.path.as_str());
            for channel in &layer.channels {
                if !layer.channel_available(&channel.path) {
                    continue;
                }
                if joined_scale
                    && channel.path.starts_with("scale.")
                    && Some(channel.path.as_str()) != first_scale
                {
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
                    && position.iter().any(|c| self.property_visible(&layer.id, c)))
                    || self.property_visible(&layer.id, channel)
                    || (joined_scale
                        && Some(channel.path.as_str()) == first_scale
                        && scale.iter().any(|c| self.property_visible(&layer.id, c)));
                if visible {
                    properties.push(
                        if joined_position && Some(channel.path.as_str()) == first_position {
                            "position".to_owned()
                        } else if joined_scale && Some(channel.path.as_str()) == first_scale {
                            "scale".to_owned()
                        } else {
                            channel.path.clone()
                        },
                    );
                }
            }
            let matching: Vec<_> = properties
                .iter()
                .filter(|path| {
                    self.property_labels
                        .get(&(layer.id.clone(), (*path).clone()))
                        .map(String::as_str)
                        .or_else(|| {
                            self.index.channel(&layer.id, path)?.vertex_label.as_deref()
                        })
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
                rows.push(Row {
                    id: RowId::Layer(layer.id.clone()),
                    depth,
                });
                if !collapsed || !query.is_empty() {
                    let mut current_section = None;
                    for path in if !query.is_empty() && !matching.is_empty() {
                        matching
                    } else {
                        properties
                    } {
                        let section = self
                            .index
                            .channel(&layer.id, &path)
                            .and_then(|channel| channel.path_group.as_ref())
                            .map(|group| RowId::Path {
                                layer_id: layer.id.clone(),
                                path: group.clone(),
                            })
                            .or_else(|| {
                                layer
                                    .effects
                                    .iter()
                                    .find(|(id, _)| path.starts_with(&format!("{id}.")))
                                    .map(|(id, _)| RowId::Effect {
                                        layer_id: layer.id.clone(),
                                        effect: id.clone(),
                                    })
                            });
                        if section != current_section && let Some(id) = &section {
                            rows.push(Row {
                                id: id.clone(),
                                depth: depth + 1,
                            });
                        }
                        let property_depth = depth + if section.is_some() { 2 } else { 1 };
                        current_section = section;
                        rows.push(Row {
                            id: RowId::Property {
                                layer_id: layer.id.clone(),
                                channel: path,
                            },
                            depth: property_depth,
                        });
                    }
                }
            }
            if let Some(kids) = kids {
                stack.extend(kids.iter().rev().map(|i| (*i, depth + 1)));
            }
        }
        self.row_positions = Arc::new(rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect());
        self.rows = Arc::new(rows);
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
        let three_d = self.index.layer(layer).is_some_and(|l| l.three_d);
        if path == "position" {
            ["position.x", "position.y", "position.z"]
                .iter()
                .filter(|p| **p != "position.z" || three_d)
                .filter_map(|p| self.index.channel(layer, p))
                .collect()
        } else if path == "scale" {
            ["scale.x", "scale.y", "scale.z"]
                .iter()
                .filter(|p| **p != "scale.z" || three_d)
                .filter_map(|c| self.index.channel(layer, c))
                .collect()
        } else if path == "anchor" {
            ["anchor.x", "anchor.y", "anchor.z"]
                .iter()
                .filter(|p| **p != "anchor.z" || three_d)
                .filter_map(|c| self.index.channel(layer, c))
                .collect()
        } else if path == "orientation" && three_d {
            ["orientation.x", "orientation.y", "orientation.z"]
                .iter()
                .filter_map(|c| self.index.channel(layer, c))
                .collect()
        } else {
            self.index.channel(layer, path).into_iter().collect()
        }
    }
    /// The property the graph plots: the focused curve, else the selected
    /// layer's selected property, else the first revealed one. A revealed
    /// row counts while it is animated or is the selected property (v1's
    /// `drawGraph` rows): a static row first in the list must not leave the
    /// graph empty.
    pub fn graph_property(&self) -> Option<(String, String)> {
        let mut candidates: Vec<(String, String)> = vec![];
        for row in self.rows() {
            if let RowId::Property { layer_id, channel } = &row.id
                && (self.selection.channel() == Some(channel.as_str())
                    || self
                        .track_channels(layer_id, channel)
                        .iter()
                        .any(|c| !c.keys.is_empty()))
            {
                candidates.push((layer_id.clone(), channel.clone()));
            }
        }
        // A focused or selected curve plots even while its row is collapsed.
        candidates.extend(self.graph_focus.clone());
        for id in self.selection.layers() {
            if let Some(layer) = self.index.layer(id) {
                candidates.extend(
                    layer
                        .channels
                        .iter()
                        .filter(|c| !c.keys.is_empty())
                        .map(|c| (id.clone(), c.path.clone())),
                );
            }
        }
        let focus = self
            .graph_focus
            .as_ref()
            .map(|(l, c)| (l.as_str(), c.as_str()));
        graph_target(
            &candidates,
            focus,
            self.selection.layers(),
            self.selection.channel(),
        )
        .cloned()
    }
    /// Composition times of the first and last keyframe on the graph's curves.
    pub fn graph_span(&self) -> Option<(f64, f64)> {
        let (series, _) = self.graph_series();
        series
            .iter()
            .filter_map(|(layer, path)| {
                Some((self.index.layer(layer)?, self.index.channel(layer, path)?))
            })
            .flat_map(|(layer, channel)| channel.keys.iter().map(|k| layer.from + k.time))
            .fold(None, |span: Option<(f64, f64)>, t| {
                Some(span.map_or((t, t), |(a, b)| (a.min(t), b.max(t))))
            })
    }
    /// The channels the graph plots, as `(layer, channel)`, in layer and
    /// channel order, and whether the key selection chose them (v1's
    /// `drawGraph`): every numeric channel holding a selected key, with the
    /// other animated axes of a position, scale, anchor or rotation one of
    /// them belongs to; with no key selected, the curves shown for the last
    /// selection while they still exist; else the target property's axes.
    pub fn graph_series(&self) -> (Vec<(String, String)>, bool) {
        let numeric = |c: &Channel| c.value.as_f64().is_some() && !c.keys.is_empty();
        let family = |path: &str| {
            ["x", "y", "z"]
                .iter()
                .find_map(|axis| path.strip_suffix(&format!(".{axis}")))
                .map(str::to_owned)
        };
        let mut picked: HashSet<(String, String)> = HashSet::new();
        let mut families: HashSet<(String, String)> = HashSet::new();
        for layer in &self.index.layers {
            for channel in layer.channels.iter().filter(|c| numeric(c)) {
                if channel
                    .keys
                    .iter()
                    .any(|k| self.selection.key_selected(&layer.id, &k.id))
                {
                    picked.insert((layer.id.clone(), channel.path.clone()));
                    if let Some(f) = family(&channel.path) {
                        families.insert((layer.id.clone(), f));
                    }
                }
            }
        }
        let in_order = |keep: &dyn Fn(&Layer, &Channel) -> bool| -> Vec<(String, String)> {
            self.index
                .layers
                .iter()
                .flat_map(|layer| {
                    layer
                        .channels
                        .iter()
                        .filter(|c| numeric(c) && keep(layer, c))
                        .map(|c| (layer.id.clone(), c.path.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        if !picked.is_empty() {
            let series = in_order(&|layer, c| {
                picked.contains(&(layer.id.clone(), c.path.clone()))
                    || family(&c.path).is_some_and(|f| families.contains(&(layer.id.clone(), f)))
            });
            return (series, true);
        }
        let shown: HashSet<_> = self.graph_shown.iter().cloned().collect();
        let series = in_order(&|layer, c| shown.contains(&(layer.id.clone(), c.path.clone())));
        if !series.is_empty() {
            return (series, false);
        }
        let Some((layer, path)) = self.graph_property() else {
            return (vec![], false);
        };
        // The property's whole track plots, every axis of it (v1: Position
        // shows X and Y, Scale both of its axes).
        let track = ["position", "scale"]
            .into_iter()
            .find(|t| {
                path.strip_prefix(t)
                    .is_some_and(|rest| rest.starts_with('.'))
            })
            .unwrap_or(path.as_str());
        let series = self
            .track_channels(&layer, track)
            .into_iter()
            .filter(|c| numeric(c))
            .map(|c| (layer.clone(), c.path.clone()))
            .collect();
        (series, false)
    }
    pub fn row_position(&self, id: &RowId) -> Option<usize> {
        self.row_positions.get(id).copied()
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Select sibling layers in the focused group, or the composition's roots.
    pub fn select_all(&mut self, focused_layer: Option<&str>) -> Call {
        let children = self.index.children();
        let group = focused_layer
            .filter(|id| self.row_position(&RowId::Layer((*id).into())).is_some())
            .and_then(|id| self.index.layer(id)?.group.clone())
            .filter(|id| {
                self.index.layer(id).is_some_and(|group| {
                    !self
                        .reveal
                        .layers
                        .get(id)
                        .map_or(group.group_collapsed, |state| state.group_collapsed)
                })
            });
        let siblings = if group.is_some() {
            children.get(&group).cloned().unwrap_or_default()
        } else if let Some(gid) = self.active_focused_group() {
            children
                .get(&Some(gid.to_owned()))
                .cloned()
                .unwrap_or_default()
        } else {
            self.index.roots(&children)
        };
        let ids = siblings
            .into_iter()
            .map(|i| &self.index.layers[i])
            .filter(|layer| !layer.effectively_locked)
            .map(|layer| layer.id.clone())
            .collect();
        self.key_selection_active = false;
        SelectionCall::Layers { ids, add: false }.user().into()
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
            .user()
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
        // Keys by (layer, key): key ids may repeat across layers.
        let hit: Vec<(String, String)> = self
            .expand_scale_keys_on(layer, hit)
            .into_iter()
            .map(|id| (layer.to_owned(), id))
            .collect();
        let before = self.selected_key_pairs();
        let all = hit.iter().any(|pair| before.contains(pair));
        let mut next = if additive || all { before } else { Vec::new() };
        if additive && !dragging && all {
            next.retain(|pair| !hit.contains(pair));
        } else {
            for pair in hit {
                if !next.contains(&pair) {
                    next.push(pair);
                }
            }
        }
        self.graph_focus = Some((layer.into(), channel.into()));
        self.key_selection_active = true;
        let mut calls = Vec::new();
        if additive || !self.selection.contains(layer) {
            calls.push(SelectionCall::Layers {
                ids: vec![layer.into()],
                add: additive,
            });
        }
        calls.push(SelectionCall::Channel(Some(channel.into())));
        calls.push(SelectionCall::LayerKeys(next));
        Ok(vec![SelectionCall::Batch(calls).user().into()])
    }

    /// The selected keys as `(layer, key)`.
    pub fn selected_key_pairs(&self) -> Vec<(String, String)> {
        self.index
            .selected_keys(&self.selection)
            .into_iter()
            .map(|(l, _, k)| (l.id.clone(), k.id.clone()))
            .collect()
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
            SelectionCall::Batch(vec![
                SelectionCall::Layers {
                    ids: layers,
                    add: additive,
                },
                SelectionCall::Keys(selected_keys),
            ])
            .user()
            .into(),
        ]
    }

    pub fn property_visible(&self, layer: &str, channel: &Channel) -> bool {
        self.reveal.visible(layer, channel)
            || self
                .editing_property
                .as_ref()
                .is_some_and(|(l, p)| l == layer && p == &channel.path)
    }
    fn group_has_children(&self, id: &str) -> bool {
        self.index
            .layer(id)
            .is_some_and(|layer| layer.kind == "group")
            && self.index.group_has_children(id)
    }

    pub fn layer_disclosable(&self, id: &str) -> bool {
        self.group_has_children(id)
            || self.index.layer(id).is_some_and(|layer| {
                let explicitly_open = self
                    .reveal
                    .layers
                    .get(id)
                    .is_some_and(|d| !d.collapsed && d.paths.is_some());
                layer.channels.iter().any(|channel| {
                    layer.channel_available(&channel.path)
                        && (!channel.keys.is_empty()
                            || (explicitly_open
                                && self.property_visible(id, channel)
                                && !self.reveal.retained_empty.contains(&(id.into(), channel.path.clone()))))
                })
            })
    }

    pub fn layer_expanded(&self, id: &str) -> bool {
        let Some(layer) = self.index.layer(id) else {
            return false;
        };
        let disclosure = self.reveal.layers.get(id);
        if self.group_has_children(id) {
            !disclosure.map_or(layer.group_collapsed, |d| d.group_collapsed)
        } else {
            !disclosure.map_or(layer.collapsed, |d| d.collapsed)
        }
    }

    pub(crate) fn set_disclosure(&mut self, id: &str, open: bool) -> Result<(), ModelError> {
        let layer = self
            .index
            .layer(id)
            .ok_or_else(|| ModelError::UnknownId(id.into()))?;
        let group = self.group_has_children(id);
        let d = self
            .reveal
            .layers
            .entry(id.into())
            .or_insert_with(|| Disclosure {
                collapsed: layer.collapsed,
                group_collapsed: layer.group_collapsed,
                paths: None,
            });
        d.collapsed = !open;
        if group {
            d.group_collapsed = !open;
        }
        d.paths = None;
        Ok(())
    }

    pub fn toggle_disclosure(&mut self, id: &str) -> Result<(), ModelError> {
        self.set_disclosure(id, !self.layer_expanded(id))?;
        self.rebuild_rows();
        Ok(())
    }

    /// A Tracks header's disclosure: absent means expanded, as every track
    /// showed its rows before headers could close.
    pub fn track_collapsed(&self, area: tracks::Area, lane: usize) -> bool {
        self.reveal
            .tracks
            .get(self.index.comp_id.as_deref().unwrap_or_default())
            .and_then(|tracks| tracks.get(&(area, lane)))
            .copied()
            .unwrap_or(false)
    }
    /// A Tracks header's chevron press: `open` is the state asked for, the
    /// same idempotent write the layer rows' disclosure makes.
    pub fn disclose_track(&mut self, area: tracks::Area, lane: usize, open: bool) {
        self.reveal
            .tracks
            .entry(self.index.comp_id.clone().unwrap_or_default())
            .or_default()
            .insert((area, lane), !open);
    }
    /// Option-click on a track header opens or closes every track at once,
    /// as the layer rows' `all` disclosure does for every layer.
    pub fn disclose_all_tracks(&mut self, open: bool) {
        let layout = self.track_layout();
        let tracks = self
            .reveal
            .tracks
            .entry(self.index.comp_id.clone().unwrap_or_default())
            .or_default();
        for (area, lane) in layout.display_order() {
            tracks.insert((area, lane), !open);
        }
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
    fn channel_label(&self, layer: &str, path: &str) -> String {
        self.index
            .channel(layer, path)
            .and_then(|channel| channel.vertex_label.clone())
            .unwrap_or_else(|| property_label(path))
    }
}

/// A property row's name, shared by the outline and the graph: the transform
/// names Properties uses, else the stored id in words (`blendMode` → `Blend Mode`).
pub fn property_label(path: &str) -> String {
    match path {
        "position" => "Position",
        "position.x" => "Position X",
        "position.y" => "Position Y",
        "position.z" => "Position Z",
        "scale" => "Scale",
        "scale.x" => "Scale X",
        "scale.y" => "Scale Y",
        "scale.z" => "Scale Z",
        "rotation" => "Rotation",
        "rotation.x" => "X Rotation",
        "rotation.y" => "Y Rotation",
        "orientation" => "Orientation",
        "orientation.x" => "Orientation X",
        "orientation.y" => "Orientation Y",
        "orientation.z" => "Orientation Z",
        "opacity" => "Opacity",
        "anchor" => "Anchor Point",
        "anchor.x" => "Anchor Point X",
        "anchor.y" => "Anchor Point Y",
        "anchor.z" => "Anchor Point Z",
        "skew" => "Skew",
        "camera.targetX" | "light.targetX" => "Point of Interest X",
        "camera.targetY" | "light.targetY" => "Point of Interest Y",
        "camera.targetZ" | "light.targetZ" => "Point of Interest Z",
        "camera.fov" => "Field of View",
        "camera.zoom" => "Zoom",
        "camera.focusDistance" => "Focus Distance",
        "camera.fStop" => "F-Stop",
        "camera.blurLevel" => "Blur Level",
        "light.intensity" => "Intensity",
        "light.color" => "Light Color",
        "environment.ambient" => "Ambient",
        _ => return pm_surface::humanize(path),
    }
    .to_owned()
}
