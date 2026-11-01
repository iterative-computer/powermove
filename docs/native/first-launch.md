# Native beta first launch

`apps/native/crates/first-launch` (`pm-first-launch`) coordinates migration on a
background worker before the shell attaches a library. This is a Phase 1 beta
bootstrap, not permission to release or cut over. The Electron profile and user
projects are read-only inputs. No real Keychain was accessed in verification.

## Flow

1. Choose Application Support/`com.zellzoi.powermove.native-beta`. Reject a source
   overlap, linked destination, foreign/nonempty initial destination or wrong
   profile identity. If Electron's `SingletonLock` indicates a live, remote or
   unknown owner, return `WaitingForElectron` without creating the native profile.
   The caller resumes after Electron quits; the coordinator never removes its lock.
2. Hold an exclusive native migration lock. Copy the file store, extension sources,
   agent workspaces/state/change history/tasks, attachment cache, fork updates,
   environment/cloud records, provenance and onboarding state into an unpublished
   checkpoint. Snapshot each Chromium origin's IndexedDB plus blob directory and
   localStorage through `pm-native-migrate`. Use APFS cloning where possible and
   bounded streaming otherwise. Compare source inventories before/after the whole
   checkpoint and hash authored files and copied blob payloads with BLAKE3.
   Hash logical store values too, after the kernel decodes gzip envelopes.
3. Fsync and rename the entire checkpoint, then publish its hash in `migration.json`.
   The complete key/file inventory and immutable values are the future 3-way base:
   a missing key on either side can be distinguished from a key absent at base.
   Unknown/quarantined store files remain in this private checkpoint.
4. Import registered nonproject state through `pm-kernel::LibraryWriter`, retaining
   original settings, workspace and `ext.<id>` shapes. Copy agent threads and
   attachments, and remap thread-owner/window project references to stable native
   copy IDs. Preserve arbitrary values/unknown fields. Copy ordinary authored
   files privately; seal `extensions-shared` read-only and create
   `extensions-overlay` for beta edits. Register installed extension/version/source
   facts through a kernel `StateTransaction`, preserving enablement.
5. Load every indexed, trashed, orphan and autosave project through the kernel's
   reader, including legacy journals and combined session/history slots. Save a
   stable named `Project — Native` copy through the kernel writer. Copy history
   unchanged, including packed dictionaries and redo position; validate it through
   `History::import`. Refused histories remain preserved and are reported by count
   and privately by project; missing slots are retained in the checkpoint/report.
   Trash stays trash. Original external-file associations are retained in the base
   and detached from these copies; external `.pmv` bytes are never copied here.
6. Before transferring media, persist availability for current/trash/take/undo/redo/
   agent references. Replay both directions of kernel-imported history so partial
   asset patches are joined to their surrounding documents. Include references
   found in thread/attachment and agent-checkpoint JSON. Preserve old bare-ID
   fallback and storage revisions. Join proxy flags from asset metadata, not from
   IndexedDB names. Record missing posters for regeneration. An unimportable undo
   inventory cannot certify complete closure.
7. Hold the native library lock while `pm-native-migrate::migration::migrate_many`
   streams each snapshot into the existing host media layout. Each immutable
   object is BLAKE3 verified; key conflicts keep both revisions. The closure records
   verified original / proxy-only / already-offline, plus explicit newly-missing,
   unclassified and poster-regeneration outcomes. Only newly lost/unknown original
   owners are blocked; offline projects still open. Proxy-only entries retain
   original identity, relink intent and an export-warning flag. No GC runs.
8. Import logical localStorage to native `local-storage.json`, preserving every
   origin/key and exact DOM string, including unknown extension keys. It does not
   activate the retired evaluator flag.
9. If encrypted secret payloads exist, return `NeedsSecretConsent` and emit
   `SecretPromptRequired`. The host displays `secrets::PROMPT`; only a subsequent
   explicit `resume_with_secret_consent` may access Keychain. The macOS backend
   reads the existing **Powermove Safe Storage** / **Powermove** item, decrypts
   Chromium macOS v10 (PBKDF2-SHA1/1003, AES-128-CBC), and places each value in a
   beta-owned Keychain item. All four families are supported: sealed env lines,
   cloud session, provider secret and human-verification records. Exact encrypted
   source files remain preserved; migration files contain only locators/hashes,
   never decrypted values. Plaintext/key buffers zero on drop. Native items use
   content-versioned identities and never replace differing items.
10. `AccessStarted` is durable before the legacy Keychain read. A kill/denial leaves
    that state and ordinary retries never silently prompt again; fresh user consent
    is required. Successfully imported secret receipts prevent repeated writes.
    After success, later `run` calls capture a fresh media delta automatically,
    retaining the initial 3-way base and all distinct delta checkpoints. They do
    not reimport settings/library state over beta edits or resurrect deletions.

`Event` provides started/published steps and count-only progress for a later UI.
The callback can pause with an error after a publication. The shell must keep its
library closed until the coordinator returns; individual writes are durable before
step receipts. Checkpoint, JSON and file publications use fsync + atomic rename.
The migration lock is process-owned and releases on kill. The kernel library lock
requires a dead owner **and** a heartbeat at least 30 seconds old before reclaim.

## Decisions and selecting rules

| Decision | Rule that selected it |
|---|---|
| Wait for Electron, then migrate a snapshot | §6 permits either safe snapshots or waiting; one coherent checkpoint is the simplest safe choice for file-store state as well as Chromium (N4, KISS). |
| Own bundle-ID profile; deterministic copy IDs; never adopt Electron's library | §6 beta isolation and named-copy rule; retries must not duplicate or overwrite work (N4). |
| Kernel reader/writer and state transaction; rollback-default `V1_2` disk format | Kernel remains the state authority; N4/§6 rollback compatibility, including the later gzip read correction. |
| Immutable base plus per-key/file receipts; preserve existing beta values | §6 checkpoint hashes, atomic resume and later 3-way merge; rollback never discards work. |
| Shared extension copies read-only, separate writable overlay | §6 beta edits cannot change shared sources; preserve forks, pending/trash installs and provenance (N4). |
| Replay history to inventory partial media patches | §6 closure includes undo-only references; complete objects alone cannot find those references (N4). |
| Keep proxy provenance separate from original identity | §6 proxy-only/export-warning rule and N3 exact authored intent. |
| Secret access is an explicit durable prompt step, with no automatic retry after access began | §6 skipped-TR direct Keychain fallback; beta identity does not inherit Electron's ACL. |
| Six GiB reserve plus worst-case staged bytes; stop rather than prune | Task's disk floor and N4; no source deletion or collection is a migration strategy. |

The additive migration API is limited to `migrate_many` in
`apps/native/crates/migrate/src/migration.rs`: multiple snapshot origins or a
source-linked/offline profile with no IndexedDB can participate in one closure
pass. `migrate` remains a wrapper, retaining the existing single-origin checkpoint
identity and serialized record hash. Ambiguous duplicate origin records are refused
explicitly; they are not silently chosen or overwritten.

## Verification and smoke counts

Verification: **16 first-launch tests and 22 migration tests pass**, as does the
existing kernel regression suite. Strict Clippy for the new crate (`--no-deps`),
formatting and diff whitespace checks pass. Workspace-wide strict Clippy also
reports existing warnings in the kernel, which is outside this lane.

All fixtures are generated in private temporary directories; tracked examples and
vectors are synthetic. The generated baseline has **1 library project, 1 history,
3 IndexedDB records, 1 localStorage key, 1 extension, 1 thread archive and 1 detached
attachment**, plus agent files. The later-import test adds **1 media record**.
Tests verify proxy-only and already-offline outcomes, media held only by a partial
undo patch, real undo after import, gzip-compressed storage/history, source file
nonmutation, source-linked media with no IndexedDB, trash/orphan/missing slots,
refused-history preservation, external-file nonmutation, post-checkpoint original
loss, beta edits/deletions, no-op repeats, later deltas, all step interruptions,
source/destination safety, low-space refusal and explicit secret consent/resume.

A separate test resumes a torn first state publication; another refuses a linked
native parent before it can create folders in Electron. All four secret payload
families are exercised.

The kill test SIGKILLs a child after its first immutable media object publication,
waits the actual 31 seconds for the kernel's stale-lock rule, and resumes without
lost state or duplicate library entries. It never edits a lock to make reclaim pass.
The crypto tests use only independent synthetic vectors and an injected in-memory
sink; the production Keychain backend is never constructed by tests.

Real-profile smoke: **0 profiles, 0 files, 0 blobs copied**. The optional APFS clone
smoke was skipped to protect the six-GiB floor while shared builds consumed disk.
No live profile, real project, Keychain item or private corpus identifier was used.
No private plan text, operational dump or screenshot is included in this document.

Run from `apps/native` with the pinned toolchain and shared target:

```sh
source ../../engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cargo test -p pm-first-launch -p pm-native-migrate --offline
cargo test -p pm-kernel --offline
cargo clippy -p pm-first-launch --all-targets --no-deps --offline -- -D warnings
cargo fmt -p pm-first-launch -p pm-native-migrate --check
```

## Integration still required

No kernel/core files were changed. Requested kernel/host APIs are:

- Logical localStorage read/write by origin/key, consuming the imported sidecar.
- An external-PMV embedded-media resolver for library-copy associations, using
  kernel-owned read capabilities without creating locks from the migration worker
  in user project folders. This coordinator preserves the association in its base;
  direct external-file open/save remains the kernel session workflow.
- A native secret service that resolves migrated Keychain locators for env/cloud/
  provider consumers and preserves current values for later reverse migration.
- Directory/profile inventory and extension-source resolution that enforce
  `readOnly` shared roots and direct all beta edits to the overlay. The coordinator
  currently performs the ordinary file copies itself; the kernel exposes store
  APIs and extension metadata, not a directory-copy API.
- Shell wiring for the background first-launch gate, waiting/progress/secret prompt,
  private history/missing-slot notices and per-owner media blocking. The media
  resolver must consume closure original/rendition bindings and export warnings.
  External `.pmv` opens/saves use existing kernel sessions/locks and default
  rollback-compatible save options; this crate never opens them as migration work.

TR version/signature/base-hash verification, the dual-write Electron transition
release and its shared-ACL Keychain handoff remain separate work. This coordinator
always uses copies, including skipped-TR installs; it never grants a cutover offer.
Signed-build Keychain ACL behavior and a real-profile full-media gate remain to be
verified. Unfinished Chromium transaction scopes require recovery/logical export
before direct reading; duplicate/ambiguous origin identities require resolution.
Arbitrary extension-owned media formats, opaque agent artifacts and runtime-owned
homes need service-specific inventory/adapters before full cutover certification;
ordinary authored agent files are preserved regardless of JSON discovery.

Three-way settings/storage/workspace/extension merges with tombstones, adding
beta-created projects/media, conflict review, journal compaction, reverse migration
and restore-build round trips are still cutover work. The checkpoint is their base,
not their implementation. GC stays disabled through cutover plus one release.


## Wave 2: Linux migration gates

Status 2026-11-01. **17 first-launch tests and 24 migration tests pass on Linux.**
This extends the earlier 16/22 verification; all fixtures remain synthetic.
The package 5 library reader also now honors v1.3 recovery sequences independently
of edit revisions (see [kernel P1 evidence](kernel.md#wave-2-linux-p1-kernel-and-library-evidence)).

| Gate P1 migration row | Linux coverage | Still open |
|---|---|---|
| Delta passes and interrupted resume | Existing step-by-step interruptions and process SIGKILL/resume stay green, including the actual 31-second dead-lock grace. The new interrupted-delta test resumes the captured pending checkpoint, imports the extra media once, and retains both completed passes. | Live macOS profile/Chromium recovery, OS-denied blobs and large-profile timing/RSS. |
| Archive-never-delete | The initial checkpoint remains byte-identical after Electron deletes an unknown file, changes a quarantined file, and adds media. Interrupted/resumed deltas retain the later quarantine bytes too. Source bytes are unchanged; beta settings and deleted extension data are not overwritten/resurrected. | Full current-project/settings/extension/secret reverse migration and restore-build round trip. |
| Proxy-record detection | The lower-level migration test proves either `playbackProxy` or a nonzero `playbackProxyVersion` prevents original promotion even when stored bytes match the sampled original fingerprint. Proxy-only status retains bytes and export warning; metadata still comes from project/history assets. | Real reference closure, orphan/unclassified record review, relink/export integration and service-owned opaque artifacts. |

Selecting rules: N4 preserves every source/base/pass and beta tombstone; N3
preserves original identity independently of a playable rendition; N6 preserves
extension storage contracts. No production first-launch behavior was replaced;
existing consumers continue to use the single kernel reader/writer (prune rule).

Signed Keychain consent/ACL, APFS behavior, quarantine/Gatekeeper, updater/TR and
physical power-loss verification require macOS. These tests do not close those
rows or authorize cutover. The kernel's headless journal/old-reader/FPS rows and
soak reproduction are recorded in [kernel.md](kernel.md).
