# RFC 09: Protocol codec and code generation

Status: draft, version 0. Modules: `pm_protocol::envelope`, `codec`,
`version`, `common`, `validate`, `ts`, and the `pm-protocol-gen` binary.
Gates Phase 1; freezes when Phase 1 starts. Owner: Engine and platform
(`pm-protocol`, section 27).

## Purpose

One transport-agnostic protocol for every engine connection (plan
section 4, principle 8; section 19, "Protocol"): MessagePack messages over
MessagePorts and pipes, TypeScript types generated from the Rust types and
checked in CI, version negotiation, and bulk data through shared surfaces or
cache files rather than inline.

## Types

**Envelope** (`envelope`):

| Type | Meaning |
|---|---|
| `Envelope` | `wire` (the envelope layout version), optional `session`, optional `revision`, `message` |
| `Message` | `hello`, `welcome`, `request {id, body}`, `response {id, body}`, `error {id, error}`, `event {body}`, `cancel {id}`; tagged by `kind` |
| `Request`, `Response`, `Event` | Every message body, tagged by `type`. Each request variant documents its direction and the response that answers it (`Request::expects`); each request and event names its owning contract (`contract()`) and whether it must carry a session (`session_bound()`) |
| `Outstanding`, `CorrelationError` | One direction's outstanding requests: allocates ids and checks that each answer has the right kind |
| `SessionRole`, `OpenSession`, `EvaluateRequest`, `RenderedImage` | Session lifecycle and the two requests without a better home |

**Versions** (`version`): `ContractId` (the fifteen contracts, with number
and RFC path), `ContractRange`, `ContractVersion`, `PeerRole`, `Hello`,
`Welcome`, `negotiate`, `accept`, `complete`, `Negotiated`,
`NegotiationError`, `WelcomeError`, `WIRE_VERSION`, `MIN_MESSAGE_LIMIT`.

**Primitives** (`common`): safe-integer newtypes (`RequestId`, `Revision`,
`SessionId`, `Seq`, `ByteSize`, `Count`, `HostMicros`), `Ticks` (signed,
1/705,600,000 s), `TickRange`, `FrameRate` and `PlaybackRate` (exact
rationals), string ids (`DocumentId`, `CompositionId`, `LayerId`, `AssetId`,
`DefinitionId`, `EffectInstanceId`, `JobId`, `AgentRunId`, `ProjectId`,
`ExtensionId`), `Size`, `PixelRect`, `Rect`, `Mat3`, `Mat4`, `ContentHash`
(32 bytes), `Bytes`, `Rgba`.

**Codec** (`codec`): `encode`, `decode`, `encode_unvalidated`,
`decode_unvalidated`, `CodecError`, `MAX_MESSAGE_BYTES` (16 MiB),
`MAX_DEPTH` (64).

**Generation** (`ts`, `samples`, `pm-protocol-gen`): TypeScript generation
from `Envelope` and everything it references, drift detection, golden
fixtures.

## The wire format

- Each message is one MessagePack value: a map keyed by camelCase field
  names (`rmp_serde::to_vec_named`). Enums are internally tagged (`kind`,
  `type`, `op`, `status`, `signal`, `command`) except unit-only enums,
  which are camelCase strings. `Option::None` is `nil` (`null` in
  TypeScript). Byte fields (`Bytes`, `ContentHash`) are MessagePack `bin`
  (`Uint8Array`). `Compat` is the number 1 or 2.
- **Every integer is a safe JavaScript number** (|v| ≤ 2⁵³ − 1). The 64-bit
  newtypes reject anything outside on construction, serialization, and
  deserialization, so no TypeScript reader needs BigInt. Ticks in the safe
  range cover about ±147 days; document key times are not limited by this
  because they travel inside patch bodies as the document's float seconds
  (contract 5).
- **Every float is finite.** NaN and infinities have no JSON form (agent
  tools serialize these messages as JSON), `serde_json::Value` silently turns
  them into `null`, and range checks would each have to special-case them.
  `codec::check_finite` scans the encoded bytes for MessagePack float32 and
  float64 values, so the rule covers every float field without a per-field
  check: `encode` fails at the sender and `decode` rejects with
  `CodecError::NonFinite`. A producer whose computation can yield NaN
  (expression evaluation, RFC 01) maps it before encoding.
- **Revision tagging.** On a request, `revision` is the minimum document
  revision the receiver must have applied before answering
  (read-your-writes); the receiver waits for patches, then fails with
  `StaleRevision`. On responses and events it is the revision the answer
  was computed at.
- **Request ids** are allocated per direction per connection, so they are
  unique only in that direction. Every request is answered by exactly one
  `response` of the kind `Request::expects` names, or one `error` with the
  same id. `cancel {id}` asks the receiver to stop; it answers with an
  error coded `Cancelled` unless it already answered.
- **Bulk data never travels inline.** Frames and surfaces use shared
  surfaces (contracts 2 and 3); large payloads (resync documents, images,
  Blender snapshots and surfaces) use cache files behind capability handles
  (contract 14). `MAX_MESSAGE_BYTES` bounds every message; a peer may
  announce a smaller limit, not below 64 KiB.

## Version negotiation

Every connection starts with `hello` from the side that opened it and
`welcome` from the other side; both carry the peer role, `wire`, a source
build id (git commit plus dirty flag), and the largest message the sender
accepts.

1. `wire` must match exactly.
2. Each contract negotiates to the highest version in both ranges
   (`negotiate`). A contract only one side lists is dropped unless that side
   marks it required; a required contract with no common version fails.
3. **Drafts (version 0) interoperate only between equal builds**, because a
   draft may change without a bump.
4. The opener checks the `welcome` against what it offered (`complete`): no
   version outside its ranges, no contract it never offered, every required
   contract present, no duplicate, an acceptable message limit.
5. After the handshake, `Negotiated::permits` rejects messages whose
   contract was not negotiated.

Failures convert to `EngineError` with `UnsupportedVersion`, connection
scope, and the versions in `ErrorDetail::UnsupportedVersion`.

## Invariants

1. **A message that decodes obeys its contract.** `decode` checks size,
   nesting depth (64), trailing bytes, and finite floats, then runs `Validate` on the whole
   envelope; `encode` validates first, so invalid messages fail at the
   sender. Hello and welcome carry no session or revision; session-bound
   requests and events must carry a session.
2. **The decoder is an untrusted-input surface** (section 19, security).
   Tests decode 2,048 random byte strings and 2,048 single-byte mutations
   of the golden fixtures per run without a panic, and every truncation of
   every fixture fails cleanly. The Platform lane adds a `cargo-fuzz` target
   over `decode` (open question 2).
3. **Every message kind has a sample** (`samples::all`). The exhaustive
   index functions fail the build when a variant is added without one, and
   a test checks that every request kind has a sample of the response kind
   that answers it.
4. **Regeneration is a no-op.** `pm-protocol-gen` writes the TypeScript
   types to `apps/desktop/src/shared/engine-protocol/` and one MessagePack
   fixture per sample to `engine/crates/pm-protocol/tests/fixtures/wire/`.
   `cargo test -p pm-protocol` fails when regenerating would change either
   (missing, changed, or stale files), when a generated type exposes
   `bigint`, or when a generated field is snake_case.
5. **TypeScript reads the same bytes.** `wire.test.ts` (vitest, hand-written
   alongside the generated files) decodes every fixture with a strict
   MessagePack reader (safe integers and finite floats only) and reads them
   through the generated types, checking field names, nulls, numbers, and
   `Uint8Array`s.
6. **`pm-protocol` builds for `wasm32-unknown-unknown`** (for `pm-wasm`),
   with or without the `ts` feature.

## Versioning

`WIRE_VERSION` is 1 and changes only if the envelope layout itself changes;
peers with different wire versions exchange nothing but the rejecting
error. Each contract versions independently through negotiation. This
contract's own version covers the envelope, the codec rules above, and the
primitives in `common`.

## What Phase 0 must confirm

- **S1:** the JavaScript cost of decoding one frame descriptor in main under
  load. The Rust cost is measured in RFC 02 (243 bytes, 1.2 µs decode
  without overlay, M4 Max). If main's decode is too slow, frame events can
  move to a fixed binary layout without changing the types.
- **S5:** that `pm-wasm` encodes canonical patch batches and decodes
  responses within its budget. The crate already builds for wasm32; S5
  measures it.
- **Transport:** that Electron's `MessagePortMain` carries `Uint8Array`
  payloads without copying more than once per hop, and that utility
  processes receive transferables. Measured with S1.

## Open questions

1. Decode cost: internally tagged enums buffer through serde's `Content`,
   which is why decoding costs about four times encoding. If S1 or S5 shows
   it matters, the hot messages (frame events, patch batches) switch to
   adjacently tagged or positional encodings in one version bump.
2. A `cargo-fuzz` target over `decode` and `PatchBatch` ingest belongs in
   the fuzzing infrastructure the Platform lane sets up; it needs a nightly
   toolchain alongside the pinned one.
3. Whether the TypeScript side gets a generated runtime validator (today it
   gets types; the engine validates everything it receives, and main
   validates what it forwards with these same Rust rules through
   `pm-bridge`).
4. Whether doc comments in the generated TypeScript should drop the Rust
   intra-doc link brackets (`[`WIRE_VERSION`]`). Cosmetic.
