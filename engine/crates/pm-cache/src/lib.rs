//! Lease-bounded memory caches. Values are owned by the cache; consumers may
//! borrow them, but cannot keep an unaccounted clone through this API.

use std::collections::HashMap;
use std::hash::Hash;

struct Entry<V> {
    value: V,
    bytes: usize,
    used: u64,
}

/// A byte- and entry-bounded least-recently-used cache. A zero budget releases
/// every entry immediately. Oversized values are refused, never overcommitted.
pub struct MemoryCache<K, V> {
    entries: HashMap<K, Entry<V>>,
    budget: usize,
    max_entries: usize,
    bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub entries: usize,
    pub bytes: usize,
    pub budget: usize,
    pub hits: u64,
    pub misses: u64,
}

impl<K: Hash + Eq + Clone, V> MemoryCache<K, V> {
    pub fn new(budget: usize, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            max_entries,
            bytes: 0,
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            // Preserve ordering even after the logical clock wraps.
            let mut order: Vec<_> = self
                .entries
                .iter()
                .map(|(k, v)| (k.clone(), v.used))
                .collect();
            order.sort_by_key(|(_, used)| *used);
            for (i, (key, _)) in order.into_iter().enumerate() {
                self.entries.get_mut(&key).expect("present").used = i as u64;
            }
            self.clock = self.entries.len() as u64;
        }
        self.clock += 1;
        self.clock
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let used = self.tick();
        match self.entries.get_mut(key) {
            Some(e) => {
                e.used = used;
                self.hits += 1;
                Some(&e.value)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Replacing a key releases its old value even if the new one cannot fit.
    pub fn insert(&mut self, key: K, value: V, bytes: usize) -> bool {
        self.remove(&key);
        if bytes > self.budget || self.budget == 0 || self.max_entries == 0 {
            return false;
        }
        while self.bytes > self.budget - bytes || self.entries.len() >= self.max_entries {
            self.evict_one();
        }
        let used = self.tick();
        self.entries.insert(key, Entry { value, bytes, used });
        self.bytes += bytes;
        true
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let e = self.entries.remove(key)?;
        self.bytes -= e.bytes;
        Some(e.value)
    }

    fn evict_one(&mut self) {
        if let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone())
        {
            self.remove(&key);
        }
    }

    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        if budget == 0 {
            self.clear();
        }
        while self.bytes > budget {
            self.evict_one();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn stats(&self) -> Stats {
        Stats {
            entries: self.entries.len(),
            bytes: self.bytes,
            budget: self.budget,
            hits: self.hits,
            misses: self.misses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_shrink_evicts_lru_and_zero_releases_even_zero_byte_entries() {
        let mut c = MemoryCache::new(10, 4);
        assert!(c.insert("a", 1, 4));
        assert!(c.insert("b", 2, 4));
        assert_eq!(c.get(&"a"), Some(&1));
        assert!(c.insert("c", 3, 4));
        assert_eq!(c.get(&"b"), None);
        c.set_budget(4);
        assert_eq!(c.get(&"a"), None);
        assert_eq!(c.get(&"c"), Some(&3));
        assert!(c.insert("empty", 4, 0));
        c.set_budget(0);
        assert_eq!(c.stats().entries, 0);
        assert!(!c.insert("empty", 5, 0));
    }

    #[test]
    fn replacement_oversize_and_entry_limit_never_exceed_budget() {
        let mut c = MemoryCache::new(usize::MAX, 2);
        assert!(c.insert(1, 1, usize::MAX - 1));
        assert!(c.insert(2, 2, 1));
        assert!(c.insert(3, 3, 2));
        assert!(c.stats().bytes <= c.stats().budget);
        c.set_budget(2);
        assert!(!c.insert(3, 9, 3));
        assert_eq!(c.get(&3), None);
        assert_eq!(c.stats().bytes, 0);
        let mut zero = MemoryCache::new(0, 1);
        assert!(!zero.insert(1, 1, 0));
    }

    #[test]
    fn clock_wrap_preserves_the_oldest_entry() {
        let mut c = MemoryCache::new(3, 2);
        c.insert(1, 1, 1);
        c.insert(2, 2, 1);
        c.clock = u64::MAX;
        assert_eq!(c.get(&1), Some(&1));
        c.insert(3, 3, 1);
        assert_eq!(c.get(&2), None);
    }
}
