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
        let mut h = Sha256::new();
        for part in [
            b"pm-runtime-v8/cache/v1".as_slice(),
            version.as_bytes(),
            snapshot,
            bundle,
            flags.as_bytes(),
        ] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
        Self(h.finalize().into())
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
