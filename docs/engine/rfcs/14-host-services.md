# RFC 14: Host services

Status: draft, version 0. Module: `pm_protocol::host` (with the lease
messages in `lease`, the Blender messages in `blender`, and the definition
set in `definitions`). Gates Phase 2; freezes when Phase 2 starts. Owner:
Engine and platform (host services in main). Implementations: Electron main
(TypeScript, over the protocol messages) and the CLI host (`pm-host`,
through the `HostServices` trait), used by `pm-cli` and, from Phase 3c, the
hidden MCP host.

## Purpose

Engines never touch the outside world directly. Arbitration (memory leases,
hardware codec sessions, GPU slicing policy), capability handles, output
destinations, Blender supervision, and the definition-snapshot source come
from one interface implemented by every host (plan section 4, principle 6;
section 19). This is what lets the same engine run under Electron main, the
CLI, and the hidden MCP host, and what lets every host meet the agent export
contract that only the desktop meets today (section 17; appendix A item 37).

## Types

| Type | Meaning |
|---|---|
| `CapabilityHandle` | An unguessable 128-bit capability written as 32 lowercase hex digits, minted only by the host from a cryptographic RNG. The decoder rejects anything else, including paths |
| `CapabilityKind` | What a handle grants: `mediaRead {asset}` (original, linked file, or media-store copy), `cache {cache}` (one cache area), `outputWrite {job}` (one job's temporary output), `projectStore` (document host persistence), `extensionBundle` (document host extension runtime) |
| `CacheKind` | Media index, thumbnails, generated media, render cache, Blender surfaces, shader cache, font subsets, large-still mips, scratch (section 14.7) |
| `MintHandle`, `HandleGrant` | `Request::MintHandle` (engine → host): a kind for a session; the grant carries the handle and, for media, the content hash the host verified |
| `PortId`, `HandleTable`, `HandleRejection` | The host's binding table: handle → (kind, session, port); check, revoke per session |
| `ResolveHandle`, `HandleResolved`, `HandleAccess` | `Request::ResolveHandle` (engine → host, on the engine's host port only): the host checks the binding against the session and the port the request arrived on, and answers with the absolute path the handle names and the access its kind grants (read for media, project store, and bundles; read-write for caches; write for one job's temporary output) |
| `OutputChoice` | `agentAutomatic {suggestedName, extension}` (MCP and agent tools: the host picks a unique name, no dialog) or `userChosen {destination}` (export sheet or CLI, already a handle) |
| `OverwritePolicy` | Unique name, replace after verification, fail |
| `OutputDestinationRequest`, `OutputDestination` | `Request::OutputDestination` (job engine → host): the host answers with a handle to a temporary file in the destination folder |
| `CommitOutput`, `OutputVerification`, `OutputCommitted` | `Request::CommitOutput` (job engine → host): move the verified output into place, or remove it; the host answers with the real path |
| `HardwareSessionKind`, `HardwareSessionRequest`, `HardwareSessionGrant` | `Request::HardwareSessions` (engine → host): acquire or release decode, encode, ProRes decode, and ProRes encode sessions; the grant says how many the requester now holds |
| `HostServices` (trait) | The operations as a Rust interface: mint and check handles, revoke a session, output destination and commit, leases, hardware sessions, Blender status and jobs, definition snapshots |

Also provided by host services, specified in their own contracts: leases
(contract 8, `Request::GrantLease`, `Request::RequestLease`), the slicing
policy and deadline relay (contract 13), Blender supervision (contract 11,
`Request::BlenderRender`, `BlenderStatus`, `BlenderModel`), the
definition-snapshot source (contract 10), and engine crash reports
(contract 7, `Event::Crash`).

## Invariants

1. **The engine accepts no raw paths** (section 19, security). Every file
   it reads or writes is named by a capability handle. `CapabilityHandle`
   only decodes from 32 lowercase hex digits, so a path in a handle field
   fails at the decoder (tested).
2. **Handles are bound to one session and one port** and rejected anywhere
   else (`HandleTable::check`, tested: unknown, wrong session, wrong port).
   The host checks every handle an engine presents and never trusts the
   engine's word about what a handle grants. Closing a session or
   restarting an engine revokes its handles. `PortId` names one engine
   launch's host connection, so a restarted engine starts with no handles.
   **Paths reach an engine only as `HandleResolved`**, only on its host
   port, and only for a handle bound to that port: renderers and the peer
   engine pass handles and cannot answer a `resolveHandle` they were never
   sent (the engine correlates answers per connection). `HandleResolved`
   validates that the path is absolute, at most 4,096 bytes, without NUL,
   and that the access equals the kind's (tested).
3. **Only the host mints**, from a cryptographic RNG. A duplicate handle
   means the RNG is broken; `HandleTable::bind` refuses it and the host must
   not use it.
4. **Agents never choose output paths** (section 3). An agent output uses
   `agentAutomatic` with the unique-name policy; its suggested name and
   extension are bare file-name parts (no separators, no `.` or `..`, at
   most 255 bytes; the extension alphanumeric), checked by
   `OutputDestinationRequest::validate` (tested with `../../etc/passwd`).
   The host creates the file owner-only and exclusively
   (`open(wx, 0600)`), with no dialog.
5. **Atomic output** (section 17): the engine writes a temporary file in the
   destination folder, verifies it, and asks the host to commit it. The host
   moves it into place and returns the real path, which reaches the agent
   only after encoding and verification. An empty or unverified file is
   removed (`commit: false`). `pm-cli` and `powermove render` accept any
   path because the CLI user chose it (`userChosen`).
6. **Agent media paths are validated by the host** (section 19): for
   `import_media`, `replace_media`, and `import_media_to_timeline`, a
   regular file whose type the importer's probe accepts, with no separate
   extension list; relative paths resolve against the host's working
   directory where accepted today. Agents are not restricted in network or
   reads; only Store extensions are contained.
7. **Hardware sessions** are arbitrated across both engines by main: a
   request changes the count (non-zero delta); the grant is the requester's
   new total (section 13, decode service).
8. **Engine ports stay private.** Main creates a `MessageChannelMain` per
   window and engine; engine ports are never given to extension sandbox
   frames (section 19).
9. **Every host behaves the same.** Electron main and the CLI host implement
   the same operations with the same rules; contract tests run both against
   the same message sequences (the Phase 3c gate: byte-identical documents
   on the desktop and hidden hosts).

## Implementation (Phase 2 infrastructure)

Electron main: `HostServicesCore` (`apps/desktop/src/main/engine/services.ts`)
behind each engine launch's host connection. CLI: `pm_host::CliHost`, which
implements `HostServices` through `pm_host::Bound` (the trait with the
connection's process, port, and envelope session). The shared sequence
`engine/crates/pm-host/tests/fixtures/host-contract.json` runs against both
(invariant 9). Output handles without an envelope session bind to session
0. Notes: `docs/engine/host-processes.md`.

Phase 3b (delivery): `OutputDestinationRequest.folder` (optional, default
false) asks for an owner-only temporary folder instead of a file, for PNG
sequences. The engine writes frames into a work folder beside it and
renames it over the temporary folder; `commitOutput` moves the folder into
place (unique names keep both; replace merges frames into the existing
folder) and reports the sum of the frames' bytes. `pm_host` and main's
`outputs.ts` implement it; the shared host contract sequence is unchanged.

## Versioning

`ResolveHandle` and `HandleResolved` were added in the draft (Phase 2
infrastructure): file access by handle needs a way for the engine to open
the file, and an Electron utility process cannot receive a file descriptor
over a `MessagePort`. If a sandbox profile is adopted later and descriptors
travel over the Mach rendezvous instead, the request keeps its shape and the
answer gains a descriptor variant (a version bump).

Version 0 until Phase 2 starts. New capability kinds, cache kinds, or
hardware session kinds are new variants and a version bump. The
`HostServices` trait is Rust-only; changing a method signature is a bump
because main's TypeScript implementation must follow.

## What Phase 0 must confirm

- **Phase 0 macOS sandbox evaluation for engine processes** (section 19):
  engine processes provide crash isolation and least authority, not a
  security sandbox. If a sandbox profile is adopted, file access by handle
  must work inside it (the host passes file descriptors or the engine opens
  handle-resolved paths within the profile); this contract does not change
  either way.
- **S1:** the per-launch rendezvous (a Mach receive right under a random
  bootstrap name, audit-token and code-signature checks, one check-in per
  engine launch) that establishes the port a handle is bound to. `PortId`
  is the host's name for that port.
- **S6:** the hardware decode and encode session limits on the reference
  machine (and on a base M1 without a ProRes engine, when that gate runs),
  which set what main grants.
- **Export and agents lane:** that the CLI host can meet the agent export
  contract (unique names, owner-only exclusive files, the real path after
  verification) on the hidden host and in serve, fixing appendix A item 37.

## Open questions

1. Whether handles expire on their own (a time-to-live) or only with their
   session. Current answer: only with the session; jobs that outlive a
   window run in job snapshot sessions with their own handles.
2. Whether a media-read grant should name the resolved representation
   (original or proxy) so the host can audit that exports never read proxy
   bytes. The engine enforces this today through cache keys and
   `FrameRequest` validation (contract 2); a host-side audit would be
   defense in depth.
3. Whether `OutputVerification` should carry the full verification report
   (codec, profile, pixel format, depth, tags, range, frame count,
   duration, audio samples, alpha, sampled pixels, section 17) instead of
   the summary. The engine verifies; the host only needs enough to log and
   to decide the commit. Decided with Phase 2 export.
4. Whether the definition-snapshot source should push to job engines
   directly (host → engine) or always through the renderer's session.
   Section 11 says the kernel pushes to every session; jobs and `pm-cli` use
   `HostServices::definition_snapshots`.

## GPU WebCodecs export frames (EXPORT-1, withdrawn)

`hostEncode` and its export bridge were removed with X-R3: the engine encodes
WebM itself (export.md, "Encoders").
