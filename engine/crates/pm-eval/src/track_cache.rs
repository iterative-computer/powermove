//! Prepared keyframe tracks kept across evaluations, by the identity of
//! their `kf` array.
//!
//! `PreparedTrack::prepare` adopts every key of a track (today's
//! `adoptTemporalEase`, auto-bezier tangents) before one value is read.
//! Today's `temporalEvaluate` caches that work per array object and version;
//! the engine prepared it again on every read. A project baked to a key per
//! frame (hundreds of keys on every channel of every layer) spent most of a
//! frame there, several times per frame.
//!
//! pm-model arrays are immutable behind an `Arc` while shared (an edit
//! copies the array it changes), so an array's address names its content
//! for as long as the cache holds a clone of the node: the address cannot
//! be reused for another array meanwhile. A changed track is a new array,
//! a new address, and is prepared again, as today's version bump does.

use std::collections::HashMap;
use std::sync::Arc;

use pm_model::{Node, NodeKind};

use crate::easing::SpringTables;
use crate::keyframes::PreparedTrack;

/// Tracks shorter than this are prepared on every read (it costs less
/// than the lookup).
pub const MIN_KEYS: usize = 8;
/// Most memory the prepared tracks take (estimated per key); the least
/// recently used go first.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// The estimated bytes a prepared key takes (time, value, curve).
const BYTES_PER_KEY: usize = 128;

struct Entry {
    /// Keeps the array alive, so its address names it.
    _kf: Node,
    track: PreparedTrack,
    bytes: usize,
    used: u64,
}

/// See the module documentation.
#[derive(Default)]
pub struct TrackCache {
    entries: HashMap<usize, Entry>,
    bytes: usize,
    tick: u64,
    /// Reads answered from a kept track, and tracks prepared.
    pub hits: u64,
    pub prepared: u64,
}

impl TrackCache {
    /// The prepared track of `kf`, from the cache when it holds this array.
    /// `None` when `kf` is not an array long enough to keep.
    pub fn get(&mut self, kf: Option<&Node>, springs: &mut SpringTables) -> Option<&PreparedTrack> {
        let node = kf?;
        let NodeKind::Array(keys) = node.kind() else {
            return None;
        };
        if keys.len() < MIN_KEYS {
            return None;
        }
        let address = Arc::as_ptr(keys) as usize;
        self.tick += 1;
        let tick = self.tick;
        if self.entries.contains_key(&address) {
            self.hits += 1;
            let entry = self.entries.get_mut(&address).expect("checked");
            entry.used = tick;
            return Some(&entry.track);
        }
        let track = PreparedTrack::prepare(kf, springs);
        self.prepared += 1;
        let bytes = keys.len() * BYTES_PER_KEY;
        if bytes > MAX_BYTES {
            return None;
        }
        self.make_room(bytes);
        self.bytes += bytes;
        let entry = self.entries.entry(address).or_insert(Entry {
            _kf: node.clone(),
            track,
            bytes,
            used: tick,
        });
        Some(&entry.track)
    }

    /// Evicts the least recently used tracks until `incoming` more bytes fit.
    fn make_room(&mut self, incoming: usize) {
        if self.bytes + incoming <= MAX_BYTES {
            return;
        }
        let mut by_age: Vec<(u64, usize)> = self.entries.iter().map(|(k, e)| (e.used, *k)).collect();
        by_age.sort_unstable();
        for (_, address) in by_age {
            if self.bytes + incoming <= MAX_BYTES {
                break;
            }
            if let Some(e) = self.entries.remove(&address) {
                self.bytes -= e.bytes;
            }
        }
    }

    /// Tracks held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Estimated bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyframes::evaluate;

    fn track(values: &[f64]) -> Node {
        let keys: Vec<serde_json::Value> = values
            .iter()
            .enumerate()
            .map(|(i, v)| serde_json::json!({ "t": i as f64 / 30.0, "v": v, "ease": "linear" }))
            .collect();
        let text = serde_json::Value::Array(keys).to_string();
        Node::from_value(pm_model::json::parse(&text).unwrap(), 0)
    }

    #[test]
    fn a_kept_track_answers_exactly_what_preparing_it_again_does() {
        let mut springs = SpringTables::new();
        let mut cache = TrackCache::default();
        let values: Vec<f64> = (0..200).map(|i| ((i * 37) % 101) as f64 - 50.0).collect();
        let kf = track(&values);
        for k in 0..400 {
            let t = k as f64 / 61.0 - 0.5;
            let kept = cache
                .get(Some(&kf), &mut springs)
                .expect("a long track is kept")
                .clone()
                .value(Some(&kf), t, false, &mut springs)
                .unwrap();
            let fresh = evaluate(Some(&kf), t, false, &mut springs).unwrap();
            assert_eq!(format!("{kept:?}"), format!("{fresh:?}"), "at {t}");
        }
        assert_eq!(cache.prepared, 1);
        assert_eq!(cache.hits, 399);
    }

    #[test]
    fn an_edited_track_is_a_new_array_and_is_prepared_again() {
        let mut springs = SpringTables::new();
        let mut cache = TrackCache::default();
        let a = track(&(0..20).map(f64::from).collect::<Vec<_>>());
        let b = track(&(0..20).map(|i| f64::from(i) * 2.0).collect::<Vec<_>>());
        let at = |cache: &mut TrackCache, kf: &Node, springs: &mut SpringTables| {
            let track = cache.get(Some(kf), springs).unwrap().clone();
            track.value(Some(kf), 0.25, false, springs).unwrap()
        };
        let first = at(&mut cache, &a, &mut springs);
        let second = at(&mut cache, &b, &mut springs);
        assert_ne!(format!("{first:?}"), format!("{second:?}"));
        assert_eq!(cache.prepared, 2);
        // A clone shares the array: the same track.
        let shared = a.clone();
        assert_eq!(format!("{:?}", at(&mut cache, &shared, &mut springs)), format!("{first:?}"));
        assert_eq!(cache.prepared, 2);
        // Short tracks are not kept.
        let short = track(&[1.0, 2.0, 3.0]);
        assert!(cache.get(Some(&short), &mut springs).is_none());
    }
}
