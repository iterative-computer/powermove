//! The message-level patch envelope (contract 5, section 6).
//!
//! The renderer is the authority for the document. `pm-wasm` ingests leaf
//! patches, applies them with id-keyed reconciliation, and emits canonical
//! binary patches to engine sessions; the job engine subscribes to the same
//! stream. This module defines only the envelope around those patches:
//! revisions, origins, bodies, resyncs, and consistency checks.
//!
//! Patch semantics (paths, canonicalization, `v1_ingest`, reconciliation)
//! belong to `pm-model`; the body is opaque here and versioned by
//! [`PatchEncoding`]. The field table and the engine-originated facts table
//! from section 6 are data here because both sides must agree on them.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::blender::BlenderOutputFile;
use crate::common::{AssetId, ByteSize, Bytes, ContentHash, DocumentId, JobId, Revision};
use crate::host::CapabilityHandle;
use crate::validate::{ensure, Validate, Violation};
use crate::version::ContractId;

/// The encoding of a patch body, owned by pm-model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PatchEncoding {
    /// pm-model canonical binary patches, version 1.
    CanonicalV1,
}

/// The encoding of a whole-document resync body, owned by pm-model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DocumentEncoding {
    /// `PM.Comps.canonicalize(PM.proj)` serialized with sorted keys (JSON, UTF-8).
    CanonicalJsonV1,
    /// pm-model's canonical binary document, version 1.
    CanonicalBinaryV1,
}

/// Why a batch exists. Decides Undo and logging, never how it is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PatchOrigin {
    /// A user or agent edit: one intent, one Undo step.
    Edit,
    Undo,
    Redo,
    /// A metadata write that is not undoable (relink to a found file), recorded in the edit log.
    Metadata,
    /// A `touch()` without a path: the structural diff of the open composition.
    StructuralDiff,
    /// Project open or switch.
    Load,
}

/// Most bytes in one inline patch body. Larger edits split into several
/// batches or resync through a cache file.
pub const MAX_PATCH_BODY_BYTES: usize = 8 * 1024 * 1024;

/// One batch of canonical patches moving a document from `base_revision`
/// to `revision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PatchBatch {
    pub document: DocumentId,
    /// The revision the batch applies to; must equal the receiver's applied revision.
    pub base_revision: Revision,
    /// The revision after applying; greater than `base_revision`.
    pub revision: Revision,
    pub origin: PatchOrigin,
    pub encoding: PatchEncoding,
    /// Number of leaf patches in `body` (for logging and budget checks).
    pub patch_count: u32,
    pub body: Bytes,
}

impl Validate for PatchBatch {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Patch;
        ensure(self.document.is_valid(), C, || "invalid document id".into())?;
        ensure(self.revision > self.base_revision, C, || {
            format!("revision {} must exceed base revision {}", self.revision, self.base_revision)
        })?;
        ensure(self.patch_count > 0, C, || "a batch carries at least one patch".into())?;
        ensure(!self.body.0.is_empty() && self.body.0.len() <= MAX_PATCH_BODY_BYTES, C, || {
            format!("patch body must be 1 to {MAX_PATCH_BODY_BYTES} bytes, got {}", self.body.0.len())
        })
    }
}

/// Where a resync body is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum ResyncSource {
    Inline { body: Bytes },
    /// Large documents go through a cache file.
    CacheFile { handle: CapabilityHandle, length: ByteSize },
}

/// Replace the receiver's mirror with the canonical document at `revision`.
/// Sent on session open, after a revision gap, after a consistency mismatch,
/// and when a renderer reconnects after an engine restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Resync {
    pub document: DocumentId,
    pub revision: Revision,
    pub encoding: DocumentEncoding,
    pub source: ResyncSource,
    /// Canonical hash of the document at `revision`.
    pub hash: ContentHash,
}

/// The production consistency check: on idle, before save, and before every
/// export or agent export, compare canonical hashes at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyCheck {
    pub document: DocumentId,
    pub revision: Revision,
    pub hash: ContentHash,
}

/// The receiver's answer. A mismatch forces a canonical resync before the
/// save or export proceeds and records a telemetry event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyResult {
    pub revision: Revision,
    pub matches: bool,
    pub hash: ContentHash,
}

/// Acknowledges applied patches (sent at most once per frame, coalesced).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PatchesApplied {
    pub document: DocumentId,
    pub revision: Revision,
}

/// What the receiver does with an incoming batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Apply it; the applied revision becomes `revision`.
    Apply,
    /// Already applied (a duplicate after reconnect); ignore it.
    Duplicate,
}

/// The batch does not follow the applied revision; the receiver asks for a resync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("revision gap: applied {applied}, batch based on {base}")]
pub struct RevisionGap {
    pub applied: Revision,
    pub base: Revision,
}

/// Receiver-side revision bookkeeping for one document stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionTracker {
    applied: Revision,
}

impl RevisionTracker {
    pub fn new(applied: Revision) -> Self {
        Self { applied }
    }

    pub fn applied(&self) -> Revision {
        self.applied
    }

    /// Decides whether `batch` follows the applied revision. Call
    /// [`RevisionTracker::commit`] after applying it.
    pub fn admit(&self, batch: &PatchBatch) -> Result<Admission, RevisionGap> {
        if batch.revision <= self.applied {
            Ok(Admission::Duplicate)
        } else if batch.base_revision == self.applied {
            Ok(Admission::Apply)
        } else {
            Err(RevisionGap { applied: self.applied, base: batch.base_revision })
        }
    }

    pub fn commit(&mut self, batch: &PatchBatch) {
        self.applied = batch.revision;
    }

    pub fn resynced(&mut self, resync: &Resync) {
        self.applied = resync.revision;
    }

    /// Whether a request needing `minimum` can be answered now (read-your-writes).
    pub fn satisfies(&self, minimum: Revision) -> bool {
        self.applied >= minimum
    }
}

/// What the legacy engine does with an additive field while it can still be
/// selected (section 6 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LegacyBehavior {
    IgnoredSafely,
    /// Read by the Phase 0 `minReaderVersion` guard.
    ReadByGuard,
    /// The legacy engine refuses the project.
    LegacyRefuses,
    /// Requires the native engine (the project is stamped `requiresEngine: "native"`).
    RequiresNative,
    /// Requires native only when the stated condition holds.
    RequiresNativeWhen,
}

/// One row of the section 6 additive-field table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldEntry {
    /// Where the field lives.
    pub owner: &'static str,
    pub field: &'static str,
    pub legacy: LegacyBehavior,
    /// The condition for [`LegacyBehavior::RequiresNativeWhen`].
    pub condition: Option<&'static str>,
}

/// The section 6 additive fields, the input to pm-model's ingest and to the
/// `requiresEngine` stamping rules.
pub const FIELD_TABLE: [FieldEntry; 15] = [
    FieldEntry { owner: "composition", field: "fpsRational", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "root", field: "minReaderVersion", legacy: LegacyBehavior::ReadByGuard, condition: None },
    FieldEntry { owner: "root", field: "requiresEngine", legacy: LegacyBehavior::LegacyRefuses, condition: None },
    FieldEntry { owner: "layer", field: "compat", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "asset", field: "link", legacy: LegacyBehavior::RequiresNative, condition: None },
    FieldEntry { owner: "asset", field: "interpret", legacy: LegacyBehavior::RequiresNative, condition: None },
    FieldEntry { owner: "asset", field: "storage", legacy: LegacyBehavior::RequiresNativeWhen, condition: Some("value is \"linked\"") },
    FieldEntry { owner: "composition", field: "motionBlurSamples", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "composition", field: "audioSampleRate", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "precomp", field: "keepFrameRate", legacy: LegacyBehavior::RequiresNativeWhen, condition: Some("value is true") },
    FieldEntry { owner: "clip", field: "pitch", legacy: LegacyBehavior::RequiresNativeWhen, condition: Some("value is not the default") },
    FieldEntry { owner: "clip", field: "alwaysOriginal", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "precomp", field: "keepRendered", legacy: LegacyBehavior::IgnoredSafely, condition: None },
    FieldEntry { owner: "project", field: "color", legacy: LegacyBehavior::RequiresNativeWhen, condition: Some("value is not the default") },
    FieldEntry { owner: "project", field: "media", legacy: LegacyBehavior::IgnoredSafely, condition: None },
];

/// Existing fields `v1_ingest` takes as they are (section 6, "Existing
/// fields ingested as they are"). Both engines read them, so they never
/// stamp `requiresEngine`; they are listed so ingest and the stamping rules
/// agree on what is already engine-relevant. `(owner, path)`.
pub const INGESTED_AS_IS: [(&str, &str); 8] = [
    ("composition", "render3d"),
    ("layer", "d.data.object"),
    ("layer", "d.data.light"),
    ("layer", "d.data.camera"),
    ("layer", "d.data.environment"),
    ("group", "d.modeling"),
    ("asset", "fps"),
    ("asset", "format"),
];

/// How an engine-originated fact reaches the document (section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum FactCommit {
    /// Kept in the engine's media index or generated-media store, never in the document.
    NotCommitted,
    /// A metadata write, not undoable, in the edit log.
    MetadataWrite,
    /// An undoable user edit.
    UndoableEdit,
    /// An undoable edit, dropped if the document revision changed meanwhile.
    UndoableIfUnchanged,
}

/// A fact the engine learned that may change the document. The renderer
/// commits it according to [`EngineFact::commit`]; the engine never writes
/// the document itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum EngineFact {
    /// An offline asset was found by content hash, UMID, or content identifier.
    RelinkedByIdentity { asset: AssetId, hash: ContentHash, revision: Revision },
    /// A Blender generation or import finished (contract 11); the result
    /// applies only if the document is still at `revision`. `manifest` is
    /// the worker's manifest JSON; the renderer mints asset ids on commit.
    BlenderResult { job: JobId, revision: Revision, files: Vec<BlenderOutputFile>, manifest: String },
    /// A name, timecode, or duration match for an offline asset. Only a
    /// proposal: the user confirms it (Relink sheet), and the confirmation is
    /// an undoable edit (section 21).
    RelinkProposal { asset: AssetId, candidate: CapabilityHandle, reason: RelinkReason },
    /// Probe results and detected color and alpha for an asset:
    /// informational only. The other uncommitted facts travel elsewhere:
    /// Blender availability as `BlenderStatus`, font substitution and
    /// crash-attributed failures as failure and crash events.
    MediaFacts { asset: AssetId, hash: Option<ContentHash> },
}

/// Why a relink candidate was proposed (never applied automatically).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RelinkReason {
    Name,
    Timecode,
    Duration,
}

impl EngineFact {
    pub const fn commit(&self) -> FactCommit {
        match self {
            EngineFact::RelinkedByIdentity { .. } => FactCommit::MetadataWrite,
            EngineFact::BlenderResult { .. } => FactCommit::UndoableIfUnchanged,
            EngineFact::RelinkProposal { .. } => FactCommit::UndoableEdit,
            EngineFact::MediaFacts { .. } => FactCommit::NotCommitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(n: u64) -> Revision {
        Revision::new(n).unwrap()
    }

    fn batch(base: u64, revision: u64) -> PatchBatch {
        PatchBatch {
            document: "doc".into(),
            base_revision: rev(base),
            revision: rev(revision),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: 1,
            body: Bytes(vec![1, 2, 3]),
        }
    }

    #[test]
    fn batches_must_advance() {
        assert!(batch(1, 2).validate().is_ok());
        assert!(batch(2, 2).validate().is_err());
        let mut empty = batch(1, 2);
        empty.body = Bytes(vec![]);
        assert!(empty.validate().is_err());
    }

    #[test]
    fn tracker_detects_gaps_and_duplicates() {
        let mut tracker = RevisionTracker::new(rev(5));
        assert_eq!(tracker.admit(&batch(5, 6)), Ok(Admission::Apply));
        tracker.commit(&batch(5, 6));
        assert_eq!(tracker.admit(&batch(5, 6)), Ok(Admission::Duplicate));
        assert_eq!(tracker.admit(&batch(7, 8)), Err(RevisionGap { applied: rev(6), base: rev(7) }));
        assert!(tracker.satisfies(rev(6)) && !tracker.satisfies(rev(7)));
        tracker.resynced(&Resync {
            document: "doc".into(),
            revision: rev(8),
            encoding: DocumentEncoding::CanonicalJsonV1,
            source: ResyncSource::Inline { body: Bytes(b"{}".to_vec()) },
            hash: ContentHash([0; 32]),
        });
        assert_eq!(tracker.admit(&batch(8, 9)), Ok(Admission::Apply));
    }

    #[test]
    fn field_table_covers_section_6() {
        assert_eq!(FIELD_TABLE.len(), 15);
        for entry in FIELD_TABLE {
            assert_eq!(entry.condition.is_some(), entry.legacy == LegacyBehavior::RequiresNativeWhen, "{}", entry.field);
        }
        let mut names: Vec<_> = FIELD_TABLE.iter().map(|f| (f.owner, f.field)).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), FIELD_TABLE.len());
        for (owner, path) in INGESTED_AS_IS {
            assert!(!FIELD_TABLE.iter().any(|f| f.owner == owner && f.field == path), "{owner}.{path} is existing, not additive");
        }
    }

    #[test]
    fn facts_commit_per_section_6() {
        let hash = ContentHash([1; 32]);
        assert_eq!(EngineFact::RelinkedByIdentity { asset: "a".into(), hash, revision: rev(1) }.commit(), FactCommit::MetadataWrite);
        assert_eq!(EngineFact::MediaFacts { asset: "a".into(), hash: None }.commit(), FactCommit::NotCommitted);
        let candidate = CapabilityHandle::from_random_bytes([3; 16]);
        assert_eq!(EngineFact::RelinkProposal { asset: "a".into(), candidate, reason: RelinkReason::Timecode }.commit(), FactCommit::UndoableEdit);
    }
}
