//! The interactive engine's document host (plan section 6, "The patch
//! stream"; contract 5): what the engine does with a renderer session's
//! canonical document besides keeping its mirror.
//!
//! * [`decode_canonical`]: the built-in `canonicalV1` body decoder
//!   (pm-model's binary patches), so a session follows the same patch
//!   stream pm-wasm receives without a decoder installed by the host.
//! * [`DocumentUpdate`]: what the presenter is told after a resync or a
//!   batch, so it keeps its own mirror on its render thread (the engine's
//!   mirror stays on the JavaScript thread for consistency checks).
//! * [`MediaTable`]: which of the document's assets the engine has asked
//!   the host to resolve (contract 14: a `mediaRead` handle minted for the
//!   session, then `resolveHandle` on the host port), with retries for
//!   assets the host could not resolve yet (the renderer binds a project's
//!   media to main a moment after it opens).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use pm_model::{Node, NodeKind, Op, Patch};
use pm_protocol::common::Revision;
use pm_protocol::patch::PatchEncoding;

/// Decodes a `canonicalV1` body into pm-model operations.
pub fn decode_canonical(encoding: PatchEncoding, body: &[u8]) -> Result<Vec<Op>, String> {
    match encoding {
        PatchEncoding::CanonicalV1 => pm_model::binary::decode(body).map_err(|e| e.to_string()),
    }
}

/// A change to a session's document, as the presenter receives it.
#[derive(Debug, Clone)]
pub enum DocumentUpdate {
    /// The whole document (a resync): replaces the presenter's mirror. The
    /// tree shares its subtrees with the engine's mirror (pm-model's shared
    /// subtrees), so handing it to the render and audio threads copies
    /// nothing.
    Replace {
        root: Option<Node>,
        revision: Revision,
    },
    /// Canonical operations taking the mirror to `revision`.
    Ops { ops: Arc<[Op]>, revision: Revision },
    /// Leaf patches taking the mirror to `revision` (a host-installed
    /// decoder that produces patches rather than operations).
    Patches {
        patches: Arc<[Patch]>,
        revision: Revision,
    },
}

impl DocumentUpdate {
    pub fn revision(&self) -> Revision {
        match self {
            DocumentUpdate::Replace { revision, .. }
            | DocumentUpdate::Ops { revision, .. }
            | DocumentUpdate::Patches { revision, .. } => *revision,
        }
    }

    /// Applies the update to a mirror. A mirror that cannot take it is
    /// replaced only by a [`DocumentUpdate::Replace`]; the caller asks for
    /// a resync otherwise.
    pub fn apply_to(&self, doc: &mut pm_model::Document) -> Result<(), String> {
        match self {
            DocumentUpdate::Replace { root, revision } => {
                *doc = pm_model::Document::from_root(root.clone(), revision.get());
                Ok(())
            }
            DocumentUpdate::Ops { ops, revision } => doc
                .apply_ops_at(ops, revision.get())
                .map(|_| ())
                .map_err(|e| e.to_string()),
            DocumentUpdate::Patches { patches, revision } => doc
                .apply_at(patches, revision.get())
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }
}

/// Retry delays for an asset the host could not resolve, in µs. After the
/// last one the asset stays offline until its record changes or the
/// document is resynced.
pub const MEDIA_RETRY_US: [u64; 6] = [
    250_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 15_000_000,
];

/// One asset's resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaState {
    /// A `mintHandle` is outstanding.
    Minting,
    /// A `resolveHandle` is outstanding.
    Resolving,
    /// The host answered with this file.
    Resolved(PathBuf),
    /// The host could not resolve it (offline, not bound yet).
    Offline,
}

#[derive(Debug, Clone)]
struct MediaEntry {
    /// The canonical digest of the asset's record when resolution started:
    /// a changed record (relink, Replace File…) is resolved again.
    fingerprint: [u8; 32],
    /// Bumped at every new attempt, so a late answer to an old one is
    /// ignored.
    generation: u64,
    state: MediaState,
    failures: u32,
    retry_at: Option<u64>,
}

/// An attempt to start: ask the host to mint a `mediaRead` handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAttempt {
    pub asset: String,
    pub generation: u64,
}

/// What reconciling a document with the table produced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MediaWork {
    /// Assets to resolve (new, or their record changed).
    pub start: Vec<MediaAttempt>,
    /// Assets that left the document.
    pub removed: Vec<String>,
}

/// The assets of one session's document and how far their resolution got.
#[derive(Debug, Default, Clone)]
pub struct MediaTable {
    entries: BTreeMap<String, MediaEntry>,
    next_generation: u64,
}

/// The record's digest without `poster`: generated thumbnail metadata,
/// persisted after import, never changes what the media resolves to.
/// Restarting on it would drop a resolved source read until the host binds
/// again.
fn media_fingerprint(record: &Node) -> [u8; 32] {
    if record.get("poster").is_none() {
        return pm_model::hash::canonical_digest(record);
    }
    let mut value = record.to_value();
    if let pm_model::Value::Object(map) = &mut value {
        map.remove("poster");
    }
    pm_model::hash::canonical_digest(&value)
}

/// The `assets` member of a canonical document: asset id → record.
pub fn asset_records(root: Option<&Node>) -> Vec<(String, &Node)> {
    let Some(assets) = root.and_then(|r| r.get("assets")) else {
        return Vec::new();
    };
    let NodeKind::Object(map) = assets.kind() else {
        return Vec::new();
    };
    map.iter()
        .filter(|(_, record)| matches!(record.kind(), NodeKind::Object(_)))
        .map(|(id, record)| (id.to_string_lossy(), record))
        .filter(|(id, _)| !id.is_empty() && id.len() <= pm_protocol::common::MAX_ID_BYTES)
        .collect()
}

impl MediaTable {
    fn generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    /// Compares the document's assets with the table: new assets and
    /// assets whose record changed are (re)started, assets that left are
    /// dropped. An asset whose record did not change keeps its answer.
    pub fn reconcile(&mut self, root: Option<&Node>) -> MediaWork {
        let mut work = MediaWork::default();
        let records = asset_records(root);
        let present: std::collections::BTreeSet<&str> =
            records.iter().map(|(id, _)| id.as_str()).collect();
        let removed: Vec<String> = self
            .entries
            .keys()
            .filter(|id| !present.contains(id.as_str()))
            .cloned()
            .collect();
        for id in removed {
            self.entries.remove(&id);
            work.removed.push(id);
        }
        for (id, record) in records {
            let fingerprint = media_fingerprint(record);
            if self
                .entries
                .get(&id)
                .is_some_and(|e| e.fingerprint == fingerprint)
            {
                continue;
            }
            let generation = self.generation();
            self.entries.insert(
                id.clone(),
                MediaEntry {
                    fingerprint,
                    generation,
                    state: MediaState::Minting,
                    failures: 0,
                    retry_at: None,
                },
            );
            work.start.push(MediaAttempt {
                asset: id,
                generation,
            });
        }
        work
    }

    /// Forgets every answer (a resync of the whole document starts over).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Whether `generation` is still the attempt in force for `asset`.
    pub fn current(&self, asset: &str, generation: u64) -> bool {
        self.entries
            .get(asset)
            .is_some_and(|e| e.generation == generation)
    }

    /// The handle was minted; a resolve follows.
    pub fn minted(&mut self, asset: &str, generation: u64) -> bool {
        match self.entries.get_mut(asset) {
            Some(e) if e.generation == generation => {
                e.state = MediaState::Resolving;
                true
            }
            _ => false,
        }
    }

    /// The host answered with a file. Returns false for a stale answer.
    pub fn resolved(&mut self, asset: &str, generation: u64, path: PathBuf) -> bool {
        match self.entries.get_mut(asset) {
            Some(e) if e.generation == generation => {
                e.state = MediaState::Resolved(path);
                e.failures = 0;
                e.retry_at = None;
                true
            }
            _ => false,
        }
    }

    /// The host could not resolve the asset at host µs `now`: it is
    /// offline, and tried again after [`MEDIA_RETRY_US`]. Returns false
    /// for a stale answer.
    pub fn failed(&mut self, asset: &str, generation: u64, now: u64) -> bool {
        match self.entries.get_mut(asset) {
            Some(e) if e.generation == generation => {
                e.state = MediaState::Offline;
                e.retry_at = MEDIA_RETRY_US
                    .get(e.failures as usize)
                    .map(|delay| now.saturating_add(*delay));
                e.failures += 1;
                true
            }
            _ => false,
        }
    }

    /// Offline assets whose retry is due at host µs `now`, restarted.
    pub fn due(&mut self, now: u64) -> Vec<MediaAttempt> {
        let due: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.state == MediaState::Offline && e.retry_at.is_some_and(|t| t <= now))
            .map(|(id, _)| id.clone())
            .collect();
        due.into_iter()
            .map(|asset| {
                let generation = self.generation();
                let e = self.entries.get_mut(&asset).expect("listed");
                e.generation = generation;
                e.state = MediaState::Minting;
                e.retry_at = None;
                MediaAttempt { asset, generation }
            })
            .collect()
    }

    /// The earliest pending retry.
    pub fn next_retry(&self) -> Option<u64> {
        self.entries.values().filter_map(|e| e.retry_at).min()
    }

    pub fn state(&self, asset: &str) -> Option<&MediaState> {
        self.entries.get(asset).map(|e| &e.state)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse_bytes;

    fn doc(json: &str) -> pm_model::Document {
        pm_model::Document::new(Some(parse_bytes(json.as_bytes()).unwrap()), 1)
    }

    #[test]
    fn reconcile_starts_new_and_changed_assets_and_drops_removed_ones() {
        let mut table = MediaTable::default();
        let first = doc(r#"{"assets":{"a":{"id":"a","name":"a.mov"},"b":{"id":"b"}}}"#);
        let work = table.reconcile(first.root());
        assert_eq!(
            work.start
                .iter()
                .map(|a| a.asset.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(work.removed.is_empty());
        let a = work.start[0].clone();
        assert!(table.minted("a", a.generation));
        assert!(table.resolved("a", a.generation, "/m/a".into()));

        // Same records: nothing to do, answers kept.
        assert_eq!(table.reconcile(first.root()), MediaWork::default());
        assert_eq!(table.state("a"), Some(&MediaState::Resolved("/m/a".into())));

        // A persisted poster flag is not a media change.
        let posted = doc(r#"{"assets":{"a":{"id":"a","name":"a.mov","poster":true},"b":{"id":"b"}}}"#);
        assert_eq!(table.reconcile(posted.root()), MediaWork::default());
        assert_eq!(table.state("a"), Some(&MediaState::Resolved("/m/a".into())));

        // A changed record (relink) and a removed asset.
        let second = doc(r#"{"assets":{"a":{"id":"a","name":"a.mov","link":{"path":"/x"}}}}"#);
        let work = table.reconcile(second.root());
        assert_eq!(work.start.len(), 1);
        assert_eq!(work.start[0].asset, "a");
        assert_ne!(work.start[0].generation, a.generation);
        assert_eq!(work.removed, ["b"]);
        // The old attempt's late answer is ignored.
        assert!(!table.resolved("a", a.generation, "/old".into()));
    }

    #[test]
    fn offline_assets_retry_with_backoff_then_wait() {
        let mut table = MediaTable::default();
        let d = doc(r#"{"assets":{"a":{"id":"a"}}}"#);
        let mut attempt = table.reconcile(d.root()).start.remove(0);
        let mut now = 1_000;
        for delay in MEDIA_RETRY_US {
            assert!(table.failed("a", attempt.generation, now));
            assert_eq!(table.next_retry(), Some(now + delay));
            assert!(table.due(now + delay - 1).is_empty());
            now += delay;
            let due = table.due(now);
            assert_eq!(due.len(), 1);
            attempt = due[0].clone();
        }
        assert!(table.failed("a", attempt.generation, now));
        assert_eq!(table.next_retry(), None);
        assert!(table.due(u64::MAX).is_empty());
        assert_eq!(table.state("a"), Some(&MediaState::Offline));
    }

    #[test]
    fn updates_apply_to_a_mirror() {
        let mut mirror = doc(r#"{"layers":[]}"#);
        let after = doc(r#"{"layers":[{"id":"x"}]}"#);
        let ops = pm_model::ops::diff_ops(mirror.to_value().as_ref(), after.to_value().as_ref());
        let body = pm_model::binary::encode(&ops).unwrap();
        let decoded = decode_canonical(PatchEncoding::CanonicalV1, &body).unwrap();
        let update = DocumentUpdate::Ops {
            ops: decoded.into(),
            revision: Revision::new(2).unwrap(),
        };
        update.apply_to(&mut mirror).unwrap();
        assert_eq!(mirror.to_value(), after.to_value());
        assert_eq!(mirror.revision(), 2);
        assert!(decode_canonical(PatchEncoding::CanonicalV1, b"\xff\x00garbage").is_err());
    }
}
