//! v1.2.0 property-reveal.ts:7–107. Clock and schema facts are injected.
use crate::{Channel, ModelError, TimelineModel};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevealTarget {
    Layer(String),
    Property { layer: String, path: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disclosure {
    pub collapsed: bool,
    pub group_collapsed: bool,
    /// None = animated channels; Some(["*"]) = all.
    pub paths: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default)]
pub struct PropertyFacts {
    /// Schema-derived facts absent from the canonical document.
    pub modified: HashSet<(String, String)>,
    pub material: HashSet<(String, String)>,
    pub paint: HashSet<(String, String)>,
    pub missing_effect: HashSet<(String, String)>,
    pub effect_channels: HashSet<(String, String)>,
}
#[derive(Clone, Debug)]
struct Last {
    key: char,
    shift: bool,
    millis: u64,
    ids: Vec<String>,
    before: Vec<Disclosure>,
}
#[derive(Clone, Debug, Default)]
pub struct RevealState {
    pub layers: HashMap<String, Disclosure>,
    last: Option<Last>,
    /// Rows kept after deleting their final key, until a collapse or refresh.
    pub(crate) retained_empty: HashSet<(String, String)>,
    pub(crate) entry: Option<u64>,
    pub(crate) revealed_in_entry: HashSet<(String, String)>,
    /// Tracks mode: the lane each clip showed on (relative to its group, as
    /// a stored `track` is) and the time it spanned then. A clip whose time
    /// has not changed stays on it through any edit, from any source; only
    /// a clip that never had a lane, or an edited one whose lane is taken,
    /// is packed (PM-477, PM-485).
    pub held: HashMap<String, Held>,
    /// Tracks mode: collapsed state per composition and track (area, lane).
    /// The empty composition key identifies the project's root-level tracks.
    /// An unrecorded track is open, matching the original track view.
    pub(crate) tracks: HashMap<String, HashMap<(crate::tracks::Area, usize), bool>>,
}
/// A clip's lane as the Tracks timeline last showed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub lane: usize,
    pub from: f64,
    pub end: f64,
}
impl RevealState {
    pub fn visible(&self, layer: &str, channel: &Channel) -> bool {
        self.layers
            .get(layer)
            .and_then(|d| d.paths.as_ref())
            .map_or(!channel.keys.is_empty(), |paths| {
                paths.iter().any(|p| p == "*" || p == &channel.path)
            })
    }
}
impl TimelineModel {
    pub fn reveal_property(
        &mut self,
        key: char,
        shift: bool,
        millis: u64,
        facts: &PropertyFacts,
    ) -> Result<(), ModelError> {
        if !"psrtaumfel*".contains(key) {
            return Err(ModelError::UnknownCommand(key.to_string()));
        }
        let layers: Vec<_> = self
            .index
            .layers
            .iter()
            .filter(|l| self.selection.layers().is_empty() || self.selection.contains(&l.id))
            .collect();
        if layers.is_empty() {
            return Ok(());
        }
        let ids: Vec<_> = layers.iter().map(|l| l.id.clone()).collect();
        let before: Vec<_> = layers
            .iter()
            .map(|l| {
                self.reveal
                    .layers
                    .get(&l.id)
                    .cloned()
                    .unwrap_or(Disclosure {
                        collapsed: l.collapsed,
                        group_collapsed: l.group_collapsed,
                        paths: None,
                    })
            })
            .collect();
        if key == '*' || (key == 'm' && self.selection.layers().is_empty()) {
            let close = before.iter().all(|d| !d.collapsed);
            for (l, mut d) in layers.iter().zip(before) {
                d.paths = Some(vec!["*".into()]);
                d.collapsed = close;
                if key == 'm' && l.kind == "group" {
                    d.group_collapsed = close;
                }
                self.reveal.layers.insert(l.id.clone(), d);
            }
            self.reveal.last = None;
        } else {
            let double = self.reveal.last.as_ref().is_some_and(|last| {
                last.key == key
                    && last.shift == shift
                    && millis >= last.millis
                    && millis - last.millis < 300
                    && last.ids == ids
            });
            let base = if double {
                self.reveal.last.as_ref().unwrap().before.clone()
            } else {
                before.clone()
            };
            let targets: Vec<Vec<String>> = layers
                .iter()
                .map(|l| {
                    l.channels
                        .iter()
                        .filter(|c| {
                            if !l.channel_available(&c.path) {
                                return false;
                            }
                            let p = c.path.as_str();
                            let pair = (l.id.clone(), c.path.clone());
                            match (key, double) {
                                ('p', false) => p.starts_with("position."),
                                ('p', true) => facts.paint.contains(&pair),
                                ('s', false) => p.starts_with("scale."),
                                ('s', true) => {
                                    self.selection.channel() == Some(p)
                                        || (self.selection.channel() == Some("scale")
                                            && p.starts_with("scale."))
                                        || c.keys
                                            .iter()
                                            .any(|k| self.selection.key_selected(&l.id, &k.id))
                                }
                                ('r', false) => {
                                    p == "rotation"
                                        || p.starts_with("rotation.")
                                        || p == "orientation"
                                        || p.starts_with("orientation.")
                                }
                                ('r', true) => p == "c.timeRemap" || p == "c.sourceTime",
                                ('a', false) => p.starts_with("anchor."),
                                ('a', true) => facts.material.contains(&pair),
                                ('t', false) => p == "opacity",
                                ('t', true) => p.starts_with("m.") && p.ends_with(".opacity"),
                                ('u', _) => {
                                    !c.keys.is_empty()
                                        || c.expression.is_some()
                                        || (double && facts.modified.contains(&pair))
                                }
                                ('m', true) => p.starts_with("m."),
                                ('m', false) => {
                                    p.starts_with("m.")
                                        && [".shape", ".x", ".y", ".w", ".h", ".rotation"]
                                            .iter()
                                            .any(|s| p.ends_with(s))
                                }
                                ('f', true) => facts.missing_effect.contains(&pair),
                                ('f', false) => p.starts_with("m.") && p.ends_with(".feather"),
                                ('e', true) => c.expression.is_some(),
                                ('e', false) => facts.effect_channels.contains(&pair),
                                ('l', false) => p == "c.gain",
                                _ => false,
                            }
                        })
                        .map(|c| c.path.clone())
                        .collect()
                })
                .collect();
            let close = !shift
                && targets.iter().zip(&base).all(|(t, d)| {
                    !d.collapsed && same_paths(t, d.paths.as_deref().unwrap_or_default())
                });
            for ((l, mut d), target) in layers.iter().zip(base).zip(targets) {
                let paths = if shift {
                    let current: Vec<_> = if d.collapsed {
                        Vec::new()
                    } else if d.paths.as_ref().is_some_and(|p| p.iter().any(|p| p == "*")) {
                        l.channels.iter().map(|c| c.path.clone()).collect()
                    } else {
                        d.paths.clone().unwrap_or_else(|| {
                            l.channels
                                .iter()
                                .filter(|c| !c.keys.is_empty())
                                .map(|c| c.path.clone())
                                .collect()
                        })
                    };
                    if !target.is_empty() && target.iter().all(|p| current.contains(p)) {
                        current
                            .into_iter()
                            .filter(|p| !target.contains(p))
                            .collect()
                    } else {
                        let mut next = current;
                        for p in target {
                            if !next.contains(&p) {
                                next.push(p);
                            }
                        }
                        next
                    }
                } else {
                    target
                };
                d.collapsed = close || (shift && paths.is_empty());
                d.paths = Some(paths);
                self.reveal.layers.insert(l.id.clone(), d);
                let mut parent = l.group.clone();
                let mut seen = HashSet::new();
                while let Some(id) = parent {
                    if !seen.insert(id.clone()) {
                        break;
                    }
                    self.reveal
                        .layers
                        .entry(id.clone())
                        .or_default()
                        .group_collapsed = false;
                    parent = self.index.layer(&id).and_then(|l| l.group.clone());
                }
            }
            self.reveal.last = if double {
                None
            } else {
                Some(Last {
                    key,
                    shift,
                    millis,
                    ids,
                    before,
                })
            };
        }
        self.rebuild_rows();
        Ok(())
    }
    /// v1–3's `TimelineService.reveal` (timeline.ts:4452): open the layer and
    /// add `keys` to what it already shows (its animated channels by default).
    pub fn reveal_paths(&mut self, layer: &str, keys: &[String]) -> Result<(), ModelError> {
        self.include_paths(layer, keys)?;
        self.rebuild_rows();
        Ok(())
    }
    pub(crate) fn include_paths(&mut self, layer: &str, keys: &[String]) -> Result<(), ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        let mut paths = self
            .reveal
            .layers
            .get(layer)
            .and_then(|d| d.paths.clone())
            .unwrap_or_else(|| {
                l.channels
                    .iter()
                    .filter(|c| !c.keys.is_empty())
                    .map(|c| c.path.clone())
                    .collect()
            });
        if !paths.iter().any(|p| p == "*") {
            for key in keys {
                if !paths.contains(key) {
                    paths.push(key.clone());
                }
            }
        }
        self.open_property_rows(layer, Some(paths))
    }

    pub(crate) fn open_property_rows(
        &mut self,
        layer: &str,
        paths: Option<Vec<String>>,
    ) -> Result<(), ModelError> {
        let l = self
            .index
            .layer(layer)
            .ok_or_else(|| ModelError::UnknownId(layer.into()))?;
        let d = self
            .reveal
            .layers
            .entry(layer.into())
            .or_insert_with(|| Disclosure {
                collapsed: l.collapsed,
                group_collapsed: l.group_collapsed,
                paths: None,
            });
        d.collapsed = false;
        d.paths = paths;
        let mut parent = l.group.clone();
        let mut seen = HashSet::new();
        while let Some(id) = parent {
            if !seen.insert(id.clone()) {
                break;
            }
            let base = self.index.layer(&id);
            let d = self
                .reveal
                .layers
                .entry(id.clone())
                .or_insert_with(|| Disclosure {
                    collapsed: base.map_or(true, |l| l.collapsed),
                    group_collapsed: base.map_or(true, |l| l.group_collapsed),
                    paths: None,
                });
            d.group_collapsed = false;
            parent = base.and_then(|l| l.group.clone());
        }
        Ok(())
    }
    /// Properties' Show in Graph Editor (v1's `showGraphEditor`): the layer
    /// opens and shows `keys` too, the graph view comes up, and it plots the
    /// track `channel` belongs to (Position and Scale are one track).
    pub fn show_graph(
        &mut self,
        layer: &str,
        channel: &str,
        keys: &[String],
    ) -> Result<crate::Plan, ModelError> {
        self.reveal_paths(layer, keys)?;
        let track = self
            .rows()
            .iter()
            .find_map(|row| match &row.id {
                crate::RowId::Property {
                    layer_id,
                    channel: track,
                } if layer_id == layer
                    && self
                        .track_channels(layer, track)
                        .iter()
                        .any(|c| c.path == channel) =>
                {
                    Some(track.clone())
                }
                _ => None,
            })
            .ok_or_else(|| ModelError::UnknownId(channel.into()))?;
        if self.graph_display == crate::GraphDisplay::DopeSheet {
            self.graph_display = crate::GraphDisplay::Graph;
        }
        self.graph_focus = Some((layer.into(), track));
        Ok(crate::Plan {
            calls: vec![
                pm_kernel::SelectionCall::Channel(Some(channel.into()))
                    .user()
                    .into(),
            ],
            ..Default::default()
        })
    }
}
fn same_paths(a: &[String], b: &[String]) -> bool {
    let a: HashSet<_> = a.iter().collect();
    let b: HashSet<_> = b.iter().collect();
    a == b
}


#[cfg(test)]
mod layer_reveal_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn layer_reveal_opens_only_its_ancestor_groups() {
        let root = pm_model::Node::parse_bytes(br#"{
            "id":"project", "dur":10, "layers":[
                {"id":"outer","type":"group"},
                {"id":"inner","type":"group","group":"outer"},
                {"id":"target","type":"shape","group":"inner"},
                {"id":"other","type":"group"},
                {"id":"hidden","type":"shape","group":"other"}
            ]
        }"#, 0).unwrap();
        let snapshot = pm_kernel::Snapshot {
            doc_id: 1,
            revision: 0,
            document: Arc::new(pm_model::Document::from_root(Some(root), 0)),
        };
        let mut model = TimelineModel::default();
        model.index = Arc::new(crate::TimelineIndex::from_snapshot(&snapshot));
        model.rebuild_rows();
        let target = crate::RowId::Layer("target".into());
        let hidden = crate::RowId::Layer("hidden".into());
        assert!(!model.rows().iter().any(|row| row.id == target));
        model.reveal_layer("target".into());
        assert!(model.rows().iter().any(|row| row.id == target && row.depth == 2));
        assert!(!model.rows().iter().any(|row| row.id == hidden));
        assert!(!model.reveal.layers["outer"].group_collapsed);
        assert!(!model.reveal.layers["inner"].group_collapsed);
        assert_eq!(model.reveal_serial, 1);
        assert_eq!(model.reveal_target, Some(RevealTarget::Layer("target".into())));
    }
}
