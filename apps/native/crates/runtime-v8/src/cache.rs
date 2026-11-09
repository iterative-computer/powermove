use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotKind {
    Editor,
    Rest,
    Agent,
    Extension,
    Compat,
    Compiler,
    Store,
}
impl SnapshotKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::Rest => "rest",
            Self::Agent => "agent",
            Self::Extension => "extension",
            Self::Compat => "compat",
            Self::Compiler => "compiler",
            Self::Store => "store",
        }
    }
}

/// Domain-separated and length-delimited. No concatenation ambiguity or secret inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(pub [u8; 32]);
impl CacheKey {
    pub fn new(version: &str, snapshot: &[u8], bundle: &[u8], flags: &str) -> Self {
        Self::prefix(version, snapshot, flags).key(bundle)
    }
    /// Everything but the bundle, hashed once per isolate: a snapshot is
    /// megabytes, and every eval is keyed.
    pub fn prefix(version: &str, snapshot: &[u8], flags: &str) -> CachePrefix {
        let mut h = Sha256::new();
        for part in [
            b"pm-runtime-v8/cache/v1".as_slice(),
            version.as_bytes(),
            snapshot,
            flags.as_bytes(),
        ] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
        CachePrefix(h.finalize().into())
    }
}
/// The digest of a cache key's V8 version, snapshot and flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePrefix([u8; 32]);
impl CachePrefix {
    pub fn key(&self, bundle: &[u8]) -> CacheKey {
        let mut h = Sha256::new();
        h.update(self.0);
        h.update((bundle.len() as u64).to_le_bytes());
        h.update(bundle);
        CacheKey(h.finalize().into())
    }
}
/// Bounded cache: eviction only removes derived code, never authored sources/storage.
#[derive(Clone)]
pub struct CodeCache {
    inner: Arc<Mutex<Cache>>,
    budget: usize,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<CacheKey, Vec<u8>>,
    bytes: usize,
}
impl CodeCache {
    pub fn new(budget: usize) -> Self {
        Self {
            inner: Default::default(),
            budget,
        }
    }
    pub fn get(&self, key: CacheKey) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().entries.get(&key).cloned()
    }
    pub fn insert(&self, key: CacheKey, data: Vec<u8>) {
        if data.len() > self.budget {
            return;
        }
        let mut c = self.inner.lock().unwrap();
        if let Some(old) = c.entries.remove(&key) {
            c.bytes -= old.len();
        }
        if c.bytes + data.len() > self.budget {
            c.entries.clear();
            c.bytes = 0;
        }
        c.bytes += data.len();
        c.entries.insert(key, data);
    }
}
