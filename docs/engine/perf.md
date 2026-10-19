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
- The e2e ledger with Use New Engine off (`playwright.ledger.config.ts`,
  `--workers=1 --retries=0`, 23:12 to 23:32, load 9 to 14): 562 run, 529
  passed, 3 failed, 30 skipped. The three failures are the gate's
  (`phase2-gate.md`): `chatgpt-connection.spec.ts:5` (environment: the
  developer's Claude settings) and `import-image-sequence.spec.ts:7` at 12 and
  120 fps (R1, the intermittent export regression of the storage lane).
- The same suite with the native evaluator in `audit` mode
  (`VITE_POWERMOVE_NATIVE_EVALUATOR=audit`, `POWERMOVE_NATIVE_REPORT_FILE`;
  23:33 to 23:53): 530 passed, 2 failed (`chatgpt-connection.spec.ts:5`,
  `import-image-sequence.spec.ts:7` at 120 fps), 30 skipped. Dual answers:
  742,860 compared, 0 mismatched, 0 stale (the structural memo is off in dual
  mode by design). Auditor: 5,302 audits, 0 mirror mismatches, 6 missed writes,
  none from this lane's changes: `requiresEngine` (4, and the one consistency
  mismatch, before a save) is written by `shared/engine-stamp.ts` without an
  announcement (the store lane's engine stamp), `assets.*.format` is
  `import-svg.spec.ts` deleting a member directly (as `write-path.md` lists),
  and `comps.*.work` one write of a composition's work area. Tracking-Proxy
  violations by app writers: 0.

## Amendment 22: nothing slower than today (2026-10-18)

Amendment 22 makes "no slower than today's viewer" a blocker for every row this
lane owns: layer drag (D6), the pan under load (S1), A/V sync under load,
cadence at 120 Hz, and the ProRes 4444 alpha line
(`phase2-gate.md`, "Rerun after the gate fixes"; `/tmp/ne-w5/issues.md`). Every
number below is from `scripts/frame-bridge-measure.mjs` on the same Mac (M4 Max,
120 Hz XDR panel, Electron 44), release addons built in the worktree
`feat/ne-perf2` (integration tip `fa05a261` plus `feat/ne-bridge` at
`e6507be4`), one Electron at a time under the machine lock, hidden and unfocused,
display awake and session unlocked before and after each run. Engine and today
runs alternate, so each pair shares the machine's load (other lanes building and
testing: 1-minute load 6.7 to 16 for the rows used, recorded in every summary).
Summaries: `tracer-measurements/*-perf2*.json`, `*-ab*.json`,
`bridge-measurements/app-latency-*-perf2*.json`.

### D6: where the drag's time went

The rerun's 49.8 ms (against `perf7`'s 29.8 ms) was not the machine. Three builds
measured back to back on 2026-10-18 (same harness, `tracer-drag-engine-500-idle`):

| Build | p50 / p95 / p99 ms | handled → render start p50 |
|---|---|---|
| `2f27b4f2` (the perf lane's tip, `perf7`'s build) | 43.1 / – / 52.5 and 39.7 / – / 64.2 (`ab2x1`, `ab2x2`) | not traced (16.5 and 18.4 handled → arrival) |
| `ce1541b8` (the perf lane's merge) | 49.4 / – / 72.8 and 55.9 / – / 72.9 | 20.7, 21.9 |
| integration tip + bridge | 53.4 / 70.5 / 71.5, 55.6 / 72.9 / 80.5, 47.9 / 64.1 / 64.6 | 18.2, 22.6, 19.7 |

`ce1541b8` is already slow: the regression came in with the merges the perf
lane's branch did not have, not after it. The engine's per-frame trace now carries
each revision's way to the render thread (`FrameTrace.revisions`, `work_start`,
`cpu_us`; `POWERMOVE_BRIDGE_TRACE=1`), and the harness splits handled → render
start in four (`perf2t1`, load 11.6):

| Hop (p50 / p99 ms) | |
|---|---|
| handled → the presenter has the batch (page flush, port, engine thread) | 0.08 / 0.21 |
| → the render thread applied it (waiting for the frame being drawn) | 9.74 / 20.79 |
| → the pass that renders it began | 0 / 0.01 |
| → render start (the pass's document step) | **9.58** / 12.21 |
| render start → GPU done | 8.24 / 10.83 (compose CPU 4.82) |

The document step decides the view's route at every new revision
(`ViewDocument::route` → `pm_render::capability::unsupported_with_fonts`), and the
check walked group membership by scanning the composition's layers for each step
of each layer, allocating a string id per comparison (`enclosing_groups`, added by
the motion lane in `77cec836` with 2.5D and motion-blur routing for group
members). `compose_bench` on the 500-layer document: **7.8 ms per decision**,
98% of it in that scan (`sample`). Because the render thread spent it before
every frame, it also doubled the time a new edit waited for the frame in progress.

After the routing fix the rest of the drag was the composite itself and how
the edits reached it. Two findings:

- **Every move composited every layer.** On the 500-layer project the plan was
  4.8 ms of CPU per frame (`compose_bench`), nearly all of it evaluating the 499
  layers the move did not touch, and the GPU drew all of them again.
- **Every move was two batches and two renders.** The engine's raw trace
  (`--keep`, `perf2raw`): 4,794 frames for 2,401 moves, each move's revisions
  posted about 0.5 ms apart. A drag's move sets `position.x`, then
  `position.y`, and the write barrier flushes before each pm-wasm query in
  between, so the page sent two batches: the engine drew the first (the layer
  moved only horizontally) and the move's own batch waited for that render
  (posted → applied p50 2.09 ms, p99 2.65, with the render thread asleep 78%
  of the time by `sample`).

### Changes

| Change | Where | Effect |
|---|---|---|
| The capability check indexes a composition's group layers once (the first group layer with each id, as the scan found it) | `pm-render` `capability.rs` (`GroupIndex`); a test compares the walk with the scan on nested, duplicate, cyclic, missing, and numeric ids | routing 7.8 → 0.19 ms per revision (`compose_bench`); pass → render start 9.6 → 0.55 ms in the app |
| Frames shown at the next refresh (paused edits, gestures) leave the render thread when submitted: a completion thread waits for the GPU, sends the descriptor the moment the work is done, and hands stats and trace back. At most two frames wait on the GPU; timed frames, frame requests, viewer states, GPU recovery, and shutdown drain it first | `pm-node` `present/complete.rs` (new), `present/render.rs` (`render` split into submit and `record_frame`), `present/mod.rs` | the next edit no longer waits for the previous frame's GPU work: applied-wait p50 4.7–5.5 → 3.2–3.5 ms |
| **Draw reuse**: layers a paused frame's edit did not touch give back their draws from the last frame (below) | `pm-render` `reuse.rs` (new), `plan.rs` (`Build::reuse`, the hook in `comp_pass`), `renderer.rs` (`set_draw_reuse`, `clear_draw_reuse`, `draw_reuse_stats`); `pm-eval` `scope.rs` (`Scope::layer_index`); `pm-node` `present/compose.rs` turns it on around the viewer's frames | compose CPU p50 4.8 → 2.1 ms on a 500-layer drag (`compose_bench`) |
| **Frame prefix**: the viewer's frame draws from a kept copy of the bottom of the last frame when it starts with the same draws (below) | `pm-render` `frame_prefix.rs` (new), `renderer.rs`, `exec.rs` (`render_draws` crate-visible), `media.rs` (`stamp` crate-visible) | a drag's compose with the GPU 5.3–5.5 → 1.2–3.3 ms, by the dragged layer's place in the stack (`compose_bench`) |
| **One batch per run**: the page's engine connection holds a run's patch batches until the run ends and sends each session's contiguous batches of one origin as one; anything else it sends first sends the held ones, and snapshots send them before they wait | `engine-connection/patch-coalesce.ts` (new), `connection.ts` (`patches`, `sendPatches`), `session.ts` (`forward`), `install.ts` (the render transport's revision reads) | one render per move: 2,395 frames for 2,401 moves; posted → applied p50 2.09 → 0.02 ms |
| Per-revision trace (revisions, pass start, compose CPU) and the harness hops that read it | `present/render.rs`, `scripts/frame-bridge-measure.mjs` | measurement only |
| `PM_BENCH_ROUTES`, `PM_BENCH_LAYER` (one layer dragged), `PM_BENCH_REUSE=0` in `compose_bench` | `examples/compose_bench.rs` | measurement only |
| Today's viewer at each cadence rate (`tracer-playback-today-*`), A/V offsets by load event with histograms (`offsetNearMs`, `offsetHistMs`) | `scripts/frame-bridge-measure.mjs` | measurement only |
| While the window is being resized (and 150 ms after its last step), the worker predicts the display 0.75 refreshes later (1.5 + 0.75 refreshes after its animation frame, plus 3.2 ms); idle prediction unchanged at 1.5 (1.75 was tried and put 208 of 1,433 frames outside ±10 ms at 23.976 fps idle) | `extensions/viewer/engine-present/presenter.ts` (`resizeLatencyRefreshes`, `noteResize`), `present-worker.ts` (`WORKER_RESIZE_LATENCY_REFRESHES`, the `resize` message), `worker-presenter.ts`, `presentation.ts` (a window `resize` notes the presenter); bridge lane's files, small edits | A/V under load (below) |

### Draw reuse

A gesture makes a revision per pointer move. `pm_render::reuse` keeps, per
composition pass, the draws each layer produced at the last frame and gives
them back when nothing they were computed from changed; the plan is then the
plan a fresh build makes, and so is every pixel.

- **What is kept**: solid layers and raster image layers (not SVG, whose plan
  reads the file) on today's fast path: no effects, masks, transitions, track
  matte, blend mode, motion blur, or 2.5D plane; no expression anywhere in the
  layer or in the layers it depends on; nothing reported for the layer (no
  failure, no unsupported feature); outside a track matte's pass, outside
  motion-blur samples, in a pass without solo layers. Everything else is
  planned as before.
- **What a kept draw depends on**: the layer's subtree; its *closure*, the
  parent chain and enclosing groups, transitively (what `world_matrix`,
  `transform_parent_matrix`, and `active` read); the composition's own
  fields, the open composition's, and the root's (`Comp::member` falls back
  to the root: assets, project settings); the time, the pass's size, scale,
  and precomp depth; the build's options (sampling, motion blur, video
  padding, view override); for an image, the path the asset resolver answers.
- **Validity** uses the document's node revisions (`pm_model::doc`: a node's
  revision moves whenever its subtree changes). Entries live one revision (the
  last frame's, the *stamp*). A kept draw is given back only when the closure,
  found again in this revision's document, is the same nodes (address and id)
  with every id unique in the composition and every revision at or below the
  stamp; when the composition's, open composition's, and root's fields other
  than `layers` and `comps` are unchanged since the stamp; and when the pass
  and options match and the media path is the same. A unique id found again
  unchanged is the same logical layer (the argument `pm-eval`'s interned
  properties make): moves keep revisions, anything created gets a new one.
- **Scope**: off unless the caller turns it on. The composer turns it on only
  around the viewer's own frames of a view's long-lived document, which
  changes in place (a replaced document is built while the old one is alive,
  so its root's first member moves; that and the project id drop every entry).
  A build at another time than the last one looks nothing up and keeps
  nothing (playback pays one comparison a frame); the first build at a new time
  stores, the next ones reuse.
- **Tests** (`pm-render/tests/draw_reuse.rs`): a renderer with the reuse and
  one without plan a document through seeded sequences of every kind of edit a
  kept draw could depend on (moves, opacity, parent and group changes,
  composition and asset fields, relinked media, expressions added and removed,
  duplicated ids, order, insert and delete, on/off, solo, track mattes, level
  2, 3D, resyncs, replaced documents, time): 62,100 frames, 511,774 draws
  reused, every plan identical (`PM_REUSE_FUZZ=30`). The parity corpus (206
  projects, 3,398 frames, each first sample also drawn and read back) plans and
  draws identically with and without it.

### Frame prefix

The top-level pass draws bottom to top into one target. `pm_render::frame_prefix`
keeps a copy of that target after the frame's first draws when they are
unchanged from the last frame (a drag leaves every layer below the dragged one
as it was) and has the next frame that starts with the same draws copy it back
and draw only the rest. Splitting a pass and copying an `rgba16f`/`rgba32f`
target are exact.

- The copy is kept only below draws whose pixels are a function of the draw
  (solids, stills, level 2 vector scenes, and groups and nested compositions of
  those; no compositing nodes, video, text, shaders, 3D, Blender, or motion-blur
  samples) that resolved without a failure, and is invalid once a still it read
  has another size or modification time (the media cache reads it again then).
  It is made only above 8 unchanged draws and moved up only for 32 more.
- Never with the depth and id surface, exports, or compositing nodes at the
  top level; the key holds the composition, the target's size and format, the
  region of interest, precision, working encoding, blend domain, background,
  and the plan's size. A frame that fails drops it.
- **Tests**: 3,000 edited frames (drags with every other kind of edit between
  them), whole frames and the viewer's region of interest, read back bit for
  bit against a renderer without it (`PM_REUSE_FRAMES=1500`); a still changed
  on disk under a kept copy (the test fails with the stamp check removed); the
  parity corpus as above.

`compose_bench` on the 500-layer project, a drag of one solid layer every frame
(p50 ms, load 10 to 15; `PM_BENCH_LAYER=<n>` picks the n-th solid of the
composition's list; the last column is the top-level draws the frame did not
draw again):

| Dragged | Compose CPU, without / with | With the GPU, without / with | Kept below |
|---|---|---|---|
| a different solid every frame | 4.80 / 2.08 | 6.80 / 4.07 | |
| solid 0 | 3.31 / 1.12 | 5.26 / 1.75 | 54 |
| solid 150 | 3.46 / 0.92 | 5.45 / 1.23 | 65 |
| solid 300 | 3.38 / 1.47 | 5.36 / 3.00 | 30 |
| solid 420 | 3.49 / 1.49 | 5.46 / 3.26 | 0 |

### Measurements (after)

#### Layer drag, 500 layers (D6)

Engine and today alternate run by run (each pair shares the machine's load; the
1-minute load is recorded with every summary). Today's rows count presented moves
only; "with drops" counts each move today never presented at the latency of the
next frame it did present. The engine presented every move in every row of the
final build.

| Build and run | Load | Engine p50 / p95 / p99 ms | Today p50 / p95 / p99 ms |
|---|---|---|---|
| Gate rerun (`phase2-gate.md`) | 5.2 | 49.8 / 58.4 / 58.6 | 24.8 / 25.4 / 33.2 |
| Routing fix (`perf2r1`) | 6.9–9.7 | 31.9 / – / 41.2 and 32.9 / 37.8 / 45.2 | 20.9 / 29.2 / 36.2 |
| Routing fix and completion thread (`perf2r2`) | 11.2–12.2 | 31.3 / – / 39.8 and 34.0 / 43.2 / 44.1 | 21.1 / 30.2 / 37.5 |
| Draw reuse and frame prefix (`perf2d1`) | 9–15 | 25.7 / – / 33.8 | 21.2 / – / 29.8 |
| Final build: and one batch per move (`perf2e1`) | 8–10 | 24.6 / 26.0 / 32.8 | 27.4 / 28.3 / 35.3 |
| Final build (`perf2g1`) | 11–15 | 26.0 / 26.6 / 34.3 | 24.7 / 29.3 / 37.1 |
| Final build (`perf2g2`) | 9–10 | 26.0 / 26.6 / 27.2 | 23.9 / 28.5 / 32.3 |
| Final build (`perf2g3`) | 8.5–9.6 | 24.2 / 28.5 / 28.9 | 26.6 / 35.0 / 35.7 |
| Final build, gate load (`perf2g1`) | 9.6–13.6 | 22.1 / 105.5 / 130.4 | 25.2 / 67.7 / 80.2 (289 never presented; with them 26.2 / 76.4 / 117.8) |
| Final build, gate load (`perf2g2`) | 12.5–13.8 | 22.5 / 110.6 / 138.8 | 22.0 / 55.4 / 72.1 (205 never presented; with them 22.1 / 80.1 / 130.1) |
| Earlier, gate load (`perf2c2`) | 9.9–10.8 | 33.8 / 116.9 / 133.8 | 28.2 / 69.3 / 86.5 (86 never presented; with them p99 103.2) |

`perf2f1` (the same build, 1-minute load 16 to 40 while other lanes built)
is in `tracer-measurements/` and not in the table: today lost 198 to 362 moves
per run there and the engine 277 to 355 under the gate load on top; it measures
the machine, not the paths.

Hops of the final build idle (`perf2e1`, p50 / p99 ms): input → handled 0.9 /
3.1; handled → the presenter has the batch 0.08 / 0.17 (the run's batch leaves
at its end); → applied on the render thread 0.02 / 0.03; → render start 0.55 /
0.63; render start → GPU done 2.86 / 4.94 (compose CPU 1.63); sent → arrival on
the page 0.13 / 3.81; arrival → the worker's submit 0.6; submit → received by viz
3.46; received → presented 15.83 / 18.59. One frame per move (2,395 frames for
2,401 moves; `perf2raw` before the change: 4,794). Today's path: input → handled
in the animation frame 1.8 to 7.8 at p50 by run (the harness's 120 Hz input
lands at a different point of the refresh in each run), that frame → presented
20.0.

Result, idle: **met at p95 and p99** in all four pairs of the final build
(engine p99 27.2 to 34.3 ms against today's 32.3 to 37.1); **p50 met in two of
four** (24.6 and 24.2 against 27.4 and 26.6) and **1.3 and 2.1 ms slower in the
other two** (26.0 against 24.7 and 23.9). The engine's path now takes about 4
ms from the handled move to its frame on the page; where a move lands relative
to the refresh decides whether that frame makes the same display refresh as
today's frame drawn in the animation frame that handled the move, so the p50
difference follows the run's input phase (today's own p50 moves 23.9 to 27.4
between runs for the same reason). Under the gate load: **p50 met** (22.1 and
22.5 against 25.2 and 22.0), **p95 and p99 not met** (105.5 / 130.4 and 110.6 /
138.8 against 67.7 / 80.2 and 55.4 / 72.1 for the moves today presented; against
every move, today's p99 is 117.8 and 130.1, and it never presented 205 to 289 of
2,401 moves where the engine presented all). The engine's tail under load is the
page's main thread: sent → arrival p99 50 to 62 ms while the window resizes
(input → handled has the same 43 to 44 ms tail on both paths). See Gaps.

#### Pan, the S1 real-viewer gate (perf project; the engine draws its test source)

| Run | Load | Engine p50 / p95 / p99 ms | Today p50 / p95 / p99 ms |
|---|---|---|---|
| Idle (`perf2c2`, `panab`) | 6.8–17.9 | 26.6 / 27.4 / 35.0; 20.8 / – / 29.2; 18.0 / – / 30.9 | 20.9 / 29.0 / 29.2 |
| Idle, `ce1541b8` (`panab`) | 7.6–8.0 | 20.9 / – / 29.4; 19.9 / – / 30.9 | |
| Idle, final build (`perf2g1`) | 11.6–16.1 | 26.3 / 27.2 / 34.9 | 28.1 / 28.5 / 29.2 |
| Under the gate load (`perf2c2`) | 7.3–10.0 | 26.0 / 84.6 / 109.4 | 27.2 / 44.8 / 64.8 (157 never presented; with them p99 77.1) |
| Under the gate load, final build (`perf2g1`) | 16.1–32.7 | 24.2 / 91.0 / 124.3 | 24.2 / 48.7 / 66.0 (134 never presented; with them 61.4 / 78.4) |

The pan does not render the document (the engine draws its test source), so this
round's render changes do not reach it. Idle its p50 is today's or better and its
p99 within 6 ms of today's (input phase moves p50 by up to 6 ms between runs of
either build). Under load it is **not met** at p95 and p99: sent → arrival on the
page p99 51.7 ms and the worker's frame reaching viz p99 36 ms, during live
resizes and long tasks. The descriptor has to pass through the page's main
thread because only the preload there can make a `VideoFrame` of the shared
texture (`bridge.md`, "Taking the descriptor off the page's main thread"); today's
viewer needs that thread once per move, the engine path twice.

#### The app's engine specs

With Use New Engine on, `native-engine-tracer.spec.ts`,
`native-engine-transport.spec.ts`, `native-engine.spec.ts`, and
`engine-render-transport.spec.ts` (24 tests, `--workers=1`): 21 passed on the
first run. `engine-render-transport.spec.ts:67` (an edit, then at once a
synchronous snapshot) found that a held batch never left while the snapshot
blocked the page; snapshots now send the held batches first, and it passes.
`native-engine-tracer.spec.ts:179` passed on the rerun. `:294` (the WebGL
canvas's visibility for a composition routed to the current engine) fails the
same way in the bridge lane's run of `feat/ne-bridge` (`bridge-r4-lifecycle`,
before this lane's changes): not from this round.

#### Cadence at 120 Hz (irregular intervals; the bar is today's count or fewer)

Engine and today alternated in each session, the same synthetic projects; 0
engine drops in every run.

| Rate | Session | Engine idle | Today idle | Engine under load | Today under load |
|---|---|---|---|---|---|
| 30 fps | `perf2c1` | 0 (ideal 0) | 777 | 397 | 519 (130 dropped) |
| 25 fps | `perf2c1` | 300 (pattern 299) | 751 | 489 | 578 (90 dropped) |
| 23.976 fps | `perf2c1` | 64 (ideal about 7) | 293 | 290 | 402 (83 dropped) |
| 30 fps | `perf2f1`, final build | 101 | 415 | 560 | 624 (138 dropped) |
| 25 fps | `perf2f1` | 302 | 650 | 625 | 655 (40 dropped) |
| 23.976 fps | `perf2f1` | 20 | 323 | 349 | 636 (99 dropped) |

Result: **met** at every rate, idle and under load, in both sessions (1-minute
load 8 to 16 idle and 8 to 41 under load in `perf2f1`: other lanes were
building). The final build's loaded engine runs again in `perf2g1`: 421, 529, and
302 irregular intervals at 30, 25, and 23.976 fps, 0 drops. A third session (`perf2c2`) had the engine at 76, 494, and 74 idle and
466, 567, and 306 under load, each below today's counts above.

#### A/V sync under load

|offset| between a frame's presentation and its time on the audio clock's line
(p95 within ±10 ms). Every engine run of the final build (`perf2f1`); today's
viewer has no audio-clock line to compare with:

| Rate | Idle: p95 / p99 ms, within ±10 ms | Gate load: p95 / p99 ms, within ±10 ms | Outside, by load event (gate load) |
|---|---|---|---|
| 30 fps | 7.12 / 7.14, 1,790 of 1,793 | 7.20 / 13.85, 1,743 of 1,793 | resize 32, long task + resize 12, none 6 |
| 25 fps | 8.15 / 8.27, 1,494 of 1,494 | 8.67 / 15.39, 1,437 of 1,494 | resize 45, long task + resize 6, none 6 |
| 23.976 fps | 7.28 / 7.94, 1,432 of 1,433 | 9.59 / 15.66, 1,374 of 1,433 | resize 49, long task + resize 4, long task 1, none 5 |

A second session of the loaded runs (`perf2g1`, load 17 to 38): 30 fps p95 9.97
(1,715 of 1,793 within), 25 fps 7.65 (1,457 of 1,494), 23.976 fps 8.79 (1,383 of
1,433); resizes again account for nearly every frame outside.

Result: **met** idle and under the gate load at every rate in both sessions of the
final build, the 30 fps loaded run by 0.03 ms in the second. The margin under load
is small; see Gaps.

Before the resize prediction (`perf2c1`, the same harness) the loaded p95 were
11.84, 11.71, and 11.49 ms; with it (`perf2c3`) 8.70, 9.36, and 10.43 ms. The
frames still outside are the live window resizes of the load profile (1.2 s
every 5 s): the WindowServer shows a worker canvas's frame one to three
refreshes later while the window resizes (`bridge.md`, "A/V sync by
presentation"), so the offsets there spread a refresh either way; no single
prediction removes them.

#### ProRes 4444 alpha, codec generation

Root-caused, not changed in this round, and not a difference from today. Within
8 px of α = 0 Apple's re-encode keeps the source's straight color under
transparent pixels, and a premultiplied pipeline holds zero there, so ProRes's
8 × 8 DCT carries that difference into the visible neighbours (max 1.603 CV at
10 bits; 0.000 beyond 8 px; alpha exact; PSNR Δ −0.184 dB; `spikes/S2.md` result
2, `export.md`). Today's exporter writes the same zero: it reads back the
composited, premultiplied WebGL frame and sets color to 0 wherever α = 0
(`legacy/core/exporter.ts:926`, `alphaFrame`), then hands 8-bit RGBA to
VideoToolbox as `ayuv64le` (`main/render-encoder.ts:31`–`35`). So the line is the
same mechanism in both engines (today's at 8 bits per channel before the
encoder); the file comparison with today's exporter was not measured. A
straight-alpha path for ProRes 4444 (color under α = 0 kept from straight
sources) would go beyond today and stays the open decision in `phase2-gate.md`.

## Gaps

- **Layer drag (D6) p50 idle, amendment 22** (2026-10-18): p95 and p99 are
  better than today's in all four final pairs; p50 is better in two and 1.3 and
  2.1 ms slower in the other two. The engine path takes about 4 ms from the
  handled move to its frame on the page (render 2.9 ms, compose CPU 1.6); the
  remaining difference is whether that frame makes the same display refresh as
  today's frame, which today draws in the very animation frame that handled the
  move, and it follows the harness's input phase. Further cuts on this lane's
  side: the per-pass setup the plan still repeats every frame (group passes'
  3D ordering and compositing lists, about 1 ms at 500 layers), the routing
  decision per revision (0.24 ms), and draws below a dragged layer near the
  bottom of the stack (no kept bottom there: a cached output per group and
  nested composition, section 8, would cover them). Owner: this lane with GPU
  and render.
- **Layer drag and pan under load (p95, p99)**: the descriptor waits for the
  page's main thread during live resizes and long tasks (sent → arrival p99 50
  to 62 ms), which today's viewer needs once per move and the engine path twice.
  Taking it off the main thread needs Electron to hand shared textures to a
  worker (`bridge.md`, option 2). Owner: engine and platform, bridge. Under the
  same load the engine presented every move; today never presented 8 to 12% of
  them.
- **A/V sync under load** passes with little margin (p95 9.97 ms at 30 fps in
  one of two sessions): the live resizes of the load profile spread
  presentation a refresh either way, and the worker cannot see which.
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
- **Render cache.** Layer draws (solids and stills on the fast path) are reused
  across frames of one paused time and the bottom of the viewer's frame is kept
  (above); no layer or precomp outputs are cached across frames by content key
  yet (section 8), so playback and other layer kinds plan and draw every layer.

## Changes outside this lane

Kept small; listed for the owning lanes.

Amendment 22 round (2026-10-18):

- `pm-render` (GPU and render lane): `reuse.rs` and `frame_prefix.rs` (new
  modules), hooked in with small edits: `plan.rs` (`Build::reuse`, three
  `Build` fields crate-visible, the reuse call in `comp_pass`), `renderer.rs`
  (the reuse and prefix state, `set_draw_reuse`, `clear_draw_reuse`,
  `draw_reuse_stats`, `render_into` and `plan` build through
  `build_blender`), `exec.rs` (`render_draws` crate-visible), `media.rs`
  (`stamp` crate-visible), `lib.rs`; `tests/draw_reuse.rs` (new).
- `pm-eval`: `scope.rs` (`Scope::layer_index`, a public view of the index the
  scope already keeps).
- `pm-render`: `capability.rs` (`GroupIndex`: group membership indexed once per
  composition in the capability check; the derouter lane removes the check
  later).
- Viewer (bridge lane): `extensions/viewer/engine-present/presenter.ts`
  (`resizeLatencyRefreshes`, `noteResize`, `RESIZE_HOLD_MS`),
  `worker-presenter.ts` and `present-worker.ts` (the `resize` message,
  `WORKER_RESIZE_LATENCY_REFRESHES`), `presentation.ts` (a window resize notes
  the presenter).
- Measurement: `scripts/frame-bridge-measure.mjs` (engine hops per revision,
  today's cadence runs, A/V offsets by load event).

Earlier rounds:

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

# Amendment 22 round: engine and today alternating, display awake (the harness
# refuses otherwise; --wait-display waits). The engine trace is on by default.
$L node scripts/frame-bridge-measure.mjs --skip-build --wait-display --tag perf2 --only \
  tracer-drag-engine-500-idle,tracer-drag-today-500-idle,tracer-drag-engine-500-load,tracer-drag-today-500-load
$L node scripts/frame-bridge-measure.mjs --skip-build --wait-display --tag perf2 --only \
  tracer-playback-engine-30,tracer-playback-today-30,tracer-playback-engine-25,tracer-playback-today-25,tracer-playback-engine-23976,tracer-playback-today-23976
# (and the -load variants of each; app-latency-{engine,today}-{idle,load} for the pan)
# The routing decision's cost, and a profile of it:
PM_BENCH_ROUTES=3000 target/release/examples/compose_bench doc.json <composition> 3
# A drag of one layer with and without the draw reuse and the frame prefix:
PM_BENCH_EDIT=drag PM_BENCH_LAYER=150 target/release/examples/compose_bench doc.json <composition> 300
PM_BENCH_EDIT=drag PM_BENCH_LAYER=150 PM_BENCH_REUSE=0 target/release/examples/compose_bench doc.json <composition> 300
# (examples and test builds: CARGO_PROFILE_RELEASE_LTO=off where kache's objects
# carry no bitcode, as pm-audio.md notes; the addons build with thin LTO.)
# The reuse's equivalence tests, longer:
PM_REUSE_FUZZ=30 PM_REUSE_FRAMES=1500 cargo test --release -p pm-render --test draw_reuse

# Compose cost of a document (a synthetic project's PM.EngineDoc.snapshot().json):
cd ../../engine && cargo build --release -p pm-node --example compose_bench
PM_BENCH_EDIT=drag target/release/examples/compose_bench doc.json <composition> 240
```


## P3/B2/COMPCLOSE-2 follow-up measurements

Measured on the same M4 Max, with native release addons and the release wasm
module built in this worktree. Electron runs use the shared lock, hidden windows,
isolated profiles and one Playwright worker. Raw results remain outside Git under
`/tmp/ne-fixups2-evidence/`. These are targeted follow-up results, not a full gate.

### Composition scopes

`scope-bench.test.ts` builds 100 layers with 40 keyed properties each (five keys
per property), alternates native/legacy order over five samples, and warms both
paths. Each root sample and each legacy sample times one million calls; detached
native samples time 24 calls because each serializes the large scope. Assertions
consume every answer, and the resident cache reports one miss and 134 hits.

| Scope, repeated evP at one time | Native median | Today median | Strict no-slower gate |
|---|---:|---:|---|
| Fresh root wrapper sharing proj.layers | 34.92 ns | 34.39 ns | Not cleanly met across repeated runs; the final run is slightly slower |
| Genuinely detached composition | 19.57 ms | 34.53 ns | Fails |

The root wrapper no longer serializes a document or resets the evaluator (the
reported regression was 62 ms/call). Repeated numeric answers retain pm-wasm's
last value after validating revision, scope, property, key and module health.
Other runs put the root path at 40–43 ns versus 41–47 ns; a run at 36.5 versus
33.2 ns and the final run above prevent claiming the strict performance line.
No timing tolerance was widened. The remaining root difference is nanoseconds,
not the original frame-scale serialization, but it is still disclosed.

Resident detached scopes avoid repeated Node construction and document-cache
resets, but their exact contents still must cross the boundary and be hashed.
This remains milliseconds versus nanoseconds: a cache hit alone does not clear
P3 for arbitrary detached objects. An explicit detached-scope write/version
contract or dependency-aware query transport is needed to remove that cost while
still seeing arbitrary outside-document mutations; silently caching by identity
and mirror revision would miss those mutations. A version contract can only be an
additive opt-in for new callers; existing apiVersion 1–3 calls must retain their
unannounced-mutation semantics.

### Boot and activation

`engine-doc-boot.spec.ts`: the activation regression and five alternating
setting-off/on pairs pass (11 fresh launches including the separate regression).
Every activation probe returns 10 from its keyed property; with the setting on
it is a native evP answer, zero fallbacks and zero named failures.

| Measurement | Setting off | Setting on |
|---|---:|---:|
| Time to installed UI, p50 / max | 590.38 / 603.65 ms | 583.53 / 597.45 ms |
| Time to opened editor, p50 / max | 866.34 / 878.57 ms | 883.45 / 910.55 ms |

On: streaming compile 11.1–14.4 ms (includes body delivery), module load
76.3–81.5 ms (source headers 65.2–67.1 ms), instantiation 0.1–0.2 ms,
mirror-ready 78.2–83.3 ms from engine-doc installation. The UI install chain never
awaits any of these. UI availability is faster on this five-pair sample; opening
the editor is 17.11 ms slower at p50. The latter is an additional observed
performance gap, not a timing allowance or a claim that all boot behavior matches.
Five observations per mode do not estimate tail latency.

### Mask correctness and fill work

The window keeps the original viewport math and global sample coordinates, and
returns full-sized transparent alpha outside the filled mesh; the existing blur
therefore keeps all feather margins and downsample phases. Exact rational
scanline spans replace per-pixel edge walks. The mask's white alpha is resolved
directly, without full-output multisampled RGBA storage.

- GPU path-mask reference gate: 54/54 canvases bit-exact (zero differing pixels),
  including HD/UHD and every blur pass structure.
- Composite path-mask corpus: seven non-text frames, zero failures; the two
  path-mask text/caption frames pass their existing text class, all other pixels
  exact. No unsupported reason, corpus case, threshold or reference was changed.
- Direct full-canvas/window comparison: exact at 317×233, HD and UHD with fractional,
  clipped and reflected geometry; winding spans also match independent per-sample
  point tests for both fill rules and sample patterns.
- Canvas2D gate: unchanged 529-case result (503 exact, 22 within one, four existing
  pinned exceptions). Those exceptions remain baseline parity gaps.
- CPU-model test HD fill: 6.1 ms for hd-f24 and 17.6 ms for the subtract combination;
  GPU reference test first-fill, including pipeline construction, HD 10.1 ms,
  UHD 19.5 ms. These are test-harness timings, not viewer playback results.

Reproduce with the opt-in `scope-bench.test.ts`, `engine-doc-boot.spec.ts` and
`path-mask-playback-measure.spec.ts` (`PM_FIXUPS2_MEASURE=1`,
`PM_FIXUPS2_EVIDENCE=<outside-repo-dir>`); Electron specs under pm-electron-lock.
Rust selections: pm-wasm tests; pm-vector canvas2d/mask_coverage and unit tests;
pm-render path_masks plus corpus with `PM_RENDER_ONLY=composite/path-masks`,
and text_corpus with `PM_TEXT_ONLY=composite/path-masks`.


### Actual viewer playback (strict performance gate still fails)

The corrected measurement spec finishes successfully and records four rows; that
means the measurement ran, not that the performance gate passed. Each composition
is 30 fps, 120 seconds long, with a feathered curved path whose rotation and one
vertex animate continuously. It does not loop, so each item raster in the measured
span is new. Both hidden viewers have the same full-sized device canvas (asserted
1920×1080 or 3840×2160), Full preview resolution and adaptive quality off. Each row
has two seconds of playback warmup followed by twelve measured seconds.

| Canvas | Today shown / skipped | Engine shown / skipped | Today interval p50 / p95 / p99 / max | Engine interval p50 / p95 / p99 / max |
|---|---|---|---|---|
| 1920×1080 | 360 / 0 | 357 / 3 | 33.30 / 34.20 / 34.50 / 40.50 ms | 33.20 / 41.50 / 41.50 / 126.70 ms |
| 3840×2160 | 360 / 0 | 360 / 0 | 33.30 / 33.80 / 34.40 / 34.50 ms | 33.20 / 35.10 / 35.30 / 35.70 ms |

Both rows fail the no-slower line at the tail; HD also loses three composition
frames. UHD's zero skips alone does not clear the row. There are no repeated
frames, render errors, layer failures, named evaluator failures or legacy routes
in either engine row. Engine compose mean is 1.36 ms (HD) and 2.50 ms (UHD),
including the warmup; maxima are 66.19 and 5.24 ms. HD's measured viewer changes
from viewSeq 4 to 5 and presenter generation 2 to 3 during the span, with three
generation drops; the result retains that event rather than dropping the sample.
The cause of the view change was not isolated in this lane.

Legacy timestamps come from the compositor's `preview:presented` submission event;
engine timestamps come from the presentation worker's onPresented/displayUs
reports. These are the real viewer paths, but different points in the compositor
pipeline. A common viz/presentation trace (the bridge/S1 harness) is still needed
for a physical-display timing comparison. The recorded skips and tail costs are
not waived pending that measurement. The remaining playback gate belongs with
the bridge/presentation follow-up; CPU fill windowing alone does not clear it.

All app runs shut down their isolated Electron instance. The initial harness
attempts used the wrong legacy event and then treated a Disposable as a function;
both failed before yielding valid paired data. The corrected run is the table
above, preserved in the evidence file together with the failed first zero-frame
observation. No production behavior or test threshold was changed to correct
those instrumentation errors.

## fixups2 round 2: startup and a common playback clock

The preload starts the editor's wasm fetch/compile while the renderer module
graph loads. The renderer privately consumes that promise and the launch flag;
`nativeEngine.enabledSync()` remains live for all other callers. A failed early
load can retry normally. This is a small edit to the bridge-owned preload hook;
no CSP or extension activation contract changed. Speculative font prewarming now
waits for a paint opportunity and idle time. Visible queries still obtain their
exact fonts on demand. Startup stats separate module wait, instantiation and
mirror sync; install/connection/first-descriptor marks and separate V8 profiles
make their timing inspectable.

The viewer resize debounce restarted its 150 ms deadline whenever an update
repeated the same pending size. The pending size now keeps its original deadline;
a real new size restarts it. Moving to another window cancels the old window's
timer and starts a new deadline there. Both regressions pass; Viewer is version
1.2.2 so forks receive update notices. **This did not fix the observed HD spike:**
the runtime trace establishes a different cause below. No gamut/EDR freeze was
retained, and no production display semantics changed to stabilize the test.

### Detached scope validation: large improvement, strict gate still fails

The locked five-sample scope run uses the same 100-layer, 40-keyed-properties,
five-keys/property fixture as round 1. Repeated root wrappers measure median
45.25 ns native versus 46.08 ns today. Genuinely detached repeats measure median
10.07 µs native versus 40.67 ns today, down from round 1's 19.57 ms. The native
samples are 11.564, 10.633, 10.066, 9.821 and 9.828 µs. No full scope crosses the
boundary on a stable repeat, and correctness tests assert no wasm query at all.
This removes the frame-scale serialization cost but remains roughly 247 times
the warmed legacy memo. The strict detached gate is not met. Structural checks
must visit layer identities/order/parents, and exact read validation must inspect
its inputs even when an object's identity did not change. No immutable/epoch
contract was imposed on existing callers. The later serializer correctness guards
passed unit tests but were not separately retimed after the disk stop.

The real-wasm randomized test retains three independent cached operations across
1,200 mutations and compares all 3,600 results with fresh full serialization.
The final engine-doc plus kernel loader/install selection passes 147 tests with
two opt-in measurement tests skipped; the strengthened mutation test was then
rerun separately (nine detached tests pass).

### Final five boot pairs: performance gate still fails

The final production startup code was measured in five fresh alternating off/on
pairs under the Electron lock. Times are launch-to-installed-UI and
launch-to-opened-editor wall times. Every activation probe returned the exact
native answer with zero fallbacks or named failures.

| Wall time | Today p50 / max | Engine p50 / max |
|---|---:|---:|
| Installed UI | 1526.25 / 1636.00 ms | 1501.48 / 1528.24 ms |
| Opened editor | 2065.01 / 2230.59 ms | 2129.16 / 2204.29 ms |

| Pair | Engine minus today: UI | Engine minus today: opened editor |
|---|---:|---:|
| 1 | +58.72 ms | +86.96 ms |
| 2 | -170.90 ms | +28.67 ms |
| 3 | +164.46 ms | +150.10 ms |
| 4 | +1.99 ms | +64.15 ms |
| 5 | -123.87 ms | -26.30 ms |

Four editor pairs regress; its p50 is 64.15 ms slower. Faster aggregate UI/maxima
do not clear the owner's no-slower line. Other lanes' load changed substantially
between runs; the paired results above remain failures, without a noise allowance.
The earlier round-2 batch before preload overlap is preserved in
`/tmp/ne-fixups2-evidence/before-preload/`; the final five pairs are `boot-r2.json`.
Their old `openMs` field included a subsequent diagnostic page evaluation and is
not an isolated open-action measurement. The harness now captures it immediately
at editor-open completion, before that evaluation; no corrected five-pair rerun
was possible after the disk stop.

The early fetch takes 7.1–22.3 ms and compile 8.0–22.3 ms. The document installer
still records a 122.1–147.0 ms module await: that interval includes the synchronous
install chain before the promise continuation can run, not another compilation.
Instantiation is 0.6–0.8 ms, mirror sync 3.0–3.5 ms and speculative font warmup
0.0–0.3 ms. Module overlap alone has not closed editor-open latency.

A separate profiled diagnostic (not one of the gate pairs) has isolated open
164.93 ms today versus 169.42 ms native. The main engine reports `adapter: null`,
zero pools and zero frames before open; afterward it has the M4 Max adapter, one
pool and eight sent surfaces, still zero frames. Its connection is ready before
open, but native GPU/pool initialization overlaps opening the editor. The renderer
profile samples 396.2 ms today and 380.1 ms native: layout reads dominate both
(88.9 / 92.9 ms `getBoundingClientRect`); native drawing initialization adds 4.2 ms
self time. The profiles do not prove a single extra forced layout or justify
changing canvas insertion speculatively. The editor metric is UI readiness, not
proof of the first native image; these snapshots still have zero native frames.

### HD playback: common feedback proves the display-capability transition

The harness uses the same Chromium Graphics.Pipeline compositor-receive to
DrawAndSwap presentation-feedback timeline for both canvas sinks, following S1.
It retains every submission and flags missing/superseded/duplicate mappings.
The windows are shown without activation entirely off every display using S1's
existing helper, with bounds checked before and after showing. Both paths assert
1920×1080 stage/raster size, Full resolution and adaptive quality off, with two
seconds warmup and twelve measured seconds.

| HD row | Shown / skipped | Feedback p50 / p95 / p99 / max |
|---|---:|---:|
| Today | 360 / 0 | 33.333 / 33.334 / 33.334 / 41.666 ms |
| Engine | 357 / 3 | 33.333 / 41.667 / 44.848 / 125.000 ms |

All 360 today and 357 native submissions match unique presentation feedback:
zero missing, superseded or duplicate mappings, so the native tail was not
removed by the parser's match bounds. The native row has zero render/layer
errors, zero legacy routes and no named evaluator failure; it still fails.

At page time 9670.1 ms, `MediaQueryList.refresh` changes only `colorSpace`
(displayP3 to srgb) and `edr` (true to false). Device size remains 1920×1080,
DPR 2, zoom 0.5, pan zero and Full resolution. No resize is pending. View sequence
4 becomes 5, generation 2 becomes 3, and exactly three frames are generation
drops. Raw display-query sampling observes the capability flip at 9659.7 ms.
This reproduces and identifies the original HD mechanism; it is not a mask
texture reallocation, size debounce, pool-budget change or DPR change. Whether
the capability loss is physical display sleep or offscreen window reassociation
is still unproven. First-show timing, both states and the callback stack are in
`path-mask-playback-r2.jsonl` and the preserved `after-preload-first/` traces.

The constant-generation assertion correctly failed on HD and stopped that run
before UHD. It is now soft-but-failing so future diagnostic runs preserve every
row. A later diagnostic records native display sleep/lock state and main-engine
startup reports. It completed another today-HD row (360 frames, zero skips,
feedback p95 33.334 ms, max 41.153 ms; physical display awake/unlocked), then was
interrupted before a native row completed. Thus **UHD has no common-timeline
result**; round 1's different-clock UHD comparison remains unresolved. The extra
hidden-HD diagnostic also did not complete.

### Validation and stop condition

Targeted startup/handoff/retry/activation tests: 74 pass; viewer timer/popout
suite: 15 pass. Web typecheck: zero errors and the existing IconCropper warning.
Node typecheck still has the existing generated cloud-client and TS6307 errors;
the one newly introduced optional-preload-field error was corrected. The latest
metric-only `openMs` correction and native-display diagnostic instrumentation
have not had a complete app rerun.

The third app run was stopped with SIGINT when free disk fell below the mandatory
40 GiB threshold (18 GiB observed). Its Electron and Playwright processes exited.
No further app run or build was started. The first playback attempt failed before
showing a window because the Electron evaluation context lacks `require`; it now
loads the existing harness through `process.getBuiltinModule('module').createRequire`.
All failed/interrupted evidence remains outside Git. There is no claim that the
editor, steady-generation, or UHD common-clock gate passes.


## fixups2 round 3: owner decisions and steady display configuration

The owner's round-3 decision accepts exact detached read-set validation at the
round-2 measured median **10.07 µs** (native samples 11.564, 10.633, 10.066,
9.821, 9.828 µs; legacy median 40.67 ns). This supersedes the earlier strict
nanosecond verdict. Exact validation stays in place; no further detached scope
optimization is required.

The owner also ruled that round-2 editor-open deltas (-26.30 to +150.10 ms) on
this shared machine cannot decide the performance gate. Keep preload/module
load overlap and deferred speculative font warmup. The five-pair boot harness
is committed in `e2e/engine-doc-boot.spec.ts`; its isolated CPU profile test stays
outside the timing pairs. The historical numbers above remain recorded, but
are not a current pass/fail verdict for editor opening.

### Phase 3c gate handoff: engine against today

| Row | Evidence / rerun | Status |
|---|---|---|
| Detached repeated queries, 100 layers × 40 properties × five keys | 10.07 µs with exact arbitrary-reference validation; round-2 `scope.json` | Accepted by owner, round 3 |
| Installed UI / opened editor | Five alternating off/on pairs, p50 and maximum, `e2e/engine-doc-boot.spec.ts` | Deferred by owner to the Phase 3c quiet-machine performance table; no new shared-machine timing claim |

For the quiet-machine row, run the committed boot spec under `pm-electron-lock`
with `PM_FIXUPS2_MEASURE=1`, an outside-repository `PM_FIXUPS2_EVIDENCE` directory,
and `--workers=1`. Record load and all pair deltas; enable `PM_FIXUPS2_PROFILE=1`
only for its separate profiling test. Reserve at least 40 GiB before build and
every app launch.

### Today's canvas on a gamut/EDR change

`legacy/gl/compositor.ts:731` creates its WebGL2 context with alpha, antialias,
premultiplication, preserve-buffer, power preference and desynchronization
options. It does not set `drawingBufferColorSpace` or subscribe to
`(color-gamut: p3)` / `(dynamic-range: high)`. Its context listeners at lines
747–762 handle GPU context loss/restoration. The viewer initializes this context
once (`extensions/viewer/viewer.ts:979`) and its resize observer updates size;
there is no gamut/EDR canvas reconfiguration. Existing legacy canvases therefore
retain their color configuration when those media queries change.

Native presentation now follows that behavior: sample the host's color
configuration at initial setup, retain it for capability-only changes and later
geometry updates, and log the newly observed capabilities. A media-query flip
alone neither sends a changed view nor retires the surface pool. Genuine size,
DPR, zoom and host-window changes retain their existing behavior. This is the
owner's explicit round-3 compatibility policy, superseding round 2's unresolved
question about whether a frozen canvas configuration was appropriate.


### Round-3 paired playback results

After the integration merge, the rebuilt release addon speaks
`protocol-2f8d7cab61015d02`. Both engines run the same animated feathered mask,
Full resolution and adaptive quality off, with two seconds warmup and twelve
seconds measured. One Electron process at a time runs under `pm-electron-lock`;
the guard observed 67.37–68.04 GiB free before these four launches. The table
uses the same Graphics.Pipeline receive → DrawAndSwap feedback parser for both
canvas sinks. Worker-predicted display times and legacy submission times are
retained only as separate diagnostics.

| Canvas | Engine | Shown / skips | Feedback p50 / p95 / p99 / max |
|---|---|---|---|
| 1920×1080 | Today | 360 / 0 | 33.327 / 33.379 / 34.141 / 36.567 ms |
| 1920×1080 | Native | 360 / 0 | 33.333 / 41.668 / 41.685 / 49.762 ms |
| 3840×2160 | Today | 360 / 0 | 33.327 / 33.456 / 35.491 / 36.530 ms |
| 3840×2160 | Native | 360 / 0 | 33.331 / 41.681 / 42.225 / 50.657 ms |

Every row maps all 360 submissions to 360 unique feedback events, with zero
missing, superseded or duplicate mappings. Both native rows keep generation 2,
viewSeq 4 and the session count constant throughout the measured span, with
zero skipped or repeated frames, render errors, layer failures or legacy routes.
The steady-playback assertion passes at both resolutions. The legacy context
stays the same object with `drawingBufferColorSpace: srgb` and unchanged context
attributes before/after each span.

This runtime sample did not observe a gamut/EDR query flip: both native hosts
reported P3/EDR throughout. The deterministic regression covers the flip itself
in both directions: native frames continue through independently changing gamut
and EDR queries, retaining generation 7 and viewSeq 1 with no drops; later real
DPR/geometry updates retain the configured color tuple. Logs record observed
changes, repeated unchanged updates do not spam them, and dispose removes the
listeners. The three targeted viewer suites pass 39/39 tests; renderer typecheck
has zero errors and the existing IconCropper warning.

The 125 ms generation-change stall is addressed by matching today's canvas
lifetime, and the observed playback now has zero skips. **The no-slower playback
performance gate is still not green:** the common-clock HD and UHD native tails
are slower than today's. UHD's earlier different-clock p95 35.1/33.8 ms comparison
is superseded by this completed common-clock row, not waived. No cadence samples
or tails were removed, and no performance tolerance was introduced.

Raw rows, all four traces, generation assertions, legacy context snapshots and
logs are outside Git in `/tmp/ne-fixups2-evidence/r3/` (`summary.json`,
`path-mask-playback-r3.jsonl`, `mask-r3-*.trace.json`, `playback.log`,
`viewer-unit.log`, `types-web.log`, `addon-build.log`). The opt-in spec passed in
1.3 minutes; that means collection and functional/generation assertions passed,
not that the timing comparison passed. `PM_FIXUPS2_RUN` gives evidence a unique
run tag; the extra hidden diagnostic is explicitly opt-in. After deleting
`engine/target`, run with `POWERMOVE_PM_BRIDGE` pointing to the absolute built
`apps/desktop/native/pm_bridge.node` path (as this run did), so the development
bridge remains available. The build target is deleted and all own app/build
processes exited.
