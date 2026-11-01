//! Read-only Chromium snapshots and a resumable, additive media-store importer.
//!
//! No code in this crate opens a live LevelDB database or accesses Keychain.
//! Start with [`snapshot::Snapshot::capture`]. Corrupt or unsupported encodings
//! fail rather than appearing as an empty library. Closure needs the complete project,
//! history, take, trash and agent-artifact inventory; see [`migration`].
pub mod indexeddb;
pub mod leveldb;
pub mod local_storage;
pub mod migration;
pub mod snapshot;
pub mod structured_clone;
mod wire;

pub use anyhow::Result;
