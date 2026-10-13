# RFC 05: Patches (message envelope)

Status: draft, version 0 for the envelope; the body format `canonicalV1`
is final (2026-10-12). Modules: `pm_protocol::patch` (envelope),
`pm_model::ops` and `pm_model::binary` (bodies), and on the renderer
`apps/desktop/src/shared/patch-ids.ts` and `patch-binary.ts`. Gates
Phase 1; freezes when Phase 1 starts. Owners: the message-level envelope
belongs to Engine and platform; patch semantics, the body format, time at
the boundary, and canonicalization to `pm-model` (Model and evaluation
lane), specified below under "Patch bodies"; the writer side (which
writes produce which operations) to the Renderer write path lane.

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
| `PatchEncoding` | `canonicalV1`: `pm-model`'s canonical binary patches ("Patch bodies" below). The body is opaque to this crate |
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

8. **Body and count agree.** A receiver decodes the body with
   `pm_model::binary::decode` and rejects the batch (and resyncs) when the
   decoded operation count differs from the batch's patch count.

## Patch bodies (`pm-model`)

This section is normative for `PatchEncoding::CanonicalV1`. It specifies
the operations a body carries, how they apply, how the renderer generates
them, and their byte format.

### Operations

A body is a list of operations. Each is a `shared/patch.ts` leaf patch or
one of three id-keyed structural operations (section 26, amendment 3):

| Operation | JSON form | Effect on the array at `path` |
|---|---|---|
| Leaf set | `{ path, exists: true, value }` | `applyPatch` semantics exactly (`pm_model::patch`, checked on 1,548 vectors from `patch.ts`) |
| Leaf set of `undefined` | `{ path, exists: true }` | As `applyPatch`: an object member disappears from the JSON, an array element serializes as `null` |
| Leaf delete | `{ path, exists: false }` | As `applyPatch`: `splice` for a numeric key on an array, `delete` otherwise |
| Order | `{ op: "order", path, ids }` | Permutes the elements into the order of `ids` |
| Insert | `{ op: "insert", path, index, value }` | `splice(index, 0, value)`; `value` is an object with a string `id` |
| Delete by id | `{ op: "delete", path, id }` | Removes the element whose `id` is `id` |

Paths are canonical (`PM.Comps.canonicalize`, `pm_model::canonical`):
they address the canonical project form, where the open composition lives
in `comps[compId]` like every other composition, so a receiver never needs
to know which composition is open. Path parts are strings or non-negative
integers (`isPatchList`).

**Id-keyed arrays.** An id-keyed operation requires the array at `path` to
be id-keyed: every element a plain object with a string `id`, no two alike.
`order` names every element exactly once; `insert` adds an id the array
lacks at an index no greater than its length; `delete` names an id it has.
Navigation follows own members of objects and canonical indices of arrays.
Anything else (a missing member on the path, a non-array target, an element
without an id, an unknown, repeated, or present id, an index past the end,
an `undefined` document) is a **mismatch**: `PatchErrorKind::Mismatch` in
Rust, `IdOpMismatch` in TypeScript. A mismatch means the receiver has
diverged from the renderer; it resyncs (invariant 4). As with `applyPatch`,
a failing operation leaves the earlier operations of its batch applied.

**Revisions.** On the revisioned `pm_model::Document`, `order` keeps every
element's node and revision and stamps the array only when the order
actually changed; `insert` creates one node at the batch revision; `delete`
drops one. The array and its ancestors are stamped whenever the array
changed; siblings keep their revisions. The resulting tree, revisions
included, is identical to the one id-keyed reconciliation builds from the
equivalent whole-array leaf patch (tested on 4,000 random document pairs
and every golden vector), so evaluation caches keyed on node revisions
behave the same whichever form arrives.

### The generator

`diffOps(before, after)` (`patch-ids.ts`) and `pm_model::ops::diff_ops`
are `diffPatches` with id-keyed arrays. They walk the two values as
`diffPatches` does, with two differences:

1. Where `diffPatches` replaces an array whole (its length or its `id`
   order changed) and both arrays are id-keyed and share at least one id,
   the generator emits, in this order: a `delete` for each old id the new
   array lacks, in old order; an `order` of the surviving ids when their
   relative order changed; an `insert` for each new id, in ascending final
   index; then, recursively, the operations that turn each survivor into
   its new value, addressed by its final index. Arrays that are not
   id-keyed on both sides, or share no id, get `diffPatches`' whole-array
   leaf set, so items without ids (or with duplicate ids) still patch
   correctly.
2. Object members are visited in UTF-16 code unit order (the canonical key
   order), not insertion order.

The output is therefore canonical: the same pair of values always yields
the same list, in TypeScript and in Rust (the vectors check that both
generators emit identical lists). Undo is `diffOps(after, before)`.
Applying the generated operations gives exactly the document `patch.ts`'s
leaf patches give (property-tested on 3,000 random pairs in TypeScript and
4,000 in Rust, forward and back).

Writers use `diffOps` where they used `diffPatches` for a structural edit,
and send leaf patches unchanged otherwise. `pm-wasm` may also convert a
whole-array leaf set it receives into operations with `diff_ops` against
its mirror before forwarding to engine sessions.

### Binary format, version 1

```text
body     = magic count op*              magic = 50 4D 50 01 ("PMP", version 1)
count    = u32                          number of operations
op       = 0x00 path                    leaf delete
         | 0x01 path json               leaf set
         | 0x02 path                    leaf set of undefined
         | 0x10 path u32 str*           order: id count, then the ids
         | 0x11 path u32 json           insert: index, then the value
         | 0x12 path str                delete by id
path     = u32 part*                    number of parts, then the parts
part     = 0x00 str                     string key
         | 0x01 u32                     integer index 0 to 2^32 - 1
         | 0x02 f64                     any other non-negative integer (-0, or 2^32 and above)
str      = u32 bytes                    WTF-8
json     = u32 bytes                    UTF-8 JSON text
```

- Integers are little-endian; `u32` lengths count bytes.
- **Strings** (keys and ids) are WTF-8: UTF-8, with unpaired surrogates
  (which JavaScript strings may hold) as three-byte sequences. A surrogate
  pair is always one four-byte sequence, so every string has exactly one
  byte form. Overlong sequences and code points past U+10FFFF are invalid.
- **Values** are JSON text. JSON text is always well-formed UTF-8 because
  `JSON.stringify` escapes lone surrogates. The renderer already produces
  JSON, the mirror's parser is the one tested against `JSON.parse`, and the
  length prefix lets a receiver skip or compare a value without scanning it.
- **Canonical form.** Encoders write values as the canonical serialization
  (`canonicalStringify`: `JSON.stringify` with keys sorted by UTF-16 code
  units) and choose the `0x01` part form whenever the index fits. Equal
  operation lists encode to equal bytes, and a canonical body re-encodes to
  itself (`is_canonical`, `isCanonicalPatchBody`).
- **Decoders** reject every structural deviation: a wrong magic or version,
  truncation, trailing bytes, an unknown tag, a count larger than the
  remaining bytes allow, a `0x02` part that fits `0x01`, a path number that
  is not a non-negative integer, invalid WTF-8, invalid UTF-8 or JSON in a
  value (including a byte order mark), and an inserted value that is not an
  object with a string `id`. They accept any valid JSON text, so a producer
  may forward JSON it already holds; such a body decodes but is not
  canonical.
- **Limits.** The envelope bounds a body to 8 MiB
  (`MAX_PATCH_BODY_BYTES`); values nest at most 512 levels (the parser's
  limit, which keeps recursion bounded under WebAssembly's 1 MiB stack);
  arrays grow to at most 16,777,216 elements.
- **Versions.** A body format change is a new magic version byte and a new
  `PatchEncoding` variant. Tags `0x03` to `0x0F` and `0x13` and above are
  reserved.

Not adopted: a frame carrying the history's own patch-object bytes
(`scopedPatches` in `legacy/core/history.ts` stringifies `{path, ...after}`
for its byte budget). S5 measured that path as passing, but with id-keyed
operations a structural edit no longer needs the whole array, and the
history's object shape is not a patch.

### Golden vectors and tests

- `engine/crates/pm-model/tests/vectors/patch-binary-vectors.json`,
  generated by running the real `patch-ids.ts`, `patch-binary.ts`, and
  `patch.ts` under bun
  (`bun engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors.ts`):
  422 diff cases (22 curated, 400 random), 20 apply cases with results
  and error kinds, and 29 invalid bodies.
- `cargo test -p pm-model --test patch_binary`: the Rust decoder reads the
  TypeScript bytes as the listed operations, the Rust encoder writes the
  same bytes, `diff_ops` emits the same operations, applying them gives
  the target document and the same revisioned tree as the leaf patches,
  apply cases match results and error kinds, invalid bodies fail, and
  truncations and random single-byte mutations never panic.
- `bun test src/shared/patch-ids.test.ts src/shared/patch-binary.test.ts`
  (also under vitest): the committed vectors equal a fresh generation, the
  TypeScript decoder agrees with every vector and rejects every invalid
  body, and the generator property tests.
- `cargo test -p pm-model --test ops`: revision preservation and the Rust
  property test.

### Measured (M4 Max, 2026-10-12, other lanes building, load 4 to 7)

An 884-layer composition (785,536 bytes of layer JSON, S5's shape).
Native timings are `cargo test --release -p pm-model --test bench_ops --
--ignored --nocapture`; TypeScript timings are bun 1.3.14.

| Edit | Body | Native decode + apply, p50 / p95 | Whole-array leaf set: body, decode + apply p50 |
|---|---|---|---|
| Reorder (move one layer) | 11,427 bytes (884 ids) | 0.090 / 0.114 ms | 785,581 bytes, 9.8 ms |
| Insert one layer | 907 bytes | 0.067 / 0.094 ms | 786,440 bytes, 9.8 ms |
| Delete one layer | 54 bytes | 0.032 / 0.053 ms | 784,702 bytes, 9.9 ms |
| Property edit | 78 bytes | 0.0004 / 0.0004 ms | - |

Renderer side, `diffOps` plus `encodePatchOps`: 0.17 to 0.20 ms p50
(p95 at most 0.36 ms) when unchanged layers are shared between `before`
and `after`, as they are when an edit rebuilds only the array. Against a
deep clone of `before` the walk compares every layer: 3.5 to 3.6 ms p50
(`diffPatches` plus stringifying its patches costs 8.6 to 9.5 ms on the
same inputs). Encoding a single leaf patch costs 0.5 µs. WebAssembly
timings for these bodies are not measured yet (S5 measured its own framing
at 0.08 to 0.2 ms for the same three edits in Electron).

**Consequence for writers:** the 2 ms structural budget holds when the
writer diffs only the edited scope, or diffs values that share unchanged
subtrees. A writer that deep-clones the whole layers array before an edit
and diffs the clone pays the walk (about 3.5 ms at 884 layers) before any
encoding; that is a cost of the snapshot, not of the format.

### Time at the boundary (`v1_ingest`)

Section 5's rules with section 26's amendment 1, as implemented in
`pm_model::ingest`:

1. `x = t × fps_f64`, `n = round(x)`. If `|x − n| < 1e-9 × max(1, |n|)`
   the time is frame-aligned and ingests as `frame_start(n)` at the exact
   rate. JavaScript's float error for a frame-aligned time is a few ulps
   (about `1e-15 × n`), so every such time is captured to 10⁸ frames, and
   off-grid times keep their offset (with section 5's `1e-6` they were
   pulled onto frames beyond 500,000 frames).
2. Otherwise the time ingests as the nearest tick.
3. Writing back: an unchanged time echoes its original float. A time the
   engine created or moved is written by `IngestRate::seconds_of`: on a
   frame boundary `n / fps_f64`, which re-ingests to the same frame at
   every `n` (property-tested to ±10⁸ at all 16 supported rates and the
   float NTSC rates, and at n = 10⁶); off the grid `ticks / TICKS`. For a
   frame start, `ticks / TICKS` does not round-trip in a document that
   stores a float NTSC rate (frame 10⁶ at 29.97 re-ingests as 999,999),
   which is why frame starts use `n / fps_f64`.

Patch values that are times travel as the document's float seconds; a
receiver ingests them with `classify_time_path` and the composition's
`IngestRate`.

### Canonical hash

`pm_model::hash::canonical_hash` is SHA-256 of the canonical serialization
of the canonical project form, as 64 lowercase hex digits, and equals
`canonicalProjectHash(PM.proj)` in `shared/canonical-json.ts` (vectors in
`tests/vectors/canonical-vectors.json`). The production consistency check
(invariant 5) compares the two.

### The identity registry (Phase 1, `pm-wasm`)

Specified by the renderer write path (`docs/engine/write-path.md`,
`apps/desktop/src/renderer/src/engine-doc/native.ts`): a live layer is
addressed by `PM.curComp()`'s id and its index in that composition (checked
against the mirror's id at that index; a mismatch repairs the mirror before
answering); a layer outside the composition is sent as JSON and evaluated
ad hoc; a live channel object maps to its member path inside its layer and an
interned id `(composition, layer id, key, Member(path))`, revalidated on
every use and released through a `FinalizationRegistry`. Answers that are
document objects come back as paths or indexes and resolve on the live
objects.

### The renderer's generator

The renderer diffs its shadow (the canonical document as last sent) against
the live document with a generator that reads the live side as
`JSON.stringify` does (`engine-doc/json-diff.ts`): live documents hold
non-enumerable accessors, `undefined` members, `NaN`, and `-0`, where
`diffOps`' `Object.hasOwn` would keep members JSON does not have. On plain
JSON it emits exactly `diffOps`' operations (tested on 2,000 random pairs),
so bodies stay canonical. Open question 5 is resolved this way: structural
edits leave the renderer as id-keyed operations; `pm-wasm` does not convert
whole-array leaf sets.

### `fpsRational` is a composition field

The frame-rate validator stores `fpsRational` beside `fps` (section 5, rule
1). It belongs to the composition: `COMP_FIELDS` (compositions.ts,
`CANONICAL_COMP_FIELDS`, `pm_model::canonical::COMP_FIELDS`) include it after
`fps`, optional like `audioGain` and `render3d`, so a patch to the open
composition's `fpsRational` addresses `comps[compId].fpsRational` and the
value moves with its composition.

## Versioning

Version 0 until Phase 1 starts. The envelope and the body format version
separately: a new `pm-model` patch format is a new `PatchEncoding` variant
(and a contract bump), so peers negotiate it like any other change.
`DocumentEncoding::CanonicalJsonV1` stays readable for resyncs from the
renderer, which produces JSON natively. `DocumentEncoding::CanonicalBinaryV1`
has no specification yet; until it has one, resyncs use
`CanonicalJsonV1` (one leaf set of the whole document at the empty path is
also a valid `canonicalV1` body).

## What Phase 0 must confirm

- **S5 (WASM evaluator):** patch apply under 0.2 ms for a property change
  and 2 ms for a structural change on the 884-layer project, with the batch
  framing here (one `PatchBatch` per renderer flush). If S5 shows that the
  per-batch overhead matters, batches coalesce per frame. *Result: pass,
  on the condition that structural edits arrive as id-keyed operations or
  bytes the renderer already has, never as a stringified whole array
  (S5: the stringify alone costs 2.1 ms). The body format above encodes
  that condition; native decode and apply of the three structural edits
  measure 0.03 to 0.09 ms p50.*
- **The writer inventory** (`scripts/engine-writers.mjs`): that every writer
  class maps to a `PatchOrigin`.
- **The shadow-replay auditor:** that the stream of batches, applied to a JS
  shadow, equals `PM.Comps.canonicalize(PM.proj)` at frame boundaries. This
  is a Phase 1 gate, but its harness is built against this envelope. The
  shadow applies operations with `applyOps` (`shared/patch-ids.ts`), which
  keeps `applyPatch`'s semantics for leaf patches.

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
4. Resolved (2026-10-12): the body format is `canonicalV1` above; the
   `leafJsonV1` fallback is not needed.
5. Whether `pm-wasm` should convert whole-array leaf sets that still arrive
   (from writers not yet moved to `diffOps`) into id-keyed operations
   before forwarding them to engine sessions, or forward them as they are.
   Converting costs a diff against the mirror; forwarding costs every
   engine session a whole-array decode (about 10 ms native at 884 layers).
