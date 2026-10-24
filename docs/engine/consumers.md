# Viewer and settings engine consumers

The editor surfaces that show engine pixels or engine facts outside the live
viewer: Library cards, composition and project thumbnails, the status bar and
performance panel, waveforms, export preparation and web export. They use the
bridge's snapshot, presentation and stats contracts (RFC 16). With Use New
Engine off, today's compositor path remains. Rendering cleanups follow
QUALITY-1 and are recorded in the [quality ledger](quality-ledger.md).

## Snapshots and the viewer pin

A UI capture is an idle `EngineRenderTransport.thumbnails` snapshot, never a
readback of the hidden legacy canvas and never a second presenter. Captures
cannot change the live viewer's state.

`SnapshotRequest.viewer` (`EngineSnapshotOptions.viewer`) pins one presented
frame: `{ view: ViewOverride | null, canvas: ViewerCanvas, viewSeq }`.

- **Source:** the Library pins `presentation.stateFor(latest.viewSeq)` for
  cropped views and for compositions with 3D objects. Full 2D views need no
  pin.
- **Validation** happens in main (`snapshots.ts`) and in `pm-protocol`:
  - the target is a frame, and `outputSize` equals the canvas backing size;
  - the top-level `viewSeq` agrees with the pin;
  - the crop and CSS box are finite and positive, and the camera and
    projection are finite;
  - X-ray requires draft shading.
- **What renders** (`run_snapshot`): the live viewer's canvas, on the same
  grid as live placement (`compose::canvas_render_grid`). It uses
  `preview_canvas`, paused motion blur and `Final` sampling, with the pinned
  editor camera and the live top-down colour conversion. The target is the
  F16 display working texture, as the live viewer's is, even when project
  intermediates use Float32. The result is the raw canvas before stage
  scaling, and reports the pinned sequence.
- **Without a pin**, export and agent captures keep their own scale,
  precision and purpose.
- **Immutability:** the transport deep-copies the pin before any wait or
  retry. The Library rejects results whose revision, time, view sequence or
  editor camera changed.

## Library cards

A card clones the live panel and replaces its canvases' bitmaps.

- **Paused views** request a snapshot at today's original canvas backing
  size: pinned for cropped and 3D views, unpinned for full 2D.
  - The clone keeps the live canvas's CSS box and the original composition
    canvas, and removes the cloned presentation layer.
  - A panel's own `library.render` runs after the pixels arrive.
- **Freshness:** the presented frame must match the snapshot's revision and
  time first. A stale paused presentation gets one exact seek per capture
  target.
- **Retries:** transient session, resync and timeout errors retry at most
  three times, 250 ms apart, while the capture is still current. Permanent
  errors propagate, with no resampled fallback.
- **Playback** copies the presented image and never waits for a capture.
  Opening the Library never pauses playback.
- **No gizmo:** cards never show the 3D transform gizmo. It is an editor
  overlay, not content, and its WebGL buffer is undefined after compositing,
  so its canvas is skipped.
- **Pre-engine forks:** a fork or override of the viewer (apiVersion 1,
  `render.gl.init` on its own canvas) gets an engine presentation bound to
  that canvas (`canvas-binding.ts`). It runs in the same presentation worker
  as the built-in viewer, so its canvas reads back and keeps playback
  cadence. It receives the same editor camera through `canvas-view.ts`,
  editor state only, with no native query.

## Other consumers

- **Composition and project thumbnails:** they reject results after a
  project, revision, engine or refresh change. Encoding waits out
  interaction. Project thumbnails also pin save generation and playhead time.
- **Status and performance:** native counters from startup, with zeros
  before the first stats. A session resync or restart clears the old view.
  Audio health is in [consumer-audio.md](consumer-audio.md).
- **Waveforms:** each request carries a generation that the decoder echoes,
  including cached and failed answers.
  - Resync replays outstanding work.
  - Relinking clears the old envelope.
  - Late answers are rejected.
- **Export preparation:** sheet exports and queue additions show "Preparing
  media…" while IndexedDB originals move to the media store. Cancellation is
  per operation: a queue addition never cancels or clears the foreground
  export (`X.busy`, `X.cancel`).
- **Web export:** a missing PNG or WebM browser rendition is generated from
  the frozen original through the host converter, then persisted by content
  hash. Renditions on screen are captured before the first await, so a
  relink cannot substitute bytes.
- **Focus in dialogs:** a pointer-clicked button inside a modal sheet or a
  focusable dialog releases focus to that dialog, keeping its Escape and Tab
  handling away from the editor's shortcuts. Keyboard activation keeps focus
  on the button.

## Gates

`e2e/library-card-parity.spec.ts` (C3) launches the app with the engine off,
then on, for the same scene and viewer. It compares the whole card across four
viewers (current, a fork of it, and the shipped 1.0.8 and 1.0.18 forks) in both
themes.

- **Fresh raster:** before each capture, the clone's layers and the shelf
  ledge are hidden for one frame. Chromium tiles keep their raster history,
  so otherwise identical DOM differed between launches.
- **2D cases** (Fit and 200%) must match exactly.
- **3D cases** (camera view and Editor view) use the reference rule:
  - pixels away from edges must be exact;
  - a differing edge pixel (3×3 span above 8 levels) must be, per channel,
    no further than today's from the engine's 4× supersampled render of the
    same pin.
- Editor-view cards currently fail that rule (quality ledger C3-Q2, owned by
  the 3D lane).

`e2e/consumer-matrix.spec.ts` covers C1, C2, C17 and C21, and `mixer.spec.ts`
covers C6, across the same viewers and themes.

## Owned elsewhere

- C4/C19 (Assets posters, filmstrips and inline media), C5 (the source
  monitor) and C28 (job-engine web renditions): `mediaframes`.
- C18 RAM preview: Phase 4a.
- C29 web player and C8/C20 text and fonts: Phase 5c and the text and
  renderer owners.
- The undo hand-over in the native document mirror: document host.
- App quit and process lifetime: bridge (`main/index.ts`).
