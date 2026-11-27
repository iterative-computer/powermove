//! A least-recently-used map whose entries carry a byte charge. Lookups and
//! inserts mark an entry used; eviction takes the least recently used entry
//! (optionally skipping pinned keys) in `O(log n)`.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

pub(super) struct Lru<K, V> {
    map: HashMap<K, Slot<V>>,
    order: BTreeMap<u64, K>,
    tick: u64,
    bytes: usize,
}

struct Slot<V> {
    value: V,
    tick: u64,
    bytes: usize,
}

impl<K: Hash + Eq + Clone, V> Default for Lru<K, V> {
    fn default() -> Self {
        Lru {
            map: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            bytes: 0,
        }
    }
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    /// The sum of the charges of every entry.
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The entry, marked used.
    pub(super) fn get(&mut self, key: &K) -> Option<&mut V> {
        let slot = self.map.get_mut(key)?;
        self.order.remove(&slot.tick);
        self.tick += 1;
        slot.tick = self.tick;
        self.order.insert(self.tick, key.clone());
        Some(&mut slot.value)
    }

    /// The entry, not marked used.
    #[cfg(test)]
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|slot| &slot.value)
    }

    #[cfg(test)]
    pub(super) fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.map.values().map(|slot| &slot.value)
    }

    /// Inserts (or replaces) the most recently used entry.
    pub(super) fn insert(&mut self, key: K, value: V, bytes: usize) {
        self.remove(&key);
        self.tick += 1;
        self.order.insert(self.tick, key.clone());
        self.bytes += bytes;
        self.map.insert(
            key,
            Slot {
                value,
                tick: self.tick,
                bytes,
            },
        );
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        let slot = self.map.remove(key)?;
        self.order.remove(&slot.tick);
        self.bytes -= slot.bytes;
        Some(slot.value)
    }

    /// Removes the least recently used entry that `pinned` does not keep.
    pub(super) fn pop_lru(&mut self, pinned: impl Fn(&K) -> bool) -> Option<(K, V)> {
        let key = self.order.values().find(|k| !pinned(k))?.clone();
        let value = self.remove(&key)?;
        Some((key, value))
    }

    /// Removes and returns every entry `keep` rejects.
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) -> Vec<(K, V)> {
        let gone: Vec<K> = self
            .map
            .iter()
            .filter(|(k, slot)| !keep(k, &slot.value))
            .map(|(k, _)| k.clone())
            .collect();
        gone.into_iter()
            .filter_map(|k| self.remove(&k).map(|v| (k, v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Lru;

    #[test]
    fn evicts_the_least_recently_used_and_skips_pinned() {
        let mut lru = Lru::default();
        lru.insert("a", 1, 10);
        lru.insert("b", 2, 20);
        lru.insert("c", 3, 30);
        assert_eq!(lru.bytes(), 60);
        // Touching `a` makes `b` the least recently used.
        assert_eq!(lru.get(&"a"), Some(&mut 1));
        assert_eq!(lru.pop_lru(|_| false), Some(("b", 2)));
        // `c` is older than `a`, but pinned.
        assert_eq!(lru.pop_lru(|k| *k == "c"), Some(("a", 1)));
        assert_eq!(lru.pop_lru(|k| *k == "c"), None);
        assert_eq!((lru.len(), lru.bytes()), (1, 30));
        lru.insert("c", 4, 5);
        assert_eq!((lru.len(), lru.bytes(), lru.peek(&"c")), (1, 5, Some(&4)));
        assert_eq!(lru.retain(|_, v| *v != 4), vec![("c", 4)]);
        assert_eq!(lru.bytes(), 0);
    }
}
