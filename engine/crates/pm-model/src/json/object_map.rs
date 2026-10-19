//! The member map of a JSON object: unique keys kept sorted in one vector.
//!
//! Documents are millions of small objects (a keyframe has ten members, its
//! ease values two each). A `BTreeMap` gives each of them a full B-tree leaf,
//! eleven key and value slots plus links, so a two-member object costs as
//! much as an eleven-member one; a project with 1.2 million keyframes (C1)
//! was about 1.8 GB as a tree. A sorted vector holds exactly its members.
//! Lookups are binary searches (as fast as a B-tree at these sizes); an
//! insert or remove moves the members after it, which only matters for
//! objects of many thousands of members, and those are rare and edited by
//! key one at a time.
//!
//! Iteration order is the key order (`JsString`'s, WTF-8 bytes), the same as
//! the `BTreeMap` this replaces; collecting duplicate keys keeps the last, as
//! `BTreeMap` and `JSON.parse` do.

use core::borrow::Borrow;
use core::fmt;

use super::JsString;

/// Unique `JsString` keys, sorted, each with a value.
#[derive(Clone, PartialEq)]
pub struct ObjectMap<V> {
    entries: Vec<(JsString, V)>,
}

impl<V> Default for ObjectMap<V> {
    fn default() -> Self {
        ObjectMap {
            entries: Vec::new(),
        }
    }
}

impl<V: fmt::Debug> fmt::Debug for ObjectMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

type Entry<V> = (JsString, V);

/// Members by reference, in key order.
pub type Iter<'a, V> =
    core::iter::Map<core::slice::Iter<'a, Entry<V>>, fn(&'a Entry<V>) -> (&'a JsString, &'a V)>;
/// Members with mutable values, in key order.
pub type IterMut<'a, V> = core::iter::Map<
    core::slice::IterMut<'a, Entry<V>>,
    fn(&'a mut Entry<V>) -> (&'a JsString, &'a mut V),
>;
/// Keys in order.
pub type Keys<'a, V> =
    core::iter::Map<core::slice::Iter<'a, Entry<V>>, fn(&'a Entry<V>) -> &'a JsString>;
/// Values in key order.
pub type Values<'a, V> =
    core::iter::Map<core::slice::Iter<'a, Entry<V>>, fn(&'a Entry<V>) -> &'a V>;
/// Mutable values in key order.
pub type ValuesMut<'a, V> =
    core::iter::Map<core::slice::IterMut<'a, Entry<V>>, fn(&'a mut Entry<V>) -> &'a mut V>;

fn pair<V>(entry: &Entry<V>) -> (&JsString, &V) {
    (&entry.0, &entry.1)
}
fn pair_mut<V>(entry: &mut Entry<V>) -> (&JsString, &mut V) {
    (&entry.0, &mut entry.1)
}
fn key<V>(entry: &Entry<V>) -> &JsString {
    &entry.0
}
fn value<V>(entry: &Entry<V>) -> &V {
    &entry.1
}
fn value_mut<V>(entry: &mut Entry<V>) -> &mut V {
    &mut entry.1
}

impl<V> ObjectMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        ObjectMap {
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn find<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.entries.binary_search_by(|(k, _)| k.borrow().cmp(key))
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| &self.entries[i].1)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| &mut self.entries[i].1)
    }

    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&JsString, &V)>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| pair(&self.entries[i]))
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).is_ok()
    }

    /// Sets `key`; the value it replaced, if any.
    pub fn insert(&mut self, key: JsString, value: V) -> Option<V> {
        match self.find(&key) {
            Ok(i) => Some(core::mem::replace(&mut self.entries[i].1, value)),
            Err(i) => {
                self.entries.insert(i, (key, value));
                None
            }
        }
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| self.entries.remove(i).1)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&JsString, &mut V) -> bool) {
        self.entries.retain_mut(|(k, v)| keep(k, v));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn iter(&self) -> Iter<'_, V> {
        self.entries
            .iter()
            .map(pair as fn(&Entry<V>) -> (&JsString, &V))
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        self.entries
            .iter_mut()
            .map(pair_mut as fn(&mut Entry<V>) -> (&JsString, &mut V))
    }

    pub fn keys(&self) -> Keys<'_, V> {
        self.entries.iter().map(key as fn(&Entry<V>) -> &JsString)
    }

    pub fn values(&self) -> Values<'_, V> {
        self.entries.iter().map(value as fn(&Entry<V>) -> &V)
    }

    pub fn values_mut(&mut self) -> ValuesMut<'_, V> {
        self.entries
            .iter_mut()
            .map(value_mut as fn(&mut Entry<V>) -> &mut V)
    }

    pub fn first_key_value(&self) -> Option<(&JsString, &V)> {
        self.entries.first().map(pair)
    }

    pub fn last_key_value(&self) -> Option<(&JsString, &V)> {
        self.entries.last().map(pair)
    }
}

impl<V> FromIterator<(JsString, V)> for ObjectMap<V> {
    fn from_iter<I: IntoIterator<Item = (JsString, V)>>(iter: I) -> Self {
        let mut entries: Vec<Entry<V>> = iter.into_iter().collect();
        if entries.windows(2).all(|w| w[0].0 < w[1].0) {
            entries.shrink_to_fit();
            return ObjectMap { entries };
        }
        // Stable, so equal keys stay in arrival order; the last one wins.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out: Vec<Entry<V>> = Vec::with_capacity(entries.len());
        for entry in entries {
            match out.last_mut() {
                Some(last) if last.0 == entry.0 => *last = entry,
                _ => out.push(entry),
            }
        }
        out.shrink_to_fit();
        ObjectMap { entries: out }
    }
}

impl<V> Extend<(JsString, V)> for ObjectMap<V> {
    fn extend<I: IntoIterator<Item = (JsString, V)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<V> IntoIterator for ObjectMap<V> {
    type Item = (JsString, V);
    type IntoIter = std::vec::IntoIter<(JsString, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a, V> IntoIterator for &'a ObjectMap<V> {
    type Item = (&'a JsString, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, V> IntoIterator for &'a mut ObjectMap<V> {
    type Item = (&'a JsString, &'a mut V);
    type IntoIter = IterMut<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<V, Q> core::ops::Index<&Q> for ObjectMap<V>
where
    JsString: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = V;
    fn index(&self, key: &Q) -> &V {
        self.get(key).expect("no member with that key")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> JsString {
        JsString::from(s)
    }

    #[test]
    fn behaves_like_the_btree_map_it_replaces() {
        use std::collections::BTreeMap;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let mut ours = ObjectMap::new();
            let mut theirs = BTreeMap::new();
            for _ in 0..(next() % 40) {
                let key = k(&format!("k{}", next() % 25));
                match next() % 4 {
                    0 => assert_eq!(ours.remove(key.as_bytes()), theirs.remove(&key)),
                    _ => {
                        let v = next() % 100;
                        assert_eq!(ours.insert(key.clone(), v), theirs.insert(key, v));
                    }
                }
            }
            assert_eq!(ours.len(), theirs.len());
            assert!(ours.iter().eq(theirs.iter()));
            assert!(ours.keys().rev().eq(theirs.keys().rev()));
            for probe in 0..25 {
                let key = format!("k{probe}");
                assert_eq!(ours.get(key.as_bytes()), theirs.get(key.as_bytes()));
            }
            let pairs: Vec<(JsString, u64)> = (0..(next() % 30))
                .map(|_| (k(&format!("d{}", next() % 9)), next()))
                .collect();
            let a: ObjectMap<u64> = pairs.iter().cloned().collect();
            let b: BTreeMap<JsString, u64> = pairs.into_iter().collect();
            assert!(a.iter().eq(b.iter()), "collecting keeps the last duplicate");
        }
    }

    #[test]
    fn holds_exactly_its_members() {
        let small: ObjectMap<f64> = [(k("speed"), 0.0), (k("influence"), 33.3)]
            .into_iter()
            .collect();
        assert_eq!(small.entries.capacity(), 2);
        assert_eq!(
            small
                .keys()
                .map(|k| k.to_string_lossy())
                .collect::<Vec<_>>(),
            ["influence", "speed"]
        );
        assert_eq!(small[&k("speed")], 0.0);
    }
}
