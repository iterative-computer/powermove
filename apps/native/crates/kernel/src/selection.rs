//! Selection: which layers, keyframes and channel the user has picked.
//!
//! Held by **id**, so it survives reordering, grouping and undo. After every
//! document change the store prunes ids that no longer exist, like the old
//! `PM.replaceProject` (`legacy/core/selection.ts`): layers must be in the
//! open composition, keys must still exist. Selection is editor state: it
//! is never saved; explicit history.selection calls can record navigation.

use std::collections::HashSet;

/// An ordered, duplicate-free selection (`PM.sel`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    layers: Vec<String>,
    /// Keyframe ids (the `i` of a stored keyframe), as `PM.sel.keys`.
    keys: Vec<String>,
    /// The layer each key was picked on, as `(layer, key)`, when it was
    /// picked on a layer (the timeline). Key ids may repeat across layers;
    /// a selected id with no entry here is selected on every layer holding it.
    key_layers: Vec<(String, String)>,
    /// The focused channel (`PM.sel.chan`), e.g. `opacity` or `position.x`.
    channel: Option<String>,
}

impl Selection {
    /// A selection an extension recorded for its history entry.
    pub fn from_parts(layers: Vec<String>, keys: Vec<String>, channel: Option<String>) -> Self {
        Self {
            layers,
            keys,
            key_layers: Vec::new(),
            channel,
        }
    }
    pub(crate) fn encoded_bytes(&self) -> usize {
        use pm_model::{Map, Value};
        let mut value = Map::new();
        value.insert(
            "layers",
            Value::Array(self.layers.iter().cloned().map(Value::from).collect()),
        );
        value.insert(
            "keys",
            Value::Array(self.keys.iter().cloned().map(Value::from).collect()),
        );
        value.insert(
            "chan",
            self.channel.clone().map(Value::from).unwrap_or(Value::Null),
        );
        pm_model::json::to_canonical_string(&Value::Object(value)).len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.keys.is_empty()
    }

    /// Selected layer ids in selection order.
    pub fn layers(&self) -> &[String] {
        &self.layers
    }

    pub fn contains(&self, layer_id: &str) -> bool {
        self.layers.iter().any(|l| l == layer_id)
    }

    /// The layer the inspector shows for a multi-selection: the first one
    /// (`PM.firstSel()`).
    pub fn primary(&self) -> Option<&str> {
        self.layers.first().map(String::as_str)
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// The `(layer, key)` pairs picked on a layer (see `key_layers`).
    pub fn key_layers(&self) -> &[(String, String)] {
        &self.key_layers
    }

    /// Whether the key `key` on layer `layer` is selected.
    pub fn key_selected(&self, layer: &str, key: &str) -> bool {
        self.key_owners(key)
            .is_none_or(|owners| owners.contains(&layer))
    }

    /// The layers a selected key id is selected on: `None` for every layer
    /// holding it (picked by id alone), `Some([])` when it isn't selected.
    pub fn key_owners<'a>(&'a self, key: &str) -> Option<Vec<&'a str>> {
        if !self.keys.iter().any(|k| k == key) {
            return Some(Vec::new());
        }
        let owners: Vec<&str> = self
            .key_layers
            .iter()
            .filter(|(_, k)| k == key)
            .map(|(l, _)| l.as_str())
            .collect();
        (!owners.is_empty()).then_some(owners)
    }

    pub fn channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }

    /// `selectLayers(ids, add)`: replaces the layers (and clears keys), or
    /// adds to them keeping keys.
    pub(crate) fn select_layers<I: IntoIterator<Item = String>>(&mut self, ids: I, add: bool) {
        let incoming = ids.into_iter().filter(|id| !id.is_empty());
        if add {
            let mut seen: HashSet<String> = self.layers.iter().cloned().collect();
            for id in incoming {
                if seen.insert(id.clone()) {
                    self.layers.push(id);
                }
            }
        } else {
            let mut seen = HashSet::new();
            self.layers = incoming.filter(|id| seen.insert(id.clone())).collect();
            self.keys.clear();
            self.key_layers.clear();
        }
    }

    /// Shift/Cmd-click: adds the layer, or removes it if selected.
    pub(crate) fn toggle_layer(&mut self, id: &str) {
        if let Some(pos) = self.layers.iter().position(|l| l == id) {
            self.layers.remove(pos);
        } else if !id.is_empty() {
            self.layers.push(id.to_owned());
        }
    }

    pub(crate) fn set_keys<I: IntoIterator<Item = String>>(&mut self, ids: I) {
        let mut seen = HashSet::new();
        self.keys = ids
            .into_iter()
            .filter(|id| !id.is_empty() && seen.insert(id.clone()))
            .collect();
        self.key_layers.clear();
    }

    /// Selects keys as picked on their layers; `keys()` lists their ids.
    pub(crate) fn set_layer_keys<I: IntoIterator<Item = (String, String)>>(&mut self, pairs: I) {
        let mut seen = HashSet::new();
        self.key_layers = pairs
            .into_iter()
            .filter(|(l, k)| !l.is_empty() && !k.is_empty() && seen.insert((l.clone(), k.clone())))
            .collect();
        let mut ids = HashSet::new();
        self.keys = self
            .key_layers
            .iter()
            .filter(|(_, k)| ids.insert(k.clone()))
            .map(|(_, k)| k.clone())
            .collect();
    }

    pub(crate) fn set_channel(&mut self, channel: Option<String>) {
        self.channel = channel;
    }

    pub(crate) fn clear(&mut self) {
        self.layers.clear();
        self.keys.clear();
        self.key_layers.clear();
    }

    /// Drops layer ids not in `layers` and key ids not in `keys` (when
    /// given). Returns whether anything went.
    pub(crate) fn prune(
        &mut self,
        layers: &HashSet<String>,
        keys: Option<&HashSet<String>>,
    ) -> bool {
        let before = (self.layers.len(), self.keys.len(), self.key_layers.len());
        self.layers.retain(|id| layers.contains(id));
        if let Some(keys) = keys {
            self.keys.retain(|id| keys.contains(id));
        }
        // A key picked on a layer that went is not selected on the others.
        let picked: HashSet<String> = self.key_layers.iter().map(|(_, k)| k.clone()).collect();
        self.key_layers
            .retain(|(l, k)| layers.contains(l) && self.keys.contains(k));
        let kept: HashSet<&String> = self.key_layers.iter().map(|(_, k)| k).collect();
        self.keys
            .retain(|k| !picked.contains(k) || kept.contains(k));
        before != (self.layers.len(), self.keys.len(), self.key_layers.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_add_toggle_and_prune() {
        let mut s = Selection::default();
        s.select_layers(["a".into(), "b".into(), "a".into()], false);
        assert_eq!(s.layers(), ["a", "b"]);
        s.set_keys(["k1".into(), "k2".into()]);
        s.select_layers(["c".into(), "a".into()], true);
        assert_eq!(s.layers(), ["a", "b", "c"]);
        assert_eq!(s.keys().len(), 2, "adding keeps keys");
        s.select_layers(["b".into()], false);
        assert!(s.keys().is_empty(), "replacing clears keys");
        s.toggle_layer("c");
        s.toggle_layer("b");
        assert_eq!(s.layers(), ["c"]);
        assert_eq!(s.primary(), Some("c"));
        s.set_keys(["k1".into()]);
        let live: HashSet<String> = ["x".to_owned()].into();
        assert!(s.prune(&live, Some(&HashSet::new())));
        assert!(s.is_empty());
    }

    /// Key ids may repeat across layers: a key picked on one layer is
    /// selected there only, and goes with its layer.
    #[test]
    fn keys_picked_on_a_layer_stay_on_it() {
        let mut s = Selection::default();
        s.set_layer_keys([("a".into(), "k".into()), ("a".into(), "k".into())]);
        assert_eq!(s.keys(), ["k"]);
        assert_eq!(s.key_owners("k"), Some(vec!["a"]));
        assert_eq!(s.key_owners("j"), Some(vec![]));
        s.set_keys(["k".into()]);
        assert_eq!(s.key_owners("k"), None, "by id: every layer holding it");
        s.set_layer_keys([("a".into(), "k".into()), ("b".into(), "j".into())]);
        let live: HashSet<String> = ["b".to_owned()].into();
        let keys: HashSet<String> = ["k".to_owned(), "j".to_owned()].into();
        assert!(s.prune(&live, Some(&keys)));
        assert_eq!(s.keys(), ["j"], "k went with layer a");
        assert_eq!(s.key_layers(), [("b".to_owned(), "j".to_owned())]);
    }
}
