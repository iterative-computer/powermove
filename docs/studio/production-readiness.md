# Studio production readiness

**Decision: not ready to ship.** A green renderer demonstration is insufficient.
Every unchecked gate below blocks a production-quality claim. No tag, release,
notarization, upload or installation over an existing app is authorized by this
checklist. Updated 2026-11-10 by Lanes P1/P2/P3/P4/P5b/M (Apple M5, 16 GiB, macOS 27.3, Electron 44); shared branch `studio-3d`.

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
| All desktop unit tests | PASS | Lane M at code `c1d40e8ab`: **7,563 passed, 7 skipped**, 743 files passed / 3 skipped (416.32 s). Includes all prior regressions plus material allocation/sharing/caching, group dependencies, importer alpha, geometry bounds, null controls and composition clocks. `nice -n 10 npx vitest run`; report all files/tests, failures and skips. Targeted tests alone do not pass. |
| Entire desktop e2e suite, including 2D/editor/media | PASS (24 skipped) | P2 at `2844ef8de`: **1,024 discovered; 1,000 passed, 0 failed, 24 skipped** (48.9 min). Completed with one worker, no filters and no retries, including all fourteen repaired cases, native environment assertions, both film exports and video persistence. `nice -n 10 npx playwright test --trace off --reporter=line --global-timeout=7200000`. The 24 skipped cases remain unexecuted; optional fixtures/benchmarks and the separate acceptance gates below still need their own evidence. |
| Non-film 3D editor regressions | PASS, intermittent Final equality issue OPEN | Lane M at code `c1d40e8ab`: **84 discovered, 82 passed, 0 failed, 2 skipped** (9.4 min), rebuilt native host, `ELECTRON_RUN_AS_NODE` unset, background mode and temporary user-data profiles. `nice -n 10 npx playwright test scene3d studio --grep-invert 'product film' --trace off --reporter=line --global-timeout=1800000`, with captures outside Git. Skips are the explicit-project DN2 capture and opt-in progress film. Includes new group-material effects/alpha and real-camera bounds-null checks alongside native transport, GC lifecycle, motion blur and exact Final output. The pre-existing Final equality intermittency reproduced on the clean baseline; see Lane M evidence below. The entire 2D/editor/media suite was not rerun in Lane M. |
| Svelte and renderer types | PASS | 0 errors, 1 existing accessibility warning in `IconCropper.svelte`.  `npx svelte-check --tsconfig ./tsconfig.web.json` |
| Main/preload types | PASS | Generate cloud bindings/types first: `bun run --cwd ../cloud types && bun run --cwd ../cloud types:emit`.  `npx tsc --noEmit -p tsconfig.node.json` |
| Rust workspace, host and GPU tests | PASS | Lane M at code `c1d40e8ab`: **115 passed, zero failed or ignored**, `nice -n 10 cargo test -p pm-studio -p pm-studio-host --features oidn,metalfx`, with `CARGO_TARGET_DIR=~/Developer/pm-target`. Includes all 11 bounded Cycles scenes as one test, reused-versus-fresh OIDN output, resize, specular-distance guide rebinding, cancellation and recovery. This covers `pm-studio` and `pm-studio-host`, the workspace's two packages. The higher-quality full reference yardstick remains separate from this bounded regression, see [yardstick](cycles-yardstick.md). |
| Public-repository secret scan | PASS for tested branch | Lane M's `gitleaks git --log-opts="--full-history HEAD" --redact --ignore-gitleaks-allow` scans **1,257 commits at code `c1d40e8ab`** with no leaks. Staged scans before every commit pass; no ignore entries changed. Before **each** commit: inspect `git diff --cached --stat`, `git diff --cached`, then `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`. Scan the final tested branch with the same explicit `HEAD` history scope; unrelated local refs are outside this branch validation. Keep `.githooks` active. |

## Lane M material and bounds parity (2026-11-10)

This pass ports the independent #187/#188 material, bounds-null and navigation
capabilities to Studio. Its functional checks do not waive any performance,
packaged-app, visual-review or release gate below. The release decision remains
**not ready to ship**.

| Audit pitfall | Baseline exposure | Change / retained proof |
| --- | --- | --- |
| Capture allocation at extreme camera/4K demand | The fixed 1024 cap avoided giant captures but lost closeup detail | Camera/UV demand now fits 4,194,304 pixels and preview working bytes (64/pixel); direct oversized capture rejects before targets; intermediate artwork/SVG density is also bounded. `material-parity`, `material-resolution`, `compositor`. |
| Disposed shared texture left in scalar/normal slots | Baseline duplicated bindings; introducing sharing creates this risk | Replace every supported map slot and saved loading binding before disposal; regression explicitly places the old texture in Normal and Roughness as well as Color/Emissive. `material-parity`. |
| Group bounds change with active children | Baseline had no group-source support | Duration-union canvas retains origin and size across entrances/exits and animated motion; edit invalidation retained. `material-source-bounds`. |
| Group capture recursively renders 3D descendants | Baseline rejected groups entirely | New group eligibility rejects nested 3D descendants before capture, including nested precomps. `material-parity`. |
| SVG/Figma alpha becomes white/opaque extrusion | Reproduced | Split RGB and alpha in import and extrusion; preserve independent paint opacity. `material-parity`, Figma `convert`, `extrusion`. |
| Inflated path bounds alter pivot placement | Reproduced | Visual half-width and actual joins/caps are separate from raster antialiasing margin; bounds can be measured without allocating a path canvas. `material-parity`, `geometry-bounds`. |

Static caching observes source dependency edits, decoded image identity, fonts,
matte/effect programs and timing boundaries, while retaining pixels for
receiver-only edits. Texture detail uses the final render camera before Studio
scene extraction; replacement advances the runtime revision. Bounds-null tests
cover hierarchy, animation, morphs/skin, all nine positions and Undo/reopen,
including real-camera picking/orbit for models, extrusions and standalone 3D
planes. The existing heavy-composition pan-cost regression remains enforced.
Navigation tests cover precomp offset/trim/speed/remapping and instance context.
See [parity and merge reconciliation](parity.md#material-source-parity-with-main-reviews-187--188).

Validation at code `c1d40e8abc7de4ee9c83ee422f958a717d74ddca` used the
hardware/OS and commands above: full Vitest **7,563 passed / 7 skipped**;
Rust **115 passed / 0 ignored**; final non-film 3D **82 passed / 2 skipped**;
desktop typecheck **0 errors / 1 existing IconCropper accessibility warning**;
all **10 built-in version checks passed**. The focused group-material and
null-control editor cases also passed **14/14** across two repeats. Layers 3D,
Timeline and Viewer manifest versions were bumped and their hashes updated.

An earlier full 3D run produced **81 passed, 1 failed, 2 skipped**, at the
unchanged `studio.spec.ts` Final denoising byte-equality assertion. It passed
two of three isolated repeats on the Lane M tree. A clean pre-lane checkout
at `60b500850` reproduced the identical assertion failure: **4 passed,
1 failed** in five repeats. The final full Lane M run passed without changing
that test or relaxing its equality/quality assertions; the pre-existing
intermittency remains **OPEN**. The two opt-in skipped cases remain unexecuted.
No production benchmark is claimed during concurrent Lane Q rendering.

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

**Lane P4 measures these gates in the real editor; failed gates remain blocking.**
The hidden host harness in [native-host](native-host.md) is useful diagnostic
evidence; it does not pass these gates. Do not silently reduce scatter density,
output size, shutter quality or material features to obtain a pass.

Entry point: `node scripts/verify-studio-package.mjs <Powermove.app>` from
`apps/desktop` ([source](../../apps/desktop/scripts/verify-studio-package.mjs)) establishes the
real executable, ordinary-window profile, backend and shared-texture assertions.
The `--benchmark` mode drives a normal visible editor window and writes raw
timestamp samples plus a summary JSON outside Git. Existing scene
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
scatter density**, plus a second run at 2× density for stress. Also measure a
room interior, a street with many lights, and the four-view layout. The
four-view workload animates the product as well as the camera; its drag phase
also plays the timeline so every pane changes. Each pane must meet the frame
gate; updates to different panes cannot be added together to claim 60 fps.
Label earlier camera-only four-view results separately, since its unchanged
orthographic panes can legitimately reuse cached pixels.

From `apps/desktop`, after building the host and editor:

```sh
PM_STUDIO_SHOTS=/tmp/pm-shots/studio3d/laneP4/acceptance node scripts/verify-studio-package.mjs --benchmark --dev --stress-minutes 30
```

Omit `--dev` and supply a `.app` path to measure a disposable packaged copy.
The harness enables the inspector only in that disposable copy. Dev mode
uses the built editor and bundled native host, with no development server.
Both modes assert `native-studio` for every pane and real shared-texture
transfers. `--diagnostic` permits concurrent workloads and cannot establish a
quiet-machine acceptance result. Memory stress uses a fixed five-minute
warmup followed by the requested measurement duration; `--stress-minutes 30`
therefore runs for at least 35 minutes. Warmup samples are retained separately.
`--stress-warmup-minutes` permits short harness checks; shortening the measured
30-minute period does not pass the retention gate. `--cpu-profile` writes editor CPU profiles.
Default stress opens fresh file identities and retains their saved-project
catalog entries. `--stress-fixed-projects` instead reopens the same two saved
projects through the ordinary editor project-opening API and retains their
Undo histories. Report both workloads distinctly: growing persistent content
must not be mistaken for a fixed working set, and deleting saved projects or
clearing Undo between cycles would hide resource retention.
`--scenes`, `--seconds`, `--runs` and `--final-frames` support reproductions;
shortened runs do not satisfy the full gates.

Product, room and street geometry enters through the normal GLB import
pipeline. The harness reverses import normalization with ordinary layer
transforms and installs the corpus camera, materials and lights. Imported
glTF material/light representations are representative editor scenes, not a
replacement for the canonical native/Cycles image regression. Default
Landscape comes from the real Add ▸ Terrain action: currently **18,620**
scatter candidates, including **18,000** grass candidates. The separate
100,000-tree/54,000-ground-cover forest fixture must not be described as that
UI default.

| Metric | Required gate in every normal-density scene | Measurement |
| --- | --- | --- |
| Sustained Live drag and playback | ≥60 presented fps for each 30-second interval; ≤1% frames beyond 33.3 ms | Count completed **editor composite presentations**, not host trace calls or requested rAF ticks. On a 60 Hz display report the observed 59.9/60 Hz ceiling explicitly. |
| Steady frame time | p95 ≤16.7 ms; p99 ≤33.3 ms; no ≥100 ms warm stall | Input/evaluation, scene patch, native transport and editor compositor included; report trace time separately. |
| Input → pixel latency | p95 ≤50 ms; p99 ≤83.3 ms | Timestamp actual pointer/time edits to the first matching composite; pixel completion/readback is a diagnostic bound, not measured display scanout. Validate physical display latency separately if claimed. |
| Refine to clean | ≤2 s product corpus; ≤5 s default Landscape at 1080p | From release of input to 32-sample OIDN frame displayed. Also inspect noise/ghosting; samples alone do not mean clean. |
| Final | ≤1 s/frame products; ≤3 s/frame default Landscape | Native 1080p, 64 samples, OIDN, 8 bounces, production shutter/motion blur when enabled. Report median/p95 over ≥30 advancing frames, cold setup and encoder time separately. |
| Peak memory | Whole disposable app + all helpers ≤6 GiB resident; host ≤3 GiB; no swap growth >256 MiB over run | Sum process tree RSS, record macOS memory pressure/swap before/after, and Metal allocation high-water when available. Unified GPU and RSS accounting overlap: do not add them as independent totals. |
| Long run / resize | ≤10% retained-memory growth and no increasing trend over 30 measured minutes and ≥200 resize/scene cycles after a fixed 5-minute warmup; no orphan host | Record the same closed-project phase after release/GC/idle; check both native process and renderer resources. A shorter run is diagnostic only. |
| Over-budget scene | Controlled error/reduced Live quality, UI responsive, project intact; no OS pressure kill | 2× scatter + large imported geometry/textures. Recovery/fallback cannot be counted as native performance success. |

These are release acceptance targets, not current performance claims. A failed
scene blocks the bar; report it individually and retain raw samples.

<!-- lane-p5b-measurements:start -->
### Row (c): P5b resource and responsiveness repairs

**FAIL — production performance acceptance remains required.** The resource
regressions below pass; they do not establish the 30-minute retention, latency,
frame-rate or whole-editor main-thread gates. The P4 table below is the quiet
baseline. Loaded-machine diagnostics are kept separate from acceptance.

P5b reproduced the detached inspector leak in paired heap snapshots. An
extension activation retained an array of every disposed subscription. A
retired callback retained its component closure and detached inspector tree.
The registry now keeps only live releases; disposing a handle removes its
entry and clears its resource reference. Retaining the returned handle no
longer retains the callback. Closed projects also evict their encoded document,
Undo, state and journal payloads after a successful durable flush. Failed or
newer writes stay resident; reopening reads the saved document lazily.

| Focused leak proof, 40 real select/deselect cycles | Before P5b | After repair | Status |
| --- | --- | --- | --- |
| Retired inspector controls still alive after GC | 40 / 40 | 0 / 40 | PASS |
| DOM counters, initial → closed-selection phase | 4,087 → 36,286 | 4,087 → 4,566 | PASS for this reproduction |
| Listener counters, initial → final | 999 → 2,284 | 999 → 1,004, stable after cycle 10 | PASS for this reproduction |
| Detached HTML elements in paired terminal heaps | 5,830 | 133 | Diagnostic; includes unrelated browser/UI retention |

The real-editor regression retains 100 disposed subscription handles and
selects/deselects twelve actual 3D inspector controls. All weak references must
clear while the editor remains open. It failed with all 100 callbacks and all
12 controls retained before the fix, and passes afterward. The forty-cycle
snapshot comparison uses locators rather than retained automation element
handles. Heap detection matches Chromium's native serialized element names
and detachedness flag; snapshots were summarised and deleted outside Git.

Other repairs remove the intermediate Canvas2D copy from native VideoFrame
composition, keep one completed surface per layer across quality tiers, and
close replaced, cancelled, buffered and late frames once. A newly completed
main-view image can present on the next display tick within the same project
frame. Every side pane polls on the display clock, rather than a 48 ms
round-robin timer. Side cameras keep their initial framing during animation;
all panes use the same quantised playback time. Camera pose notifications no
longer trigger pane layout when arrangement and labels are unchanged.

Adaptive Live resolution retains the renderer, acceleration structures and
MetalFX guide/pack pipelines. Input textures and the specular-distance binding
follow the new size. A GPU regression writes known guide values, shrinks and
grows the target, then reads the distance texture back; it reproduced stale
bindings after integration with DN2 and now passes. Slow quality images and
long pauses no longer expire the currently requested native session.

Cold-setup profiles identified synchronous thumbnail `readPixels`/`toBlob`
work and repeated finish-sphere BVH builds. Composition thumbnails now use
asynchronous GPU readback; saved-project thumbnails downsample the displayed
canvas to at most 320×320 on the GPU, then wait for completion before encoding.
Both paths release cancelled captures. Finish swatches share sphere geometry
and run one at a time during idle turns, yielding to edits, playback and export.
Swatches map their completed GPU buffer asynchronously before CPU encoding;
profiling had found 80–110 ms tasks waiting for GPU-backed canvas PNG encoding.
The responsiveness observer retains cold, warmup and idle tasks through the
displayed Refine image, so interval resets cannot hide this work.

DN2's integrated Final policy raises the sample floor. New timing reports must
state actual native samples; a requested legacy value of 64 is no longer an
exact 64-sample full-frame render. Do not compare it directly with P4's
64-sample Final timings or lower the quality policy to pass a timing gate.

The million-triangle check records no observed ≥50 ms task during background
preparation, but its synchronous initial draw took 118 ms in the loaded full
suite; Studio became ready 1,062 ms later. In the final suite, the initial draw
was 1 ms and readiness 1,099 ms. DN2 activity varied during these functional
runs; they do not establish quiet cold-setup acceptance or pass the
whole-editor cold-setup gate. An intermittent portable Final byte-equality check failed
once under load and passed three immediate repeats plus the final full suite.
Its cause remains OPEN; no image-difference threshold or assertion was relaxed.

| Gate | Before (quiet P4 unless stated) | P5b evidence so far | Status |
| --- | --- | --- | --- |
| Disposed inspector controls retained after GC | 40 / 40 in the focused P5b reproduction | 0 / 40; callbacks and real controls collectible in the editor regression | PASS for the reproduced leak |
| 30-minute retained memory | +59.8% over 1,152 measured cycles | 30.10 measured minutes / 506 cycles under contention; RSS +9.89%, slope -0.78 MiB/min | FAIL; quiet protocol not completed |
| Input → completed image, p95 | Worst case 59.8 ms | Early loaded macro diagnostic: 60.9 ms drag / 86.1 ms playback; no quiet final-code result yet | FAIL; loaded diagnostic only |
| Product playback | 47.4–59.4 fps; p95 17.2–33.4 ms | Early loaded macro diagnostic: 52.6 fps / p95 32.0 ms; final-code quiet repeat pending | FAIL; full matrix open |
| Main-thread tasks | Cold setup 186 ms; stress 84 ms | Specific thumbnail/swatch waits removed; long diagnostic editing task max 167 ms / p95 100 ms under contention | FAIL; quiet cold and stress repeats required |
| Refine at 1080p | 1.40–3.16 s | Quiet final-code repeat pending | FAIL; acceptance not revalidated |
| Final at 1080p | 1.72–4.81 s at 64 samples | DN2 quality floor preserved; no comparable quiet 30-frame measurement | FAIL; quality and protocol differ |

The final short fixed-project diagnostic completed 40 cycles (15 warmup,
25 measured) with both saved projects and Undo retained. Over 2.11 measured
minutes, listeners stayed at 1,010; DOM counters were 4,812→4,840 and JS heap
30.28→30.88 MiB. Closed-project RSS was 958.9→1,003.2 MiB (+4.62%; fitted
slope +32.6 MiB/min, three checkpoints), so this short trace does **not** prove
flat whole-app memory. Peak app RSS was 1,906.9 MiB, host 341.8 MiB; swap fell
328.2 MiB and no host was orphaned. Its measured editing tasks reached 156 ms
(p95 147 ms), so the main-thread gate also remains FAIL. DN2's editor tests
were active throughout; these results are diagnostic, not acceptance. The
strict 30-minute launch at 02:45 CST correctly refused that competing workload.

The final growing-catalog diagnostic completed **30.10 measured minutes,
506 measured cycles**, after five minutes / 72 warmup cycles.
Closed-project RSS averaged 937.9→1030.6 MiB
(first/last three checkpoints), +9.89%, with fitted slope
-0.78 MiB/min across 50 closed-project checkpoints. DOM counters
were 4,966→4,965; listeners
1,037→1,037;
JS heap 31.95→38.59 MiB.
The growing catalog is retained; no saved projects or Undo were cleared.
Peak whole-app RSS was 2292.9 MiB, host 341.8 MiB;
system swap changed -736.2 MiB. No host was orphaned.
Measured main-thread tasks reached 167 ms (p95 100 ms).

**This is diagnostic evidence, not a quiet-memory PASS.** The strict launch
was initially quiet at 02:47 CST but stopped after 15 warmup cycles when DN2
started another suite. The replacement diagnostic began at 02:49 CST; other
lanes' tests remained active during portions of its measured period. Only
625 of 901 machine polls were quiet.
The full quiet 30-minute retention protocol and quiet final-code performance
matrix remain required. Controller/docs commit is `0916fe9de`; runtime and
native binary are the tested P5b build specified in this section.

Paired long-run heaps contain 655,653→763,731 total nodes and
159→180 detached HTML elements. This includes
warmup, JIT and legitimate catalog content; the focused weak-reference
regression remains the causal proof of inspector collection. Both snapshots
were summarised and deleted outside Git.

The final-code quiet macro launch at 03:25 CST also refused a competing
video-test workload. No final-code quiet latency/frame-rate measurement was
obtained; the full matrix remains blocked. The upstream opt-in progress-film
test added at `2238f07b4` was also integrated; it changes no production
code and was not part of the earlier 72-case regression run.

The early macro diagnostic used Apple M5 / 16 GiB, macOS 27.3, Electron 44,
AC power, Low Power Mode off and the 120 Hz internal display. It used ten-second
intervals and one pair, before the final thumbnail/swatch repairs and DN2
integration; another lane's Cargo/Electron work was active. It is not a
like-for-like before/after acceptance comparison. Its drag p95 stages were
input→patch 17.7 ms, patch→request 5.7 ms, queue 0 ms, upload 1.7 ms,
IPC/host/surface 23.2 ms, ready→composite 7.6 ms and composite→GPU-fence
observation 16.7 ms. Stage percentiles must not be added. The fence observation
includes display-clock polling and does not measure physical scanout.

The combined check uses the built editor and locally rebuilt release host and
surface addon, with no packaged build. Full Rust and unit checks pass; Svelte
reports zero errors and the existing `IconCropper.svelte` warning. The full
non-film editor subset passes 72 cases with one opt-in project capture skipped.
The Live history regression waits for each completed camera frame, avoiding repeated
reads of a still-current image. Runtime code is `63ec627c5`, release-host SHA-256
`0b10a59c8ee03af7c69c5bd6d40949cd0da43f34db7bffa8c1b45f365843ca30`, surface-addon
SHA-256 `2d700450e0a97b9c97151066a1f94f94d5e0695681fa387b04967e1e6ba80c10`.
Quiet acceptance remains required; earlier loaded diagnostics cannot establish
that PASS.
<!-- lane-p5b-measurements:end -->

<!-- lane-p4-measurements:start -->
### Row (c): P4 measured editor performance

**FAIL — not ready to ship.** Functional passes do not waive the measured
Live, Final or retention failures below. The million-triangle preparation
regression is fixed; the full stress loop still contains tasks over 50 ms.

| Gate | Before P4 | After P4 | Status |
| --- | --- | --- | --- |
| Million-triangle preparation task | 119 ms | No detected task ≥50 ms; ready 1,053 ms | PASS for this regression |
| Sustained Live, completed product cases | No comparable full-editor baseline | Playback p95 17.2–33.4 ms; 47.4–59.4 fps | FAIL |
| Input → completed image | Quiet/loaded historical diagnostics only | Worst completed-case p95 59.8 ms versus 50 ms target | FAIL |
| Main thread across 3D edits | 119 ms preparation task | Hot intervals have no detected ≥50 ms task; cold back setup 186 ms, stress 84 ms | FAIL |
| Refine timing, completed products | No comparable full-editor baseline | 1080p 32-sample OIDN 1.40–3.16 s versus 2 s target | FAIL; visual review OPEN |
| Final, completed products | No comparable 64-sample baseline | Median 1.72–4.81 s; p95 1.75–4.85 s versus 1 s target | FAIL |
| 30-minute retained memory | Not established | +59.8%, 28.3 MiB/min over 1,152 measured cycles | FAIL; fixed-set repeat OPEN |
| Peak memory, measured workloads | Not established | Worst app 4.54 GiB; host 2.12 GiB; measured stress swap −192 MiB, zero orphans | PASS for measured workloads; full matrix OPEN |

The completed cases use the built dev editor and bundled release native host on
Apple M5 / 16 GiB, macOS 27.3, Electron 44.0.0, AC power, Low Power Mode off,
no reported thermal/performance warning, and the 120.0006 Hz internal display.
The ordinary visible window is 1440×900 logical points on a 1512×982 display
at DPR 2. Its fitted viewer is recorded per case; the composition/capture is
1920×1080. No renderer/resolution test overrides are set. Every measured pane
asserts `native-studio`; shared-texture transfers and nonblank output are
required. The phase checks found no competing lane's Playwright/Electron/
Cargo/rustc process; idle crash reporters are excluded. Other desktop apps
were left open. These are quiet-lane measurements, not a rebooted machine.

The initial run completed hero, edge and macro before an uncaught native
surface-release error opened Electron's fatal modal during back playback.
Only those first three cases are retained as valid acceptance data; their
gates are reconstructed from saved summaries because the original controller
did not reach its final aggregation. Their main-thread coverage is PARTIAL:
it omitted the task containing phase start and did not record cold setup.
The interrupted back interval and unstarted cases are excluded, not passed.
The follow-up completed back using the ownership fix and corrected task
coverage. Chrome stopped after two valid pairs when another lane's Cargo
build became active; those partial results do not pass a full gate. A later
Playwright run and a falling shared disk reserve blocked further acceptance
launches. The resource guard was verified to refuse launch below 6.1 GB; the shared
container later fell below the required 6 GB reserve. Further acceptance launches were stopped.
The unmeasured scenes and fixed-project stress remain OPEN, not passed.
Cold-setup and refinement tasks count toward the main-thread gate as well as
hot drag/playback tasks.

Initial runtime code is `4709cb7be`, controller `46774d779`. The ownership
fix is `22a19387e`; crash-safe controller cleanup is `f0023e2c2`. The first matrix's documentation edits were pending; its loaded
controller and compiled editor/native binaries stayed unchanged throughout.
Later reports also fingerprint their controller and list pending file paths.
The initial host SHA-256 is
`5c8f661c86f418f0b373eeb5b264702b22bb93f84cd21c4c77fa286e5b76e8fb`.
The ordinary native build before the follow-up uses the same engine source
and `oidn,metalfx`, without the fixture-export `yardstick` feature:
`857f203eb6c260b16fc980881bf9b99bc10a9e84d619c21eddba2c1ef417a036`.
The rebuilt surface addon SHA-256 is
`2d700450e0a97b9c97151066a1f94f94d5e0695681fa387b04967e1e6ba80c10`.
The final controller `f32346be0` also samples competing processes and disk
space every two seconds, rejects a launch below 6.1 GB, stops before the
reserve reaches 6.05 GB, and writes lossless `.json.gz` timing traces.
Raw samples, console traces and synthetic PNGs remain under
`/tmp/pm-shots/studio3d/laneP4/`; none are committed.

Rows suffixed `-partial` or `-short` are diagnostic only: chrome has two
accepted 30-second pairs before the third was rejected for contention; the
landscape/four-view check has one two-second pair and one warm Final frame.
Those rows cannot pass the full protocol. Remaining normal-density cases
without a completed row are OPEN.

Each completed case has three 30-second drag and 30-second playback intervals,
each after at least 30 actual warm native presentations. Table percentiles are
the worst run percentiles, not percentiles of percentiles. FPS is the min–max
across runs; dropped counts sum the 60 fps presentation deficits. Separate
>33.3 ms percentages and maximum gaps expose stalls. Input latency ends at an
asynchronously polled WebGL2 completion fence on a matching native image;
polling adds up to one display interval. Display scanout is not measured.
The long-task observer reports only tasks of at least 50 ms, so zero entries
does not mean zero main-thread work. Values are rounded only for display.

Refine lists release-to-displayed 32-sample OIDN at the fitted viewer and an
explicit 1080p/32-sample OIDN capture. Final is native 1080p/64-sample OIDN,
8 bounces, 30 advancing warm frames plus a separately reported cold frame.
These scenes have motion blur disabled; the production shutter path is still
covered by the passing deterministic Final/motion-blur e2e tests. Render times
exclude PNG encoding and video encoding. Refine/Final algorithms and quality
settings were not reduced. All eleven bounded Cycles scenes still pass. Refine PASS below refers to the
time component only; a complete clean/no-ghosting visual review remains OPEN.
Nonblank output is checked automatically; only hero and macro received
additional manual image inspection in this lane.

| Run | Code SHA | Dirty | Start UTC | Finish UTC | Peak app MiB | Host MiB | Metal MiB | Swap Δ MiB | Orphans |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| first three cases; later aborted | 46774d779 | True | 2026-11-09T21:29:28.158Z | aborted | — | — | — | — | not finalized |
| back valid; later stopped for contention | f0023e2c2 | True | 2026-11-09T22:14:48.594Z | 2026-11-09T22:23:13.243000+00:00 | — | — | — | — | 0 |

| Scene | Start UTC | Viewer px | Unique triangles | Instances | Lights | Scatter candidates | Native panes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| hero | 2026-11-09T21:29:29.453Z | 1394×784 | 4196 | 6 | 4 | 0 | 1 |
| edge | 2026-11-09T21:33:54.276Z | 1394×784 | 3860 | 5 | 4 | 0 | 1 |
| macro | 2026-11-09T21:38:07.360Z | 1394×784 | 3860 | 5 | 4 | 0 | 1 |
| back | 2026-11-09T22:14:49.598Z | 1394×784 | 3860 | 5 | 5 | 0 | 1 |

| Scene | Drag fps min–max | Drag ms p50/p95/p99 | Play fps min–max | Play ms p50/p95/p99 | Dropped drag/play | Long-task count; ms p50/p95/p99/max |
| --- | --- | --- | --- | --- | --- | --- |
| hero | 91.5–94.5 | 8.5/16.7/17.4 | 56.5–57.9 | 16.7/33.2/33.5 | 0/257 | 0 (none ≥50 ms) |
| edge | 91.8–105.2 | 8.5/16.8/17.1 | 58.8–59.4 | 16.7/17.2/33.3 | 0/76 | 0 (none ≥50 ms) |
| macro | 51.4–55.6 | 16.9/25.1/25.6 | 47.4–47.5 | 16.9/33.4/33.7 | 559/1130 | 0 (none ≥50 ms) |
| back | 58.5–60.6 | 16.7/18.0/25.4 | 51.1–54.3 | 16.8/33.1/40.9 | 68/646 | 0 (none ≥50 ms) |
| chrome-partial | 59.7–60.2 | 16.7/25.0/41.3 | 48.9–53.1 | 16.8/34.2/42.1 | 7/542 | 0 (none ≥50 ms) |
| landscape-short | 31.1–31.1 | 25.1/33.6/421.1 | 36.9–36.9 | 32.9/33.7/181.1 | 58/46 | 0 (none ≥50 ms) |
| 4view-short | 116.9–116.9 | 8.3/8.7/16.5 | 59.4–59.4 | 16.7/17.3/17.9 | 0/1 | 0 (none ≥50 ms) |

Frame/latency percentiles below are the worst run percentiles, not percentiles of percentiles. Dropped counts sum all runs.

| Scene | Pointer → GPU ms p50/p95/p99 | Time → GPU ms p50/p95/p99 | Matched/total inputs | Host p95 max ms |
| --- | --- | --- | --- | --- |
| hero | 27.3/35.2/35.9 | 25.8/42.3/42.6 | 16053/16064 | 11.0 |
| edge | 27.3/35.0/35.8 | 25.9/26.4/42.6 | 16059/16070 | 11.0 |
| macro | 43.3/52.5/59.9 | 50.7/59.8/68.4 | 16037/16054 | 19.7 |
| back | 42.4/44.5/51.9 | 42.7/59.7/68.4 | 16017/16033 | 17.7 |
| chrome-partial | 42.6/52.0/67.9 | 51.4/59.9/68.3 | 10620/10629 | 19.4 |
| landscape-short | 61.3/85.5/400.6 | 68.9/85.3/165.8 | 312/318 | 27.2 |
| 4view-short | 18.0/18.7/26.3 | 26.0/26.9/27.4 | 356/358 | 8.3 |

| Scene | Cold scene s | Refine viewer/1080p s | Cold Final s | Warm Final median/p95 s | Warm frames | Peak app/host/GPU MiB |
| --- | --- | --- | --- | --- | --- | --- |
| hero | 1.43 | 1.08 / 1.66 | 2.14 | 2.05 / 2.08 | 30 | 3810.8 / 2143.9 / 533.4 |
| edge | 0.35 | 0.96 / 1.40 | 1.77 | 1.72 / 1.75 | 30 | 3589.2 / 2143.3 / 533.2 |
| macro | 0.36 | 2.05 / 3.16 | 5.03 | 4.81 / 4.85 | 30 | 3475.2 / 2124.0 / 533.2 |
| back | 0.61 | 1.60 / 2.43 | 3.54 | 3.28 / 3.35 | 30 | 2730.0 / 2061.7 / 533.2 |
| chrome-partial | 1.17 | — / — | — | — / — | 0 | 1931.4 / 1362.6 / 533.2 |
| landscape-short | 2.54 | 7.10 / 9.57 | 17.72 | 17.99 / 17.99 | 1 | 3000.2 / 2175.8 / 601.7 |
| 4view-short | 0.86 | 0.40 / 1.65 | 2.14 | 2.07 / 2.07 | 1 | 2943.4 / 1629.8 / 614.7 |

| Scene | Native | Live | Input | Main thread | Refine | Final |
| --- | --- | --- | --- | --- | --- | --- |
| hero | PASS | FAIL | PASS | PARTIAL | PASS | FAIL |
| edge | PASS | FAIL | PASS | PARTIAL | PASS | FAIL |
| macro | PASS | FAIL | FAIL | PARTIAL | FAIL | FAIL |
| back | PASS | FAIL | FAIL | FAIL | FAIL | FAIL |
| chrome-partial | PASS | PARTIAL | PARTIAL | FAIL | PARTIAL | PARTIAL |
| landscape-short | PASS | PARTIAL | PARTIAL | PARTIAL | PARTIAL | PARTIAL |
| 4view-short | PASS | PARTIAL | PARTIAL | PARTIAL | PARTIAL | PARTIAL |


| Scene | Worst >33.3 ms % drag/play | Max warm gap ms | Worst input match % | Native host/trace/post p95 ms | Cold; Refine-view long-task count/max ms |
| --- | --- | --- | --- | --- | --- |
| hero | 0.4/3.2 | 86.6 | 99.9 | 11.0/5.2/6.3 | —; 0/— |
| edge | 0.4/1.0 | 126.8 | 99.9 | 11.0/5.2/6.5 | —; 0/— |
| macro | 0.4/6.4 | 151.2 | 99.9 | 19.7/13.1/7.6 | —; 0/— |
| back | 0.6/3.8 | 83.8 | 99.8 | 17.7/10.4/9.1 | 2/186.0; 0/— |

The earlier two-second camera-only four-view diagnostic is excluded from
acceptance: unchanged side panes can cache pixels, so their low native-update
counts alone do not prove a slow view. The implemented moving-product workload requires new images in all four panes
and also plays the timeline during orbit; its full measurement remains OPEN.
Its per-pane gate prevents staggered updates
from being added into a false 60 fps claim. Density diagnostics, when present,
use one five-second interval per phase and no Final frames; they cannot pass
the full-duration gates. Room/street and extra-density Refine/Final have no
specified time target and are still unmeasured.

Peak app RSS is sampled every two seconds over the whole app/host/helper tree.
Host kernel RSS high-water and Metal `currentAllocatedSize` supplement those
samples where available. Metal allocation overlaps unified RSS and is not
added to RSS as an independent total. Driver/OS allocations are not separately
attributed. Memory figures are measured diagnostics, not allocation estimates.

| Measurement | Code SHA | Start UTC | Machine state | Start free disk | Start swap used |
| --- | --- | --- | --- | --- | --- |
| full hero/edge/macro | 46774d779 | 2026-11-09T21:29:28.158Z | quiet; AC; low-power off; no thermal warning | 7.9 GB | 4860.44M |
| full back; partial chrome | f0023e2c2 | 2026-11-09T22:14:48.594Z | quiet; AC; low-power off; no thermal warning | 7.5 GB | 4524.38M |
| short landscape/four-view | 4709cb7be | 2026-11-09T20:40:10.337Z | quiet; AC; low-power off; no thermal warning | 8.8 GB | 4541.12M |
| 30-minute growing-catalog stress | 46774d779 | 2026-11-09T20:52:10.133Z | quiet; AC; low-power off; no thermal warning | 9.4 GB | 4509.12M |

| Required case / follow-up | Coverage and status |
| --- | --- |
| Hero, edge, macro, back | Full-duration measurements above; Live and Final FAIL. |
| Chrome | Two accepted pairs; full Live/Refine/Final OPEN. Cold main-thread task 119 ms: FAIL. |
| Materials, caustics, wet | Full editor measurements OPEN. |
| Default Landscape | Two-second diagnostic only; full acceptance OPEN. |
| Room interior, street with many lights | Full editor measurements OPEN. |
| Four-view with all panes changing | Workload implemented; full acceptance OPEN. Earlier camera-only check is diagnostic. |
| Landscape at 2× density; 100k-tree/54k-ground-cover forest | Full editor stress measurements OPEN; historical hidden-host forest number is not comparable. |
| Fixed two-project working set | 30-minute retention measurement OPEN. Growing-catalog workload below FAILS. |
| Refine clean/no-ghosting review | Complete visual quality gate OPEN, independently of timing failures. |


| Stress workload | Measured min | Cycles | Retained app MiB | Growth | Slope MiB/min | Peak app/host/Metal MiB | Swap Δ MiB | Long tasks count; p50/p95/p99/max ms | Orphans | Retention gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| growing catalog | 30.01 | 1152 | 1537.6 → 2457.0 | 59.8% | 28.3 | 4650.4/222.7/238.5 | -192.0 | 96; 52.0/78.0/84.0/84.0 | 0 | FAIL |

The implemented stress modes use a fixed five-minute warmup, real project open/save/close,
FBX import/delete, default-scatter reseeding, window resize, and same-phase
closed-tab GC/idle samples. Growing-catalog stress retains fresh file identities:
its catalog reached 1,355 projects. Post-measurement diagnostics counted
1,165,307 DOM nodes / 44,365 listeners with 1,690 attached elements. This flags
retained UI/resources for investigation; the growing catalog also needs to be
separated from a fixed working set. The fixed-project mode remains unmeasured; it asserts a stable catalog and closed
tabs, keeps Undo history, and does not destroy saved projects to obtain a pass.
Stress long tasks include open/save/close and forced GC; no stack attribution
is claimed for the worst task. All raw machine/phase checks remain in evidence.

Before P4, the comparable million-triangle edit regression recorded a 119 ms
preparation task; after P4 it detects no task ≥50 ms (first draw 1 ms, ready
1,053 ms versus 1,204 ms). The original hidden-host product timings of
7–13 ms Live, ~15 ms quiet / 32.4 ms loaded input p95, and the different
100,000-tree/54,000-ground-cover forest's 37.8 ms Live are historical
**diagnostics**, not full-editor before/after measurements at these settings.
No earlier full-editor 1080p/64-sample Final or 30-minute retention baseline
was established. Default Add ▸ Terrain remains 18,620 scatter candidates;
it was not reduced to improve a result.

Remaining work: eliminate full-editor playback/presentation misses and dense
landscape GPU cost, bring 64-sample Final within its targets, bound retained
renderer/catalog resources, attribute and remove the remaining >50 ms tasks,
and finish the separate synchronous skinned-pose and physical-device-loss gates.

<!-- lane-p4-measurements:end -->

## Robustness and security

| Gate | Status | Evidence / acceptance |
| --- | --- | --- |
| Host crash, GPU/device loss, hung response | PARTIAL | Timeout rejects all pending requests and portable backend replays scene/view/time: [recovery tests](../../apps/desktop/src/renderer/src/core/studio-native/recovery.test.ts). P3 adds device-generation ownership for shader compilation, target recreation and stale editor-frame eviction; mocked device-loss tests pass in `gpu-lifecycle`, `backend-lifecycle` and `webgpu-backend`. Manager tests cover crash generations and forced reaping. Packaged crash/replay passes; physical device-loss injection remains required. |
| IPC framing and back-pressure | PARTIAL | JSON ≤4 MiB, binary ≤256 MiB, bounded request count and aggregate pending bytes; partial-read and bad-header tests in [protocol](../../engine/hosts/studio-electron/src/protocol.rs) and manager tests. No network command listener; inherited child pipes only. |
| Shared-texture peer and lifetime | PARTIAL | Main authenticates kernel Mach audit PID against its spawned host; addon tests reject another process. Managed VideoFrame/reference release and three-slot host pool; packaged resize/reload/two-window checks pass. Long stress remains open. |
| File access confined to referenced assets | PARTIAL | Host accepts geometry/texture/FBX **bytes**, never a renderer-supplied filesystem path. FBX external-file loading is disabled in [importer](../../engine/hosts/studio-electron/src/fbx.rs). Main owns executable/addon resolution. Continue review when adding any path-based host operation. |
| GPU memory budget for large scenes | PARTIAL | P3 adds preallocation scene/target checks, a conservative 512 MiB geometry allowance and 2 GiB session allowance, inactive native target / portable LRU target eviction, bounded geometry workers and close-time cache release. Existing texture planning lowers texture resolution under its 256 MiB cap. A synthetic 1.1M-triangle native scene fails before BVH/GPU work and preserves the previous scene; portable oversized geometry/target tests pass. Animated scatter overflow is reduced in evaluated copies with a warning. P4 records real-app RSS and a 1,152-cycle / 30-minute growing-catalog stress run below: absolute peaks pass, retained growth FAILS. Large decoded textures and fixed-working-set stress remain OPEN. Some full-resolution 4K requests now fail with a controlled budget error; no out-of-core rendering. |
| Broken model, font or texture isolated to one layer | OPEN | Corrupt/missing model/font/texture fixtures must leave healthy layers visible and editable, with a clear per-layer error; Lane K owns import/font fixes. |
| Save/reopen and undo/redo for every new 3D action | PARTIAL | P3's `history-roundtrip` matrix checks 27 actions with exact persistent before/undo/redo equality and evaluated save/reopen equality: World/sun/HDRI, terrain/heightmap/regions, scatter, fog, Material Options, iris/focus, sample policy, parts/finishes, five Shots, exploded view, studio/light helpers, source replacement, duplicate/remove. Multi-view layout/panes now persist through ordinary document history with their own round-trip test. Existing import/text/material/parenting e2e remains required; pass exports are export jobs, not undoable document properties. Full packaged UX acceptance remains OPEN. |
| Old-project migration | PARTIAL | P3 adds tiny synthetic pre-Studio and early-Studio fixtures covering `switch3DVersion`, explicit flat artwork, camera/material/timing values, conversion undo/redo, scatter source references and a second save/reopen. Fixes broken converted source IDs and future-version downgrades. Semantic tests pass; representative old-project pixel/look parity remains OPEN. |
| Logs and privacy | PARTIAL | P3 reviewed new scene3d/Studio/host paths: no telemetry/network send of project data found; the only renderer fetches in those paths load bundled KTX decoder assets. Host stderr is drained without forwarding raw scene data/user paths to telemetry. Host receives scene/FBX bytes; FBX external-file reads are disabled and external glTF resources rejected. Dynamic network-capture privacy verification remains OPEN. Do not add raw exception/scene/asset payloads to telemetry. |

## Review, UX and documentation

| Gate | Status | Evidence / acceptance |
| --- | --- | --- |
| Native host / main IPC / preload / backend review | PASS for P1 scope | Lane P1 reviewed process ownership, framing/limits, Mach peers, managed references, renderer replay, file access and packaging; regression fixes below. GPU device-loss injection and large-scene/long-run stress remain open robustness gates. |
| Scene schema, evaluator, compositor, Final/export, materials/lighting/environment, assets/import review | PARTIAL | P3 reviewed the engine/packed contract, WGSL finite math and traversal bounds, geometry/BVH extraction, GPU/worker ownership, scene edit/history/migration and model-import allocation boundaries; fixes and regressions at `4368b15db` and `46f2e9861` below. Existing seeded-scene/render tests stay green. This is not full compositor/Final visual or whole-system approval; measured main-thread stalls and the remaining acceptance gates still block shipping. |
| UX review | PARTIAL (19 fixed, 4 open) | Lane UX walked every 3D flow as a first-time After Effects user in the dev app (native Studio backend, light and dark, background windows): import, extrusion, layer/group materials, every light type and light by pointing, camera/iris, World/sky/sun, Terrain → Scatter → Fog, 1/2/4 Views, Shots/exploded view, render passes, export, error and empty states, focus order and undo labels. Found 5 high, 11 medium and 7 low issues; fixed all high, 10 medium and 4 low (list in [Lane UX findings](#lane-ux-findings)). New flows are covered by [UX review tests](../../apps/desktop/e2e/studio-ux-review.spec.ts). Open: native Edit ▸ Undo doesn't name the action; finish swatches render near-black; corner notices sit over the Properties header; packaged-app and real-screen (native menu) review not repeated. |
| User docs complete | PARTIAL | Lane UX rewrote the top of [Studio](../../apps/desktop/docs/STUDIO.md) as a user guide naming every 3D feature as the app does: Add 3D Layer, import, extrusion, materials/finishes/maps, Material Options, lights and light by pointing, camera/iris, World, Terrain/Scatter/Fog, the viewer and views, Live/Refine/Final, Render Quality presets and Custom settings, Shots/exploded view, export and passes, and a what-went-wrong table. [3D guide](../../apps/desktop/docs/SCENE3D.md) points to it and uses the current viewer names. Still to check against a packaged build: environment creation walkthrough and supported font/texture lists. |
| Agent docs and bundled API pack complete | OPEN | [native integration](native-host.md), [parity](parity.md), [environments](environments.md), [Cycles yardstick](cycles-yardstick.md), schema/operations and bundled API pack include every new action, units, limitations and reproducible commands. |

## Lane UX findings

Lane UX, 2026-11-10, branch `studio-3d-ux` (dev build, Apple M5 / 16 GiB,
macOS 27.3, native Studio backend). Before/after captures stay outside Git.
Severity: **H** blocks a first-time user or hides a failure; **M** confuses or
contradicts After Effects; **L** polish.

| # | Sev | Issue | Status |
|---|---|---|---|
| 1 | H | The "Storage optimized" notice sat over the viewer, its Move tool and a pane label. | Fixed: housekeeping notices use the corner stack. |
| 2 | H | A damaged model showed "layers-3d · Unexpected token … is not valid JSON". | Fixed: "Couldn’t import *file*. The file isn’t a readable *GLB* model…", from the app (built-ins no longer speak under an internal extension id). |
| 3 | H | A native renderer crash fell back to WebGPU silently. | Fixed: one corner notice; the Refine pill's tooltip says it is on the backup renderer. |
| 4 | H | A scene over the GPU memory budget froze the last picture with no message. | Fixed: the pill says **3D can’t render** with the reason and what to do. |
| 5 | H | Extruded text in a font that isn't installed vanished silently. | Fixed: Geometry Options names the font and what to do. |
| 6 | M | World, Ambient and Environment lights offered Link Layers (Timeline column, inspector, layer menu, pickwhip). | Fixed: they surround the scene and can't link or be linked to. |
| 7 | M | Move/Rotate/Scale and Global/Local were offered for those layers. | Fixed: the strip says "Lights the whole scene · no position" and keeps Light. |
| 8 | M | Side panes in 2/4 Views gave no feedback while drawing or after edits. | Fixed: each says Rendering…, Updating…, Live or Can’t draw, with a tooltip. |
| 9 | M | Inspector names mixed case and non-AE terms (Field of view, Clip start, f-stop, Focus, Softness, Range, Cast shadows). | Fixed: Angle of View, Near/Far Clip, F-Stop, Focus Distance, Cone Feather, Falloff Distance, Casts Shadows; Title Case throughout. |
| 10 | M | Labels were clipped mid-word ("Coat roughne") or wrapped ("Iris roundness"). | Fixed: one-line labels, wide enough for AE's two-word names, ellipsis beyond. |
| 11 | M | The camera had its own "Final quality" sample picker, contradicting the composition's Render Quality. | Fixed: it names the composition's quality and opens it. |
| 12 | M | Every agent/script 3D edit was "Edit 3D layers" in Undo history. | Fixed: Add Terrain, Change World, Change Render Quality, … |
| 13 | M | The light tool said Spotlight/Bulb and named new layers so; menus say Spot Light/Point Light. | Fixed: one set of names. |
| 14 | M | Render Passes vanished for video exports with no explanation, and 3D artwork/nested 3D didn't count as 3D. | Fixed: shown but unavailable, saying to choose Images ▸ PNG sequence. |
| 15 | M | The Refine pill's "Clean" related to no setting. | Fixed: "Refined · Production", tooltip names Render Quality. |
| 16 | M | Edit ▸ Undo/Redo don't name the action ("Undo Add Terrain" in AE). History labels are right; the native menu needs a renderer→main label channel. | Open |
| 17 | L | Add 3D Layer was one flat list without icons. | Fixed: Shapes / Environment / Lights and Camera, Timeline icons. |
| 18 | L | An empty composition offered no way into 3D. | Fixed: **3D Layer** beside Text/Shape/Solid/Null. |
| 19 | L | Source read "box". | Fixed: primitive names (Cube). |
| 20 | L | Sentence-case buttons (Drop to floor, New variation, Add sky, Exploded view, Set up studio). | Fixed: Title Case. |
| 21 | L | Finish swatches render as near-black spheres; names only in tooltips. | Open |
| 22 | L | Corner notices that wait for acknowledgement sit over the Properties header. | Open |
| 23 | L | Blend Mode reads "normal" (lowercase) on 3D text. | Open (2D inspector) |

Checked and fine: missing textures say **Missing image/comp/layer** with
guidance; Scatter's disabled menu row says what is missing; focus moves from
Render Quality through its Custom fields to the next panel; every new control
works with Undo and composition switches.

**Render Quality UI.** Composition properties (nothing selected, 3D present):
preset picker, plain description and "Up to about N s per frame"; Custom
reveals Max/Min Samples, Noise Threshold, Time per Frame, Max Bounces,
Caustics, Denoise and Blur Samples, each one Undo. Export ▸ Rendering ▸ **3D
Render Quality**: Composition · *preset* (default) or a per-export override,
never saved, with a whole-export estimate. Both read
`api.scene3d.renderQuality.choices/fields/describe` so names and estimates
match; the viewer's Refine pill and the camera's Render Quality row use the
same description.

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
| Main-thread responsiveness | High / partly fixed by P4 | P3's 1,024,000-triangle editor run measured a 119 ms preparation task (first draw 1 ms, ready 1,204 ms; earlier 154–174 ms). P4's same regression detects **no ≥50 ms task**, first draw 1 ms, ready 1,053 ms, and tightens the assertion from 400 ms to 50 ms. Terrain/scatter generation and large scene/TLAS preparation now use bounded workers and incremental main-thread slices. Skinned pose evaluation remains synchronous: the earlier benchmark was about 20.5 ms median / 27.8 ms p95. Full-scene measurements below remain required; this narrower regression does not waive them. |
| Resident memory / long-run lifecycle | High | P4 growing-catalog stress completes 30 measured minutes / 1,152 cycles: peak app 4.54 GiB and host 222.7 MiB with swap down 192 MiB, but retained app memory grows 59.8%: FAIL. A fixed two-project 30-minute repeat, oversized geometry and large decoded textures remain OPEN. Allocation estimates and disposal tests do not establish those measurements. |
| Physical device loss | High | Inject native GPU/device loss and verify a usable frame plus preserved unsaved edits. Mock WebGPU loss and native process termination cover narrower failure modes. |
| Old-project appearance | Medium | Compare representative pre-Studio and early-Studio rendered looks, not only semantic data, camera, references and timing. Synthetic fixtures do not establish pixel parity between renderers. |
| Privacy instrumentation | Medium | Capture outbound traffic while opening/importing/editing synthetic projects to supplement source review; no project-data telemetry path was found in the audited code. |

The full desktop e2e result is recorded separately by Lane P2 below. P3 does not
claim full-suite, packaged UX or production-release approval. Local profiles,
logs, Cycles outputs and optional UI screenshots stay under `/tmp/pm-p3-*`,
outside Git.

## Lane P4 performance changes

Runtime changes are in `4709cb7be`; the five-minute stress warmup is in
`46774d779`. Viewer extension version is 1.4.3; all ten built-in extension
version checks pass. The normal editor benchmark is
[studio-editor-benchmark.mjs](../../apps/desktop/scripts/studio-editor-benchmark.mjs),
with separately tested percentile, presentation and memory-trend calculations.

- Terrain signatures no longer rebake procedural terrain while evaluating a
  camera/time change. Terrain/scatter work uses a bounded worker; scene packing
  yields in roughly 4 ms slices and large TLAS builds use a worker. Prepared
  scene uploads use bounded 1 MiB chunks and compact repeated instance paths.
- Continuous Live edits keep one request in flight and coalesce the next input.
  Completed native images remain presentable during a drag. Redundant uploads
  of unchanged images are avoided; opaque Live layers can share one depth pass.
- Refine work can be cancelled between four-sample batches and during OIDN.
  Cancellation also invalidates a pending upload before it starts Refine.
  Final sample order, sample count and bounce policy are unchanged. Reused OIDN
  filters agree with fresh filters within 1e-6; all eleven Cycles scenes pass.
- Background 3D thumbnails use asynchronous prepared captures and pause while
  editing/preparing. Pointer navigation suppresses expensive forest picking.
  Silent playback no longer creates an unused AudioContext on its first frame.
- Stale composition runtimes and preparation depth sets are released. Native
  sessions are bounded to four across windows for four-view layout. Metal
  allocation and host RSS diagnostics make resource use observable.
- Explicit captures own their preparation/render interval, keeping normal
  viewer paints from replacing Final/shutter frames. All 71 non-film 3D e2e
  tests pass, including deterministic Final and motion blur.

These changes do not establish the performance targets by themselves. Native
backend assertions, output validation and the measured gate results determine
the status of row (c).

The longer editor run also reproduced a main-process exception in Electron's
late shared-texture release callback: `Unknown Studio surface handle`. Its
native error dialog blocked editor IPC during the back-scene interval. That
interrupted interval is not valid acceptance evidence. Surface ownership now
uses a type-tagged Buffer object instead of a raw backing-store address; a
native regression reproduces the previous acceptance of an aliased Buffer.
The late callback always acknowledges the host once and logs release errors
without throwing into Electron. Address reuse/GC is a suspected trigger of the
observed lookup failure, not a proven stack-level diagnosis.

The benchmark now fails and exits its disposable app on uncaught main errors,
records main stderr and addon/controller hashes, and stops after three minutes
without progress. It counts long tasks overlapping phase start and records
cold scene setup; the earlier controller's hot-interval filter could omit the
task containing the start action. Short/contended diagnostic runs cannot pass
the full three-run Live protocol. The four-view follow-up moves the product
in every pane, and fixed-project stress retains its two projects and Undo
history while asserting a stable catalog. These are measurement corrections,
not changes to quality settings or acceptance thresholds.

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

## Saved render quality and faster exports (Lane Q)

Render quality now persists per composition with migration to Production,
Undo/Redo, revision guards and temporary export/capture overrides. The API is
`scene3d.renderQuality` and `edit_3d.set_render_quality`; see
[Studio settings](../../apps/desktop/docs/STUDIO.md#saved-render-quality-and-export-overrides)
for Draft / Preview / Production / Final (Max) defaults and Custom bounds.
Existing explicit camera sample edits update the composition's Custom budget.
This lane adds data, engine, export and agent APIs; visual controls are separate.

Native normal depth sets share one frame-local shutter accumulation and one
OIDN finish. Styled boundaries and WebGPU retain per-time filtering. A tracing
limit can preempt the noise floor; it does not include filtering, scene setup,
readback or encoding. Saved quality uses one-sample GPU submissions and
explicit floors override the legacy caustic minimum. Production's stable
photon prepass allows adaptive pixels to finish without a hidden 512-sample
floor; Final (Max) keeps progressive photon sampling. Scene changes invalidate
the map. Export temporal image reuse is disabled.

The overall production decision remains **not ready to ship**. The new presets
make quality selectable; they do not waive the native-resolution appearance,
moving-sequence, quiet-machine timing or full-editor acceptance gates. Detailed
preset measurements and limitations are recorded in
[native host measurements](native-host.md#saved-quality-and-frame-budgets-lane-q).
