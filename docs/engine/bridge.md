# The production frame bridge

Design notes and measurements for the frame bridge (plan section 18, with
section 26 amendments 4 and 5; contracts 2 and 3). It productionizes spike
S1 (`docs/engine/spikes/S1.md`, `engine/spikes/s1-bridge/`). Everything runs
behind "Use New Engine".

**Status: flag-gated; the amended S1 gate against the real WebGL viewer
is not met, on pointer-to-pixel (amendment 18: Jude decides).** The bridge
passes its Electron integration test in both presentation modes (frames
drawn on the page's main thread and by the viewer's presentation worker)
and the quality lines. Against the real viewer playing the corpus project
in the real app, the engine path presented from the worker is better for
playback, idle and under the gate load (0 to 5 drops a minute against 84
to 238 under load), and worse on gestures: 5 to 9 ms slower at p50 idle
in three of four paired runs (r9: 19.5 against 14.5 ms) and slower at p99
idle and under load (see
[The amended S1 gate](#the-amended-s1-gate-against-the-real-viewer)).
Amendment 18's three steps are in and measured; the C′ prototype misses
the zero-drop line and is not adopted; Electron offers no ProMotion
request. Everything stays behind "Use New Engine": nothing presents
unless the kernel registers `engine.viewer` and the engine has a frame
source. The options are in [Decision](#decision-before-the-phase-2-gate).

## Where the code is

| Part | Code |
|---|---|
| Rendezvous, audit token, Team ID, Mach wire format, main's receive loop | `engine/crates/pm-bridge` (`rendezvous`, `audit`, `wire`, `host`, `node`) |
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
| S1 measurement of the production path | `apps/desktop/scripts/frame-bridge-measure.mjs`, `src/main/engine/integration/measure-*.ts` (stand-in page), `measure-app-*.ts` (the real app); summaries in `docs/engine/bridge-measurements/` |

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
   device pixels: ring (2) + render-ahead (4) + 2 = 8 surfaces, `rgbaf16`
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
   when that refresh reaches the display (two refreshes out for a worker
   canvas, measured), maps it to media time on the latest clock sample,
   and draws the frame whose interval contains it (`importExternalTexture`,
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
  Play Every Frame, and the 3D viewport's shading and X-ray. `ViewSequencer`
  numbers each distinct state; descriptors echo it. The editor camera is not
  in the 3D viewport's API yet, so the override carries no camera (see
  gaps).
- **Revision tagging and overlay geometry.** Every drawn frame's `revision`,
  `viewSeq`, and overlay geometry go to `onPresented` listeners;
  `overlayInCss` converts the geometry with the DPR of the state that frame
  shows. The viewer exposes `engineFrame()` and redraws its overlay at each
  new tag, so handles drawn from frame geometry never lead the pixels.
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
  presentation on the main thread (the harness's comparison).
- **Gestures on raw updates.** While the engine presents, the viewer's
  drags take their moves from `pointerrawupdate` instead of the
  rAF-aligned `pointermove` (`raw-gestures.ts`), so the engine hears about
  a move before the animation frame that would carry it. The drag helper
  keeps capture, up, cancel, and the cursor; pointer-lock drags keep
  `pointermove`. The view state leaves before `V.layout` writes styles.
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
- **Packaging.** `native/pm_bridge.node` is in electron-builder's `files`
  and the release script's `--with-engine` check; before, a packaged app
  could not load the bridge, so it presented nothing and the Team ID policy
  could never run in a signed build.

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

## Gaps and next steps

- The kernel's engine connection (which registers `engine.viewer` and
  decodes the viewer's port) is not in this lane. The integration test and
  the real-app harness play that part (`integration/page-engine.ts`).
- Pointer-to-pixel fails the amended gate (idle p50 and p99, load p99);
  see [Decision](#decision-before-the-phase-2-gate).
- Engine playback frames show 6.1 to 17.9 ms after their time on the
  transport clock's line (p50, r6 to r9), outside section 15's ±10 ms in
  every run but r9. With frames
  picked for two refreshes out and rAF to presentation at 18.6 to 20.3 ms,
  a frame shown at its first chance would land 2 to 12 ms after its time
  (about 7 ms on average); the measured p50 sits at the top of that
  range, which is not explained. The presenter's
  clock-to-display offset needs calibrating against measured presentation
  (and the audio device latency added) before A/V sync can be claimed.
- Layer drag, orbit, and walk through the engine, and playback with the
  engine drawing the document, wait for `pm-render` to draw real layers.
- C′ is a prototype: not adopted (25 drops a minute under load). Its
  overlay, capture, and resize handling are not built.
- No ProMotion request: Electron has none, and public API cannot reach a
  multiple of 25 fps on the 120 Hz panel.
- The pool retry after a failed hand-off is covered by unit tests of its
  policy (`PoolRetry`, `Generations`) only; making a real Mach send fail
  part-way through a pool in a test needs a fault hook in pm-bridge.
- The editor camera of the 3D viewport is not exposed by its API, so the
  view override sends shading and X-ray without a camera.
- A signed end-to-end run (Team ID through Electron) needs the release
  signing identity; the Team ID check is covered by Rust tests against a
  real Developer ID process.
- The worker canvas's rAF → presentation is one or two refreshes from run
  to run (r9: about half each, idle and under load; r5 to r8: mostly
  two), which costs cadence in such a run (jitter p50 3.7 ms idle in r9)
  and moves the A/V offset. A worker's WebGPU canvas has no presentation
  feedback, so the presenter cannot correct it; viz's timing or a
  feedback path would be needed.
- The measurements share the machine with other agents' builds (load
  average 2.3 to 14.3); a quiet-machine round would tighten the spread.
