//! A JSON object's sorted values and shared immutable key schema.
//!
//! Millions of keyframes repeat the same member names. Keeping one schema
//! for each repeated shape avoids a `JsString` slot for every member while
//! preserving arbitrary JSON keys, WTF-8 ordering, and value-only mutation.
//! Schema interning is bounded and weak: documents own their schemas, and
//! dropping a document releases its keys even when the interner remembers it.

use core::borrow::Borrow;
use core::fmt;
use core::hash::{Hash, Hasher};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::JsString;

#[derive(Clone, Debug, PartialEq)]
struct KeySchema {
    keys: Vec<JsString>,
}

const MAX_SCHEMAS: usize = 1024;
const MAX_SCHEMA_KEYS: usize = 32;
const MAX_SCHEMA_BYTES: usize = 4096;

#[derive(Default)]
struct SchemaInterner {
    schemas: HashMap<u64, Weak<KeySchema>>,
    order: VecDeque<u64>,
}

impl SchemaInterner {
    fn intern<'a, I>(&mut self, keys: I, hash: u64) -> Arc<KeySchema>
    where
        I: Iterator<Item = &'a JsString> + Clone,
    {
        if let Some(schema) = self.schemas.get(&hash).and_then(Weak::upgrade) {
            // Hashes are only an index. A collision must never change keys.
            if schema.keys.iter().eq(keys.clone()) {
                return schema;
            }
        }
        let schema = Arc::new(KeySchema {
            keys: keys.cloned().collect(),
        });
        // Replace collisions/expired entries without adding duplicate FIFO
        // slots. A collision merely loses a sharing opportunity.
        if !self.schemas.contains_key(&hash) {
            if self.schemas.len() == MAX_SCHEMAS
                && let Some(oldest) = self.order.pop_front()
            {
                self.schemas.remove(&oldest);
            }
            self.order.push_back(hash);
        }
        self.schemas.insert(hash, Arc::downgrade(&schema));
        schema
    }
}

fn empty_schema() -> Arc<KeySchema> {
    static EMPTY: OnceLock<Arc<KeySchema>> = OnceLock::new();
    EMPTY
        .get_or_init(|| Arc::new(KeySchema { keys: Vec::new() }))
        .clone()
}

fn schema_for<'a, I>(keys: I) -> Arc<KeySchema>
where
    I: Iterator<Item = &'a JsString> + Clone + ExactSizeIterator,
{
    if keys.len() == 0 {
        return empty_schema();
    }
    let eligible = keys.len() <= MAX_SCHEMA_KEYS
        && keys
            .clone()
            .try_fold(0usize, |bytes, key| {
                let next = bytes.saturating_add(key.as_bytes().len());
                (next <= MAX_SCHEMA_BYTES).then_some(next)
            })
            .is_some();
    if !eligible {
        return Arc::new(KeySchema {
            keys: keys.cloned().collect(),
        });
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    keys.len().hash(&mut hash);
    for key in keys.clone() {
        key.hash(&mut hash);
    }
    let hash = hash.finish();
    // A thread's recent schemas first: threads parsing one document side by
    // side (json::parse_bytes_parallel) would otherwise queue on the lock
    // for every keyframe. Its entries are weak hints too.
    let slot = (hash as usize) % RECENT_SCHEMAS;
    let recent = RECENT.with(|recent| {
        let (key, schema) = &recent.borrow()[slot];
        if *key == hash { schema.upgrade() } else { None }
    });
    if let Some(schema) = recent
        && schema.keys.iter().eq(keys.clone())
    {
        return schema;
    }
    static INTERNER: OnceLock<Mutex<SchemaInterner>> = OnceLock::new();
    // No user code runs while locked. Recovering poison remains safe because
    // entries are only weak hints and the map validates all key bytes.
    let schema = INTERNER
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .intern(keys, hash);
    RECENT.with(|recent| recent.borrow_mut()[slot] = (hash, Arc::downgrade(&schema)));
    schema
}

const RECENT_SCHEMAS: usize = 64;
thread_local! {
    static RECENT: core::cell::RefCell<[(u64, Weak<KeySchema>); RECENT_SCHEMAS]> =
        core::cell::RefCell::new(core::array::from_fn(|_| (0, Weak::new())));
}

/// Unique `JsString` keys, sorted, each with a separately stored value.
#[derive(Clone, PartialEq)]
pub struct ObjectMap<V> {
    schema: Arc<KeySchema>,
    values: Vec<V>,
}

impl<V> Default for ObjectMap<V> {
    fn default() -> Self {
        Self {
            schema: empty_schema(),
            values: Vec::new(),
        }
    }
}

impl<V: fmt::Debug> fmt::Debug for ObjectMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Members by reference, in key order.
pub type Iter<'a, V> = core::iter::Zip<core::slice::Iter<'a, JsString>, core::slice::Iter<'a, V>>;
/// Members with mutable values, in key order.
pub type IterMut<'a, V> =
    core::iter::Zip<core::slice::Iter<'a, JsString>, core::slice::IterMut<'a, V>>;
/// Keys in order.
pub type Keys<'a, V> = core::iter::Map<Iter<'a, V>, fn((&'a JsString, &'a V)) -> &'a JsString>;
/// Values in key order.
pub type Values<'a, V> = core::slice::Iter<'a, V>;
/// Mutable values in key order.
pub type ValuesMut<'a, V> = core::slice::IterMut<'a, V>;

impl<V> ObjectMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            schema: empty_schema(),
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Schema identity, its allocation estimate (count once per identity),
    /// and the values' capacity. Excludes this map and its enclosing Arc.
    pub fn storage_estimate(&self) -> (usize, usize, usize) {
        let schema = &self.schema;
        let bytes = 2 * core::mem::size_of::<usize>()
            + core::mem::size_of::<KeySchema>()
            + schema.keys.capacity() * core::mem::size_of::<JsString>()
            + schema.keys.iter().map(JsString::heap_bytes).sum::<usize>();
        (Arc::as_ptr(schema) as usize, bytes, self.values.capacity())
    }

    fn find<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.schema.keys.binary_search_by(|k| k.borrow().cmp(key))
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| &self.values[i])
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key).ok().map(|i| &mut self.values[i])
    }

    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&JsString, &V)>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.find(key)
            .ok()
            .map(|i| (&self.schema.keys[i], &self.values[i]))
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
            Ok(i) => Some(core::mem::replace(&mut self.values[i], value)),
            Err(i) => {
                // Wide extension objects are not interned. Keep their normal
                // amortized Vec growth instead of cloning every key per set.
                if self.len() >= MAX_SCHEMA_KEYS {
                    self.values.reserve(1);
                    Arc::make_mut(&mut self.schema).keys.insert(i, key);
                    self.values.insert(i, value);
                    return None;
                }
                let mut keys = self.schema.keys.clone();
                keys.insert(i, key);
                let schema = schema_for(keys.iter());
                self.values.insert(i, value);
                self.schema = schema;
                None
            }
        }
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        JsString: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let i = self.find(key).ok()?;
        if self.len() > MAX_SCHEMA_KEYS + 1 {
            Arc::make_mut(&mut self.schema).keys.remove(i);
            return Some(self.values.remove(i));
        }
        let mut keys = self.schema.keys.clone();
        keys.remove(i);
        let schema = schema_for(keys.iter());
        let value = self.values.remove(i);
        self.schema = schema;
        Some(value)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&JsString, &mut V) -> bool) {
        // Decide before changing structure, so a panicking predicate leaves
        // key/value alignment intact (its value edits still take effect).
        let retained: Vec<bool> = self.iter_mut().map(|(k, v)| keep(k, v)).collect();
        if retained.iter().all(|keep| *keep) {
            return;
        }
        let keys: Vec<JsString> = self
            .schema
            .keys
            .iter()
            .zip(&retained)
            .filter(|(_, keep)| **keep)
            .map(|(key, _)| key.clone())
            .collect();
        let schema = schema_for(keys.iter());
        let mut i = 0;
        self.values.retain(|_| {
            let keep = retained[i];
            i += 1;
            keep
        });
        self.schema = schema;
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.schema = empty_schema();
    }

    pub fn iter(&self) -> Iter<'_, V> {
        self.schema.keys.iter().zip(self.values.iter())
    }
    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        self.schema.keys.iter().zip(self.values.iter_mut())
    }
    pub fn keys(&self) -> Keys<'_, V> {
        self.iter().map(|(key, _)| key)
    }
    pub fn values(&self) -> Values<'_, V> {
        self.values.iter()
    }
    pub fn values_mut(&mut self) -> ValuesMut<'_, V> {
        self.values.iter_mut()
    }
    pub fn first_key_value(&self) -> Option<(&JsString, &V)> {
        self.iter().next()
    }
    pub fn last_key_value(&self) -> Option<(&JsString, &V)> {
        self.iter().next_back()
    }
}

impl<V> FromIterator<(JsString, V)> for ObjectMap<V> {
    fn from_iter<I: IntoIterator<Item = (JsString, V)>>(iter: I) -> Self {
        let mut entries: Vec<(JsString, V)> = iter.into_iter().collect();
        if !entries.windows(2).all(|w| w[0].0 < w[1].0) {
            // Stable, so equal keys stay in arrival order; the last one wins
            // (moved into the first of its run, which is kept).
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries.dedup_by(|later, kept| {
                let same = later.0 == kept.0;
                if same {
                    core::mem::swap(later, kept);
                }
                same
            });
        }
        let schema = schema_for(entries.iter().map(|(key, _)| key));
        // A vector of its own: collected in place, the values would keep the
        // pairs' allocation and need shrinking (a reallocation).
        let mut values: Vec<V> = Vec::with_capacity(entries.len());
        values.extend(entries.into_iter().map(|(_, value)| value));
        Self { schema, values }
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
        let keys = Arc::try_unwrap(self.schema)
            .map(|schema| schema.keys)
            .unwrap_or_else(|schema| schema.keys.clone());
        keys.into_iter()
            .zip(self.values)
            .collect::<Vec<_>>()
            .into_iter()
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
        assert_eq!(small.values.capacity(), 2);
        assert_eq!(
            small
                .keys()
                .map(|k| k.to_string_lossy())
                .collect::<Vec<_>>(),
            ["influence", "speed"]
        );
        assert_eq!(small[&k("speed")], 0.0);
    }

    #[test]
    fn repeated_shapes_share_only_keys_and_mutations_preserve_snapshots() {
        let first: ObjectMap<u32> = [(k("speed"), 1), (k("influence"), 2)].into_iter().collect();
        let other: ObjectMap<bool> = [(k("influence"), false), (k("speed"), true)]
            .into_iter()
            .collect();
        assert!(Arc::ptr_eq(&first.schema, &other.schema));
        let snapshot = Arc::new(first);
        let mut edited = snapshot.clone();
        Arc::make_mut(&mut edited).insert(k("speed"), 9);
        assert!(Arc::ptr_eq(&snapshot.schema, &edited.schema));
        assert_eq!(snapshot[&k("speed")], 1);
        assert_eq!(edited[&k("speed")], 9);
        Arc::make_mut(&mut edited).insert(k("custom"), 3);
        assert!(!Arc::ptr_eq(&snapshot.schema, &edited.schema));
        assert!(!snapshot.contains_key(b"custom".as_slice()));
        Arc::make_mut(&mut edited).remove(b"custom".as_slice());
        assert!(Arc::ptr_eq(&snapshot.schema, &edited.schema));
        for (_, value) in Arc::make_mut(&mut edited).iter_mut().rev() {
            *value += 10;
        }
        assert_eq!(snapshot.values().copied().collect::<Vec<_>>(), [2, 1]);
        assert_eq!(edited.values().copied().collect::<Vec<_>>(), [12, 19]);
    }

    #[test]
    fn exact_wtf8_keys_duplicate_last_and_owned_iteration_survive_sharing() {
        let lone = JsString::from_wtf8(&[0xed, 0xa0, 0x80]).unwrap();
        let replacement = k("\u{fffd}");
        let supplementary = k("😀");
        let pairs = vec![
            (supplementary, 1),
            (lone.clone(), 2),
            (replacement.clone(), 3),
            (lone.clone(), 4),
        ];
        let map: ObjectMap<i32> = pairs.clone().into_iter().collect();
        let expected: std::collections::BTreeMap<_, _> = pairs.into_iter().collect();
        assert!(map.iter().eq(expected.iter()));
        let mut copy = map.clone();
        assert!(Arc::ptr_eq(&map.schema, &copy.schema));
        copy.retain(|key, value| {
            *value += 1;
            key != &replacement
        });
        assert_eq!(copy[&lone], 5);
        assert_eq!(map[&lone], 4);
        assert!(map.into_iter().eq(expected));
        copy.clear();
        assert!(copy.is_empty());
        assert_eq!(copy.keys().len(), 0);
    }

    #[test]
    fn interner_is_bounded_weak_and_checks_hash_collisions() {
        let mut interner = SchemaInterner::default();
        let a = [k("one")];
        let b = [k("two")];
        let first = interner.intern(a.iter(), 7);
        let same = interner.intern(a.iter(), 7);
        assert!(Arc::ptr_eq(&first, &same));
        let collision = interner.intern(b.iter(), 7);
        assert!(!Arc::ptr_eq(&first, &collision));
        assert_eq!(collision.keys, b);
        let weak = Arc::downgrade(&collision);
        drop(collision);
        assert!(
            weak.upgrade().is_none(),
            "the interner cannot retain a document's schema"
        );
        for hash in 0..(MAX_SCHEMAS * 4) {
            interner.intern(a.iter(), hash as u64);
        }
        assert_eq!(interner.schemas.len(), MAX_SCHEMAS);
        assert_eq!(interner.order.len(), MAX_SCHEMAS);
        assert!(
            interner
                .schemas
                .values()
                .all(|schema| schema.upgrade().is_none())
        );
    }

    #[test]
    fn wide_custom_objects_keep_values_aligned_and_clone_on_write() {
        let mut map: ObjectMap<usize> = (0..200).map(|i| (k(&format!("key{i:04}")), i)).collect();
        let snapshot = map.clone();
        map.insert(k("key0200"), 200);
        map.remove(b"key0002".as_slice());
        map.retain(|key, value| {
            *value *= 2;
            key != &k("key0003")
        });
        assert_eq!(snapshot[&k("key0002")], 2);
        assert_eq!(map[&k("key0199")], 398);
        assert_eq!(map.len(), 199);
        assert!(map.iter().all(
            |(key, value)| key.to_string_lossy()[3..].parse::<usize>().unwrap() * 2 == *value
        ));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map.retain(|_, value| {
                if *value == 20 {
                    panic!("predicate");
                }
                *value != 0
            });
        }));
        assert!(panic.is_err());
        assert_eq!(map.len(), map.schema.keys.len());
        assert!(map.iter().all(
            |(key, value)| key.to_string_lossy()[3..].parse::<usize>().unwrap() * 2 == *value
        ));
    }
}
