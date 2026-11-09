# Studio production readiness

**Decision: not ready to ship.** A green renderer demonstration is insufficient.
Every unchecked gate below blocks a production-quality claim. No tag, release,
notarization, upload or installation over an existing app is authorized by this
checklist. Updated 2026-11-09 by Lane P1 (Apple M5, 16 GiB, macOS 27.3, Electron 44); shared branch `studio-3d`.

Status: **PASS** = reproduced against the stated build; **OPEN** = evidence still
required; **FAIL** = attempted and failed; **PARTIAL** = narrower evidence only.
Record the tested commit, hardware, OS, commands and counts when updating a row.
Never add credentials, private projects, screenshots or raw local logs to Git.

## Build and regression gates

Run desktop commands from `apps/desktop`, Rust commands from `engine`. Use a
clean checkout outside File Provider/iCloud with `bun install --frozen-lockfile`.
Unset `ELECTRON_RUN_AS_NODE`; set `CARGO_TARGET_DIR=$HOME/Developer/pm-target`.

| Gate | Status | Evidence / required command |
| --- | --- | --- |
| All desktop unit tests | PASS | Final full run: **7,352 passed, 7 skipped**, 714 files passed / 3 skipped (370.81 s). One additional compatibility-gate regression passes in the targeted follow-up.  `nice -n 10 npx vitest run`; report all files/tests, failures and skips. Targeted tests alone do not pass. |
| Entire desktop e2e suite, including 2D/editor/media | FAIL (running) | Full 1,022-test discovery, no filter: `nice -n 10 npx playwright test --trace off --reporter=line --global-timeout=7200000`. Failures listed below.  `nice -n 10 npx playwright test`; report full discovery, completion and skips. A global timeout is a failure, not a pass. |
| Svelte and renderer types | PASS | 0 errors, 1 existing accessibility warning in `IconCropper.svelte`.  `npx svelte-check --tsconfig ./tsconfig.web.json` |
| Main/preload types | PASS | Generate cloud bindings/types first: `bun run --cwd ../cloud types && bun run --cwd ../cloud types:emit`.  `npx tsc --noEmit -p tsconfig.node.json` |
| Rust workspace, host and GPU tests | OPEN | `nice -n 10 cargo test --release --locked --workspace --features pm-studio-host/oidn --target-dir "$(node ../apps/desktop/scripts/studio-cargo-target.mjs)" -- --test-threads=1`; optional Cycles/reference checks separately, see [yardstick](cycles-yardstick.md). |
| Public-repository secret scan | OPEN | Before **each** commit: inspect `git diff --cached --stat`, `git diff --cached`, then `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`. Also scan final branch with `gitleaks git --redact --ignore-gitleaks-allow`. Keep `.githooks` active. |

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
| Quit, renderer reload, two windows/projects and resize lifecycle | PARTIAL | [Manager tests](../../apps/desktop/src/main/studio-native/manager.test.ts), [IPC tests](../../apps/desktop/src/main/studio-native/ipc.test.ts), [adapter tests](../../apps/desktop/src/renderer/src/core/studio-native/backend.test.ts); packaged process/resize stress still required. |

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
| Host crash, GPU/device loss, hung response | PARTIAL | Timeout rejects all pending requests and portable backend replays scene/view/time: [recovery tests](../../apps/desktop/src/renderer/src/core/studio-native/recovery.test.ts). Manager tests cover crash generations and forced reaping. Physical device-loss injection and packaged recovery remain required. |
| IPC framing and back-pressure | PARTIAL | JSON ≤4 MiB, binary ≤256 MiB, bounded request count and aggregate pending bytes; partial-read and bad-header tests in [protocol](../../engine/hosts/studio-electron/src/protocol.rs) and manager tests. No network command listener; inherited child pipes only. |
| Shared-texture peer and lifetime | PARTIAL | Main authenticates kernel Mach audit PID against its spawned host; addon tests reject another process. Managed VideoFrame/reference release and three-slot host pool; packaged resize/reload stress remains open. |
| File access confined to referenced assets | PARTIAL | Host accepts geometry/texture/FBX **bytes**, never a renderer-supplied filesystem path. FBX external-file loading is disabled in [importer](../../engine/hosts/studio-electron/src/fbx.rs). Main owns executable/addon resolution. Continue review when adding any path-based host operation. |
| GPU memory budget for large scenes | OPEN | Packet/frame/instance/texture limits exist; prove aggregate GPU/CPU allocation behavior and controlled errors against the memory stress gate above. Packet size alone is not a GPU-memory guarantee. |
| Broken model, font or texture isolated to one layer | OPEN | Corrupt/missing model/font/texture fixtures must leave healthy layers visible and editable, with a clear per-layer error; Lane K owns import/font fixes. |
| Save/reopen and undo/redo for every new 3D action | OPEN | Cover model/material/maps, text extrusion, camera/light creation and edits, World/HDRI/fog, terrain/heightmap/regions, scatter/prototype references, parenting, duplicate/remove and motion options. Round trip semantic values and references, not only pixels. |
| Old-project migration | OPEN | Reopen representative pre-Studio and earlier scene3d version projects; retain 2D behavior, assets, timing and undo-safe converted values. Save/reopen again. |
| Logs and privacy | PARTIAL | Host stderr is drained without forwarding raw scene data/user paths to telemetry. Do not add raw exception/scene/asset payloads to telemetry; review future diagnostics for path redaction. |

## Review, UX and documentation

| Gate | Status | Evidence / acceptance |
| --- | --- | --- |
| Native host / main IPC / preload / backend review | PASS for P1 scope | Lane P1 reviewed process ownership, framing/limits, Mach peers, managed references, renderer replay, file access and packaging; regression fixes below. GPU device-loss injection and large-scene/long-run stress remain open robustness gates. |
| Scene schema, evaluator, compositor, Final/export, materials/lighting/environment, assets/import review | OPEN | An accountable reviewer per area, findings resolved, targeted regressions and review SHA recorded. P1's bridge review is not whole-system approval. |
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

## Full-suite failures awaiting follow-up

The full suite is still running. Current failures include agent extension
recovery (timeout), authenticated visual replies (missing iframe), large agent
attachments, idle effect GPU accounting, durable OBJ import, sandbox clipboard,
reference graph editing, group Properties keyboard reveal, 2.5D group switch
expectations, multi-layer value dragging and property reveal. These need their
own reproduction/review; P1 has not labeled them pre-existing or waived them.

Several native World/environment/iris assertions also failed because the
initial full-suite build picked the incompatible shared-cache helper described
above. The isolated build and targeted native rerun must resolve these failures
before treating them as cleared. Do not count WebGPU-rendered successes as
native verification. Extended film exports are part of the full suite and can
take substantially longer than the default 10-minute global test timeout.
