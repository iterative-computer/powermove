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
supply fresh generations, signed identities, state/secret closure and current-state
Restore proof. No production success implementation is provided. The isolated
beta can finish importing while cutover remains blocked. Poll scheduling uses the
existing 30-second/four-hour intervals.

`restore::merge` supports forward and reverse *current-state store* inputs with
missing/null distinction, tombstones and member-level `ext.<id>` merge. Atomic
project/history/workspace values retain both sides in conflict records. The
kernel owner must first flush/compact current journals and capture current state
under its leases. `prepare_store` writes a new rollback-readable store through
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
- Readiness remains false without signed identity/full closure/current Restore
  evidence; passing import models is not release proof (N1/N4).
- Archive failed and duplicate attempts, including partial snapshots (N4).
- Preserve missing versus null and concurrent values; merge extension members,
  keep authored arrays/history atomic (N3/N4/N6).
- Use the kernel writer for a new Restore store; require the owning actor's
  compaction barrier and preserve old inputs (single writer/N4/prune rule).
- Implement exact policy validation over a macOS capability, and report the
  unsupported Sparkle adoption API explicitly (N3/N4/N8). No invented selector,
  permissive installer or synchronous main-thread wait.

`DESIGN-TODO(opus)`: N001–N007 first-launch/update/consent/conflict/Restore
presentation, text, actions, progress and accessibility await specs. Only stable
existing model states/events are supplied. Existing secret-prompt copy remains
provisional; no new copy, spacing, colour, layout or interaction is introduced.

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
   Verify current sign-out/deletion and current-secret reverse Restore; never
   restore checkpoint credentials over a deliberate sign-out.
5. Signed Squirrel/ShipIt-to-native delivery, followed by native first-launch
   final delta after interrupted replacement. Preserve old bundles/caches.
6. Linked Sparkle custom-user-driver callbacks, exact policy enclosure, EdDSA,
   Team/DR, architecture and wrong-payload rejection; ordinary save/quit refusal,
   retry and persisted recovery around extraction/replacement/relaunch. This is
   blocked on the public-API integration change described above.
7. Current full-profile forward/reverse Restore using the actual Electron
   reader/build after prolonged native journal edits, unknown/quarantined files,
   native-only projects/media, settings/extension tombstones and secrets.
8. Authorized notarization/stapling/Gatekeeper and offline/translocated/read-only
   launches/installations, LaunchServices/URL/TCC continuity and isolated beta
   identity. This lane does not perform or authorize those distribution actions.
9. Physical APFS clone/rename/full-sync, low-space/permissions/power interruption
   tests for saves and installs, both sides of old-bundle acquisition/replacement.

N001–N007 native presentation still needs Opus specs and later Mac review. No UI
acceptance, whole-profile Restore, signed installer or beta release gate is closed.

## Merge preparation and Linux verification

2026-11-04 (America/Chicago). Tests use canonical temporary roots, synthetic data
and injected native capabilities. The two workspace lockfile additions are only
`pm-first-launch` dependency edges to existing locked `pm-distribution` and `sha2`;
no package versions were upgraded. Tests use `--locked`. New Restore preparation
also owns an exclusive sibling lease; it refuses unrelated stages and linked
paths instead of moving another producer's work. The bundle audit verifies Mach-O
header kinds for every executable/helper slice and refuses silent traversal errors.

Item 13 remains **blocked**, despite mergeable preparation code: no production
asynchronous Sparkle adapter is connected, no shell evidence/import lease wiring
is available inside the owned paths, and complete current-state profile/media/
secret Restore cannot be certified by the store-preparation tests. The signed Mac
steps above remain open. The final branch signal must report BLOCKED, not convert
these missing integrations into beta readiness.
