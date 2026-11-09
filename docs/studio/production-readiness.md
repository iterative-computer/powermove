# Studio production readiness

**Decision: not ready to ship.** A green renderer demonstration is insufficient.
Every unchecked gate below blocks a production-quality claim. No tag, release,
notarization, upload or installation over an existing app is authorized by this
checklist. Updated 2026-11-09 by Lanes P1/P2/P3 (Apple M5, 16 GiB, macOS 27.3, Electron 44); shared branch `studio-3d`.

Status: **PASS** = reproduced against the stated build; **OPEN** = evidence still
required; **FAIL** = attempted and failed; **PARTIAL** = narrower evidence only.
Record the tested commit, hardware, OS, commands and counts when updating a row.
Never add credentials, private projects, screenshots or raw local logs to Git.

## Build and regression gates

Run desktop commands from `apps/desktop`, Rust commands from `engine`. Use a
clean checkout outside File Provider/iCloud with `bun install --frozen-lockfile`.
Install the browser used by the Figma plugin UI tests with
`nice -n 10 npx playwright install chromium --only-shell` from `apps/desktop`.
Unset `ELECTRON_RUN_AS_NODE`; set `CARGO_TARGET_DIR=$HOME/Developer/pm-target`.

| Gate | Status | Evidence / required command |
| --- | --- | --- |
| All desktop unit tests | PASS | P2 full run on combined code `bca577334`: **7,428 passed, 7 skipped**, 727 files passed / 3 skipped (268.17 s). Includes the P3 engine-facing, history/migration, import and lifecycle regressions. `nice -n 10 npx vitest run`; report all files/tests, failures and skips. Targeted tests alone do not pass. |
| Entire desktop e2e suite, including 2D/editor/media | PASS (24 skipped) | P2 at `2844ef8de`: **1,024 discovered; 1,000 passed, 0 failed, 24 skipped** (48.9 min). Completed with one worker, no filters and no retries, including all fourteen repaired cases, native environment assertions, both film exports and video persistence. `nice -n 10 npx playwright test --trace off --reporter=line --global-timeout=7200000`. The 24 skipped cases remain unexecuted; optional fixtures/benchmarks and the separate acceptance gates below still need their own evidence. |
| Non-film 3D editor regressions | PASS | Lane P3 at `46f2e9861`: **71 passed, zero skipped** (6.0 min), fresh native host, `ELECTRON_RUN_AS_NODE` unset, background mode, temporary user-data profile and optional UI screenshots outside Git. `npx playwright test scene3d studio --grep-invert 'product film' --trace off --reporter=line --global-timeout=1800000`; see [parity reproduction](parity.md#reproduce-the-checks). This is the 3D subset of the full desktop gate above. |
| Svelte and renderer types | PASS | 0 errors, 1 existing accessibility warning in `IconCropper.svelte`.  `npx svelte-check --tsconfig ./tsconfig.web.json` |
| Main/preload types | PASS | Generate cloud bindings/types first: `bun run --cwd ../cloud types && bun run --cwd ../cloud types:emit`.  `npx tsc --noEmit -p tsconfig.node.json` |
| Rust workspace, host and GPU tests | PASS with disk-guard retry | P3 at `46f2e9861`: **100 tests passed** across 99 native/host/GPU tests and the optional 11-scene Cycles test. The latest Cargo invocation stopped only that optional test at its unchanged 8 GB disk guard; after pruning P3's own completed compiler cache, the same compiled test binary passed all 11 scenes (43.90 s). Earlier full invocation at `4368b15db` passed all 100 together. Features `oidn,metalfx,yardstick`; exact command below. This covers `pm-studio` and `pm-studio-host`, the workspace's two packages. The higher-quality full reference yardstick remains separate from this bounded regression, see [yardstick](cycles-yardstick.md). |
| Public-repository secret scan | PASS for tested branch | Staged scans before every commit pass; P2's `gitleaks git --log-opts="--full-history HEAD" --redact --ignore-gitleaks-allow` scans **1,228 commits at tested code `2844ef8de`** with no leaks. No ignore entries changed. Before **each** commit: inspect `git diff --cached --stat`, `git diff --cached`, then `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`. Scan the final tested branch with the same explicit `HEAD` history scope; unrelated local refs are outside this branch validation. Keep `.githooks` active. |

## Real packaged app

Reproduce with `bun run package:studio-local`, then
`node scripts/verify-studio-package.mjs` from `apps/desktop`.
The harness first drives unchanged production fuses through Chromium CDP; only
its disposable Playwright copy enables the Node inspector fuse and is re-signed.
The supplied package retains its fuse policy. Results distinguish both runs.

The acceptance app must come from `electron-builder --dir`, not dev mode or an
`out/main` launch. Keep its profile temporary and use a disposable
`/tmp/.../Applications/Powermove.app`, never the owner's `/Applications` app.

| Gate | Status | Evidence / required command |
| --- | --- | --- |
| Release helper built and bundled with OIDN, TBB and surface addon | PASS | `bun run package:studio-local` builds Cargo `--release --locked -p pm-studio-host --features oidn`; `Contents/Resources/studio/pm-studio-host` must exist. Direct packager calls also rebuild the native runtime in [before-pack](../../apps/desktop/scripts/before-pack.mjs). |
| Consistent hardened signing and entitlements | PASS (local ad-hoc) | Initial ad-hoc hardened package passed `codesign --verify --deep --strict` but its Electron helpers crashed with library-validation Team ID errors. [Local packaging](../../apps/desktop/scripts/package-studio-local.mjs) uses [ad-hoc entitlements](../../apps/desktop/resources/entitlements.mac.adhoc.plist). These are deliberately separate from Developer ID release entitlements. Verified runtime flags/team for the app, host, addon and all four bundled dylibs; no Homebrew absolute load paths or absolute rpaths. [Verifier](../../apps/desktop/scripts/verify-studio-runtime.mjs) runs in local and release packaging. Real Developer ID signing/notarization remains unperformed, requiring separate authorization. |
| Ordinary packaged launch from relocated Applications directory | PASS | Unchanged production fuses, temporary Applications folder and normal visible editor; native child observed.  Launch with `POWERMOVE_BACKGROUND_TEST=0`; ordinary window/GPU preferences; `app.isPackaged === true`; no dev server or host override. |
| Real editor model + extruded text + World + terrain/scatter + lights | PASS | Eight layers; visible model and Arial extrusion, World, seeded terrain/60-instance scatter, area light. Shared-texture transfers observed (5 before explicit capture); `native-studio`.  Use the packaged acceptance harness; assert `PM.Studio.status().backend === 'native-studio'`, native host PID and actual visible nonblank frame. Require shared-texture transfers, not only a backend label. |
| Kill host during unsaved editing and recover | PASS | SIGKILL of the disposable app's native child; complete scene/layer data unchanged, subsequent edit succeeds; valid WebGPU fallback frame.  Kill only the child PID of this disposable app. Compare full scene data before/after, create another edit and capture a valid frame. WebGPU fallback after a crash is permitted and must be reported. No stale host remains. |
| Save, quit and reopen identical scene | PASS | All persisted layers/comps and scene dimensions/timing compare equal (absent `solo` normalized to `false`); reopened scene uses a fresh native host.  Save a synthetic `.pmv`, quit, reopen the same profile/project and compare normalized persistent scene/layer data. Render again; never compare only layer counts. |
| Short Final MP4 export | PASS | 640×360 H.264, 24 fps, 12 frames / 0.5 s; native Final at 16 samples/OIDN. ffprobe and decoded frame inspected. This small acceptance export does not pass the 1080p/64-sample performance gate.  Export through `PM.Export.run`, verify native Final sample/tier evidence, `ffprobe` codec/dimensions/duration/frame count and inspect a decoded frame. Export must occur before intentional fallback or after a native restart. |
| Quit, renderer reload, two windows/projects and resize lifecycle | PASS functional / OPEN stress | Packaged app passes six output-size changes, renderer reload with old host released, two independent native windows, closing one window without changing the other, and quit with no orphan native PID. Long-run/200-cycle memory stress is still open.  [Manager tests](../../apps/desktop/src/main/studio-native/manager.test.ts), [IPC tests](../../apps/desktop/src/main/studio-native/ipc.test.ts), [adapter tests](../../apps/desktop/src/renderer/src/core/studio-native/backend.test.ts); long-run stress remains required. |

Local screenshots, `captions.md`, ffprobe results and acceptance logs belong in
`/tmp/pm-shots/studio3d/laneP1/`, **never committed**. Keep only synthetic evidence;
remove `.app` bundles, build archives and temporary profiles when finished.
Maintain more than 6 GB container free space (`diskutil info /`).

## Performance acceptance: base M5 / 16 GB

**OPEN — later performance lane must measure these gates in the real editor.**
The hidden host harness in [native-host](native-host.md) is useful diagnostic
evidence; it does not pass these gates. Do not silently reduce scatter density,
output size, shutter quality or material features to obtain a pass.

Entry point: `node scripts/verify-studio-package.mjs <Powermove.app>` from
`apps/desktop` ([source](../../apps/desktop/scripts/verify-studio-package.mjs)) establishes the
real executable, ordinary-window profile, backend and shared-texture assertions.
The performance lane should extend that entry with a `--benchmark` mode and
write raw timestamp samples plus a summary JSON outside Git. Existing scene
builders are [the product corpus](../../engine/crates/pm-studio/src/corpus.rs),
[editor environment acceptance](../../apps/desktop/e2e/studio-environments.spec.ts)
and the **default Landscape UI action** in
[environment UI tests](../../apps/desktop/e2e/studio-environment-ui.spec.ts).
The UI action is authoritative for current default scatter counts. The
[hidden transport script](../../apps/desktop/scripts/studio-native-electron.mjs)
may diagnose regressions but must not replace this editor entry point.

Protocol: plugged in, Low Power Mode off, no concurrent builds; record other
active GPU workloads, OS/Electron/host SHA and display refresh. Use 1920×1080
composition, normal fitted viewer at recorded pixel size, default Live quality,
three independent runs per scene after 30 warmup frames. Record cold start
separately. Each run includes 30 seconds continuous camera/object dragging and
30 seconds timeline playback at 60 fps. Include hero, edge, macro, back, chrome,
materials, caustics, wet, and the default Landscape at its **unchanged default
scatter density**, plus a second run at 2× density for stress.

| Metric | Required gate in every normal-density scene | Measurement |
| --- | --- | --- |
| Sustained Live drag and playback | ≥60 presented fps for each 30-second interval; ≤1% frames beyond 33.3 ms | Count completed **editor composite presentations**, not host trace calls or requested rAF ticks. On a 60 Hz display report the observed 59.9/60 Hz ceiling explicitly. |
| Steady frame time | p95 ≤16.7 ms; p99 ≤33.3 ms; no ≥100 ms warm stall | Input/evaluation, scene patch, native transport and editor compositor included; report trace time separately. |
| Input → pixel latency | p95 ≤50 ms; p99 ≤83.3 ms | Timestamp actual pointer/time edits to the first matching composite; pixel completion/readback is a diagnostic bound, not measured display scanout. Validate physical display latency separately if claimed. |
| Refine to clean | ≤2 s product corpus; ≤5 s default Landscape at 1080p | From release of input to 32-sample OIDN frame displayed. Also inspect noise/ghosting; samples alone do not mean clean. |
| Final | ≤1 s/frame products; ≤3 s/frame default Landscape | Native 1080p, 64 samples, OIDN, 8 bounces, production shutter/motion blur when enabled. Report median/p95 over ≥30 advancing frames, cold setup and encoder time separately. |
| Peak memory | Whole disposable app + all helpers ≤6 GiB resident; host ≤3 GiB; no swap growth >256 MiB over run | Sum process tree RSS, record macOS memory pressure/swap before/after, and Metal allocation high-water when available. Unified GPU and RSS accounting overlap: do not add them as independent totals. |
| Long run / resize | ≤10% retained-memory growth after 10 min and 200 resize/scene cycles; no orphan host | Record steady baseline after warmup and after release/GC/idle; check both native process and renderer resources. |
| Over-budget scene | Controlled error/reduced Live quality, UI responsive, project intact; no OS pressure kill | 2× scatter + large imported geometry/textures. Recovery/fallback cannot be counted as native performance success. |

These are release acceptance targets, not current performance claims. A failed
scene blocks the bar; report it individually and retain raw samples.

## Robustness and security

| Gate | Status | Evidence / acceptance |
| --- | --- | --- |
| Host crash, GPU/device loss, hung response | PARTIAL | Timeout rejects all pending requests and portable backend replays scene/view/time: [recovery tests](../../apps/desktop/src/renderer/src/core/studio-native/recovery.test.ts). P3 adds device-generation ownership for shader compilation, target recreation and stale editor-frame eviction; mocked device-loss tests pass in `gpu-lifecycle`, `backend-lifecycle` and `webgpu-backend`. Manager tests cover crash generations and forced reaping. Packaged crash/replay passes; physical device-loss injection remains required. |
| IPC framing and back-pressure | PARTIAL | JSON ≤4 MiB, binary ≤256 MiB, bounded request count and aggregate pending bytes; partial-read and bad-header tests in [protocol](../../engine/hosts/studio-electron/src/protocol.rs) and manager tests. No network command listener; inherited child pipes only. |
| Shared-texture peer and lifetime | PARTIAL | Main authenticates kernel Mach audit PID against its spawned host; addon tests reject another process. Managed VideoFrame/reference release and three-slot host pool; packaged resize/reload/two-window checks pass. Long stress remains open. |
| File access confined to referenced assets | PARTIAL | Host accepts geometry/texture/FBX **bytes**, never a renderer-supplied filesystem path. FBX external-file loading is disabled in [importer](../../engine/hosts/studio-electron/src/fbx.rs). Main owns executable/addon resolution. Continue review when adding any path-based host operation. |
| GPU memory budget for large scenes | PARTIAL | P3 adds preallocation scene/target checks, a conservative 512 MiB geometry allowance and 2 GiB session allowance, inactive native target / portable LRU target eviction, bounded geometry workers and close-time cache release. Existing texture planning lowers texture resolution under its 256 MiB cap. A synthetic 1.1M-triangle native scene fails before BVH/GPU work and preserves the previous scene; portable oversized geometry/target tests pass. Animated scatter overflow is reduced in evaluated copies with a warning. Real-app RSS, large decoded textures, pressure/no-swap and 200-cycle stress remain OPEN. Some full-resolution 4K requests now fail with a controlled budget error; no out-of-core rendering. |
| Broken model, font or texture isolated to one layer | OPEN | Corrupt/missing model/font/texture fixtures must leave healthy layers visible and editable, with a clear per-layer error; Lane K owns import/font fixes. |
| Save/reopen and undo/redo for every new 3D action | PARTIAL | P3's `history-roundtrip` matrix checks 27 actions with exact persistent before/undo/redo equality and evaluated save/reopen equality: World/sun/HDRI, terrain/heightmap/regions, scatter, fog, Material Options, iris/focus, sample policy, parts/finishes, five Shots, exploded view, studio/light helpers, source replacement, duplicate/remove. Multi-view layout/panes now persist through ordinary document history with their own round-trip test. Existing import/text/material/parenting e2e remains required; pass exports are export jobs, not undoable document properties. Full packaged UX acceptance remains OPEN. |
| Old-project migration | PARTIAL | P3 adds tiny synthetic pre-Studio and early-Studio fixtures covering `switch3DVersion`, explicit flat artwork, camera/material/timing values, conversion undo/redo, scatter source references and a second save/reopen. Fixes broken converted source IDs and future-version downgrades. Semantic tests pass; representative old-project pixel/look parity remains OPEN. |
| Logs and privacy | PARTIAL | P3 reviewed new scene3d/Studio/host paths: no telemetry/network send of project data found; the only renderer fetches in those paths load bundled KTX decoder assets. Host stderr is drained without forwarding raw scene data/user paths to telemetry. Host receives scene/FBX bytes; FBX external-file reads are disabled and external glTF resources rejected. Dynamic network-capture privacy verification remains OPEN. Do not add raw exception/scene/asset payloads to telemetry. |

## Review, UX and documentation

| Gate | Status | Evidence / acceptance |
| --- | --- | --- |
| Native host / main IPC / preload / backend review | PASS for P1 scope | Lane P1 reviewed process ownership, framing/limits, Mach peers, managed references, renderer replay, file access and packaging; regression fixes below. GPU device-loss injection and large-scene/long-run stress remain open robustness gates. |
| Scene schema, evaluator, compositor, Final/export, materials/lighting/environment, assets/import review | PARTIAL | P3 reviewed the engine/packed contract, WGSL finite math and traversal bounds, geometry/BVH extraction, GPU/worker ownership, scene edit/history/migration and model-import allocation boundaries; fixes and regressions at `4368b15db` and `46f2e9861` below. Existing seeded-scene/render tests stay green. This is not full compositor/Final visual or whole-system approval; measured main-thread stalls and the remaining acceptance gates still block shipping. |
| UX review | OPEN | Normal packaged app; light/dark, empty/new/open project, native controls, camera navigation, selection/picking, text, World/terrain/scatter defaults, errors/recovery, undo/redo, export and accessibility. See [UI review tests](../../apps/desktop/e2e/studio-ui-review.spec.ts); automated screenshots alone are insufficient. |
| User docs complete | OPEN | [Studio](../../apps/desktop/docs/STUDIO.md), [3D guide](../../apps/desktop/docs/SCENE3D.md), environment creation, supported imports/fonts/textures, Live/Refine/Final expectations, failures/limits and export instructions all agree with shipped behavior. |
| Agent docs and bundled API pack complete | OPEN | [native integration](native-host.md), [parity](parity.md), [environments](environments.md), [Cycles yardstick](cycles-yardstick.md), schema/operations and bundled API pack include every new action, units, limitations and reproducible commands. |

## Lane P1 findings

- Reproduced: edits arriving during a native texture upload were marked sent
  prematurely and skipped. Revisions now describe only the input actually sent;
  a regression test holds the IPC promise while editing geometry/textures.
- Reproduced: pending bytes were unbounded across large requests; release-frame
  messages also bypassed request count limits. Aggregate bytes and a bounded
  three-acknowledgement reserve now constrain retained pipe work.
- Reproduced: a process ignoring SIGTERM and retaining its event loop survived
  disposal. A bounded SIGKILL escalation now reaps it.
- Found in review: a random Mach bootstrap name was the only peer protection.
  Receiver now checks the kernel audit PID; no OS handles/PIDs are page inputs.
- Reproduced: ad-hoc hardened packaging could verify signatures yet fail launch.
  A separate local entitlement policy enables library loading without weakening
  Developer ID release signing. Packaged native acceptance passes.

- Reproduced: failed Mach receives leaked host pool acknowledgements and duplicate
  managed-release callbacks returned a slot twice. All paths now return ownership
  once, with retained handles safe across receiver close.
- Reproduced: a successful build in a shared Cargo target produced 1,088-byte
  materials / 144-byte triangles for a renderer expecting 128 / 96. Native
  initialization correctly declined that binary, but dev e2e runs then used
  WebGPU. Native builds now isolate each checkout under the configured Cargo
  root, verify the executable's real handshake before packaging, and refuse
  discovery of an unrelated shared-cache helper. Signing checks also execute
  this compatibility probe.
- Main now validates page commands before creating sessions, keeps lifecycle
  commands private, bounds native sessions/import workers across all windows,
  and preserves sessions across same-document navigation.

## Lane P3 findings and fixes

Reviewed the `origin/main...origin/studio-3d` engine, WGSL, scene contract,
3D document/edit/agent, import and renderer-lifecycle changes. Fixes below
are in **`4368b15db`**, with the extreme-finite-value follow-up in
**`46f2e9861`**. Severity totals: **9 High, 4 Medium fixed**. No built-in
extension source changed; the ten built-in version checks pass without a bump.

| Area | Finding | Severity | Fix / commit | Regression evidence |
| --- | --- | --- | --- | --- |
| Engine / packed contract | Non-finite UV/lens/material/light data, extreme finite material/light values and overflowing transform inverses could reach GPU work; patches could mutate transforms before rejecting other fields. | High | Validate before expansion/allocation or patch mutation; bound physical values and preserve packed RGBA bit words (`4368b15db`, `46f2e9861`). | Rust validation, scene and packed tests; invalid data leaves the previous scene usable; finite-overflow regression reproduced before the follow-up fix. |
| WGSL shading | Zero interpolated normals and degenerate triangles produced non-finite/black output. | High | Scaled safe normalization, face-normal fallback and finite sample outputs (`4368b15db`). | `zero_normals_and_degenerate_triangles_keep_healthy_pixels_lit`, hardware and portable traversal. |
| BVH / traversal | Skewed SAH trees could exceed fixed shader stacks; binary depth acceptance exceeded the actual stack. | High | Cap generated depth and enforce BLAS/TLAS stack-safe binary depths (`4368b15db`). | Rust and TS skewed-tree tests retain all triangles; host rejects deep/cyclic/out-of-range trees and accepts empty scenes. |
| GPU / allocation | Scene and multi-tier target allocations lacked an aggregate preallocation allowance. | High | 512 MiB geometry / 2 GiB session checks, per-binding checks and target eviction; retain texture-resolution planning (`4368b15db`). | Native 1.1M-triangle rejection before BVH/GPU work; portable oversized geometry/targets and eviction tests. |
| Workers / close | Idle worker handlers retained completed results; queued inputs/workers survived project close, and failed workers could trigger a large synchronous retry. | High | Bound retained work, time out/reap workers, clear completed handlers, cancel on close and isolate failed layers (`4368b15db`). | Worker lifecycle/transfer-failure tests and healthy-layer oversized-scene isolation. |
| Device recovery | Old asynchronous shader results/errors could poison a replacement device; the editor could remain disabled or show stale completed frames after loss. | High | Generation-owned compilation, retry initialization, recreate targets and discard stale frames (`4368b15db`). | `gpu-lifecycle`, `backend-lifecycle`, `webgpu-backend`; physical loss remains open. |
| Document / agent edits | Individually valid operations could overflow composition-wide budgets; animated count/range/resolution combinations could invalidate the evaluated scene. | High | Atomic aggregate edit checks; bounded evaluated scatter, crossed-range clamp and heightmap resampling, with saved data unchanged (`4368b15db`). | 37 history/round-trip/invalid-input tests, including aggregate rejection and animated evaluation. |
| Model import | Tiny sparse/bufferless glTF accessors could request huge decoded arrays before loader validation. | High | Check decoded accessor bytes/types before GLTFLoader; clear missing-scene error (`4368b15db`). | `import-model` sparse-allocation regression. |
| Migration | Preserving a converted container ID left scatter/model references pointing at a removed generated ID; future switch versions were downgraded. | High | Remap converted references and respect versions ≥2 (`4368b15db`). | Four synthetic migration-fixture tests, exact conversion undo/redo and save/reopen. |
| Imported finishes | Applying a finish left source-material mode active, so the model's appearance did not change. | Medium | Explicitly select the finish override (`4368b15db`). | Real runtime material changes, then exact Undo/Redo restores source/override. |
| Render passes | Pass image and live-composition maps used fixed 1024 dimensions despite a different planned texture size. | Medium | Upload/resample at the scene's planned map size (`4368b15db`). | Two pass texture-size regressions. |
| Multi-view | Layout, pane selection and pane views existed only in transient maps and lacked save/history behavior. | Medium | Validated per-composition data through the ordinary edit/history boundary; release prior-project maps (`4368b15db`). | Layout/pane/view exact Undo/Redo and reopen; malformed view edits reject atomically. |
| Mesh / math utilities | Malformed OBJ indices could panic, relative tuple caching reused the wrong vertex, short/Unicode colors could panic, and extreme normalization could become non-finite. | Medium | Validate indices, resolve relative indices before caching, safe color parsing and scaled normalization (`4368b15db`). | Rust OBJ, color and finite-normalization regressions. |

P3 validation uses this checkout's isolated Cargo target under
`$HOME/Developer/pm-target/studio/`, two build jobs, `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_RELEASE_DEBUG=0` and `nice -n 10` to fit the shared machine.
Native validation enables **`oidn,metalfx,yardstick`** and passes
**100 tests**, including 25 host tests, GPU tests and all 11 Cycles scenes.
The final revision uses the disk-guard retry described above; the guard and
test thresholds were unchanged. Compiler-cache cleanup was confined to this
checkout's isolated target after native builds/tests had finished.
Command from the root:

```sh
CARGO_TARGET_DIR="$HOME/Developer/pm-target" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 CARGO_PROFILE_RELEASE_DEBUG=0 PM_STUDIO_YARDSTICK_OUT=/tmp/pm-p3-cycles nice -n 10 "$HOME/.cargo/bin/cargo" test --manifest-path engine/Cargo.toml --target-dir "$(node apps/desktop/scripts/studio-cargo-target.mjs)" --release --locked --no-fail-fast -p pm-studio -p pm-studio-host --features oidn,metalfx,yardstick -- --test-threads=1
```

### P3 remaining acceptance work

These are **3 High and 2 Medium open gates**, distinct from the fixed defects
above. They are not waived by green functional tests.

| Area | Severity | What remains |
| --- | --- | --- |
| Main-thread responsiveness | High | The latest 1,024,000-triangle editor run measured a 119 ms preparation task (first draw 1 ms, ready after 1,204 ms; earlier runs 154–174 ms). Its existing 400 ms assertion is weaker than the production bar. Terrain/scatter generation and skinned evaluation still contain synchronous work; the existing skin benchmark is about 20.5 ms median / 27.8 ms p95. Move/profile bulk preparation and pose work off-thread, then measure against the 16 ms bar. No e2e threshold was weakened. |
| Resident memory / long-run lifecycle | High | Prove whole-app ≤6 GiB, host ≤3 GiB, bounded swap and ≤10% retained growth over 10 minutes / 200 resize and project cycles, including oversized geometry and decoded textures. Allocation estimates and worker/cache disposal tests do not establish these measurements. |
| Physical device loss | High | Inject native GPU/device loss and verify a usable frame plus preserved unsaved edits. Mock WebGPU loss and native process termination cover narrower failure modes. |
| Old-project appearance | Medium | Compare representative pre-Studio and early-Studio rendered looks, not only semantic data, camera, references and timing. Synthetic fixtures do not establish pixel parity between renderers. |
| Privacy instrumentation | Medium | Capture outbound traffic while opening/importing/editing synthetic projects to supplement source review; no project-data telemetry path was found in the audited code. |

The full desktop e2e result is recorded separately by Lane P2 below. P3 does not
claim full-suite, packaged UX or production-release approval. Local profiles,
logs, Cycles outputs and optional UI screenshots stay under `/tmp/pm-p3-*`,
outside Git.

## Lane P2 full-suite failure audit

Baseline: `origin/main` at `d7e9f9205`, checked out independently outside iCloud,
installed with `bun install --frozen-lockfile`. Running the thirteen named spec
files discovered **34 tests: 21 passed, 13 failed** (3.1 min). All fourteen
reported cases were included; the durable OBJ case passed on main. The baseline
checkout and its dependencies were removed after classification.

The unchanged Studio cases were also reproduced at `bcd623b3e`: the ten
non-agent spec files produced **11 passed, 11 failed** (2.1 min), matching P1's
repeatable failures. P1's full run and isolated retry provide the three agent
reproductions. No case is classified as environmental without repeated evidence.

Case paths are relative to `apps/desktop/e2e`.

| Reported case | Classification | Correction / retained assertion |
| --- | --- | --- |
| `agent-extension-recovery.spec.ts` | Pre-existing on main | Replace the obsolete `codex exec` fixture with App Server thread/turn replies. Still checks isolated staging, promotion, recorded change set and Undo. |
| `agent-visual-replies.spec.ts` (authenticated MCP) | Pre-existing on main | Provider fixture reads run-scoped MCP configuration from App Server and publishes through the real authenticated tool session. |
| `chatgpt-connection.spec.ts` (large attachments) | Pre-existing on main | Use the current App Server protocol; retain exact image/file byte checks across renderer/main IPC. |
| `effects-idle-cost.spec.ts` | Pre-existing on main | Account for the current 47 bundled effects, rather than the old 43; retain every GPU allocation, draw, cache and memory assertion. |
| `extension-layers.spec.ts` (durable OBJ) | Studio regression | Select the imported model before committing the first-model studio transaction. Selection otherwise created an extra Undo step after the model/camera/lights edit. Real-history regression test checks one-step Undo, asset retention and Redo. |
| `graph-reference.spec.ts` | Pre-existing on main | Enable Separate Dimensions explicitly for independent X/Y curve editing. |
| `group-transforms.spec.ts` (Properties) | Pre-existing on main | Leave text-entry focus before the shortcut and assert the default joined Position row. |
| `groups-25d.spec.ts` | Pre-existing on main | Assert explicit descendant 3D switches, as the group switch intentionally sets its members (`ef414bbb2`). Pixel changes, animation, picking, masks and Undo remain checked. |
| `property-animation.spec.ts` (multi-layer drag) | Pre-existing on main | Enable Separate Dimensions for the Y-only gesture; retain both layer values and one-step Undo. |
| `property-reveal.spec.ts` (shortcuts) | Pre-existing on main | Assert joined Position by default (`bff2ccd70`); retain Shift toggles, global disclosure and text-input isolation. |
| `timeline-keyframe-drag.spec.ts` (Position Y) | Pre-existing on main | Enable Separate Dimensions for the scalar Y row; retain key values, times and Undo checks. |
| `timeline-keyframe-navigation.spec.ts` (shortcuts) | Pre-existing on main | Enable Separate Dimensions for axis-specific navigation; retain exact times and input isolation. |
| `timeline-keyframe-navigation.spec.ts` (deselection/deletion) | Pre-existing on main | Enable Separate Dimensions for Y-row selection; retain evaluated-value preservation and Undo. |
| `timeline-track-values.spec.ts` (values) | Pre-existing on main | Enable Separate Dimensions for X-only editing; retain typing, scrubbing, Escape, locks and Undo. |

Position is intentionally one timeline row by default; axis-specific tests now
request the supported Separate Dimensions mode instead of silently looking up
an absent row. The updated fixtures retain the existing interaction assertions
and the product's default behavior.

The incompatible shared-cache native helper found by P1 remains fixed: native
builds use this checkout's isolated Cargo target and verify the real executable
handshake. Full-suite validation must rebuild that release helper before launch.
P2's targeted rerun passed **all 34 tests** (1.0 min), including all fourteen
reported cases. An initial full attempt was interrupted after four Figma UI
cases could not launch an absent Playwright Chromium executable. The browser
was installed before restarting the full suite; no product/test assertion was
changed for that environment setup failure; all nine Figma link/plugin cases
then passed (22.1 s). A second full attempt passed 803 cases before it was
stopped to integrate Lane K's new native renderer/import/font changes. The
first complete combined-branch run at `be13c9731` finished with **999 passed,
1 failed, 24 skipped** (50.4 min), including native environment assertions, both
film exports and all fourteen repaired cases. The extra failure was a video
restoration preview-seek wait; the unchanged case passed all three isolated
repeats (1.1 min). Its persistence check now awaits the actual capture decoder
and asserts a prepared source frame before retaining the existing byte, pixel
and incremental-save checks. All five media-persistence cases passed three
repeated runs after the change: **15 passed** (32.3 s).

The final unfiltered run at **`2844ef8de`** includes the subsequent Lane K/P3
hardening commits and the deterministic video wait: **1,024 discovered,
1,000 passed, 0 failed, 24 skipped** (48.9 min). Both product films completed;
the extended film took 17.9 min. All fourteen original blockers are resolved.
The 24 skipped cases remain unexecuted, and the separate performance,
physical-device-loss, long-run, packaged UX and documentation gates above
remain open; this result does not establish production-release readiness.

P2 reproduction from `apps/desktop` after the frozen install and browser setup:

```sh
unset ELECTRON_RUN_AS_NODE
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1
export POWERMOVE_BACKGROUND_TEST=1
export POWERMOVE_USER_DATA="$(mktemp -d)"
nice -n 10 node scripts/build-studio-native.mjs
nice -n 10 npx playwright test --trace off --reporter=line --global-timeout=7200000
```

The release helper was rebuilt with OIDN in this checkout's isolated Cargo
target and its copied runtime verified before the final run. The suite used
temporary profiles. Raw logs, captures and output media remain outside Git.


## Verified code and retained evidence

Packaged acceptance tested code commit `13f69c6f2ae4bad388f6a7f4971f7e359df1ed42`
from a clean working tree. `packaged-results.json` outside Git records that SHA,
app.asar/host hashes, source cleanliness, the unmodified-fuse smoke run and the
separate inspector-copy assertions. Evidence includes eight scene/window PNGs,
`captions.md`, animated `final.mp4` (12 distinct decoded frames), `ffprobe.json`
and local suite summaries. The final frame was visually inspected: model,
extruded “Studio”, seeded terrain/scatter, sky and lighting are present.
Temporary profiles/app copies and the packaged `dist/` build were removed.

The final packaged rerun caught and corrected an over-broad finite-data check:
`GpuTri.n[*].w` holds packed RGBA8 **bits**, including values that look like NaN
as floats. Validation now permits those words while still rejecting non-finite
normal xyz, UVs and tangents. A regression test reproduces the valid-white-colour
case and invalid shading coordinates. A successful native smoke run also
confirms the corrected 128-byte material / 96-byte triangle layout and OIDN.

The Python host smoke and FBX measurement defaults now use this checkout's
verified `resources/studio` helper rather than an unrelated shared Cargo binary.
