# RFC 05: Patches (message envelope)

Status: draft, version 0. Module: `pm_protocol::patch`. Gates Phase 1;
freezes when Phase 1 starts. Owners: this RFC covers the message-level
envelope (Engine and platform). Patch semantics (paths, canonicalization,
`v1_ingest`, id-keyed reconciliation, the identity registry) belong to
`pm-model` (Model and evaluation lane) and the writer side to the Renderer
write path lane; their specification is added to this RFC by those lanes
before the freeze.

## Purpose

The renderer stays the authority for the document and Undo (plan
section 6). `pm-wasm` ingests the renderer's leaf patches, applies them with
id-keyed reconciliation, and emits canonical binary patches to engine
sessions; the job engine subscribes to the same stream so agent snapshots
read their own writes. This contract is the envelope around that stream:
revisions, origins, opaque bodies with a versioned encoding, resyncs, the
production consistency check, and how engine-originated facts reach the
document. It also carries the section 6 additive-field table as data,
because the stamping rules and `v1_ingest` must agree on it.

## Types

| Type | Meaning |
|---|---|
| `PatchBatch` | Document, `base_revision`, `revision`, `PatchOrigin`, `PatchEncoding`, patch count, body bytes. Sent as `Event::ApplyPatches` from `pm-wasm` to engine sessions |
| `PatchEncoding` | `canonicalV1`: `pm-model`'s canonical binary patches. The body is opaque to this crate |
| `PatchOrigin` | Edit (one intent, one Undo step), undo, redo, metadata (not undoable, recorded in the edit log), structural diff (a `touch()` without a path), load. Decides Undo and logging, never how a batch is applied |
| `Resync`, `ResyncSource`, `DocumentEncoding` | Replace the mirror with the canonical document at a revision: inline or through a cache file behind a capability handle; `canonicalJsonV1` (`PM.Comps.canonicalize(PM.proj)` with sorted keys) or `canonicalBinaryV1`; with the canonical hash |
| `ConsistencyCheck`, `ConsistencyResult` | Compare canonical hashes at one revision |
| `PatchesApplied` | Coalesced acknowledgement (at most once per frame) |
| `RevisionTracker`, `Admission`, `RevisionGap` | Receiver bookkeeping: apply, ignore a duplicate, or detect a gap and ask for a resync; `satisfies(minimum)` for read-your-writes |
| `FIELD_TABLE`, `FieldEntry`, `LegacyBehavior` | The fifteen section 6 additive fields and what the legacy engine does with each |
| `INGESTED_AS_IS` | The section 6 existing fields ingested as they are |
| `EngineFact`, `FactCommit`, `RelinkReason` | Facts the engine (or host) learned that may change the document, and how each is committed |

## Invariants

1. **Revisions only move forward.** A batch's `revision` exceeds its
   `base_revision`; it carries at least one patch and 1 byte to 8 MiB of
   body (`MAX_PATCH_BODY_BYTES`); larger edits split or resync through a
   cache file. Checked by `PatchBatch::validate`.
2. **A batch applies only to its base.** The receiver applies a batch whose
   `base_revision` equals its applied revision, ignores one at or below it
   (a duplicate after reconnect), and treats anything else as a gap that
   requires a resync (`RevisionTracker`, tested).
3. **Read-your-writes.** A request's envelope revision is the minimum
   revision the receiver must have applied before answering (contract 9).
   The receiver waits for patches up to its wait bound, then fails with
   `StaleRevision` and `Retry::AfterResync`.
4. **Resync** happens on session open, after a gap, after a consistency
   mismatch, and when a renderer reconnects after an engine restart.
5. **Production consistency check** (section 6, item 7): on idle, before
   save, and before every export or agent export, the renderer compares a
   canonical hash of `PM.proj` with `pm-wasm`'s hash at that revision. A
   mismatch forces a canonical resync before the save or export proceeds
   and records a telemetry event.
6. **The engine never writes the document.** Engine-originated facts are
   events; the renderer commits them per `EngineFact::commit`:

   | Fact | Commit |
   |---|---|
   | `relinkedByIdentity` (content hash, UMID, content identifier) | Metadata write, not undoable, in the edit log |
   | `relinkProposal` (name, timecode, duration) | Only after the user confirms; then an undoable edit |
   | `blenderResult` (generation or `.blend` import, contract 11) | Undoable edit, dropped if the revision changed meanwhile |
   | `mediaFacts` (probe results, detected color and alpha) | Not committed |
   | Blender availability (`BlenderStatus`), font substitution and crash-attributed failures (`Event::Failure`, `Event::Crash`) | Not committed; not facts, so never offered for commit |

7. **Field table.** Every additive field from section 6 appears once, with
   its legacy behavior; conditional rows carry their condition (tested).
   Existing fields that `v1_ingest` takes as they are (composition
   `render3d`; layer `d.data.{object,light,camera,environment}`; group
   `d.modeling`; asset `fps` and `format`) are a separate list,
   `INGESTED_AS_IS`, and never appear in the additive table (tested).

Owned by `pm-model` (added to this RFC before the freeze): the canonical
binary patch format, leaf paths and canonical paths without the
open-composition root rotation, canonicalization, `v1_ingest`
(section 5's time snapping and the frame-rate mapping), id-keyed
reconciliation, the canonical hash function, and the identity registry.

## Versioning

Version 0 until Phase 1 starts. The envelope and the body format version
separately: a new `pm-model` patch format is a new `PatchEncoding` variant
(and a contract bump), so peers negotiate it like any other change.
`DocumentEncoding::CanonicalJsonV1` stays readable for resyncs from the
renderer, which produces JSON natively.

## What Phase 0 must confirm

- **S5 (WASM evaluator):** patch apply under 0.2 ms for a property change
  and 2 ms for a structural change on the 884-layer project, with the batch
  framing here (one `PatchBatch` per renderer flush). If S5 shows that the
  per-batch overhead matters, batches coalesce per frame.
- **The writer inventory** (`scripts/engine-writers.mjs`): that every writer
  class maps to a `PatchOrigin`.
- **The shadow-replay auditor:** that the stream of batches, applied to a JS
  shadow, equals `PM.Comps.canonicalize(PM.proj)` at frame boundaries. This
  is a Phase 1 gate, but its harness is built against this envelope.

## Open questions

1. Whether `PatchBatch` should carry the canonical hash after applying, so
   every batch doubles as a consistency check. It costs a hash per batch on
   the renderer; S5 measures.
2. Whether the job engine should subscribe to the interactive engine's
   stream (one producer) or receive its own copy from `pm-wasm` (two
   sends). Section 6 says the job engine subscribes to the same stream;
   which process fans out is decided with the frame bridge in S1.
3. Asset-level facts that must survive a relaunch (a found relink) are
   metadata writes; whether they need their own `PatchOrigin` beyond
   `metadata` is up to the writer inventory.
4. The body format. On `feat/ne-model` (2026-10-11), `pm-model` encodes
   patch lists as JSON in `shared/patch.ts`'s leaf form
   (`patches_to_value`, `patches_from_value`) and has no binary format yet.
   If the canonical binary format is not specified when Phase 1 starts,
   the fallback is a second variant, `leafJsonV1` (UTF-8 JSON leaf
   patches), added before the freeze so `pm-wasm` and the engines can run
   the stream end to end. `PatchEncoding` has only `canonicalV1` until
   `pm-model` decides.
