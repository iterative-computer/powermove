# The production frame bridge

Design notes and measurements for the frame bridge (plan section 18, with
section 26 amendments 4 and 5; contracts 2 and 3). It productionizes spike
S1 (`docs/engine/spikes/S1.md`, `engine/spikes/s1-bridge/`). Everything runs
behind "Use New Engine".

**Status: flag-gated infrastructure; the amended S1 gate is not met.** The
bridge is complete and passes its Electron integration test and the quality
lines, but on the S1 harness the engine path drops more frames under the
gate load and is slower from pointer to pixel at p50 than the harness's
stand-in for today's viewer (see [Measurements](#measurements-against-the-amended-s1-gate)).
It can land only behind the flag, presenting nothing unless the kernel
registers `engine.viewer` and the engine has a frame source. Whether Phase 2
proceeds on this path needs a decision before its gate; the options and what
each is expected to change are in [Decision needed](#decision-needed-before-the-phase-2-gate).

## Where the code is

| Part | Code |
|---|---|
| Rendezvous, audit token, Team ID, Mach wire format, main's receive loop | `engine/crates/pm-bridge` (`rendezvous`, `audit`, `wire`, `host`, `node`) |
| Viewer surfaces, display transform, bridge test source | `engine/crates/pm-gpu/src/bridge_out` |
| Presenter in the interactive engine (pools, render thread, transport clock, descriptors) | `engine/crates/pm-node/src/present`, hook in `engine/crates/pm-engine/src/present.rs` |
| Main: one bridge per engine launch, import once, send once per receiver | `apps/desktop/src/main/engine/bridge.ts`, wired in `main/engine/index.ts` |
| Preload receiver | `apps/desktop/src/preload/frame-bridge.ts`, `shared/frame-bridge.ts` |
| Viewer presentation (jitter buffer, WebGPU painter, view state, overlay tags) | `apps/desktop/src/extensions/viewer/engine-present/`, hooked into `viewer.ts` |
| Electron integration test (run on every Electron upgrade) | `apps/desktop/scripts/frame-bridge-integration.mjs`, `src/main/engine/integration/bridge-*.ts` |
| S1 measurement of the production path | `apps/desktop/scripts/frame-bridge-measure.mjs`, `src/main/engine/integration/measure-*.ts`; summaries in `docs/engine/bridge-measurements/` |

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
   queues it in the jitter buffer. At each animation frame it predicts when
   that refresh reaches the display, maps it to media time on the latest
   clock sample, and draws the frame whose interval contains it
   (`importExternalTexture`, sampled 1:1 into a WebGPU canvas:
   `rgba16float` with extended tone mapping for `rgbaf16` pools).
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

## What the Electron integration test found

`bun run test:frame-bridge` (Electron 44.0.0, Chrome 152) passes and checks,
on the production code: one verified check-in from the spawned pid; each
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

Two production fixes came out of it:

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

## Measurements against the amended S1 gate

Amendment 5: the engine path is no worse than today's viewer on the same
harness, at p50 and p99, idle and under the load profile; the quality lines
stay absolute.

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

**Verdict: the amended gate is not met.** The quality lines pass. Playback
jitter is no worse at p50 and borderline at p99 (the engine path sits in the
two-refresh mode more often). Drops under load are within run-to-run spread
in the latest runs but worse over all runs (median 7 against 4 a minute).
Pointer-to-pixel at p50 is worse than today's stand-in idle and under load,
and the engine path presents one refresh later from rAF in every run
(19.7 to 20.0 ms against 11.5 to 11.7 ms under load). The latency runs were
not repeated in this round: nothing in it changes the input or presentation
path.

## Decision needed before the Phase 2 gate

The bridge lands only as flag-gated infrastructure until one of these closes
the gap (each measured with `scripts/frame-bridge-measure.mjs`):

1. **Autosave off main** (amendment 5). Done in another lane; the
   no-autosave profile above is its effect on this harness. It does not
   close the gap by itself: the remaining drops sit in long tasks and live
   resizes, and it does nothing for the extra refresh.
2. **Engine gestures read `pointerrawupdate`.** Measured: one to two
   refreshes off engine p50 latency (23 to 29 ms idle against 33 to 36 ms),
   which brings it within today's spread idle but not under load. A Viewer
   UI change.
3. **A vsync-phase hint** (a `ViewerState` field with the viewer's rAF
   phase) so the engine's GPU work lands just after composition. Untested;
   it targets the one-refresh-later presentation, the largest consistent
   difference. A contract 2 change.
4. **C′ for playback** (amendment 5): presentation outside Chromium's
   compositor. The only option that removes the compositor-depth question;
   the most work.
5. **Measure the real viewer** instead of the stand-in before deciding: the
   stand-in is the cheapest possible renderer draw.

## Contract changes (draft, version 0)

These need the contract owner's sign-off before the merge. They are not
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

## Gaps and next steps

- The kernel's engine connection (which registers `engine.viewer` and
  decodes the viewer's port) is not in this lane. The integration test plays
  that part (`integration/page-engine.ts`).
- Engine-path gestures should read `pointerrawupdate`; the viewer's drag
  code (Viewer UI) still uses `pointermove`.
- The pool retry after a failed hand-off is covered by unit tests of its
  policy (`PoolRetry`, `Generations`) only; making a real Mach send fail
  part-way through a pool in a test needs a fault hook in pm-bridge.
- The editor camera of the 3D viewport is not exposed by its API, so the
  view override sends shading and X-ray without a camera.
- The engine renders on the clock's frame grid at an arbitrary phase to the
  display's vsync. Telling the engine the viewer's vsync phase (a
  `ViewerState` field) so its GPU work lands right after composition is the
  next thing to measure against the compositor depth above.
- A signed end-to-end run (Team ID through Electron) needs the release
  signing identity; the Team ID check is covered by Rust tests against a
  real Developer ID process.
