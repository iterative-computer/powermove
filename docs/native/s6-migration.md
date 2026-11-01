# S6: distribution and migration readers

Status: implemented reader/importer spike, **not a cutover approval**. Contract:
plan v6 §0 N4, all of §6, P0 S6 and §9; storage baseline is
[D-STATE](d-state.md), shipped `v1.2.0`. Only synthetic fixtures are checked in.
No real Keychain item was queried, no release/update was published, and no real
profile was modified. This crate has no Electron, V8 or Keychain runtime dependency.

## Decisions and the rules that select them

| Decision | Selecting rule |
|---|---|
| Read only a coherent copy, including external blobs; never open a live database engine | §6 snapshot rule; N4 |
| Scan manifest-selected SST blocks and WALs directly, checking checksums; recognize `idb_cmp1` without doing sorted lookups | N4: corruption must fail explicitly rather than silently shorten the library |
| Retain every record, old bare key, revision, unknown record field and external File descriptor | N4; D-STATE unknown-state rule |
| Join project metadata to records; never infer original identity from a record's name, MIME or fingerprint field alone | N3/N4; D-STATE §3 schema correction |
| Keep sampled v2 identity and full BLAKE3 separately; retain transformed/proxy bytes as renditions | N3; §6 original/proxy distinction |
| Publish immutable objects, keys and checkpoints atomically; preserve conflicting mappings and previous passes | N4: rollback never discards work; §6 conflict and state-machine rules |
| Require an explicit availability inventory for current/trash/takes/undo/redo/agent owners | §6 reference closure is measured against availability, not existence of a source path |
| Use the existing Electron/Squirrel path for transition, then Sparkle 2 with a fresh EdDSA feed | §6; choose the smallest maintained solution satisfying signing, recovery and native-update requirements (N4, pragmatic §9) |

The evaluated pure-Rust alternative was `rusty-leveldb` 4.0.2. Its `Cmp` API can
name `idb_cmp1`, but normal opening performs database recovery/writes; its public
iterator also lacks a reliable error-status channel for failed table-block reads.
That is unsuitable for a loss-sensitive migration proof. The smaller scanner
uses maintained `snap` and `crc32c` primitives and implements only the physical
read formats it needs. See [rusty-leveldb's API](https://docs.rs/rusty-leveldb/4.0.2/rusty_leveldb/)
and [table iterator implementation](https://docs.rs/crate/rusty-leveldb/4.0.2/source/src/table_reader.rs).

## What the reader decodes

Implementation: `apps/native/crates/migrate`, package `pm-native-migrate`.
The workspace already glob-includes `crates/*`; no workspace edit was necessary.

- **LevelDB:** CURRENT, the complete version-edit manifest (including table
  additions/deletions), only the live `.ldb`/`.sst` set, table footers/index/data
  blocks, prefix-compressed entries/restarts, masked CRC32C, raw Snappy, fragmented
  32 KiB WAL records, write batches, 56-bit sequence numbers and tombstones.
  Recovery ignores only a final torn/uncommitted WAL append; checksum errors fail.
- **Comparator:** accepts `idb_cmp1` or localStorage's byte comparator. It never
  seeks or merges by byte ordering. It scans live blocks and reconciles exact
  canonical encoded keys by sequence. Thus numeric prefix/string ordering does
  not need an imitation comparator. Arbitrary noncanonical comparator-equivalent
  encodings are not a supported input contract.
- **IndexedDB:** variable-width database/store/index prefixes; global database
  name/id mapping; database versions; store names, key paths, auto-increment and
  raw store metadata; record version varints and UTF-16BE string primary keys;
  external object entries for Blob and File (MIME, size, filename, modified time).
  It enumerates every record in `powermove-media`, including all object stores;
  shipped `assets` contains both media and `:poster` families.
- **Serialized values:** Blink envelopes 16–21, V8 formats 13–16, trailer interface
  lists, primitive/Unicode strings, plain objects, dense arrays, references to
  completed objects, and Blob/File host-object indices. IDB Snappy compression
  and external value-wrapper blobs are supported. Unknown tags/versions, cycles,
  invalid counts/references and incompatible metadata fail explicitly.
- **External files:** resolve Chromium's hexadecimal
  `<database>/<two hex digits from blob_number bits 8–15>/<blob_number>` layout.
  Empty blobs need no external bytes. File descriptors remain data; filenames
  are never used to construct filesystem paths.
- **localStorage:** `_origin\0<encoded-key>`, Latin-1 and UTF-16LE DOM strings,
  all origins and arbitrary trusted-extension keys. Export preserves exact
  strings and adds parsed JSON where possible. This includes easing, recent
  effects, browser fallback `pm.*` state and the retired evaluator flag; importing
  that flag does not enable it in a native app.

Format sources: [Chromium's coding scheme](https://github.com/chromium/chromium/blob/main/content/browser/indexed_db/docs/leveldb_coding_scheme.md),
[Chromium 144 coding implementation](https://github.com/chromium/chromium/blob/144.0.7559.0/content/browser/indexed_db/indexed_db_leveldb_coding.cc),
[Blink wrapper](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/modules/indexeddb/idb_value_wrapping.cc),
[V8 serialization](https://github.com/v8/v8/blob/main/src/objects/value-serializer.cc),
[blob path implementation](https://github.com/chromium/chromium/blob/main/content/browser/indexed_db/file_path_util.cc),
[localStorage format](https://github.com/chromium/chromium/blob/130.0.6723.0/components/services/storage/dom_storage/local_storage_impl.cc).

### Snapshot and memory boundaries

`Snapshot::capture` copies to a new private directory, preferring `clonefile`
(`CLONE_NOFOLLOW`) with a bounded streaming fallback. It compares source
size/mtime/inode/device inventories before and after the whole operation. A
changed inventory fails and deletes only this attempt's destination. The caller
retries or waits for Electron to quit; it never kills Electron or takes its lock.
The complete source inventory includes blobs, so imports during capture cannot
silently appear as a complete snapshot. CURRENT/manifest/table/WAL copies have
BLAKE3 seals that the reader checks. No LevelDB LOCK, recovery, compaction or
source-file write occurs. Destinations overlapping source/snapshot roots fail
before importer writes.

Sample mode caps blob bytes/files and always marks the snapshot incomplete. It
may skip an OS-denied blob to choose another sample. **Full mode fails on an
unreadable blob. A sample can never authorize migration.** The real-profile
proof below used at most 32 MiB/eight files per origin; no multi-GB blob directory
was copied. Full capture is available to the host for a coordinated migration
with sufficient disk space; it was not run on the real profile.

Space preflight requires a 4 GiB reserve plus worst-case copy bytes, even when
APFS cloning might save space. Payload hashing/copying uses bounded I/O (1 MiB
hash buffers, no payload-sized allocation). Descriptor sizes and offsets are
64-bit. Metadata is bounded separately: 64 MiB per encoded/decompressed value or
block, 256 MiB for the live LevelDB map, depth/node/expansion limits for structured
clone. Exceeding a bound is an explicit logical-export/recovery requirement, not
an empty result.

**Unfinished IndexedDB scopes:** physical WAL recovery is not IndexedDB
transaction recovery. If Chromium's scope undo/cleanup records remain, the reader
refuses that snapshot. It does not pretend tentative writes were committed.
A quiescent/running Electron and a normally quit Electron were tested. Standalone
recovery of a crashed Chromium scope remains a cutover blocker for the direct
path; see unverified work below.

## Media transfer and reference closure

The importer matches `main/media-store/store.ts` and the shared host schema:
`objects/<first-two-BLAKE3-hex>/<BLAKE3>` and
`keys/<SHA256(UTF8 legacy key)>.json`. Key JSON retains `key/hash/size/type/revision/at/source`.
Source records and unknown fields stay in the private migration checkpoint.

1. Lock the destination's migration writer; the host must also hold the library
   lock. Verify space, hash the source, clone/copy to staging, fsync and hash the
   stored bytes with BLAKE3, then atomically publish the immutable object.
2. Checkpoint the record before publishing its key; publish the key and checkpoint
   again. A crash between these operations is replayable. Existing differing keys
   are preserved, incoming objects/records remain in the manifest, and the conflict
   prevents cutover readiness. No incoming mapping overwrites native or beta work.
3. Resolve references by content key with old-id fallback and optional revision.
   Ambiguous records fail. `playbackProxy || playbackProxyVersion > 0` comes from
   the project/history/take reference, never an assumed IndexedDB field.
4. Check a candidate original at `sourcePath` by the shipped v2 rule (deduplicated
   start/middle/end 64 KiB samples plus `powermove-media-v2\n<size>\n`), or an
   existing full BLAKE3 identity. Recheck the copied object, record its full BLAKE3,
   and retain the stored proxy separately. A proxy flag prevents promotion even
   if a sampled fingerprint collides. A transformed payload mismatch stays a
   rendition with an export warning. Orphan records remain unclassified.
5. Publish closure entries with reference/proxy-of fingerprint and source path,
   original/stored hashes and one of verified-original, proxy-only, already-offline,
   newly-missing, regenerate-poster, or unclassified. The last three are explicit
   operational states, not invented originals. Already-offline references remain
   usable as offline; absent posters can regenerate. Only affected owners enter
   `blockedOwners`. Losing an available original still blocks its owner when a
   proxy survives. Unknown original identity also blocks export-safe admission.
6. Set `cutoverReady` only when all six owner domains were inventoried, objects
   verify, no owner is blocked and no key conflict remains. Atomically finish the
   pass. A repeated identical pass verifies objects/key mappings and performs no
   publication or timestamp rewrite. New source/reference checkpoints add a delta
   pass; previous passes and objects remain intact.

`ClosureInput` is an explicit host contract, not a claim of complete app wiring.
`discover_assets` walks nested JSON asset objects (including history dictionary
records and take/patch values); it cannot prove closure from packed paths alone,
PMV byte ranges, arbitrary extension data or agent artifact files. The host must
supply those owners too, and take the availability checkpoint before migration.
This crate deliberately cannot declare those missing integration steps complete.

The native media resolver must consume closure original/rendition bindings before
using legacy keys. A key pointing at legacy proxy bytes is not an original simply
because it exists in the host store. Keep GC disabled through cutover + 1 release.
Restore must use these same immutable objects and all retained key revisions;
current project/history/settings/secret reverse migration and journal compaction
remain the host's responsibility. An old pre-migration backup is not rollback.

## Verification and measurements

The independent fixture writer is Electron **44.0.0**, using its actual IndexedDB
and localStorage APIs in a new synthetic profile. Its small committed fixture
contains five records/85 payload bytes, a File, Unicode, a poster, an old bare id,
and five localStorage entries. IndexedDB was captured with that synthetic process
still running after transaction completion; localStorage was captured after quit.
No installed app or real profile was opened by the fixture writer.
The end-to-end CLI also migrated those five records with six closure references,
exported all five localStorage entries, and verified that repeating the migration
left checkpoint bytes and modification time unchanged. An invalid snapshot-mode
argument was rejected before copying anything.

Additional generated tests exercise SST Snappy/restarts, manifest table deletion,
custom-comparator prefix ordering, sequence/tombstone reconciliation, fragmented
and torn WALs, CRC failures, wrapped/compressed values, unfinished scopes, source
nonmutation, denied/sampled blobs, low space, proxy/original/transformed cases,
history/offline/poster closure, key conflicts, changed deltas, publication-boundary
interruptions/resume, repeated-pass no-op, missing published keys, and source/
destination overlap. The crypto proof is test-only.

Reproduce without modifying the shared workspace or its lockfile:

```sh
apps/native/crates/migrate/tests/check.sh
apps/native/crates/migrate/tests/check.sh --release-bench
```

The script sources `engine/env.sh`, reads the pinned native toolchain (1.96.1),
uses only `~/Developer/pm-target`, checks free space, and creates a temporary
manifest referencing the actual source/test/example files. No alternative rustc,
extra target directory or `cargo clean` is used.

### CLI example

The example provides `snapshot-media`, `snapshot-local`, `media-stats`,
`local-stats`, `local-export`, and `migrate-media` commands. Stats print only
counts/sizes/timings. Logical exports and manifests are private destination files.
For example, after building the example with the pinned toolchain/shared target:

```sh
READER="$HOME/Developer/pm-target/debug/examples/migrate"
FIXTURES="apps/native/crates/migrate/tests/fixtures"
"$READER" snapshot-media "$FIXTURES/electron/indexeddb" "$FIXTURES/electron/blobs" /tmp/pm-s6-example-snapshot
"$READER" media-stats /tmp/pm-s6-example-snapshot
"$READER" migrate-media /tmp/pm-s6-example-snapshot "$FIXTURES/closure.synthetic.json" /tmp/pm-s6-example-store
```

Use fresh destinations. The synthetic closure illustrates all six owner domains;
real closure input must come from the host's complete availability inventory.
A completed step can still have `cutover_ready=false`; callers must honor that
flag and `blockedOwners`/`mappingConflicts` rather than treating process exit 0 as
cutover approval.

### Real-profile read-only proof (aggregates only)

All reader inputs were copies under `/tmp/pm-s6-snapshot`, deleted after each
attempt, including failed attempts. Two IndexedDB origins were scanned. No names,
paths, keys, values, secrets or real-derived fixtures were retained.

| Reader result | Count / logical bytes |
|---|---:|
| IndexedDB records | 593 |
| Media records | 442 |
| Poster records | 151 |
| Payload bytes described by records | 15,924,540,390 |
| External files inventoried | 591 |
| External file bytes inventoried | 15,742,869,131 |
| Sampled blobs actually read and hashed | 16 / 9,128,094 |
| localStorage entries / UTF-8 value bytes | 6 / 1,589 |

The two catalog reads, including their sample hashing, took 0.100959 s and
0.050401 s; capture took 0.104916 s and 0.090773 s. The localStorage read took
0.001179 s. These are single warm/uncontrolled observations, not paired §5 gates.
Descriptor totals need not equal file totals; the sample does not establish
full payload availability, reference closure or original/proxy classification.
Some eligible blob reads were denied by the OS. Sample mode selected other
readable blobs; a real full migration would stop and preserve the source until
those access failures were resolved.

All **22 tests**, formatting and strict Clippy checks passed.

The **64 MiB synthetic release migration measured 284.31 MiB/s**, 0.225105 s,
with **5,226,496 bytes maximum resident memory** for the entire test process
(`/usr/bin/time -l`; 0.39 s including fixture setup/capture). The run prefers
cloning and includes full source/copy/publication hashing, fsync, original
verification and checkpoints. This is effective importer throughput, not an
isolated hash-loop result. The machine/cache were not controlled for a §5 gate.
Multi-GB full transfer was deliberately not run against real media.

## Safe Storage and two-bundle Keychain plan (research only)

**Format correction:** macOS OSCrypt uses `v10`; `v11` belongs to Linux's secret
backend, not a second macOS variant. Mac v10 derives 16 bytes with PBKDF2-HMAC-SHA1
(password = the exact Keychain password bytes, salt `saltysalt`, 1003 iterations),
then AES-128-CBC with a 16-space IV and PKCS#7 padding. Linux v11 uses its backend
password and one derivation iteration. Reject a Mac v11 record explicitly rather
than decrypting with guessed parameters. Empty ciphertext is the empty value;
unknown formats are retained for recovery. Sources:
[macOS OSCrypt](https://github.com/chromium/chromium/blob/144.0.7559.0/components/os_crypt/sync/os_crypt_mac.mm),
[Linux OSCrypt](https://github.com/chromium/chromium/blob/144.0.7559.0/components/os_crypt/sync/os_crypt_linux.cc).

`tests/safe_storage_proof.rs` implements only this synthetic decryptor. A Mac
vector was generated independently with Python PBKDF2 and OpenSSL encryption;
Mac v10 decrypt, Linux v11 round trip, wrong-password/truncation rejection and
Mac-v11 rejection pass. It is a dev-only test, absent from library/CLI exports.
CBC has no integrity protection: the proof is format compatibility, not a new
secret-storage design. Production must validate the expected payload schema.

For skipped TR, query the exact legacy generic-password service **Powermove Safe
Storage**, account **Powermove**, through Security.framework in the signed app,
allowing the OS's access prompt once. Never create a missing legacy password:
that would not decrypt existing ciphertext. Preserve ciphertext on denied/locked/
missing item. No shell `security ... -w`, password logging, migration-manifest
plaintext or runtime OAuth credential harvesting. The actual installed item and
its ACL were **not read** in this spike.

TR handoff plan:

1. Keep production/cutover bundle id, Team ID and designated requirement identical.
   Beta has its distinct native-beta bundle id and profile.
2. TR, already authorized for legacy Safe Storage, decrypts the four D-STATE
   families in memory: extension env, cloud session, compatible-provider secret,
   and human-verification proof. Preserve comments/unknown non-secret fields and
   expiry semantics; never put decrypted values in a checkpoint.
3. Create versioned **legacy file-based Keychain generic-password items**, one per
   handoff secret, with an explicit trusted-application ACL for production and
   beta designated requirements. Keep the legacy Safe Storage item unchanged.
   Use public `SecTrustedApplicationCreateFromPath` on validated signed bundles
   and `SecAccessCreate` with the two trusted applications. Do not use a nil
   allowlist or a Team-ID-wide grant. Do not confuse legacy ACLs with modern
   data-protection-Keychain access groups.
4. If beta is not installed, ship a small inert, signed identity-anchor helper in
   TR with beta's exact standard designated requirement; extract the trusted
   application from that signed code. Verify the future beta satisfies that
   requirement. This avoids assuming bundle-id text alone grants access, and
   avoids the private `SecTrustedApplicationCreateFromRequirement` SPI.
5. Read back each handoff item using signed production and beta fixtures before
   retiring any checkpoint step. Preserve denied handoffs for retry. Native
   changes and intentional deletions must be mirrored into the versioned handoff
   or reverse-migrated from current native Keychain state on Restore; never restore
   stale pre-cutover secrets over a deliberate sign-out.

Apple API basis: [trusted applications](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecTrustedApplication.h),
[access objects](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecAccess.h),
[ACLs](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecACL.h).
Signed tests on supported macOS versions must cover both bundle ids, future
updates with the same DR, rejection of a third/untrusted identity, locked/login
Keychain, denied prompt, moved bundles, item replacement, and current-state
rollback. The identity-anchor approach is a plan, not a demonstrated ACL pass.

## Squirrel transition and interrupted updates (research only)

Research pinned to `electron-updater` **6.8.3**, Electron **44.0.0**, and its Squirrel
revision **8d808803bc89ec0e2aa1450474856dfee3b00c6b**. `MacUpdater` chooses the matching
architecture's **ZIP**, downloads/checks it and exposes it through a local proxy
feed to Electron's native updater. Squirrel validates the incoming application
against the installed app's code requirement, including nested code and all
architectures. That path operates on signed `.app` bundles; it does not require
an Electron executable inside the replacement. Therefore a native replacement
is supported by the inspected mechanism, **subject to signed installation tests**.
Sources: [MacUpdater 6.8.3](https://github.com/electron-userland/electron-builder/blob/electron-updater%406.8.3/packages/electron-updater/src/MacUpdater.ts),
[Electron 44 pins](https://github.com/electron/electron/blob/v44.0.0/DEPS),
[Squirrel installer](https://github.com/Squirrel/Squirrel.Mac/blob/8d808803bc89ec0e2aa1450474856dfee3b00c6b/Squirrel/SQRLInstaller.m),
[signature verification](https://github.com/Squirrel/Squirrel.Mac/blob/8d808803bc89ec0e2aa1450474856dfee3b00c6b/Squirrel/SQRLCodeSignature.m).

Feed plan:

- Existing public stable/beta feeds deliver TR and maintenance Electron builds.
  Publish native cutover metadata at a separate generic HTTPS feed known to TR;
  never expose a higher native version as the legacy GitHub latest release or
  legacy `latest-mac.yml`/`beta-mac.yml`. A separate URL is sequencing, not an
  authorization secret. A manually installed DMG still runs the direct delta pass.
- Only TR on macOS 14+ with a fresh successful media/state/secret checkpoint may
  switch to that feed or check/download/stage its update. Re-run the delta after
  later imports; TR dual-writes new imports. macOS 13 stays on Electron's feed.
- Serve architecture-correct signed/notarized native ZIPs with YAML version,
  URLs, sizes and SHA-512 integrity metadata. Preserve bundle id, Team ID, DR,
  URL/document associations, display name and bundle layout. Disable differential
  download for the first Electron-to-native transition to avoid relying on an
  unrelated previous Electron archive; subsequent Sparkle updates use its format.
- Wait for the **native** updater's `update-downloaded` event, not merely
  electron-updater's earlier notification. Quit normally through the save/journal
  barrier. Never bypass it with a forced updater quit. Existing `updates.ts`
  already makes that distinction.

Interruption plan: test network cancellation/bad checksum during download (old
app remains), TR crash before staging, quit/save cancellation, ShipIt termination
before acquiring the old bundle, after moving it into owned storage, after
replacement, and before relaunch. The pinned installer synchronizes its owned-
bundle recovery state in CFPreferences; on a new install attempt it tries to
restore an owned old bundle before installing again. This is evidence of a
recovery mechanism, not proof of power-loss safety. Do not delete old owned
bundles, installer state or update caches during recovery. Keep profile migration
state independent of installer staging; native first launch re-verifies the
checkpoint and performs its delta even after an interrupted replacement. Test
permission/read-only volume failure, insufficient disk, translocation, same-DR
acceptance, wrong-DR rejection, macOS 13 gating, both architectures and rollback
with current untitled/journal work. No real installation/interruption was run.

## Post-cutover updater recommendation

**Choose Sparkle 2**, pinned to a currently security-maintained release during
integration, with a **new Ed25519/EdDSA keypair and appcast**. Bootstrap
`SUPublicEDKey` and the new HTTPS feed URL inside the notarized native bundle
that Squirrel installs. There is no old EdDSA key to migrate. Keep the private
signing key outside Git in release signing infrastructure; sign every native
archive/appcast enclosure and preserve app code-signature validation. Do not
reuse the Electron YAML as an appcast or reuse beta feed identity as production.

The selecting rule is N4 plus §9 pragmatism: the smallest maintained updater
that covers signed payloads, staged installation, recovery, supported macOS and
normal native app behavior. Sparkle supplies those mechanisms and an actively
maintained security history. An own updater would need equivalent signature/
key-rotation validation, safe extraction, atomic replacement/recovery, helpers,
permissions, relaunch/save-barrier integration and hostile-input testing. No
unmet requirement justifies that extra implementation. Sources:
[Sparkle setup/signing](https://sparkle-project.org/documentation/),
[security/reliability changes](https://sparkle-project.org/documentation/security-and-reliability/).

## Remaining gates (explicitly unverified)

- Standalone recovery of unfinished Chromium scope transactions. Current behavior
  is an explicit stop, preserving source, until Electron recovery/quiescence.
- Complete real-profile transfer/availability verification, unreadable blob access,
  large real media throughput, complete project/history/trash/take/PMV/agent
  reference extraction, and original/proxy classification across that closure.
- Structured-clone formats outside the supported media-record subset, historical
  schema 3 external File metadata, noncanonical equivalent IDB keys, and metadata
  above the documented bounds. These fail rather than silently omit data.
- Signed Keychain two-DR ACL/identity-anchor proof, skipped-TR real secret decrypt,
  secure production handoff, deletions/sign-out, and reverse secret migration.
- Signed Squirrel-to-native install and actual ShipIt/network/power interruptions;
  feed gating, notarization, TCC continuity, Sparkle integration and key rotation.
- Complete §6 forward/reverse application round trip, prolonged native journal
  editing then Restore, binary PMV/container import, locks/cloud conflict handling,
  beta conflict UI and GC coordination. The crate preserves media; it does not
  implement those application owners.
- P0 S6's agent-client conformance replay belongs to the service/runtime integration;
  it was not run by this storage/distribution reader spike.

These remaining gates prevent declaring S6/cutover passed. They are not exceptions
to N4, and no data is removed to make a gate appear green.
