# Export through the job engine

Design notes for the Phase 2 export lane: plan section 17 (pipeline, encoder
input, dither, muxers, agent delivery, presets), section 20 (agents and MCP:
`export_video`, snapshots, `render_frames`), section 22 (the CLI), section 25
(infrastructure: `pm-cli parity` for renders; codec generation,
transparency, time, and cadence bars), section 26's Phase 2 list and gate,
contracts 12, 13, and 14. Branch `feat/ne-jobexport`. The plan stays the
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
  → per frame: GpuGate admits bands → pm-render (float, Standard) → readback
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
- **Dither**: on for H.264 (8-bit), off for ProRes unless asked, never on
  alpha (delivery.md has the tile and its tests).
- **Precision**: exports render at 32-bit (amendment 8 asks for 32-bit
  whenever a source exceeds 10 bits; agent exports do not inspect sources
  first, so they always do).
- **Audio**: `pm_audio::MixGraph` over the composition with a source cache;
  the samples of export frame n are `[sample(tick(n)), sample(tick(n + 1)))`
  at the project rate, so totals equal `round(duration × sr)`. Audio
  problems (undecodable sources) route the export.
- **Timecode**: MOV exports carry a `tmcd` track naming their first frame in
  the rate's timecode (frame `start × rate`, rounded half up; drop-frame at
  29.97 and 59.94; wrapped to a day). Start timecode and reel settings
  arrive with Phase 3b presets (contract 12, open question 4). MP4 cannot
  carry one (AVAssetWriter refuses it).
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
measured up to 6 ms. Devices without `TIMESTAMP_QUERY` fall back to wall
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
writes it without a second save panel. The render queue panel, presets,
and user jobs that survive relaunch are Phase 3b.

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
| Agent snapshots during 4K engine playback (Phase 2 row), 60 s per run, frame bridge test pattern at 60 fps | Baseline: 0 drops of 3,587 frames. 4 frames at 720 px every 2 s on a 24-layer 4K project: 0 drops of 3,588, 30 batches, batch p50 160 ms, p95 228 ms, max 238 ms, no failures or routing, `playbackStarted`/`Stopped` relayed. Back-to-back batches: 0 drops of 3,587, 589 batches (2,356 frames), p95 103 ms. Summaries in `bridge-measurements/*-jobexport.json`. Before the gate fix, the 2 s run finished no batch during 20 s of playback |
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

## Not done here

- **Cadence bar.** It is a presentation bar (23.976 and 25 fps content on
  60 and 120 Hz displays); the frame bridge harness measures 60 fps content
  only. It belongs with the presentation lane's S1 harness.
- **`frameDone` signals.** The presenter's render thread does not report
  frame completions, so the job engine slices reactively.
- **GPU spans under contention.** Spans leave out the queue wait but still
  include other clients' work the GPU interleaves inside them (cheap bands
  measured up to 6 ms during 4K playback). Per-pass Metal command buffer
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
- **29.97 non-drop timecode** in exports: the job engine labels 29.97 and
  59.94 drop-frame (the timeline's default); a start timecode, reel, and
  DF/NDF choice arrive with presets (contract 12, open question 4).
- **Render queue, presets, persistence of user jobs across relaunch**:
  Phase 3b. User exports run in the job engine but are not persisted.
- **The hidden MCP host** keeps the current exporter until Phase 3c; its
  `test-mcp.mjs` mode was not rerun here (only `--desktop` changed
  behavior).
- **Base-machine numbers**: everything above is this Mac only.
