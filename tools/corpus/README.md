# Native corpus P1 runner

**Cloud follow-up:** portable compiler/selection reductions are validated. Native glow/shadow pixel/export and default-path rotation qualification remain pending on Mac. See the [dated handoff](../../docs/native/handoff/corpus-p1.md#2026-11-04--cloud-corpus2-synthetic-reductions) for exact one-command folder rerun and limits. P1 remains open.

Run on the owner's Mac, never in CI with real data. Sources are read-only.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd apps/native
flock /tmp/pm-build.lock cargo build --locked -p pm-native-app --example pm-corpus
cd ../..
cd engine
flock /tmp/pm-build.lock cargo build --locked -p pm-glsl --bin pm-glsl-compiler
cd ..
python3 tools/corpus/run.py
```

Requires macOS/APFS for database metadata, `rg`, FFmpeg and at least **6 GiB free throughout each batch**.
Run Cargo from `apps/native` to select the pinned toolchain. All builds/tests must
also hold `/tmp/pm-build.lock`. Do not delete another lane's build artifacts.

The private `/tmp/pm-corpus` root is exclusively created with mode 0700. A single
driver lock prevents overlapping runs. Only library and database **metadata** is
retained between batches; the entire media library is never copied. The kernel
reader inventories indexed, trash, orphan and autosave slots. Associated `.pmv`
files and files under Documents/Desktop/Downloads/Movies are separate entries;
repeat `--projects DIRECTORY` to replace those search roots. Searches time out
after 30 seconds; Spotlight supplies indexed candidates as a fallback. The
timeout remains visible because an index cannot certify unindexed files.
`--extra-files PRIVATE_JSON` accepts an additional private list of source PMVs.

Chromium LevelDB is copied through `pm-native-migrate`, without opening a database
engine or recovering the original. `BlobCopy::Deferred` captures the blob file
identities but no ordinary media payloads. External wrapped database values are
materialized only when required to decode metadata. `Snapshot::copy_blob` copies
one referenced payload per project, verifies captured inode/device/size/mtime and
both source/copy checksums, and refuses changed or missing sources. Deferred
snapshots remain explicitly incomplete and cannot authorize full migration.
Ordinary `All` migration behavior and existing snapshot identities are preserved.

Each batch admits **one project plus at most 2 GiB of input media/project
bytes**, using independent APFS clones where possible. Cross-volume linked
originals use a bounded stream with their full size reserved above the floor.
Temporary extracted media, save staging and export outputs have separate free-
space reserves; they do not authorize a larger input snapshot. `--batch-gib 1.5` lowers that limit.
Only referenced assets are selected, across all compositions. Both shipped
`Index/sourcePath` and newer `objects/keys` media layouts are supported. Ambiguous
IndexedDB origins are resolved only when their copied content checksums agree.
A changed source, copy timeout, missing original or oversized project is recorded
explicitly. Capture limitations block dependent stages; they are not proof of an
engine defect. Exact captured IndexedDB identities are preferred over stale host paths; host
records remain available when the snapshot has no matching record.
macOS dataless placeholders are refused before payload reads, so the runner does
not ask a system daemon to hydrate originals outside the batch's disk controls.
Their dependency status remains unknown until a local source is available.

Kernel safe-copy open decisions are accepted within the authorized private
corpus. Pending recovery choices are still refused.

The worker receives only temporary media paths and an isolated native profile.
Library file associations are detached unless remapped to a copied file. No
original project is opened through a writable session. There is no Keychain,
Electron, extension execution, media GC, source cleanup or production-profile
write. No GUI app is launched. Any added on-screen run must first check
`/tmp/pm-shots/is_locked.sh` and exclusively create `/tmp/pm-shots/gui.lock`;
remove only the lock it owns and quit any launched app.

For every eligible project the worker attempts:

1. Kernel open with persisted history and PMV media.
2. Windowless Metal samples at start, midpoint and 90% duration. Missing original
   media cannot silently count as a render pass.
3. Transport playback that advances, followed by eight deterministic seeks,
   alternating the cached midpoint and seeded random positions. The worker waits
   for fresh GPU completion at the containing frame, and observes a cache hit.
4. Offline stereo audio mixes at the sampled positions, checking graph/decoder
   errors and finite samples without opening a speaker device.
5. A composition-name edit through the **actual app MCP adapter**, commit, kernel
   session SaveAs to a temporary PMV and document equality after reopen.
6. Service-child H.264 MP4 and ProRes MOV exports of a 0.1-second range, with audio
   enabled, followed by complete FFmpeg decoding with errors fatal.

Every completed batch is deleted immediately after recording results, including errors.
Admission stops below the 6 GiB floor; active headless children are checked each
second and stopped if available space drops below it. A refused payload reserve
blocks that capture while space remains above the floor; it is not a source-copy
defect. Whole-worker timeouts kill the process group
and export descendants. Oversized inputs remain explicit coverage blockers rather
than bypassing the batch limit. Results use pass/fail/blocked separately.

A `/tmp/pm-shots/PAUSE_LOCAL` marker prevents admission, stops an active
headless child and preserves the private root/batch. After the marker is gone,
`--resume-snapshot --resume-results` restarts only the unfinished batch from the
retained metadata snapshot. Use `nice -n 20` and four Cargo jobs while the owner
is active; the operator controls all screen-dependent checks.

The default removes the remaining metadata/log root even on ordinary failure.
Pause and disk-floor stops preserve it for a later resume.
Disk stops retain completed stage checkpoints and mark unfinished work blocked;
resuming recorded results continues to other entries. Use `--only-ids` for a
later resource-blocked recheck. Pause stops preserve the unfinished batch itself.
`--retain-for-debug` temporarily retains metadata and private error logs, **never
completed batch media**; remove `/tmp/pm-corpus` when done. An existing root is
refused unless `--resume-snapshot` explicitly selects this user's owned snapshot
of the same profile. Deferred sources that changed since capture remain refused.
`--resume-results` skips recorded entries; `--only-ids` selects anonymous IDs for
a focused check. A focused success cannot close the complete corpus gate.
Runner/worker checksums and the snapshot run ID record provenance. Resuming
results across changed tools or another snapshot marks mixed provenance and
cannot certify a complete gate. The worker executable is frozen in the private
run directory so shared-target rebuilds or cleanup cannot change a running audit.
The GLSL compiler helper is frozen too and explicitly supplied to workers and
their export children. Its checksum participates in provenance. `--compiler`
selects a helper explicitly; missing helpers fail setup before any project is
copied, rather than masquerading as unsupported GLSL effects.
Retained inventories are reused only with the same frozen worker, avoiding
repeated parsing of large project documents during a resource-limited resume.
The discovered file list and its search limitations are retained for the same
search roots; a resume does not erase earlier discovery timeouts.

`/tmp/pm-corpus-summary.json` contains only anonymized hash prefixes, counts,
byte sizes and fixed status/failure labels. Private manifests, exceptions and
media never belong in Git. Public evidence is `docs/native/corpus-p1.md`.
Identical content in separate library/file entries is intentionally counted twice.

This is sampled functional evidence at 320×180. It does not certify full-timeline
visual equivalence, physical scanout, live speaker/A-V sync or latency SLOs.
Unknown runtime layer/effect/transition definitions are deferred to P2; native
embedded definitions stay in P1. Opaque runtime references may require further P2
classification. A green functional sample is not a reference comparison of expression/runtime output.

Synthetic validation (no owner data):

```sh
python3 tools/corpus/synthetic.py
python3 tools/corpus/synthetic.py --worker-only --legacy --text
python3 tools/corpus/synthetic.py --rotated
python3 tools/corpus/synthetic.py --offline
python3 tools/corpus/synthetic.py --effect glow
python3 tools/corpus/synthetic.py --effect shadow
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_runner.py
# From apps/native, with engine/env.sh sourced and the shared target set:
flock /tmp/pm-build.lock cargo test --locked -p pm-native-migrate -p pm-first-launch -p pm-kernel
```


The driver accepts `--projects-only --projects DIRECTORY` for folder-only runs
without inspecting the default library or its media stores. Add
`--synthetic-preflight` to prove current builtin shader translation and generated
glow/shadow preview + MP4/ProRes pixels using the frozen worker/compiler before
capturing projects. Failed preflights stop setup and retain private diagnostics
with `--retain-for-debug`. Rotation qualification remains a separate explicit
`synthetic.py --rotated` check; the production guard is intentionally unchanged.

Portable checks live in the standalone workspace `tools/corpus/checks` (own
lockfile). Build `pm-glsl-compiler` with `--locked` from `engine`, export its
absolute path as `POWERMOVE_GLSL_COMPILER`, then run:

```sh
cargo +1.96.1 test --locked --manifest-path tools/corpus/checks/Cargo.toml
cargo +1.96.1 build --locked --manifest-path tools/corpus/checks/Cargo.toml
export PM_CORPUS_CHECKS="$CARGO_TARGET_DIR/debug/pm-corpus-checks"
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_runner.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_real_tools.py
```

The real-tool test freezes this portable inspection worker and the production
compiler, removes their original copies, and translates all 20 vendored public
builtin shaders through naga/MSL. It does not render or encode native pixels.
The five media reductions apply real library journal patches and inspect the
kernel's canonical document before exercising the worker's shared binding code.
