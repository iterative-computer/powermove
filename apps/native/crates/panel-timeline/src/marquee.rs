//! Scroll-independent marquee selection, timeline.ts:4110–4190.
use crate::{Plan, TimelineModel, unique_ids};
use pm_kernel::SelectionCall;
/// Keys are `(layer, key)`: key ids may repeat across layers.
#[derive(Clone, Debug)]
pub struct MarqueeSession {
    keys: Vec<(String, String)>,
    layers: Vec<String>,
    additive: bool,
    graph: bool,
    picked: Vec<(String, String)>,
}
impl MarqueeSession {
    pub fn begin(model: &TimelineModel, additive: bool, graph: bool) -> Self {
        Self {
            keys: model.selected_key_pairs(),
            layers: model.selection.layers().to_vec(),
            additive,
            graph,
            picked: Vec::new(),
        }
    }
    pub fn update(&mut self, picked: Vec<(String, String)>, layers: Vec<String>) -> Plan {
        self.picked.clear();
        for pair in picked {
            if !self.picked.contains(&pair) {
                self.picked.push(pair);
            }
        }
        let mut keys = if self.additive || self.graph {
            self.keys.clone()
        } else {
            Vec::new()
        };
        for pair in &self.picked {
            if self.additive && keys.contains(pair) {
                keys.retain(|k| k != pair);
            } else if !keys.contains(pair) {
                keys.push(pair.clone());
            }
        }
        let mut selected_layers = if self.additive {
            self.layers.clone()
        } else {
            Vec::new()
        };
        for (layer, _) in &keys {
            if !selected_layers.contains(layer) {
                selected_layers.push(layer.clone());
            }
        }
        if keys.is_empty() {
            for id in layers {
                if !selected_layers.contains(&id) {
                    selected_layers.push(id);
                }
            }
        }
        selection(selected_layers, keys)
    }
    pub fn finish(&self) -> Plan {
        if !self.graph || self.additive {
            return Plan::default();
        }
        let keys = if self.picked.is_empty() {
            self.keys.clone()
        } else {
            self.picked.clone()
        };
        let layers = unique_ids(keys.iter().map(|(layer, _)| layer.clone()));
        selection(layers, keys)
    }
    pub fn cancel(&self) -> Plan {
        if self.graph {
            selection(self.layers.clone(), self.keys.clone())
        } else {
            Plan::default()
        }
    }
}
fn selection(layers: Vec<String>, keys: Vec<(String, String)>) -> Plan {
    Plan {
        calls: vec![
            SelectionCall::Layers {
                ids: layers,
                add: false,
            }
            .into(),
            SelectionCall::LayerKeys(keys).into(),
        ],
        ..Default::default()
    }
}

/// graph-selection.ts:28–42: explicit curve focus precedes layer selection.
pub fn graph_target<'a>(
    candidates: &'a [(String, String)],
    focus: Option<(&str, &str)>,
    selected: &[String],
    channel: Option<&str>,
) -> Option<&'a (String, String)> {
    if let Some((l, c)) = focus
        && let Some(hit) = candidates
            .iter()
            .find(|(layer, path)| layer == l && path == c)
    {
        return Some(hit);
    }
    candidates
        .iter()
        .find(|(l, c)| {
            selected.contains(l)
                && channel
                    .is_some_and(|path| path == c || (c == "scale" && path.starts_with("scale.")))
        })
        .or_else(|| candidates.first())
}
