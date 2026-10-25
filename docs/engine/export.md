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
  calling thread's QoS and refresh it as rows complete. User exports use
  default QoS while playback is idle and utility while it plays; agent and
  background work stays at utility (RFC 13 invariant 10).
- **Dither**: the desktop enables it for 8-bit PNG only, a final image; lossy
  codecs take the float frame rounded once (EXPORT-DITHER-1). It is never applied
  to alpha (delivery.md has the tile and its tests).
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
| WebM VP9 / VP8 | WebM | `yuv420p` / `yuva420p` | libvpx through `pm_media`'s FFmpeg bindings (today's realtime settings: deadline realtime, VP9 cpu-used 7, row-mt and tile columns) | yes (`alpha_mode` tag) | Opus at 48 kHz; frame-exact; the export's colour tags |
| PNG sequence / still | folder of `<name>_00000.png` (≥ 5 digits) / file | RGBA8 or RGBA16, straight alpha | `png` crate | yes | `sRGB` chunk plus `cICP` (1-13-0, or 1-1-0 with "Rec.709 image"), full range; verified lossless |

`ExportSpec::input_format()` picks the input from codec and alpha.
Conversion (`convert.rs`) stays the f64 reference: Rec.709 matrix, siting
type 0, 4:2:0 vertical filter, the deterministic blue-noise dither (±0.5 LSB,
8-bit PNG by default), round half to even, clipping to the codable range.
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

### Beta decision OFFLINE-1 (2026-10-21)

An interactive export in either engine asks before writing when a referenced
clip is offline: Locate Media…, Export Without Missing Media, or Cancel.
The native sheet also offers Export with Proxies when all offline clips have
proxies. Export Without Missing Media keeps the layer and asset records in the
snapshot, with their missing picture omitted and missing audio silent. This
preserves parenting and matte references. The job engine renders it; it does not route the export to the
legacy renderer. Agent and CLI exports continue unattended with missing
layers omitted and include the missing asset ids and names in their results.
Queued user jobs continue waiting for relinked media. The phase 3b routing
description below records the earlier implementation and is superseded for
offline media by this decision.

Media that is still loading is not offline. With the setting off (and when an
engine export comes back to the current exporter), an export waits for the
project's media restore and for referenced media that is still loading, 30 s
in all. It shows the export sheet ("Waiting for media…") with Cancel while it
waits, and then goes on as before: it is never refused for it. Only clips the
finished restore found missing are asked about; a restore still running at the
deadline asks nothing, as today. Beta-list bug fix: media that finishes during
the wait is in the export (today an export started at once and rendered it
absent).

### EXPORT-1, retired (X-A2)

EXPORT-1 (2026-10-21) asked every export to decode byte for byte to the prior
exporter's pictures through today's encoders. Under MISSION and QUALITY-1/2 it is
replaced by G-X1 (gate-policy.md): an export keeps today's stream properties (codec,
pixel format, size, frame rate and count, audio family, rate, channels and length) and
its decoded pictures are at least as close to the reference as today's
(`e2e/export-encoding-parity.spec.ts`). MOV/MP4 video time bases follow the
[bundled FFmpeg 9.0.2 MOV/MP4 muxer's rule](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavformat/movenc.c)
(`timescale.rs`), and the movie timescale is the video's.

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

## Gaps (Phase 3b)

- **4K ProRes 422 HQ export at real time on base**: not met on this Mac
  under shared load (20.9 fps with three layers, 23.976 needed), and no base
  machine was measured. Rendering and the f32 readback dominate; the GPU
  encoder-input pass (convert on the GPU, read back `v216`, a quarter of the
  bytes) is designed but not built: exports convert on the CPU.
- **At least today's baseline, every format**: pending the quiet window
  (`e2e/export-baseline.spec.ts`, `export-encoding-parity.spec.ts`).
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


## Encoders (X-R2 to X-R5)

Every format takes the engine's own conversion of the working frames
(`convert.rs`: Rec.709 matrix, siting type 0, the 4:2:0 vertical filter, round half
to even, the codable range; blue-noise dither only for 8-bit PNG, X-K6 as amended by
EXPORT-DITHER-1) and the f32 mix:

- **H.264 and ProRes** (`mac::VideoToolboxSink`): VideoToolbox from `420v` (undithered),
  `v216` or `y416` (ProRes at its native 10 or 12 bits from the float frame), muxed by
  AVAssetWriter; MOV sound tracks get a `chan` layout box after AVAssetWriter finishes. Today's "over black, no dither" RGBA8 pipe to an
  FFmpeg process (swscale) is gone, and so is the FFmpeg audio remux: PCM is written as
  24-bit (or 16-bit with TPDF dither, or float) `lpcm` from the f32 mix, AAC by
  AudioToolbox from f32 with an edit list, and MP4 starts with its movie header.
- **WebM** (`webm::WebmSink`): libvpx and libopus in the engine with today's realtime
  encoder settings and keyframe schedule, on its own thread one frame behind the
  conversion (`pipeline::Overlapped`; thread count, tile columns and frame-parallel
  changed nothing measurable), tagged with the export's colour description
  (1-13-1 limited range). OpusHead carries libopus's 312-sample pre-skip with packets
  from zero and no CodecDelay, so audio starts with the first frame. The hidden Chromium
  encoder window, its preload and page, the GPU frame transport and the `hostEncode`
  protocol are deleted.
- **Samples**: integer PCM decodes with one scale on both sides of zero (1/32768,
  AES17), not Chromium's 1/32767 above zero (`pm_media::audio`).
- A frame today's mux drops (COMPCLOSE-1) is moot: every frame is written.

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

## User-export scheduling (EXPORTQOS-1)

EXPORTQOS-1 changes only user-started exports (sheet or a started render
queue): the render lane and encoder/conversion workers use Darwin default
QoS while playback is idle, utility during playback, then default again
when it stops. Per-frame/band/row refresh and host-reply wait refresh carry
transitions into active work. The lane returns to utility on exit. Agent,
snapshot, generation and background work stay utility. Chromium and external
encoder processes retain their own thread classes. Contract 13 invariant 10
and `host-processes.md` describe the scope.

The job process's JavaScript thread, which pumps requests and answers,
also follows playback while a user export is running and retains its
class between native calls. Otherwise it returns to utility. Its shared
transport can carry agent messages during a user export; dedicated agent
workers remain utility.

Exports construct `Mixer::new_legacy_export`: today's offline audio walk
ignores clip speed and sourceTime, and omits remapped precompositions or
embedded soundtracks. Ordinary playback keeps the engine's retiming.
The export-only sample-bit regression compares nested speed 1 and 1.2,
including trims, fades and gain automation, while checking that playback
at speed 1.2 still differs. Only an explicit legacy remap omission becomes
silent; unrelated shape or source failures keep their existing behavior.
