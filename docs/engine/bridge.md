# The production frame bridge

Design notes and measurements for the frame bridge (plan section 18, with
section 26 amendments 4 and 5; contracts 2 and 3). It productionizes spike
S1 (`docs/engine/spikes/S1.md`). Everything runs
behind "Use New Engine".

**Status: flag-gated; the Phase 2 gate's frame-bridge rows are not all
met (2026-10-16, [The Phase 2 gate items](#the-phase-2-gate-items-2026-10-16)).**
H1, H2, and D3 are fixed (and a renderer crash after a GPU process crash
that D3's reproduction found). A/V sync by presentation is within ±10 ms
at p95 at 23.976, 25, and 30 fps idle; under the gate load it fails, the
frames outside being those of live window resizes. Cadence at 120 Hz is
not ideal: the remaining irregular intervals are a race between the
worker's and the page's compositor frames in viz and the WindowServer's
presentation lag, neither of which the worker can see; C′ is the only
measured path to the ideal cadence. Against the real viewer on the perf
project (through the kernel's connection, the production path), playback
is better than today under load and equal idle, the pan is equal idle
and better at p50 under load but worse at p99 under load (102.5 against
74.9 ms); layer drag with the engine drawing the document is worse, set
by the document's render cost and the page's main thread. The bridge
passes its Electron integration test in both presentation modes and the
quality lines. Everything stays behind "Use New Engine": nothing presents
unless the kernel registers `engine.viewer` and the engine has a frame
source. The open options are in
[Decision](#decision-before-the-phase-2-gate).

## Where the code is

| Part | Code |
|---|---|
| Rendezvous, audit token, Team ID, Mach wire format, main's receive loop | `engine/crates/pm-bridge` (`rendezvous`, `audit`, `wire`, `host`, `node`) |
| Main's addon (`pm_bridge.node`), the cdylib over pm-bridge | `engine/crates/pm-bridge-addon` |
| Viewer surfaces, display transform, bridge test source | `engine/crates/pm-gpu/src/bridge_out` |
| Presenter in the interactive engine (pools, render thread, transport clock, descriptors) | `engine/crates/pm-node/src/present`, hook in `engine/crates/pm-engine/src/present.rs` |
| Main: one bridge per engine launch, import once, send once per receiver | `apps/desktop/src/main/engine/bridge.ts`, wired in `main/engine/index.ts` |
| Preload receiver | `apps/desktop/src/preload/frame-bridge.ts`, `shared/frame-bridge.ts` |
| Viewer presentation (jitter buffer, WebGPU painter, view state, overlay tags) | `apps/desktop/src/extensions/viewer/engine-present/`, hooked into `viewer.ts` |
| Presentation worker (the presenter off the page's main thread) | `engine-present/present-worker.ts` (worker side), `worker-presenter.ts` (page side), `present-worker-entry.ts` |
| Engine-path gestures on `pointerrawupdate` | `engine-present/raw-gestures.ts`, used by the viewer's drag helper |
| Vsync-phase hint (contract 2 `ViewerState.vsync`) | viewer: `engine-present/display-timing.ts`, `presenter.ts`; engine: `pm-node/src/present/schedule.rs` (`next_in_phase`) |
| C′ prototype (remote Core Animation layer, measurement builds only) | `engine/crates/pm-gpu/src/bridge_out/remote_layer.rs` and `pm-node/src/present/render.rs` (`remote-layer` feature of both), hosting in `pm-bridge` (`harness` feature), `pm-gpu/examples/refresh_probe.rs` (needs `remote-layer`) |
| Electron integration test (run on every Electron upgrade) | `apps/desktop/scripts/frame-bridge-integration.mjs`, `src/main/engine/integration/bridge-*.ts` |
| S1 measurement of the production path | `apps/desktop/scripts/frame-bridge-measure.mjs`, `src/main/engine/integration/measure-*.ts` (stand-in page), `measure-app-*.ts` (the real app); summaries in `docs/engine/bridge-measurements/` (and `tracer-measurements/` for the document runs) |
| Presentation recovery e2e (GPU process crash, a transfer that lands late) | `apps/desktop/e2e/engine-present-recovery.spec.ts` |

## How a frame gets to the screen

1. **Rendezvous** (once per interactive engine launch). Main registers a
   receive right under a random bootstrap name (`com.powermove.engine-bridge.`
   plus 128 bits) before the fork and passes the name in
   `POWERMOVE_FRAME_BRIDGE`. Right after the spawn it records the child's
   audit token (pid and pidversion), parent, and uid. It accepts exactly one
   check-in, verified against that record and against main's own code
   signature policy (a signed main requires its Team ID through
   `SecCodeCopyGuestWithAttributes` with the audit token; the unsigned
   development bypass exists only when main itself has no Team ID and the
   app is not packaged). It then destroys the name and continues on an
   anonymous port where every message must carry the accepted audit token.
   A restarted engine gets a new name.
2. **Pool.** On the first viewer state the engine sizes a pool at viewer
   device pixels: ring (2) + render-ahead (4) + 2 = 8 surfaces (render-ahead
   is longer in frames at high frame rates: [Pacing at 120 Hz](#pacing-at-120-hz)), `rgbaf16`
   when the viewer's display is EDR and the lease allows it, `bgra` with the
   blue-noise dither otherwise (amendment 4). Each IOSurface goes to main
   once over Mach; main imports it once with `importSharedTexture` and sends
   it once to the window that owns the viewer's port, where the preload
   keeps it for the pool's lifetime. The engine announces the pool to the
   viewer (`Event::SurfacePool`) on the viewer's own port.
3. **Per frame.** After the command buffer completes, the engine sends a
   `FrameDescriptor` straight to the viewer's `MessageChannelMain` port; main
   is never on the per-frame path. The descriptor carries `revision`,
   `viewSeq` (the viewer state it shows), `pts`, `duration` (timed playback
   frames) or `null` (shown at the next refresh), the color space, and the
   overlay geometry at that revision.
4. **Presentation.** The viewer asks the preload for a `VideoFrame` of the
   named surface (one persistent import, a new `VideoFrame` per frame) and
   transfers it at once to its presentation worker, which owns the
   viewer canvas's drawing (`OffscreenCanvas`) and queues the frame in the
   jitter buffer. At each of the worker's animation frames it predicts
   when that refresh reaches the display (1.5 refreshes and 3.2 ms after
   the animation frame for a worker canvas, calibrated on viz's
   presentation feedback; see [A/V sync](#av-sync-by-presentation)), maps
   it to media time on the latest clock sample, and draws the frame whose
   interval contains that instant rounded to the nearest refresh (`importExternalTexture`,
   sampled 1:1 into a WebGPU canvas: `rgba16float` with extended tone
   mapping for `rgbaf16` pools). Frames shown at the next refresh (paused,
   scrub, gesture) are drawn as they arrive. Without a worker (or if it
   cannot draw), the same presenter runs on the page's main thread.
5. **Release.** After submitting the draw the viewer closes the
   `VideoFrame` and returns `{poolIndex, seq}` when
   `queue.onSubmittedWorkDone()` resolves. The engine never writes a slot
   the viewer holds (`PoolLedger`). The per-use release callback is not used
   (S1: it runs before the GPU finished reading).

## Viewer integration

The viewer looks for the `engine.viewer` service (`EngineViewerService` in
`engine-present/presentation.ts`). The kernel's engine connection registers
it only when the new engine is on; without it the viewer is unchanged. With
it, `ViewerPresentation` puts a canvas over the stage under the overlay,
keeps it hidden until the engine presents a frame for the session, and:

- **View override.** Sends `ViewerState` with size in device pixels, DPR,
  zoom, pan, preview resolution, media preference, canvas color space, EDR,
  Play Every Frame, and, in the 3D editor's Editor view, its camera.
  `ViewSequencer` numbers each distinct state; descriptors echo it. The
  camera comes from `engine.viewer`'s `editorCamera()`
  (`core/scene3d/viewport.ts` `viewportEngineCamera`: world-to-camera,
  three.js's zoom folded into the vertical field of view) and is sent again
  at each `scene3d.onViewChange`, which fires for every pose the editor's
  orbit, pan, dolly, and frame set. In the Camera view there is no
  override. The 3D editor has no viewport shading or X-ray since main
  removed the Blender viewport (2b1fa379a), so the override never sets them
  and Blender is retired in the engine too (`pm-render` `blender` module
  notes).
- **Revision tagging and overlay geometry.** Every drawn frame's `revision`,
  `viewSeq`, and overlay geometry go to `onPresented` listeners;
  `overlayInCss` converts the geometry with the DPR of the state that frame
  shows. The viewer exposes `engineFrame()` and redraws its overlay at each
  new tag, so handles drawn from frame geometry never lead the pixels. The
  selection box, handles, anchor, and paths are drawn at the frame on
  screen's time (`overlayTime`) and revision, not the playhead's: during a
  scrub the playhead's frame is still on its way. A paused draw while a new
  frame is on its way emits no `overlay` (the frame's announcement does,
  `PresentedEvents.redraw` answering `coming`), unless none lands within
  500 ms.
  Drawing the existing handles from engine geometry is the Viewer UI's next
  step once `pm-render` draws real layers.
- **Popout following.** The loop runs on the animation frames of whichever
  window hosts the stage, and EDR, color gamut, DPR, and resizes come from
  that window. A popout's rAF timestamps are moved onto the opener's clock
  (`timeOriginShift`): without it the jitter buffer compared times from two
  origins and held every frame (found by the integration test). Same-origin
  popouts draw from the opener's realm, so surfaces keep going to the
  opener's frame.
- **Live resize.** A new size reaches the engine only after it held still
  for 150 ms (`SIZE_SETTLE_MS`); until then frames keep the pool's size and
  the canvas stretches. Every size is a new pool (up to 531 MiB at 4K
  `rgbaf16`), so a live resize must not replace it every step.
- **EDR setting.** `edr: 'off'` presents 8-bit even on an EDR display (a
  future setting; the measurement uses it to compare formats).
- **Attaching.** The services API has no change notification, so the
  engine connection dispatches `ENGINE_VIEWER_CHANGED` on the window right
  after it registers or disposes `engine.viewer`; an open viewer attaches
  (or detaches, when the service went away or was replaced) at once rather
  than at its next layout.
- **Presentation worker.** `viewer.ts` starts `present-worker-entry.ts`
  (bundled by Vite) and the canvas hands its drawing to it. The page keeps
  what needs its main thread: the engine port, the preload's imports,
  parking a descriptor until its import arrives, retiring replaced pools,
  viewer state, and overlays. Every message carries its session, so a
  release or presented frame of an ended session never reaches the next
  one, while frames a closing presenter held still go back to the engine.
  A worker that cannot draw (no WebGPU in workers) hands back the frames
  sent to it and the presentation falls back to the main thread in a new
  canvas without ending the session. A popout gets a new canvas and
  worker, and the session and pool generation carry over. Worker times
  are moved onto the page's clock (the realm that stamps arrivals, also
  for a popout). `EngineViewerService.offMainThread === false` keeps
  presentation on the main thread (the harness's comparison). A frame
  shown on arrival (paused, scrub, gesture) goes to the worker at the
  page's next animation frame and is announced there, before the worker
  draws it, so the overlay drawn for it commits in the same refresh as its
  pixels; timed frames go at once and are announced on the worker's report
  (their overlay lands a refresh or more after the pixels: measured 24 ms
  p50 during playback of a 258-layer project).
- **Gestures on raw updates.** While the engine presents, the viewer's
  drags take their moves from `pointerrawupdate` instead of the
  rAF-aligned `pointermove` (`raw-gestures.ts`), so the engine hears about
  a move before the animation frame that would carry it. A raw update
  only records the pointer; the move runs in an ordinary task right behind
  it (newest wins), never inside the input event, so the engine's frames
  and releases (port messages the page relays) are not held behind a
  gesture's input tasks. The drag helper keeps capture, up, cancel, and
  the cursor; pointer-lock drags keep `pointermove`. The view state
  leaves before `V.layout` writes styles.
- **Vsync hint.** The presenter fits its animation frames to a refresh
  grid, maps it to the host clock, and the presentation re-sends the
  current view with it (`ViewerState.vsync`, same `viewSeq`; contract 2
  version note). The engine starts each timed frame half a refresh after
  a BeginFrame (measured in [step 3](#closing-the-gap-step-by-step-amendment-18)).

## What the Electron integration test found

`bun run test:frame-bridge` (Electron 44.0.0, Chrome 152) passes twice,
with frames drawn on the page's main thread and by the presentation worker
(in worker mode `bridge-worker.ts` reads every drawn frame back inside the
worker), and checks, on the production code: one verified check-in from the spawned pid; each
surface imported once, sent once, received once; 3 s of playback with every
drawn frame's code read back from its pixels (180 of 180, 180 distinct codes
from 8 surfaces, so repeated `getVideoFrame()` calls see new contents);
`capturePage` showing the held frame; a 10-bit ramp (1024 of 1024 codes, max
error 0.40 CV10) and patches 0.25 to 4.0 exact through import and sampling;
view changes tagging frames and moving overlay geometry; a resize replacing
the pool with `allReferencesReleased` once per old surface; a popout and
back; a reload; an engine crash with a new name and check-in; an engine
that exits before it checks in (stopping its launch returns in 0.02 ms and
the receive thread ends 53 ms later, reporting nothing); teardown with
nothing held.

Production fixes that came out of it (the third in worker mode: a
popout's worker took its clock from the popout and dropped every timed
frame as late until it decided on the page's clock):

1. **Reloads leaked every surface of the page's pools.** Electron sends the
   renderer's release notice from a callback that does not run once the page
   unloads (the preload's `pagehide` release is not delivered either), so
   `allReferencesReleased` never fires for a reloaded, navigated, crashed, or
   closed page. Main now marks surfaces sent to a page that went away and,
   once its own import is released, drops its reference after 2 s. Main's
   reference is the only one in main while an import is alive (retain count
   1, measured), and the GPU process holds its own.
2. **Display-p3 canvases banded 10-bit greys.** Apple GPUs truncate on
   render-target writes; after the P3 conversion the display transform lost
   up to 0.97 CV10 (1023 of 1024 codes). It now rounds with
   `quantizeToF16` before the write: 0.47 CV10, 1024 codes (Rust test
   `rgbaf16_on_a_display_p3_canvas_keeps_ten_bit_grey_ramps`).

## Failure handling (review fixes)

- **An engine that exits before its check-in.** The receive thread waits for
  the check-in in 50 ms slices (`STOP_POLL`) and ends when the launch is
  stopped, deregistering the name. `BridgeHost.stop()` never joins the
  thread: main calls it from the child's exit handler on its JavaScript
  thread, and before this fix it blocked there for up to the 15 s check-in
  timeout (addon load failure, an early crash, a quit during startup, a
  respawn). Events the thread still had in flight are ignored. Rust tests
  `a_stop_while_waiting_for_the_check_in_ends_the_loop_at_once` and
  `accept_until_returns_stopped`; the Electron test checks it with the real
  addon.
- **A failed `sendSharedTexture`** (or no window for the client port yet)
  is retried after 16, 100, 500 and 2000 ms while the import is live and its
  page is there; otherwise that slot's import never reached the preload and
  every descriptor for it parked for 1 s and was released undrawn until the
  pool was replaced. Counted as `sendRetries` and `undelivered`.
- **A pool whose hand-off to main failed part-way** (a Mach send timed out)
  is retired, so main lets go of the surfaces it already got, and the render
  thread makes it again as a new generation after 50 ms, 250 ms, 1 s and
  4 s, then at the viewer's next state. The JavaScript side had already
  charged and recorded the spec, so an identical viewer state never asked
  again. Generations are one counter shared by both sides.
- **Ended launches** beyond the newest four that still hold surfaces are
  disposed (main drops its references) rather than left to the garbage
  collector.
- **A surface delivered twice** (phase2-gate.md D3). While the page's
  main thread is held up for over a second, Electron's 1000 ms transfer
  timeout fires on main, the bridge sends the import again, and the first
  transfer still lands: main counts one renderer reference, the preload
  held two, and the second release notice reached main after it had
  dropped the texture ("Shared texture with id … not found" as a page
  error). The receiver keeps the first copy of a texture it already holds
  and lets go of the other without a notice; it sends release notices
  itself (Electron's channel, as Electron's own wrapper does), so a notice
  main can no longer place is counted (`lateReleases`), never thrown.
  See [D3](#d3-a-surface-delivered-twice).
- **A lost WebGPU device** (a GPU process crash). Drawing with the lost
  device in the presentation worker crashed the renderer; the painter now
  reports the loss and refuses to draw, the worker hands its frames back,
  and the presentation draws again on a new canvas and device, keeping the
  session (it gives up after 4 losses in 30 s). See
  [D3](#d3-a-surface-delivered-twice).
- **Packaging.** `native/pm_bridge.node` is in electron-builder's `files`
  and the release script's `--with-engine` check; before, a packaged app
  could not load the bridge, so it presented nothing and the Team ID policy
  could never run in a signed build.
- **Build.** The addon is the cdylib-only `pm-bridge-addon` crate over the
  `pm-bridge` rlib, so the release profile's thin LTO applies; see
  [release-build.md](release-build.md).

## The Phase 2 gate items (2026-10-16)

The frame-bridge items `phase2-gate.md` left open, fixed or measured on
`feat/ne-bridgefix` (the integration branch at `f47b1f8c` plus this lane).
Measured on the same MacBook Pro (M4 Max, built-in 120 Hz XDR panel),
Electron 44.0.0, release addons, every Electron run hidden and unfocused
under the machine lock, display awake and session unlocked before and
after every run. The Mac was shared with other lanes: each summary records
the 1-minute load average (5 to 16 in these runs; one run at 99 is left
out). Summaries: `bridge-measurements/*-bf*.json` (the perf project) and
`tracer-measurements/*-bf*.json` (synthetic projects).

### H1: the engine-host harness reads main's clock

`integration/electron-main.ts` passed libuv's `hostNowUs` (which counts
sleep) as main's host clock while main uses the engines'
`CLOCK_UPTIME_RAW` through pm-bridge. Both engine harnesses now load
pm-bridge and use `engineClock` (`integration/main-clock.ts`), and refuse
to fall back to libuv's clock. Measured after 4 days of uptime with the
Mac having slept: libuv's clock was 28,892.9 s ahead of the engines';
`engine-host-integration.mjs` passes (interactive engine killed and
resynced in 332.9 ms, job engine 328.7 ms), `engine-fonts-integration.mjs`
passes.

### H2: the S1 app harness shares the kernel's connection

The harness's `engine` mode opened a second connection on the window's
interactive port; the engine refused its hello because the kernel's
connection had negotiated the port, and every `app-*-engine-*` run timed
out. Both engine modes now wrap the kernel's `engine.viewer` (asking for
the per-frame trace and the presentation thread) and play through the
kernel's transport driver; `engine` mode keeps the test source over the
perf project, which routes to the current engine. This is the production
path: the kernel's session, its overlay at the frame's revision, and the
transport on the engine's clock are all live, which the old harness
connection left out. The S1 comparison below uses it.

### D3: a surface delivered twice

Reproduced deterministically (`e2e/engine-present-recovery.spec.ts`, the
second test): a renderer main thread busy for 1.8 s while a pool is
handed over makes 7 of 8 transfers time out on main (Electron's 1000 ms),
the bridge sends them again, the first transfers land too, and after the
pool is retired 7 page errors "Shared texture with id … not found".
Electron counts one renderer reference per transfer completed in time,
so the preload held two copies of a texture main counted once; releasing
both sent one notice too many. The receiver now keeps the first copy of a
texture it holds and lets the duplicate go without a notice, and sends
release notices itself with the rejection handled (`lateReleases`). After
the fix: 7 duplicates, 0 late releases, every retired surface reported
`allReferencesReleased` (16 of 16), no page errors.

The same spec found a worse failure behind "a WebGL context loss": after
a GPU process crash, the first engine frame drawn with the presentation's
lost WebGPU device crashed the renderer (SIGSEGV on the presentation
worker's thread, 3 of 3). The painter now reports the loss and refuses to
draw, the worker hands its frames back, and the presentation draws again
on a new canvas, device, and worker, keeping the session. The spec checks
the edited engine frame's pixels after the crash.

Proof: `engine-present-recovery.spec.ts --repeat-each=5` 10 of 10 pass
(twice); `viewer-tools.spec.ts:140` with the flag on `--repeat-each=10`
10 of 10 pass.

### A/V sync by presentation

The analysis now compares each frame's presentation with the audio
clock's line in effect at that instant (the newest sample made before
it), not the run's last sample: the engine re-anchors the line every
100 ms and an output glitch moves it, which one line for the run charged
to every earlier frame (`avSync` in each playback summary).

What the presentation feedback shows (`vizHopsByDepth`, `vizLags`): a
worker canvas's frame is drawn by viz at the BeginFrame of the animation
frame that drew it when its compositor frame arrives before the page's
frame for that BeginFrame, and at the next BeginFrame otherwise; the
panel shows it one refresh after that draw (sometimes two), 3.2 ms into
the refresh (p5 3.21, p95 3.41 ms). The race between the two frame sinks
went the early way for 0 to 40% of frames depending on the run. The
viewer predicted rAF + 2 refreshes and showed the frame whose interval
contained that instant, so a frame drawn at the next BeginFrame showed
3.2 ms to a refresh after its start and one drawn early up to a refresh
before: presentation minus the frame's start p95 10.2 and 11.0 ms at 25
and 23.976 fps idle, 12.2 ms at 30 fps under load (bf2), 10.2 ms on the
perf project idle (bf1).

The worker now predicts 1.5 refreshes plus 3.2 ms after its animation
frame and picks the frame nearest that instant (`nearestRefresh`): either
draw lands within a refresh of the frame's time on the line.

(2026-10-18, `perf.md` "Amendment 22": measured again, draw and presentation
together took two refreshes for nearly every frame idle and more under load,
so the halfway prediction left offsets 3 to 4 ms late at every rate. The worker
predicts 1.75 refreshes since; the table below is the 1.5 history.)

| Run (bf5, bf6) | Load avg | Presented within ±10 ms | \|offset\| p95 (ms) | Outside ±10 ms, by load event |
|---|---|---|---|---|
| 23.976 fps idle | 8.8 | 1433 of 1433 | 6.78 | |
| 25 fps idle | 6.0 | 1488 of 1494 | 7.63 | |
| 30 fps idle | 9.5 | 1794 of 1794 | 5.98 | |
| Perf project, 30 fps, gate load | 12.1 | 1729 of 1793 | 8.89 | resize 47, long task + resize 8, long task 2, none 7 |
| 23.976 fps, gate load | 8.0 | 1333 of 1432 | 11.85 | |
| 25 fps, gate load | 9.2 | 1363 of 1494 | 12.15 | resize 107, long task + resize 10, long task 1, none 13 |
| 30 fps, gate load | 5.9 | 1678 of 1794 | 12.33 | resize 88, long task + resize 18, long task 3, none 7 |

Idle passes ±10 ms at p95 at all three rates. Under the gate load it
passes on the perf project and fails on the synthetic projects, and the
frames outside are the live window resizes (1.2 s every 5 s): outside
resizes 25 fps has p95 7.24 ms (1,040 frames) and 30 fps 4.10 ms; during
them p5 −8.5 / p95 15.5 ms and p5 −7.9 / p95 12.4 ms: the WindowServer
shows frames a refresh early or late while the window resizes, both
ways, so no constant prediction removes it.

### Cadence at 120 Hz

Each irregular interval is attributed to the lag that changed between
its two frames (`vizLags`). Idle, they are the sink race (draw lag) and
the WindowServer's presentation lag; the engine and the presenter are
never late (0 drops in every run but one whose clock line jumped
10.7 s, replaced by bf6; frames arrive 78 to 153 ms before their time):

| Run | Irregular (ideal) | Draw lag changed | Present lag changed | Both | Neither |
|---|---|---|---|---|---|
| 23.976 idle (bf6) | 105 (≈7) | 0 | 99 | 0 | 6 |
| 25 idle (bf5) | 405 (299) | 92 | 33 | 0 | 280 (the 5-5-5-5-4 pattern) |
| 30 idle (bf5) | 162 (0) | 162 | 0 | 0 | 0 |
| Perf project 30 idle (bf1) | 859 (0) | 678 | 135 | 46 | 0 |
| 25 gate load (bf6) | 521 (299) | 64 | 128 | 131 | 198 |
| 30 gate load (bf6) | 416 (0) | 121 | 189 | 96 | 10 |

Today's WebGL viewer on the perf project at the same time: 917 irregular
intervals idle and 664 under load (bf7).

What was tried, in amendment 18's order and after it:

1. `pointerrawupdate` and the vsync-phase hint are in (earlier rounds).
2. **Redrawing the last frame at every refresh** (so the canvas submits a
   frame every BeginFrame): no change in which refresh shows a frame
   (rAF to present p50 19.85 against 19.84 ms, bf2). Removed.
3. **Drawing timed frames outside the animation frame**, at a chosen point
   of the refresh, to fix which side of the race they land on. At 0.15 of
   a refresh, idle drew 88% at the next BeginFrame (363 irregular at
   25 fps against 376 drawn in the rAF) but under load 679 against 514;
   at 0.5 and 0.75, frames drawn mid-refresh made viz's own BeginFrames
   irregular (rAF gap p95 10.0 against 9.0 ms) and the present lag erratic
   (460 of 1,500 frames two refreshes late at 0.75). Removed.
4. **C′** (measured earlier, not adopted): jitter p99 0.21 ms, the ideal
   cadence, but 25 drops a minute in live resizes under load and private
   API (see C′ below). It is the only measured path to the ideal cadence.

The ideal cadence (zero irregular intervals beyond the 25 fps pattern) is
not reached on the Chromium path. The draw lag is decided by which of two
compositor frames, the page's and the worker's, reaches viz first for a
BeginFrame, which neither can see; the present lag by the WindowServer.

### Pointer-to-pixel, the bridge's half

The engine's per-frame trace (render start, GPU done, sent; host µs) now
reaches the app harness, which splits `handledToArrival` into the hop to
the engine (the page's flush, the engine's JavaScript thread, waiting for
the render thread), the render, and the descriptor's way back (the
engine's JavaScript thread, the port, and the page's main thread).

Layer drag on 500 layers, idle (bf8, load 7):

| Hop (ms, p50 / p99) | |
|---|---|
| input → handled | 3.5 / 11.2 |
| handled → render start (page flush, engine thread, render thread busy) | 9.3 / 23.4 |
| render start → GPU done (pm-render) | 14.0 / 18.7 |
| sent → arrival on the page (the way back) | 6.4 / 30.1 |
| viz draw → presented | 18.5 / 19.2 |
| total | 55.3 / 88.4 (today 20.8 / 36.7, same session) |

Renders that take longer than a refresh (14 ms here, 10 ms in the gate's
runs) run back to back while the pointer moves, so a new edit waits for
the current render (about half a render) and the compositor's own GPU
work is pushed a refresh later (viz draw → presented 18.5 against 11 ms
for the pan, whose renders take 1 ms). The way back waits on the page's
main thread: under load in a pan, outside load events 8.6 ms p50, during
live resizes 29.4 ms p50 and 98 ms p99 (bf6).

Two bridge-side changes were tried against this and not kept:

- **Pacing renders of frames shown at the next refresh** (paused,
  gestures) to the vsync phase, once per refresh, so the engine's GPU work
  lands after the compositor's (`POWERMOVE_BRIDGE_PAUSED_PHASE` in an A/B
  build of pm-node). Three pairs of 500-layer drags idle and two under
  load (summaries tagged `bf9phase*` and `bf9base*`):
  where the render cost matched between the two runs of a pair, paced was
  4 ms better at p50 idle once (38.4 against 42.1 ms, render 10 ms) and
  7 ms worse at p50 under load once (75.4 against 68.5 ms, render 16.5 ms);
  the render cost itself moved between 10 and 17 ms from run to run with
  the machine's other work, more than any difference between the arms.
  Not adopted on that evidence; the code was not kept.
- **Taking the descriptor off the page's main thread.** The way back
  needs the page's main thread only to turn the descriptor into a
  `VideoFrame` from the preload's import. A `VideoFrame` made once per
  pool slot and kept would let the descriptor go straight to the worker.
  Probed in the Electron integration test (main-thread mode, not kept):
  of 180 frames, a kept `VideoFrame` of the slot showed the same code as
  a fresh one 5 times; it shows the contents of when it was made. So a
  fresh `VideoFrame` per frame is needed, and it can only come from the
  preload on the page's main thread: the way back stays on the main thread
  until Electron hands shared textures to a worker (option 2 under
  [Decision](#decision-before-the-phase-2-gate)).

What is left for pointer-to-pixel is the render cost of the document
(the perf lane: pm-render compose, pm-eval lookups, the per-layer passes
in `tracer.md`) and the page's main thread, which both the input and the
way back need (today's pan needs it once).

### S1 against the real viewer (perf project, 884 layers)

Paired runs the same night (bf6, bf7, bf8; load averages 5.6 to
12.1). The engine path is the production path now (above), drawing its
test source over the project.

| Line | Today (real WebGL viewer) | Engine path (worker) |
|---|---|---|
| Playback idle: drops, \|jitter\| p99 | 1 of 1,793, 8.35 ms | 0 of 1,793, 8.57 ms |
| Playback under load: drops, \|jitter\| p99 | 56 of 1,793, 58.3 ms | 0 of 1,793, 12.7 ms |
| Pan idle, pointer-to-pixel p50 / p99 | 19.98 / 28.89 ms | 19.41 / 29.19 ms |
| Pan under load, p50 / p99 | 24.89 / 74.90 ms (with dropped first frames 24.92 / 91.08) | 19.21 / 102.53 ms (bf8); 37.02 / 153.93 ms (bf6) |

Playback is better than today's viewer under load and equal idle. The
pan is equal idle and better at p50 under load, but its p99 under load is
worse (102.5 against 74.9 ms, or 91.1 ms with today's dropped-frame
inputs), set by the live resizes holding the page's main thread with a
descriptor waiting in its queue. Layer drag (the gate's row) is worse
because the engine renders the document (above).

### Files outside this lane (2026-10-16)

Kept small, for the owning lanes to review:

- `apps/desktop/src/main/engine/engine-process.ts`: with
  `POWERMOVE_BRIDGE_TRACE=1` (measurements only) the status report carries
  the presenter's per-frame trace (`presentTrace` of pm-node's addon,
  which nothing called before).
- `apps/desktop/scripts/engine-fonts-integration.mjs` and
  `src/main/engine/integration/fonts-main.ts`: the fonts harness had the
  same libuv clock as H1.
- `apps/desktop/src/preload/index.ts`: the receiver sends release notices
  on Electron's channel (`IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN`,
  the channel Electron 44's own wrapper invokes; S1 found it); an Electron
  upgrade must keep it (the integration test's release checks cover it).
- `apps/desktop/src/shared/frame-bridge.ts`: two receiver counters.
- `apps/desktop/e2e/engine-present-recovery.spec.ts`: new.

## The amended S1 gate against the real viewer

Amendment 5 made the gate relative: the engine path is no worse than
today's viewer on the same harness, at p50 and p99, idle and under the
load profile; the quality lines stay absolute. Amendment 18 made "today"
the real WebGL viewer and fixed the order of the work: engine gestures on
`pointerrawupdate`, then a vsync-phase hint, then the C′ prototype for
playback, measuring after each step; if the comparison still fails, the
gate reports it as measured and Jude decides.

**Harness** (`scripts/frame-bridge-measure.mjs`, `app-*` runs;
`measure-app-main.ts`, `measure-app-page.ts`). The real app, built, runs as
a hidden background test launch with an isolated profile; its editor
window is off every screen and never activated (C′ runs: on the primary
display behind the desktop picture, where Core Animation still presents).
It opens a copy of the corpus perf project (the Powermove ad project:
1920 × 1080, 30 fps, 884 layers, 138 MB) through Open Project. The stage
is 1195 × 546 CSS px at DPR 2 on the built-in 120 Hz XDR panel. Electron
44.0.0, Chrome 152.

- **Today:** the WebGL viewer plays the project; every compositor draw is
  recorded with the animation frame it ran in and the content time it
  drew, against the legacy playback line.
- **Engine:** the page registers `engine.viewer` (the kernel's part), so
  the real viewer attaches its presentation over the stage and presents
  the interactive engine's frames from its presentation worker, while the
  legacy transport still drives the timeline, playhead, and panels. The
  engine draws the bridge test source, not the document: `pm-render` does
  not draw real layers yet, so the engine-side render cost is small and
  equal across runs.
- **Load:** S1's gate profile without its main-thread autosave, as the app
  runs since autosave serialization moved to a persistence worker
  (`docs/engine/autosave-off-main.md`): a 50 ms long task every second, a
  1.2 s live resize every 5 s, agent tool traffic 20 times a second.
- **Timing:** Chromium's `PipelineReporter` for the page's own frames; for
  the worker's canvas (which has no `PipelineReporter`), viz's
  `Graphics.Pipeline` (frame received on its sink, shown by the next
  `DrawAndSwap`; in a plain page the two agree to the microsecond).
  Pointer-to-pixel is section 18's definition, from the input event's
  timestamp to the presentation of the first frame whose revision (today)
  or view sequence (engine) includes it. 60 s playback runs, 20 s pointer
  runs at 120 events a second.
- **Gestures:** a middle-button pan of the view is the gesture both paths
  share (the same viewer handler; today moves its canvas with CSS, the
  engine renders the panned view). Today also drags a layer. Layer drag,
  orbit, and walk on the engine path need `pm-render` to draw the
  document.
- **Display:** every run checks that the main display is awake and the
  session unlocked before and after (with the display asleep nothing is
  scanned out and Chromium's presentation times are its own estimates);
  runs that saw otherwise are marked invalid. All r8 and r9 runs were
  valid. Runs r5 to r7 predate the check (`caba37f5`; their summaries
  have no `displayState`), so they were not verified this way.
- **Machine:** shared with up to three other agents' builds; the 1-minute
  load average is in each summary (2.3 to 14.3 across these runs). The
  spread between runs of the same configuration is larger than some
  differences between paths, so every configuration has several runs.

Runs r5 to r9 are the current configuration (presentation worker, raw
gestures, vsync hint); r9 is the branch head after the last merge of
`feat/native-engine` and the review fixes.

**Playback** (1,793 expected content frames a run):

| Path | Load | Runs | Drops | \|jitter\| p50 (ms) | \|jitter\| p99 (ms) | rAF → present p50 (ms) |
|---|---|---|---|---|---|---|
| Today (real WebGL viewer) | idle | r5, r6, r8, r9 | 1, 1, 4, 3 | 0.11, 8.04, 0.28, 4.69 | 8.48, 16.37, 33.51, 12.21 | 27.4, 27.1, 20.0, 19.8 |
| Engine (worker) | idle | r5, r6, r8, r9 | 0, 0, 0, 0 | 0.08, 0.08, 0.11, 3.70 | 8.45, 8.45, 8.35, 8.36 | 18.7, 18.7, 20.3, 19.6 |
| Today | gate load | r5, r6, r7, r8, r9 | 84, 139, 113, 238, 91 | 0.14, 0.10, 0.11, 8.21, 4.52 | 50.04, 33.25, 33.33, 108.25, 53.84 | 25.1, 10.9, 11.0, 20.8, 19.9 |
| Engine (worker) | gate load | r5, r6, r7, r8, r9 | 5, 0, 0, 0, 0 | 0.38, 0.09, 0.09, 0.16, 0.38 | 11.02, 10.88, 10.82, 14.25, 12.08 | 18.6, 18.8, 18.6, 20.0, 19.8 |

Today's drops under load sit inside the live resizes (202 of r8's 238,
22 more in a long task during a resize; 87 of r9's 91). The engine
path's 5 drops (r5) are its only ones in the eleven worker runs (with the
phase A/B below). In r9 both paths' idle jitter p50 rose to 3.7 and 4.7
ms: the engine's rAF → presentation rounded to one refresh for 836 of
1,793 frames and to two for 957 (r8: 59 and 1,665), so cadence
alternated between refreshes in that run; the worker canvas has
no presentation feedback, so the presenter cannot see which it gets.
Engine frames showed 10.9 to 12.4 ms after their time on the transport
clock's line at p50 in r6 and r8, 17.9 ms in r7, and 6.1 (idle) and 7.6
ms (load) in r9 (`dueToPresentMs`; p99 10.8 to 26.4 ms): inside section
15's ±10 ms in r9 only, and not yet calibrated (see gaps).

**Pointer-to-pixel, pan** (2,401 events a run; ms, p50 / p99):

| Path | Load | r5 | r6 | r7 | r8 | r9 | counting inputs whose first frame dropped (r8, r9) |
|---|---|---|---|---|---|---|---|
| Today | idle | | 16.5 / 26.0 | 19.5 / 28.5 | 16.0 / 25.2 | 14.5 / 23.1 | 16.0 / 31.8, 14.5 / 23.1 |
| Engine (worker) | idle | 28.3 / 36.7 | 25.0 / 33.6 | 15.3 / 31.0 | 23.3 / 56.4 | 19.5 / 32.3 | 23.3 / 56.4, 19.5 / 32.3 |
| Today | gate load | 25.0 / 66.7 | 22.3 / 71.7 | 24.1 / 65.4 | 27.9 / 69.9 | 22.7 / 67.5 | 27.9 / 86.4, 25.5 / 84.1 |
| Engine (worker) | gate load | 27.6 / 69.2 | 16.9 / 83.3 | 19.1 / 77.5 | 22.7 / 105.9 | 23.0 / 81.6 | 22.7 / 105.9, 23.0 / 81.6 |

r9 is the current head after the review fixes (raw moves applied at most
once per 4 ms, which the harness's 120 events a second never reach; the
skipped WebGL resize; a failing worker's frames handed back). Today's
layer drag: 20.6 to 25.4 ms p50 and 33.4 to 37.4 ms p99 idle (r6 to r9);
24.8 to 34.2 ms p50 and 74.7 to 91.8 ms p99 under load (r5 to r9). Today
drops the first frame of up to 182 inputs a run, which
`pointerToPixelMs` leaves out; `pointerToPixelWithDropsMs` (added for r8)
times them to the next presented frame. The engine path dropped none in
r6 to r9.

**Where the gesture time goes** (r9, `hopsMs` in each latency summary;
ms, p50 / p99). Every hop is timed per input on one clock: the page's
handling of the event, the descriptor's arrival on the page, the worker's
submit, viz receiving the worker canvas's compositor frame, the start of
viz's `DrawAndSwap` that drew it, and that display frame's presentation.

| Hop | Engine idle | Engine load |
|---|---|---|
| input → handled on the page (`pointerrawupdate`) | 1.6 / 6.0 | 1.0 / 41.0 |
| handled → descriptor on the page (view state out, engine render, descriptor back) | 1.1 / 8.7 | 1.4 / 30.1 |
| arrival → worker submit (import, transfer, draw) | 0.5 / 0.6 | 0.4 / 0.5 |
| submit → viz received | 1.1 / 10.9 | 1.6 / 22.2 |
| received → viz draw starts | 0.1 / 7.8 | 0.1 / 3.9 |
| viz draw → presented | 11.4 / 20.0 | 14.1 / 26.4 |
| total | 19.5 / 32.3 | 23.0 / 81.6 |

Today's pan idle: input → handled 7.1 / 11.8 ms (`pointermove` waits for
the BeginFrame) and that frame's begin → presented 11.6 / 20.5 ms; under
load 9.8 / 47.1 and 19.9 / 45.4 ms. viz's draws run on the 8.3 ms grid
(1,781 of 2,452 draw-start intervals 8 ms in the engine idle trace) and
take about 11 ms to presentation on both paths, so the paths differ
before the draw: the engine's frame reached viz's draw about 8.1 ms
after the input at p50. That figure is the total's p50 minus the p50 of
viz draw → presented (19.53 − 11.37 ms), not a distribution of its own;
the summaries keep percentiles per hop only. The per-hop p50s sum to
4.4 ms, and medians do not add: no single hop holds the time (each is
at most 1.6 ms at p50), and the hops' tails, all on threads that do
other work (the page's main thread twice, the worker's commit, viz),
land on different inputs and add up per input. Rounded to whole refreshes, the engine's idle pan took one
refresh for 0.5% of inputs, two for 73%, three for 21%, and four for 5%.

**Quality lines** (absolute): pass, in the Electron integration test in
both presentation modes (see above).

**Verdict: the amended gate is not met, on pointer-to-pixel.**

- Playback passes on the gate metrics, by a wide margin under load: no
  drops in four of five runs against 84 to 238, jitter p99 10.8 to 14.3
  ms against 33 to 108 ms. Idle the engine path is no worse at p50 or
  p99 in any run. **This is not yet like for like:** the engine draws the
  bridge test source, not the 884-layer document, and today's WebGL
  compositor draws nothing while the engine presents, so the engine-side
  cost of rendering real content is missing from both playback and
  gestures until `pm-render` draws the document.
- Cadence regularity is worse on the engine path idle even though jitter
  passes: r9 idle had 843 irregular frame intervals of 1,792 against
  today's 317 (under load 838 against 975). That is the worker canvas
  alternating between one and two refreshes from rAF to presentation
  (above), which the presenter cannot see without presentation feedback
  in a worker. It is not a gate metric, but an FCP-grade cadence target
  needs it fixed.
- Pointer-to-pixel idle fails: in the runs with both paths (r6 to r9)
  engine p50 is 5.0 to 8.5 ms worse in three (no worse in r7), and p99
  is worse in all four (31 to 56 ms against 23 to 29 ms).
- Pointer-to-pixel under load: p50 is within run-to-run spread (16.9 to
  27.6 ms against 22.3 to 27.9 ms; r9 23.0 against 22.7 ms, or against
  25.5 ms with today's dropped-frame inputs counted); p99 fails as
  `pointerToPixelMs` counts it in all five runs (69 to 106 ms against 65
  to 72 ms) and is
  mixed with today's dropped-frame inputs counted (r8 105.9 against 86.4
  ms, r9 81.6 against 84.1 ms).
- The gesture gap is before viz draws the frame, not after: both paths
  take about 11 ms from viz's draw to the panel idle, and the engine's
  frame reaches that draw about 8.1 ms after the input at p50 (total p50
  minus draw → presented p50; hops above).
- The paired r9 latency runs did not share equal machine load: today's
  idle run started at a 1-minute load average of 14.3 (7.2 at its end),
  the engine's at 6.7 (5.4 at its end). The heavier load fell on today's
  run, so a quiet-machine round may widen the idle gap, not close it.
- Layer drag, orbit, and walk through the engine are not measurable until
  `pm-render` draws the document.

## Closing the gap, step by step (amendment 18)

1. **The real viewer as today.** The first real-viewer runs (r2 to r4,
   engine frames drawn on the page's main thread) showed that the
   stand-in had flattered today: the real viewer dropped 55 to 146 frames
   a minute under load (stand-in: 0 to 9), and its jitter p99 reached 33
   to 133 ms. They also showed main-thread costs on the engine path that
   the stand-in page did not have, now fixed: the viewport overlay
   resizing its WebGL canvas on every draw (6.2 s of main a minute with
   the engine presenting, `db44b09d`), the WebGL canvas resizing during
   live resizes while the engine presents (3.3 s a minute, `821d7f06`), a
   forced layout per gesture step (4 ms a step, `a73eb082`), and a clock
   estimator that took the most stalled sample (110 ms behind, `b4d040ef`).
2. **Engine gestures on `pointerrawupdate`.** In the real app it changed
   the pan by about 1 ms (r2, main-thread presentation: idle 31.7 → 30.7 ms
   p50, under load 34.6 → 34.5 ms p50 and 125.7 → 118.5 ms p99), against
   one to two refreshes in the stand-in. Kept: it never costs, and it
   matters more once main's per-step work is small.
3. **Vsync-phase hint** (`ViewerState.vsync`, contract 2 version note in
   RFC 02). The engine starts timed frames half a refresh after a
   BeginFrame. `POWERMOVE_BRIDGE_PHASE=off` A/B in the real app with the
   worker (r8): idle 0 drops either way, jitter p99 8.35 (on) against 8.54
   ms (off), rAF → present 20.3 against 19.2 ms; under load 0 drops either
   way, jitter p99 14.3 against 16.5 ms, rAF → present 20.0 against 19.6
   ms. On the main thread (r2) it changed neither the drops (105 on; 124
   and 203 off) nor rAF → present (28.3 against 28.3 ms). The extra
   refresh is not GPU contention at viz's deadline; the hint stays (it
   costs nothing and keeps the engine's GPU work off the compositor's).
4. **Presentation worker** (not in amendment 18's list; what closed the
   playback gap). The canvas hands its drawing to an `OffscreenCanvas` in
   a dedicated worker, so main-thread stalls no longer hold frames. Under
   load: drops 105 to 350 (main thread, r2 to r5) → 0 to 5 (worker, r5 to
   r8); jitter p99 25 to 84 → 11 to 14 ms; pan p99 118 to 161 → 69 to
   106 ms.
5. **C′ for playback**: below. Measured, not adopted.
6. **ProMotion** (section 15): below. Not possible from Electron, and not
   reachable on this panel through public API for 25 fps content.

What is left on gestures is structural, as far as these runs show. The
r9 hop split puts it before viz's draw: from viz's draw to the panel
both paths take about 11 ms idle, while the engine's frame needs five
asynchronous hops to reach that draw (8.1 ms p50 after the input, against
about 3 ms from input to today's BeginFrame). Two of them are on the
page's main thread: sending the view state for the input, and turning
the frame's descriptor into a `VideoFrame` (the preload's shared-texture
receiver runs there) before the worker can draw it; today's pan needs
main once. Under load those two hops have the long tails (p99 41 and 30
ms). Moving the receive off main needs Electron to hand shared textures
to a worker, which Electron 44 cannot.

## C′: playback through a remote Core Animation layer (prototype)

**What it is.** With `POWERMOVE_BRIDGE_REMOTE_LAYER=1`, in a pm-node
built with its `remote-layer` feature, the interactive engine makes a `CAContext` on its own WindowServer connection whose layer
is a `CAMetalLayer` (`pm-gpu::bridge_out::remote_layer`). Timed playback
frames, already rendered into a pool surface, are copied into the layer's
next drawable and presented at their display time with
`presentDrawable:atTime:`; Core Animation's `presentedTime` reports when
each reached the panel. The engine announces the context to main over the
verified bridge port (`ID_REMOTE_LAYER`, a `remoteLayer` event), and main
hosts it with a `CALayerHost` over the viewer's stage. Paused, scrub,
gesture, and capture frames stay on the Chromium path. One context lives
as long as the presenter; a new pool reconfigures its layer (size, scale,
format, color space) instead of making a new context, so live resizes
stay hosted. When Core Animation stays more than half a content frame
late for two frames, pacing skips one frame so its queue drains
(`remote_resyncs`).

**Private API.** Three symbols are not public macOS API:
`+[CAContext contextWithCGSConnection:options:]` (engine), `CALayerHost`
(main), and `CGSMainConnectionID` (engine). Assessment:

- They are the mechanism Chromium itself uses on macOS to move its GPU
  process's layers into the browser process (`ui/accelerated_widget_mac`).
  Every Electron window already depends on them, so a macOS release that
  breaks them breaks Electron first: the risk is correlated with the
  platform we already ship on rather than added to it.
- Apple documents none of it and may change it without notice. App Store
  review rejects it; Powermove ships with Developer ID and notarization,
  which do not check for private API.
- The engine looks every private symbol up at run time (`dlsym`,
  `AnyClass::get`) only when a remote layer is made (`f0c4f821`). Since
  it was not adopted, the engine side is compiled only into measurement
  builds: `pm-gpu`'s and `pm-node`'s `remote-layer` feature (`4fd9a56f`),
  which also carries the QuartzCore and CoreGraphics links. The app's
  pm-node addon (`build-engine-addon.mjs`) is built without it; asked for
  C′ anyway, it reports `remoteError` ("built without the C′ prototype")
  and presents through the viewer (pm-node `present` test). Hosting in
  main is compiled only into the harness build of `pm-bridge` (`harness`
  feature); the S1 harness builds pm-node with `remote-layer` for its C′
  runs.
- A missing class or symbol is an error the presenter can answer with the
  bridge, which stays complete for every kind of frame.
- Product costs if it were adopted: during playback the frames are
  outside Chromium's compositor, so `capturePage` does not see them
  (section 18 item 8 would hold only for paused frames), and the hosted
  layer sits above the page, so the DOM overlay (handles, guides) would
  need its own layer above it or would have to hide during playback.
  Neither is built.

**Measured in the real app** (30 fps project, 120 Hz panel, window behind
the desktop picture, display awake):

| Run | Load | Drops | \|jitter\| p50 / p99 (ms) | due → present p50 / p99 (ms) |
|---|---|---|---|---|
| r2 (a new context per pool) | idle | 2 | 0.14 / 0.28 | 11.99 / 12.25 |
| r2 (a new context per pool) | gate load | 104 | 0.20 / 3.89 | 12.94 / 13.20 |
| r8 (one context, reconfigured) | idle | 0 | 0.11 / 0.21 | 17.92 / 18.12 |
| r8 (one context, reconfigured) | gate load | 25 | 0.10 / 0.21 | 11.62 / 11.82 |
| r10 (the `remote-layer` feature build) | idle | 1 | 0.12 / 0.24 | 14.87 / 15.10 |

r10 checks the measurement build after C′ moved behind the
`remote-layer` feature (one idle run, display awake, 1-minute load
average 4.4 to 6.0): it still presents through the remote layer with
the same jitter. Its one drop (content frame 384, not presented, outside
any resize or long task since the run was idle) is the first idle drop
of a one-context C′ run; one run does not say whether it recurs.

Cadence is what C′ was for: jitter p99 0.21 ms against the Chromium
path's 8.4 to 14.3 ms, and a constant presentation offset (p99 − p50
0.2 ms) that asking earlier would remove. But the zero-drop line fails
under load. All 25 r8 drops were committed about 60 ms before their
time (the engine was not late): 18 fell inside the 1.2 s live resizes
(300 to 650 ms into them, while the window itself was resizing), 2 in a
long task during a resize, and 5 within 100 ms after a resize ended,
when the new pool reconfigured the layer. The Chromium path presented
every frame through the same resizes (r6 to r8).

**Not adopted** (amendment 5: only if it meets the original cadence line,
p99 jitter ≤ 4 ms *and* zero drops over 60 s under the load). The drops
after a resize could go with a second `CAMetalLayer` in the same context
for the new size, shown once its first drawable presents; the drops
while the window resizes are the WindowServer's and were not explained.

## ProMotion (section 15)

Section 15 asks the viewer to request a refresh rate that is a multiple of
the content rate on ProMotion displays.

- **Electron has no such request** for its compositor:
  `webContents.setFrameRate` applies to offscreen rendering only, and
  Chromium picks the display link rate itself.
- **Through public API the request cannot express it on this panel.** A
  display link with an exact `preferredFrameRateRange`
  (`pm-gpu/examples/refresh_probe.rs`, `RatePreference`), measured
  2026-10-14 on the built-in XDR panel: requests for 100 and 96 Hz gave
  8.33 ms callbacks (120 Hz), 75, 72, 60, 50, and 48 Hz gave 16.67 ms
  (60 Hz). Every request snaps to a divisor of 120 Hz, and no divisor of
  120 is a multiple of 25 fps.
- **Main asking while Chromium plays 25 fps** (stand-in, r8): without a
  request, 0 drops, jitter p99 4.78 ms, interval error p50 1.67 ms (each
  40 ms frame shows for 41.7 or 33.3 ms); with main's display link
  asking for 100 Hz, its 7,199 callbacks were all 8.3 ms apart and the
  cadence was unchanged (0 drops, jitter p99 6.75 ms, interval error p50
  1.67 ms). C′ asking for 100 Hz from the engine likewise saw only 8.3 ms
  refreshes (`playback-cprime-25-rate-idle`).
- 24, 30, and 60 fps content already divides 120 Hz; 25 and 50 fps (and
  the 1.001 rates, by drift) play with a 4-5 refresh cadence. A fixed 50
  or 100 Hz display mode would fix 25 fps, but switching the display mode
  is system-wide and visible, so the viewer does not do it.

## Decision before the Phase 2 gate

The gate reports, as measured against the real viewer after all three of
amendment 18's steps (plus the presentation worker): playback better than
today's viewer on the gate metrics, idle and under load (with the
engine drawing the test source, not the document, and less regular
cadence idle); pointer-to-pixel 5 to 9 ms worse at
p50 idle in three of four paired runs, worse at p99 idle, and worse at
p99 under load as `pointerToPixelMs` counts it; C′ not adopted (25 drops
a minute under load); no ProMotion request possible. Jude
decides. The options as they stand:

1. **Proceed on the Chromium path** with the bridge behind the flag,
   accepting the gesture gap for now. The engine path wins where today's
   viewer is worst (playback under load), and loses up to a refresh on
   gestures.
2. **Take the gesture frame off main** before the gate: it needs a way to
   receive shared textures in a worker (an Electron change), or to keep
   the preload's receive off the page's busy main thread. Expected to
   address the p99 gap; unmeasured.
3. **Finish C′ for playback** (a second layer across pool changes, the
   overlay above the hosted layer, capture through the bridge), with its
   private-API risk as assessed above, and accept or explain the drops
   during live window resizes. It does not change gestures.
4. **Measure gestures again once `pm-render` draws the document**, with
   layer drag, orbit, and walk, before deciding; the engine's render cost
   is the one thing these runs do not include.

**Update (2026-10-16).** Option 4 is measured: with the document drawn,
the 500-layer drag is set by its render (10 to 17 ms on this shared
machine, run to run) and the page's main thread, not by the bridge's
hops ([Pointer-to-pixel](#pointer-to-pixel-the-bridges-half)). Option 2
without an Electron change is ruled out: a `VideoFrame` kept per pool
slot shows the slot as it was when made (175 of 180 frames stale), so a
fresh one per frame must come from the preload on the page's main
thread. The cadence work found no Chromium-path change that reaches the
ideal cadence ([Cadence](#cadence-at-120-hz)); option 3 (C′) remains the
only measured path to it.

## Stand-in measurements (before amendment 18)

Kept for history. These runs compare against the harness's synthetic
stand-in, which amendment 18 replaced with the real viewer for the gate
(above); the engine path here drew on the page's main thread.

**Harness.** `scripts/frame-bridge-measure.mjs`, S1's method on the
production code: 4K (3840 × 2160 at DPR 2) 60 fps content on the built-in
120 Hz XDR display, window shown without activation off every screen, 60 s
playback runs and 20 s pointer runs at 120 events a second. Presentation
times are Chromium's `PipelineReporter` events. "Today" is a renderer-drawn
WebGPU canvas that draws the clock's current frame at rAF (as the WebGL
viewer does) while the engine keeps rendering, so both runs carry the same
engine GPU load.

> **"Today" is a stand-in, not the real viewer.** `TodayViewer` in
> `measure-page.ts` is a synthetic WebGPU canvas, not the WebGL viewer with
> its layer stack, overlay, and inspector. That is S1's control method, and
> it isolates the bridge (same page, same load, same engine GPU work), but
> every "no worse than today" comparison below is against this stand-in. It
> is probably the cheapest possible renderer-side draw, so it flatters
> today's path; a gate decision that hinges on a few milliseconds or a few
> drops should also measure the real viewer. Load is S1's gate profile (`load.cjs`: 7.1 MB autosave
every 2 s, 50 ms long task every second, a 1 s live resize every 5 s) plus
agent traffic 20 times a second. The machine was shared with other agents'
builds (load average 4 to 6 during runs; recorded in each summary). Runs
were made before the f16 rounding fix, which changes no timing.

**Playback** (expected content frames per run: about 3,588):

| Path | Load | Runs | Drops | \|jitter\| p50 (ms) | \|jitter\| p99 (ms) | rAF → present p50 (ms) |
|---|---|---|---|---|---|---|
| Today | idle | 2 | 0, 0 | 4.19, 3.78 | 8.64, 8.75 | 19.8, 19.9 |
| Engine, `rgbaf16` (production on this display) | idle | 2 | 0, 0 | 0.51, 0.14 | 12.32, 8.48 | 19.9, 19.9 |
| Engine, `bgra` | idle | 2 | 0, 0 | 0.12, 0.13 | 8.42, 8.54 | 20.0, 19.9 |
| Today | gate load | 4 | 0, 3, 9, 1 | 0.28, 0.23, 3.99, 4.13 | 8.40, 12.17, 8.51, 8.48 | 11.6 (all) |
| Engine, `rgbaf16` | gate load | 4 | 10, 7, 18, 7 | 0.16, 0.14, 0.20, 0.18 | 8.50, 8.49, 8.52, 12.37 | 19.8 to 20.0 |
| Engine, `bgra` | gate load | 3 | 8, 5, 18 | 0.26, 4.17, 0.29 | 8.45, 8.33, 12.15 | 19.5 to 19.9 |
| Engine, render-ahead 8 (pool 12) | gate load | 1 | 16 | 0.17 | 12.62 | 19.9 |

**Re-run after the review fixes** (tag `r5`, `r6`, `noautosave`; 2026-10-13;
machine load average 3.2 to 7.3, higher than the first runs). None of the
fixes touch the per-frame path; these runs check that and add samples:

| Path | Load | Runs | Drops | \|jitter\| p50 (ms) | \|jitter\| p99 (ms) | rAF → present p50 (ms) |
|---|---|---|---|---|---|---|
| Today | idle | 1 | 0 | 0.13 | 8.51 | 11.6 |
| Engine, `rgbaf16` | idle | 1 | 0 | 0.22 | 12.42 | 19.9 |
| Today | gate load | 2 | 5, 5 | 0.19, 0.23 | 12.65, 16.31 | 11.5, 11.6 |
| Engine, `rgbaf16` | gate load | 2 | 4, 2 | 0.19, 4.38 | 8.48, 8.43 | 19.8, 19.7 |
| Today | gate load without autosave | 2 | 4, 2 | 3.74, 0.22 | 8.55, 12.58 | 11.7, 11.7 |
| Engine, `rgbaf16` | gate load without autosave | 2 | 4, 0 | 8.15, 0.36 | 16.70, 8.57 | 19.9, 19.9 |

"Without autosave" is the gate load minus its main-thread autosave, as the
app runs since autosave serialization moved to a persistence worker
(`docs/engine/autosave-off-main.md`; `loadProfile: 'no-autosave'`).

Across all gate-load runs the engine path dropped 10, 7, 18, 7, 4, 2 frames
a minute (median 7) and today's stand-in 0, 3, 9, 1, 5, 5 (median 4): the
two latest pairs favour the engine, the four earlier ones today. Every
r5/r6/no-autosave engine drop sits in a long task (or autosave plus a long
task). The first no-autosave engine run had the worst jitter recorded on
either path (p50 8.15 ms, p99 16.70 ms): 2,525 of its frames reached the
display two refreshes after their rAF and 68 three, against 589 and 67 for
today in the same profile. The engine idle run again landed in the
two-refresh mode (p99 12.42 ms against 8.51 ms for today).

- Jitter is bistable on both paths, as S1 found (a frame reaches the
  display one or two 120 Hz refreshes after its rAF, and runs switch), so
  p50 lands near 0 or near 4.2 ms depending on the run and p99 near 8.3 or
  12.3 ms. The engine path is no worse at p50. At p99 the pass is
  borderline: the first engine `rgbaf16` idle run reached 12.32 ms against
  today's 8.64 and 8.75 ms (the other engine idle runs are 8.41 to 8.54 ms),
  and under load both paths have runs at 12.2 to 12.4 ms.
- **Drops under load are worse on the engine path**: 7 to 18 a minute
  against 0 to 9. They are Chromium's `STATE_DROPPED` (drawn frames viz did
  not present), plus a few frames the jitter buffer dropped as late right
  after them. Most sit within main-thread stalls (autosave, long task,
  resize; per-frame attribution in each summary's `missing`, recorded from
  the r3 runs on): 55 of the 59 attributed engine drops, while 4 are near
  no stall (3 in `playback-engine-bgra-load-r3`, 1 in
  `playback-engine-load-r4`). Today's stand-in has 3 of its 10 attributed
  drops near no stall. In the run that
  recorded it (`playback-engine-load-r4`), the renderer's draw cost 0.3 ms
  p50 and 4.5 ms at most, frames arrived 60 to 64 ms before their due
  time, and no rAF gap exceeded 12.2 ms, so neither the bridge nor the
  presenter was late; a deeper render-ahead (pool 12) did not help. Idle
  runs drop nothing on either path.
- **The engine path presents one refresh later.** rAF to presentation is
  19.9 ms on the engine path in every run (mostly two refreshes), and
  11.6 ms for today's viewer under load (mostly one). Today's viewer also
  sat at two refreshes in the idle runs, so this is Chromium's compositor
  depth, not a fixed cost of the bridge, but it was consistent for engine
  frames.

**Pointer-to-pixel** (2,401 events per mode):

| Path | Load | Runs | Drag p50 / p99 | Orbit p50 / p99 | Walk p50 / p99 |
|---|---|---|---|---|---|
| Today (renderer draws the marker at the next rAF) | idle | 2 | 25.5 / 26.4; 12.7 / 28.9 | — | — |
| Engine, `pointermove` | idle | 2 | 36.1 / 36.6; 34.5 / 38.8 | 34.3 / 35.3; 33.2 / 34.0 | 33.9 / 34.8; 35.2 / 36.1 |
| Engine, `pointerrawupdate` | idle | 2 | 27.0 / 27.9; 27.6 / 28.5 | 25.6 / 26.6; 24.4 / 25.3 | 23.0 / 31.8; 28.9 / 29.7 |
| Today | gate load | 2 | 18.8 / 111.5; 14.1 / 97.6 | — | — |
| Engine, `pointermove` | gate load | 2 | 36.3 / 119.8; 29.1 / 120.3 | 29.4 / 120.2; 33.7 / 117.1 | 30.5 / 114.2; 29.6 / 113.0 |
| Engine, `pointerrawupdate` | gate load | 2 | 26.5 / 110.1; 27.3 / 110.7 | 29.6 / 104.9; 23.5 / 110.6 | 23.8 / 115.0; 28.7 / 104.2 |

- With `pointermove` (rAF-aligned, what the viewer's drags use today) the
  engine frame misses the rAF that handled its input and costs one to two
  refreshes more than today's path at p50.
- With `pointerrawupdate` the engine path is within today's run-to-run
  spread idle (23 to 29 ms against 12.7 and 25.5 ms) but not below it, and
  4 to 15 ms worse at p50 under load. p99 under load is set by main stalls
  on every path (98 to 120 ms).

**Quality lines** (absolute): pass. `capturePage` shows the held frame's
code and an exact 8-bit ramp; through Electron the `rgbaf16` path keeps all
1024 10-bit codes (max error 0.40 CV10) and patches 0.25 to 4.0 exact; the
`bgra` dither keeps the 10-bit local mean within 0.48 CV10 (Rust test). EDR
headroom on the panel was not re-measured: that needs a window on screen
while Jude uses the machine; S1 measured it (1.0 to 15.7 with the 4.0
patch).

**Verdict then: not met against the stand-in.** The quality lines pass. Playback
jitter is no worse at p50 and borderline at p99 (the engine path sits in the
two-refresh mode more often). Drops under load are within run-to-run spread
in the latest runs but worse over all runs (median 7 against 4 a minute).
Pointer-to-pixel at p50 is worse than today's stand-in idle and under load,
and the engine path presents one refresh later from rAF in every run
(19.7 to 20.0 ms against 11.5 to 11.7 ms under load). The latency runs were
not repeated in this round: nothing in it changes the input or presentation
path.

## Contract changes

Accepted in amendment 17 (`protocol-16c8e6e125019103`). They were not
additive: the four new fields are required, the descriptor gained
validation, the ts-rs types and wire fixtures were regenerated, and
`PROTOCOL_BUILD` moved from `protocol-15e365d7043a8bfe` to
`protocol-16c8e6e125019103`, so every lane that constructs these structs or
speaks the protocol regenerates and rebuilds. This lane also touched code
outside its ownership to carry them: `pm-engine` (`engine.rs`, `present.rs`,
`lease.rs`), `pm-node` (`present/`, `Cargo.toml`), main's
`engine/index.ts`, `engine-process.ts`, `native-engine.ts`, the tsconfigs,
`package.json`, `build-engine-addon.mjs`, `electron-builder.yml`, and
`release-mac.mjs`.

- RFC 02: `FrameDescriptor.viewSeq`, `FrameDescriptor.duration`,
  `ViewerState.viewSeq`, `ViewerState.edr`; per-frame release on
  `onSubmittedWorkDone`; transport on the viewer's port. Sizes (Rust codec,
  M4 Max, release): descriptor with overlay geometry (one layer, one path, a
  caret) 771 bytes, encode 976 ns, decode 4,055 ns; without overlay 266
  bytes, 437 ns, 1,356 ns.
- RFC 03: the bridge-out half lives in `pm-gpu::bridge_out` as standalone
  types (`ExportedSurface::create` takes the contract's `SurfaceRequest`;
  `SurfacePool::descriptor` is the contract's `SurfacePool`). `pm-gpu`'s
  `decode_in` is behind a default `decode-in` feature so the presenter does
  not link FFmpeg until media reaches the engine. One `GpuInterop` impl
  still waits for `encoder_input` and `import_blender_surface`.
- RFC 02 version note (amendment 18, additive): `ViewerState.vsync`, an
  optional `VsyncHint { hostTime, periodUs }` (serde default, TypeScript
  optional and nullable); a peer that does not send it decodes as `None`,
  and a change of the hint alone is not a new view. Additive on the wire
  only: `PROTOCOL_BUILD` still moved (`protocol-16c8e6e125019103` →
  `protocol-4263a0f5f5f748a0`), and draft contracts interoperate only
  between equal build ids, so the handshake refuses a peer built on the
  old id. Every lane regenerates the ts-rs types and rebuilds its addon
  against the new id when this branch merges; the merge needs to be
  coordinated with the lanes merging around it.

### Files outside this lane's ownership (amendment 18 work)

The lane owns `pm-bridge`, `pm-node/src/present`, `pm-gpu/src/bridge_out`,
the preload's `frame-bridge.ts`, the viewer's `engine-present` and engine
hooks, main's `engine/bridge*.ts` and integration harness, the
`frame-bridge-*.mjs` scripts, contract 2's types, and these docs. The
branch also changes the files below. Two of them change the app for every
user, flag or not, and are for their owning lanes to review:

- **`apps/desktop/src/renderer/src/legacy/app.ts`, Open Project**
  (`beeffe15`; moved into `withProjectReader` in
  `renderer/src/legacy/core/project-file.ts` with tests in
  `project-file.test.ts`, `827eb9ed`). Not behind the flag. A failed close
  of main's project reader no longer fails an open: main closes a reader
  left idle for 120 s itself, and opening the 138 MB corpus project on a
  loaded machine read every byte, then spent over 120 s storing media
  before the close, which threw "Unknown project read token" and failed
  the whole open (3 of 11 real-app measurement opens, 2026-10-14). The
  restore's own error is still reported. Owner: project open/save.
- **`apps/desktop/src/renderer/src/core/scene3d/overlay.ts`, viewport
  overlay** (`db44b09d`; tests in `overlay.test.ts`, `7ce605e8`). Not
  behind the flag. `ViewportOverlay.draw` calls `renderer.setSize` only
  when the stage's size in device pixels changed: assigning a canvas size
  reallocates its drawing buffer even when unchanged, a GPU-process round
  trip every overlay frame (2.7 s of the renderer's main thread a minute
  in real-app playback under load on the WebGL path, 6.2 s with the
  engine presenting). Both paths' r5 to r9 measurements include it.
  Owner: the 3D viewport.
- Generated TypeScript for the additive `ViewerState.vsync`:
  `apps/desktop/src/shared/engine-protocol/ViewerState.ts`,
  `VsyncHint.ts`, and `index.ts` (`PROTOCOL_BUILD`), from pm-protocol's
  contract 2 types (`frame.rs`, `samples.rs`) and a regenerated wire
  fixture (`pm-protocol/tests/fixtures/wire/event-viewer.msgpack`).
- `engine/crates/pm-engine/tests/present.rs`: one line (`vsync: None`) so
  the test builds a `ViewerState` with the new field.
- `apps/desktop/tsconfig.node.json` and `tsconfig.web.json`: one entry
  each, so the harness's `integration/bridge-worker.ts` type-checks as
  renderer code.
- `engine/crates/pm-gpu/Cargo.toml` (C′'s dependencies, optional behind
  `remote-layer`) and `pm-gpu/examples/refresh_probe.rs` (the ProMotion
  probe); `engine/crates/pm-node/Cargo.toml` (the `remote-layer` feature)
  and `pm-node/tests/present.rs` (the presenter end to end, and a build
  without C′ asked for it); `engine/Cargo.lock` (C′'s crates).

## The Phase 3c render transport (2026-10-18)

Plan sections 18 to 20, amendments 4, 17, 18, 21 to 24; the contract is
[RFC 16](rfcs/16-render-transport.md), whose TypeScript interfaces are
frozen for the other Phase 3c lanes. Everything runs only with "Use New
Engine" on; with it off nothing here is installed.

| Item | Code | Tests |
|---|---|---|
| Synchronous pixels (`render.snapshot`, `renderFrameTo`, `raster`; apiVersion 1 to 3) | renderer `engine-connection/render-transport.ts` → preload `snapshotSync` (`sendSync`) → main `engine/snapshots.ts` → host `snapshot` request → `pm-node/src/present/snapshot.rs` and `render.rs` (`run_snapshot`, `run_layer_snapshot`) → pixels inline or through `pm-bridge/src/shm.rs` | `pm-node/tests/snapshot.rs` (child engine), `pm-bridge` `shm` unit tests, `main/engine/snapshots.test.ts`, `render-transport.test.ts`, `e2e/engine-render-transport.spec.ts` |
| Render fonts (text and captions in the viewer, snapshots, and rasters) | `pm-node/src/fonts/render.rs` (broker, `BrokerResolver`, `EngineFonts`), `lib.rs` (`serve_fonts` in `pump`/`tick`), `present/render.rs` (`Gpu::composer_with`, font failures), `present/document.rs` (routing with fonts) | `fonts::render` unit tests, `pm-node/tests/render_fonts.rs` (main-thread child), e2e text test |
| Sync snapshots the window's session cannot serve (boot, closed viewer, staged resync in flight) | renderer `windowDocument`, `documentSnapshotSync`; main `engine/window-sessions.ts`; pm-engine host-owned sessions | `window-sessions.test.ts`, `snapshots.test.ts`, `render-transport.test.ts`, `snapshot.rs` `child_host_session`, e2e staged-resync test |
| Layer rasters of every kind in today's surface shape | `pm-render/src/renderer/layer_raster.rs` (`Renderer::layer_raster`), `SnapshotResult.raster`, renderer `engine-connection/raster-surface.ts`, `kernel/install.ts` (engine base beneath wrappers while a public call runs) | `snapshot.rs` `child_snapshots`, `render_fonts.rs`, `raster-surface.test.ts`, `kernel/install.test.ts`, e2e |
| Async siblings (`snapshotAsync`, `renderFrameToAsync`, `rasterAsync`) | `kernel/install.ts` `makeRender`; preload `snapshot` (`invoke`) | the same |
| UI snapshots and the app's captures (composition thumbnails, project thumbnail, the Library's and the agent's captures) | `EngineThumbnails` (idle class, stale-rejecting); `panels/comp-thumbnails.svelte.ts` (encodes only when unblocked), `legacy/app.ts` `projectThumb`, `PM.EngineCaptures` (`engine-connection/app-captures.ts`) asked by `legacy/core/exporter.ts` | `app-captures.test.ts`, `comp-thumbnails.test.ts`, `snapshot.rs` scheduling child, e2e |
| Depth and object-id surface per presented frame | `ViewerState.depthId` → `pm-node` `plan_depth`/`configure_depth`, `Composer::compose_with_depth`; `FrameDescriptor.depthId`; viewer `presenter.ts`/`worker-presenter.ts` (`role: depthId` pools), `ViewerPresentation.depthFrame` | `snapshot.rs` `child_depth`, `presenter.test.ts` |
| Presentation events and stats | `ViewerPresentation` (a shown canvas's first frame counts once it is on screen) → `EngineViewerService.presented` → `engine-connection/presentation-events.ts` (`preview:presented` → `frame:rendered`, then `overlay`); `pm-node/src/present/stats.rs` (`RenderStats`) → `PM.EngineStats` → `runtime/install-legacy.ts` | `stats.rs` unit, `child_scheduling`, `presentation-events.test.ts`, `presentation.test.ts`, e2e |
| Compatibility pixels (upload and export frames) | renderer `uploadCompat`/`provideCompat`; `pm-node/src/present/compat.rs` (store); main `engine/compat-frames.ts` (job engine `compatFrame` host service) | `compat.rs` unit, `child_snapshots`, `compat-frames.test.ts`, `render-transport.test.ts` |
| Crash, GPU loss, cold start with the flag on | `EngineViewerService.exclusive`; `ViewerPresentation` status and no hand-back; `viewer.ts` hides the WebGL canvas; `EngineTransport` `exclusive` (holds and replays the playhead) | `presentation.test.ts`, `presentation-events.test.ts`, e2e crash test |
| `render.gl.init`/`resize` bind presentation to the caller's canvas | `engine-connection/canvas-binding.ts` (`PM.EngineCanvas`), `kernel/install.ts`; viewer stages marked `data-engine-stage` | `canvas-binding.test.ts` |

### How a synchronous snapshot works

1. The renderer flushes pending edits to the engine (`PM.EngineDoc.barrier`).
   If its session has the document and no staged resync is on its way, it
   asks main with `sendSync`, naming the session, its engine launch, and
   the revision. Otherwise (no session yet, a closed viewer, a staged
   resync whose next step would need this blocked thread) it sends the
   document instead: `{ id, revision }`, with the canonical JSON and hash
   unless main already has that revision.
2. Main checks the request and names the window's own port to the
   interactive engine (a window cannot name another's session), or opens
   and prepares its own session for the window (definitions from main's
   relay, the document resynced inline up to 12 MiB or staged, fallback
   pixels posted), waits for a starting engine within the request's wait,
   and sends a host `snapshot` request with the revision as the envelope
   minimum. It sets the sync reply when the engine answers, or after the
   request's wait plus 10 s (main's own preparation counts against it).
3. The engine checks the session belongs to the named port (the window's
   renderer port, or main's host port for main's sessions) and routes it to
   the presenter's render thread. The render thread queues it (sync before
   async before idle), waits for the revision and for the host's media
   answers, renders it between viewer frames with the session's
   definitions and media and the engine's render fonts (`Renderer::render`
   at Float16, today's scale derivation), reads it back, encodes RGBA8 like
   today's capture (round half to even), and answers on main's port: inline
   up to 1 MiB, otherwise a `/pm.<24 hex>` shared-memory segment.
4. Main maps the segment once (`takePixels`, which unlinks it) and hands the
   buffer to the renderer; the facade makes today's shape (a canvas, a JPEG
   data URL at 0.74, a `RasterSurface`).

A frame that needs the renderer's own pixels (a shader the native pipeline
cannot translate, decision 19) is answered `needsRenderer`: the transport
draws those programs on the calling thread
(`ShaderFallbackRunner.renderSync`), posts them on the session's port (or
sends them to main with the next ask, for main's sessions), and asks again
naming them in `uploaded`; the engine waits for those bands instead of
naming them again. Nothing waits for a renderer task, so a call from a
`raster` wrapper or an animation-frame callback cannot deadlock (the e2e
proves both).

### Render fonts

The registry stays on the utility process's main thread (AppKit). The
render thread lays text out with pm-text's level 1 layout over a replay of
the registry's answers and the full font files (`BrokerResolver`). A
lookup the replay cannot answer is asked of the main thread through a
`FontBroker`: the render thread queues it, wakes the host (an empty action
list makes the utility process call `pump`), and waits at most 8 s. The
main thread answers with the registry's answers and the full bytes of
every face they name (shared, not copied). It never waits for the render
thread, and a closing engine releases waiting asks at once. A frame whose
fonts did not answer is not shown (the viewer draws it again; a snapshot
fails with `timeout`), and later asks in that render fail at once. Project
fonts move the broker's generation: the composer installs fresh fonts (its
glyph and raster caches go) and the views draw again. With fonts, routing
uses `unsupported_with_fonts`, so text and captions no longer route the
viewer to the current engine.

### Blender frames in async captures (2026-10-18, second round)

Today's `PM.Export.snapshotAsync` (the agent's `look` and `observe`, and
apiVersion 4's `render.snapshotAsync`) runs `prepareFrame` (Blender renders
the frame at the capture's size with the composition's settings) and then
captures it. With the engine on:

- `SnapshotRequest.blenderWaitMs` (protocol, async and idle only, at most
  900,000 ms): above 0, the render thread plans the frame's Blender
  snapshots exactly as the render plans them (`Renderer::blender_snapshots`,
  final, no preview options), and hands the missing ones to the paused
  viewer's Blender worker (`present/blender.rs`, `PreviewBlender::capture`):
  captures run before previews and never halt them; their surfaces go
  through the same Blender stage, surface cache, and host services'
  supervised Blender as previews and exports. The job waits outside the
  queue (`BlenderPrep::Running`, `present/snapshot.rs`), with Blender's
  time not counted against `waitMs`; the worker's answer
  (`Command::BlenderCaptured`) puts it back with the surfaces, and it
  draws them with `BlenderRender::Final`. An edit while Blender rendered
  re-plans and asks again (at most 3 times). Blender failing, missing, or
  past `blenderWaitMs` (Blender is then stopped) fails the snapshot with
  `blenderFailed`, `blenderMissing`, or `timeout`; never native shading in
  place of a surface, never the current engine.
- `blenderWaitMs` 0 (sync captures): draws a frame an earlier capture
  prepared (its surfaces stay in the worker's map, under today's 512 MB
  budget) or fails with today's sentence, as today's prepared frame is
  drawn by the next capture.
- Main answers such a request within twice `waitMs` plus `blenderWaitMs`
  plus the render allowance (`answerBoundMs`), and refuses a Blender wait
  on a sync request.
- The renderer asks for it only for async captures in documents with a
  Blender composition (`usesBlender`): `PM.EngineCaptures.snapshotAsync`
  (which also keeps today's refusal while an export or another capture
  runs, and pauses and resumes playback around the capture) and
  `render.snapshotAsync`, which goes through `PM.Export.snapshotAsync` like
  the flag-off path. `project.snapshot` stays today's synchronous capture
  shape: an unprepared Blender frame rejects with today's words.

Also in this round: sync snapshots flush pending edits before choosing
between the window's session and main's (a flush that starts a staged
resync no longer waits out the engine); thumbnails require the window's
revision when they asked by default; the built-in text editor measures its
font-anchor offset once per revision and time instead of a synchronous
engine raster per overlay draw and hit test (layers without recorded font
bounds have none and never ask).

Measured: `child_blender` (a stand-in for main's supervision) prepared the
frame in 66 ms and drew the magenta surface; a held Blender failed the
snapshot at 1.52 s against a 1.5 s bound and was cancelled.
`e2e/engine-blender-captures.spec.ts` on Blender 4.5.14 (EEVEE, 192 × 108):
the async capture took 1.15 to 1.49 s through the engine's Blender stage
(four runs) with playback paused and resumed around it; the engine's
pixels equal today's Blender surfaces composited (100% within 2 CV, max
1 CV); `render.snapshotAsync` and the sync capture after it equal it
exactly (mean 0.00 CV); before preparation the sync capture fails with
today's words in both engines. Together with `engine-blender.spec.ts` (2)
and `engine-render-transport.spec.ts` (7): 10 of 10.

### Measured (2026-10-18, M4 Max)

- `pm-node/tests/render_fonts.rs`: a text frame's pixels through the
  broker equal a renderer over the CoreText registry byte for byte (0 of
  230,400 bytes differ), and the layer raster equals `text_raster`'s; the
  first frame took 4 asks; a project font moved the generation 100 → 101
  and the next frame drew with it; a silent font service failed the
  snapshot with `timeout` after 8.0 s.
- `child_scheduling`: a sync snapshot during playback answered in 8.4 to
  9.1 ms (five runs, earlier); an idle snapshot waited until the transport
  stopped.
- `e2e/engine-render-transport.spec.ts` (seven tests): 21 of 21 with
  `--repeat-each=3` after merging the integration branch (load average
  about 5). A sync snapshot during a staged resync of a 13 MiB document
  answered through main's session in 101.8 to 109.4 ms (four runs; the
  5 s sync wait before); a sync snapshot's fallback round drew the
  program on real WebGL with the expected pixels.
- `render.raster` of a 96 px text layer with the engine: p50 2.6 ms, p95
  3.1 ms, max 4.4 ms (30 calls); `render.snapshot(0.5, 480)` p50 3.0 ms.
- `native-engine-tracer.spec.ts:144`: 20 of 20 as the first test of a
  batch (20 separate runs) and 16 of 16 with `--repeat-each=8` (twice).
- The flag-on engine specs together (`engine-render-transport`,
  `native-engine-tracer`, `native-engine-transport`,
  `engine-present-recovery`, `native-engine`, `engine-blender`,
  `engine-export`, `engine-delivery`): 30 of 31; the one failure is the
  J/K/L A/V sync check (2× reverse, a sawtooth of 6 to 11 ms), which
  failed 1 of 6 runs on this branch and 1 of 6 on the integration
  branch's own build (`575a7cd0`) with the same pattern.
- Two regressions found and fixed on the way: waveform requests made
  before the host answered for a just-imported file were told the media
  is offline and never asked again (`native-engine-transport`'s audio
  scrub test failed 3 of 4 runs; 4 of 4 after); and the viewer announced a
  newly shown canvas's first frame before the page showed it, so for a
  display frame the composition area showed the panel's background where
  the box was (`native-engine-tracer.spec.ts:144`'s first-frame pixel read
  245, 245, 247: 1 of 12 first-of-batch probe runs, then 2 of 14 with a
  two-frame wait alone and 2 of 24 with the canvas size as the signal).
  The frame now counts two display frames after the drawing reports its
  GPU work finished: 0 of 25 probe runs (see the lane report for the
  tracer's own first-of-batch runs).

### Round 4: viewerless playback and existing Blender pixels (2026-10-18)

One `DocumentSession` per window now owns transport independently of the
viewer. `ViewerState.headless` retires presentation pools and depth/ID
surfaces while retaining pm-audio, its device clock, levels and periodic
clock messages. Play, seek and captures lazily start this session; readiness
requires the document/media and a revision-tagged clock, rather than a drawn
frame. Paused headless document updates publish a new readiness clock too,
so Play cannot remain held behind an older clock revision. The presenter
also retains the latest Play/play-around request while media is pending,
replays it on readiness, and clears it on Pause; a Play that races an import
is never dropped. Closing a presentation sends headless state; opening it reuses the
session and epoch without Play or seek. Foreground async snapshots have no
headless draw deadline to compete with; idle snapshots still wait for playback.
Removing presentation cancels Blender preview demand while preserving captures
owned by the retained session.
Pending loop and scrub settings replay before a cold session's Play. A load
batch immediately synchronizes the replacement document's requested UI time,
so a queued home-screen seek cannot survive into a shorter new composition. The
legacy canvas stays hidden on viewer removal when the service is exclusive.

The app's stock workspace always retains its viewer slot. The headless e2e
therefore disables the viewer extension, relaunches without it, and enables
it mid-play. It checks advancing time, nonzero audio levels, seek/reverse,
work-area looping, zero headless frames, the same session/epoch after joining,
continuous time, device callbacks, no host-clock fallback, and no page errors.
The Rust document test also proves no pools or draws before joining.

The apparent Blender geometry offset was the old compositor's alpha upload:
`createImageBitmap(..., {premultiplyAlpha: 'none'})` preserves straight RGB,
including RGB under alpha zero, despite WebGL's pixel-store premultiply flag.
Today blends that upload as premultiplied. Correctly premultiplying removed
its bright fringe and visibly changed existing projects. Level 1 now retains
that upload, crop and normalized rectangle plus today's PNG8/dither 1 output;
level 2 retains PNG16/dither 0 and exact premultiplied E. `compatibilityLayers`
in the canonical snapshot and the worker-script hash prevent cache mixing.
Both buffers count against the surface-cache budget. Viewer, capture and
export share this source path. No case or tolerance was removed or widened.

The real-app capture JPEG and still export each match today's corresponding
output at **100% within 2 CV, max 0 CV** (192×108, Blender 4.5.14 EEVEE).
`render.snapshotAsync` and the prepared sync capture also match the async
capture exactly. Raw surfaces are diagnostic references only for level 1.
The engine Blender stage took 1,134 ms in the final parity run.

Additional fixes: an undefined font offset is never cached while fonts load;
Blender preparation gets one total deadline across replans; a missing worker
always answers with an error; and native async Blender captures mirror today's
scene3d busy/error state, including clearing busy on failure. These controls
are covered by focused tests.

The final headless app run recorded 526 device callbacks, peak 0.24994,
zero frames before opening the viewer, and no host-clock fallback. Joining
kept the session and epoch; the largest adjacent UI-time step discrepancy was
37.8 ms (the test bounds it at 80 ms). After closing again, the epoch stayed
22, frames stayed 14, levels advanced 68 → 76, and the playhead advanced
300.2 ms during a 300 ms observation.

Final validation (including a serialized native rerun under the Electron lock):

- pm-node: 87 library tests, 3 document integrations and 7 snapshot
  integrations; ignored child entry points are executed by their parent tests.
- pm-protocol: 91 library, 2 routing and 11 wire tests; pm-blender: 21 tests.
- pm-render: 5 compositor tests and the snapshot corpus comparison;
  pm-export: all 5 Blender export integrations.
- Vitest: 474 tests in the affected directories; the 64 connection tests
  were rerun after the final TypeScript changes. Svelte: 0 errors, 1 existing
  ImageCrop warning. Boundary lint passes. Node tsc passes with composite mode
  disabled; standard tsc still fails the known T0 TS6307 file-list issue.
- App: all 10 existing bridge/Blender tests pass, plus the new headless
  lifecycle test (separate final run). The expanded recovery/tracer/transport
  batch passed 15 of 17. Its path-mask test expects the legacy canvas to show,
  contrary to native exclusivity; path masks still need renderer support. Its
  2× reverse A/V check recorded 14 samples above 10 ms, max 11.8 ms, the
  previously documented sawtooth. Neither gate was relaxed or marked passing.

### Round 5: unsupported compositions, composition switches, the headless gate (2026-10-18)

Review of round 4 found three blockers; all are fixed.

**A composition the engine cannot draw is no longer silent.** Round 4 kept
the current engine's canvas hidden (amendment 21) but still cleared the
status line and dropped Play on the `legacy` route, so a path-mask project
showed an empty stage and Play did nothing. Now the engine runs Play, play
around, paused seeks, scrub grains and audio for that route exactly as for
a drawn one (`takes_transport` in `pm-node/src/present/render.rs`), a
headless view publishes its clock for it, and the route change no longer
stops a run (only `pending` defers and resumes). The failure reads `The new
engine cannot draw this composition yet: <reasons>.`, is repeated when a
viewer attaches to a viewerless session, and the kernel connection replays
it to a viewer that replaces another (`missedRoute`). The exclusive
presentation shows it as the stage's status line (wrapping to three lines so
the named reasons fit), never hands the stage back, and clears it at the
next engine frame. The renderer's `EngineTransport` takes commands for every
decided route. `native-engine-tracer.spec.ts` now asserts this contract
with a project color setting this engine does not know (path masks are drawn
since the compclose merge; a separate case checks that): the legacy canvas
stays hidden, the line names the setting, Play runs on the engine clock
(state `playing`, `PM.time` advancing), the line clears when the setting
goes and returns when it comes back, and no console errors.

**A viewerless session follows the open composition.** Switching
compositions inside a project is a document batch, not a load, so nothing
told the headless session; Play then followed the previous composition's
clock and audio. The kernel connection now sends the headless state on
`comps`/`project` and before every transport command (`prepare`), so Play
right after a switch plays the composition just opened. The headless e2e
creates and opens a silent second composition with the viewer closed: Play
plays it (session composition is the new id, the engine counts one more
silent run, peak 0), and switching back plays the first composition's tone.

**The headless gate.** The review's failing runs had a viewer attached
(`headless:false`, 3 pools) and a run whose clock samples, device callbacks
and levels all stopped while the transport said `playing`. Neither
reproduced here: 3 runs of the review's procedure on the round-4 head,
8 runs of two variants of its scenario (a viewer present at a cold Play with
media still resolving, and a viewer enabled 60 ms after Play), and 4 + 1
runs of the new spec (including after the Blender captures spec, the
review's batch order) passed without a stall. The premise is now checked
(the relaunched app's extension list must have the viewer off, and Play
throws a named error if a viewer is attached), so a setup race fails as
such instead of as a freeze. For the freeze itself the presenter now counts
render-thread turns over 250 ms (`stalls`, `stallUsMax`, `lastStall` in
the status report, and a warning naming the command or the view pass), and
the spec takes a native stack sample of the interactive engine on failure
with `PM_SAMPLE_ON_FAIL=1`. The symptom pattern (levels from the audio
thread stopping as well as the render thread's clock samples) points at the
audio thread rather than the transport; the next occurrence will name it.

**The join.** The join probe now measures two things separately: the
engine's line (how far `transport.now()` strays from wall time across the
mount: at most 0.1 ms) and `PM.time` (how far the playhead strays: the
page's main thread mounting the viewer). The playhead is back on the line
within two animation frames (at most 0.4 ms off). The same probe on the
current engine (flag off) is the baseline, run in the same invocation of
the suite. Playhead error, engine against today, per invocation: 21.6 to
34.1 ms against 22.3 to 23.7 ms (4 runs), 70.3 against 56.3 ms (the loaded
batch run), and 33.2 to 38.2 ms against 27.3 to 44.8 ms (3 runs after the
compclose merge). The engine viewer's mount (its presentation worker and
painter) is at times about 10 to 14 ms longer than today's and at times
shorter; this is recorded, not bounded, and listed as an open difference.
The engine's line itself does not move (at most 0.1 ms), and no
render-thread stall was recorded (`stalls` 0 in every run).

**Paused audio after an undecided route.** `native-engine-transport.spec.ts`
"paused exact and silent scrub seeks prepare audio before Play" failed 1 in 3
to 2 in 4 runs this round, with round 4's TypeScript too (prepared starts
`[0, 384000]`: the work-area edges, not the 0.4 s playhead). A paused exact
seek while the view's route was undecided (right after the test's project
load) moved only the line, and the audio link the session already had kept
its old position. When the route becomes decided, a paused link now seeks
to the line (`document_step`). After the fix: 5 of 5 isolated runs and the
suite run pass.

**Final run (after the compclose merge, one lock).** pm-node document (4)
and snapshot (7) integrations, 87 library tests; e2e
`native-engine-transport`, `native-engine-tracer`, `engine-headless-playback`,
`engine-render-transport`, `engine-present-recovery`: 27 of 27 pass,
including the J/K/L A/V gate this time (it failed in an earlier batch this
round on 2x reverse, 20 samples over 10 ms, the documented sawtooth; audio
and performance owners). Headless: 706 device callbacks, no host-clock run,
no stall, join playhead error 31.5 ms against today's 30.4 ms.

After merging the dochost lane as well (its sync font IPC touches
`preload/index.ts`), with pm-wasm and the addon rebuilt, the same five e2e
specs pass 27 of 27 again.

Smaller items: rustfmt is clean on pm-node; the viewer's version lock entry
matches 1.2.2 (inspector, keymap-default and timeline remain stale on the
base, other lanes).

### Round 6: composition switches with the viewer attached (2026-10-19)

Review of round 5 found one blocker; it is fixed.

**A switch to a drawable composition left the failure on the stage.** With
the viewer attached, opening a composition the engine can draw after one it
cannot (composition A with an effect that has no definition, then an empty
composition B) kept `The new engine cannot draw this composition yet: …`
on the stage, the route `legacy` with A's reason, and the canvas hidden,
until the next edit. The switch reaches the engine as a document batch
(revision N) before the viewer state for B, so the engine re-decides A at N
and reports it, then draws B at N. The session (`legacyRevision`) and the
presentation (`routedRevision`) both dropped frames at or below the
reported revision, so B's frame never showed, and nothing else was dirty.

A session's events arrive in the engine's order on one port, and a
session's frame `seq` only increases (a view lives as long as its session),
so a frame after the report is a later decision at any revision. The
session now drops only frames with `seq` at or below the newest it had when
the report came, and the presentation drops only presented frames with
`seq` at or below the newest its channel had delivered then: frames still
in the presenter's queue. RFC 16 states the contract.

`native-engine-tracer.spec.ts` "switching between a composition the engine
cannot draw and one it can, with the viewer attached" switches A, B, A, B
with `PM.Comps.create(…, { open: true })` and `PM.Comps.open`: on A the
route is `legacy`, the line names the effect, and the canvas is hidden; on
B the route is `engine`, the reason and line are gone, the canvas presents,
and the frame on screen is at the mirror's revision (5, then 7: the
revision the engine reported A at). The engine counted `routedLegacy` 1 to
5 over the switches, the same-revision case of the review. The test fails
on round 5's gating (A/B with the old `session.ts` and `presentation.ts`:
the B expectation times out) and passes with the fix. Unit tests cover the
same A, B, A, B sequence at one revision in the session and the
presentation.

**A viewerless route no longer sticks at `legacy`.** A headless session has
no frames to learn its route from, and its clock carried the document
revision for every decided route, so once reported unsupported it stayed
so (`status().route`, `routeReason`, `engineDrawsViewer`, and the audio
prefs that `audio.sync()` sends on `engine`). The engine now tags a
headless clock with the revision only while it draws the composition, and
the session takes `engine` from any route on a tagged clock at or past the
revision it applied. The native unsupported test checks an untagged clock
for an unsupported headless composition and a tagged one after a resync
makes it drawable; the headless e2e turns the project unsupported (an
unknown project color setting) and back without a viewer and sees the route
and reason follow. A limit of the engine, not of this change: a missing
effect definition is found only by drawing (`decide` skips definition
reasons when definitions are present; the render routes with
`force_legacy`), so a viewerless session of such a composition reports
`engine` until a viewer draws it. Nothing is drawn or heard differently
either way.

**Smaller items.** `takes_transport`'s doc comment is on it again (it had
been inserted between `blender_routed` and its comment). install.ts's
composition follower moved to `composition-follow.ts`
(`followOpenComposition`), and the unit test drives the real follower and
its `comps`/`project` subscriptions instead of a copy. The presenter counts
render-thread turns of at least 50, 100, 250 ms and 1 s (`slowTurns`) with
the longest (`turnUsMax`), so hitches below the 250 ms stall show; the
headless e2e prints them (every run this round: `stalls` 0, `slowTurns` at
most one turn of 50 to 100 ms, the longest 85 ms). The headless spec's
premise check threw `a viewer is attached` when no engine session existed
yet before Play (Play opens one, viewerless); it now fails only on a viewer
service or a session with presentation, and says which. Three first-test
runs of a full suite later failed it the other way: no viewer service, but
a session with presentation. With the error naming who
presents, the next one showed the `render.gl.init` binding presenting over
the built-in viewer's `#gl` canvas (connected, the session's first state
552x311). The path, inferred from that state rather than traced: the
viewer was mounted and disposed at startup (its extension off), and the
animation frame its `attach` schedules ran `V.layout` after the dispose
(only the viewer calls `render.gl.resize`, and layout had no disposed guard), so it committed the
compositor's pending size through `render.gl.resize`, and the binding, no
longer seeing the viewer's presentation on the stage, presented over the
canvas the viewer leaves in the panel skeleton. That is a production defect
(disabling the viewer, or a viewer runtime replaced at startup, leaves a
ghost presenter holding the window's session attached), and three fixes
close it:

- `V.layout` and `commitPendingResize` return once the runtime is disposed
  (`viewer-engine.test.ts`: a layout after dispose makes no
  `render.gl.resize` call; it fails without the guard).
- The built-in viewer marks its stage `data-engine-self-presents` from
  `attach` (before `render.gl.init`) until it is disposed, and the binding
  yields to that as well as to the viewer's presentation, so it never
  presents over the built-in viewer before its presentation exists either.
  Old forks that draw through the compositor still get the binding.
- The binding watches its canvas with a ResizeObserver and unbinds as soon
  as it leaves the document, instead of at the caller's next resize.

After the three fixes, two full-suite runs passed 28 of 28 each (the
headless spec first in both).

**Merged feat/native-engine twice more** during the round: perf2 (protocol
unchanged) and then svgcompat (the protocol build is now
`protocol-05819931f26a31b9`; the only conflict was install.ts's dispose,
which keeps both teardowns). pm-wasm and the addon were rebuilt each time,
and the viewer's version lock entry follows the merged tree (1.2.2,
unreleased).

**Final run (one lock, after every change).** pm-node: 88 library tests,
document 4 of 4, snapshot 7 of 7. Vitest (main/engine, preload,
engine-connection, kernel, extensions/viewer, engine-doc): 823 passed, 1
skipped. E2E `native-engine-transport`, `native-engine-tracer`,
`engine-headless-playback`, `engine-render-transport`,
`engine-present-recovery`: 28 of 28 (the J/K/L gate passed: 2x reverse one
sample over 10 ms, worst 10.1 ms). Then `engine-headless-playback` three
times: 6 of 6.

After the merges and the viewer fixes, at HEAD: pm-node 94 library tests,
document 4 of 4, snapshot 9 of 9; vitest 842 passed, 1 skipped;
svelte-check 0 errors; boundaries ok. The five e2e specs twice: 28 of 28
and 28 of 28 (J/K/L 2x reverse worst 7.4 and 7.0 ms, no sample over
10 ms). Earlier in the round, runs failed only on the headless premise
(above) and once on the J/K/L 2x reverse sawtooth (20 samples over 10 ms,
8.6 to 12.6 ms, the documented pattern; audio and performance owners).

**The join, for the orchestrator's decision.** Playhead (`PM.time`) error
while the viewer mounts mid-play, engine against today (flag off), same
invocation: 39.3 against 26.9 ms (the suite), then 36.9 against 25.2, 32.2
against 24.1, and 32.1 against 34.7 ms (the repeats). After the merge: 36.7
against 27.0, 32.0 against 23.5, 29.6 against 32.1 (repeats); final lock:
43.1 against 37.3 ms (the suite), then 28.9 against 24.0, 34.9 against
29.2, and 36.6 against 33.5 ms; after the svgcompat merge (repeats): 33.7
against 22.2, 32.2 against 31.1, 39.8 against 30.9, then 34.2 against
24.5, 32.4 against 24.0, 33.6 against 24.7; the last two suites: 27.8
against 25.4 and 22.1 against 23.2 ms. Over the 19 pairs the engine is
6.0 ms slower on average (from 2.6 ms faster to 12.4 ms slower; worst
43.1 ms against today's worst 37.3 ms). The engine's line
stays within 0.1 ms and the playhead is back on it within two animation
frames. The stall is the page's main thread mounting the engine viewer
(its presentation worker and painter); it is still recorded, not bounded.

### Round 7: a frame in flight at an `unsupported` report (2026-10-19)

Round 6's seq gate assumed a session's events arrive in the engine's
order. Paused frames do not: they leave from the completion thread
(`present/complete.rs`) once the GPU finishes them, and the render thread
sent the report from `document_step` without waiting for them. A paused
frame of revision R, still on the GPU, could arrive after a report caused
by the batch at R+1, with a higher seq than any frame the viewer had. The
session took it as a later decision, the route went back to `engine`, the
status line cleared, and a stale picture stayed up with the reason hidden.

- **The engine drains first.** `report_legacy` waits for the completion
  thread before it sends the report. Every route to `legacy` goes through
  it: a static reason, a draw that fails with `RenderError::Unsupported`
  (`force_legacy`), and a shader fallback runner failure (`M::Failed`, also
  `force_legacy`). The other `Unsupported` codes the engine sends are
  request-scope errors (frame requests, snapshots, Blender jobs) and do not
  change the route. The wait is bounded by the GPU work of at most
  `MAX_IN_FLIGHT` (2) frames, and happens only when the route changes.
- **The report names the newest seq.** `ErrorDetail::Unsupported {
  lastFrame }` is `view.next_seq - 1` at the decision. The session drops
  frames at or below it (falling back to the newest frame it received when
  the detail is absent) and passes it on in `routed`. The presentation
  drops presented frames at or below the larger of it and the newest its
  channel delivered. Protocol build: `protocol-a8f64376eb9e1e12` (the
  addon is rebuilt; pm-wasm does not depend on pm-protocol).
- **Tests.** `tests/document.rs`
  `an_unsupported_report_follows_the_frames_drawn_before_it` holds a paused
  frame in the completion thread for 600 ms
  (`POWERMOVE_BRIDGE_TEST_SEND_DELAY_MS`, tests only), sends a batch that
  adds an effect without definitions 150 ms later, and checks that the
  frame arrives before the report, that the report covers its seq, and
  that no frame follows within 1.2 s. Without the drain it fails exactly as
  reviewed: the report (`lastFrame` 2), then the seq 2 frame of revision 2.
  `engine-connection.test.ts` covers the in-flight frame again (seq 2 of
  revision 5 after a report naming seq 2: stale), replacing round 6's
  re-delivery of seq 1, which cannot happen. `presentation.test.ts` does
  the same with a report naming seq 7. Both TS cases fail with round 6's
  gating.
- **Runs.** pm-node: 94 library tests, document 5 of 5 (the new test three
  times on its own), present 2 of 2, shader_fallback 1 of 1; pm-protocol
  wire 11 of 11 (regeneration is a no-op); clippy `-D warnings` clean;
  vitest (engine-connection, engine-present) 152 of 152; svelte-check 0
  errors. Under the lock, `engine-render-transport`,
  `engine-headless-playback` and `native-engine-tracer` (the A, B, A, B
  switch with the viewer attached): 22 of 22. Join in that run: engine
  25.1 ms against today's 20.1 ms.

The viewer-mount playhead stall and the J/K/L 2x reverse sawtooth (issue
P5) now belong to the perf3 lane.

### Round 8: the viewer's version lock, the held-frame knob, 0x0 canvases (2026-10-19)

- **The viewer's version lock.** Round 7 changed `engine-present`
  (presentation.ts, protocol.ts, presentation.test.ts) after the lock was
  last refreshed, so `check:builtins` reported `viewer: tree changed
  without a version bump`. 1.2.2 is unreleased (main has 1.0.8, the
  integration branch 1.2.1), so only the viewer's entry is refreshed to the
  merged tree; the version stays 1.2.2. The check now fails only for
  inspector, keymap-default and timeline (other lanes).
- **The held-frame knob is test-only in the build.**
  `POWERMOVE_BRIDGE_TEST_SEND_DELAY_MS` is read only with pm-node's new
  `test-hooks` feature, which the crate's own tests turn on through a
  dev-dependency on itself (pm-audio's pattern). The addon
  (`build:engine`: `--features node`) compiles the other `send_delay`, which
  answers `None`, so a stray variable can no longer hold a paused frame in
  a shipped build. `the_send_delay_knob_is_read_only_with_test_hooks` covers
  the parse and the 5 s cap; `tests/document.rs` still holds its frame.
- **A 0x0 canvas that leaves the document.** The binding's default watch
  (`watchRemoval`, canvas-binding.ts) keeps the ResizeObserver and adds a
  MutationObserver on the document's child lists that reports once the
  canvas is disconnected: a ResizeObserver never fires for a canvas whose
  box was already 0x0, so such a canvas stayed bound until the caller's
  next `render.gl.resize`. Two happy-dom tests (the watch on its own, and a
  bound 0x0 canvas whose stage is removed) fail without the
  MutationObserver.
- **Sync snapshots that need the fallback runner (a regression from the
  merge).** After merging feat/native-engine, `engine-render-transport`'s
  real-WebGL case failed: `failed` 1, `internal: its draw did not finish
  within 10 s`. The defs lane's runner (shader-fallback.ts, not this lane's
  file; the edit is minimal) made `renderSync` poll each pass's fence on
  the calling thread, but WebGL lets a fence signal only once the event
  loop runs again, so every synchronous snapshot needing a fallback program
  spun to the 10 s draw deadline and failed with `watchdog`. `finishedSync`
  now waits with `finish` (it blocks until the GPU ran the pass) and judges
  the deadline when it returns: a pass over it still fails with `watchdog`,
  loses the context, and its program is never run again; a hang the GPU
  process cuts surfaces as `contextLost`. The calling thread cannot stop a
  draw part way, which the asynchronous path still can. A unit test models
  WebGL's rule (every poll answers `TIMEOUT_EXPIRED`) and fails without the
  fix; the e2e now names `lastFailure` when it fails. The async path
  (viewer, job exports) is unchanged.
- **Wording.** Commit 73b2b2a2's message states the ghost-presenter path
  as fact; it is inferred from the session's state, as round 6 above says.
  The fixes stand either way; history is not rewritten.
- **Merged feat/native-engine** (69 commits: defs, svgcompat and others).
  The only conflict was the generated `PROTOCOL_BUILD`; regenerating
  (`pm-protocol-gen`) gives `protocol-6fa9b179042f4a70`, and both addons
  were rebuilt with `bun run build:engine` (development runs load
  `engine/target/pm-bridge/pm_bridge.node`; without it, flag-on routes stay
  `pending`). `cargo fmt --check` over the whole workspace now reports
  drift in pm-eval, pm-export, pm-geom3d, pm-render and pm-vector files
  from merged lanes; this lane's crates are clean.
- **Runs.** pm-node: 96 library tests, document 5 of 5; pm-protocol: 93
  library, wire 11 (1 ignored); clippy `-D warnings` clean for pm-node and
  pm-protocol with tests and for pm-node `--features node`; vitest
  (engine-connection, extensions/viewer, main/engine, preload) 42 files,
  466 passed; svelte-check 0 errors. E2E under the lock with the rebuilt
  addon (`engine-render-transport`, `engine-headless-playback`,
  `native-engine-tracer`): 22 of 22 after the fallback fix (before it, 21
  of 22). Join in that run: engine 35.6 ms against today's 22.1 ms (issue
  P5, perf3).

### What remains

- **The job engine's fonts.** Export and agent frames still refuse text:
  the job engine has no font service, and pm-export gates with the
  fonts-less capability check (`plan_export`, `StillRouting`). The broker
  and `RenderFonts` are process-independent; the job engine needs main to
  replay project fonts to a job-engine font port and pm-export to gate
  with fonts (export lane).
- **Compat level 2 text** needs level 2 fonts (`TextFonts::level2`); the app
  creates no level 2 layers yet, so none are drawn either way.
- **Compatibility rasters in frames.** The interactive engine assembles and
  holds uploaded compatibility rasters, and main serves the job engine's
  `compatFrame`, but pm-render does not draw a layer from them yet and
  pm-export does not ask for them yet; `needsRenderer.compat` is always
  empty (lanes L1, L4, L6).
- **Blender frames that cannot be built** (imported models, model-group
  recipe parts: the snapshot builder's refusals that exports route) fail
  an async capture with `unsupported`, where today's capture has Blender
  convert and render them (3D lane).
- **What the engine still refuses** (the remaining
  `pm_render::unsupported` reasons: SVG images with text, animated images
  without a rendition, unknown project color settings, missing
  definitions, and the rest) fails a snapshot with the engine's
  reasons, and the viewer shows `The new engine cannot draw this
  composition yet: <reasons>.` on an empty stage while the transport and
  audio run on the engine. That is a visible difference from today, so these
  projects remain cutover blockers until L1 draws the features; the
  derouter lane removes the `legacy` route name and the `routed` message.
- **Performance panel.** `FX passes` reads 0 (the engine counts no effect
  passes) and `GL draws` shows the layers the engine drew (not WebGL draw
  calls).
- **Depth and id consumers.** The surfaces are delivered with each
  frame (`child_depth` proves the pool, the per-frame tag, and the slot
  rule; `presenter.test.ts` the viewer side), but nothing reads
  `depthFrame` yet and no test reads the surface's values back through the
  bridge: the 3D lane's chrome sets `ViewParams.depthId` and samples it.
- **Old custom viewers** are covered by unit tests of the binding; no e2e
  loads a frozen viewer fork yet (the extcompat lane's fork corpus).
- Capability routing (`routed`) remains in the document/presentation
  scaffolding for the derouter lane to remove. With the flag on, transport
  and audio remain exclusive to the native engine through these failures.

## Gaps and next steps

- The kernel's engine connection (which registers `engine.viewer` and
  decodes the viewer's port) is not in this lane. The integration test and
  the real-app harness play that part (`integration/page-engine.ts`).
- Pointer-to-pixel: the layer drag fails the Phase 2 row (render cost of
  the document, the page's main thread); the pan's p99 under load is worse
  than today's (live resizes hold the main thread with a descriptor
  queued). See [Decision](#decision-before-the-phase-2-gate).
- A/V sync under the gate load: frames during live window resizes fall
  outside ±10 ms both ways (the WindowServer's presentation timing), 6 to
  9% of frames on the synthetic projects. The prediction is calibrated on
  this panel (3.2 ms presentation phase); a 60 Hz display and the base
  machines are not measured.
- Cadence: not ideal on the Chromium path (the sink race and the
  WindowServer's presentation lag); see [Cadence](#cadence-at-120-hz).
- Layer drag, orbit, and walk through the engine, and playback with the
  engine drawing the document, wait for `pm-render` to draw real layers.
- C′ is a prototype: not adopted (25 drops a minute under load). Its
  overlay, capture, and resize handling are not built.
- No ProMotion request: Electron has none, and public API cannot reach a
  multiple of 25 fps on the 120 Hz panel.
- The pool retry after a failed hand-off is covered by unit tests of its
  policy (`PoolRetry`, `Generations`) only; making a real Mach send fail
  part-way through a pool in a test needs a fault hook in pm-bridge.
- A signed end-to-end run (Team ID through Electron) needs the release
  signing identity; the Team ID check is covered by Rust tests against a
  real Developer ID process.
- A worker canvas has no presentation feedback at run time: the
  prediction is calibrated from viz's traces in the harness, not
  corrected per frame.
- 3D orbit and walk through the engine are not measured (no engine-drawn
  3D corpus scene with a gesture harness).
- The measurements share the machine with other agents' builds (load
  average 2.3 to 16; the document's render cost moved between 10 and
  17 ms from run to run on 2026-10-16); a quiet-machine round would
  tighten the spread.

## Pacing at 120 Hz

Playback at the display's rate shows every frame the engine renders, from
the first refresh the first new frame can make. The 120 Hz rig's `playing`
row checks it (`docs/engine/perfbench.md`; `PM_PERFBENCH_RIG_TRACE` writes
the per-frame trace).

1. **One vsync timeline** (`VsyncEstimator`, `display-timing.ts`). The
   presentation worker locks once on the median of its BeginFrame samples,
   then tracks the phase like a PLL: each sample within a quarter refresh
   moves it by 1/16 of its error (at most 1/16 of a refresh); twelve
   agreeing samples outside that band relock it. The engine's vsync hint
   is this timeline, re-sent when the phase moved 100 µs (at most every
   250 ms), so the engine's plan and the viewer's decisions agree.
   Decisions sit halfway between frame boundaries, folded by the frame's
   exact share of a refresh (not the estimated period, which a seconds-old
   clock sample would turn). Within a run each refresh decides one refresh
   after the last, as many as its animation frame says passed (a BeginFrame
   up to three quarters of a refresh off is still the next refresh): a
   BeginFrame phase that moves by half a refresh under load neither skips
   nor repeats a frame, and a missed refresh skips its frame. Decisions
   more than a refresh and a half from the grid's are centred again.
2. **The worker draws timed frames in its animation frame.** A worker canvas
   hands viz what an animation frame drew when the callback returns; a frame
   drawn later in the refresh waits for the next BeginFrame. The display
   prediction is 1.5 refreshes and 3.2 ms (A/V sync above).
3. **Render-ahead covers twice the delivery time and a phase slot**
   (`Schedule::lead_frames`): at least `DEFAULT_LEAD` (4) frames, and
   2 × delivery + one refresh. Delivery is the viewer's display latency
   (the hint's `latencyUs`), half a refresh and the arrival; without a hint,
   3 refreshes. One delivery is the viewer's horizon, the other covers the
   render and a stall on the way (the window's main thread busy for a dozen
   milliseconds); the refresh is the longest wait for the render's phase
   slot: 7 frames at 120 fps on a 120 Hz display, 6 at 60 fps on a 60 Hz
   one, the floor of 4 for slower frame rates on 120 Hz. A frame leaves the pool when the viewer has drawn it, about two refreshes
   before it shows, so `poolWaits` stays 0.
4. **Playback frames leave from the completion thread**
   (`present/complete.rs`). The render thread encodes the next timed frame
   while the GPU finishes the last; each is sent, in order, when its GPU
   work is done, so a frame costs the render thread its compose only. The
   schedule's cost estimate is render start to GPU done. The C′ prototype
   presents on the render thread.
5. **Frame events follow the refresh that drew the frame**
   (`PresentedEvents`). A timed frame is announced at the first page
   animation frame of a later refresh than its draw (the page's and the
   worker's BeginFrame times agree within 0.1 ms), at most one emission per
   page animation frame, inside it (decision EXT-TIMING-1).
6. **Play starts the picture and the sound on one line**
   (`Thread::start_time`). The line's first media shows at the earliest
   instant both can make: for the picture, the first refresh the first new
   frame can be ready for (what is left of its render, its delivery to the
   worker, then the viewer's display latency); for the sound, the device's
   latency and one of its buffers. pm-audio presents the run's first sample
   at that host time (`Start::At`), partway into a device buffer, or passes
   over what should already have played when its audio was late; its first
   clock sample confirms the line (`TransportClock::follow`). A Play the
   presenter forwarded to the audio at ingress (PLAY-1) is prepared there
   and held (`Start::Held`, 150 ms at most) until the render thread times
   it (`AudioCtl::StartAt`); a forwarded Play the render thread defers or
   ignores starts at once. A device that never confirms leaves the line on
   the host clock; without the viewer's timing the line starts after
   `PREROLL_US`. When the playhead's frame is on screen and current, the
   run renders from the next frame and the engine connection announces the
   frame on screen again at the next animation frame
   (`PresentedEvents.replay`), as today's Play does.
7. **The first new frames are rendered while paused** (`Thread::arm`).
   Once the paused frame has stayed 50 ms, the next frames of a forward 1×
   run go into the paused pool's free slots (one stays free); any change
   gives them back. A Play sends them at once, before the playing pool
   replaces the paused one (`BridgePresenter::transport` sends the Play
   first), and the viewer keeps drawing frames of a pool whose surfaces
   match the new one's (`presenter.ts` `onPool`). Sessions with
   compatibility producers start through the native preroll instead.

`POWERMOVE_ENGINE_SIM_AUDIO=1` replaces the audio device with a simulated
one (48 kHz, 512-frame buffers, 10 ms latency, called back a buffer ahead)
for a machine whose output does not run (rig `PM_PERFBENCH_RIG_AUDIO=1`).

Open, outside the bridge: **viz's latch.** viz draws a refresh as soon as
every client has submitted, about 1.4 ms after BeginFrame with the page and
the worker canvas. BeginFrames can come up to 2 ms early under load, and a
draw that early is presented with the previous one, which is never seen
(`Graphics.Pipeline.DrawAndSwap` feedback). Today's viewer escapes it on
heavy projects only because its page frame takes about 3 ms. C′ presents
outside viz.

#### perf3: prepare the viewer drawing before mount

The native connection owns `PresentationWarmup`, a single detached drawing for
its document. It starts before registering the viewer service. The drawing has
no engine session or animation loop; the worker initializes its WebGPU device
and compiles SDR/EDR pipelines asynchronously. `ViewerPresentation` claims the
canvas and worker together and transfers failure/loss callbacks to the current
backend token. A popout cannot claim a drawing from a different document. If
worker WebGPU fails before claim, a detached page painter is prepared instead.
Closing a viewer prepares the next spare; removing the connection disposes the
unused spare, including painters that finish initialization after disposal.

This removes worker construction and first-pipeline compilation from a warmed
mid-play mount. It does not remove the viewer extension's own DOM work. See
`perf.md` P5 for the deterministic reverse scheduling result and remaining paired
mount/A/V gates; neither physical-display gate is claimed from unit tests.

## Today's viewer canvas (decision V1, 2026-10-19)

With "Use New Engine" on, the viewer renders at today's canvas resolution
and scales it as the browser scales today's canvas, at every zoom and
preview resolution. Above 100% it renders today's cropped canvas from the
crop's unrounded origin (decision V1b, "The output origin" below). What
was measured, and the differences that remain (the video's editing copy,
the P3 canvas), are below.

**Today.** The legacy viewer draws a view into a WebGL canvas it sizes
itself (`V.layout` in `extensions/viewer/viewer.ts`):

- below 100% (and for crop-unsafe projects at any zoom), `previewRenderSize`:
  the composition at `min(1, zoom × min(2, DPR)) × quality`, so a 2x display
  at 61% draws 1 canvas pixel per composition pixel, not 1.22;
- with an explicit preview resolution (Full, Half, Quarter), the composition
  at `quality` whatever the zoom;
- above 100% in a crop-safe project, the cropped `previewRenderViewport`
  (`round(cssWidth × min(2, DPR) × quality)` over the crop).

The canvas fills its CSS box (`V.inner`, or the crop's box), and Chromium
composites it there: the box snapped to whole device pixels (each edge
rounded, as Blink snaps a replaced element's content box) and the 8-bit
canvas sampled bilinearly, without mipmaps, clamped at its edges. The
compositor picks every raster (text, shapes, SVG images) at that canvas's
`outputScale`.

**The engine.** The viewer computes the same geometry before it sends the
view and sends the canvas as `ViewerState.canvas` (`todaysCanvas` in
`engine-present/view-params.ts`: backing size, the composition part it
holds, and its CSS box; RFC 02 version note). The kernel's canvas binding
sends the one a `render.gl.resize` caller sized. pm-node's
`compose::canvas_placement`:

- renders the frame at the canvas's size and today's `outputScale` (the
  canvas over its crop), so every layer and effect is drawn at today's
  resolution; only the canvas texels the visible frame pixels read are
  rendered;
- stores the region as today's 8-bit opaque canvas (`Rgba8Unorm`,
  composited over black), then scales it into the box snapped as Blink snaps
  it, with hardware bilinear sampling clamped at the canvas edges, into the
  working texture the display transform presents;
- without a canvas (the integration harnesses, older callers), draws the
  composition at zoom × DPR × the preview resolution as before.

**The output origin (decision V1b, 2026-10-21).** Today's compositor maps
composition pixel `x` to canvas pixel `x · sx − viewport.x · sx` with a
preview viewport (`scaledWorld`), the origin unrounded, and its background
(`backgroundView`) and effect coordinates (`effectViewportCoordinates`)
follow the same origin. pm-render now has that origin
(`RenderRequest::origin`): every level 1 placement goes through it
(`scaled_world`, `vector_world`: shapes, text, SVG images, solids, images,
video), as do 2.5D planes, masks, local level 2 content, groups, mattes and
motion-blur accumulations; a precomp renders its own target from (0, 0) as
today's nested `renderProject` does. The level's `OutputView` gives the
background gradient today's view and the effect chains (native and the
fallback runner) today's `u_pxOrigin` and `u_coordRes`. Draw reuse keys on
it. `canvas_placement` renders today's canvas itself: its backing size, at
today's `outputScale`, from `crop.x · sx` unrounded. So a crop never needs
more than today's canvas (at most the largest output): the smaller-canvas
fallback (`Placement::reduced`, the `reducedFrames` counter) is gone, and a
crop at 800% of a 1920-pixel composition's right edge or 250% of a 4K
one draws at today's full resolution and buckets. Without a canvas (the
harnesses), a view past the largest output draws its visible part from
that part's whole-pixel origin. Snapshots and exports send no origin and
are bit-identical to before (measured below).

The preview quality is honoured exactly as today because the canvas is
today's: Auto follows `api.transport.quality` (the adaptive ladder),
explicit Full and Half draw the composition at their quality whatever the
zoom (Full at 25% is a 2× downscale by the browser, as today). During a
paused zoom or pan gesture today keeps its committed canvas and shows its
last frame scaled by CSS until the gesture settles (`canRetainNavigation`,
100 ms after the last event). The engine view keeps its committed canvas by
the same rule (`engine-present/canvas-hold.ts`, unit tested): inside an
unsettled, paused gesture; while the engine's frame on screen is current
(drawn at the committed canvas, at the engine's revision and today's time,
not a stand-in; today's `presentationIsCurrent`) and the project object,
its size, the animation version, time and quality are unchanged; and, for
a crop, while the visible part of the composition stays inside it. Its box
follows the gesture, and when the gesture settles the hold lays the viewer
out again so the engine takes the new canvas (today's frame loop commits
it the same way). Below 100% the canvas holds the whole composition, so a
frame rendered at the old canvas is today's stale frame, rescaled. Today
keeps a crop's pan buffer (`previewRenderViewport`'s `previous`, the
compositor's `previewViewport`) while it covers the view; while the
engine presents, the compositor never resizes, so the viewer passes the
crop committed for the engine instead, and the engine draws the crop today
would.

GPU work below 100% falls with the canvas: a 1920 × 1080 composition at
61% on a 2x display renders 2.07 MP instead of 3.09 MP, at 100% 2.07 MP
instead of 8.29 MP.

**Measured** (`e2e/engine-viewer-canvas.spec.ts`, Electron lock, one
worker, 2026-10-19 run 3; a 640 × 360 project with two text layers, a
rotated rounded rectangle, a 1 px line, a blurred star, a rotated level 1
SVG image and a video, crop-safe; and a 1920 × 1080 project with text, a
rotated box and a hairline at its right edge; stage 748 × 452 CSS at DPR
2, captured with `capturePage`, 1496 × 904 pixels; both windows with
`--force-color-profile=srgb`; corpus bound: every channel within 1 CV for
99.9% of pixels, none beyond 4). "Content" compares the engine's snapshot
with today's `renderToPixels` at the canvas size (whole-canvas cases only;
today's ran twice, with the same result both times):

| Case | Today's canvas | Whole stage: within 1 CV, max | Outside the video layer | Content | Display profile (P3), run 1 |
|---|---|---|---|---|---|
| 25% Auto | 320 × 180 | 99.997%, 49 | 100.0000%, 1 | 93.75%, 228 | 99.919%, 49 |
| 50% Auto | 640 × 360 | 100.000%, 1 | 100.0000%, 1 | 100%, 1 | 99.778%, 2 |
| 61% Auto | 640 × 360 | 99.999%, 2 | 99.9995%, 2 | 100%, 1 | 99.892%, 4 |
| 100% Auto | 640 × 360 | 99.999%, 2 | 99.9997%, 2 | 100%, 1 | 99.714%, 4 |
| 25% Full | 640 × 360 | 100.000%, 1 | 100.0000%, 1 | 100%, 1 | 99.979%, 3 |
| 50% Full | 640 × 360 | 100.000%, 1 | 100.0000%, 1 | 100%, 1 | 99.781%, 2 |
| 61% Full | 640 × 360 | 100.000%, 2 | 99.9998%, 2 | 100%, 1 | 99.895%, 4 |
| 100% Full | 640 × 360 | 100.000%, 2 | 99.9996%, 2 | 100%, 1 | 99.723%, 4 |
| 25% Half | 320 × 180 | 99.997%, 49 | 100.0000%, 1 | 100%, 1 | 99.919%, 49 |
| 50% Half | 320 × 180 | 99.973%, 37 | 99.9998%, 2 | 100%, 1 | 99.905%, 37 |
| 61% Half | 320 × 180 | 99.957%, 44 | 99.9980%, 2 | 100%, 1 | 99.873%, 44 |
| 100% Half | 320 × 180 | 99.888%, 43 | 99.9962%, 2 | 100%, 1 | 99.620%, 43 |
| 237% Auto (crop) | 2056 × 1464 | 99.080%, 59 | 99.0627%, 59 | cropped | not run |
| 237% panned Auto | 2056 × 1464 | 98.315%, 58 | 98.2941%, 58 | cropped | not run |
| 237% Full (crop) | 2056 × 1464 | 99.080%, 59 | 99.0627%, 59 | cropped | not run |
| 237% panned Full | 2056 × 1464 | 98.315%, 58 | 98.2941%, 58 | cropped | not run |
| 237% Half (crop) | 1052 × 756 | 98.524%, 55 | 98.4970%, 55 | cropped | not run |
| 237% panned Half | 1052 × 756 | 96.556%, 171 | 96.5193%, 55 | cropped | not run |
| 800% right edge Full (1920 × 1080) | 2008 × 1416 | 96.719%, 226 | 96.7187%, 226 | cropped | not run |

The control: today's viewer captured again with the video's editing copy
removed (it then draws the original):

| Case | Whole stage against the engine |
|---|---|
| 25% Auto | 100.000%, 1 |
| 25% Half | 100.000%, 1 |
| 50% Half | 100.000%, 2 |
| 61% Half | 99.996%, 2 |
| 100% Half | 99.996%, 2 |

What the numbers say:

- At 25 to 100% (today's whole-composition canvas) every case holds the
  bound outside the video layer, and the whole stage holds it at every
  Full case and at Auto from 50% up. Where it misses (Auto at 25% and
  every Half case, up to 49 CV, on the video layer's edges only), the
  cause is now proven: today's viewer draws a video's editing copy when the
  preview resolution is Auto or its quality is below 1
  (`previewVideoElement`, the intra-1280 derivative) and the engine draws
  the original (no proxies yet, section 14). With the copy removed, today's
  viewer matches the engine on the whole stage in all five cases. This
  belongs to Phase 4a (proxies).
- **Above 100%, cropped (this table, before V1b).** Every 237% case
  missed the bound outside the video too: 1 to 3.5% of pixels, up to 59
  CV, on every hard edge, because the engine's crop grid started at the
  nearest output pixel instead of the crop's fractional origin (`crop.x ×
  sx`, here 488.8 output pixels). Fixed by V1b; see the table below.
- At 800% flush with a 1920-pixel composition's right edge the engine
  drew today's crop at 53% of today's resolution (up to 226 CV on 3.3% of
  pixels). Fixed by V1b.
- Content without the scaling holds within 1 CV in 11 of 12 cases. 25%
  Auto misses it on exactly the video layer (3,600 pixels, the 80 × 45 the
  video covers in that 320 × 180 canvas), in both of today's renders, so
  it is not a first-render effect: today's `renderToPixels` at Auto draws
  the video's editing copy at a different frame there. The same canvas at
  25% Half, where today draws the original, is within 1 CV.
- With the display's own profile (last column, run 1) the engine's
  display-p3 canvas moves single channels of smooth gradients by 2 CV
  through the 8-bit P3 round trip at every zoom, including 50% where both
  canvases map 1:1: the presentation's color space, not the scaling.

Before V1 the engine drew 61% at 1.22 and 100% at 2 device pixels per
composition pixel, sharper than today everywhere. Evidence (today, engine,
differences in red), outside the repository:
`/tmp/ne-w5/viewer-canvas-evidence3/` (this table),
`/tmp/ne-w5/viewer-canvas-evidence2/` (run 2) and
`/tmp/ne-w5/viewer-canvas-display/` (display profile).

The spec asserts the bound on the whole stage and outside the video for
every case, and lists the cases above that are known to miss it, with
their issue, in `KNOWN`; a known case that starts passing fails the test,
so none can pass by accident. A second run (`viewer-canvas-evidence4`)
reproduced every number above within 0.002 points and passed in 1.4
minutes.

Also run after V1 (Electron lock, one worker; again after round 2 with
the same results): `engine-svg.spec.ts` 6 of 7 (the seventh is the exported
still, E1 in vector.md, unchanged); `engine-render-transport.spec.ts` 7 of
7; the `viewer-*.spec.ts` specs 21 of 23, the two failures in
`viewer-resize.spec.ts` (a handle drag leaves `scale.x` at 100) failing the
same way with the branch's base `viewer.ts`. The svg and text gates (round
1): `svg_corpus` 34 projects, 105 frames, 0 failed; `text_corpus` 576
frames, 0 failed; `text_raster` 0 failed. pm-node `tests/compose.rs` checks
the snapped box, the rendered texels, the bilinear values against the
8-bit canvas, and (V1b) a crop far from the origin drawn at today's scale
from its unrounded origin, each hard edge on today's texel; the compose
unit tests check its geometry for 1920 and 4K compositions.
`canvas-hold.test.ts` checks the gesture hold: holding inside a gesture,
releasing on a version, time, quality, project or size change, before the
engine presented the committed canvas, and when the view leaves the crop,
and the settle's re-layout.

**After V1b** (2026-10-21, the same spec and machine, run
`viewer-canvas-evidence-v1b`; the 237% and far-crop cases are no longer
listed as known, so the spec requires the bound on the whole stage):

| Case | Before V1b: whole stage | After V1b: whole stage |
|---|---|---|
| 237% Auto (crop 2056 × 1464) | 99.080%, 59 | 100.000%, 1 |
| 237% panned Auto | 98.315%, 58 | 99.9999%, 2 |
| 237% Full | 99.080%, 59 | 100.000%, 1 |
| 237% panned Full | 98.315%, 58 | 99.9999%, 2 |
| 237% Half (crop 1052 × 756) | 98.524%, 55 | 100.000%, 1 |
| 237% panned Half | 96.556%, 171 | 99.986%, 2 |
| 1920 × 1080 at 800%, right edge, Full | 96.719%, 226 (53% resolution) | 100.000%, 1 |
| 3840 × 2160 at 250%, right edge, Full | not run | 100.000%, 1 |

The 25 to 100% cases are unchanged (within 0.001 points of the table
above; the five video editing-copy cases still miss the stage bound on the
video's edges only and pass with today's copy removed). The spec passed
(1.6 minutes).

Snapshots and exports, which never set an origin, are bit-identical to
before V1b: `pm-cli parity render` at the branch's pre-V1b head and after
it, with the SHA-256 of every sample's float output dumped, gave the same
179 hashes over 98 corpus projects (and the same 9 parity failures), and
`pm-cli render --format png` of 45 corpus projects (shapes, effects,
masks, groups, precomps, mattes, backgrounds, transitions, 2.5D, text,
compositing; 12 frames each) wrote 540 byte-identical PNGs. pm-render
`tests/output_origin.rs` checks the origin itself: a whole-pixel origin
draws the part of the whole frame it starts at (gradient background,
blurred star, masked shape, 2.5D plane, multiply group, precomp), a
fractional one moves every layer like a composition translation, the
background follows today's view, and draw reuse follows the origin.

Gates rerun after V1b (2026-10-21): `svg_corpus` 34 projects, 105 frames,
0 failed; `text_corpus` 576 frames, 0 failed, 4 FONTS-R ratcheted (as
before); `text_raster` 0 failed; `path_masks`, `effects_webgl` and
pm-vector `canvas2d` pass; `corpus.rs` 354 frames, 5 failed, all video
class (`motion/blur-video` at 0.5 and 1.2 s, max 255; `retime/speed` at
2.9 s, max 3; `space25d/precomps` at 1.5 and 2.6 s, max 7 and 9), failing
identically (same pixel counts and maxima) at the branch's pre-V1b head,
so not V1b's.
E2e after V1b: `engine-svg.spec.ts` 6 of 7 (E1 unchanged: 56,037 and
59,892 values off by 1 CV), `engine-render-transport.spec.ts` 7 of 7, the
`viewer-*` specs 21 of 23 (the two `viewer-resize.spec.ts` failures that
also fail on the base).

Remaining:

- The video's editing copy (Phase 4a).
- The engine presents a display-p3 canvas on a wide-gamut display where
  today's canvas is sRGB; the 8-bit P3 round trip moves single channels of
  smooth gradients by 2 CV (above), at every zoom.
- No e2e drives a pinch with the engine on and compares frames during it;
  the hold's rule is unit tested.
- The stage origin is assumed to sit on whole device pixels (Blink snaps
  the canvas box in window coordinates; the engine snaps it in the stage's),
  and an edge exactly halfway between device pixels rounds away from zero
  (Rust's `f64::round`), which may differ from Blink on negative ties.
