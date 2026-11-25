//! Scroll-independent marquee selection, timeline.ts:4110–4190.
use crate::{Plan, TimelineModel, unique_ids};
use pm_kernel::{Selection, SelectionCall};
use std::collections::HashSet;
/// Keys are `(layer, key)`: key ids may repeat across layers.
#[derive(Clone, Debug)]
pub struct MarqueeSession {
    before: Selection,
    document: u64,
    keys: Vec<(String, String)>,
    layers: Vec<String>,
    additive: bool,
    graph: bool,
    picked: Vec<(String, String)>,
    preview: Option<crate::render::Marquee>,
}
impl MarqueeSession {
    pub fn begin(model: &TimelineModel, additive: bool, graph: bool) -> Self {
        Self {
            before: model.selection.clone(),
            document: model.index.doc_id,
            keys: model.selected_key_pairs(),
            layers: model.selection.layers().to_vec(),
            additive,
            graph,
            picked: Vec::new(),
            preview: None,
        }
    }
    /// Whether the box is drawn in the graph (it picks the graph's keys).
    pub fn in_graph(&self) -> bool {
        self.graph
    }
    /// Computes the live selection. Unchanged pointer samples do no work
    /// beyond testing the covered geometry and comparing the selected IDs.
    pub fn preview(&mut self, picked: Vec<(String, String)>, layers: Vec<String>) -> bool {
        let next = self.covered_selection(picked, layers);
        let (old_layers, old_keys) = self.current();
        if next.clips == old_layers && next.keys == old_keys {
            return false;
        }
        self.preview = Some(next);
        true
    }

    pub fn reset(&mut self) {
        self.picked.clear();
        self.preview = None;
    }

    pub fn current(&self) -> (&[String], &[(String, String)]) {
        self.preview
            .as_ref()
            .map_or((self.layers.as_slice(), self.keys.as_slice()), |preview| {
                (preview.clips.as_slice(), preview.keys.as_slice())
            })
    }

    fn covered_selection(
        &mut self,
        picked: Vec<(String, String)>,
        layers: Vec<String>,
    ) -> crate::render::Marquee {
        self.picked.clear();
        if !self.graph && !layers.is_empty() {
            let previous = self.layers.iter().filter(|_| self.additive).cloned();
            return crate::render::Marquee {
                clips: unique_ids(previous.chain(layers)),
                keys: Vec::new(),
            };
        }
        let mut seen = HashSet::new();
        self.picked = picked
            .into_iter()
            .filter(|pair| seen.insert(pair.clone()))
            .collect();
        let keep_original_keys = self.additive || self.graph;
        let mut keys: Vec<_> = self
            .keys
            .iter()
            .filter(|pair| keep_original_keys && !(self.additive && seen.contains(*pair)))
            .cloned()
            .collect();
        let before: HashSet<_> = self.keys.iter().collect();
        keys.extend(
            self.picked
                .iter()
                .filter(|pair| !keep_original_keys || !before.contains(pair))
                .cloned(),
        );
        // Keep the graph's curves visible while an empty box is being drawn.
        let selected_layers = self
            .layers
            .iter()
            .filter(|_| self.additive || (self.graph && keys.is_empty()))
            .cloned();
        let clips = unique_ids(
            selected_layers
                .chain(keys.iter().map(|(layer, _)| layer.clone()))
                .chain(layers.into_iter().filter(|_| keys.is_empty())),
        );
        crate::render::Marquee { clips, keys }
    }

    /// For consumers that publish live selection, emit only changed samples.
    pub fn update(&mut self, picked: Vec<(String, String)>, layers: Vec<String>) -> Plan {
        if !self.preview(picked, layers) {
            return Plan::default();
        }
        let (layers, keys) = self.current();
        self.selection(layers.to_vec(), SelectionCall::LayerKeys(keys.to_vec()))
    }

    pub fn finish(&self) -> Plan {
        let plan = if self.graph && !self.additive {
            self.graph_selection()
        } else {
            Plan::default()
        };
        self.record_finished(plan)
    }

    /// Publish the final selection for a surface that previewed it locally.
    pub fn finish_local(&self) -> Plan {
        if self.graph && !self.additive {
            if self.picked.is_empty() {
                return Plan::default();
            }
            return self.record_finished(self.graph_selection());
        }
        let (layers, keys) = self.current();
        if layers == self.layers && keys == self.keys {
            return Plan::default();
        }
        self.record_finished(
            self.selection(layers.to_vec(), SelectionCall::LayerKeys(keys.to_vec())),
        )
    }

    fn graph_selection(&self) -> Plan {
        if self.picked.is_empty() {
            return if self.preview.is_some() {
                self.cancel()
            } else {
                Plan::default()
            };
        }
        let keys = self.picked.clone();
        let layers = unique_ids(keys.iter().map(|(layer, _)| layer.clone()));
        self.selection(layers, SelectionCall::LayerKeys(keys))
    }

    fn record_finished(&self, mut plan: Plan) -> Plan {
        plan.expected_document = Some(self.document);
        plan.calls.push(
            SelectionCall::FinishGesture {
                document: self.document,
                before: self.before.clone(),
            }
            .into(),
        );
        plan
    }

    pub fn cancel(&self) -> Plan {
        restore_selection(self.document, &self.before)
    }

    fn selection(&self, layers: Vec<String>, keys: SelectionCall) -> Plan {
        selection(self.document, layers, keys, None)
    }
}

fn restore_selection(document: u64, before: &Selection) -> Plan {
    let keys = if before.key_layers().is_empty() {
        SelectionCall::Keys(before.keys().to_vec())
    } else {
        SelectionCall::LayerKeys(before.key_layers().to_vec())
    };
    selection(
        document,
        before.layers().to_vec(),
        keys,
        Some(before.channel()),
    )
}

fn selection(
    document: u64,
    layers: Vec<String>,
    keys: SelectionCall,
    channel: Option<Option<&str>>,
) -> Plan {
    let mut calls = vec![
        SelectionCall::Layers {
            ids: layers,
            add: false,
        },
        keys,
    ];
    if let Some(channel) = channel {
        calls.push(SelectionCall::Channel(channel.map(str::to_owned)));
    }
    Plan {
        calls: vec![SelectionCall::Preview(calls).into()],
        expected_document: Some(document),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_hit_sets_do_not_publish_selection() {
        let model = TimelineModel::default();
        let mut session = MarqueeSession::begin(&model, false, false);
        let keys = vec![("layer".to_owned(), "key".to_owned())];
        for _ in 0..120 {
            assert!(session.update(vec![], vec![]).calls.is_empty());
        }
        assert_eq!(session.update(keys.clone(), vec![]).calls.len(), 1);
        for _ in 0..120 {
            assert!(session.update(keys.clone(), vec![]).calls.is_empty());
        }
        assert_eq!(session.update(vec![], vec![]).calls.len(), 1);
        assert!(session.update(vec![], vec![]).calls.is_empty());
    }

    #[test]
    fn timeline_cancel_restores_keys_picked_by_id_and_the_selected_channel() {
        let model = TimelineModel {
            selection: Selection::from_parts(
                vec!["original".into()],
                vec!["shared-key".into()],
                Some("position.x".into()),
            ),
            ..Default::default()
        };
        let mut session = MarqueeSession::begin(&model, false, false);
        session.update(vec![], vec!["another".into()]);
        let plan = session.cancel();
        assert!(matches!(
            &plan.calls[..],
            [pm_kernel::Call::Selection(SelectionCall::Preview(calls))]
            if matches!(&calls[..], [
                SelectionCall::Layers { ids, add: false },
                SelectionCall::Keys(keys),
                SelectionCall::Channel(Some(channel)),
            ] if ids == &["original"] && keys == &["shared-key"] && channel == "position.x")
        ));
    }
}
