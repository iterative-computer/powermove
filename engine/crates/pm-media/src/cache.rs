//! The decoded frame cache, under a lease (contract 8).
//!
//! Frames are keyed by source, sample, and decode variant. The cache holds
//! at most its budget in bytes; the engine sets the budget from its lease's
//! frame-cache share and lowers it when main shrinks the lease (playback
//! start, memory pressure), and the cache evicts least recently used frames
//! until it fits. A frame evicted while something still renders from it
//! stays alive until that use ends; usage reports count what the cache
//! holds.

use crate::frame::DecodedFrame;
use crate::vt::DecodePurpose;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Identifies a decoded frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub source: u64,
    /// Decode-order sample index.
    pub sample: usize,
    pub purpose: DecodePurpose,
}

struct Entry {
    frame: DecodedFrame,
    bytes: usize,
    /// The frame lives in the software decoders' pool (not a CoreVideo buffer).
    pooled: bool,
    stamp: u64,
}

#[derive(Default)]
struct Inner {
    map: HashMap<FrameKey, Entry>,
    lru: BTreeMap<u64, FrameKey>,
    bytes: usize,
    pool_bytes: usize,
    budget: usize,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// Cache statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub frames: usize,
    pub bytes: usize,
    /// Of `bytes`, frames in the decode pool (the rest are CoreVideo buffers).
    pub pool_bytes: usize,
    pub budget: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

pub struct FrameCache {
    inner: Mutex<Inner>,
}

impl FrameCache {
    pub fn new(budget_bytes: usize) -> FrameCache {
        FrameCache {
            inner: Mutex::new(Inner {
                budget: budget_bytes,
                ..Default::default()
            }),
        }
    }

    pub fn get(&self, key: &FrameKey) -> Option<DecodedFrame> {
        let mut g = self.inner.lock().unwrap();
        g.clock += 1;
        let clock = g.clock;
        let Some(e) = g.map.get_mut(key) else {
            g.misses += 1;
            return None;
        };
        let old = std::mem::replace(&mut e.stamp, clock);
        let frame = e.frame.clone();
        g.lru.remove(&old);
        g.lru.insert(clock, *key);
        g.hits += 1;
        Some(frame)
    }

    /// The cached samples of a source and purpose, without touching their
    /// recency.
    pub fn samples_of(&self, source: u64, purpose: DecodePurpose) -> Vec<usize> {
        self.inner
            .lock()
            .unwrap()
            .map
            .keys()
            .filter(|k| k.source == source && k.purpose == purpose)
            .map(|k| k.sample)
            .collect()
    }

    pub fn contains(&self, key: &FrameKey) -> bool {
        self.inner.lock().unwrap().map.contains_key(key)
    }

    /// Inserts a frame; returns false when it alone exceeds the budget (it
    /// is then not kept).
    pub fn insert(&self, key: FrameKey, frame: DecodedFrame) -> bool {
        let bytes = frame.bytes();
        let pooled = matches!(frame.pixels, crate::frame::Pixels::Pool(_));
        let mut g = self.inner.lock().unwrap();
        if bytes > g.budget {
            return false;
        }
        g.clock += 1;
        let stamp = g.clock;
        if let Some(old) = g.map.insert(
            key,
            Entry {
                frame,
                bytes,
                pooled,
                stamp,
            },
        ) {
            g.lru.remove(&old.stamp);
            Self::forget(&mut g, &old);
        }
        g.lru.insert(stamp, key);
        g.bytes += bytes;
        if pooled {
            g.pool_bytes += bytes;
        }
        Self::evict(&mut g);
        true
    }

    /// Sets the budget (a lease grant or shrink) and evicts down to it.
    /// Returns the bytes held afterwards, which acknowledges the lease.
    pub fn set_budget(&self, bytes: usize) -> usize {
        let mut g = self.inner.lock().unwrap();
        g.budget = bytes;
        Self::evict(&mut g);
        g.bytes
    }

    /// Drops every frame of a source (the source changed or went offline).
    pub fn remove_source(&self, source: u64) {
        let mut g = self.inner.lock().unwrap();
        let keys: Vec<FrameKey> = g
            .map
            .keys()
            .filter(|k| k.source == source)
            .copied()
            .collect();
        for k in keys {
            if let Some(e) = g.map.remove(&k) {
                g.lru.remove(&e.stamp);
                Self::forget(&mut g, &e);
            }
        }
    }

    fn evict(g: &mut Inner) {
        while g.bytes > g.budget {
            let Some((_, key)) = g.lru.pop_first() else {
                break;
            };
            if let Some(e) = g.map.remove(&key) {
                Self::forget(g, &e);
                g.evictions += 1;
            }
        }
    }

    fn forget(g: &mut Inner, e: &Entry) {
        g.bytes -= e.bytes;
        if e.pooled {
            g.pool_bytes -= e.bytes;
        }
    }

    pub fn stats(&self) -> CacheStats {
        let g = self.inner.lock().unwrap();
        CacheStats {
            frames: g.map.len(),
            bytes: g.bytes,
            pool_bytes: g.pool_bytes,
            budget: g.budget,
            hits: g.hits,
            misses: g.misses,
            evictions: g.evictions,
        }
    }
}
