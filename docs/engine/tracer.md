# The Phase 2 tracer bullet

Design notes for the end-to-end path behind "Use New Engine" (plan section
26, Phase 2; sections 6, 15, 18, 19, 21; amendments 1 to 20). With the
flag on, the viewer shows frames the interactive engine draws from the
same canonical document pm-wasm mirrors, the transport runs on the
engine's clock (the audio device's), edits reach the engine through the
patch stream, compositions the engine cannot draw stay with the legacy
renderer, and a crashed engine or a lost GPU recovers and resyncs. With
the flag off nothing here runs.

Builds on: [bridge.md](bridge.md) (the frame bridge and the viewer's
presentation), [host-processes.md](host-processes.md) (processes, ports,
crash handling, resync), [pm-render.md](pm-render.md) (what the renderer
draws, capability routing), [pm-audio.md](pm-audio.md) (playback and its
clock), [write-path.md](write-path.md) and the pm-wasm notes (the barrier
and the patch stream), RFCs 02, 05, 06, 07, 14, 15.

## The path of an edit

```text
inspector / canvas gesture / Undo / agent
  └─ PM.Edit / PM.hist / PM.touch ── write barrier (engine-doc/barrier.ts)
        └─ flush (end of the task; the connection asks for it)
              ├─ pm-wasm  (applies; synchronous queries, hit-testing)
              └─ patch batch {baseRevision, revision, canonicalV1 body}
                    └─ DocumentSession (engine-connection/session.ts)
                          └─ applyPatches on the window's engine port
                                └─ pm-engine document host (mirror, consistency)
                                      └─ DocumentUpdate → presenter thread
                                            ├─ pm-render frame (revision-tagged)
                                            └─ pm-audio mirror (retune glide)
```

- **One stream.** The engine follows exactly the batches pm-wasm applied
  (`PM.EngineDoc.subscribe`), in order, with the same `canonicalV1` bodies
  (contract 5). A session opens with a resync of the window's canonical
  document (inline, or staged as a cache file above 12 MiB), then
  forwards each batch whose `baseRevision` is the revision it holds. A gap,
  a `load` batch (another project), an oversized body, or an engine
  `staleRevision`/`afterResync` failure makes it resync.
- **Promptness.** The barrier flushes before queries, and nothing else used
  to flush it: an inspector edit with no query after it reached the engine
  at the next overlay draw. The barrier now tells listeners when a write
  leaves something to flush (`onPending`), and while an engine session is
  open the connection flushes once at the end of that task (a microtask),
  so a drag's move leaves the renderer before the event handler's task
  ends and before the next animation frame. Inside an open plain history
  transaction (a precomp or group drag), writes reach the barrier only as
  the speculative diff each flush marks first; those marks are not
  announced, or every flush would ask for the next one (this hung the
  renderer on the first move of a precomp drag until fixed), so such
  drags reach the engine with the flushes their own queries make (the
  overlay draws once per frame).
- **Engine facts.** The engine commits nothing to the document; probe
  results, routing, and media resolution stay engine-side (section 6
  table).

## The kernel's engine connection

`apps/desktop/src/renderer/src/engine-connection/` (installed by
`legacy/bootstrap.ts` after the transport, only when main says the new
engine runs: `nativeEngine.enabledSync()`):

- `connection.ts`: the renderer end of a window's engine port (hello as
  `renderer`, request ids, per-session listeners, close).
- `install.ts`: registers `engine.viewer` (the viewer's
  `EngineViewerService`) and dispatches `ENGINE_VIEWER_CHANGED`; opens a
  `DocumentSession` per viewer presentation; takes a new port per engine
  launch (`powermove:engine-port` window messages, asked again with
  `requestPorts` once it listens, since ports posted before the kernel
  booted were dropped); measures the page ↔ host clock offset
  (`host-clock.ts`, the round trip with the least delay); flushes the
  barrier promptly; exposes `PM.EngineConnection` (status, the session,
  trace and clock hooks for the measurements). Performance marks:
  `powermove:engine-connected`, `powermove:engine-first-frame`.
- `session.ts`: the document session (resync, batches, route, clock,
  failures).
- `transport.ts`: the transport driver (below).

Engine ports are posted to the window's main frame only, never to
extension sandbox frames (section 19).

## Playback and transport

The legacy transport (`legacy/core/engine.ts`) asks the driver
(`PM.setTransportDriver`) whether the engine runs the transport: it does
while the session's composition routes to the engine and a frame has
arrived. Then:

- **Play, pause, seek, loop, frame step, scrub.** `PM.play` sends
  `seek {exact}` + `play {1/1}`; `PM.pause` sends `pause`; `PM.setTime`
  sends `seek` (`scrub` while paused, `exact` while playing); the work-area
  loop goes as `setLoop` whenever it changes; `PM.step` is a seek to the
  stepped frame. The legacy compositor, Web Audio, and video elements
  rest.
- **The playhead follows the engine's clock.** Each animation frame the
  playhead is the clock line at that instant (contract 6 math, through the
  measured host offset); a run ends when a stopped sample of the run
  arrives; without the loop a run stops at the work-area end.
- **The audio device clock is the master** (section 15). The presenter
  sends Play to the view's audio thread (pm-audio `Playback`) and starts
  its line with the audio's first playing sample; samples of the same
  epoch re-anchor it every 100 ms. With no output device, or after 1.5 s
  without a playing sample, the line runs on the host clock (counted in
  `hostClockRuns`). The thread opens the device ahead of the first Play
  once the composition has something to hear and the controls rest for
  100 ms (today's `requestContextWarmup`), so that Play does not wait for
  the device to open (about 150 ms on the reference machine). A
  composition with nothing to hear (`pm_audio::graph::audible`) plays on
  the host clock from Play, as today's playhead does, after the line's
  50 ms frame pre-roll (counted in `silentRuns`); see `gate-fixes.md`, D2.
- **A run starts where Play was pressed.** The audio's first playing
  sample names the instant its first buffer is heard, output latency
  included (160 ms on the Bluetooth output measured in pm-audio.md). The
  presenter used to republish the line at the moment it published, which
  extrapolated back to media the run never plays: the playhead jumped back
  by the output latency at every Play. A run's first sample is now
  published at its start (a host time still ahead), the renderer holds the
  playhead until then, and the viewer's presenter does not use such a
  sample as a clock-offset bound (`clock.rs::run_starts_at`).
- **Frames the engine cannot show in time are skipped.** The presenter
  renders four frames ahead. When a composition costs more than a frame
  to draw, rendering every frame in order delivers each one after its
  interval ended and the viewer drops them all (1,774 of 1,793 at 1,000
  layers). The schedule keeps a running render cost and skips to the next
  frame that can still reach the display (render cost plus three
  refreshes of delivery), so playback stays on the clock at a lower rate
  (384 of 1,793 skipped in the same run; `schedule.rs`).
- **Handover.** When the composition routes to legacy (or the engine
  restarts) mid-run, the legacy transport continues from the same time,
  and the other way round.
- **A/V sync.** Each timed frame the viewer presents now carries
  `displayUs`, the display instant it was picked for; the measurements
  compare the audio clock's line at that instant with the frame's
  interval.

Not in Phase 2 (contract 6, pm-audio.md): J/K/L at rates other than 1×
(refused by pm-audio until Phase 3b), play around the current frame, audio
scrub grains from the viewer.

## Capability routing (section 21)

The presenter keeps its own mirror of the session's document and decides,
per (revision, composition, media answers), whether the engine draws the
composition the viewer shows (`present/document.rs`):
`pm_render::unsupported_with_media` lists everything the renderer does not
draw yet (effects, masks, mattes, transitions, text, shapes, blend modes,
motion blur, 2.5D, compat 2, non-default color, unreadable media, ...).
A composition with any of them routes to legacy: the session receives an
`unsupported` failure with one sentence per reason; the viewer hides the
engine canvas and the legacy compositor, transport, and audio take over,
with no error shown. An edit that removes the last reason brings engine
frames back at the next revision. Routing is per composition, so a project
can switch as the user enters precomps.

## Overlays and the view override (section 18)

- The viewer sends its size, DPR, zoom, pan, preview resolution, and the
  3D view settings (`ViewerState`, bridge.md); the engine places the
  composition where the legacy viewer puts its canvas (snapped to device
  pixels), and frames carry overlay geometry: the composition rectangle
  and each top-level layer's drawn quad.
- **Handles never lead the pixels.** While the engine presents, the viewer
  keeps the selection geometry it drew for each document revision and
  draws the geometry of the newest revision the frame on screen includes
  (`engine-present/revision-geometry.ts`; `V.overlayRevision`). During a
  drag the box, handles, and anchor follow the pixels; hit-testing and the
  drag itself use the current geometry. Multi-selection outlines and hover
  outlines are still drawn at the current revision (a gap below).
- Engine frames are premultiplied and transparent around the
  composition, so the viewer's stage shows around it as today.

## Crash and GPU loss

- **Interactive engine crash.** Main restarts it with backoff
  (host-processes.md); the connection closes, sessions end, the legacy
  viewer shows meanwhile, and the next launch's port opens new sessions
  that resync the whole document. Edits made meanwhile are in the resync.
- **GPU device loss.** The presenter's device-lost callback (or
  `simulateGpuLoss`, main → control port) makes the render thread drop the
  device, report `deviceLost` to each view, retire every pool, and make
  them again on a new device as new generations; the viewer imports the
  new surfaces and the views redraw at the current revision.
- **The presentation worker.** A renderer abort found while measuring:
  Electron runs a shared texture's release callback (a function of the
  preload's realm) from the GPU sync-token signal of the thread that drops
  the last reference to the `VideoFrame`. Transferred to the presentation
  worker and closed there, the callback ran on the worker's thread and V8
  aborted the renderer ("Cannot create a handle without a HandleScope",
  symbolicated from Electron 44's breakpad symbols; 3 aborts in about 15
  playback runs). The page now transfers a clone and closes its own
  reference after the worker hands the frame back, so the last reference
  always drops on the page's thread (0 aborts in 18 runs since).

## Switching engines safely (section 21)

- "Use New Engine" is Settings › General › Engine (a switch; the copy says
  it applies at the next launch, and says "Restart Powermove to switch
  engines." while the running engine differs from the choice). Main reads
  the store key `useNewEngine` once at launch; `POWERMOVE_NEW_ENGINE`
  overrides it.
- With the flag on, the document mirror (pm-wasm) is on at least in its
  plain mode (`withNewEngine`), since the engine follows its stream.
- The round trip on → off → on keeps the stored project identical (sorted
  keys, open/save timestamps aside) and shows engine frames again
  (`e2e/native-engine-tracer.spec.ts`); the media store side of the gate
  is the media-store lane's (`e2e/media-store.spec.ts`).

## Tests

- Rust: `pm-engine` document host (`tests/document_host.rs`), `pm-node`
  document and compose (`tests/document.rs`, `tests/compose.rs`, the GPU
  loss and routing child-process test), the presenter clock
  (`present/clock.rs` unit tests, including the run-start sample), the
  schedule's deadline-aware skips (`present/schedule.rs`);
  `examples/compose_bench.rs` (the composition cost of a document file).
- vitest: `engine-connection/engine-connection.test.ts` (handshake,
  resync, batches, gaps, routing, restart, transport, host clock, the
  run-start hold), `engine-present/*.test.ts` (presenter display instants,
  the worker's VideoFrame references, revision geometry),
  `engine-doc/engine-doc.test.ts` (pending notification; no announcement
  of a transaction's speculative marks),
  `main/engine/native-engine.test.ts` (status report).
- e2e, serially under the Electron lock:
  `e2e/native-engine-tracer.spec.ts` (frames and overlay geometry with
  window pixels; inspector edit and canvas drag through the stream without
  manual flushing, with handles never ahead of the frame; a precomp drag
  in a 200-layer project of slice features; Undo; routing to
  legacy and back; playback rate, run-start hold, A/V sync, pause, steps,
  end of run, loop; ProRes and long-GOP H.264 clips: scrub, colors, and
  playback with their audio on the device clock; engine kill and GPU loss
  with resync; the engine-switch round trip; the Settings switch).
- Measurements: `e2e/native-engine-tracer-measure.spec.ts` (opt in with
  `PM_TRACER_MEASURE=1`; page timing) and the `tracer-*` runs of
  `scripts/frame-bridge-measure.mjs` (viz presentation timing; summaries in
  `docs/engine/tracer-measurements/`). The synthetic projects come from
  `src/main/engine/integration/synthetic-project.ts`; the clips are
  generated outside the repository:

  ```sh
  M=~/Library/Caches/powermove-engine/tracer-media
  ffmpeg -f lavfi -i testsrc2=s=1920x1080:r=30000/1001:d=20 -f lavfi -i sine=frequency=1000:sample_rate=48000:duration=20 \
    -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le -c:a pcm_s16le $M/prores422hq-1080p-2997.mov
  ffmpeg -f lavfi -i testsrc2=s=1920x1080:r=30000/1001:d=30 -f lavfi -i sine=frequency=440:sample_rate=48000:duration=30 \
    -c:v libx264 -profile:v high -g 250 -keyint_min 250 -sc_threshold 0 -bf 3 -b:v 20M -pix_fmt yuv420p -c:a aac $M/h264-longgop-1080p-2997.mp4
  ffmpeg -f lavfi -i testsrc2=s=3840x2160:r=30000/1001:d=30 \
    -c:v libx264 -preset faster -g 250 -keyint_min 250 -sc_threshold 0 -bf 3 -b:v 50M -pix_fmt yuv420p $M/h264-longgop-2160p-2997.mp4
  ```

## Measurements

Measured 2026-10-15 on Jude's MacBook Pro (Apple M4 Max, 128 GB, macOS
27, built-in 120 Hz display, MacBook Pro Speakers at 15.6 ms output
latency), Electron 44, release addon. The Mac was shared with other
engine lanes building and testing: the 1-minute load average was 5 to 16
during the runs (each summary records it). Not one of the three reference
machines (section 25), so no row is a gate verdict; the numbers are as
measured. Harness rows: `scripts/frame-bridge-measure.mjs` `tracer-*` runs
(compositor presentation times from viz, summaries in
`tracer-measurements/`). Page rows: `e2e/native-engine-tracer-measure.spec.ts`
(the page's clock; seek and edit rows end at the presentation worker's
draw, which reaches the display about two refreshes, 19 ms, later).

| Row (section 25) | Gate | Measured | |
|---|---|---|---|
| Layer drag, synthetic 500-layer project | p95 ≤ 33 ms pointer-to-pixel; amendment 5: no worse than today at p50 and p99 | Idle: engine p50 81.4, p95 113.8, p99 138.6 ms (2,287 of 2,401 events presented); today (WebGL viewer, same project) p50 69.6, p95 86.6, p99 94.7 ms. Under the gate load (no autosave): engine p50 97.5, p95 207.5, p99 241.6 ms; today p50 35.6, p95 94.1, p99 117.6 ms (576 of 2,401 unpresented) | Fails both forms |
| Inspector edit during playback, 1,000 layers | p95 ≤ 16 ms, no dropped frames | Edit applied and sent (page main thread): p50 39, p95 60 ms page timing (the same edit with Use New Engine off: p50 41, p95 82 ms); harness p50 90, p95 191 ms. Edit to presented frame: p50 162, p95 264 ms (harness). Drops: 951 of 1,793 frames with an edit every 250 ms; 384 of 1,793 without edits | Fails |
| Scrub, intra (ProRes 422 HQ 1080p29.97) | p95 ≤ 50 ms | Random seek to draw: p50 10.8, p95 11.6, max 12.4 ms (60 seeks). Playhead drag (a seek every animation frame, 3 s): 301 of 301 frames drawn, request to draw p95 27.3 ms | Passes (page timing) |
| Scrub, long-GOP (H.264 High, GOP 250, 3 B-frames) | p95 ≤ 300 ms | 1080p: p50 38.1, p95 112.7, max 636 ms. 2160p: p50 119.6, p95 349.6, max 413 ms; a playhead drag shows 165 of 272 asked frames | 1080p passes; 2160p fails |
| A/V sync | ±10 ms | Page timing (the audio line at the instant each frame was picked for, against the frame's interval): 0 of 899 frames outside their interval (two 15 s runs at 30 and 29.97 fps, without and with a clip's audio). Viz timing (presentation minus the frame's start on the audio line): 25 fps p50 8.9, p95 15.6, p99 19.0 ms; 23.976 fps p50 9.0, p95 16.1, p99 19.9 ms; 30 fps under load p50 8.1, p95 16.5, p99 24.8 ms | Median within, p95 outside |
| Cadence (120 Hz display; no 60 Hz display here) | Ideal cadence, zero irregular repeats over 60 s | 25 fps: 0 drops, 488 irregular intervals of 1,493 (the ideal 5-5-5-5-4 pattern has 299); 23.976 fps: 0 drops, 253 of 1,432 (ideal about 7); 30 fps under load: 0 drops, 703 of 1,792 (ideal 0) | Fails |
| Cold start | Engine ready before the window finishes loading the last project | 3 launches of a 500-layer project: the engine connection is ready 912 to 921 ms after launch, the project 1,012 to 1,017 ms (about 100 ms after); the first engine frame 1,446 to 1,467 ms (about 440 ms after the project) | Passes for the connection; the first frame comes later |

Where the time goes (profiles in this lane's runs):

- **Drag.** The page's main thread is the bottleneck on the engine path:
  `handledToArrival` (view state and edit out, engine render, descriptor
  back to the page) is 52 ms p50 against 10 ms of compose work. A CPU
  profile of 240 drag moves over the 500-layer project spends 1,250 ms of
  2,756 in pm-wasm queries answered for the UI (`groupAncestors` from
  timeline rows: 576 ms; JSON marshaling `readJson`, `encodeInto`,
  `decode`), leaving 48 ms idle; with Use New Engine off the same drag
  leaves 128 ms idle (the WebGL compositor costs 1,089 ms there). With the
  new engine on, the mirror's query routing is on (`withNewEngine`), so
  every synchronous geometry query in the panels goes through pm-wasm.
- **Composition cost.** `compose_bench` (this lane, release): 500 layers
  7.6 ms CPU p50, 10.1 ms with the GPU; 1,000 layers 18.2 ms CPU, 22.3 ms
  with the GPU, at the viewer's size. 46% of the CPU is pm-eval's channel
  lookups (`Scope::num` → `channel`, B-tree string compares, `memcmp` 31%
  self), 14% is `compositing_layers` scanning every layer for each group's
  members. Routing costs 0.45 ms per revision, a document copy 2.6 ms.
- **Playback at 1,000 layers** stays under 30 fps on this machine, so
  frames are skipped (above). Before the deadline-aware schedule every
  frame was rendered after it could still be shown and 1,774 of 1,793
  were dropped late.


## Changes outside this lane

Kept small; listed for the owning lanes.

- `engine/crates/pm-render`: `Rendered::quads` and `LayerQuad` (each
  top-level layer's drawn quad, the overlay geometry), video probe answers
  cached per file, size, and modification time (routing asks at every
  revision); a headless test for the quads.
- `engine/crates/pm-audio/src/playback.rs`: `ClockHandle`, a read-only view
  of a playback's clock for another thread.
- `engine/crates/pm-gpu/src/bridge_out/display.wgsl`: coverage passes
  through the display transform (premultiplied output), so the stage shows
  around a composition.
- `apps/desktop/src/main/engine/`: `index.ts` (the interactive engine starts
  on the document source; `hostTime` and `requestPorts` IPC; the engines'
  clock through pm-bridge), `native-engine.ts` (`simulateGpuLoss`,
  `report`), `supervisor.ts`, `ports.ts`, `engine-process.ts` (the `status`
  control message and notice); `preload/index.ts` and `shared/engine-ipc.ts`
  (the three IPC calls); `shared/store-keys.ts` (`useNewEngine`).
- `apps/desktop/src/extensions/viewer/`: `viewer.ts` (engine frame tags
  redraw the overlay, the selection drawn at the frame's revision,
  `V.engineFrame`, `V.overlayRevision`), `engine-present/presentation.ts`
  (`revision` on the service), `presenter.ts` (`displayUs`; a run's first
  clock sample is not an offset bound), `worker-presenter.ts` (the page
  keeps the last `VideoFrame` reference), `gpu.ts` and `protocol.ts`
  (premultiplied canvas, routed message), `revision-geometry.ts` (new).
- `apps/desktop/src/renderer/src/`: `legacy/core/engine.ts` (the transport
  driver hand-over), `legacy/bootstrap.ts` (installs the connection, the
  mirror's mode with the new engine), `legacy/app.ts` (the
  `powermove:project-ready` mark), `engine-doc/barrier.ts` and
  `engine-doc/install.ts` (`onPending`), `engine-doc/flags.ts`
  (`withNewEngine`), `settings/SettingsScreen.svelte` (the switch).
- Measurement harness: `scripts/frame-bridge-measure.mjs`,
  `src/main/engine/integration/measure-app-main.ts` and
  `measure-app-page.ts` (the `tracer` mode, synthetic projects, edits
  during playback); `tsconfig.web.json` and `tsconfig.node.json` list
  `synthetic-project.ts`.
- Tests of other lanes adapted to this lane's behavior:
  `engine/crates/pm-node/tests/present.rs` (the first playing sample names
  the run's start), `e2e/native-engine.spec.ts` (the kernel's connection
  negotiates the interactive port itself), `apps/desktop/scripts/engine-writers.json`
  (regenerated: the synthetic project builder writes the document).
- `engine/crates/pm-engine/src/engine.rs`: the integration branch's
  `viewer_transport` (deadline signals for viewer playback) replaced this
  lane's earlier version of the same signals at the merge.
- `docs/engine/host-processes.md`: the Settings row exists now.

## Gaps

- **Phase 2 performance rows that fail as measured** (above): layer drag
  against today, inspector edits during playback with 1,000 layers,
  long-GOP UHD scrub, A/V sync at p95 by viz timing, cadence. The causes
  found are outside this lane: pm-wasm query cost for the UI (Phase 1, S5
  "targets met in the app"), pm-eval channel lookups and group scans
  (pm-eval), one render pass per layer (pm-render), and the page's main
  thread on the frame path (bridge.md: the shared-texture receive). This
  lane's part (prompt flushes, no flush loop, deadline-aware skips, no
  worker aborts) is done.
- **Edits during playback** reach the screen after the frames already
  rendered ahead (render-ahead of 4 frames): frames queued at an older
  revision are shown, not re-rendered.
- **Predicted display time.** The viewer picks frames for rAF plus two
  refreshes (bridge.md); viz shows them about one refresh later at the
  median, which is most of the 8 to 9 ms A/V offset above. Calibrating the
  prediction from presentation feedback belongs with the bridge.
- **Overlays.** Multi-selection and hover outlines, path handles, and the
  text caret are drawn at the current revision; only the single selection
  box, handles, and anchor follow the frame's revision. Frames carry no
  paths or caret yet (`OverlayLayer.paths`, `caret`).
- **Transport.** J/K/L at rates other than 1×, play around, and audio
  scrub from the viewer are not wired (pm-audio refuses other rates until
  Phase 3b). Reverse playback is not available.
- **Cold start.** The first engine frame waits for the media answers of
  every asset (by design, so offline media never flashes), about 440 ms
  after the project finished loading.
- **Reference machines and displays.** Only this Mac (120 Hz built-in
  display) was measured; no 60 Hz display, no base M1 Air.
- **Routing** reports reasons as `info` log lines; the user sees the
  legacy viewer without an explanation (no UI for "drawn by the current
  engine" yet).

