//! Last-painted panel pixels. All compatibility realms in a window share the
//! same budget, but a bitmap can only be read by its original owner.
use std::{collections::VecDeque, rc::Rc};

pub(crate) const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SNAPSHOTS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PanelSnapshotKey {
    pub(crate) realm: u64,
    pub(crate) panel: String,
    pub(crate) revision: u64,
    pub(crate) size: [u64; 2],
    pub(crate) theme: u64,
}

struct Entry<T> {
    key: Option<PanelSnapshotKey>,
    bytes: usize,
    bitmap: Rc<T>,
}

pub(crate) struct PanelSnapshots<T> {
    entries: VecDeque<Entry<T>>,
    bytes: usize,
    next_realm: u64,
}

impl<T> Default for PanelSnapshots<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            next_realm: 0,
        }
    }
}

impl<T> PanelSnapshots<T> {
    pub(crate) fn allocate_realm(&mut self) -> u64 {
        let realm = self.next_realm;
        self.next_realm += 1;
        realm
    }

    pub(crate) fn contains(&self, key: &PanelSnapshotKey) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.key.as_ref() == Some(key))
    }

    pub(crate) fn get(&mut self, key: &PanelSnapshotKey) -> Option<Rc<T>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.key.as_ref() == Some(key))?;
        let entry = self.entries.remove(index)?;
        let image = entry.bitmap.clone();
        self.entries.push_back(entry);
        Some(image)
    }

    pub(crate) fn insert(&mut self, key: PanelSnapshotKey, bytes: usize, bitmap: T) {
        if bytes == 0 || bytes > MAX_SNAPSHOT_BYTES {
            return;
        }
        self.retain(|current| current.realm != key.realm || current.panel != key.panel);
        while self.entries.len() >= MAX_SNAPSHOTS || self.bytes + bytes > MAX_SNAPSHOT_BYTES {
            // An image presented while WebKit starts is still part of the
            // budget. Keep it until AppKit presents the live view.
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| Rc::strong_count(&entry.bitmap) == 1)
            else {
                return;
            };
            self.bytes -= self.entries.remove(index).expect("snapshot index").bytes;
        }
        self.entries.push_back(Entry {
            key: Some(key),
            bytes,
            bitmap: Rc::new(bitmap),
        });
        self.bytes += bytes;
    }

    pub(crate) fn remove_realm(&mut self, realm: u64) {
        self.retain(|key| key.realm != realm);
    }

    pub(crate) fn remove_panel(&mut self, realm: u64, panel: &str) {
        self.retain(|key| key.realm != realm || key.panel != panel);
    }

    fn retain(&mut self, mut keep: impl FnMut(&PanelSnapshotKey) -> bool) {
        for entry in &mut self.entries {
            if entry.key.as_ref().is_some_and(|key| !keep(key)) {
                entry.key = None;
            }
        }
        self.discard_invalid();
    }

    pub(crate) fn discard_invalid(&mut self) {
        self.entries.retain(|entry| {
            if entry.key.is_some() || Rc::strong_count(&entry.bitmap) > 1 {
                true
            } else {
                self.bytes -= entry.bytes;
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(realm: u64, panel: usize) -> PanelSnapshotKey {
        PanelSnapshotKey {
            realm,
            panel: panel.to_string(),
            revision: 1,
            size: [240f64.to_bits(), 200f64.to_bits()],
            theme: 2,
        }
    }

    #[test]
    fn snapshot_requires_matching_owner_revision_size_and_theme() {
        let mut cache = PanelSnapshots::default();
        let original = key(1, 2);
        cache.insert(original.clone(), 1024, "Pexels");
        for other in [
            key(1, 3),
            key(2, 2),
            PanelSnapshotKey {
                revision: 2,
                ..original.clone()
            },
            PanelSnapshotKey {
                size: [242f64.to_bits(), 200f64.to_bits()],
                ..original.clone()
            },
            PanelSnapshotKey {
                theme: 3,
                ..original.clone()
            },
        ] {
            assert_eq!(cache.get(&other), None);
        }
        assert_eq!(cache.get(&original).as_deref(), Some(&"Pexels"));
        cache.remove_realm(1);
        assert_eq!(cache.get(&original), None);
    }

    #[test]
    fn snapshot_cache_evicts_the_least_recently_used_image_at_both_limits() {
        let mut cache = PanelSnapshots::default();
        for id in 0..12 {
            cache.insert(key(id as u64 % 3, id), 1024, id);
        }
        assert_eq!(cache.get(&key(0, 0)).as_deref(), Some(&0));
        cache.insert(key(0, 12), 1024, 12);
        assert_eq!(cache.get(&key(1, 1)), None);
        assert_eq!(cache.entries.len(), 12);
        cache.insert(key(0, 13), MAX_SNAPSHOT_BYTES - 1024, 13);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.bytes, MAX_SNAPSHOT_BYTES);
        assert_eq!(cache.get(&key(0, 0)), None);
        cache.insert(key(1, 14), MAX_SNAPSHOT_BYTES + 1, 14);
        assert_eq!(cache.get(&key(1, 14)), None);
    }

    #[test]
    fn a_displayed_bitmap_remains_within_the_budget_until_its_view_drops() {
        let mut cache = PanelSnapshots::default();
        cache.insert(key(0, 1), MAX_SNAPSHOT_BYTES, 1);
        let presentation = cache.get(&key(0, 1)).unwrap();
        cache.remove_realm(0);
        cache.insert(key(0, 2), 1024, 2);
        assert_eq!(cache.get(&key(0, 1)), None);
        assert_eq!(cache.get(&key(0, 2)), None);
        assert_eq!(cache.bytes, MAX_SNAPSHOT_BYTES);
        drop(presentation);
        cache.discard_invalid();
        assert_eq!(cache.bytes, 0);
    }
}
