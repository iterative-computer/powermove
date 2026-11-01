# Transition Release preparation (P1 package 7)

Review branch: `feat/tr-prep`, based on `main` at `70ffd6494`. This branch must
not be merged or released as a cutover. Reference behavior is `v1.2.0`; the
implementation preserves the current Electron editing, extension and playback
paths. Native docs were read from `origin/feat/native-engine`: P0 decisions,
D-STATE, S6 migration, first launch and their media/reference-closure contracts.
No installed profile or real Keychain was read in the Linux implementation/tests.

## Decisions and selecting rules

| Decision | Rule |
| --- | --- |
| Port the native branch's BLAKE3 host store and worker hasher; keep its exact object/key layout | N3 exact identity, N7 inspectable state, N8 hashing off main's JS thread |
| New imports acknowledge IndexedDB, then CAS, then the existing playback library; preserve the same revision in each | N4 rollback; N6 existing synchronous/extension/media API behavior |
| Retain IndexedDB, immutable objects, key history, record history, encrypted sources and interrupted imports | N4 archive rather than delete; prune rule: the Electron playback library still has consumers |
| Migration conflicts retain both byte revisions and block completion; corrupt checkpoints fail closed | N3 correctness over imitation; N4 no silent replacement |
| Proxy provenance comes from source/project metadata, never a storage key or MIME guess; unknowns remain unclassified | N3 authored original identity; N4 no proxy promotion |
| Require 4 GiB free reserve plus the next staged payload before copying | N1 professional recovery; N4 no pruning to make room |
| Re-read the IDB inventory and Blob hashes before finishing; invalidate a pass if new TR imports arrive | N4 concurrent edits must not be certified as migrated |
| Native-compatible stable-inode `flock`, PID and heartbeat leases; locked external projects open as independent copies | N3/N4 multiple editors cannot overwrite one another; N7 inspectable ownership |
| Hash journal bases, preserve old unhashed journal support, reject replay onto a different checkpoint | N3 exact authored edits; N4 preserve both inputs; N6 legacy journal compatibility |
| Require two pinned, independently validated designated requirements on a legacy Keychain item | N4 secret continuity and both-app access; no decrypted profile/log files |
| Keep the actual switch offer disabled in this review build | N1/N2 cutover proofs and signing verification remain required, even after preparation completes |

## Media store and import path

The port is `apps/desktop/src/main/media-store/` plus `shared/blake3.*` and
`shared/media-store.ts`. It originates from `feat/native-engine` and has these
TR changes: archive replaced key mappings; fail on corrupt mappings; archive
launch-time partials; verify an existing immutable object before deduplicating.
Production BLAKE3 file hashing uses `WorkerPool`; Linux tests use the equivalent
`InProcessExecutor`. The main/preload/renderer build includes `media-hash-worker.js`.

Within the existing profile's `Media Store`:

```text
objects/<first 2 BLAKE3 hex>/<64 BLAKE3 hex>  immutable payloads
keys/<SHA256(UTF8 storage key)>.json          native host key schema
key-history/<key filename>/<hash>-<receipt hash>.json
migration.json                              resumable logical-copy checkpoint
records/<SHA256 logical-record JSON>.json    immutable record versions
passes/<SHA256 completed-pass JSON>.json     immutable completed inventories
local-storage/<SHA256 logical JSON>.json     exact localStorage snapshots
secret-handoff.json                         receipts only, never decrypted secrets
archived-tmp-<uuid>/                         interrupted CAS uploads
TR Archive/                                 interrupted old-library uploads/index writes
store.lock                                  media-store exclusion lease
```

Key JSON is the host's `key/hash/size/type/revision/at/source` schema. Posters
retain their own keys and immutable bytes. Every upload compares a full source
BLAKE3 against the fsynced file's BLAKE3 before publishing an object. A same-size
pre-existing object is rehashed rather than assumed correct. Writes fsync and
atomically rename JSON, then fsync the parent. The existing `Index/` playback
library remains in place; the prune rule forbids deleting its implementation or
records while its streaming consumers remain.

`legacy/core/media.ts` wraps `PM.MediaStore.put`. IndexedDB transaction completion
must acknowledge before the CAS upload starts. A failed CAS/library write returns
false and retains the already-written copies; it never reports a partial import
as fully saved. CAS receives full Blob bytes even when the playback library
chooses to link an original. The old profile and API remain usable on rollback.
Removal of browser duplicates and background source/media collection is disabled
for TR. Retain this policy through cutover plus one release. User-initiated project
editing/deletion and ordinary nonmedia cache/backup policies retain their existing
behavior. Imported agent artifact source files are retained in TR as well.

## Background pass, resumption and progress

After project readiness and a five-second first-paint delay, the renderer starts
preparation. A task toast shows completed/total record counts; Settings > Storage
shows the state and provides sign-in preparation. A one-minute status poll starts a delta pass when a new TR import has invalidated
readiness. A stable completed library is not repeatedly rehashed; settings exports
still refresh. Direct writes outside TR's media API are covered by native's final
origin delta. Each pass:

1. Opens the current origin's `powermove-media` database without forcing a schema
   version. Enumerates **every object store and primary key**, including old bare
   IDs, posters, unknown stores and metadata-only values. Resolves reads only after
   transaction completion. A blocked/error/abort leaves preparation incomplete.
2. Encodes structured-clone values as tagged logical JSON, retaining object
   references, arrays, Maps/Sets, Dates, binary values, undefined/nonfinite numbers,
   BigInts, unknown fields and Blob/File descriptors. Streams every Blob, including
   nested posters. Unsupported host values fail explicitly; source records stay.
3. Main leases a pass to one trusted app frame. It checkpoints the inventory and
   each verified payload before publishing its key. Payload IDs derive from its
   logical descriptor plus full BLAKE3. Normal asset `blob` fields keep their old
   key; nested/unknown payloads get deterministic `idb:<SHA256(locator+path)>` keys.
4. On restart or retry, hash-identical descriptors reuse previously verified
   objects and mappings without uploading again. Interrupted transfers retry that
   payload; all earlier completed objects remain. Every launch resets readiness
   and requires a delta pass. A destroyed, crashed or navigated renderer relinquishes its pass; old partial streams are retired before a replacement pass.
5. Flushes the logical file store and reads project/history/take metadata for
   supplemental proxy bindings. `playbackProxy` / positive `playbackProxyVersion`
   binds the stored payload as `proxy-of(original v2 fingerprint, sourcePath)`.
   Receipts keep the full stored hash separately; key source becomes `rendition`.
   Explicit import provenance is retained too. Other payloads are `unclassified`
   or `poster`, never invented originals.
6. Publishes immutable logical record receipts. Different pre-existing CAS keys
   keep their old mapping, keep incoming objects/receipts and block the pass.
7. Re-enumerates every record and hashes every Blob again. Any changed value,
   bytes, added/deleted key, incomplete record/upload, conflicting mapping or
   concurrent TR import prevents completion. Main rechecks published object
   hashes and mappings before atomically marking the pass complete.
8. Exports all remaining localStorage keys, preserving exact strings and adding
   parsed JSON where possible. Unknown/extension keys and prototype-like names
   survive. Logical snapshots are immutable; changed/deleted values leave prior
   versions available. No key is cleared or interpreted as a native feature flag.

`transition:request` is a trusted-frame-only, asynchronous preload bridge with
status/start/reuse/begin/chunk/finish/abort/record/complete/localStorage/secrets
operations. Upload and pass IDs are main-owned and window-bound. Chunks are at
most 1 MiB; neither main nor renderer reads an entire media Blob into memory.
JSON records/inventories are metadata-sized, not a streaming JSON export. BLAKE3
in the renderer yields between source chunks. A failed transfer retains its
source, pauses readiness, and retries from receipts. `migration.json` and immutable
record files are suitable for headless inspection; no document behavior depends
on a UI toast.

Preparation is **not** a native availability/reference-closure certificate. The
native importer must still capture a coherent final profile delta after Electron
quits and inventory current/trash/take/undo/redo/agent references, replay partial
history patches, resolve offline/original availability and adjudicate unclassified
payloads. Supplemental project scanning cannot prove packed/partial/opaque owner
closure. It must not promote any TR key to an original just because bytes exist.
The native importer must treat TR's `migration.json` as source evidence, not as
its own destination migration checkpoint: the schemas are intentionally distinct.
Its reader needs the TR record/proxy/pass/localStorage/Keychain receipt formats
before it can certify reuse. That native consumer work is outside owned paths.
Current `main` can also have older owned files/links under the playback library's
`Index/` without any remaining IndexedDB copy. Native must inventory these files
and links too; TR retains them and does not certify availability from IDB alone.
Other Chromium origins are preserved in the profile; native's direct-origin scan
remains necessary for origins not running this trusted renderer.

## Locks and journals

The small app-owned POSIX helper is `native/transition-lock.c`. It is built with
`-Wall -Wextra -Werror` on Linux and macOS and packaged as a separately signed
helper. Node does not provide `flock`, so the helper owns the descriptor while
Electron owns a pipe to it. It never reads project content or starts a runtime.

- Profile library lock: `<userData>/store/library.lock`, matching the native
  kernel writer's location for that library.
- Media store lock: `<userData>/Media Store/store.lock`. Any native process sharing
  this media store must acquire the same lock; this cross-lane consumer change is
  outside this branch's ownership.
- External project lock: replace `.pmv` with `.pmvlock`, exactly native's
  `project_lock_path`. Hold across the editing session, not just file reads/saves.
- Stable inode; nonblocking exclusive `flock`; `O_NOFOLLOW`; regular file only;
  JSON `{pid,heartbeat}` in epoch milliseconds. Heartbeat every five seconds.
  A nonzero owner is reclaimable only if PID is dead **and** heartbeat is at least
  30 seconds old. Malformed/empty/unknown ownership fails closed. PID zero means
  explicitly released; the inode remains. OS locking still excludes another
  process even if it observes a stale or released JSON record.
- Parent EOF/exit releases the helper lock; normal release records PID zero.
  A lost helper prevents subsequent writes. Closing a project releases its
  lease after queued file operations. Locked/read-only projects open as copies
  without an overwrite association. Saving them requires Save As.

Both external-file pending-publication journals and logical recovery patches now
carry `baseRevisionHash`. External hashes use the existing full-file SHA-256.
Logical patch hashes use SHA-256 of UTF-8 `JSON.stringify(checkpoint.proj)`; this
is a revision seal, not a normalized/reordered document representation. New entries
are based on the stored checkpoint. Asynchronous checkpoints capture their base
before awaiting disk, then rebase only surviving newer patches onto that captured
base. The seal is cached per checkpoint rather than fetching/hashing a whole
project for every edit (N2). Sync and async replay reject a differing seal without modifying either
checkpoint or journal. Existing unhashed journals continue to replay using their
old revision/sequence rules. Native readers must recognize this optional field
and preserve mismatches as recoverable conflicts rather than replaying blindly.

## Secrets and signing

`SecretHandoff` accepts a `LegacyKeychain` interface; Linux tests inject an
in-memory adapter. The macOS adapter launches the fixed packaged
`powermove-transition-keychain` helper and writes the request to stdin. There is
no shell, secret argv/env, plaintext profile file, or raw error/log forwarding.
The helper creates **legacy generic-password Keychain items**, so Keychain does
the re-encryption. It does not put the original safeStorage key into a manifest.

Discovery covers all four D-STATE families: sealed environment lines, cloud
session, all saved compatible-provider addresses (including the old single-key
field), and human-verification records. Original encrypted source files remain
unchanged. Items use service `Powermove Native Legacy Handoff` and a content-
versioned SHA-256 account derived from locator, sealed-source hash and requirements
hash. A new secret/identity creates another item rather than replacing old data.
Durable receipts contain only hashes and opaque Keychain persistent references.
Resume skips successful receipts; a concurrent encrypted-source change refuses
completion. Mutable plaintext buffers are zeroed after use. Electron's
`decryptString`, JSON serialization and Foundation necessarily create short-lived
immutable strings; no claim of complete process-memory zeroization is made.

The signed resource `transition/identities.json` must be supplied before signing:

```json
{
  "applications": ["/installed/Electron.app", "/installed/Native.app"],
  "designatedRequirements": ["<exact Electron DR>", "<exact native DR>"]
}
```

This is a schema example, not signing configuration. Do not ship placeholders.
Both signed bundles must be present. Requirements must be distinct, pinned to
the actual bundle identifiers, Apple generic anchor, Developer ID certificate and
Team ID. Obtain/review the exact DRs with `codesign -dr -`; compile them with
Security.framework and compare canonical requirement data. The helper validates
each signed bundle against its pin, compares its actual designated requirement,
creates `SecTrustedApplication` entries for both applications, and creates the
item with `SecAccess` containing both trusted applications. It reads back the
access list, refuses any broader decrypt ACL, and returns a persistent reference
only after verification. A pre-existing duplicate must match both bytes and ACL.
No `security ... -A` broad-access fallback is permitted.

The review branch intentionally includes **no identity configuration**. Missing
adapter/configuration, locked safeStorage, denied Keychain access, incorrect DRs,
ACL mismatch or failed receipts block secret completion. "Prepare Sign-ins" is
an explicit user action. No native app is launched or downloaded by this button.
Even if the preparation status is complete, "Switch to Native" remains disabled
until a separately reviewed release wires the signed native availability and
cutover gates. There is no update-feed or release-workflow change here.

## Validation and remaining macOS proof

Linux validation: **275 tests pass across 27 focused regression files**. This
covers TR migration/structured-clone export, native-store BLAKE3 vectors, all
related storage/save/media/preload tests, checkpoint rebasing, PID/flock leases,
publication interruption/resume, low space, corruption/conflicts, dual-write
failure retention and all four secret families. Separate journal/history regression
runs pass too. Main TypeScript checking passes with the cloud route declarations
emitted into scratch space; renderer Svelte checking has zero errors and two
existing warnings. `electron-vite build` passes, including the POSIX helper and
hash worker. The full-suite attempt found **6,765 tests: 6,706 passed, 21 failed, 38 skipped**.
One result came from Vite's cached pre-edit polling implementation; a fresh run of
all failed files clears that test. The **remaining 20 failures reproduce by exact
test name on an isolated `origin/main` (`70ffd6494`) checkout** under the same
Linux dependencies: media range/network checks, process-family cleanup, one
encoder case, media-tool output, audio/effect vocabulary, thumbnails, asset-delete
copy and status contributions. There are no current-only failures in that paired
rerun (139 passing on this branch versus 133 on main; both have 20 failures and
three skipped tests). The final 275-test lane run is fresh, after the last source
edit; all 27 files pass. The complete Electron suite is therefore **not green**,
and these baseline failures are not hidden or fixed by this lane.
Tests use synthetic temporary directories and fake secrets only. Rust `cargo fmt`,
Cargo tests and Clippy do not apply: this lane changes Electron TypeScript,
Svelte, C and Objective-C, and no Rust files.

Required on a signed macOS build before release:

- Compile and run the Objective-C Keychain helper with the real SDK; verify both
  app DRs, exact two-app decrypt ACL, readback by each app, locked/denied access,
  duplicate and interrupted handoff, changed secrets, uninstall/reinstall, and
  original Electron sign-ins remaining usable. No signing claim is made by Linux.
- Verify both helper architectures match the shipped app, separate helper signing,
  hardened runtime/notarization and stdin transport. The build uses the host
  compiler architecture; cross-architecture packaging requires an explicit target
  build/universal helper before signing.
- Exercise actual Electron IndexedDB/Blob/File export on synthetic and consented
  copied profiles (all stores/origins), power-loss boundaries, low disk, multi-GB
  media, nested/unknown values, permission failures, and concurrent imports.
- Verify APFS durability and `flock` exclusion against the actual Rust kernel,
  open-as-copy behavior, cloud/read-only paths, heartbeat/kill recovery, project
  tab/window/quit lifecycle, and the shared media-store lock consumer.
- Compare Settings/task progress against built-in panels in light/dark themes,
  accessibility and responsiveness while copying. Linux compiles these components
  but cannot prove AppKit/display behavior.
- Complete the native six-domain reference-closure, final delta and reverse-
  migration proofs, current secrets/settings handoff, restore/rollback workflow
  and updater/signing gates. Preserve archives and disabled media GC through
  cutover plus one release. This branch grants no release authorization.
