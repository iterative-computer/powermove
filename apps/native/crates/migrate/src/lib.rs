//! Read-only Chromium snapshots and a resumable, additive media-store importer.
//!
//! No code in this crate opens a live LevelDB database or accesses Keychain.
//! Start with [`snapshot::Snapshot::capture`]. Corrupt or unsupported encodings
//! fail rather than appearing as an empty library. Closure needs the complete project,
//! history, take, trash and agent-artifact inventory; see [`migration`].
pub mod indexeddb;
pub mod leveldb;
pub mod library_media;
pub mod local_storage;
pub mod media_reader;
pub mod migration;
pub mod snapshot;
pub mod source_access;
pub mod storage_upgrade;
pub mod structured_clone;
mod wire;
pub mod workspace;

pub use anyhow::Result;
