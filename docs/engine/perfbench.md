# Deterministic performance counters, benches, ratchets, and the 120 Hz rig

The owner's goal (2026-10-22): make the engine measurably and permanently
faster than today's WebGL app while staying pixel-identical. Wall-clock time on
this shared Mac is noisy (1-minute load 6 to 60 from other lanes), so this lane
builds deterministic proxies for it, shows which of them track wall-clock, and
locks them in: once a number is exact and repeatable it can be driven down and
never allowed back up.

Everything here is measured on Jude's MacBook Pro (M4 Max, 128 GB, macOS 27,
120 Hz panel), with other lanes building and running Electron at the same time;
every measurement records its load.

## The counters (`pm-perf`)

`engine/crates/pm-perf` is a fixed catalog of counters with no dependencies.
Each add increments a const-initialized thread-local cell (no lock, no
allocation) and reads one relaxed flag; only while a bench has turned on
`pm_perf::count_globally` does it also do a relaxed atomic add to a
process-wide total (every thread's counts). The app never turns the totals on,
so in the shipped engine an add touches no cache line shared between threads.
The counters are always on, in release builds too. A `Scope` diffs a thread's
(or every thread's) counters around a span of work.

| Counter | Counted by |
|---|---|
| `gpu.render_passes`, `gpu.compute_passes` | `begin_render_pass`, `begin_compute_pass` |
| `gpu.draw_calls`, `gpu.dispatches` | every draw (passes and bundles, direct and indirect), every dispatch |
| `gpu.pipeline_binds`, `gpu.bind_group_binds` | `set_pipeline`, `set_bind_group` |
| `gpu.pipeline_creations`, `gpu.pipeline_layout_creations` | render, mesh and compute pipelines; layouts |
| `gpu.bind_group_creations`, `gpu.bind_group_layout_creations` | `create_bind_group`, `create_bind_group_layout` |
| `gpu.shader_modules` | every shader module (a backend compile) |
| `gpu.sampler_creations`, `gpu.texture_views`, `gpu.command_encoders`, `gpu.submits` | the matching calls |
| `gpu.buffer_allocs` / `_bytes`, `gpu.texture_allocs` / `_bytes` | `create_buffer`; `create_texture` (every mip and layer, times samples) |
| `gpu.uploads` / `gpu.upload_bytes` | `write_buffer`, `write_buffer_with`, `write_texture`, buffers created mapped, buffers mapped for writing |
| `gpu.readbacks` / `gpu.readback_bytes` | buffers mapped for reading |
| `gpu.copies`, `gpu.texture_to_buffer_bytes` | every recorded copy; the bytes of texture-to-buffer copies |
| `cpu.heap_allocs` / `cpu.heap_alloc_bytes` | `pm_perf::CountingAlloc`, installed by bench and test binaries (growing `realloc`s count their new size) |
| `engine.glsl_compiles` | GLSL programs through glslang (`pm-gpu` `glsl.rs`) |
| `engine.raster_requests` / `engine.raster_answers` | level 1 SVG rasters asked of the window and put back; fallback runner jobs and their pixels |
| `engine.layers_planned`, `engine.comp_passes` | layers planned from scratch; composition passes built (`pm-render` `plan.rs`) |
| `cache.{pipeline,program,media,vector,text,svg,draw_reuse,frame_prefix,bind_group,target_pool}.{hits,misses}` | effect pipelines per format, effect programs, media textures (stills and decoded frames), vector rasters, text rasters, SVG rasters, layer draw reuse, the kept frame bottom, kept bind groups, pooled targets |

**GPU calls without call-site edits.** `engine/third_party/wgpu` is wgpu
30.0.1's front crate, vendored unchanged except one marked counter line in each
resource creation, pass command, copy, upload, readback, and submit
(`POWERMOVE.md` there lists every change), patched in with `[patch.crates-io]`.
Every engine crate, Vello included, counts without touching its call sites, so
new code that goes through wgpu's API is counted. Not counted:
`create_texture_from_hal` and `create_buffer_from_hal` (they wrap memory made
elsewhere, such as IOSurface imports), and work below wgpu (`wgpu-hal` or Metal
directly: `pm-gpu`'s `decode_in`, `bridge_out` and `device.rs`). `wgpu-core`,
`wgpu-hal`, and `wgpu-types` are crates.io's. Moving to a new wgpu: copy the
release in and re-apply the lines marked `Powermove: pm-perf counters`; the
`[patch.crates-io]` applies to the whole workspace.

**Overhead.** Measured, not assumed (`examples/counter_overhead.rs`,
`perfbench-measurements/overhead-20261022.json`; release, load 20 to 25). One
add costs 2.6 ns on its thread (5.6 ns with the bench-only totals on, timed over
50 million adds); `playback_500` makes 2,529 adds a frame, `drag_500` 1,661 a
move, so the counters cost about 6.5 µs of a 6 to 15 ms frame (0.04 to 0.1%).
A wall-clock A/B against a `pm-perf/noop` build (every add compiled out) cannot
resolve that on this machine: 10 pairs gave the counting build +4.4% on the
frame p50, then 12 pairs with the order alternating gave it −5.5%. Both are the
machine's noise; the micro measurement is the figure.

**In the app.** The presenter's render thread counts each document frame's
compose (`pm-node` `present/render.rs`): `Stats.work` (every frame) and
`Stats.workLast` (the last one) are in the engine's status report
(`__powermoveNativeEngine.engine.report('interactive').presenter`). The heap
counters are zero there (the addon does not install the counting allocator).

**The page.** The rig (below) installs hooks before the app's scripts run:
messages posted and received by ports, workers and channels, their
structured-clone bytes (estimated by walking the payload: typed data by length,
strings two bytes a unit, eight a number), and today's WebGL calls (draws,
framebuffer and program binds, uniform sets, uploads and their bytes,
readbacks, allocations, programs, shader compiles, links). Per step it also
records, from CDP, the main-thread tasks begun between the request and the
presentation (`RunTask` events on the page's `CrRendererMain` in a `toplevel`
trace, bracketed by user-timing marks), forced layouts (`LayoutCount`) and
style recalculations (`RecalcStyleCount`).

**Instruction counts.** Not used. Valgrind does not run on macOS arm64 and
there is no Linux engine CI job on this machine. `xctrace` has a CPU Counters
template that records without root (a 1.6 s `typing_60` recording worked), but
its guided configuration reports cycles and bottleneck shares rather than
retired instructions, and scoping a count to a bench's span needs signposts;
the counts above are what the ratchets use.

## The benches (`pm-perfbench`)

Public, synthetic fixtures generated in code (`src/fixtures.rs`): the app's
synthetic project ported to Rust with the same seeded generator (mulberry32,
the same proportions of solids, images, groups of 20, precomps of 10 used
twice, and nulls; two generated 256 × 256 PNGs), a text layer, the public
`h264-aac.mp4` e2e fixture, and a generated 600,000-vertex GLB. Temporary files
are removed on drop.

The viewer benches drive the presenter's own path: `pm_node::present`'s view
document (replaced, then patched revision by revision), its routing decision at
each revision, and the composer drawing the viewer's 1496 × 904 frame at
DPR 2. Setup and two warm-up frames run before the counted span, except where
the cold work is the point.

| Bench | Unit | What |
|---|---|---|
| `playback_500` | frame | 120 consecutive playing frames of the 500-layer project |
| `drag_500` | move | 60 paused moves of one solid (x and y), a revision each |
| `inspector_edit_500` | edit | 60 paused moves as `PM.Edit.apply` writes them: the channel, the project's `revision`, an `edits` entry |
| `edit_playback_500` | frame | an opacity edit every frame while 60 frames play |
| `scrub_24` | seek | 24 exact paused seeks across the video, every length forward and back |
| `export_120` | frame | 120 frames of the 500-layer project to H.264 1920 × 1080 (VideoToolbox; the file is written and removed) |
| `typing_60` | key | 60 keystrokes into a 96 px SF Pro title, each a revision drawn paused |
| `open_large_2000` | open | the 2,000-layer project's text parsed, given to a new view, routed, and drawn by a new composer |
| `import_3d_600k` | frame | the first frame of a lit, shadowed scene placing the 600k-vertex model, then 10 frames turning it |

Each bench reports the bench thread's counters (the ratchet's numbers), every
thread's (informational), and wall-clock per unit including the GPU
(informational).

```sh
source engine/env.sh
cargo test -p pm-perfbench                                    # the ratchet
PM_PERF_RATCHET=update cargo test -p pm-perfbench --test ratchet   # lower baselines
cargo run --release -p pm-perfbench --example perfbench -- [bench ...] [--repeat N]
cargo run --release -p pm-perfbench --example perfbench -- --correlate --repeat 3
cargo run --release -p pm-perfbench --example alloc_sites -- playback_500 16 40
cargo run --release -p pm-perfbench --example perfbench -- export_120 --job-qos   # at the job engine's QoS class
cargo run --release -p pm-perfbench --example counter_overhead -- --micro          # the counters' own cost
```

`alloc_sites` samples every Nth heap allocation of a bench with a backtrace and
prints the source lines that allocate most per unit.

## Correlation

`perfbench --correlate` runs the workloads at several sizes (playback at 125,
250, 500 and 1,000 layers; drags at 250, 500 and 1,000 with the viewer's reuse
on and off; exports of 100 and 500 layers; models of 150k and 600k vertices;
the other benches once), three rounds interleaved, in release. For each counter
it reports Pearson r and Spearman rho of counts per unit against the median
wall-clock per unit, and paired agreement: among configurations of one workload
whose counts differ by more than 5%, in how many pairs wall-clock moved the same
way. 2026-10-22 20:05, load 9.2 to 10.1
(`perfbench-measurements/correlation-20261022.json`):

| Counter | r | rho | Paired | Gated |
|---|---|---|---|---|
| `cpu.heap_allocs` | 0.989 | 0.820 | 22/23 | yes |
| `cpu.heap_alloc_bytes` | 0.973 | 0.779 | 21/22 | yes |
| `gpu.uploads` | 0.896 | 0.850 | 20/21 | yes |
| `gpu.render_passes` | 0.890 | 0.850 | 20/21 | yes |
| `gpu.draw_calls` | 0.884 | 0.850 | 21/22 | yes |
| `gpu.pipeline_binds`, `gpu.bind_group_binds` | 0.884, 0.882 | 0.848, 0.846 | 21/22 | yes |
| `gpu.bind_group_creations` | 0.876 | 0.850 | 21/22 | yes |
| `engine.layers_planned` | 0.876 | 0.875 | 21/22 | yes |
| `engine.comp_passes` | 0.840 | 0.799 | 17/19 | yes |
| `gpu.upload_bytes` | 0.678 | 0.375 | 22/23 | yes |
| `gpu.texture_allocs`, `_bytes`, `gpu.buffer_alloc_bytes` | 0.973, 0.974, 0.603 | ~0 | 1/1, 1/1, 5/13 | yes: the export fix below moved them with a 34% faster export |
| `gpu.buffer_allocs` | 0.970 | -0.173 | 2/10 | no |
| pipeline, layout and shader module creations | 0.78 to 0.89 | 0.232 | none | no: one-time costs, only the open bench differs |
| others (compute passes, dispatches, samplers, GLSL compiles, views, encoders, submits, readbacks, copies, cache misses) | | | | no (reasons in `baselines.json`) |

Cache hits are benefits: reported, never bounded. A counter that does not
correlate is still printed by every run; it is gated again once a paired
experiment shows its cost.

**What the correlation does and does not show.** Across the set, the project
size (125 to 1,000 layers) moves almost every counter together, so r and rho
for `gpu.draw_calls`, `gpu.render_passes`, `gpu.uploads`,
`gpu.pipeline_binds` and `gpu.bind_group_binds` mostly say "bigger projects
cost more", not that each of these costs time on its own. The evidence that
isolates a counter is narrower:

- the fix pairs, each changing one thing at a fixed workload: bind group
  creations 98 → 4 a frame (playback −1.5%), layers planned 550 → 34 a paused
  edit (−67%), file stats 36 → 2 a frame (faster in 12 of 12 pairs), export
  GPU allocation bytes 1.99 GB → 16.9 MB (−34%);
- the drag pairs with the viewer's reuse on and off at a fixed layer count
  (250, 500, 1,000): draws 270 → 222, 548 → 291, 1,092 → 413 a move with
  wall-clock 3.69 → 2.51, 6.84 → 3.63, 13.47 → 5.81 ms. These isolate the draw
  path from project size, but not draws from planning: passes, uploads, binds,
  layers planned and heap allocations fall together in them.

No experiment isolates draw calls, passes, uploads or binds from each other.
They stay gated because a rise in any of them is more work for the render
thread and the GPU; the cost of a false alarm is a reviewed hand edit of the
baseline.

The paired before/after runs of each fix below are the direct evidence: for
example the bind group cache cut `gpu.bind_group_creations` 98 → 4 a frame and
moved playback by −1.5% (about 0.9 µs a creation), while the export fix cut
allocation bytes by 6 GB over 120 frames and moved the export by −34%.

## Ratchets

`engine/crates/pm-perfbench/baselines.json` holds, per bench, its units (120
frames, 24 seeks, ...) and the highest count allowed of every gated counter on
two scopes: the bench thread (`thread`: the engine's render thread or the
export's thread) and every thread (`all_threads`: decode, encoder and pool
threads too, so work moved off the bench thread still counts).
`cargo test -p pm-perfbench` (and so `cargo test --workspace`) runs every bench
in the test profile and fails when:

- any count on either scope rises above its baseline (each rise printed);
- a bench measures other units than its baseline (cutting a bench's frames is
  not an improvement);
- in a run of the whole set (`PM_PERF_ONLY` unset), a bench of the file did
  not run (a removed or renamed bench); `PM_PERF_ONLY` must name real benches.

GPU and engine counts are exact on both scopes (three consecutive full runs
gave identical counts; `cargo test -p pm-perfbench -p pm-node`, which builds
pm-node with its test hooks, too). The only tolerance is on the heap counters:
the render thread also runs wgpu-core's bookkeeping of submissions in flight,
which allocates according to how far the GPU has got when a frame polls
(measured spread over identical runs: up to 0.011% of allocations and 0.03% of
bytes), and the export thread parks on the encoder thread a different number
of times per run. Every heap counter has a 0.1% relative tolerance for that
reason, on both scopes; `scrub_24`'s every-thread heap counts have 3% (its
video decode thread allocates according to how far decoding ahead got: up to
1.9% of allocations and 1.1% of bytes over 12 runs at load 29; a full run once
measured 0.94% above its baseline). A run of some benches (`PM_PERF_ONLY`) can
count less than the whole set (state warmed by earlier benches: `scrub_24` alone
allocates 1.39 MB on its thread against 1.49 MB in the whole run), so lower
baselines from whole runs only. Each is listed with its evidence in
`baselines.json`; every other count is exact.

`PM_PERF_RATCHET=update` rewrites the file: every count that went down is
lowered and a new bench is recorded (printed, for review in the diff). It
never raises a count, never changes a bench's units, never records a counter a
known bench did not have, and refuses while a bench of the file did not run, so
a rename cannot record the new name at whatever level it runs: renaming,
removing or re-unit-ing a bench, or raising a baseline, is a hand edit reviewed
with the change that needs it. The baselines are for the test profile (debug
assertions allocate); release counts differ in `cpu.*` only.

**Where the ratchet runs.** Locally only, today. It needs Metal (on other
platforms the test prints that it is macOS-only and passes), and no CI workflow
runs the engine's `cargo test` (the `engine-wasm-identity` workflow builds and
tests `pm-wasm` only). So a lane's PR is gated only when its author or reviewer
runs `cargo test -p pm-perfbench` on a Mac. A macOS CI job needs the pinned
toolchain and the LGPL FFmpeg build (`PM_FFMPEG_DIR`) on the runner, and the
counts must be checked on the runner's GPU (the GPU counts are API calls and
should match; the heap tolerance may not): not done here.

## The 120 Hz rig

`apps/desktop/e2e/perfbench-rig.spec.ts` drives today's viewer (Use New Engine
off) and the engine (on) through the same deterministic sequence of frames, one
per 120 Hz refresh (8.33 ms). Chromium's begin-frame control
(`HeadlessExperimental.beginFrame`) exists only in headless Chromium: Electron's
windowed renderer answers `'HeadlessExperimental.enable' wasn't found`, which
every row records. So the stepped rows step a virtual clock: step k asks for the
frame of refresh k and waits for **that frame's pixels to be GPU-complete,
then the next page animation frame**. This is a compositor opportunity, not a
measurement of physical display scanout. The old `preview:presented` endpoints
were unequal: today emits synchronously at the end of `GL.render`, after issuing
WebGL commands but before GPU completion; the engine reports its presentation
draw, then `PresentedEvents` emits at most one bus event per rAF (a timed
frame at the first page rAF of a later refresh than the one that drew it).
The direct presentation callback can precede the page's WebGPU
completion too. The rig now uses:

- today: `gl.fenceSync` after the final `GL.render` commands, `gl.flush`,
  nonblocking `clientWaitSync` polling with timers, then one rAF after the fence
  signals (polling itself does not consume another rAF);
- engine: the matching frame's direct `onPresented` identity joined to the
  presenter's `onDone(seq)` (the page/worker WebGPU draw's `workDone`), then one
  page rAF. This bypasses `PresentedEvents`' additional hold. Cold reveal can
  report the identity after GPU completion; the rig remembers completions and
  observes replacement presenters during open.

`stepLatencyMs` is this common endpoint. `oldEndpointMs` retains the previous
endpoint (bus event, except engine scrub which already used direct
`onPresented`); `busPresentedMs`, `drawCallbackMs`, and `gpuCompleteMs` expose
the individual milestones. Old endpoint columns are diagnostic, not comparable
latencies between paths. The stepped operations are:

- paused seeks: the time k/120 s on a 120 fps composition, set while paused
  (every refresh a new frame); for the private projects, k/120 s snapped to
  their own frame grid, only refreshes that change the frame. This is the
  stepped twin of `playback_500`, but it does not exercise the playing path
  (the composer's playing mode, decode-ahead): the `playing` row does;
- edit: one layer moved a pixel through `PM.Edit.apply`, paused;
- scrub: `scrub_24`'s 24 seeks on its document (the public `h264-aac.mp4`
  fitted to 1920 × 1080 under a sliding solid). A step ends at the engine's
  exact frame (stand-ins skipped), or, today, at the first presented frame
  after the video element completed its seek to that time;
- typing: `typing_60`'s 60 keys into the 96 px title, each a `set_content`
  through `PM.Edit.apply` (the document path both benches take; the canvas
  text tool's session is not driven).

Per step it records the page's main-thread work (CDP `Performance.getMetrics`
`TaskDuration`, minus the rig's calibrated round trip), its tasks, layouts and
style recalculations, the page's messages and clone bytes, today's WebGL calls,
the step's latency (request to GPU-complete plus next rAF), and, from the
engine's stats, its work counters and three times:

- compose CPU (`composeUsTotal`: planning, encoding, submission);
- span (`gpuUsTotal`): from the compose's start to the GPU finishing the frame
  (`pm-node` `record_frame` measures `gpu_done − start`, so the span already
  contains the compose CPU);
- GPU only: span − compose CPU.

Today's rows time the WebGL work of each step on the GPU with a `TIME_ELAPSED`
query (`EXT_disjoint_timer_query_webgl2`) on the viewer's context from the
request to the presentation, where the project has no shader or effect layers
(the app times those itself, and queries cannot nest); otherwise the row says
the GPU time was not measured.

A step **fits the budget** when it was presented and every measured stage is
within 8.33 ms: the page's main thread; for the engine its compose CPU and its
GPU-only time (the stages of a pipelined 120 Hz loop); for today its WebGL GPU
time. The engine rows also report **spanFit**, the serial criterion: compose
start to GPU done within 8.33 ms. The rest are **over budget** (they would be
dropped at 120 Hz). The first rig report summed compose CPU and span, counting
the compose twice; its engine render times (13.27 ms playback, 7.51 ms paused
edit, 13.0 ms before the reuse fix) were wrong and are withdrawn.

Rows that are not frame steps:

- **playing**: `PM.play()` for 3 s on the 500-layer project at 120 fps
  (`PM_PERFBENCH_RIG_PLAY_FPS` for another rate): frames presented against the
  360 a 120 Hz display would show, distinct frame times on the bus, frames
  missing between the first and last shown (`droppedFrames`), the gaps between
  presentations (longest, and their standard deviation as judder), the time
  from `PM.play()` to the first later frame (`firstAdvanceMs`: the engine's
  50 ms preroll), the page's animation-frame rate (the hidden test window's
  display link), and per presented frame the page's main thread and the
  engine's compose CPU, GPU-only and span. It retains bus-event counts/gaps and
  counts engine direct callbacks, GPU draw completions, bus emissions,
  emissions passed over for a newer one, and pending reports at the
  measurement's boundaries. Presented counts completed draws reaching a
  following rAF, not physical scanouts. With `PM_PERFBENCH_RIG_TRACE=<file>`
  the row also writes a per-frame trace there (outside the repository): the
  page's animation frames, reports and emissions; the viewer presenter's
  arrivals, draws and drops; the engine's per-frame record (render start, GPU
  done, sent, the time it shows from; `POWERMOVE_BRIDGE_TRACE`); and viz's
  presentation feedback for each submitted canvas frame
  (`e2e/helpers/compositor-timing.ts`: which display frame showed it, and
  submissions another one superseded). `PM_PERFBENCH_RIG_ORDER=engine` runs
  the engine first, for paired runs that alternate;
- **open**: the 2,000-layer project saved to a `.pmv` by today's app, then
  opened three times on each path through `PM.openProject`, each in a fresh app
  on a small project: time to its first GPU-complete frame plus next rAF, the
  old bus endpoint (`oldFirstFrameMs`), and the page's main thread over it;
- **export**: 120 frames of the 500-layer project at 30 fps to H.264 1920 × 1080
  through the export sheet's `PM.Export.run` (`quality: 'high'`): wall time,
  frames in the file (ffprobe), the muxer tag (FFmpeg's `Lavf` is today's
  exporter; otherwise the job engine's AVAssetWriter), and the page's main
  thread.

Fixtures: the synthetic project, the public clip, and the projects a manifest
outside the repository names (`$PM_PERFBENCH_PRIVATE`: label, file,
extensions folder; only labels are reported, and every copy is removed).

```sh
PM_PERFBENCH_RIG=1 pm-electron-lock bunx playwright test e2e/perfbench-rig.spec.ts --workers=1 --global-timeout=0
```

Results are in the section "Results" below.

## Fixes

Each fix is pixel-identical (named tests and gates), committed with its counts
and paired wall-clock, and ratcheted.

| Fix | Counts (before → after) | Wall-clock (release) |
|---|---|---|
| **Export reads through one staging buffer into a recycled target** (`Renderer::read_rgba_reusing`, `Renderer::recycle`; a single band is the frame) | `export_120`: buffer bytes 1.99 GB → 16.9 MB; textures 152 → 33 (2.33 GB → 359 MB); heap bytes 10.31 → 4.34 GB (−49.8 MB a frame) | 23.3–23.8 → 15.0–15.7 ms a frame (−34%; 3 runs each, load 12–15) |
| **Hex colors parsed on the stack** (`pm-color`) | `playback_500` heap allocs 4,866,001 → 4,750,321 (−964 a frame) | within noise |
| **Draw reuse kept across the project's provenance edits** (`revision`, `edits`; `pm-render` `reuse.rs`) | `inspector_edit_500`: layers planned 33,000 → 2,040; heap allocs 2.70 M → 0.85 M | 5.10–5.23 → 1.68–1.74 ms p50 an edit (−67%) |
| **Composition passes' bind groups kept across frames** (`pm-render` `binds.rs`) | `playback_500` bind group creations 11,760 → 480 (98 → 4 a frame); heap −1,384 a frame | playback total −1.5% (5 alternating pairs, after faster in 4) |
| **Each media file's stamp read once per frame and per routing check** (`media::StampFrame`) | `engine.file_stats` (a stat each): `playback_500` 4,320 → 240 (36 → 2 a frame), `drag_500` 3,372 → 240, `open_large_2000` 224 → 4 | after faster in 12 of 12 alternating pairs (playback −2% to −21%, drag −2% to −5%, inspector edit −2% to −8%; load 16) |
| **Export pixel buffers circulate between render and encoder** (`Renderer::give_rgba`, three buffers) | `export_120` heap bytes 4.32 GB → 0.44 GB (−32.4 MB a frame) | total −5.3% (5 alternating pairs, after faster in 4; load 26–30) |
| **The timeline's timecode changes its text in place** (`timeline.ts` `syncTime`; the node was replaced each tick, and a child list change rechecks the page's `:has()` rules) | Chromium trace of 44 playback steps: the forced `UpdateLayoutTree` under the timeline's size read, 1,236 elements and about 7.6 ms each, gone | rig playback, page main thread p50 today 11.11 → 4.96 ms (fit 3 → 236 of 244), engine 11.03 → 4.69 ms (load 49–56) |
| **The titlebar keeps its tab list while its tabs show the same** (`Titlebar.svelte`; every edit rebuilt the keyed list, whose `animate:flip` measured each tab twice) | shell test: 5 unchanging edits measure no tab (20 before) | rig paused edit, page main thread p50 today 35.84 → 19.88 ms, engine 32.14 → 19.76 ms (load 42–45) |
| **The home and settings screens' `body:has()` rules look at body's children only** (`app.css`, `settings.css`: `body:has(> #…-screen.on)`) | trace of 44 paused edits: `UpdateLayoutTree` 247.0 → 13.6 ms (1,257 → 4 elements each); no body subtree invalidation | rig paused edit, engine page main p50 19.76 → 17.58 ms (load 42 → 66) |
| **Channel memo** (`pm-eval` `channel_memo`: a channel's static value or prepared track kept across an unchanged document's frames, by layer address, valid while `pm_model::Document::edition` holds) | `playback_500` heap allocs 4,549,081 → 4,342,681 (release, −1,720 a frame); plans identical | `playback_500` p50 median 8.82 → 7.16 ms (4 rounds, load 13–20); 6.05 → 5.25 ms (load 10) |
| **Group ancestry kept by the same memo** | `playback_500` heap allocs 4,342,681 → 3,704,281 (−5,320 a frame) | median 12.48 → 11.60 ms (3 rounds, load 37–40) |
| **Static local matrices kept by the same memo** (all eight channels static) | `playback_500` heap allocs 3,704,281 → 3,408,001 (−2,470 a frame) | within noise (6 pairs, load 10: medians 4.79 → 4.61 ms) |
| **A nested composition planned the same twice renders once a frame** (`exec.rs` `comp_twins`: twins found before drawing, targets keyed by plan address, no plan copies; dropped when a motion-blur sample gives its targets back) | `playback_500` render passes 32 → 27 a frame, draws 548 → 498, uploads 32 → 27; heap 28,400 → 28,235 a frame | after faster in 5 of 6 pairs (median 6.93 → 6.64 ms, load 13) |
| **Compositing nodes give back each accumulator a node replaced** (`composite/exec.rs` `take_scoped`/`release`, as today's compositor frees one after its last use; to the frame's end with the depth and id surface) | `blend_stack_cold` (new): textures 15 → 11, 141.1 → 100.8 MB; target pool misses 13 → 9 | cold counts |

The export reuse is held only while an export runs: `render_frame` (agent
stills, parity frames, any one-off frame) releases the staging buffer, target
and pixels after its frame, an export's frames go through
`render_frame_reusing` and release them at the export's end, and
`Renderer::trim` drops them (`frame_reuse.rs`
`one_off_frames_and_trims_keep_nothing`: 0 bytes held after a still, a one-off
frame and a trim). Before, a job renderer kept a frame-sized target and staging
buffer idle after an agent still.

Correctness evidence: `pm-export` `tests/frame_reuse.rs` (eight successive
frames through one renderer equal a fresh renderer's bit for bit, opaque and
transparent, f16 and f32) and `job_export` (sliced equals whole, pipeline equals
render); `pm-color` (the stack parser equals the old one on 47,000 strings);
`pm-render` `draw_reuse` with provenance edits in the fuzz (`PM_REUSE_FUZZ=8
PM_REUSE_FRAMES=800`: every plan and frame identical); the pm-render corpus
(357 frames compared, 0 failed, none skipped) with headless, composite, effects
and stills.

## Results

The rerun with corrected endpoints used the release addon with the fixes
above, 2026-10-22 22:34 (Pacific), one-minute load 44.3 to 70.5 on 16 logical CPUs
(`perfbench-measurements/rig-endpoints-20261022.jsonl`, run A2). The original
run A was at load 12–15. Concurrent work prevented matching that load; the
rerun records load at each row and at the start of the stepped rows. A2 ran
after a 180 s cooldown and had the lower mean recorded row load (51.6 versus
62.1 for the first pass, B2, in the same file). These
are within-run comparisons, not a controlled isolation of the endpoint change
from load. Stepped rows: 240 measured steps after 4 warm-up (scrub 24 after 1,
typing 60 after 3); times below are p50 / p95 in ms.

The old latency verdicts (“paused rows add 5 to 12 ms” and “slower everywhere
else”) are withdrawn: their endpoints differed. The corrected comparison is:

| Row | Today: GPU done + next rAF | Engine: GPU done + next rAF | Today: old endpoint | Engine: old endpoint |
|---|---|---|---|---|
| Paused seeks | 14.40 / 18.90 | 20.10 / 25.90 | 7.30 / 11.90 | 18.80 / 24.20 |
| Paused edit | 34.10 / 39.40 | 38.10 / 44.30 | 26.20 / 29.80 | 32.90 / 41.30 |
| Scrub 24 seeks | 18.80 / 27.40 | 14.60 / 21.30 | 10.80 / 18.50 | 3.70 / 12.30 |
| Typing 60 keys | 24.00 / 28.40 | 29.50 / 32.30 | 14.20 / 15.30 | 22.00 / 27.10 |

The old endpoint is the bus event, except engine scrub's direct presentation
callback; those two columns are retained for diagnosis only. The common
endpoint waits for GPU completion and a compositor opportunity on both paths;
it does not prove when a monitor scanned out the frame.

Stage measurements and the unchanged 8.33 ms fit criterion:

| Row | Fit 8.33 ms | Page main thread | Main-thread tasks a step | Engine compose CPU | Engine GPU only | Engine span | Today WebGL GPU |
|---|---|---|---|---|---|---|---|
| Paused seeks, today | 0 of 240 | 10.49 / 13.37 | 8.00 / 11.00 |  |  |  | 3.42 / 4.71 |
| Paused seeks, engine | 24 of 240 (span: 0) | 8.86 / 10.12 | 20.00 / 26.00 | 6.53 / 7.10 | 2.38 / 2.96 | 8.94 / 9.80 |  |
| Paused edit, today | 0 of 240 | 30.80 / 37.02 | 13.00 / 19.00 |  |  |  | 4.50 / 10.07 |
| Paused edit, engine | 0 of 240 (span: 0) | 28.04 / 35.63 | 25.00 / 30.00 | 3.57 / 4.02 | 1.52 / 2.27 | 5.08 / 6.05 |  |
| Scrub 24 seeks, today | 0 of 24 | 9.18 / 10.28 | 34.00 / 37.00 |  |  |  | 0.28 / 0.72 |
| Scrub 24 seeks, engine | 11 of 24 (span: 11) | 8.35 / 10.06 | 15.00 / 26.00 | 0.86 / 1.55 | 0.90 / 1.43 | 1.74 / 2.97 |  |
| Typing 60 keys, today | 0 of 60 | 16.16 / 17.91 | 23.00 / 27.00 |  |  |  | 0.35 / 1.02 |
| Typing 60 keys, engine | 0 of 60 (span: 0) | 18.87 / 21.00 | 23.00 / 26.00 | 2.05 / 3.02 | 0.85 / 1.25 | 2.91 / 4.16 |  |

The latency is not a fit-count gate. Main-thread time/tasks now cover the
longer common endpoint, including the rig's WebGL fence polling and the next
rAF's work; task counts are not application-only task counts. Stage values
from the old endpoint cannot be treated as measurements of this longer span.

Real playback (`PM.play`, 3 s, 500 layers at 120 fps). “Completed + rAF” counts
completed draws reaching a following page rAF, not distinct physical scanouts.
The old bus event counts/gaps are retained in the JSON.

| Row | Completed + rAF / expected | Old bus events | Page rAF Hz | Gap p50 / p95 | Page main thread per completed + rAF | Engine per rendered frame: CPU / GPU only / span |
|---|---|---|---|---|---|---|
| Playing, today | 271 / 360 | 273 | 102.5 | 10.00 / 21.90 | 11.03 |  |
| Playing, engine | 35 / 360 | 35 | 119.9 | 27.60 / 164.90 | 80.12 | 8.02 / 3.01 / 11.03 |

The engine rendered 101 frames, reported 35 direct presentation callbacks,
and completed 35 of those page draws within the window. `PresentedEvents`
received 35 pushes, replaced a pending emission 0 times, and emitted
35 bus events (none pending at either boundary). No reports were merged
in this rerun; the remainder of the rendered-versus-bus gap
cannot all be called dropped displayed frames. Engine rendering, delivery,
selection, and the measurement boundaries still need separate tracing.

Open and export (ms unless marked s):

| Row | Today | Engine |
|---|---|---|
| Open 2,000 layers, GPU done + next rAF (3 fresh opens) | 615.6, 593.9, 559.8 | 894.3, 950.9, 885.9 |
| Open, old bus endpoint (same opens) | 595.0, 575.2, 541.4 | 894.2, 950.8, 885.8 |
| Export 120 frames, H.264 1080p | 4.56 s (38.0 ms/frame; 120 frames verified) | 50.23 s (418.6 ms/frame; 120 frames verified) |

What this says in this rerun:

- **Latency to the common endpoint varies by operation.** Engine minus today at
  p50: paused seeks +5.7 ms; paused edit +4.0 ms; scrub 24 seeks −4.2 ms;
  typing 60 keys +5.5 ms. These replace the old endpoint comparison; there is no “slower everywhere else” conclusion.
- **Playback remains below a 120 Hz cadence:** the engine completed
  35 draws through the next-rAF endpoint against today's
  271. The bus-event count is a separate, merged signal.
- **Open's median latency to the common endpoint** was 894.3 ms with
  the engine against 593.9 ms today; **export** took
  50.23 s against 4.56 s. Neither is a general verdict across loads.
- **Export still runs at the job engine's utility QoS** (contract 13). The
  earlier load-40–47 run B took 36.0 s with the engine against 11.6 s today;
  at load 13 the old A took 3.14 s against 4.00 s, and at load 17 run C took
  5.28 s against 8.05 s. Export endpoints did not have the presentation mismatch.
  The job-class bench (`perfbench --job-qos`, three alternating pairs at load
  24–27) was 1.3–1.6 times slower than default class; the rest of the loaded
  in-app gap is not explained.

An earlier V8 profile of paused-seek steps (`PM_PERFBENCH_RIG_PROFILE`, load
11) put 17% of sampled page time (about 4.3 ms/frame) in the timeline's
`getBoundingClientRect`: a forced layout after the engine path writes the
DOM. This is historical profile evidence, not a new profile of A2. Private
projects were not rerun: their earlier attempts stalled before measured frames.

B2 recorded 197 engine renders, 169 direct callbacks, 20 replacements of
pending emissions, and 149 bus events. Its 169 completed draws through the
next-rAF endpoint (against A2's 35) also show substantial playback variation
under contention.

## What remains

- **Cold texture bytes (colordepth lane's counter comparison).** Two cold
  benches ratchet `gpu.texture_alloc_bytes`: `blend_stack_cold` (six
  blend-mode layers' first frame, 1920 × 1080: 11 textures, 100.8 MB after the
  accumulator fix; today's compositor holds three) and `text_cold` (a 96 px
  title: 12 textures, 24.6 MB, a 2048 × 2048 glyph atlas page among them).
  The remaining blend-stack targets are layer images and their content
  targets, which `layer_node` puts in `taken` (giving them back needs the node
  loop to own them; the depth and id surface reads them). The atlas could
  start at one 512² plot and grow in powers of two (texel coordinates times a
  power-of-two inverse size are exact), sampling past a small page's edge as
  zero with a clamp-to-border sampler (`ADDRESS_MODE_CLAMP_TO_BORDER`, an
  optional device feature), which is what the full page's unused, zeroed
  plots read; growth copies the page through its own submitted encoder, and
  each draw's `atlasSizeInv` is read after `locate`. Not yet done.
- **Instanced solids.** Encoding is about 20% of a playback frame; per draw,
  `set_bind_group` with a dynamic offset and `draw`. Consecutive solids in a
  pass could be one instanced draw reading the same quad data from a storage
  array (primitive order is kept within a draw).

The remaining gaps and uncertainties from the corrected rig (Results):

- **Playback pacing and event accounting.** The original A's 224 rendered
  versus 189 “presented” was a comparison to bus emissions, not a count of 35
  known dropped pictures. Engine presented emissions are merged to one per
  rAF (newest wins), which may explain part of that gap. A2 measures
  101 rendered, 35 direct callbacks, 0 replacements of pending emissions,
  and 35 bus events. That measures merging in A2; it cannot retroactively
  partition the old 224/189 gap. The other corrected pass (B2) measured
  20 replacements (169 direct callbacks to 149 bus events), so
  merging's contribution varies with pacing. Trace rendering, descriptor delivery,
  selection, GPU completion, and commit/scanout separately. bigrender owns
  large-project playback.
- **The page's main thread** still includes forced layout from the timeline's
  `getBoundingClientRect` after the engine path writes the DOM; cache the rect
  on resize (bigrender). The corrected task span includes rig fence polling
  and next-rAF work, so the old “today 1 task versus engine 14–21” is not a
  comparison of application-only work through the common endpoint.
- **Interactive latency** needs quieter paired runs: engine minus today
  at p50 in A2 is +5.7 ms for paused seeks, +4.0 ms for paused edits,
  −4.2 ms for scrub, and +5.5 ms for typing. The old blanket “6–12 ms slower” paused-edit/typing claim is
  withdrawn; use the corrected table and its recorded load.
- **Opening 2,000 layers:** continue profiling the first frame's 117 targets
  (811 MB) alive at once; release group/precomp targets as passes finish.
  bigopen owns open. Use the corrected first-frame endpoint above.
- **Exports under load:** utility QoS costs 1.3–1.6 times in the job-class
  bench and the earlier loaded app run took 3.1 times today's export duration.
  Raising a user-started export to default class while nothing plays is a plan
  decision (invariant 10), not made here.
- **Playback planning allocates about 38,000 times a frame** at 500 layers
  (`alloc_sites playback_500`): JavaScript-string clones in `pm-eval`
  (`node.rs` `js_value`, `scope.rs` channel keys, `comp.rs` id keys: about
  14,000), string conversions in `pm-render` `js.rs` (`member_string`,
  `layer_id`, about 2,700), keyframe tracks prepared again on every read
  (`Evaluator::ev` → `keyframes::evaluate`: about 1,750 allocations, 350 track
  preparations), group ancestry (about 2,300), content resolution
  (`resolve_content`, about 1,650). The bigopen lane's inline `JsString`
  (`feat/ne-bigopen`) removes most of the first group; the ratchet will show it
  and `update` locks it in.
- **Typing** allocates two textures per keystroke (the text raster target is
  not pooled, `text/gpu.rs`, which the enginemem lane is changing).
- **Readbacks in export** still decode f16 to a fresh f32 frame (36 MB a frame of
  heap in the conversion to encoder input).
- Ungated counters need paired experiments before they can be gated: pipeline
  and shader creation (cold start), buffer allocations; draw calls, passes,
  uploads and binds are gated without an experiment that isolates each
  (Correlation).

Not measured on today's side, and why:

- `edit_playback_500` (an edit every frame while playing) has no rig row: the
  rig does not drive edits during real playback yet.
- `import_3d_600k` has no rig row: today imports models through three.js and
  the rig does not drive a model import on either path yet.
- The private projects: not run this round (above).
- Today's WebGL GPU time is measured only on projects without shader or
  effect layers (the app's own timer queries cannot nest with the rig's).
