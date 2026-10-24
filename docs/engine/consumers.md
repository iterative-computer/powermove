# Viewer and settings engine consumers

The consumer lane uses the bridge's snapshot, presentation and stats contracts
(RFC 16). The flag-off compositor remains available. No consumer change removes
an unsupported reason. QUALITY-1 permits a rendering cleanup only with a
high-precision reference proving the engine is no worse at every sample;
authored and UI intent remain exact. No C3 residual is approved; the one
open raw-frame row is in the [quality ledger](quality-ledger.md).

The orchestrator's current scope decisions (2026-10-23) transfer C4/C19
(Assets posters, filmstrips and inline media), C5 (the source monitor), and
C28 (job-engine web renditions) to the new `mediaframes` lane. Their missing
engine contracts are outside this consumer lane. C18 RAM preview belongs to
Phase 4a and is done on `feat/ne-phase4a`; that completion is not claimed to
be merged into this branch. Earlier C18 failures below remain historical
run results, with ownership transferred to Phase 4a.

## Implemented in this lane

Composition thumbnails reject results after a project, revision, engine or
refresh change. Encoding waits out interaction and disposal releases its waits.
Project thumbnails also pin save generation and playhead time before updating
saved metadata. Neither path reads the hidden legacy canvas in engine mode.

Library cards of paused full-composition 2D views request an idle snapshot through
`EngineRenderTransport.thumbnails` at today's canvas backing size, before
presentation scales it into the stage. The clone keeps the live canvas's CSS
box and overlay, restores its original composition canvas, and removes its
cloned presentation layer. A bound presenter in a pre-engine fork takes its backing
size from the original canvas immediately before it. Captures reject document,
time, view, engine, source-panel and Library-generation changes; stale captures
retry while the card is still current. The presented frame must match the
snapshot revision and time before its overlay is cloned. A panel's own
`library.render` still runs after the pixels arrive and controls its card.
If a paused presentation has the wrong time or revision, the card requests
one exact paused seek per capture target before re-cloning. An unchanged
layout alone does not request a frame. The readiness diagnostic observed UI
time 0 with a frame still at 0.9 seconds, at the same revision; the refresh
preserves the strict frame/overlay checks instead of accepting stale pixels.
Transient session, resync and timeout errors retry at most three attempts,
250 ms apart, while the pinned capture is still current; readiness waits for
the result. Permanent errors propagate, without a resampled fallback.

Paused cropped and 3D views use the same backing-size snapshot path with a
pinned editor view and composition crop. Playback retains the native
presented-image copy and never waits for an idle capture; opening the
Library keeps playback running. The pinned contract and remaining C3
qualification are specified below.
Section and Look thumbnails retain the synchronous compatibility API already
backed by engine snapshots. Frozen render APIs retain their existing option and
pixel semantics.

A user's fork or override of a viewer from before the engine (apiVersion 1,
`render.gl.init` on its own canvas) gets an engine presentation bound to that
canvas (`engine-connection/canvas-binding.ts`). That bound presentation now
runs in the same presentation worker as the built-in viewer's
(`engine-connection/install.ts`). Before, it painted on the main thread
without a preserved drawing buffer, so its canvas read back empty after
compositing and the Library card of a shipped-viewer fork showed only the
selection overlay with the engine on (the composition was missing; engine off
showed it). The worker also gives forks the built-in viewer's playback
cadence. If the browser cannot transfer a canvas to a worker (it always can
in the shipped Electron), both the built-in and the bound presentation fall
back to the main thread, where a card copy would again be empty.

Status/performance use native counters from startup, with zeros before the first
stats, rather than stale GL counts. Session resync/restart clears the old view.
Audio health is described in [consumer-audio.md](consumer-audio.md).

Waveforms carry a request generation echoed by the native decoder, including
cached and failed answers. Document resync replays outstanding work. Source
replacement or relink clears the old envelope and requests the new one; late
answers are rejected. Persisted identity also guards restored live assets that
lack hashes. Engine peaks invalidate Assets panel bars as well as the timeline. Existing meter, monitor solo and
transport adapters remain the only playback audio path with the flag on.

Sheet exports and queue additions show “Preparing media…” in the existing export
progress sheet while IndexedDB originals migrate to the media store. Progress
advances per upload chunk. Cancel prevents submission once the migration finishes;
closing the sheet releases its animation and preserves the cancellation result.
Imports keep their existing acknowledgement barrier without a progress sheet.

Web export generates a missing PNG or WebM browser rendition from the frozen
original reference through the existing host converter, persists it by content
hash, and releases the converter's read token. It first preserves a kept or
matching browser rendition, freezing a live URL before the first await so a
relink cannot substitute its bytes. Conversion tries main's source reference
before reading/uploading an original. The engine-off browser-copy path and its
missing-copy error remain, with no new persistence. This is not yet the
requested job-engine rendition pipeline, which now belongs to `mediaframes`
(C28) rather than this lane.

Pointer actions inside a focusable dialog release the clicked button to that
dialog, preserving its Escape and Tab handling instead of exposing the editor’s
shortcuts. Keyboard activation keeps its normal focus. The eyedropper restores
focus if disabling its button left the still-open picker on the page body.
These are small shared-control edits outside the consumer folders. Attachment
preview tests distinguish keyboard focus restoration from the existing rule
that pointer-clicked buttons do not stay focused.

The root typecheck generates cloud bindings before declaration emit and includes
the node project's actual transitive source dependencies. It does not disable
compiler checks. Onboarding's SVG and vendored animation stay unchanged in Phase
3c; replacement remains Phase 5c. welcome.ts changes are type annotations only.

### Pre-existing consumer UI failures closed (2026-10-22)

These Phase 0 ledger failures had the consumer lane as owner and failed with
the engine on and off. Each is a product fix or a port to UI that shipped
after the spec was written; no assertion was loosened.

- The selected timeline mode segment drew a 1 px ring that the flat-control
  rule forbids. It now matches the Library and Store segmented controls
  (raised fill, no ring). Timeline 1.1.3.
- The Library screen sat 32 px from the top in short windows, under the
  44 px titlebar drag area. The overlay now pads 44 px top and bottom, which
  leaves tall windows exactly where they were (the centering is symmetric).
- The canvas layer menu lacked the timeline's Import Captions item. Captions
  contributes it to `viewer:context` as well, so both menus are identical.
- Test ports: the unified color picker closes from its header (no OK button);
  the Library is inspected once `data-previews="ready"`, and controls the page
  replaced between Playwright's query and callback are skipped; extension
  parameters inside the inspector's More disclosure are opened as a user
  would; nested timeline rows are aimed after the group's rows scroll into
  view; the keyframe menu's item runs after its close blink, so the eased
  values are polled; the path-mask stage status is read once the engine frame
  is revealed (the canvas turns visible with the first frame, the starting
  line clears at reveal).
- The Library card spec compares two fresh drawings of the live overlay.
  With the engine on, a presented frame at the same revision redraws the
  overlay after the spec's own readbacks; the old and new drawings of an
  identical state (same frame, revision, zoom, pan and handle geometry)
  differed only in edge coverage (alpha 0x66 vs 0x80 on the outline), the
  signature of a 2D canvas moved to another rasterizer by readbacks.

The viewer is 1.2.10: merging `feat/native-engine` again brought the 3D
lane's surface-epoch presentation changes into the viewer's tree (1.2.9; the
version lock failed `check:builtins` until the bump), and the viewer now tags
its presentation canvas with today's canvas box (1.2.10, below).

### Review fixes (2026-10-22)

- Library cards for forks of shipped viewers: fixed as above. The fork matrix
  (`e2e/helpers/viewer-fork.ts`) copies the viewers of released builds by
  commit: 1.0.8 (desktop v1.1.0, `ded961354`) and 1.0.18 (desktop v1.2.0,
  `07ed5fa17`), beside the engine-era viewer (`8d772dac8`, a fork made after
  this update) and the current one.
- The fork of viewer 1.0.18 needs main's kernel API (`scene3d.getMode` and
  the gizmo/navigation/view calls GizmoToolbar.svelte uses). It activates
  since the main merge reached this branch (2026-10-23).
- Library cards of the current viewer differed from today's along the
  composition's edge. The built-in viewer's engine presentation covers the
  whole stage with the composition drawn inside it; scaled into the card, the
  composition edge was filtered inside that bitmap, where today's card shows
  the edge of its own canvas element. Measured on/off at 200% (panned): 1,063
  differing pixels, a half-covered line, up to 98 levels in light and 8 in
  dark. The viewer now tags its presentation canvas with today's canvas box
  for the frame on screen (`data-todays-canvas`: stage CSS box and backing
  size, from the frame's `ViewerState.canvas`), and the card copies that
  region at today's box and backing size (`library/viewer-preview.ts`). The
  same cases are now identical on and off (`e2e/library-card-parity.spec.ts`).
- Panel windows got `data-theme="undefined"` and `data-density="undefined"` in
  the light theme and default density (`layout/popout.ts` assigned `undefined`
  to the dataset). Absence is now copied as absence.
- `settings/install.ts` imports the Settings page type from `settings/pages.ts`
  instead of keeping a copy.
- Queue and thumbnail decisions, updated by the orchestrator (2026-10-23):
  `X.addToQueue` accepts a queue addition while an export is busy, matching
  today's behaviour. The earlier "Export already running" queue rejection
  and queue writes to the foreground export's `X.busy`/`X.cancel` state are
  reverted. Queue media preparation has operation-local cancellation, so
  cancelling or finishing it does not cancel or clear the foreground export.
  The guard arrived with the preparation UI; no prior queue bug was proven.
  With the engine off, `projectThumb` still retries at the end of the
  five-second capture limit, so the saved thumbnail follows the last save
  instead of keeping an older capture. This is an accepted beta bug fix.
- Commit `11f75ca00` ("perf(3d): reuse depth prepasses") duplicates the 3D
  lane's `4fea55d37`; it leaves no tree difference from `feat/native-engine`.
- `timeline.spec.ts:55` (one failure in six engine-on runs): the spec read
  the Scale key's position before the timeline's first draw of its fixture,
  and that draw widens the gutter for the new property labels (measured 224
  to 266 px in three of three probes). When a draw landed between the read
  and the press, the press fell 42 px beside the key. The spec now aims after
  painted frames, once two frames agree; 6 of 6 repeats and the 17-case file
  pass with the engine on. No product change: the hit test always uses the
  geometry on screen.
- The graph editor spec, with the engine on, also checks the composition's
  centre on the presented viewer canvas (exactly `[96, 123, 255, 255]`), not
  only a fresh render.
- The Library card spec checks the overlay the live viewer kept while the
  Library opened by its outline position before redrawing it, so a stale
  overlay drawing is caught again; the exact hash still compares two fresh
  drawings.

### Appendix C test matrix

Each row in both themes, with the current viewer, a fork of it, and forks of
the viewers shipped in desktop v1.1.0 (1.0.8) and v1.2.0 (1.0.18), run with
the engine on and off (`e2e/helpers/viewer-fork.ts`):

- C1 composition thumbnails, C2 the saved project thumbnail, C17 status bar,
  performance panel and frame events (`frame:rendered` carries the exact
  `render.gl.previewViewport`, versions only move forward, `overlay` follows
  each frame), C21 popout: `e2e/consumer-matrix.spec.ts`.
- C2 stale captures: an engine-on case holds each project capture and checks
  that a playhead move, an edit, and a newer save each drop the capture, and
  that the retry saves the current frame (`consumer-matrix.spec.ts`).
- C3: `consumer-surfaces.spec.ts` (card geometry, overlay and picture against
  the live viewer) and `library-card-parity.spec.ts` (card on against off).
- C6: `mixer.spec.ts` "retune matrix": wav, mp3, m4a, flac, opus, mp4, mov
  and webm each sound through their strip; layer gain, a video's audio gain
  and mute, the layer and group switches, a precomp's output level and mute,
  and a relink change what plays, with gains measured as strip-level ratios
  with the engine on and no legacy voice allowed. Today's mix has no pan or
  pitch control (`legacy/core/audio.ts`, `voiceSignature` and
  `levelSignature`), so there is nothing to retune for them. Observed with the
  engine off only (not changed here): an agent `replace_media` during
  playback leaves today's audio stopped while the transport runs, until
  playback restarts; the matrix relinks while paused, as the Replace dialog
  does. Owner: agents / media.


## Pinned viewer snapshots (C3)

`SnapshotRequest.viewer` and `EngineSnapshotOptions.viewer` carry an
immutable `{ view: ViewOverride | null, canvas: ViewerCanvas, viewSeq }`.
The Library pins the presented frame's `presentation.stateFor(viewSeq)` for
cropped views and compositions with 3D objects; its original backing size
must equal `outputSize`. The engine renders the composition crop at per-axis
scale `width / crop.width`, `height / crop.height`, with the same grid helper
and `preview_canvas` rule as live presentation, and reports the pin's
sequence. Conflicting top-level and pinned sequences, invalid cameras and
invalid canvas boxes are rejected.

The raw canvas is returned before stage scaling, with paused viewer motion
blur, sampling, the editor camera and the live top-down color conversion.
Like the live viewer, the pin lands in the F16 display working texture even
when project intermediates use Float32. Export and agent captures are
unchanged without a pin. Captures never change the live viewer state or open
another presenter. The transport deep-copies the pin before waiting or
retrying; the Library rejects superseded revision, time, view sequence or
editor camera, and keeps the original cloned canvas hierarchy and
`library.render`.

Main's 3D editor (merged 2026-10-23) has a Camera view and a free Editor
view; the override carries only the editor camera
(`core/scene3d/viewport.ts`, read without any native query). Pre-engine forks
bound through `render.gl.init` receive the same camera
(`engine-connection/canvas-view.ts`), refreshed on each view change. The
viewport chrome is DOM plus the WebGL transform gizmo. Library cards never
show the gizmo: it is an editor overlay, not content. The card copy skips its
canvas, whose WebGL buffer is undefined after compositing; copying it raced
the buffer swap, so cards usually lacked the gizmo and occasionally showed it
misplaced for the card's layout.

The C3 spec recreates the card clone's compositor layers (`display: none`,
then restored) before each screenshot, with the engine off and on. Chromium
tiles keep their raster history: eight engine-off launches of the same card
differed by 1–7 pixels, up to 36 levels, at header SVG icon samples, and were
identical after that fresh raster. The gate compares the same final DOM and
pixels with no mask. Its non-camera case is the Editor view framed on the
selected cube.

2D cards must match exactly. The native 3D pass uses Metal's hardware MSAA
resolve (no WebGL resolve emulation), so 3D cards follow the reference rule:
a differing pixel must be an edge (3×3 span above 8 levels in today's card)
and, per channel, no further than today's from the engine's 4× supersampled
render of the same pinned view, written into the card's composition canvas.
Camera-view cards pass; Editor-view cards have 4–5 one-level edge pixels
further from the reference (quality ledger C3-Q2, owned by the 3D lane).

Rust tests cover wire validation, cropped pixels, the selected sequence, an
unchanged subsequent live frame, the editor camera and the display-format
pin of a Float32 project. TypeScript tests cover input validation, immutable
asynchronous retries, mismatched sequences, original backing sizes and the
bound editor camera.

## Contracts transferred out of consumers

The inventory after merging `feat/native-engine` (2026-10-22, `49cc9e41d`)
found no source-frame, poster, filmstrip or browser-rendition request in
`pm-protocol`, the renderer engine connection or the generated TypeScript;
the checked `feat/ne-mediaclose`, `feat/ne-bridge` and `feat/ne-phase4a` trees
also lacked those media contracts. The orchestrator has now re-scoped the
following rows; these are ownership boundaries, not consumer acceptance
blockers or claims that the missing contracts have landed here.

- C4/C19 — `mediaframes`: `pm-media::preview::PreviewReader` and probe
  filmstrip seeds exist in Rust, but the renderer bridge has no asset
  frame/poster/filmstrip request or response. Posters still come from
  `legacy/core/poster.ts`, Assets inline media from browser renditions. Engine
  peaks are integrated; native poster and filmstrip parity is not claimed.
  The `mediaframes` lane owns source frames with source identity,
  interpretation, time, generation and cancellation.
- C5 — `mediaframes`: no independent engine source-audio/frame monitor
  contract. The working shipped source monitor remains. See
  [source-preview.md](source-preview.md).
- C18 — Phase 4a: RAM preview is done on `feat/ne-phase4a`, outside this lane;
  this branch does not claim that completion is merged. Earlier runs here
  lacked the composed preview-frame cache and renderer warm/play/clear API:
  the native `viewer_frame_cache` lease budgeted decode neighbourhoods,
  `PreviewControl` cache-only requests prepared Blender surfaces, and the
  existing RAM preview used ImageBitmaps and WebAudio. Those observations and
  failed runs are historical, not an outstanding consumers implementation
  requirement. Phase 4a owns integration and follow-up qualification.
- C28 — `mediaframes`: existing host conversion is functional, but the job
  protocol has no browser-rendition job/output contract. Unmarked native-only
  originals and source-audio codecs without browser support need that lane's
  media/job service.

## Other owner qualifications

- Shared controls: destroying a color picker during an active preview can leave
  its edit transaction open. Cancelling during destruction can reenter document
  replacement through history rollback; the editing/controls owners must settle
  the gesture safely outside that teardown. The focus fix removes the observed
  Escape-to-deselect trigger but does not qualify arbitrary teardown.
- C29: the existing TS web player and vendored web output remain. The web-v1
  freeze, serve boundary and native player replacement belong to Phase 5c.
- C8/C20: no new text or font implementation is claimed by this lane. The
  paired viewer-canvas gate, with explicit on/off launches, records 25 image
  comparisons. Five video editing-copy/original comparisons miss the existing
  pixel bound (maximum channel errors 37–49); outside the video they pass.
  The inherited test calls these known differences; they remain acceptance
  blockers here, with paired images retained, and are not approved exceptions.
  Native raster-source diagnostics and customized font/fork setups also remain
  unqualified by this lane. Media, renderer and extension owners must close them.

- An earlier engine-on qualification found multi-selection rotate after Undo:
  `viewer-resize.spec.ts`
  resizes a selection, undoes with `PM.hist.undo()`, then rotates by 45°. On
  the press, `EngineDoc.api.worldMatrix` first answers the undone state for
  the root text layer (identity) and, a few calls later in the same press,
  the resized one (scale 1.1148, translation 502.875, 192.987), while
  `PM.ev` still reads the undone document. The rotate solves from that and
  writes scale 111.481 into the document. With the engine off the same code
  rotates exactly. The native mirror replays the resize after the undo; the
  document-host owner (engine-doc mirror and undo hand-over) must fix it.
- Session teardown: in targeted runs on a loaded machine `app.close()` hung
  until the 30 s test budget ended after the body passed (a trace shows the
  body done at 2.4 s and "Close context" until 32.4 s), with the engine on or
  off. The round 5 full runs had four (on) and one (off) worker teardowns
  over budget. Quit and process lifetime belong to
  the bridge owner (`main/index.ts`).

Panel popout already moves live nodes in the same realm and the bridge presenter
follows `stage.ownerDocument` (DPR, color capability, resize and drawing backend).
The existing viewer/timeline/toolbar dock-only rule remains. `consumer-matrix`
C21 pops out the Media panel (engine composition thumbnails keep updating in
the window, through a resize, and after return) and a user's apiVersion 1
canvas panel drawing `render.renderFrameTo` (its pixels move with it and it
draws edits there), in both themes with each viewer variant.

## Validation

Current results (2026-10-24, after merging `feat/native-engine` 84bcb32f8
with main's 3D editor and kernel API). Full ON/OFF runs, owners and load
conditions are in the [e2e ledger](e2e-ledger.md).

- C3 matrix, 32 cases (2D Fit and 200%, 3D camera and Editor view, both
  themes; current, frozen-current, shipped 1.0.8 and shipped 1.0.18
  viewers). The 1.0.18 fork now activates. With the WebGL resolve emulation
  (before its revert), all 32 matched exactly, and the 16 rendered 3D cases
  passed 48/48 across three repeats with the gizmo excluded. With Metal's
  hardware resolve, 2D cards and camera-view 3D cards pass (3D under the
  reference rule above). All eight Editor-view cards fail the reference
  rule at 4–5 one-level edge pixels (quality ledger C3-Q2, 3D lane).
- Raw 3D frame, all viewers: 16 one-code MSAA edge pixels and the C3-Q1
  midpoint tie at `(799, 448)`. No other interior pixel differs.
- Engine-off card captures are identical across eight launches after the
  fresh raster.
- Rust: `pm-protocol` 105 tests, `pm-node` snapshot contract 17 tests
  (including the pinned F16 display format of a Float32 project);
  protocol regeneration is current. Desktop typecheck has 0 errors.
- Desktop units: 7,623 passed, 17 failed in one loaded run. Seven persist
  in isolation and are integration-owned, since the lane changes none of
  their inputs: six scene3d geometry schema rejections (fixed by prune1, not
  yet on integration) and one media-path-cache test. The lane's own
  failures, picker focus and the generated writer inventory, are fixed.
