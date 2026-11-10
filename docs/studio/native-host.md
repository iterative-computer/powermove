# Native Studio in Electron

Studio runs in `pm-studio-host`, an isolated Rust process. Electron owns the
window, controls, scene evaluation and compositing. No Blender is needed.
The integration implements Lane A's `StudioBackend` contract; the factory is
registered by `core/studio-native/install.ts` before editor boot. The combined
`studio-3d` branch retains main's document model/panels and Lane A's current
backend dispatch, extractor, compositor and final-frame preparation.

## Rendering and transport

- **Live:** bounded tracing resolution, one sample, three-bounce path tracing
  with Metal hardware ray queries. Direct lighting uses bounded weighted reservoirs:
  two fresh candidates (four above eight lights), one temporal reservoir and
  one spatial neighbor, with one final visibility test. Short-path indirect
  light is reused by the temporal denoiser. This is not a full ReSTIR GI solver.
- **macOS 26+:** `MTLFXTemporalDenoisedScaler` receives linear color, depth,
  camera motion, diffuse/specular albedo, view normals, roughness, jitter and
  camera matrices, then denoises and upscales to the output size. Object,
  material and lighting changes reset history; camera movement retains it.
- **Refine/Final:** the same tracer accumulates at output resolution using saved
  Draft/Preview/Production/Final (Max) or Custom settings. Native Draft uses a
  fast GPU filter; other presets use OIDN with prefiltered path-following guides
  for beauty and catcher shading. Time caps can finish before the noise floor.
  Eligible shutter frames accumulate before one OIDN finish. The bundled OIDN
  build uses its CPU device.
- **Portable fallbacks:** WGSL BVH traversal without ray queries, temporal
  reprojection plus edge-aware à-trous filtering without MetalFX, binary RGBA
  frames without the surface addon, and the in-page WebGPU backend after a
  process or transport failure.

MetalFX and the Metal handle borrowing live in `gpu/metalfx.rs` and
`native/metalfx.mm`. Reservoir sampling has its own WGSL/module. No Metal object
or raw pointer is exposed to the editor page.

Electron 44 provides experimental
[`sharedTexture.importSharedTexture` and `sendSharedTexture`](https://github.com/electron/electron/blob/v44.0.0/docs/api/shared-texture.md).
Its [macOS handle is a process-local IOSurfaceRef](https://github.com/electron/electron/blob/v44.0.0/docs/api/structures/shared-texture-handle.md),
not an integer surface ID or a pointer that can be copied from Rust.
Electron's [implementation notes](https://github.com/electron/electron/blob/v44.0.0/shell/common/api/shared_texture/README.md)
require transferring a Mach right before import. Offscreen shared textures
export Chromium's own frames; they do not import Studio's frames.

The chosen path is:

1. Studio packs the finished output on the GPU and blits it into one of three
   IOSurfaces.
2. A private Mach message transfers an IOSurface right to a small stable N-API
   addon in main. The addon authenticates the kernel audit PID against main's
   spawned helper before resolving a local reference. The random bootstrap
   service name routes messages; it is not the authentication boundary.
3. Electron imports and transfers the texture to a sandboxed preload receiver.
4. The renderer retains the managed VideoFrame and imports it directly into
   the WebGL compositor, avoiding an intermediate Canvas2D copy. There is no
   CPU frame readback on this path. GPU copies into the compositor remain.
5. `allReferencesReleased` releases the local reference and returns the surface
   slot to the host. Each layer keeps one completed image across quality tiers;
   superseding or disposing it closes its VideoFrame once. Buffered, cancelled
   and late frames also close on their respective release paths. A busy pool
   or unavailable API uses binary frames; conversion closes those VideoFrames
   even if Canvas2D throws.

The receiver follows Electron's
[managed preload example](https://github.com/electron/electron/blob/v44.0.0/spec/fixtures/api/shared-texture/managed/preload.js).
The MetalFX output is initialized once through wgpu before Metal writes it;
otherwise wgpu's first sampled read would clear an externally written texture.

## Hidden-host measurements (historical diagnostic)

Apple M5, 16 GB, macOS 27.3, Electron 44.0.0, 1920 × 1080 output.
Measured 2026-11-09. These are wall times, not GPU timestamp estimates.
The shared-texture path uses Metal ray queries, direct-light reservoirs and
MetalFX. Refine uses sixteen samples and OIDN at full output resolution.

Full product corpus, 45 camera/time edits per scene, 960 × 540 Live tracing:

| Scene | Host median | Input → pixels median / p95 | Refine + OIDN |
|---|---:|---:|---:|
| hero | 8.26 ms | 10.4 / 10.8 ms | 1.30 s |
| edge | 7.25 ms | 9.5 / 10.0 ms | 1.09 s |
| macro | 12.45 ms | 14.6 / 15.0 ms | 1.41 s |
| back | 10.94 ms | 13.1 / 13.5 ms | 1.21 s |
| chrome | 11.52 ms | 14.3 / 14.9 ms | 1.37 s |
| materials | 10.93 ms | 15.1 / 15.4 ms | 1.28 s |
| caustics | 9.39 ms | 12.5 / 13.0 ms | 1.14 s |
| wet | 12.73 ms | 14.8 / 15.3 ms | 1.39 s |

The timer starts before a camera/time edit, includes the small scene patch,
render, interprocess transfer and canvas draw, and ends after a pixel read
forces completion. This includes diagnostic synchronization/readback and
excludes display scanout/vsync. It is a hidden Electron integration harness,
not the full editor compositor. Host times include GPU submission and waits.
All eight scenes met a 16.7 ms p95 in this run. That is not a sustained full-app
60 fps guarantee.

A later 60-edit repeat during a Cargo check and concurrent Electron rendering
measured **10.8–21.9 ms median**, up to **32.4 ms p95**, and **1.29–2.07 s**
Refine. The target is not maintained under all concurrent workloads. No other
lane or user application was stopped for these measurements.

Peak native-process RSS, read with `getrusage` after Live, 1080p Refine and
960 × 540 Final, was **1452–1505 MiB (1.42–1.47 GiB)** across the product
corpus. This excludes Electron and is not a total unified-memory footprint.
Steady first-hit queries took roughly **0.5–0.9 ms** round trip. A first query
also compiles its pipeline. Cold first frames varied from approximately
**0.17 s to 3.4 s** with driver/shader/shared-image cache state.

Sequential runs with identical scene, size and sampling settings compared
transport costs (background machine load was not locked):

| Transport | Hero input → pixels median / p95 | Chrome input → pixels median / p95 |
|---|---:|---:|
| IOSurface + sharedTexture | 10.4 / 10.8 ms | 14.3 / 14.9 ms |
| Binary RGBA + canvas upload | 16.1 / 22.7 ms | 19.8 / 21.2 ms |

Both transports used MetalFX. Shared textures saved about 5–6 ms at the median
in this pair. The production shared-texture path has no CPU frame readback;
the benchmark's final pixel read is only a completion check.

At 960 × 540, sixty-four samples and denoising disabled in both engines,
all eight native/WebGPU comparisons passed. Mean absolute RGB differences
were at most **0.814 code values out of 255**. Different photon batch sizes and
hardware intersection rounding can contribute to the larger differences in caustic/glass scenes.
This establishes shared-renderer parity, not Blender Cycles equivalence.

Additional low-detail environment fixtures exercise scene size and many lights:

| Fixture | Instances / lights | Live input | Input → pixels median / p95 | Refine |
|---|---:|---:|---:|---:|
| Room and tabletop | 21 / 64 | 768 × 432 | 16.5 / 34.5 ms | 1.95 s |
| 144 simple trees and ground | 289 / 1 | 768 × 432 | 14.4 / 17.0 ms | 1.43 s |
| Room, orthographic camera | 21 / 64 | 768 × 432 | 17.1 / 23.2 ms | 2.11 s |

These render and pick correctly and match WebGPU. They do not establish
photorealistic environment quality or universal 60 fps. The room's 64 finite
point lights exercise bounded direct-light resampling. The forest fixture has
5,772 triangles; it is not a production foliage/large-world stress test.

Live initially traces at 50% of output dimensions; scenes above 128 instances
or 32 lights start at 40%, and participating media start at one third.
`STUDIO_LIVE_SCALE` fixes a diagnostic scale between one third and one half.
The adaptive budget is enabled by default (`STUDIO_ADAPTIVE_LIVE=0` disables
it for diagnosis): twelve frames above the 13 ms moving-average budget reduce
resolution; 120 frames below 8 ms allow recovery. Refine/Final retain the
requested output dimensions.

The historical 120-edit experiment rebuilt the renderer for resolution steps
and caused **0.8–1.57 s pauses**. P5b retains the device, pipelines, scene and
acceleration structures while replacing image buffers. When only tracing size
changes, MetalFX retains its guide/pack pipelines and output textures and
recreates its input textures/scaler with background pipeline optimization.
Resized Final images match fresh renderers exactly in the pixel regression;
MetalFX output-size tests cover shrinking, growing and returning to a prior
size. Full-editor timing acceptance remains separate from these regressions.

The integration script also killed the live native process deliberately.
Scene replay into WebGPU produced verified pixels in **38–52 ms** in two runs.
The same script verifies a populated → empty scene transition, including the
change from hardware queries to portable guide traversal.

<!-- lane-p5b-measurements:start -->
### Row (c): P5b hardening

The forty-cycle inspector reproduction retained **40/40** retired controls
before P5b and **0/40** afterward. After GC, DOM counters changed from
**4,087 → 36,286** before the repair to **4,087 → 4,566** afterward; listeners
end at **1,004**, stable after cycle ten, rather than **2,284**. Paired terminal
heaps contain 133 detached HTML elements after the fix versus 5,830 before.
These are focused diagnostics, not the whole-editor 30-minute retention gate.
The real-editor regression also requires collection of 100 disposed callbacks
and twelve retired inspector controls while the editor remains open.

The leak came from activation-long retention of disposed extension resources.
The live-release registry now removes and clears each retired entry. Closed
project document/history/state/journal caches also evict only after durable
flush, with failure and newer-write guards. VideoFrame ownership is bounded
across quality tiers, and long quality images no longer expire their active
native session. Native completion wakes the next eligible composite; side
panes follow display refresh and share the main pane's playback time.

Cold-setup profiling found synchronous GPU thumbnail reads and repeated
finish-sphere BVH builds. Thumbnail reads now use asynchronous GPU buffers;
saved-project posters capture the displayed canvas at at most 320×320 without
rendering a second composition. Finish swatches share geometry and yield to
editing. Swatches map completed GPU buffers before CPU encoding, eliminating
measured 80–110 ms waits on GPU-backed canvas PNG encoding. Responsive-edit
checks include cold, warmup and idle work through the displayed Refine image. The integrated DN2 specular-distance pass rebinds its
new guide textures after every adaptive resize; a shrinking/growing GPU-value
regression reproduced and verifies that repair.

The loaded million-triangle check observed no ≥50 ms background-preparation
task, but its initial synchronous draw took 118 ms. A portable Final
byte-equality check failed once and passed three immediate repeats plus the
final full suite; its cause remains OPEN and the exact-pixel assertion remains unchanged.

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

**Production performance remains FAIL until the quiet acceptance protocol
passes.** DN2's higher Final sample floor is preserved; actual native sample
counts must accompany any new timing and cannot be labelled as an exact
64-sample comparison with P4. See the P5b results in
[production readiness](production-readiness.md#row-c-p5b-resource-and-responsiveness-repairs)
for the current before/after gate results and combined checks. The combined
code `63ec627c5` passes 115 Rust tests, 7,486 unit tests (7 skipped), Svelte
checking (zero errors / one existing warning), and 72 non-film 3D editor cases
(one opt-in local-project capture skipped).
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

## Scene and process protocol

stdin/stdout carry framed messages. stderr is reserved for diagnostics:

```
4 bytes  "PMS1"
u32 LE   JSON byte length
u32 LE   binary byte length
JSON     { id, op, ... }
binary   raw little-endian arrays or RGBA8 pixels
```

Responses contain `{id, ok, result}` or `{id, ok:false, error}` plus optional
binary data. JSON is limited to 4 MiB and each binary packet to 256 MiB.
`hello` reports the protocol and exact GPU buffer strides, so stale packaged
hosts cannot silently interpret a newer material layout.

`scene` accepts the serialized `pm_studio::scene::Scene`. Its meshes can be
empty JSON placeholders plus binary `meshes` attachments identifying an object
and byte ranges for positions, normals, UVs and indices. `patch` replaces
camera/material/light data or supplies absolute column-major object transforms.
Base meshes, per-mesh BVHs and native BLAS remain resident; dragging updates
only the top-level hierarchy and instance data. Camera-only edits retain both
levels of acceleration structures.

`prepared` is the fast path for Rust's `Prepared` layout already produced by
the app's shared scene extractor. It supplies positions, triangle attributes,
BLAS/TLAS nodes, material data and lights/instances as binary ranges.
`preparedPatch` sends only the TLAS prefix, materials, lights/instances and
view and instance part identities. A changed layout or layer list sends a new
full upload. Invalid cyclic/deep trees and stale geometry patches are rejected
before GPU traversal. Texture arrays are uploaded separately; subsequent edits
replace only changed layers.

Other operations are `textures`, `screen`, `render`, `pick`, `bounds`,
`releaseFrame`, `snapshot` (corpus validation) and `shutdown`. GPU picks return
the actual first visible layer, part, world point and shading normal (including
normal maps); before a target exists a CPU triangle query is available. Bounds
are calculated from cached local boxes and current transforms, optionally
filtered by layer. Time is evaluated by the desktop host, passed with edits
and render requests, and seeds deterministic Final sampling. Per-layer output retains other layers' occlusion/reflections.
Multiple layer requests reuse the same finished trace.

The manager bounds pending requests, times out a stuck process, rejects work
on exit, and starts a new process/generation on the next connection. The
recovering backend replays its scene into WebGPU when native work fails.
Navigating, closing or crashing an editor releases its sessions and surfaces.
There are at most four native sessions across all editor windows to bound
memory while supporting a native session for each pane of the 4 Views layout.

Large prepared scenes use capability-negotiated `preparedBegin`, sequential
`preparedChunk` packets (at most 1 MiB), and an atomic `preparedCommit`.
Invalid/incomplete uploads leave the installed scene intact. Repeated part
identities use a binary index into a small string dictionary. The original
4 MiB JSON and 256 MiB binary limits remain enforced.

`cancelRefine` interrupts Refine between existing four-sample submissions and
through OIDN's progress callback. The pipe reader can observe cancellation
while the render thread is busy; queued input has both count and byte limits.
Final never uses this cancellation policy. Its sample order, bounce count,
denoiser quality and output dimensions are unchanged. OIDN reuses its filter
and working storage across frames, with regression coverage for resize,
rebinding and recovery after cancellation.

Scene and prepared-binary boundaries reject non-finite coordinates, invalid
physical material/light ranges, singular transforms and BVHs deeper than the
shader traversal stacks. Validation runs before expansion, BVH construction or
GPU allocation. Geometry has a conservative 512 MiB allowance; render targets,
scene buffers and textures share a 2 GiB session allowance. The inactive
Live/Full target is evicted before allocating a replacement that would exceed
that allowance. Texture planning reduces resolution under pressure, while
requests that cannot fit fail with a clear error and preserve the document.
Some full-resolution 4K requests exceed this allowance. These estimates and
oversized-scene regressions do not replace the real-app resident-memory stress
gate in [production readiness](production-readiness.md).

Portable WebGPU recovery starts a new device generation, discards stale shader
compilations and completed frames, then recreates targets from the retained
scene. Native process-crash replay and mocked WebGPU device-loss recovery are
tested; physical native GPU loss remains an open acceptance gate.

## Build and verification

From the repository root:

```sh
bun install --frozen-lockfile
bun run --cwd apps/desktop build
CARGO_TARGET_DIR=~/Developer/pm-target CARGO_BUILD_JOBS=2 nice -n 10 ~/.cargo/bin/cargo test --manifest-path engine/Cargo.toml --target-dir "$(node apps/desktop/scripts/studio-cargo-target.mjs)" -p pm-studio -p pm-studio-host --features oidn,metalfx,yardstick
bun run --cwd apps/desktop test -- src/main/studio-native/manager.test.ts src/renderer/src/core/studio-native/backend.test.ts src/renderer/src/core/studio-native/recovery.test.ts
python3 apps/desktop/scripts/studio-native-smoke.py
node apps/desktop/scripts/studio-native-electron.mjs
PM_STUDIO_SCENES=hero PM_STUDIO_RECOVERY=1 PM_STUDIO_EMPTY=1 node apps/desktop/scripts/studio-native-electron.mjs
```

The app build compiles the host and N-API addon, copies OIDN and its dependency
closure, rewrites library references/rpaths, includes licenses, and signs the
local binaries ad hoc. `electron-builder.yml` copies `resources/studio` into
`Contents/Resources/studio`. Rust, Xcode tools, Node headers and OIDN 2.x are
build prerequisites (`OIDN_DIR` and `NODE_INCLUDE_DIR` can select installations).
Dev resolves the built resource directory or the shared Cargo target directory;
`POWERMOVE_STUDIO_HOST` can select an explicit development binary.

`PM_STUDIO_SCENES`, `PM_STUDIO_FRAMES`, and `PM_STUDIO_SHOTS` configure the
Electron corpus script. Additional fixtures are `room`, `forest` and
`room-ortho`. `PM_STUDIO_RECOVERY=1` kills the owned test process to verify
WebGPU recovery; `PM_STUDIO_EMPTY=1` checks empty-scene transitions.
`STUDIO_NO_RT`, `STUDIO_NO_RIS`, `STUDIO_NO_METALFX` and
`STUDIO_NO_SHARED_TEXTURE` exercise fallbacks. Screenshots and raw measurement
logs belong only in `/tmp/pm-shots/studio3d/laneB/`, never Git.

Validation completed: Rust library/host/GPU tests; TypeScript manager, binary
adapter and fallback tests (22 Rust/GPU and 17 targeted TypeScript tests in
total); full desktop typecheck (zero errors, two existing
Svelte warnings); desktop build; local `electron-builder --dir --mac --arm64
--publish never`; relocated bundled-host pixel smoke; real Electron shared
textures (also with the packaged host/addon), all eight corpus comparisons,
environment/orthographic fixtures,
crash recovery and empty-scene transitions. A no-default-features Cargo check
and a hardware-RT/MetalFX-disabled rendering smoke passed. Release signing,
notarization and publication were not run.

## Limits and remaining gates

- The product corpus meets the hidden-host frame-time target in the lower-load
  run above. That experiment did not compare Cycles. P4 separately passes the
  bounded eleven-scene Cycles regression; the full reference yardstick and
  full-editor performance targets remain separate acceptance gates.
- Live's first frame is noisy; temporal history improves it over subsequent
  frames. Glass, mirror edges and floor shadows still show artifacts before
  Refine. MetalFX is not used to judge Final/export parity.
- Object movement resets denoiser history; camera motion has reprojection.
  Per-object motion-vector reprojection is not implemented.
- Prepared geometry uses chunked uploads, while storage-binding and aggregate
  allocation limits still bound very large scenes. There is no out-of-core
  mesh/texture paging or mesh LOD in this host.
- Output is SDR RGBA8, up to 4096 pixels on an axis and 8,847,360 output pixels.
- Metal BLAS builds are submitted and completed separately. Batched builds
  dropped the intermediate glass-sphere BLAS on this M5/SDK; a hardware/BVH
  intersection regression test covers the fix. Transform drags reuse BLAS.

## Combined editor integration

The editor requests native Live at output resolution; the host controls tracing
resolution and MetalFX upscaling. Overlapping superseded depth sets release
sessions immediately, while opaque layers share a job and finished trace.
Prepared stills/exports retain async images until composition readback. Capture
readiness tracks only that capture's requested images and temporarily defers
viewer paints. Nested 3D compositions inherit Final quality. Unprepared
synchronous readbacks use Studio's WebGPU implementation.

The editor integration test verifies native pixels, actual Electron shared
texture transfers and repeated layer additions. The Final-quality regression
checks a 3D shot nested in a 2D composition against its configured sample goal.
Development launches resolve the built runtime beside the desktop project so
its surface addon and verified helper come from this checkout; no shared-cache
binary is used implicitly.

## Local packaged acceptance

From `apps/desktop`, run `bun run package:studio-local`, then
`node scripts/verify-studio-package.mjs`. An alternative generated `.app` path
can be passed as the first argument. The package is a Cargo release build with
OIDN and a hardened, ad-hoc app signature. This command never notarizes,
publishes, creates a release or installs over `/Applications/Powermove.app`.
Direct macOS packager calls also rebuild the host in `beforePack`.
The build uses a checkout-specific subdirectory under `CARGO_TARGET_DIR`
(default `~/Developer/pm-target`) and verifies the executable's actual protocol
and GPU data layout. A shared Cargo target was observed returning another
worktree's incompatible material/triangle layout, despite a successful build.
Never resolve a dev helper by guessing the shared cache's `release/` directory.
`node scripts/studio-cargo-target.mjs` prints this checkout's isolated target;
use it as Cargo's `--target-dir` when running the matching Rust tests.

Ad-hoc signatures have no Team ID, so this local command uses a separate
`entitlements.mac.adhoc.plist` with library validation disabled. Developer ID
release signing keeps its existing policy. Both local packaging and the release
verification path check the Studio executable, surface addon, OIDN CPU plugin,
TBB, licenses, matching hardened signatures and relocatable library load paths.

The acceptance harness clones the bundle into a disposable
`Applications/Powermove.app` directory and uses a temporary profile with normal
visible-window preferences (`POWERMOVE_BACKGROUND_TEST=0`). It first exercises
the unchanged production fuses through Chromium CDP. Playwright's Electron
main-process inspection requires the Node inspector fuse; only the disposable
copy enables that fuse and is then ad-hoc signed again. The supplied package
retains every production fuse. Results distinguish these two runs.

Acceptance covers native model/text/World/terrain/scatter/light rendering,
shared-texture transfers, changing render sizes, a native Final MP4 checked by
ffprobe, killing the host with unsaved edits, portable recovery, another edit,
save/quit/reopen and a fresh native render. Persistent layer/scene data is
compared with absent `solo` normalized to `false`, matching project migration.
The managed receiver releases its import reference after delivery; the backend
owns the VideoFrame until replacement or disposal. Each host slot returns once,
including failed receive/import/transfer paths, and retained surface handles
remain valid after the receiver port closes.

Synthetic evidence goes to `/tmp/pm-shots/studio3d/laneP1/` by default
(`PM_STUDIO_SHOTS` overrides it); it must stay outside Git. Temporary app copies
and profiles are removed. The harness prepares the normal close/save barrier;
for the known Electron 44 macOS native teardown stall, it reaps a process only
after its successful Node exit marker. The ordinary smoke process is separately
bounded and discarded. This is functional acceptance, not an editor performance
benchmark: the [production checklist](production-readiness.md#performance-acceptance-base-m5--16-gb)
defines the M5 timing/memory gates and the visible-editor benchmark entry point.

## Saved quality and frame budgets (Lane Q)

`studioQuality` is composition data, migrated to Production, guarded by normal
edits and Undo. The shared preset source is
[`studio-quality-presets.json`](../../apps/desktop/src/shared/studio-quality-presets.json);
[the Studio API guide](../../apps/desktop/docs/STUDIO.md#saved-render-quality-and-export-overrides)
lists every default, bound and export/agent override. No visual controls are
introduced by this engine change.

Native quality requests explicitly set noise threshold, floor, ceiling, bounce
limit, caustics, denoise and tracing deadline. The deadline now preempts the
noise floor. Saved quality uses one-sample submissions so a four-sample
dispatch cannot consume several seconds before checking that deadline. Explicit
Custom floors also override the legacy progressive-caustic safety minimum.
Pixel error needs at least two observations; raw/unlimited references retain their requested budget.
Layer masks reuse the completed bounded trace with error stopping on or off.
Custom long tracing limits receive a matching transport watchdog.

Production uses a fixed, hardware-traced photon prepass before camera samples.
This removes the drifting-map reason for DN2's per-pixel 512-sample floor.
Camera-only changes retain that scene-dependent map; geometry, materials,
lighting, bounds, bounce limits and texture changes invalidate it. Final (Max)
retains the progressive high-quality photon estimator. Scenes with no qualifying caustic
materials skip the photon work. No prior output image is reused temporally.

Eligible native shutter frames keep scene/BLAS state, distribute a common
camera-sample budget across the evaluated times, accumulate linear radiance and
path-following guides, and finish OIDN once. Equal sample counts per stratum
keep their weights equal. Styled/partly transparent layer boundaries, mixed
blur switches and WebGPU retain separate images. Export's existing queued
encoder write overlaps following GPU preparation; this change does not claim a
new encoder pipeline. Public layer identity, transmission and camera policies
remain covered by the existing tests.

The measurement harness is `engine/tools/render-quality.py`, using DN's SSIM,
CIEDE2000 and frozen-sequence flicker metrics without lowering acceptance
thresholds. Supply an exact PMS1 glass replay and independently rendered raw
references. It reads the application's preset JSON directly and records native
RT, actual samples, tracing/filtering/readback, cold setup, three warm timings
and eight advancing-seed frames. It does not count frozen images as moving-film
proof. All raw images, replays and logs remain outside Git.

```sh
nice -n 10 python3 engine/tools/render-quality.py \
  --host apps/desktop/resources/studio/pm-studio-host \
  --out /tmp/pm-shots/studio3d/laneQ/presets \
  --glass /absolute/path/to/glass.pms \
  --references /absolute/path/to/audited-raw-references \
  --sequence 8 --timing-runs 3 --check
```

Viewer Refine submits the selected ceiling once, stops on the same noise/time
policy and reports completion even when fewer samples were needed. A quality
edit resets that completion state and cancels an older Refine. Constant RGB
layers (notably unit catcher shading in closed rooms/skies) bypass the redundant
OIDN color pass; noisy companion layers and both auxiliary prefilters still run.

## Render-quality measurements — Lane Q, 10 November 2026

Apple M5, 16 GB; native hardware ray queries were active throughout. Times are
medians of three warm native 1920×1080 frames, including tracing, guides,
filtering and readback, excluding the encoder. A target warm-up is excluded;
the first quality request is recorded separately by the harness. Quality and
eight-frame advancing-seed flicker use frozen **320×180** scenes against audited
raw high-sample references (2048 samples, 16384 for room fog). These small
reference scores do not certify the appearance of a moving 1080p film.

Product shots retain the complete preset run; fog/landscape quality presets were
rerun after the exact-constant-layer optimization. Draft does not use OIDN.

| Scene | Preset | 1080p seconds/frame | SSIM | Mean ΔE00 | Flicker RMS |
|---|---|---:|---:|---:|---:|
| Glass | Draft | 0.451 | 0.8647 | 5.120 | 0.01995 |
| Glass | Preview | 3.476 | 0.9236 | 3.054 | 0.00925 |
| Glass | Production | 5.024 | 0.9909 | 0.610 | 0.00632 |
| Glass | Final | 24.163 | 0.9946 | 0.456 | 0.00470 |
| Product hero | Draft | 0.202 | 0.9820 | 0.398 | 0.01700 |
| Product hero | Preview | 2.678 | 0.9917 | 0.237 | 0.00700 |
| Product hero | Production | 4.465 | 0.9925 | 0.220 | 0.00321 |
| Product hero | Final | 22.819 | 0.9950 | 0.258 | 0.00205 |
| Chrome | Draft | 0.562 | 0.8779 | 2.628 | 0.01553 |
| Chrome | Preview | 3.162 | 0.9360 | 1.257 | 0.00976 |
| Chrome | Production | 4.551 | 0.9057 | 0.846 | 0.00696 |
| Chrome | Final | 24.412 | 0.9668 | 0.540 | 0.00714 |
| Room fog | Draft | 0.660 | 0.8299 | 2.697 | 0.02834 |
| Room fog | Preview | 3.547 | 0.9420 | 1.482 | 0.01328 |
| Room fog | Production | 5.172 | 0.9555 | 1.174 | 0.01120 |
| Room fog | Final | 22.924 | 0.9676 | 0.909 | 0.00625 |
| Default landscape | Draft | 0.673 | 0.7939 | 2.989 | 0.05058 |
| Default landscape | Preview | 3.023 | 0.8630 | 1.992 | 0.01299 |
| Default landscape | Production | 4.351 | 0.9167 | 1.527 | 0.00903 |
| Default landscape | Final | 23.290 | 0.9504 | 1.200 | 0.00618 |

All Production/Final rows pass the unchanged DN harness gates, SSIM ≥ .90 and
mean ΔE00 ≤ 4. Draft stays below 1 s and Production below 6 s in these median
native measurements. This is not a quiet-machine, full-editor shipping signoff:
other desktop work was present during this overnight session. The fixed-map
Production chrome image still shows caustic grain at native size; a passing
small-image score does not waive that issue.

### Actual moving glass export

The normal compositor and H.264 encoder produced eight 1920×1080, 30 fps frames
from source time 17.5 s. Production used two shutter times and one OIDN finish:
**5.784 s median native time**, 5.551–6.578 s range, and **52.808 s wall time**
including project setup and encoding. Actual camera counts were 30–40 per
frame: the tracing deadline can preempt the noise floor. Draft used one shutter
time: **.533 s median native time**, .526–.657 s range, **10.760 s wall time**.
Both files were independently checked for dimensions, duration and eight frames.
DN2's earlier 120-frame export measured 54.522 s/output frame including encoding;
the shorter current wall-time sample includes proportionally more startup.

Against matching frames 45–52 of DN2's cleaned movie, Production averages
**SSIM .99481, mean ΔE00 .661** at full 1080p. This is a compressed, denoised movie
comparison, separate from the independent raw-reference test. Cup-interior
SSIM is .9949; the cup-caustic region is .9541. The prism-face region averages
ΔE00 **4.839**, so its colour/detail acceptance remains open despite the strong
whole-frame score. Adjacent-frame high-pass RMS (Gaussian σ 1.5, decoded sRGB)
is .00191 versus DN2 .00248 in the cup interior, .00354 versus .00341 on the
prism, and .00426 versus .00418 in the caustic. These moving-image differences
include real motion and compression and are not isolated noise measurements.

### Verification and remaining gates

Rust: **120 passed**, including the Cycles corpus, unchanged tolerances, actual
GPU adaptive-floor/deadline tests and exact constant-layer preservation.
The full desktop unit suite and focused native four-view/quality/UX checks are
rerun on the combined branch; their final counts are recorded with delivery.
Svelte has zero errors with the existing IconCropper accessibility warning.

Four Views now retries temporary native capacity rather than permanently
moving a pane to WebGPU when a retiring preview occupies a slot. The four-host
memory cap stays intact. A regression covers Terrain, 18600+ scatter instances,
local/World Fog and temporarily occupied preview slots, requiring every side
pane to deliver a native picture. Refine completes on its bounded quality result
and a quality edit starts a new request. The UX `choices`, `fields` and `describe`
API additions are preserved.

**Still not ready to ship:** native-size chrome caustic grain, prism colour
fidelity, longer moving references for the other four scenes and quiet-machine
full-editor acceptance remain open. Styled depth sets and nested Studio
compositions can require several bounded traces; the per-trace time setting is
not an absolute wall-clock timeout. No temporal denoiser history is used for
exports. No renders, videos or local logs are committed.
