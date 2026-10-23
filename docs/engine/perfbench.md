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
reason, on both scopes; `scrub_24`'s every-thread heap counts have 0.5% (its
video decode thread allocates according to how far decoding ahead got: 0.067%
and 0.12% spread over three runs). Each is listed with its evidence in
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
frame of refresh k and waits for its `preview:presented` (both paths emit it):

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
the step's latency (request to presentation), and, from the engine's stats, its
work counters and three times:

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

- **playing**: `PM.play()` for 3 s on the 500-layer project at 120 fps: frames
  presented against the 360 a 120 Hz display would show, the gaps between
  presentations, the page's animation-frame rate (the hidden test window's
  display link), and per presented frame the page's main thread and the
  engine's compose CPU, GPU-only and span;
- **open**: the 2,000-layer project saved to a `.pmv` by today's app, then
  opened three times on each path through `PM.openProject`, each in a fresh app
  on a small project: time to its first presented frame, and the page's main
  thread over it;
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

The rig, release addon with the fixes above, 2026-10-22 22:00, load 12 to 15
(run A in `perfbench-measurements/rig-20261022.jsonl`; run B is the same rows at
load 31 to 47, run C the export rows at load 17). Stepped rows: 240 measured
steps after 4 warm-up (scrub 24 after 1, typing 60 after 3); times are p50 / p95.

| Row | Fit 8.33 ms | Page main thread | Main-thread tasks a step | Engine compose CPU | Engine GPU only | Engine span | Today WebGL GPU | Request → presented |
|---|---|---|---|---|---|---|---|---|
| Paused seeks, today | 227 of 240 | 6.66 / 8.37 ms | 1 | | | | 3.79 / 4.23 ms | 10.1 / 11.1 ms |
| Paused seeks, engine | 150 of 240 (span: 98) | 7.94 / 9.89 ms | 14 | 5.77 / 7.10 ms | 2.42 / 2.99 ms | 8.41 / 9.60 ms | | 15.6 / 22.3 ms |
| Paused edit, today | 0 of 240 | 25.0 / 29.6 ms | 1 | | | | 4.33 / 5.15 ms | 22.4 / 25.3 ms |
| Paused edit, engine | 0 of 240 | 28.6 / 34.2 ms | 21 | 3.58 / 3.92 ms | 1.56 / 1.80 ms | 5.18 / 5.52 ms | | 34.1 / 39.5 ms |
| Scrub 24 seeks, today | 24 of 24 | 6.98 / 7.37 ms | 18 | | | | 0.29 / 0.39 ms | 12.9 / 13.5 ms |
| Scrub 24 seeks, engine | 22 of 24 (span: 22) | 5.52 / 7.47 ms | 4 | 0.77 / 1.32 ms | 0.79 / 1.54 ms | 1.58 / 2.78 ms | | 3.1 / 9.7 ms |
| Typing 60 keys, today | 0 of 60 | 14.3 / 15.6 ms | 1 | | | | 0.38 / 0.53 ms | 12.5 / 13.5 ms |
| Typing 60 keys, engine | 0 of 60 | 16.8 / 18.8 ms | 17 | 1.79 / 2.84 ms | 0.91 / 0.97 ms | 2.71 / 3.77 ms | | 18.4 / 26.0 ms |

Real playback (`PM.play`, 3 s, 500 layers at 120 fps; the hidden window's page
ran animation frames at 114 Hz today and 120 Hz with the engine on):

| Row | Presented of 360 | Gap between presentations | Gaps over 12.5 ms | Page main thread per presented frame | Engine per rendered frame (CPU / GPU only / span) |
|---|---|---|---|---|---|
| Playing, today | 343 | 8.6 / 10.1 ms | 5 | 8.73 ms | |
| Playing, engine | 189 (224 rendered) | 16.1 / 25.8 ms | 114 | 12.5 ms | 6.37 / 2.49 / 8.86 ms |

Open and export:

| Row | Today | Engine |
|---|---|---|
| Open the 2,000-layer project, first frame (3 opens) | 433, 541, 552 ms (main thread 360 to 467 ms) | 683, 711, 859 ms (main thread 586 to 751 ms; engine compose 104 to 134 ms, span 156 to 197 ms) |
| Export 120 frames, H.264 1080p, load 13 | 4.00 s (33.3 ms a frame; main thread 3.23 s) | 3.14 s (26.2 ms a frame; main thread 0.55 s) |
| Export, load 17 (run C) | 8.05 s (67.1 ms a frame) | 5.28 s (44.0 ms a frame) |
| Export, load 40 to 47 (run B) | 11.6 s (96.9 ms a frame) | 36.0 s (300 ms a frame) |

What this says, as measured:

- **The engine is faster than today at scrubbing** (exact frame 3.1 ms p50 after
  the seek against 12.9 ms, 4 main-thread tasks against 18) and **at exporting**
  on a moderately loaded machine (26.2 against 33.3 ms a frame at load 13, 44
  against 67 at load 17, with a sixth of today's main-thread time).
- **It is slower everywhere else.** Real playback presents 189 of 360 frames
  (today 343), the paused rows add 5 to 12 ms of latency, and opening the large
  project takes 1.3 to 1.6 times as long. The engine's own render work is not
  the main cost in the interactive rows: the page's main thread with the engine
  on runs 14 to 21 tasks between a request and its presentation (today 1),
  posts and receives about 5 messages a frame (56 KB of structured clones), and
  spends 1.3 to 3.6 ms more per step than today's whole WebGL frame in the
  paused seek, edit and typing rows. In real playback the engine rendered 224
  frames and presented 189.
- **The export runs at the job engine's utility QoS** (contract 13), and on a
  heavily loaded machine that starves it: at load 40 to 47 it took 3.1 times as
  long as today's exporter, which runs on the page's main thread. The bench at
  the job engine's class (`perfbench --job-qos`, 3 alternating pairs at load 24
  to 27) is 1.3 to 1.6 times slower than at the default class (18 → 28,
  28 → 40, 26 → 34 ms a frame); the rest of the in-app gap at that load is not
  explained.

Per frame, today's WebGL viewer makes 557 draws, 607 framebuffer binds and
2,388 uniform sets on the paused seeks; the engine 549 draws in 33 render
passes, 32 uploads (148 KB), 5 bind group creations, 1 buffer allocation and 2
file stats. The paused edit plans 75 layers a frame in the app (550 before the
reuse fix).

An earlier V8 profile of the paused-seek steps (`PM_PERFBENCH_RIG_PROFILE`, load
11) put 17% of the page's sampled time (about 4.3 ms a frame) in
`getBoundingClientRect` from the timeline's `draw` on each presented frame: a
forced layout, because the engine path writes the DOM (overlay, presentation)
before the timeline reads it. The private projects did not reach measured
frames in the last attempt (today's viewer on the first one took more than 60 s
for a warm-up frame and the page stopped answering, twice; an earlier engine
run never reached its first frame); they were not run again this round.

## What remains

The next biggest gaps, from the rig against today (Results), largest first:

- **Real playback presents half the frames today does** (189 of 360 against
  343; the engine rendered 224). Where the 35 rendered frames went and why the
  engine renders at 75 fps when its compose CPU (6.4 ms) and GPU (2.5 ms) fit a
  120 Hz pipeline is the first thing to trace (presentation pacing, the
  descriptor path to the page). The bigrender lane owns large-project playback.
- **The page's main thread with the engine on** does more than today's whole
  WebGL frame: 14 to 21 tasks and about 5 messages per step against today's 1
  task, and a forced layout from the timeline's `getBoundingClientRect` after
  the engine path writes the DOM (cache the rect on resize). bigrender owns the
  page side.
- **Paused edits and typing** add 6 to 12 ms of latency over today, mostly on
  the page (the engine's own compose is 1.8 to 3.6 ms).
- **Opening the 2,000-layer project** takes 1.3 to 1.6 times as long; the
  engine's first frame allocates a target per group and precomp pass, all alive
  at once (117 targets, 811 MB). bigopen owns open.
- **Exports under load**: the job engine's utility QoS (contract 13) costs 1.3
  to 1.6 times in the bench and, at load 40 to 47, the in-app export took 3.1
  times as long as today's. Raising a user-started export to the default class
  while nothing plays is a plan decision (invariant 10), not made here.
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
