//! Selection: which layers, keyframes and channel the user has picked.
//!
//! Held by **id**, so it survives reordering, grouping and undo. After every
//! document change the store prunes ids that no longer exist, like the old
//! `PM.replaceProject` (`legacy/core/selection.ts`): layers must be in the
//! open composition, keys must still exist. Selection is editor state: it
//! is never saved and never an undo step.

use std::collections::HashSet;

/// An ordered, duplicate-free selection (`PM.sel`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    layers: Vec<String>,
    /// Keyframe ids (the `i` of a stored keyframe), as `PM.sel.keys`.
    keys: Vec<String>,
    /// The focused channel (`PM.sel.chan`), e.g. `opacity` or `position.x`.
    channel: Option<String>,
}

impl Selection {
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

    /// Selected keyframe ids.
    pub fn keys(&self) -> &[String] {
        &self.keys
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
    }

    pub(crate) fn set_channel(&mut self, channel: Option<String>) {
        self.channel = channel;
    }

    pub(crate) fn clear(&mut self) {
        self.layers.clear();
        self.keys.clear();
    }

    /// Drops layer ids not in `layers` and key ids not in `keys` (when
    /// given). Returns whether anything went.
    pub(crate) fn prune(
        &mut self,
        layers: &HashSet<String>,
        keys: Option<&HashSet<String>>,
    ) -> bool {
        let before = (self.layers.len(), self.keys.len());
        self.layers.retain(|id| layers.contains(id));
        if let Some(keys) = keys {
            self.keys.retain(|id| keys.contains(id));
        }
        before != (self.layers.len(), self.keys.len())
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
}
