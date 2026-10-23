# Export through the job engine

Design notes for the Phase 2 export lane: plan section 17 (pipeline, encoder
input, dither, muxers, agent delivery, presets), section 20 (agents and MCP:
`export_video`, snapshots, `render_frames`), section 22 (the CLI), section 25
(infrastructure: `pm-cli parity` for renders; codec generation,
transparency, time, and cadence bars), section 26's Phase 2 list and gate,
contracts 12, 13, and 14. Branch `feat/ne-jobexport`. The Phase 3b delivery
lane (every format at cutover, presets, the Render Queue, offline media at
export; branch `feat/ne-exports`) is [its own part below](#phase-3b-delivery). The plan stays the
authority; this file records what was built, how it behaves, and what was
measured. It builds on [delivery.md](delivery.md) (the VideoToolbox encoder,
muxers, verification, atomic output), [pm-render.md](pm-render.md), and
[pm-audio.md](pm-audio.md).

## The pieces

| Where | What |
|---|---|
| `engine/crates/pm-export/src/render.rs` | The job pipeline: capability check, pm-render frames, pm-audio's offline mix, encoder input, VideoToolbox, AVAssetWriter, verification, atomic in-place output; agent stills (JPEG, PNG) |
| `engine/crates/pm-export/src/par.rs` | Row-parallel CPU conversion at the caller's QoS |
| `engine/crates/pm-node/src/jobs/` | The job engine's service: three lanes and the GPU gate |
| `engine/crates/pm-engine/src/jobs.rs`, `engine.rs` | Routing `submitJob`, `cancelJobs`, `agentFrames` to the service with a frozen document; read-your-writes waits; host calls by token; playback signals from viewers |
| `apps/desktop/src/main/engine/agent-jobs.ts` | Main's side: job snapshot sessions, export settings, contract 12's reply rule, `get_render_status`, run cancels, agent frame sessions |
| `apps/desktop/src/main/agent-tools/bridge.ts` | `export_video`, `render_frames`, `get_render_status` behind Use New Engine, with capability routing to the current exporter and capture |
| `apps/desktop/src/main/engine/outputs.ts` | Destinations and commits (contract 14), now also removing the job engine's work files |
| `engine/crates/pm-cli` | `pm-cli render` and `pm-cli parity render` |

## Pipeline

```text
JobDocument (frozen Arc<Document> at the session's revision)
  → plan_export: capability check (pm_render::unsupported_with_media + pm-audio's mix graph problems)
  → per frame: GpuGate admits bands → pm-render (project depth, Standard) → readback
      ─(sync_channel(1))→ encoder input (convert.rs: matrix, siting, dither; row-parallel)
                         + pm-audio offline mix for the frame's samples
                         → VideoToolbox → AVAssetWriter → verify → rename over the host's file
```

- **Frozen documents.** The engine hands the service an `Arc` of the session
  mirror at its applied revision (`JobDocument`). Edits after submission
  reach the session, never the running export (section 17: users keep
  editing).
- **Two threads.** The job lane's thread owns the `Renderer` and renders;
  conversion, mixing, encoding, and muxing run on a second thread with one
  frame queued between them, so the GPU renders frame n + 1 while frame n is
  encoded.
- **Encoder input** (amendment 6): ProRes 422 profiles take `v216`, ProRes
  4444 and XQ `y416`, H.264 `420v`. Conversion is the f64 reference in
  `convert.rs`, split across rows on scoped threads that first take the
  calling thread's QoS (utility in the job engine, so it never competes with
  the interactive engine for CPU).
- **Dither**: the desktop enables it for delivery formats at 8 bits or less;
  it is never applied to alpha (delivery.md has the tile and its tests).
- **Precision**: saved `project.colorDepth` selects 8-bit normalized, 16-bit
  float, or 32-bit float working targets. The desktop still sends a Float32
  request hint for wire compatibility; it does not promote project depth.
- **Audio**: `pm_audio::MixGraph` over the composition with a source cache;
  the samples of export frame n are `[sample(tick(n)), sample(tick(n + 1)))`
  at the project rate, so totals equal `round(duration × sr)`. Audio
  problems (undecodable sources) route the export.
- **Timecode**: MOV exports carry a `tmcd` track naming their first frame in
  the rate's timecode (frame `start × rate`, rounded half up; drop-frame at
  29.97 and 59.94; wrapped to a day). Phase 3b adds the start timecode,
  drop or non-drop, and the reel (below). MP4 cannot carry one
  (AVAssetWriter refuses it).
- **Alpha** (ProRes 4444): VideoToolbox describes an all-opaque frame as
  depth 24 and AVAssetWriter takes the track's description from the first
  sample, so an alpha export that opened on an opaque frame lost its alpha.
  Samples of alpha exports are re-described with depth 32
  (`mac/alpha.rs`); the coded frames are unchanged.
- **Atomic output** (contract 14): host services create the temporary file
  `open(wx, 0600)`; the job engine resolves its write handle, encodes into
  `<temporary>.<16 hex>.partial.<ext>` beside it
  (`AtomicOutput::in_place`), verifies, renames the work file over the
  temporary file, and asks host services to commit; main moves it into
  place (unique names for agents, hard links so nothing is replaced) and
  only then reports the real path. Failed, cancelled, empty, or unverified
  outputs are removed with their work files, and so are those of a crashed
  job engine (`abandonPort`) and leftovers in the agent folder at launch.
- **Verification** (delivery.md): codec, profile, pixel format, depth, tags,
  range, frame count, duration, audio samples, alpha, and sampled pixel
  values; its byte count is what `commitOutput` checks.

## Capability routing

`plan_export` fails with `JobError::Unsupported` before anything is
written when pm-render cannot draw the composition (text, shapes, effects,
masks, mattes, transitions, non-normal blends, motion blur, 2.5D, Blender,
compat level 2, and the rest of pm-render.md's list), when a medium cannot
be read, or when pm-audio cannot decode an audio source. The job ends
`failed` with code `unsupported`; agent frames fail per frame with the same
code. Main treats `unsupported` and `mediaOffline` (the host media store
cannot hand the engine the file) as routing, not failure: `export_video`
runs the current exporter and `render_frames` the current capture, and main
logs `export_video routed to the current engine: <reasons>`. A layer with
`mblur` on routes even when the export asks for motion blur off; the
current exporter is the conservative answer there.

## The job engine's service (`pm-node`)

Three lanes, each a utility-QoS thread with its own `Renderer` on one GPU
device:

| Lane | Work | Class (contract 12) |
|---|---|---|
| Snapshots | `agentFrames` batches | agent snapshot |
| Agent exports | `export_video` jobs | agent export |
| User jobs | export sheet and queue jobs, background | user job, background |

- **Queue, never fail for being busy** (contract 12 invariant 3). Each
  batch reports `queuedMs`.
- **Read-your-writes** (invariant 2): an agent frame request carries the
  agent's revision in its envelope; the engine holds it until the session
  has applied that revision and fails it with `staleRevision` after 2 s.
- **Outbox.** The service never touches engine state: responses, status
  events, and its host calls (`mintHandle`, `resolveHandle`,
  `outputDestination`, `commitOutput`) wait in an outbox the engine drains
  on its own thread; host answers come back by the service's token.
  Breadcrumbs of running work go to crash attribution (contract 7).
- **Cancel**: one job or every job of an agent run; a running export stops
  at the next frame and its temporary output is removed.

### The GPU gate (contract 13, amendment 12)

Every GPU submission of every lane passes one gate (`pm-node`
`jobs/gate.rs`): one submission in flight across the job engine, a lane
never submits while a higher class waits, and while anything plays,
`JobSlicer` sizes each submission (bands of 16 output rows rendered as
regions of interest, bit for bit equal to the same rows of a whole frame)
to the 1 ms target under the 2 ms bound.

**Every admission is released.** `render_frame` holds each admission in a
guard: a render that fails after admission (or panics) reports
`abandoned`, which frees the gate and the slicer's in-flight slot without
calibrating anything. Before this, one failed render left the gate taken
and every later batch and export waited forever (the review reproduced it
with a text layer added and removed). Agent stills also route through
`pm_render::unsupported_with_media` (once per composition per batch,
`StillRouting`) before any GPU work, as exports do in `plan_export`.

**GPU time, not wall time.** The gate used to measure each submission from
`queue.submit` to completion, which counts the wait behind the interactive
engine's frames: a 16-row band of a 720 px still measured 3.8 ms. The
renderer now brackets each frame's commands, inside the frame's own command
buffer (`pm_render::CommandBracket`, additive), with two one-workgroup
dispatches whose passes take end-of-encoder timestamps
(`pm_export::gpu_time::GpuSpan`; the second reads a texel of the frame's
output, so it runs after it; the samples are resolved in a later
submission, because resolving them in the same command buffer returned the
previous span's end). Idle, spans match the work (a 16-row band of the
heavy test below: 0.98 ms span, 1.6 ms wall; whole frame 5.4 ms span,
6.3 ms wall). During 4K engine playback they still include the interactive
engine's work that the GPU interleaves inside the span: cheap 720 px bands
measured up to 9.2 ms. Devices without `TIMESTAMP_QUERY` fall back to wall
time.

**Calibration.** Per-unit costs come from whole frames submitted while
nothing plays (EMA) and from one-unit probes of a pass first met during
playback (least sample: contention only adds time), never from a pass's
first submission.

**Over the bound.**

- A unit predicted over the bound is never submitted on that prediction.
  If the pass's last uncontended whole frame took more than the bound, it
  waits until playback stops (never probed): a band can cost as much as the
  whole frame, because a precomposition or a 3D layer renders whole for any
  region. Otherwise it gets one-unit probes so a contended estimate can come
  down: at once, then after 100 ms, doubling, at most 4 per playback.
- Whole-frame calibration predicts a fraction of the frame per band, which
  is wrong for content that renders whole for every band. A GPU-timed band
  over the bound during playback is a strike against its pass, unless the
  pass's uncontended whole frame fits the bound (then the excess was
  contention). A strike backs the pass off for 100 ms, doubling with each
  further strike up to 5 s; a band within the bound clears the strikes.

A hard stop after three strikes (r3 below) held light agent snapshots for
the rest of playback, because contended spans of cheap bands crossed the
bound: 3 batches in 60 s instead of 30. The back-off replaced it.

### Playback signals

The job engine slices while anything plays. Two sources, merged in main
(`NativeEngine`): the interactive engine's own deadline signals (its viewer
sessions' `Transport` events, `pm-engine` `viewer_transport`, with a fixed
60 Hz budget because viewers do not report their refresh rate), and the
editor's transport (`engine:editor-playback`, sent by the legacy runtime's
`transport` hook; main takes the frame budget from the window's display,
16.7 ms at 60 Hz, 8.3 ms at 120 Hz). The production viewer sends no
`Transport` events to the interactive engine (only the integration pages
do), so before the editor relay the job engine never sliced during user
playback in the app, and agent `render_frames` (which no longer pause the
user's playback, unlike today's `snapshotAsync`) rendered unsliced. Main
sends `playbackStarted` when the first source plays (again when the
shortest editor budget changes or the interactive engine starts, which
carries its own rate and budget), and `playbackStopped` when the last
stops; a closed window stops its playback, and a dead interactive engine
stops playback only if no editor plays. `frameDone` alignment still needs
the presenter's render thread to report completions; without it the slicer
slices reactively, as amendment 12 allows.

## Main: agent delivery (`agent-jobs.ts`, `bridge.ts`)

With Use New Engine on and the job engine running:

- **`export_video`** reads the project as the agent sees it (`__mcp_context`,
  which also refreshes the run's baseline), opens a `jobSnapshot` session,
  resyncs the canonical document (inline up to 4 MiB, else through a staged
  scratch file and a capability handle), and submits an `agentExport` job
  owned by the run with `agentAutomatic {suggestedName, extension}`.
  Settings match the current exporter: the composition size at scale 1
  rounded to even pixels, the composition's exact rate, all or the work
  area, ProRes 4444 (straight alpha when asked) or H.264 High at 16 Mbps,
  "Rec.709 (QuickTime and Safari)" (CICP 1-13-1, limited).
- **Replies** follow `agent_export_reply`: done answers
  `{path, projectId, width, height, fps, jobId}` with the committed path;
  failed throws the engine's sentence; cancelled throws; `mode: "async"`
  answers `{status: "queued", jobId}` at once; `wait` answers queued when the
  job has waited in the queue longer than 60 s, and keeps waiting while it
  runs. The hour-long tool timeout counts from job start: when the job
  starts running, main writes one space on the tool socket, which restarts
  the idle timeouts on both ends (the MCP bridge parses the trimmed reply).
- **`get_render_status {jobId}`** answers `{jobId, state, progress, path?,
  error?}` for jobs of the same window's agents; it is in the inspection
  subset. With the engine off it answers that exports finish before
  `export_video` returns.
- **Run cancels**: finishing a run without committing (rollback or cancel)
  sends `cancelJobs {kind: "run"}`. Agent jobs never resume: when the job
  engine restarts, unfinished agent jobs fail with "The job engine stopped
  during the export; run it again."
- **The output scope is per job**, not per window: several agent exports
  can run at once (`withAgentExport`'s one-per-window rule is the current
  exporter's).
- **`render_frames`** asks the renderer for the times and width today's
  capture would use and the project at the agent's revision
  (`__engine_frames_plan`), then sends one `agentFrames` batch (JPEG 0.74,
  motion blur off, Blender `cachedOrPending`; the longest edge is today's
  width, scaled for portrait compositions) on the window's `job` session,
  which is resynced at a new revision only when the canonical hash changed.
  The media tools' contact sheets take the same path.
- **Exports folder**: `~/Powermove`; background tests set
  `POWERMOVE_AGENT_EXPORTS`.

`apps/desktop/docs/MCP.md` documents `mode` and `get_render_status`; the
agent instructions (`codex/instructions.ts`) tell agents to follow a queued
export with `get_render_status`.
- **Relinks**: the service keeps resolved media paths per session with the
  digest of the asset record they were resolved for
  (`pm_export::render::asset_records`); a relinked asset (changed record)
  is resolved again, and assets that left the document leave the cache.
- **Destinations of routed exports**: an agent export from a window always
  gets its own unique file, even while a routed export sheet job's chosen
  file waits for the current exporter (`render-encoder.ts`).

## Main: the export sheet

With Use New Engine on and the job engine ready, `exporter.ts` hands ProRes
and MP4 exports (not agent exports, which route in main) to main
(`engine:export-video`) with the project and the sheet's rate, scale,
range, tier, and alpha. Main shows the save panel, binds the chosen file as
a `userChosen` write handle for the job's snapshot session
(`NativeEngine.chooseOutput`; the engine never sees the path), and submits
a `userJob` (replace policy: the panel confirmed it). Job statuses drive
the progress sheet (`engine:export-status`); its Cancel cancels the job;
a done export feeds caption sidecars and the sidecar memory. A routed job
comes back with `routed`, main records the chosen file for the window
(`render-encoder.ts` `routeToCurrentExporter`), and the current exporter
writes it without a second save panel (WebM and PNG sequences too since
Phase 3b, through `save.ts`). Phase 3b's sheet, presets, and Render Queue
are below.

## Fixed along the way

- `QuickTimeTag`'s TypeScript bindings said `cicp1131` while serde sends
  `cicp1_13_1`; the first export settings main built from the generated
  types failed to decode in the job engine and closed main's host port. The
  variants are named explicitly (wire unchanged) and a test checks the
  bindings against serde.

## The CLI

- `pm-cli render <project.pmv|.json> -o <out.mov|.mp4>` renders PMV4, PMV3,
  or JSON through the same pipeline: presets `master` (ProRes 422 HQ),
  `master-alpha` (ProRes 4444), `web` (H.264), or explicit format, quality,
  alpha, range, rate, size, color, dither, precision, composition, and
  `--media ASSET=FILE`. Embedded media is unpacked to a private temporary
  folder; linked media is read from its `sourcePath`. Exit 3 lists the
  reasons the engine cannot render a project, which the app routes to the
  current engine; existing files are kept unless `--force`.
- `pm-cli parity render` renders every corpus project the engine supports,
  at every captured sample, through `render_frame` (the path exports and
  agent stills take) and compares with the legacy captures by section 25's
  classes; routed projects are listed with their reasons.

## Tests and measurements

Measured on this Mac (Apple silicon, macOS 27, Electron 44) on 2026-10-14
and, after the review fixes, 2026-10-15, with Jude using it; numbers are as
measured.

| What | Result |
|---|---|
| `test-mcp.mjs --desktop` (Phase 2 gate), under the Electron lock | Pass, 2 of 2 runs after the review fixes (the first review run had raced a cold start: the smoke now waits for the hidden editor). `render_frames` returns the job engine's JPEG frames; `export_video` (wait) returns a `jobId` and an owner-only (0600) H.264 MP4 in the test folder whose codec, `yuv420p`, CICP 1-13-1 tags (primaries `bt709`, transfer `iec61966-2-1`, matrix `bt709`, range `tv`; the matrix and range checks had been commented out by an inline comment), 24 frames, audible AAC, and pixels check out with our FFmpeg; a ProRes export in async mode answers `queued`, reaches `done` with progress 1 through `get_render_status`, and carries a `tmcd` track at 00:00:00:00; an unknown job id is an error; a project with a text layer routes `render_frames` and `export_video` to the current engine and still delivers; with the text layer deleted, `render_frames` and `export_video` run in the job engine again at once (the review's stuck-gate repro); no temporary or empty files remain |
| e2e `engine-export.spec.ts` | Pass: the export sheet's H.264 and ProRes 4444 with alpha (`yuva444p12le`) come from the job engine (AVAssetWriter, no `Lavf` encoder tag), a text project from the current exporter (`Lavf`), one save panel each, nothing left beside them. `native-engine`, `export-mp4`, `export-destination` specs still pass |
| Agent snapshots during 4K engine playback (Phase 2 row), 60 s per run, frame bridge test pattern at 60 fps on the 120 Hz panel, final code (r4 and r5, 2026-10-15) | Baselines: 0 drops of 3,587, jitter p99 0.24 and 0.23 ms. Light, 4 frames at 720 px every 2 s on a 24-layer 4K project: 0 drops of 3,587, 30 batches, batch p50 274 ms, p95 406 ms, max 462 ms, jitter p99 4.40 ms; 39 strikes, because GPU spans of these cheap bands reached 9.2 ms under contention. Back-to-back batches: r4 0 drops of 3,587, 534 batches (2,136 frames), jitter p50 4.04 ms, p99 8.34 ms, 1,222 irregular intervals; r5 0 drops of 3,587, 556 batches (2,224 frames), batch p95 103 ms, jitter p99 4.39 ms, 15 irregular intervals (the first pre-review busy run had 589 batches, jitter p99 8.11 ms, 41 irregular intervals) |
| Over-bound passes during the same playback (r4): 2 frames at 3840 px of 1,500 solids in a full-size 4K precomposition, whose 16-row band takes about 6.7 ms of GPU time idle | Cold (first met during playback): 0 drops of 3,588, jitter p99 4.39 ms; 6 submissions during playback (the cold one, the calibration probe, 4 probes, then the probe limit), the largest GPU span 18.0 ms; no batch finished during playback, and it finished 428 ms after the stop. Warm (one agent turn before playback, 461 ms): 0 drops of 3,588, jitter p99 0.22 ms; 17 sliced submissions during playback, every one over the bound (the largest span 24.4 ms), each striking the pass and backing it off up to 5 s; the batch finished 345 ms after the stop |
| Earlier rounds, same harness | Pre-review (wall time, unlimited 100 ms probes): light 30 batches, busy 589, 0 drops; the playback runs used solid layers only. r2 (wall time, probe limit, 96 solids at 1920 px in a precomposition, whose band is 0.2 to 0.5 ms of GPU time idle): 0 drops, but wall-time probes measured over the bound and the batch waited for the stop. r3 (GPU spans, a hard stop after 3 strikes): 0 drops in every run, but light snapshots finished 3 batches and busy 31, because contended spans of cheap bands crossed the bound. Summaries in `bridge-measurements/*-jobexport*.json` |
| Pipeline transparency (section 2, amendment 8), media corpus | ProRes 4444 XQ → `y416`: 1,148,760 samples, 0 differ (3,240 color samples under α = 0 not compared); ProRes 422 HQ → `v216`: 576,000 samples, 0 differ |
| Codec generation, ProRes 422 from 422 HQ | Engine vs Apple's re-encode: max 0.000 CV at 10 bits; Y′CbCr PSNR 63.549 dB both (Δ 0.000) |
| Codec generation, ProRes 4444 with alpha (premultiplied, amendment 8) | Max 1.603 CV at 10 bits overall, 0.000 beyond 8 px of α = 0; Y′CbCr PSNR Δ −0.184 dB, Y′ Δ −0.004 dB. Not met within 8 px of α = 0, as S2 found: Apple keeps the source's color under α = 0, a premultiplied pipeline cannot, and the 8 × 8 DCT carries it into the block |
| Codec generation, H.264 at a matched data rate (amendment 8) | Y′ PSNR engine 42.123 dB, Apple 41.643 dB (Δ +0.480). Chroma, not judged by the bar: engine 34.337 dB, Apple 41.676 dB against VideoToolbox's own 4:2:0 decode of the source (see below) |
| Time bar (`time_bar_full`, release, 2026-10-14) | Partial. 91,800 frames at 9 of the bar's 10 rates (10,000 consecutive from 0, 100 at 10⁶, 100 at 10⁷), each carrying a binary frame code burned into its pixels, all read back in order; `tmcd` names each window's first frame (drop-frame at 29.97 and 59.94; section 2's vectors); 294 s. Missing: 29.97 non-drop (exports label 29.97 and 59.94 drop-frame until presets choose), the burned-in timecode string (text is not in the Phase 2 renderer), and a `tmcd` check per frame rather than per window |
| Job exports (`job_export.rs`) | Generated projects export and verify in every Phase 2 format; sliced frames equal whole frames bit for bit; the pipeline equals rendering and converting each frame; unsupported projects and offline media route before writing; a cancelled export leaves nothing; host temporary files are replaced in place; alpha survives an opaque first frame |
| `pm-node` jobs (`tests/jobs.rs`) | Agent exports commit through host services with unique names; unsupported exports fail as routed without an output; run cancels remove the temporary file; agent frames answer at the agent's revision; during playback they are sliced and still answer; an unsupported batch (text, routed before the gate) and an offline-media batch are each followed by a supported batch that answers at once, and an export after them completes; a render that fails after admission releases the shared gate, idle and while playing; a relinked asset is resolved again in the same session |
| GPU gate (`pm-node` `jobs/gate.rs` tests) | Whole frames idle; sliced during playback; abandoned admissions free the gate and the in-flight slot; known-over-bound passes are never probed; probes of a pass first met during playback back off (0, 100, 200, 400 ms) and stop at 4; contended inflation of a pass within the bound is probed and is no strike; strikes back off 100, 200 ms and a band within the bound clears them; higher lanes first; cancel releases a waiter; aligned slices |
| GPU spans (`job_export.rs`) | A measuring gate gets GPU-timestamped times on this Mac (`TIMESTAMP_QUERY`); spans grow with the work (4 solids at 320 px ≈ 0.25 to 0.46 ms, 200 solids at 3840 px ≈ 3.7 to 4.1 ms); `measure_heavy_precomp_band_cost` (ignored, timing): a 16-row band of 1500 solids in a 4K precomposition at 3840 px takes 6.7 ms of GPU span at p50 idle, 400 solids 1.0 ms, 96 solids 1.1 ms or less; whole frames 12.2, 5.4, 1.6 ms |
| Every admission released (`job_export.rs`) | A text layer rendered without routing fails after admission and is reported `abandoned`; an offline clip is a layer failure of a finished render; a still routes the text layer before admitting anything |
| `pm-cli parity render` (corpus `v1-20261012-230504-ae47a3b3` and `render-20261014-131536-ea28ddc1`) | 95 frames compared, 0 failed (standard 32, alpha 25, video 24, standard+3D 14); 126 projects routed with their reasons; 3 skipped (the legacy frame shows a playback proxy) |
| `pm-cli render` tests | JSON with linked media and PMV3 with embedded media render to verified files; routed projects exit 3 and write nothing; existing files kept unless `--force` |
| Unit tests | `agent-jobs.test.ts` (settings, reply rule, queue bound, staging, run cancels, restarts, frames, export sheet jobs, the start callback), `bridge.test.ts` routing and the job-start keepalive on the tool socket, `native-engine.test.ts` editor and interactive playback merged, `host-services.test.ts` work files, `render-encoder.test.ts` routed destinations and agent exports never taking them, pm-engine playback signals and contended calibration, pm-node ended-job pruning, pm-host work files. Vitest over the lane's main, agent-tools, runtime, and codex tests: 439 of 441; the two failures are `instructions.test.ts`'s golden and 10 KB checks, which fail on `feat/native-engine` too (the golden lacks the whole delivery paragraph; the instructions were 10,769 bytes before this lane's sentence, 10,888 after) |

The H.264 chroma number needs a look before Phase 3b. `convert.rs`
decimates 4:2:0 chroma by point sampling horizontally (siting type 0) and
the inverse-linear filter vertically, which makes section 2's invariant (1)
exact; the comparison's reference is VideoToolbox's own filtered 4:2:0
conversion of the source, which favors Apple's re-encode, so the gap is
part filter and part reference. A filtered horizontal decimation for
rendered (non-pass-through) frames would close most of it without touching
invariant (1), which applies to pass-through sources.

## Changes outside this lane's files

The lane owns `pm-export`, the job engine's render and export service
(`pm-node` `jobs/`, `pm-engine` `jobs.rs`), main's export routing and agent
tool handlers for `export_video` and snapshots, `pm-cli`, and this file.
It also changed, minimally:

- `pm-protocol`: `QuickTimeTag`'s TypeScript names (wire unchanged).
- `pm-engine` `engine.rs` (routing jobs, viewer transports to deadline
  signals) and `slicing.rs` (`calibrate_contended`, `JobSlicer::abandoned`).
- `pm-render` `renderer.rs`: `CommandBracket` and `Renderer::set_bracket`,
  additive (no bracket, no change).
- `pm-gpu` `device.rs`: `TIMESTAMP_QUERY` among the optional features.
- `pm-host` `outputs.rs`: removing the job engine's work files.
- Main: `engine/index.ts` (export sheet jobs, the editor playback relay),
  `engine/native-engine.ts` (playback merge), `engine/engine-process.ts`
  (a status log line for measurement harnesses, only with
  `POWERMOVE_ENGINE_STATUS_LOG_MS`), `render-encoder.ts` (routed
  destinations), `codex/instructions.ts` (one sentence), `docs/MCP.md`.
- Preload and `shared/engine-ipc.ts`: `nativeEngine.exportVideo`,
  `cancelExport`, `onExportStatus`, `playback`.
- Renderer: `legacy/core/exporter.ts` (the export sheet through the job
  engine), `runtime/install-legacy.ts` (the transport hook that reports
  playback), `legacy/assistant/harness.ts` hooks.
- The frame bridge measurement harness (`measure-main.ts`,
  `frame-bridge-measure.mjs`): agent snapshot runs.

## Not done in Phase 2

- **Cadence bar.** It is a presentation bar (23.976 and 25 fps content on
  60 and 120 Hz displays); the frame bridge harness measures 60 fps content
  only. It belongs with the presentation lane's S1 harness.
- **`frameDone` signals.** The presenter's render thread does not report
  frame completions, so the job engine slices reactively.
- **GPU spans under contention.** Spans leave out the queue wait but still
  include other clients' work the GPU interleaves inside them (cheap bands
  measured up to 9.2 ms during 4K playback). Per-pass Metal command buffer
  times (`gpuStartTime`/`gpuEndTime`) are not reachable through wgpu.
- **The interactive engine's playback budget** stays a fixed 60 Hz
  (`pm-engine` `VIEWER_FRAME_BUDGET_US`) because viewers do not report
  their refresh rate; the editor relay takes the display's rate (this Mac's
  panel runs at 120 Hz).
- **Agent snapshots during the editor's own playback** in the real app were
  not measured: the playback runs above use the frame bridge harness's
  engine playback; the editor relay is unit-tested only.
- **Codec generation, ProRes 4444 alpha**, within 8 px of α = 0: measured,
  not met (above).
- **29.97 non-drop timecode**, the start timecode, the reel, the render
  queue, presets, and user jobs that survive relaunch: done in Phase 3b
  (below).
- **The hidden MCP host** keeps the current exporter until Phase 3c; its
  `test-mcp.mjs` mode was not rerun here (only `--desktop` changed
  behavior).
- **Base-machine numbers**: everything above is this Mac only.

# Phase 3b: delivery

The delivery lane (plan section 17, section 9's output tags and ProRes
ranges, sections 14.6 and 14.8, section 25's 3b rows, amendments 6, 7, and
8). Branch `feat/ne-exports`. Measured on this Mac (Apple M4 Max, macOS
27, Electron 44) on 2026-10-15 and 2026-10-16, with other lanes' builds and
Electron runs sharing the machine (load averages 7 to 17); numbers are as
measured.

## Formats at cutover

| Sheet format | Container | Encoder input | Encoder | Alpha | Notes |
|---|---|---|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ | MOV | `v216` (amendment 6) | VideoToolbox | no | PCM by default |
| ProRes 4444, 4444 XQ | MOV | `y416` | VideoToolbox | straight or premultiplied | opaque exports code no alpha channel (`PreserveAlphaChannel` false; before, every frame carried a 16-bit all-ones alpha) |
| H.264 High | MP4 or MOV | `420v` | VideoToolbox, VBR at the tier | no | AAC in MP4, PCM in MOV |
| HEVC Main / Main10 | MP4 or MOV | `420v` / `x420` (10-bit, MSB-aligned) | VideoToolbox, VBR at the tier | 8-bit only | |
| HEVC with alpha | MOV | `v0a8`, codec type `muxa` | VideoToolbox, average bit rate (its encoder accepts VBR and fails every frame, -12912) | straight or premultiplied (`almo` box's mode byte: 0 straight, 2 premultiplied) | depth 32 |
| WebM VP9 / VP8 | WebM | `yuv420p` / `yuva420p` | libvpx through `pm_media`'s FFmpeg bindings (deadline good, cpu-used 2, row-mt and tile columns for VP9) | yes (`alpha_mode` tag) | Opus at 48 kHz; frame-exact (replaces "WebM realtime capture") |
| PNG sequence / still | folder of `<name>_00000.png` (≥ 5 digits) / file | RGBA8 or RGBA16, straight alpha | `png` crate | yes | `sRGB` chunk plus `cICP` (1-13-0, or 1-1-0 with "Rec.709 image"), full range; verified lossless |

`ExportSpec::input_format()` picks the input from codec and alpha.
Conversion (`convert.rs`) stays the f64 reference: Rec.709 matrix, siting
type 0, 4:2:0 vertical filter, the deterministic blue-noise dither (±0.5 LSB,
8-bit outputs only), round half to even, clipping to the codable range.
Verification (`verify.rs`) covers every format: for MOV and MP4 the movie's
boxes (`hvcC` profile and depth, `almo` mode, PCM bits and float flag,
`tmcd` and reel), for WebM a demux and libvpx decode of sampled frames, for
PNG the decoded files against the encoder input exactly; per-codec pixel
tolerances.

## Settings and audio

- Today's settings survive: tiers 4, 16, 40 Mbps; scale 0.01 to 16 and a
  custom size; the frame-rate list; work area or whole composition; alpha.
  PNG takes odd sizes; video is rounded to even pixels.
- Audio: PCM 16, 24 (MOV default), 32-bit float, or AAC (AudioToolbox,
  MP4 default, priming in `edts`) in QuickTime family files; Opus in WebM
  (mixed at 48 kHz). Integer PCM is converted with `pm_audio::offline::encode`
  and TPDF dither seeded by the export sample index, so two exports of a
  project are identical.
- Timecode: `DeliveryOptions.timecode {start, dropFrame, reel}`; MOV only,
  drop-frame only at 29.97 and 59.94, `01:00:00;00` DF names frame 107,892.
- Color: "Rec.709 (QuickTime and Safari)" (1-13-1) or broadcast (1-1-1) for
  video; sRGB or Rec.709 image tags for PNG.
- Captions: burned in as today, or left out when a sidecar is delivered
  (`hideCaptions`: captions layers render as nothing, equal frame for frame
  to the project without them); SRT and VTT sidecars are written beside the
  output by main.
- Comfort (section 17): the system stays awake while an export or the queue
  runs (`powerSaveBlocker`), a notification when one finishes while the
  window is not focused, a size estimate in the sheet (Apple's ProRes rates:
  45, 102, 147, 220, 330, 500 Mbps at 1080p29.97, scaled by pixels and rate;
  bit rate times duration otherwise), overwrite policy per export (keep
  both, replace, stop), file name tokens (`{project}`, `{composition}`,
  `{preset}`, `{format}`, `{size}`, `{width}`, `{height}`, `{fps}`,
  `{date}`, `{time}`), start timecode and reel name.

## Presets and migration

`shared/delivery.ts` holds the format catalog, delivery options, the five
built-in presets ("Master · ProRes 422 HQ", "Master with alpha · ProRes
4444", "Web · H.264", "Web · HEVC", "Image sequence · PNG"), and
migration: stored format ids `prores` become ProRes 4444, `mp4` H.264, and
`webm` and the realtime capture `rec` WebM VP9; saved presets and project
export defaults migrate through the same function. With Use New Engine off
the sheet offers today's formats only (a project's stored new-format
default falls back to its legacy equivalent).

## The export sheet and the Render Queue

- The sheet is a Settings sheet: Output, Rendering, Delivery, File,
  Captions, and Presets cards; rows appear only for settings the format
  reads (`fieldSupport`). Actions: Cancel, Add to Queue, Export.
- The progress sheet keeps a live preview: job statuses carry a JPEG of the
  latest encoded frame (`JobStatus.preview`, at most 640 px, every 250 ms
  and the last frame).
- The Render Queue panel (`render-queue-panel.ts`, main `render-queue.ts`):
  jobs from the sheet (Add to Queue) and agent exports while they render,
  each with its destination; Start Queue and Pause Queue; jobs run
  unattended in the job engine from snapshots taken when they were added; a
  failed job shows why and the next one runs; a job menu (Cancel, Render
  Again, Show in Finder, Move Up/Down, Remove). User jobs persist in
  `<userData>/Render Queue/queue.json` with one snapshot file per job and
  survive relaunch (a job running at quit is pending again).
- Offline media at export (section 14.8): before the save panel main lists
  offline clips and asks with a native message: Locate Media…, Export with
  Proxies (only when proxies exist), Cancel. Agent exports fail with
  `mediaOffline` (routed); a queued user job waits (`waitingForMedia`,
  rechecked every 2 s) instead of failing.
- Routed jobs (the engine cannot render the project): the sheet's export
  falls back to today's exporter at the chosen destination (ProRes 4444,
  H.264, WebM VP9, PNG); an engine-only format says so; a queued job fails
  with the reason.

## Agents

`export_video` takes `preset` (`master`, `master-alpha`, `web-h264`,
`web-hevc`, `png`) or any format id of the catalog (today's `mp4` and
`prores` still accepted); `docs/MCP.md` and the tool schema document them.
An explicit format overrides the preset's, alpha included: alpha defaults to
the preset's only where the written format keeps it (`master-alpha` with
`h264` writes opaque H.264; with `prores-4444xq` it keeps alpha). When the
current exporter takes the export (Use New Engine off, offline media, or a
project the engine routes), the formats and presets it writes still work:
`mp4`/`h264`/`web-h264` become its MP4 and `prores`/`prores-4444`/
`master-alpha` its ProRes 4444; the preset's delivery options (for example
`master-alpha`'s 24-bit PCM) are not carried over. Other formats and presets
fail with the reason (`legacyExportArgs`, `bridge.test.ts`).
Agent exports keep contract 12's reply rule; PNG sequences answer the
folder's path.

## Contracts

- Contract 12: `ExportSettings.delivery` (`DeliveryOptions`: audio coding,
  timecode, hide captions, preview, sequence name), `JobStatus.preview`;
  both optional on the wire. This answers RFC 12's open question 4.
- Contract 14: `OutputDestinationRequest.folder`: the host creates an
  owner-only temporary folder, the engine writes frames into a work folder
  beside it, and the commit moves the folder. Replace moves the frames into
  an existing folder, then removes the frames of an earlier, longer export of
  the same sequence (same `<name>_<digits>.<ext>` name, digit count and
  extension), so the folder never mixes old and new frames; other files stay.
  `pm-host`, main's `outputs.ts` and `pm_export::AtomicOutput` implement it.

## Tests and measurements (Phase 3b)

| What | Result |
|---|---|
| Conformance per format (`tests/delivery.rs`, `formats.rs`, `videotoolbox.rs`) | Every format at cutover from one project exports and verifies (320 × 180, 30 frames; worst block error against the encoder input: ProRes HQ 0.063 CV, HEVC 10-bit 3.453 CV at 10 bits, VP9 1.906 CV, PNG 0); HEVC 8/10-bit in MP4 and MOV, HEVC alpha straight and premultiplied, PCM 16/24/float and AAC, H.264 in MOV with timecode and reel, WebM VP9/VP8 with and without alpha and Opus (checked with FFmpeg too), PNG sequences and stills, nothing left behind on failure or drop; opaque ProRes 4444 and XQ code no alpha channel (frame header `alpha_info` 0; it was 2) |
| Bit-depth bar (`bars_delivery.rs`) | ProRes 422 HQ ramp: 877 luma codes 64 to 940, monotonic, step 1, chroma neutral; 897 Cb codes 64 to 960. 16-bit PNG: every column on the ideal value, largest step 18 (≤ ⌈65535 / 3839⌉). Dithered 8-bit within 0.5 CV |
| Alpha bar | ProRes 4444 alpha exact at 16 bits; color worst 1.34 CV straight, 2.09 CV premultiplied (12-bit, α ≥ 1/64). HEVC with alpha: PSNR engine 53.20 dB, AVFoundation's preset 53.08 dB (mean of three frames) |
| SDR color bar (chart through each codec, ΔE2000 decoded by FFmpeg and AVFoundation) | H.264 patch centers 0.496 CV, ΔE max 0.628 mean 0.162 (AVFoundation decode 0.900 / 0.176); HEVC 10-bit 0.495 CV, ΔE 0.127 / 0.043; ProRes HQ 0.070 CV, ΔE 0.127 / 0.042 |
| Audio bar | Clicks land on their samples at 60000/1001 (frame 37: sample 29630 for 29629, within ±1); totals equal round(duration × rate). It found a pm-audio bug: a clip's first source sample was dropped (fixed, below) |
| Export safety (`delivery.rs`) | Encoder input of an export after preview renders at 160, 80, and 320 px in f16 equals a fresh renderer's, frame for frame; 16-bit PNG sequence files byte-identical between the two |
| VMAF, H.264 (`measure_delivery.rs`, 1080p, 48 frames, libvmaf v2 on the engine's own `420v` input; x264 from a GPL FFmpeg, measurement only) | Landscape pan with grain: 4 Mbps VideoToolbox 95.57 (4.16 Mbps actual), x264 medium 95.11, slow 95.16; 16 Mbps 96.17 / 95.93 / 96.01; 40 Mbps 96.56 / 96.21 / 96.28. Color bars: 4 Mbps 95.03 / 94.93 / 95.07; 16 Mbps 97.95 / 96.87 / 96.96; 40 Mbps 98.32 / 97.96 / 98.03. Before VBR (`AverageBitRate`): VideoToolbox undershot to 0.72, 1.28, 17.2 Mbps on the grain clip (VMAF 93.39, 94.86, 96.17). No software encoder is needed for H.264 |
| Throughput, job engine in-process (`measure_delivery.rs`, UHD ProRes source, 48 frames, end to end incl. verify) | UHD ProRes 422 HQ with 3 layers: 20.9 fps (12.3 fps in a run under heavier load; real time is 23.976). 1080p: ProRes Proxy 46.9, HQ 46.3, 4444 43.6, 4444 XQ 42.0, H.264 40.1, HEVC 40.6, HEVC 10-bit 42.1, HEVC alpha 43.2, VP9 10.3, VP8 23.7, PNG 8-bit 43.9, PNG 16-bit 34.5 fps. UHD render plus convert alone: 19.8 fps (render and f32 readback dominate; `v216` conversion is 6.5 ms per UHD frame; FFmpeg decodes the source at 78 fps) |
| Throughput against today's exporter (`e2e/export-baseline.spec.ts`, one 1080p project of 6 moving solids, 144 frames, wall clock from Export to done) | Today's exporter / job engine: H.264 63.8 / 52.9 fps (0.83×), ProRes 4444 63.7 / 59.5 (0.93×), WebM VP9 227.8 / 11.1 (0.05×; today's is realtime-deadline WebCodecs at 4.4 Mbps), PNG 40.3 / 32.9 (0.82×). The engine's H.264 writes 16.4 Mbps where today's undershoots to 1.9 Mbps at the same tier |
| e2e (`engine-delivery.spec.ts`, under the Electron lock, one worker) | The sheet's formats, presets, start timecode and reel, live previews, HEVC 10-bit MOV, 16-bit PNG sequence, VP8; the offline-media message (Cancel, Locate Media…); the Render Queue (three jobs including a routed failure, persistence across relaunch, the job menu's Remove). With `engine-export`, `export-dialog`, `export-video-frames`, `export-webm`, `export-mp4`, `export-destination`: 22 passed |
| Unit tests | Rust: pm-export, pm-node, pm-host, pm-protocol, pm-cli: 178 passed, 7 ignored (measurements); one load-dependent failure in the first of three runs did not recur. Vitest: main and shared 1,514 passed; the two failures are `instructions.test.ts`'s golden and 10 KB checks, unchanged from `feat/native-engine`. Renderer core and legacy: 558 passed earlier in the lane (one pre-existing timeout) |
| Lint | `cargo clippy -D warnings` and `cargo fmt --check` clean for the touched crates; `svelte-check` and `tsc -p tsconfig.node.json` clean |

## Changes outside this lane's files (Phase 3b)

- `pm-media` `ff/mod.rs`: `init()` public (FFmpeg log level and
  registration before the WebM muxer).
- `pm-audio` `mixer.rs`: `sample_at` takes amendment 1's tolerance
  (`1e-9 × max(1, |n|)`), and a copy reader's voice window uses the reader's
  own rounded lattice (`voice_bounds`), so a clip's first source sample is
  no longer dropped (found by the audio bar).
- `pm-host` `outputs.rs` and `lib.rs`: folder temporaries, `remove_path`,
  `output_bytes`, folder placement.
- `pm-protocol`: the contract 12 and 14 additions above; fixtures and
  generated TypeScript regenerated.
- Main `save.ts`: a routed WebM or PNG sequence export takes the user's
  destination (the folder is made) instead of asking again.
- Renderer `legacy/ui/project-settings.ts` (engine-aware format list),
  `css/app.css` (Render Queue rows).

## Gaps (Phase 3b)

- **4K ProRes 422 HQ export at real time on base**: not met on this Mac
  under shared load (20.9 fps with three layers, 23.976 needed), and no base
  machine was measured. Rendering and the f32 readback dominate; the GPU
  encoder-input pass (convert on the GPU, read back `v216`, a quarter of the
  bytes) is designed but not built: exports convert on the CPU.
- **At least today's baseline, every format**: not met for WebM VP9 (0.05×:
  today's WebM is a realtime-deadline encode; the engine's is libvpx good
  quality, cpu-used 2), and below 1× for H.264 (0.83×, writing 8.6× the
  bits today's encoder does at the same tier), ProRes 4444 (0.93×), and PNG
  (0.82×). One project, this Mac only.
- **Proxies** do not exist until Phase 4a: "Export with Proxies" appears
  only when main's `resolveProxy` finds one, which it never does yet; draft
  exports without proxies fail.
- **Blender layers** route to today's exporter; the Blender-required
  message is not wired.
- **Routed queue jobs** fail with the reason rather than running today's
  exporter unattended; engine-only formats cannot fall back.
- **16-bit PNG still sources** are routed by pm-render; the bars use 12-bit
  ProRes 4444 XQ sources instead.
- **Export safety** covers preview renders at reduced sizes and f16 only;
  proxies, Blender previews, and the Phase 4b disk cache do not exist yet.
- **Audio "no length limit"** is not tested here.
- **8-bit PNG** dithers by default, which enlarges files.
- **Base-machine numbers**: everything is this Mac only.

## Standard transparent PNG compatibility (E1)

Today's `alphaFrame` reads the premultiplied accumulator into RGBA8,
truncates `(RGB * 255 / alpha)`, then sends that ImageData through Canvas2D
before PNG encoding. Directly dividing the float frame before quantization
changed translucent stills by up to 1.97 premultiplied CV. `convert.rs`
reproduces both byte boundaries for standard-color, straight-alpha RGBA8
PNG output, including Canvas2D's rounded premultiply and PNG's reciprocal
unpremultiply. This boundary is byte-quantized and does not add dither.
Linear output, 16-bit PNG, float rendering and other formats retain their
existing conversions.

The E1 conversion is selected once from the frozen job document. A document
containing any compat-level-1 layer (including a missing `compat` field) uses
the legacy boundary. Every composition is considered, while the stored open
composition wins over its stale `comps` copy. Documents whose layers are all
level 2 use direct float unpremultiplication before byte quantization, so
low-alpha color retains its precision. `Pipeline::new` (and so
`pm_export::export`) defaults to the legacy boundary, so a caller that never
calls `set_legacy_png` keeps today's level 1 bytes; the job renderer selects
direct precision only for all-level-2 documents. Linear and 16-bit output
retain their existing conversions.

Unit tests pin both conversions and document selection, including mixed and
nested compositions. `job_export::png_byte_compatibility_follows_the_frozen_document`
checks the actual GPU-to-export pipeline: a 0.3%-opaque solid gives the
legacy black RGB bytes at level 1 and preserves its #336699 color at level 2.

The hidden-app transparent-export regression passes all 58 drawn
orientation/color-description cases at max 1 CV, no pixels above 1, with
completed native export jobs and no fallback. The oriented PNG-alpha and
AVIF-alpha E1 examples both measure 0 CV. P3 and Adobe RGB PNG/TIFF alpha
ramps also compare at 0 CV after Chromium decoding; the decoded straight
PNG channels can differ by 1 byte while premultiplied pixels are identical.
The Rust regression pins alpha 0/low/128/255, RGB above alpha, and unchanged
16-bit quantization. The separate legacy/native float accumulator comparison
remains below 0.125 CV for all eight ICC compositions (`media.md`).


## B1: export media binding lifetime (2026-10-18)

Sheet, queue and agent exports wait for the renderer's host-media binding
acknowledgement. Main's queued project snapshot records copied-media hashes at
Add to Queue, and `AgentJobs` pins copied and linked bindings to each export's
snapshot session before resync. Host-service resolution selects those bindings
by process and session until the export ends, independently of open windows.
See `media-store.md` for the lifetime and regression cases. The original
capability-handle checks still grant paths only to the owning engine port.

## Export encoding parity (EXPORT-1)

Decision EXPORT-1 (orchestrator, final form): for the same settings, the
engine hands each encoder today's frames and samples, uses today's encoder
implementation and settings, decodes to today's pictures and samples (or
within today's own run-to-run variance), and writes today's stream
properties. Branch `feat/ne-exportaudio`.

### Today's formats through today's encoders

| Format (today's id) | Today | Engine (job exports) |
|---|---|---|
| H.264 MP4 (`mp4`) | renderer RGBA8 → bundled FFmpeg (`render-encoder.ts` `encoderArgs`: swscale bt709 limited, `h264_videotoolbox` high, yuv420p, `-b:v 16M` per tier, `-g 250`) → FFmpeg mux with the WAV (`aac`) | `mac::legacy_video`: the same RGBA8 bytes (`convert` to RGBA8, over black, alpha 255, no dither) piped to the same FFmpeg with the same arguments; the same mux (`mac::legacy_audio`) |
| ProRes 4444 MOV (`prores`) | RGBA8 (opaque: `renderOpaqueFrame`; alpha: `alphaFrame`'s truncating unpremultiply and Canvas2D round trip) → FFmpeg `prores_videotoolbox` 4444 `p416le`/`ayuv64le` → mux with `pcm_s24le` | The same path for documents with level 1 content (`png_compat::has_level1_content`); straight alpha uses the PNG lane's `legacy_png_rgba` bytes. Level 2 content and other ProRes profiles keep VideoToolbox from the float frames |
| WebM (`webm`) | WebGL canvas → WebCodecs `VideoEncoder` (`vp09.00.31.08`, else `vp8`; tier bit rate; `latencyMode: 'quality'`; keyframe every `round(2 × fps)`), `AudioEncoder` Opus 160 kbps on 20 ms chunks, `muxWebM` | `webcodecs::WebCodecsSink` → `hostEncode` → main's `ExportEncoders` → a hidden encoder window running the same code (shared `webm-video.ts`, `opus-encode.ts`, `webm-mux.ts`) on a WebGL2 canvas with the compositor's context attributes, so `new VideoFrame(canvas)` takes Chromium's GPU REC709 readback |
| MOV/MP4 audio | `OfflineAudioContext` stereo 48 kHz, `ceil((t1 − t0) × 48000)` samples, 16-bit WAV (`wavBytes`: −32768/+32767 scaling, `Math.round`) → FFmpeg `pcm_s24le`/`aac` with `-shortest` | The engine's mix at 48 kHz for exactly that many samples (`render::legacy_audio_samples`), the same WAV quantization, the same FFmpeg command (see the frame fix below) |

New delivery formats (HEVC, ProRes 422 family, PCM 16/float, AAC in MOV,
Rec.709 broadcast tags, alpha WebM) have no counterpart today and keep
VideoToolbox/AVAssetWriter or libvpx/libopus. The CLI has no encoder window
and keeps libvpx/libopus for WebM ([export-opus.md](export-opus.md)).

### The WebCodecs transport

`pm_protocol::host_encode` (contract 14): `open` (codec, size, rate, bit rate,
frames, audio, the export's `outputWrite` handle), `video` per frame (RGBA8
inline up to 1 MiB, else a pm-bridge shared-memory segment main takes once;
the engine unlinks it after every step), `audio` (interleaved f32, ≤ 768 KB
per message), `finish` (main writes the muxed file into the handle's
temporary output; the engine then verifies it with `verify_webm` and commits
as usual), `abort`. Main answers frame and audio steps once the window has
them with at most four outstanding, so rendering overlaps encoding; a window
error fails the next step. The window (`export-encoder.html`, sandboxed,
own preload that only hands over a MessagePort) opens on the first encode
and closes 15 s after the last; encodes of an ended engine launch or idle
for five minutes are aborted. A MessagePortMain does not take transferred
ArrayBuffers, so the muxed file is copied, not transferred.

### Samples: Chromium's integer conversion

`decodeAudioData` converts integer PCM with Chromium's
`FixedSampleTypeTraits::ToFloat`: below zero × 1/32768, above zero × the f32
constant 1/32767 (8-bit: 1/128, 1/127). `pm_media::audio` now does the same
(it divided by 32768). On a 16-bit WAV at gain 0.8, all 96,000 pre-encode
samples equal today's (before: 48,080).

### A frame today's mux drops (decision COMPCLOSE-1)

Today's final mux passes `-shortest`. When the mix is shorter than the frames
(a whole-second range at 29.97 fps: 48,000 samples under 30 frames lasting
1.001 s), FFmpeg ends the file at the audio and drops the last frame (30 → 29
frames in MOV and MP4, measured with the bundled FFmpeg 9.0.2). That is a
missing frame, so the engine leaves `-shortest` out exactly in that case
(`LegacyMux::shortest`): every frame stays and the audio is today's. When the
audio is at least as long, the option trims it as today. Recorded for the
orchestrator as a COMPCLOSE-1 bug fix.

### Movie timescales

FFmpeg 9.0.2's `mov_init` sets the movie timescale to the denominator of the
GCD of its input streams' time bases, at least 1000: 1000 for an encode at
24/25/30 fps without audio, 24000 at 23.976, and lcm(video, 48000) after the
audio mux (24 fps: 1,536,000). `verify::Expected::movie_timescale` follows it
for FFmpeg-written files.

### Measured (`e2e/export-encoding-parity.spec.ts`, 2026-10-22)

Each fixture exports ProRes 4444, ProRes 4444 with alpha, H.264 MP4 and WebM
twice with the setting off and once (1080p: twice) with it on; inputs are
captured at both encoder boundaries in hidden test launches.

- Today against itself: identical in every format and fixture (decoded RGBA,
  decoded samples, encoded audio bytes, stream properties), including
  multi-threaded 1080p VP9.
- **basic** (320×180, 30 fps, 1 s, rotated translucent moving shape, 16-bit
  WAV at gain 0.8): engine identical to today in all four formats: encoder
  input RGBA8 and PCM, decoded frames and samples, encoded audio bytes (AAC,
  PCM24, Opus), every ffprobe stream property including `channel_layout`.
- **large** (1920×1080, 29.97 fps, 60 frames): decoded frames, samples and
  stream properties identical in all four formats (per-frame hashes).
- **automation**, **advanced** (gain keyframes, fades, trims, a nested
  composition with level and, in `advanced`, speed 1.2): video identical;
  audio differs because the engine's export mix is silent when the project
  holds that nested composition (see the gap below).
- Export time, 1080p, best of two, under shared-machine load 10–33: ProRes
  1.74 s vs today 2.05 s, ProRes alpha 2.00 vs 6.42, MP4 1.81 vs 2.45 (one run);
  the final run under heavier load: 4.73 vs 2.13, 5.87 vs 5.20, 2.71 vs 2.23.
  WebM 1.41–1.92 s vs today 0.50 s in both: slower (see gaps).

### Gaps

- **WebM export time** is about 3× today's at 1080p. Today's frames never leave
  the GPU (WebGL canvas → GPU readback → encoder, ~8 ms a frame); the engine's
  go GPU → f32 readback → RGBA8 → shared memory → main → window → texture
  upload. Matching it needs GPU-resident frames into the encoder window (the
  viewer's IOSurface frame bridge and `sharedTexture`) or an 8-bit readback.
- **Nested-composition audio in exports is silent** (all samples): a project
  whose root holds an audio layer plus a precomposition of an audio layer
  exports silence with the setting on (today: the mix). Each audio feature
  alone matches today to the sample level; pm-audio's offline mixer renders
  the same document shape correctly in a unit test, so the fault is in the
  app's job path (document or media binding of the nested composition).
- **Mix parity** beyond integer decode and length: pm-audio's documented
  "allowed differences" (speed and remaps followed where today ignores them,
  automation at 3 kHz instead of frame ramps, the resampler) are visible
  differences on existing projects under the owner rules; the corpus parity
  test still measures −82 dBFS (gain-fades) and −88 dBFS (precomp-nesting)
  residuals.
- A project frame rate held as a decimal that is not the rational's double
  (for example 29.97 rather than 30000/1001) gives today's encoders a
  different `-framerate`/`framerate`; the engine passes `num / den`.

## Phase 3c agents: strict batches, host packing and CLI delivery

This section supersedes the capability-routing behavior described above for
agent jobs. With the engine enabled, its presence selects native work even while
it is recovering. `unsupported`, `mediaOffline` and other engine failures retain
their named error; agent exports and frame batches never invoke a legacy fallback.
User job statuses produced by `AgentJobs` no longer mark failures as routed.
The broader sheet/router removal remains the derouter lane's work.

`AgentHarness.capture` and `observeVisual` now use the authenticated
`engine:agent-frames` batch endpoint, at JPEG 0.74 and motion blur off. The endpoint
uses the job lane, pins the document revision, waits for media bindings and
extension definitions, and never pauses/resumes playback or enters the exclusive
compatibility capture helper. MCP composition contact sheets submit all cells as
one engine batch so the document and definitions freeze together. A partial,
duplicate or out-of-order native response is validated and restored to request
order. At a contact-sheet deadline or cancellation, that batch alone is
cancelled and completed cells return with the remaining cells omitted. Ordinary
frame requests still require every requested frame. Frame sessions pin media like export
sessions, including across project changes.

Native `save_project` asks for a single serialized document envelope, workspace
and Undo snapshot, after pending imports finish. The host writes the same PMV3
container as today's agent save, then records the project with `Projects.put`.
The shared `project-assets.ts` helpers retain linked-original conversion and
Undo-reachable media. Each save gets an exclusive owner-only name and media
binding scope, including frozen blob MIME types. Missing sources are skipped;
their metadata stays in the document. Source changes during copying fail by
name and leave no output. Private document-plan responses have a separate bounded
budget from paginated public agent state.

`powermove render <project> <output> [options]` invokes the native CLI without a
shell, preserves its exit code and signals, and accepts user-chosen destinations
outside the agent exports folder. A missing runtime is
`native-renderer-unavailable`. `pm-cli render --param NAME=JSON` validates all
values before applying them to its isolated document, retaining each parameter's
definition. Existing definition snapshots, host fonts and Blender service remain
in the native CLI; a missing WebGL runner remains a named headless failure.

Agent exports, frame batches and contact sheets wait, bounded at 45 s, for a
starting or restarting job engine (the longest crash backoff is 30 s), then
fail as `The job engine is not running.`; a stopped engine fails at once.
Frames requested as PNG are labelled `image/png`. Native `save_project`
validates its (empty) arguments with today's strict schema and message.

Targeted tests cover native failures before/after job acceptance and during
recovery; pinning and playback isolation; incomplete frames; media binding
lifetimes; PMV3 byte equality, missing originals, MIME, Undo and exclusive naming; CLI argument boundaries;
parameter validation and rendered expression changes. The desktop MCP smoke now
checks native text, publication followed by frames, and host-packed PMV3 media and
Undo. `e2e/engine-agent-observation.spec.ts` measures twenty five-frame 720 px
observations during a UHD project's playback. Only measured runs establish its
performance result.

The macOS native hidden host now embeds host services and the production
per-project document workers. Its render jobs, batches and saves use the same
`NativeEngine` and `AgentJobs` as the desktop, over Node child-process ports.
The desktop persists its probe namespace beside the per-machine shader cache;
the hidden host, Node render command and direct bundled `pm-cli` read it, read
only: their first-use decisions stay in memory (hidden-host.md). Unknown
and pinned user programs request the desktop's WebGL runner in on-demand
Chromium. This is the shader runner only; composition frames remain native.
The hidden tabs present CPU snapshots from `pm-node`.

See [hidden-host.md](hidden-host.md) for runtime wiring, reproducible gates and
remaining compatibility dependencies. Both macOS architectures build through
`pm-node-addon`, `pm-bridge-addon` and the shipping release profile; no LTO
profile override is required after the release-build repair. Assembly requires
matching LGPL FFmpeg and native image-decoder licenses, matching architecture
checks, and clean Node 22 load tests before distributing artifacts.
