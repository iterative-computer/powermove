# Beta readiness preparation — cloud beta2

2026-11-04 (America/Chicago). Synthetic Linux evidence only. No signing,
notarization, publishing, workflow dispatch, real profile or Keychain access.
This lane does not declare beta/cutover readiness.

The worker bridge in `pm-first-launch::integration` implements the existing
`pm-distribution::update_handoff::HandoffHost` using the real first-launch
coordinator. Direct DMG, skipped/invalid TR and verified TR all run the importer
and a fresh final delta before reading owner evidence. Electron lock waiting,
secret consent, missing/refused history and owner blocking remain explicit.
`Evidence` is an application capability held under source/import leases; it must
supply fresh generations, signed identities, state/secret closure and the existing distribution recovery gate. No production success implementation is provided. The isolated
beta can finish importing while cutover remains blocked. Poll scheduling uses the
existing 30-second/four-hour intervals.

`store_preparation::merge` experiments with supplied three-way store snapshots and
missing/null distinction, tombstones and member-level `ext.<id>` merge. Atomic
project/history/workspace values retain both sides in conflict records. The
kernel owner must first flush/compact current journals and capture current state
under its leases. `prepare_store` writes an unpublished store through
`LibraryWriter` and archives complete input maps, unknown values and conflicts.
An interrupted `.preparing` attempt is retained, then archived by a retry. This is
store preparation, not a complete profile/media/Keychain Restore implementation.
No source profile is replaced. Never pass decrypted secrets to store maps.

First-launch capture failures, stale capture attempts and duplicate completed
deltas now remain archived. Failed lower-level snapshots and unreadable sampled
partials also remain intact; an incomplete snapshot cannot be admitted as complete.
No source, base, history, quarantine file or previous attempt is pruned.

## Sparkle public-API mismatch and required integration

`first-launch::sparkle::Driver` implements the existing `Installer` trait over an
injected Sparkle capability. It checks exact version/URL/length/EdDSA enclosure
selection and streams a fresh SHA-256/size verification before delegating stage.
The native capability owns authentication, extraction, signatures/DR, platform,
expanded-space checks, tickets and persisted recovery. No macOS success double is
compiled into production. Tests exercise enclosure mismatches, changed bytes,
interrupted extraction, retained archives, pending recovery and explicit install.

The existing `Installer::stage(archive) -> ticket` API is **not implemented by
Sparkle 2.10.1's public API**. `SPUUpdater` owns its own download/extraction cycle;
`SPUUserDriver` receives callbacks and reply continuations. It offers no public
"adopt this externally downloaded ZIP" entrypoint. Calling an imagined selector
or returning a staged ticket before the authenticated callback would be incorrect.
This lane cannot truthfully call its policy bridge a linked production installer.

Required outside owned paths: distribution/services must expose an asynchronous
Sparkle-owned download path (or obtain an upstream supported archive-adoption
API), with policy/enclosure binding and freshly checked migration/save barriers at
all side effects. Shell must retain its custom `SPUUserDriver` on the main run loop,
enqueue Rust worker events and enforce normal quit. In particular, Sparkle may
install after ordinary termination once extraction starts; holding only the
`showReadyToInstallAndRelaunch` reply is insufficient to guard every quit. Recovery
must query installer state and verify the installed signed identity, not infer
success from a cached in-memory ticket. Public header evidence, inspected at the
pinned tag: [SPUUpdater](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUpdater.h),
[SPUUserDriver](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUserDriver.h),
[SPUUpdaterDelegate](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUpdaterDelegate.h).

## Decisions and selecting rules

- Always rerun the final delta after bootstrap/consent and then obtain owner
  evidence; TR receipts never skip data work (N3/N4).
- Readiness remains false without signed identity/full closure/installer recovery
  evidence; passing import models is not release proof (N1/N4).
- Archive failed and duplicate attempts, including partial snapshots (N4).
- Preserve missing versus null and concurrent values; merge extension members,
  keep authored arrays/history atomic (N3/N4/N6).
- Use the kernel writer for a new unpublished store; require the owning actor's
  compaction barrier and preserve old inputs (single writer/N4/prune rule).
- Implement exact policy validation over a macOS capability, and report the
  unsupported Sparkle adoption API explicitly (N3/N4/N8). No invented selector,
  permissive installer or synchronous main-thread wait.

Opus specifications at `ce36cb6fc` now define N001–N007 and S111–S113.
`pm-first-launch::surfaces` projects their copy, stable actions and state policies
without drawing or changing tokens. Product copy states that original data
remains unchanged. It offers no way-back action or file-compatibility promise.
Reverse migration is not built; the
experimental unpublished store preparation helper is not a user-facing Restore
feature and its tests establish no such capability.

## Native resource/rpath inventory

Source inventory at integration base `ed5482622f41c533199a724dcb8c8738dd4dcc93`.
This lists concrete consumers, not a claim of a closed release payload. Final
per-file hashes and transitive Mach-O loads require an actual Mac bundle.

| Payload | Existing consumer / distribution requirement | Current disposition |
| --- | --- | --- |
| `powermove-native` | app/build.rs sets macOS 14.0 and `@executable_path/../Frameworks` | Audit every delivered slice and copied binary. |
| FFmpeg (`avformat`, `avcodec`, `avutil`, `swscale`, all transitive dylibs) | engine/crates/pm-media/build.rs; engine-service/build.rs; app/build.rs | Build scripts also embed an absolute FFmpeg rpath. Release must copy the full closure and rewrite/relink those loads/rpaths before signing. This lane cannot edit those owners. The audit blocks absolute host paths. |
| `pm-glsl-compiler` | engine/crates/pm-glsl/src/compiler.rs searches adjacent to the current executable, or an explicit `POWERMOVE_GLSL_COMPILER` development override | Bundle adjacent to the app binary and audit/sign as a helper. Do not depend on the build-machine override. |
| Store realm helper `pm-store-helper` | runtime-v8/src/store.rs `StorePool` launches its configured executable with OS confinement | Explicit helper path, same-architecture V8, sandbox policy and signed confinement/JIT checks remain required. |
| V8 archive/bindings/snapshots and compiler JS | runtime-v8/build.rs requires macos-sandbox; vm.rs includes the pinned Svelte compiler and web.js; runtime/cache constructors consume snapshot/bundle bytes | Record statically linked inputs separately from external runtime resources. No invented mandatory snapshot filename. Owner must specify the actual snapshot provider and hash. |
| Built-in v4 and legacy extension source/resources | runtime/Store activation consumes actual entry bundles; first launch copies shared extension sources and creates an overlay | Bundle production entries/resources and preserve installed user sources. A manifest must distinguish shipped payloads from per-profile copies. |
| Agent runtime helpers/assets | agents/src/runtime.rs reads pinned runtime facts; actual provider executables/homes are separate runtime capabilities | Inventory selected native provider binaries and resources; do not copy terminal homes or credentials. Missing provider payloads cannot become optimistic capabilities. |
| Fonts | pm-services FontHook uses the CoreText broker; project/extension fonts may be runtime resources | System fonts require no copied files. Owner must enumerate any shipped/custom fonts and registration paths; no substituted font can count as exact authored output. |
| Sparkle 2.10.1 framework and nested helpers | packaging pin and bundle input; release framework preserves contained version symlinks | Verify pinned upstream archive, inventory all nested Mach-O/signatures, then audit framework links and each helper entry point. No framework binary is vendored. |
| Icons, accent catalog/Assets.car, update-config.json | existing packaging/package.py layout | Existing packaging inputs; keep update config disabled/unset until owner gates. No new visual choice. |

`apps/native/scripts/audit-native-bundle.py` is a read-only audit. It records all
files/hashes/contained links, and runs `lipo -archs` plus `otool -arch <slice> -l`
for **every delivered slice**. It checks the macOS 14.0 floor, resource hashes,
all explicit helper roots, inherited `LC_RPATH` stacks, loader/executable-relative
loads, architecture closure, escaping links, undeclared/unreachable Mach-O and
absolute host dependencies/rpaths. Missing weak loads stay visible too. It makes
no `install_name_tool`, copy, signing or distribution calls. Errors exit 2; a
completed blocked report exits 1; a clean audit exits 0. It never overwrites a
report. Reports may contain private source paths from load commands; keep actual
reports outside Git and review before sharing.

The owner-supplied JSON requirements schema is:

```json
{
  "schema": 1,
  "reviewed_complete": false,
  "architectures": ["arm64", "x86_64"],
  "minimum_macos": "14.0",
  "executable": "Contents/MacOS/powermove-native",
  "helpers": [],
  "roles": {},
  "resources": []
}
```

This intentionally incomplete example **fails**. Each role `engine`, `v8`,
`extensions`, `agents`, `fonts`, `sparkle` needs `{ "kind": "bundled" | "linked" |
"not_required", "evidence": "owner-reviewed source/build disposition" }`.
Every bundled role needs at least one resource. Every required file, including
every Mach-O, needs `{ "path": "Contents/...", "role": "engine", "sha256":
"actual lowercase SHA-256" }`. List helper executables as bundle-relative paths.
Use separate architecture requirements for separately shipped arm64/x86_64 apps.
The review flag establishes declared inventory completion only; it does not
prove signing, runtime discovery, actual installation or application readiness.

Mac command (read only; no signed build required for this first audit):

```sh
python3 apps/native/scripts/audit-native-bundle.py \
  --bundle "$NEW_APP" --requirements "$REVIEWED_REQUIREMENTS" \
  --output "$NEW_PRIVATE_AUDIT_JSON"
```

## Steps that need a signed Mac build

1. Both arm64 and x86_64 bundles on macOS 14+; inspect actual nested hardened
   runtime/entitlements and deployment targets. Run the resource audit again on
   the signed candidate and launch without build-machine paths/environment.
2. Main-process V8 JIT plus signed Store helper sandbox/denial/confinement, and
   all engine/compiler/provider helper launches. Linux policy doubles do not
   prove these properties.
3. Direct DMG and skipped/valid/invalid TR starts with production/beta identity
   isolation. Verify two-DR handoff ACLs with exact production and beta DRs,
   wrong/untrusted identity rejection, moved apps and future same-DR versions.
4. Real Keychain allow/deny/locked/missing-item paths for all four secret
   families. Preserve legacy ciphertext/item; fresh consent only after denial.
   Verify current sign-out/deletion and never import checkpoint credentials over
   a deliberate sign-out. No reverse secret migration is built.
5. Signed Squirrel/ShipIt-to-native delivery, followed by native first-launch
   final delta after interrupted replacement. Preserve old bundles/caches.
6. Linked Sparkle custom-user-driver callbacks, exact policy enclosure, EdDSA,
   Team/DR, architecture and wrong-payload rejection; ordinary save/quit refusal,
   retry and persisted recovery around extraction/replacement/relaunch. This is
   blocked on the public-API integration change described above.
7. Prolonged native journal edits and reopen/save/retention with unknown fields,
   media and tombstones, plus the fileguard lane's safe refusal of unsupported
   native files by the old app. There is no reverse-profile migration proof or
   way-back feature.
8. Authorized notarization/stapling/Gatekeeper and offline/translocated/read-only
   launches/installations, LaunchServices/URL/TCC continuity and isolated beta
   identity. This lane does not perform or authorize those distribution actions.
9. Physical APFS clone/rename/full-sync, low-space/permissions/power interruption
   tests for saves and installs, both sides of old-bundle acquisition/replacement.

N001–N007 models now follow Opus specifications. Native presentation still needs
application bindings and Mac review. Reverse migration is not built. No UI,
signed-installer or beta release gate is closed.

## Merge preparation and Linux verification

2026-11-04 (America/Chicago). Tests use canonical temporary roots, synthetic data
and injected native capabilities. The two workspace lockfile additions are only
`pm-first-launch` dependency edges to existing locked `pm-distribution` and `sha2`;
no package versions were upgraded. Tests use `--locked`. Experimental store preparation
also owns an exclusive sibling lease; it refuses unrelated stages and linked
paths instead of moving another producer's work. The bundle audit verifies Mach-O
header kinds for every executable/helper slice and refuses silent traversal errors.

Item 13 remains **blocked**, despite mergeable preparation code: no production
asynchronous Sparkle adapter is connected, no shell evidence/import lease wiring
is available inside the owned paths, and the main-reference file-media adapter
and active storageSettings registry import are missing. The signed Mac
steps above remain open. The final branch signal must report BLOCKED, not convert
these missing integrations into beta readiness.

The retention review also removed cleanup/truncation of prior first-launch file
copy/state temporaries and media importer payload/metadata temporaries. Retries
archive the previous attempt before create-new publication. The empty-payload
scratch input remains owned and retained even when import fails. Focused tests
verify that interrupted state metadata, snapshot metadata, payload copies and an
unexpected existing empty-payload input retain their exact prior bytes.


Pre-spec post-merge Linux verification: **167 Rust pass / 0 fail / 0 ignored**
(32 first-launch, 27 migrate, 33 distribution, 75 services), plus **15 Python
bundle-audit tests pass**. Canonical-temp SIGKILL/resume retains the actual kernel
31-second stale-lock grace. Formatting, strict Clippy for touched crates, Python
3.9 syntax parsing for both new scripts, diff whitespace and staged gitleaks pass.
Pre-spec fetched/merged integration base: `ed5482622f41c533199a724dcb8c8738dd4dcc93`
(already an ancestor; no conflicts). The locked test/lint commands:

```sh
cd apps/native
cargo test --locked -p pm-first-launch -p pm-native-migrate -p pm-distribution -p pm-services
cargo fmt -p pm-first-launch -p pm-native-migrate --check
cargo clippy --locked -p pm-first-launch -p pm-native-migrate --all-targets --no-deps -- -D warnings
```

From repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s apps/native/scripts/tests -v
gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow
```

These results certify the tested worker/models and audit tooling only. Item 13,
production Sparkle installation, complete media admission and signed Mac proof
remain blocked as listed above. No workflow was dispatched and no release action
was taken. This was the pre-spec checkpoint; the specification alignment below
supersedes its presentation status.


## 2026-11-04 — Opus specification alignment (ce36cb6fc)

The supplied Opus migration, updates, authorization, tasks/storage, colour and
pro-edit specs now back headless N001–N007 and S111–S113 presentation/action models
under `pm-first-launch::surfaces`. Engine/kernel/media mutations remain with their
owners. These are models, not windowed Mac acceptance. No copied design tokens
or new visual choices are introduced.

- N001: local/unknown owner waiting, count-only determinate/indeterminate steps,
  durable pause boundaries, resume, low disk, safe error/details projection,
  5-second polite progress announcements and silent later media deltas.
- N002: `skip_secret_consent` durably records `Skipped` and completes import with
  ciphertext/checkpoints/partial locator receipts retained. Ordinary launches do
  not prompt again. Both Not Now and Continue Without Them use this API.
  `retry_secret_consent` supports a closed-library worker retry;
  `retry_secret_consent_in_library` uses the already attached kernel actor for
  Settings, with no second writer. Both require a fresh explicit click and capture
  current encrypted inputs so a source sign-out is not resurrected from backup.
- N003: non-zero summary rows, typed report sections/actions, conflict rows that
  keep both versions and offer Show in Finder, blocked-owner relink, proxy-only
  export warning and no How to Go Back action or compatibility promise, per the latest owner
  decision. There is no reverse migration or Restore UI. The former experimental `restore` module was renamed
  `store_preparation`; no external consumer remains.
- N004: explicit stage/matrix/range, 10 Hz playback sampling, hidden suspension,
  previous-result waiting, cleared read/non-finite error state, 8-bit Extended
  refusal, existing token identifiers and LUT state facts.
- N005: task actions/ETA warm-up/retention, zero-total progress, owner-reported
  storage quotas/overage, archive references and sole-proxy exclusion. Changing
  locations/limits deletes or moves no existing data.
- N006: approved Alt chords, J/K/L paused/unavailable fall-through, kernel-owned
  source resolution and atomic refusal copy. No fit-to-fill, inferred duration or
  partial edit is introduced.
- N007: six exact approved usage strings, owning-surface denial routes, no
  automatic retry after Settings, and host refusal for camera/microphone/screen.
  The unused camera/microphone strings were removed from os-base.md as requested.
- Updates: silent automatic checks, cancellable manual download, explicit verify
  failures, persistent per-version banner/skip preferences with unknown fields
  preserved, library notice independent of dismissal, save/export quit barriers
  and deferred once-only What's New marking at actual show time.

Two narrow `DESIGN-TODO(opus)` defaults remain neutral: the initial scope type is
not explicitly selected by the spec (the example shows Waveform), and initial
non-original archive checkbox values are not stated (only Originals is explicitly
off). Models use None until a saved/reviewed choice or the person selects a value.
Token additions in README §7 must come from pm-design; no literal substitute is
provided. Mac visual/AX/keyboard acceptance remains open.

Decisions: explicit decline is durable and never deletes inherited encrypted
inputs (N4); later retry reads current source, respects deletions and uses the
kernel actor (N3/N4/single writer); shipped shortcuts retain fall-through (N6);
OS capture requests fail before any system prompt (Opus N007/N8); supplied
specifications own all copy and interactions. The latest owner decision removes any going-back promise or action. The
`feat/na-cloud-fileguard` lane owns the old app refusing native files.


## 2026-11-04 — owner reference and policy update

Behavior reference is now `origin/main` at
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, per p0-decisions.md and
main-delta.md. v1.2.0 remains a mandatory compatibility input, not the current
parity target. This lane re-read the pinned main updater, builder, store registry,
`main/library-media.ts`, `shared/library-media.ts`, `host/library-media.ts`,
`main/storage-upgrade.ts` and environment storage. It does not certify the other
lanes' main-delta coverage.

- Main updates still gate on packaged macOS, check after 30 s/every four hours,
  preserve normal save/quit, and announce Ready only after Squirrel staging.
  The native updater models preserve that intent; Sparkle remains unlinked.
- Main's media schema is **not** this branch's BLAKE3 host schema: `Media Store/Index`
  records point to flat SHA-256-named payloads or `sourcePath`, with full SHA-256,
  revision, size, MIME and mtime. Its storage upgrade receipt is
  `storage-upgrade-v1.json`. First launch now archives the complete media tree and
  receipt (including interrupted incoming files and unknown fields). It never
  copies that incompatible schema into the active native Media Store.
- The native adapter for main file-media is **not built**. Such a checkpoint
  returns typed `MigrationRequirement::MainFileMediaAdapter`, stable code
  `main_file_media_adapter_required`, before library import or a Complete result.
  The original and sealed checkpoint remain untouched. Later deltas retain their
  pending capture and return the same explicit requirement. They cannot turn
  unavailable native bindings into an "already offline" success.
- `storageSettings` is new in the main registry and absent from the owned kernel
  registry. It remains byte-preserved in the checkpoint. Its active import needs
  the kernel/storage owners; this lane does not add an unrecognized kernel key.
- Source file reads refuse Darwin SF_DATALESS placeholders rather than hydrating
  them. Linux tests exercise the flag policy; both Apple target standard-library
  metadata APIs type-check without linking. Actual provider/TCC/APFS behavior is
  still a Mac check.
- The owner's latest no-way-back decision supersedes migration.md's older footer.
  The report exposes no How to Go Back or Restore action and promises no old-app
  native-file compatibility. `feat/na-cloud-fileguard` owns safe old-app refusal.
  Reverse migration is not built. The owner also requires opt-in Claude client
  registration and Welcome-window animation; no registration or animation side
  effect is implemented by these models.

Selecting rules: preserve every new-reference byte and unknown field (N4), name
an unsupported schema instead of silently treating it as offline (N3/N4/N7),
keep the kernel the writer for new registry entries, and follow owner decisions
where they supersede prior specs. No deployment, feed or workflow action occurs.


The legacy distribution readiness structs still contain `restore_verified`.
This lane does not set it in production or reinterpret it as reverse-migration
proof. The distribution owner must align that contract with actual installer
recovery and the no-reverse product decision. The Sparkle public-API mismatch and
application evidence/import leases remain unresolved outside the owned paths.

## 2026-11-04 — final Opus refresh (78294c4c9)

Merged integration `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9`. The updated
migration spec now provides an informational `About Going Back…` explanation:
Native work does not move back; the original app can open a file only if it can
read every part, otherwise it refuses without changing it. The model projects
that exact explanation with only OK. It offers no reverse operation or guarantee
that a Native file opens in the old app; actual refusal remains the fileguard
lane's responsibility. This supersedes the earlier temporary omission of an
informational footer, without restoring the removed How to Go Back promise.

The updated N005 models include storage-report loading/unavailable/zero-reclaim
states, Older Backups and approved budget choices. Referenced items, protected
newest backups and recovery snapshots stay kept. Rebuildable-cache clearing is
only a typed request to the storage owner, never a deletion in this lane. Source
or authored data always moves to archive, never deletion. Remaining two neutral
Opus defaults (initial scope type and non-original cleanup selections) are
unchanged. No literal token fallback is supplied.
