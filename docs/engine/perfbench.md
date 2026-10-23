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
allocation) and a process-wide relaxed atomic, so the counters are always on, in
release builds too. A `Scope` diffs a thread's (or every thread's) counters
around a span of work.

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
Every engine crate, Vello included, counts without touching its call sites, and
new code cannot bypass the counts. `wgpu-core`, `wgpu-hal`, and `wgpu-types`
are crates.io's. Moving to a new wgpu: copy the release in and re-apply the
lines marked `Powermove: pm-perf counters`.

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
readbacks, allocations, programs, shader compiles, links).

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

The paired before/after runs of each fix below are the direct evidence: for
example the bind group cache cut `gpu.bind_group_creations` 98 → 4 a frame and
moved playback by −1.5% (about 0.9 µs a creation), while the export fix cut
allocation bytes by 6 GB over 120 frames and moved the export by −34%.

## Ratchets

`engine/crates/pm-perfbench/baselines.json` holds, per bench, the highest count
allowed of every gated counter. `cargo test -p pm-perfbench` (and so
`cargo test --workspace`) runs every bench in the test profile and fails when
any count rises above its baseline, printing each rise. Counts are exact (two
consecutive full runs and four runs of three benches gave identical counts;
`cargo test -p pm-perfbench -p pm-node`, which builds pm-node with its test
hooks, too). The only tolerance: `export_120`'s heap counters, 1e-5 relative,
because the export thread parks waiting on the encoder thread a different
number of times per run (measured +1 to +2 allocations of 4.9 M).

`PM_PERF_RATCHET=update` rewrites the file: every count that went down is
lowered, a new bench is recorded, and nothing is ever raised. Raising a
baseline is a hand edit reviewed with the change that needs it. The baselines
are for the test profile (debug assertions allocate); release counts differ in
`cpu.*` only.

## The 120 Hz rig

`apps/desktop/e2e/perfbench-rig.spec.ts` drives today's viewer (Use New Engine
off) and the engine (on) through the same deterministic sequence of frames, one
per 120 Hz refresh (8.33 ms). Chromium's begin-frame control
(`HeadlessExperimental.beginFrame`) exists only in headless Chromium: Electron's
windowed renderer answers `'HeadlessExperimental.enable' wasn't found`, which
every row records. So the rig steps a virtual clock: step k asks for the frame
of refresh k and waits for its `preview:presented` (both paths emit it):

- playback: the time k/120 s on a 120 fps composition (every refresh a new
  frame); for the private projects, k/120 s snapped to their own frame grid,
  only refreshes that change the frame;
- edit: one layer moved a pixel through `PM.Edit.apply`, paused.

Per step it records the page's main-thread work (CDP `Performance.getMetrics`
`TaskDuration`, minus the rig's calibrated round trip), the engine's
render-thread compose CPU plus GPU time and work counters from its stats, the
page's messages and clone bytes, today's WebGL calls, and the step's latency. A
step **fits the budget** when the page's main-thread work and the engine's
render-thread time are each within 8.33 ms; the rest are **over budget** (they
would be dropped at 120 Hz). Fixtures: the synthetic project, and the projects a
manifest outside the repository names (`$PM_PERFBENCH_PRIVATE`: label, file,
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

Correctness evidence: `pm-export` `tests/frame_reuse.rs` (eight successive
frames through one renderer equal a fresh renderer's bit for bit, opaque and
transparent, f16 and f32) and `job_export` (sliced equals whole, pipeline equals
render); `pm-color` (the stack parser equals the old one on 47,000 strings);
`pm-render` `draw_reuse` with provenance edits in the fuzz (`PM_REUSE_FUZZ=8
PM_REUSE_FRAMES=800`: every plan and frame identical); the pm-render corpus
(357 frames compared, 0 failed, none skipped) with headless, composite, effects
and stills.

## Results

See the lane report for the run-by-run numbers (`/tmp/ne-w5/reports/perfbench.json`
outside the repository); the synthetic rows are summarized here.

## What remains

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
- **The engine misses the 120 Hz budget on the 500-layer project in the app**
  while today's page also does: see Results. The page's main thread with the
  engine on still spends about as long per frame as today's WebGL viewer
  (timeline, overlay and the descriptor path), which the bigrender lane owns.
- **Typing** allocates two textures per keystroke (the text raster target is
  not pooled, `text/gpu.rs`, which the enginemem lane is changing).
- **Opening a large project** allocates a target per group and precomp pass, all
  alive at once (117 targets, 811 MB, for the first frame at 2,000 layers).
- **Readbacks in export** still decode f16 to a fresh f32 frame (36 MB a frame of
  heap in the conversion to encoder input).
- Ungated counters need paired experiments before they can be gated: pipeline
  and shader creation (cold start), buffer allocations.
