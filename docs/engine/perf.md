# Latency on the Phase 2 rows: the page and the engine

Design and measurement notes for the page and engine costs behind two Phase 2
performance rows (plan section 25; `phase2-gate.md`): layer drag on the
synthetic 500-layer project (p95 ≤ 33 ms pointer-to-pixel, at most one frame
worse than today, idle and under load) and inspector edits during playback at
1,000 layers (p95 ≤ 16 ms, no dropped frames). Builds on `tracer.md` (the
edit path, the transport, the first profile), `bridge.md` (presentation, the
viewer's jitter buffer), `pm-render.md` (the render graph), `pm-eval.md` (the
evaluator's caches), section 8 (render graph, caching), and section 15
(playback, render-ahead).

The frame bridge and the viewer's presentation stay the bridge lane's; this
lane changes the page's edit and query paths, pm-eval and pm-wasm hot paths,
pm-render's compose cost, and the presenter's render-ahead schedule.

## Profile (before any change)

Measured 2026-10-16 on the integration tip `2b7b1346` (waves 1 to 4) on
Jude's MacBook Pro (M4 Max, 128 GB, macOS 27, 120 Hz built-in display),
Electron 44, release addons, one Electron at a time under the machine lock,
hidden and unfocused windows. The 1-minute load average was 6.5 to 7.7 from
other lanes. Page profiles are V8 CPU profiles of the editor window taken
through the harness (`PM_MEASURE_CPU_PROFILE`, 500 µs sampling,
`scripts/cpuprofile-summary.mjs`); the engine profile is macOS `sample` over
`compose_bench` (1 ms sampling). Profiling adds overhead, so the latencies in
this section are the profiled runs' and are not gate numbers.

### Layer drag, 500 layers, Use New Engine on

`tracer-drag-engine-500-idle` with a profile: pointer-to-pixel p50 33.3, p95
48.8, p99 50.1 ms (2,398 of 2,401 presented); the same run with Use New
Engine off: p50 20.9, p95 28.6, p99 36.2 ms. Hops (p50/p99): input to handled
2.1/4.8, handled to the frame's arrival 17.8/26.1, arrival to submit 0.4/0.6,
submit to received 1.9/8.7, received to presented 10.8/20.3 ms. Everything
after arrival is presentation (bridge lane); "handled to arrival" is the page
sending the edit, the engine applying and composing it, and the descriptor
reaching the page's main thread.

The page's main thread, 20.5 s sampled over the 30 s run:

| Where | ms | Note |
|---|---|---|
| idle | 5,606 | 27% (46% with the engine off) |
| pm-wasm queries for the UI (`NativeApi.answer`) | 6,958 | 34% of the thread |
| — `groupAncestors` | 3,312 | 2,183 from the timeline's `buildRows` (one per row per redraw) |
| — `wouldCycle` | 2,175 | the inspector's parent menu (`parentOptions`, one per candidate layer per change) |
| — `is3DLayer` | 930 | the move tool, per pointer move |
| — `bounds`, `transformRoots`, `hasKeyAt`, others | 600 | |
| Svelte effects and deriveds | 3,915 | mostly `parentOptions` above |
| Timeline redraw (`drawInner`) | 3,330 | `buildRows` is 2,375 of it (the queries above) |
| Viewer overlay (`frame` → `drawOverlay`) | 1,552 | |
| 3D viewport controller (`render` → `compositionRuntime`) | 462 | runs with no 3D layer in the project |
| Frame descriptors received (`receive`) | 405 | |

The queries are what the engine path adds: with the flag on, the mirror's query
routing is on and every synchronous query goes through pm-wasm (a JSON request,
the barrier's flush check, the identity registry, a JSON answer: 4 to 10 µs
each). The legacy answers are memoized per animation version (`layer-groups.ts`)
or are a few property reads. Structural queries dominate (`groupAncestors`,
`wouldCycle`, `is3DLayer`, `transformRoots`): their answers depend only on the
layers' `id`, `parent`, `group`, `type`, and `threeD` and on the layer order, and a
drag changes none of them.

### Inspector edits during playback, 1,000 layers

`tracer-edit-playback-engine-1000` with a profile (an Opacity edit through
`PM.Edit.apply` every 250 ms for 60 s, as the Properties panel applies it):
apply p50 46.9, p95 134.9 ms; edit to presented p50 162.2, p95 186.1 ms; 45
of 1,793 frames dropped (the gate's run: 381).

Per edit, from the 238 edits' 12,948 ms inside `PM.Edit.apply`:

| Where | ms per edit | Note |
|---|---|---|
| Crash-recovery journal (`history:project-patch` → `projects.ts`) | 35.5 | reads the whole journal (`structuredClone`), measures it with `JSON.stringify`, clones and writes it again |
| Composition tabs (`comp-tabs.ts` `render`) | 13.6 | rebuilt three times per edit (`history`, `layers`, `project`), each ending in `scrollIntoView`, a forced layout of the editor |
| `PM.UIState.prune` | 1.7 | twice per edit, a walk of every layer and keyframe |
| Viewer layout (`V.layout`) | 1.8 | |
| The rest (history entry, scoped patches, write barrier) | about 2 | |

The journal is the reason the cost grows with the session. Every edit tracks
`['revision']` and `['edits']` (the provenance log, up to 200 entries), so every
journal entry carries the whole provenance log as it was after that edit. The
journal is rewritten whole on each edit until it reaches 4 MiB (about 70 edits
here), then the whole project is checkpointed and it starts again. The cost is
the same with Use New Engine off (the gate measured 29.5/39.3 ms off against
24.5/31.5 ms on, in a shorter run).

Engine side, the edit's frame waits behind the frames already rendered ahead:
the presenter renders four frames ahead (133 ms at 30 fps) and frames queued
at the older revision are shown; nothing re-renders them. That is most of the
162 ms from edit to presented.

### Compose cost (engine)

`compose_bench` (release), the synthetic documents at the viewer's size
(1496 × 904 device pixels):

| Document | Compose CPU p50 / p95 | With the GPU p50 / p95 |
|---|---|---|
| 500 layers, playback | 5.21 / 5.75 ms | 7.38 / 9.89 ms |
| 500 layers, a drag (one layer moved per frame, a new revision each) | 5.14 / 5.80 ms | 7.15 / 10.09 ms |
| 1,000 layers, playback | 12.57 / 13.11 ms | 16.47 / 17.98 ms |

A new revision costs the same as a repeated one: nothing is cached across
frames. Sampled under `Composer::compose` (500 layers, drag, 2,853 samples):

| Where | Share |
|---|---|
| Planning (`plan::comp_pass`, evaluation through `Scope`) | 85% |
| — world matrices (`scaled_world` → `world_matrix`) | 43% (group members re-evaluate their groups' transforms for every member) |
| — `Evaluator::error_key` (an allocated, SipHash-hashed key per channel read, to remove an expression error that is almost never there) | 12% |
| — `compositing_layers` (a scan of every layer for each group's members) | 9% |
| — the matte-source check in `comp_pass` (every layer against every layer's `matteSource`) | 3% |
| — keyframe track preparation per read (`PreparedTrack::prepare`, `adopt`) | 5% |
| Encoding and GPU submission (`exec`) | 14% |

By leaf: `memcmp` 28% and `node::get` 8% (object member lookups by string key
in `BTreeMap`s), `malloc`/`free` 13%, SipHash 4%.

## Changes

### Edits during playback re-render the frames rendered ahead

The presenter renders four frames ahead; frames queued at an older revision
used to be shown, so an edit appeared up to five frames late.

- **Engine** (`pm-node` `present/schedule.rs`, `render.rs`). Every document
  update while playing calls `Schedule::invalidate`. The schedule then renders
  the stale frames again, from the first one whose replacement can still reach
  the viewer before the frame starts showing (render cost plus delivery, the
  same estimate the deadline-aware skips use), before any new frame, as long as
  the next new frame stays on time after it. A frame that cannot be replaced in
  time keeps its stale copy: the edit costs no frame. Frames before the run's
  first frame are never rendered again. `Stats.rerendered` counts them.
- **Viewer** (`engine-present/jitter-buffer.ts`, `presenter.ts`; the bridge
  lane's files, two small edits). A timed frame replaces, in place, a queued
  frame with the same pts and duration at an older revision; the replaced one is
  released as `replaced`, not counted as a drop. An older revision arriving late
  never replaces a newer one, and equal revisions (a loop's next lap) queue as
  before.

A replacement must arrive before the frame starts showing: the viewer takes a
frame out of its queue for the refresh that shows its start, and the first
version of the rule (replace while any part of the interval was left) produced
504 copies that arrived after their frame was taken and were dropped late.

### The page's edit path (Use New Engine on and off)

| Change | Where | Per edit, 1,000 layers (profiled) |
|---|---|---|
| The recovery journal drops patches a later edit overwrites at a project member (`['edits']`, `['revision']`): one copy of the provenance log instead of one per entry | `legacy/core/projects.ts` `supersede` | 35.5 → 7.4 ms |
| The journal listener keeps what it wrote (no read-back clone) and each entry's size (no whole-journal `JSON.stringify` for the 4 MiB bound); the write goes through the store's asynchronous path (yielding serialization, superseded by a newer write or a checkpoint's delete) | `projects.ts` | 7.4 → under 0.5 ms |
| The composition tabs rebuild only when their ids, names, or open state change | `extensions/timeline/comp-tabs.ts` | 13.6 → 0 ms |
| Project changes lay the viewer out once at the end of the task, not three times (each forced the editor's layout) | `extensions/viewer/viewer.ts` | 9.4 → about 3 ms |
| 3D viewport reports reuse the stage size the last layout measured | `viewer.ts` | 3.7 → 0 ms |
| The legacy mixer reads a composition's solo state once per pass (it scanned every layer for each layer) | `legacy/core/audio.ts` | 7.5 → under 0.1 ms (Use New Engine off) |
| An edit's batch leaves for the engine before the listeners of its announcement run (`EngineDoc.flushBeforeListeners`, while a session is open) | `engine-doc/install.ts`, `engine-connection/install.ts` | the engine starts 10 to 20 ms sooner |

The journal's content on disk is unchanged in meaning (it replays to the same
document; `projects.behavior.test.ts` checks the replay, the byte-exact bound,
the checkpoint, and the asynchronous write). The asynchronous write widens the
window between an edit and its journal entry on disk by the serialization's few
milliseconds; main already received the journal asynchronously
(`ipcRenderer.send`).

### pm-wasm queries for the UI

The barrier keeps `shapeRevision`: the revision of the last batch with an
operation that can change a composition's shape (`changesShape`: the layer set
or order, a layer's `id`, `type`, `parent`, `group`, `threeD`, or `lock`, the
open composition, the compositions themselves). `groupAncestors`, `wouldCycle`,
`is3DLayer`, and `transformRoots` answer again from a memo while the shape and
the current composition are unchanged, for layers the mirror holds at the index
they were answered at (one look at that index and the shadow's id there). Only
native answers for layers of the current composition are kept (a detached layer
can change without the barrier seeing it); dual mode always asks, so the
auditor still compares every answer. Callers get their own copy of an array
answer. In a 500-layer drag (20.5 s sampled) the structural queries were
6.6 s (timeline rows, the inspector's parent menu, the move tool) and are
1.4 s after, most of it the barrier's flush of the drag itself; all pm-wasm
queries together went from 7.0 to 3.5 s, and the page's main thread from 27%
to 43% idle.

The page's other per-frame cost found on the way: with no 3D layer, the 3D
viewport controller drew the same empty overlay at every invalidation and built
the composition's 3D runtime for it (`core/scene3d/controller.ts`, 9% of the
main thread during playback at 1,000 layers, flag on or off). It now draws the
empty viewport once per stage rectangle and viewport preferences.

### Compose cost (pm-eval, pm-render)

| Change | Where |
|---|---|
| `Evaluator::ev` builds and hashes an expression-error key only when an error is recorded (12% of a frame) | `pm-eval` `evaluator.rs` |
| `L.from` is read only for keyframed channels | `evaluator.rs` |
| `Scope::memoize_transforms`: each layer's local matrix per time and its group ancestors, for the scope's life (opt-in; the legacy evaluation window memoizes the same way). Group members no longer re-evaluate their groups' transforms | `pm-eval` `scope.rs`, `transform.rs`; enabled by `pm-render` `plan.rs` |
| `compositing_layers` reads each layer's group once (`LayerOwners`); one frame's passes over a composition share the owners, matte sources (a set instead of a scan per layer), and solo flag | `groups.rs`, `plan.rs` |
| The passes' quad uniform buffers are reused across frames (`UniformPool`) | `pm-render` `exec.rs`, `renderer.rs` |

Every number keeps the TypeScript expression order; the plan is the same data.
`pm-eval` tests (parity vectors, invalidation, shadow), `pm-render` tests
(headless determinism, regions, corpus: 87 frames compared, 0 failed), and
`pm-cli parity` (below) pass.

## Measurements (after)

Measured 2026-10-16 between 23:00 and 23:30 on the same Mac, Electron 44,
release addons and pm-wasm built in the worktree, one Electron at a time under
the machine lock, hidden and unfocused. Other lanes were building and running
their own Electron work (one at a time, under the same lock): the 1-minute load
average was 6.8 to 12.1 during the runs (each summary records it). Two harness
rounds of the final branch (`perf6`, before the legacy-mixer and 3D-viewport
changes, and `perf7`, after them; summaries in `tracer-measurements/*-perf6.json`,
`*-perf7.json`) and one page-timing run (`e2e/native-engine-tracer-measure.spec.ts`
on the `perf6` build; `perf-measurements/tracer-measure-page-perf6.jsonl`). Nothing here is passed on a looser bar than the plan's.

### Inspector edits during playback, 1,000 layers (p95 ≤ 16 ms, no dropped frames)

| | Before (phase2-gate.md) | After |
|---|---|---|
| Apply and send, harness (an Opacity edit every 250 ms for 60 s) | p95 157.7 ms | `perf6`: p50 10.9, p95 13.8, p99 15.3, max 16.9 ms. `perf7`: p50 11.2, p95 13.3, p99 14.5, max 15.9 ms |
| Apply and send, page timing (60 edits) | p50 24.5, p95 31.5 ms | p50 9.1, p95 12.9, p99 14.7, max 14.8 ms |
| Use New Engine off, harness | | `perf6`: p50 20.1, p95 22.0 ms. `perf7` (with the mixer change): p50 10.3, p95 14.6 ms |
| Use New Engine off, page timing | p50 29.5, p95 39.3 ms | p50 8.5, p95 10.0 ms (`perf6` build) |
| Edit to presented (viz) | p50 156.5, p95 223.7 ms | `perf6`: p50 70.9, p95 89.8 ms. `perf7`: p50 70.5, p95 82.7 ms |
| Edit to draw (page timing) | p50 136.3, p95 155.3 ms | p50 43.8, p95 59.6 ms |
| Dropped frames with edits | 381 of 1,796 | `perf6`: 0 of 1,794. `perf7`: 1 of 1,793. Page run: 0 (156 frames replaced by newer revisions) |
| Frames rendered again for edits | | 601 (`perf6`) and 592 (`perf7`) replaced in the viewer, 0 and 2 dropped late |
| Playback without edits | 0 of 1,793 | 0 of 1,793 (`perf7`) |

Result: apply-and-send p95 is under 16 ms in every run (13.3 to 13.8 ms by
the harness, 12.9 ms by the page). No dropped frames held in two of three runs;
`perf7` lost one frame of 1,793 (load 12.1 at its start), so the row's second
half is not met in every run as measured. With Use New Engine off the same edit
costs about what it costs with the engine on.

Edit to presented is now bounded by the schedule: a replacement is rendered
for the first frame that can still be replaced, render cost (about 11 ms at
1,000 layers here) plus delivery (25 ms) after the batch reaches the presenter,
then shown at that frame's start: 70 ms at the median against the 156 ms before.

### Layer drag, 500 layers (p95 ≤ 33 ms; at most one frame worse than today)

| Run | Engine p50 / p95 / p99 | Today p50 / p95 / p99 |
|---|---|---|
| Before, idle (phase2-gate.md) | 38.0 / 46.2 / 51.0 | 26.6 / 34.9 / 37.0 |
| `perf6`, idle | 30.4 / 34.9 / 39.1 | 21.8 / 30.0 / 33.4 |
| `perf7`, idle | 29.8 / 38.0 / 38.4 | 23.5 / 40.6 / 64.7 |
| Before, under load | 41.8 / 125.0 / 175.0 | 24.4 / 65.7 / 74.5 |
| `perf6`, under load | 34.4 / 125.8 / 143.3 | 22.6 / 63.9 / 80.4 |
| `perf7`, under load | 41.4 / 133.2 / 174.5 (178 of 2,401 unpresented) | 23.3 / 64.4 / 81.4 |

Hops, engine idle (`perf7`, p50 / p95 / p99 ms): input to handled 1.0 / 1.7 /
5.4; handled to the frame's arrival 14.5 / 17.0 / 20.9 (before: 21.5 at p50);
arrival to submit 0.5; submit to received 2.3 / 5.6; received to presented
11.3 / 16.7 (presentation, bridge lane). Under load, handled to arrival is
18.9 / 75.1 / 120.1 ms: the engine path waits on the load in main's process
(S1's long tasks, resizes, agent traffic), which today's WebGL path does not.

Result: fails as measured. Idle p95 is 34.9 and 38.0 ms against 33; against
today at p50 the engine is 8.6 and 6.3 ms worse (one refresh is 8.3 ms at
120 Hz), at p99 5.8 ms worse and 26.3 ms better. Under load it fails both
forms, as before.

### Compose cost

`compose_bench` at the viewer's size (release; load about 10 with other lanes'
Electron runs, against about 7 before, so the gains are if anything understated):

| Document | Compose CPU p50 / p95 | With the GPU p50 / p95 |
|---|---|---|
| 500 layers, drag | 5.14 / 5.80 → 3.63 / 3.99 ms | 7.15 / 10.09 → 5.61 / 7.86 ms |
| 500 layers, playback | 5.21 / 5.75 → 3.65 / 4.00 ms | 7.38 / 9.89 → 5.66 / 7.41 ms |
| 1,000 layers, playback | 12.57 / 13.11 → 7.00 / 7.83 ms | 16.47 / 17.98 → 10.71 / 12.98 ms |

### Correctness

- `pm-render` tests, all (corpus: 87 frames compared, 0 failed; headless
  determinism at 16 and 32 bit); `pm-cli parity render`: 96 frames compared, 0
  failed, 125 projects routed, 3 skipped (legacy playback proxies).
- `pm-eval` tests, all (TypeScript parity vectors, invalidation, the shadow
  patch stream); `pm-node` tests, all (schedule: 10 tests, 3 new).
- vitest over `src/renderer`, `src/extensions`, `src/shared`: 3,360 passed.
- e2e with Use New Engine on: `native-engine-tracer.spec.ts` and
  `native-engine.spec.ts` 12 of 12; `native-engine-tracer-measure.spec.ts` 9 of 9.
- The e2e ledger with Use New Engine off: see the report of this lane.

## Gaps

- **Layer drag p95 ≤ 33 ms idle** is missed by 2 to 5 ms as measured. Of the
  14.5 ms from handling a move to the frame's arrival, compose is about 5.5 ms
  at 500 layers (CPU 3.6, GPU 2); the rest is the batch's trip to the engine,
  waiting for a frame already rendering (a move every 8.3 ms), the presenter's
  own mirror and routing updates, and the page's main thread taking the
  descriptor. Presentation adds 11.3 ms (bridge lane). Next steps on this
  lane's side: evaluation caches across frames keyed by node revisions for
  layers whose inputs did not change (a drag changes one layer; today every
  layer is evaluated every frame), and the per-channel `BTreeMap` lookups in
  `Evaluator::ev` (24% of compose is `memcmp` on member keys).
- **Layer drag under load** fails both forms: handled to arrival reaches 75 ms
  at p95 and 120 ms at p99 under S1's main-process load. Owner: engine and
  platform (frame bridge, ports) with this lane.
- **No dropped frames during edits** held in two of three runs; one run lost
  one frame of 1,793 at a load of 12.
- **Edit to presented** is about 70 ms at 1,000 layers: the first frame that
  can still be replaced starts a render cost plus 25 ms of delivery after the
  batch arrives. A cheaper compose shortens it directly.
- **Remaining per-edit page costs** (profiled, 1,000 layers): `UIState.prune`
  walks every layer twice per edit (1.8 ms), the one remaining forced layout of
  the viewer (about 3 ms when other listeners wrote to the DOM first), and the
  inspector's derived lists (the parent menu asks `wouldCycle` for every layer;
  answered from the structural memo now).
- **Render cache.** No layer or precomp outputs are cached across frames yet
  (section 8's content keys); every frame plans and draws every layer.

## Changes outside this lane

Kept small; listed for the owning lanes.

- Viewer (bridge lane): `extensions/viewer/engine-present/jitter-buffer.ts`
  (a timed frame replaces a queued older revision of itself) and `presenter.ts`
  (the reason `replaced`); `extensions/viewer/viewer.ts` (one layout per task
  after project changes; the 3D viewport listener passes the measured stage
  size).
- Timeline: `extensions/timeline/comp-tabs.ts` (skip identical rebuilds).
- 3D lane: `core/scene3d/controller.ts` (an empty viewport is drawn once).
- Legacy core: `legacy/core/projects.ts` (the journal), `legacy/core/audio.ts`
  (the solo scan).
- `pm-render`: `plan.rs` (memoized transforms, pass facts), `exec.rs` and
  `renderer.rs` (`UniformPool`).
- `pm-node`: `present/schedule.rs` and `present/render.rs` (render-ahead
  invalidation, `Stats.rerendered`), `examples/compose_bench.rs` (edit modes).
- Measurement: `scripts/frame-bridge-measure.mjs` (a legacy edit-playback run;
  edit summaries without engine frames), `measure-app-main.ts` (CPU profiles of
  drags), `scripts/cpuprofile-summary.mjs` (new),
  `e2e/native-engine-tracer-measure.spec.ts` (replaced frames are not drops;
  `engineRerendered`).

## Reproducing

```sh
source engine/env.sh
cd apps/desktop && node scripts/build-engine-addon.mjs \
  && node ../../engine/crates/pm-wasm/scripts/build.mjs && bun run build
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock
$L node scripts/frame-bridge-measure.mjs --tag perf --only \
  tracer-edit-playback-engine-1000,tracer-edit-playback-today-1000,tracer-drag-engine-500-idle,tracer-drag-today-500-idle,tracer-drag-engine-500-load,tracer-drag-today-500-load
# A page profile of a run (V8 .cpuprofile), then its summary:
PM_MEASURE_CPU_PROFILE=/tmp/drag.cpuprofile $L node scripts/frame-bridge-measure.mjs --skip-build --only tracer-drag-engine-500-idle
node scripts/cpuprofile-summary.mjs /tmp/drag.cpuprofile --top 40
PM_TRACER_MEASURE=1 $L bunx playwright test e2e/native-engine-tracer-measure.spec.ts --workers=1

# Compose cost of a document (a synthetic project's PM.EngineDoc.snapshot().json):
cd ../../engine && cargo build --release -p pm-node --example compose_bench
PM_BENCH_EDIT=drag target/release/examples/compose_bench doc.json <composition> 240
```
