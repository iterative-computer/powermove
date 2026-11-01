# Store view host (package 10)

`apps/native/crates/store-host` is `pm-store-host`. It implements the policy,
archive admission, install publication, permissions, channel admission and view
recovery that can run without WebKit. It has no Node, Chromium, V8, UI, kernel or
engine dependency. **This is not a macOS WebKit adapter, Store distribution
service, sandboxed helper launcher, or proof of v1–3 corpus compatibility.**

Read with [compat-host.md](compat-host.md), [runtime-v8.md](runtime-v8.md),
[s5-compat.md](s5-compat.md), [kernel.md](kernel.md), [first-launch.md](first-launch.md),
[D-STATE](d-state.md) and [C-MATRIX](c-matrix.md). Orchestrator decision 5 in
[p0-decisions.md](p0-decisions.md) wins over the original WebKit-owned runtime.

## Placement and identities

Store logic/PM/services/canvas execute in a confined V8 helper. Only the bundled
remote-DOM applier executes in WebKit; extension scripts never enter the page.
`ViewPlatform::enqueue` is the nonblocking boundary for a main-thread WebKit
adapter. Worker-side archive/file/kernel methods must never run on main. They
must be wrapped in the kernel actor's tickets/commands, not a UI blocking wait.
No live JS objects or callbacks cross this crate's JSON channel.

`ExtensionId` validates the shipped registry grammar, `[a-z0-9][a-z0-9-]{1,63}`,
including on deserialization. `DataStoreIdentity` hashes a domain and
length-delimited immutable native profile namespace + exact extension ID using
SHA-256. Keep the profile namespace through moves, upgrades and relaunches. A
release/panel/PID change never changes the key. Beta and stable use different
profile namespaces. Kernel JSON storage remains `ext.<id>`, including unknown
keys and own `__proto__` keys; the renderer is not its authority.

An app-owned registry retains a separate WKWebsiteDataStore object for each full
key. Reuse it across views, detached panels, crashes and helper restarts. Never
use WebKit's default store or share a Store page with a trusted page. The
`webkit_uuid` projection is for an OS identifier API where available; detect
UUID/full-key collisions and fail closed rather than share stores. The current
compat host uses separate nonpersistent stores; durable extension storage stays
in the kernel. This lane does not claim durable WebKit cookies/localStorage or
require an unavailable persistent-store API on an older supported OS. Rebuild
renderer state from the realm/kernel, with an opaque document origin.

Uninstall marks the data identity **Archived**, archives the active code and
retains decisions, source overlays and every version. Reinstall reactivates the
same identity. Closing the last view does not remove its store. No data-delete,
clear-storage or archive-GC operation exists here.

## v1.2.0 migration reader design (not implemented here)

Source evidence is tag `v1.2.0`, specifically
`src/shared/sandbox-origin.ts`, `src/main/security-policy.ts`,
`src/renderer/src/kernel/sandbox-host.ts`, `sandbox-view.ts`, and
`src/main/cloud/install.ts` under `apps/desktop`.

The shipped Store sandbox uses `app://x-<slug>-<26 base32 chars>` inside the
**shared Chromium profile**, not a separately named `persist:<id>` Electron
partition. The suffix is the leading 130 bits of SHA-256(exact ID); slug runs
collapse to hyphens and truncate to 20 characters. `LegacyImportPlan::v1_2_0`
reproduces that address for selecting archived records, verified against public
shipped test vectors. This is storage identity, not a Chromium rendering quirk.
The panel iframe is sandboxed with an opaque origin. Ordinary API storage was
seeded from kernel/file-store `ext.<id>`; do not assume a persistent per-view
browser database existed.

The first-launch reader must:

1. Wait for Electron to quit or obtain the existing consistent read-only profile
   snapshot, including LevelDB WAL/manifests, IndexedDB external blobs and unknown
   records. Reuse `pm-first-launch`/`pm-native-migrate` snapshot and direct readers;
   never open a live Chromium database with a second writer or start Electron.
2. Select exact old app origins from the installed manifest IDs, not by slug
   prefix. Include any separately discovered Electron `Partitions/*` directories
   as archived inputs; inventory them instead of inventing a partition name.
   Preserve unknown/unassignable origins, session files, caches and encrypted
   cookies as opaque private checkpoint data. No credentials enter logs/repo.
3. Import `store/ext.<id>.json` through the kernel's existing logical reader,
   retaining object keys, value shapes and gzip envelopes from later profiles.
   Preserve local source edits, installed provenance/enablement, `.updates`,
   `.trash`, values, and unrecognised files. Native beta already has read-only
   shared extension sources plus an editable overlay; don't overwrite it.
4. Associate checkpoint/source hashes, origin/key mappings and a migration
   version with the full data-store identity in the kernel transaction. Preserve
   UTF-16 strings and IndexedDB/blob types through the migrate adapters. Do not
   copy Chromium files into WebKit directories; formats and origin models differ.
5. Stage import receipts and publish only after all referenced bytes are durable.
   Resume from the receipt on interruption. Perform delta/three-way merge with
   tombstones on subsequent launches; never overwrite native edits or resurrect
   deleted object keys. Keep both sides on conflict. Reverse migration must
   include current acknowledged native values, not only the original checkpoint.

Opaque renderer storage stays unavailable. A discovered authored dependency on
legacy browser storage must get a realm-side compatibility adapter backed by the
preserved records; it cannot be silently reset or called unsupported. This lane
provides the reader's data/identity plan only. Existing unsigned installations
must enter via that migration/provenance path, without rewriting their source or
pretending a new publisher signature existed.

## Signed release admission and install transactions

A native remote release carries `SignedRelease` with protocol format 1,
publisher, key ID, extension/release IDs, archive SHA-256 and the complete sorted
path → `{sha256,bytes}` inventory. Ed25519 `verify_strict` checks a host-pinned
publisher/key pair over `powermove.store-release.v1\0` followed by deterministic
serde struct JSON (field order is the declared struct order; inventory keys sort
lexicographically). This is a new explicitly versioned wire format, not a claim
that v1.2.0's registry already signs releases. The distribution/integration lane
must provision/rotate pinned public keys and sign these exact bytes. Public
keys are configuration; extension-supplied keys cannot establish trust.

Only `verify` constructs `VerifiedRelease`. It validates signature, archive
hash, gzip checksum through EOF, complete file inventory, individual sizes and
hashes, manifest identity/API version/permission names, and bounds. Defaults:
64 MiB compressed, 128 MiB expanded file bytes, 4,096 files, 1 MiB manifest,
1,024-byte UTF-8 paths, plus a bounded tar-header allowance. Reject traversal,
absolute/backslash/colon/control/dot-segment paths, duplicates, links, device
nodes and nonregular payload entries. Preserve unknown manifest contributions
and fields. API versions 1–4 are admitted. Full compiler/registry manifest
semantic validation still belongs to its adapter before activation.

Files remain a virtual exact-path map; only the compressed archive is stored on
disk. Do not unpack with a second reader or approximate case/Unicode names on a
case-insensitive filesystem. Compilation must consume verified virtual files.
Large-policy exceptions must be explicitly configured and verified; no legacy
installation is removed because a new remote-release quota would reject it.

`FileArtifacts` durably writes an app-private `.staging` object, atomically
hard-links it into `objects/<sha256>.tar.gz`, fsyncs directories and never
replaces an existing hash object. Existing content is rehashed before reuse.
Staging hard links remain as recovery archives, including failed candidates;
there is no cleanup/GC in this package. Root directories must be app-owned and
not writable by extension helpers. Source overlays/user data never live in a
candidate extraction directory. Read objects with a byte limit and hash check;
activation/compilation must reverify the saved signed inventory before use.

`install` requires the expected kernel revision, preserves bytes first, then
CAS-publishes the entire extension record. Replaced code becomes an archive in
the same transaction. Identity and pinned publisher cannot change through an
update. Existing local edits cause a **ConflictPreserved** candidate beside the
active version; they are never overwritten. An explicit archive selection can
resolve that conflict. A failed artifact write or CAS keeps the active version.
An interrupted write is an unreferenced preserved candidate; an interrupted
acknowledgment is resolved by querying the kernel revision/active digest before
retrying. No blind repeated publication is required.

`rollback` selects an archive ID and archives the replaced code. It never rolls
back kernel project edits or extension JSON values. Old code must adapt to
current data or surface a recoverable migration issue; it cannot erase new
values. All archive choices and permission/install decisions are inspectable
serializable data. No filesystem rename ever makes incomplete code active.

`StoreKernel` is the required durable single-writer integration seam:
`load` plus whole-record `compare_exchange`. An error must mean nothing was
published. The kernel adapter must revoke old realm/view capabilities and
registrations on install/uninstall/rollback before admitting replacement traffic.
No production kernel writer is implemented here. `MemoryKernel` is a Linux
reference double; tests additionally exercise a durable file-backed double
across reopen. This does not certify the production kernel journal integration.

## Permission prompts and CSP

The permission names match the shipped registry: `network`, `clipboard`,
`assets`, `project:write`, `full-access`. Undeclared permissions fail without a
prompt. Requests return Granted/Denied/a serializable prompt. Prompts bind the
extension, signed bundle, publisher, capability and current kernel revision.
Only a native UI/authorized agent decision may call `respond`; pages cannot
approve themselves. A grant is returned only after durable CAS. Forged, replayed
and update-stale responses fail. Denials are retained without repeated prompts.

Existing decisions apply only to the same pinned publisher and currently
requested permission. A newly added permission requires consent. Removed
permissions become inactive; the old decision remains archived. `revoke`
persists denial. The adapter must cancel in-flight network requests and rebuild
network-enabled pages on revocation. `full-access` consent explicitly returns
**RequiresTrustedRealm**; it never grants all isolated Store operations. Route
approved trusted code through the existing trusted v1–3 contract, preserving
all legacy capabilities. Never silently downgrade it to the Store allowlist.

`MethodPolicy` is supplied by the host, never by a manifest/page. Every method
must be registered, including reads; multi-capability calls require all grants.
The concrete kernel adapter must additionally enforce resource/project identity,
own registrations/commands, network origins/redirects/credentials/byte limits,
and native clipboard focus plus recent gesture/rate checks. This crate's
permission membership alone does not certify those operation implementations.

`renderer_csp` derives policy from the installed manifest and persisted grants.
A host CSPRNG supplies a fresh hex nonce (32–128 chars); only that bundled
applier executes. Network consent permits passive HTTPS style/font/image/media
resources. V8 owns fetch/WebSocket/wasm; page connect-src is only its bound
`pm-store://<full-key>` bridge. Workers/iframes/objects/base/form submission are
denied. Navigation, popups, upload/download, browser media permissions and JS
modal dialogs must be denied by the platform delegate as well. The adapter must
check opaque `location.origin` before declaring Ready; a message's origin field
is never transport authentication. CSP is not a replacement for native guards.

## Message schema and recovery

`Envelope` carries protocol version 1, extension/view/generation, per-source
sequence, from/to and tagged `Message`. Host-bound endpoints authenticate the
source and capability before `Channel::admit`; never trust the JSON sender.
Allowed routes: view → realm Ready/Input/PaintAck/Failure; realm → view
DomBatch/Snapshot; realm → kernel ServiceCall; kernel → realm
ServiceReply/KernelEvent. A view cannot directly issue kernel operations.
DOM payload semantics belong to the full remote-DOM bridge, not a fixture
allowlist. Node/callback identity stays in V8; native pixels cross as IDs.

Admission rejects stale identities/generations, unknown schema/version,
sequence gaps and invalid routes. Limits default to 1 MiB/message,
256 queued messages and 8 MiB total. Backpressure consumes neither sequence nor
older work. Consumers complete the front entry only after acceptance. Duplicate
receipts distinguish queued from completed requests; they never redispatch an
edit. Service request IDs must also map to kernel realm/generation/txids for
lost-ack replay across connections. The transport owns response replay.

`View` uses a host monotonic clock and explicit states. Start → Ready requests a
fresh authoritative DOM snapshot. Crash/hang/ready timeout → **Seal** kernel
acknowledged gestures and revoke view callbacks → **Stop** → confirmed process
exit/detachment → delayed restart with a fresh kernel generation/capability →
Ready + snapshot. The V8 realm continues after a renderer-only crash. No
project mutation is replayed from an optimistic DOM snapshot. Store hang evidence
must be attributed JS execution time, not an unfocused/background heartbeat.

Defaults: 20 s startup deadline, at most three automatic restarts within 60 s,
250 ms exponential delay capped at 8 s. A fourth failure disables the view with
an inspectable reason; native UI/authorized agents can explicitly restart it.
User Stop disables without automatic restart. Failed sealing never authorizes a
kill. Failed platform enqueue is returned; retain/retry the owned Action.
Stop completion requires actual old PID exit (or complete crash detachment),
not merely an SPI return. `close` cancels pending restart; the adapter revokes
that view and keeps its store and realm data. Recovery has no delete/reset-data
step. Main-thread timers/JS waits do not drive export or native editing.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| V8 owns logic; rendering-only WebKit behind a queued trait | N8; adopted S5c architecture, N4/N6 synchronous contracts |
| Stable profile/extension identity; archive on replace/uninstall; exact opaque migration records | N4 no user data loss; N3 authored intent; prune rule |
| Read shared Chromium origins instead of inventing per-ID partitions | N3 correctness from shipped-source evidence; N4 storage preservation |
| Signed, bounded, fully hashed immutable bundles before CAS publication | N1 reliable installation; N2 stronger release admission; N3 exact bytes |
| Preserve local overlays and conflict candidates; explicit rollback keeps current values | N3 authored edits; N4 no discarded work |
| Publisher/capability/revision-bound durable prompts; explicit full-access route | N6 shipped permission names/trust contract; N4 no excluded legacy extensions |
| Nonce applier, opaque origin, passive HTTPS only after consent | N8 logic isolation; N1 permission enforcement |
| Generation/sequence fencing and lossless bounded queues | N3 exact acknowledged operations; N7 inspectable ID-driven automation |
| Seal before kill, fresh snapshot, bounded Store crash retries | N4 acknowledged state survives; N1/N2 editor responsiveness; N8 main independent |
| Retain every existing app/spike/vendor consumer | Prune rule: no replaced code is removed before its last consumer |

## Linux validation and macOS handoff

Every shell sources `/workspace/.tools/activate.sh` and enters the repository.
The initial container lacked that script/toolchain; Rust 1.96.1, rustfmt,
Clippy and gitleaks were provisioned outside the repo, under `/workspace/.tools`.

```sh
cd apps/native
cargo test --locked -p pm-store-host
```

Attempted: blocked because the shared native lockfile needs the new package and
Ed25519/tar dependencies. This lane owns neither workspace manifest nor lock.
The existing `crates/*` glob discovers the crate. Integration must regenerate
the shared lock, then rerun that exact command; it must not be silently unlocked
here. The owned isolated manifest builds **the same source and test files**:

```sh
cargo test --locked --manifest-path apps/native/crates/store-host/isolated/Cargo.toml
cargo clippy --locked --manifest-path apps/native/crates/store-host/isolated/Cargo.toml --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/store-host/isolated/Cargo.toml --check
```

Linux evidence: **65 contract tests pass**, covering signed/tampered releases,
archive safety/budgets/checksums, durable immutable storage, CAS failure/reopen,
local edits/conflicts, archive rollback, permission persistence/revocation,
CSP, route/identity/schema/sequence enforcement, backpressure, process lifecycle
policy and independent retained stores in the Linux platform double. Strict
Clippy and formatting are required before commit. No private profile or real
project was read. No GUI, performance, sandbox or whole-app compatibility claim.

Must verify on macOS / integrate in other owned lanes:

- Wire this policy into `pm-compat-host`/remote DOM and the kernel actor; use
  async tickets and a confined Store V8 helper with signed sandbox entitlements.
  Provision signed-release distribution/rotation and provenance-based unsigned
  legacy import. Add agent/MCP routing for these exact IDs/prompts/actions.
- Real separate WKWebsiteDataStore identities/objects and content processes for
  two extensions across windows, close/reopen, crash/Stop, upgrade and uninstall/
  reinstall. Verify opaque storage denial, persistent-data policy and UUID/API
  availability across supported macOS; never share a store as fallback.
- Real CSP nonce and bridge-origin enforcement; external navigation/popup/file/
  media/download denial; permission persistence and revocation during requests;
  clipboard native focus/gesture and kernel resource checks; full-access path.
- Real process reset-state termination with old PID exit, callback cancellation,
  acknowledged edits/gesture sealing, native Stop/Wait independence, snapshot
  restoration, fault injection during install/kernel fsync, and disk-full errors.
- Read-only real-profile partition/origin inventory, IndexedDB/blob/localStorage
  reader handoff, merge/tombstone/idempotent receipts and reverse migration.
- IME, keyboard defaults/decline, VoiceOver/foreign view composition, overlays,
  detach/popout, 39-extension memory, headless/background export throughput and
  supported-OS performance/compatibility corpus gates. Linux doubles do not
  establish any of those results.
