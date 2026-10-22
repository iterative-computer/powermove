# Viewer and settings engine consumers

The consumer lane uses the bridge's snapshot, presentation and stats contracts
(RFC 16). The flag-off compositor remains available. No consumer change removes
an unsupported reason or approves a visible difference.

## Implemented in this lane

Composition thumbnails reject results after a project, revision, engine or
refresh change. Encoding waits out interaction and disposal releases its waits.
Project thumbnails also pin save generation and playhead time before updating
saved metadata. Neither path reads the hidden legacy canvas in engine mode.

Library cards copy the actual native presentation canvas, including its current
camera, shading, sampling and cropped canvas, alongside the existing overlay.
The stage-level native canvas retains its source CSS size in the fixed-size card
so its pixels keep the overlay's zoom/pan coordinate system. Legacy and custom
canvases keep their original copy behavior, and a panel's own `library.render`
callback still controls its card after copying. Frozen viewers without a callback
use the same presented-image copy. No second render or snapshot changes the view.
The viewer version is 1.2.6. Section and Look thumbnails retain the synchronous
compatibility API already backed by engine snapshots. Frozen render APIs retain
their existing option and pixel semantics.

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
requested job-engine rendition pipeline.

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

## Required contracts still absent

- C3: all four native current/frozen light/dark card cases pass exact native
  bitmap and 2D-overlay hashes, unchanged zoom/pan and source-stage geometry.
  Two current-viewer right-view bitmap checks pass, but visual inspection finds
  the grid and orange 3D selection outline missing from the card. They are a
  separate `core/scene3d/overlay.ts` WebGL surface whose drawing buffer clears
  after presentation, outside the copied native composition bitmap. Complete
  Library chrome parity needs a capture at that overlay's draw/fence boundary
  from the 3D owner. Paired live/card evidence is retained externally. Full
  private-corpus and API 1–3 customized-fork acceptance also remains outstanding;
  the synthetic frozen fork is from the native base, not the oldest API fork.
- C4/C19: `pm-media::preview::PreviewReader` and probe filmstrip seeds exist in
  Rust, but the renderer bridge has no asset frame/poster/filmstrip request or
  response. Posters still come from `legacy/core/poster.ts`, Assets inline media
  from browser renditions. Engine peaks are integrated; native poster and
  filmstrip parity is not claimed. The media/bridge owner must expose source
  frames with source identity, interpretation, time, generation and cancellation.
- C5: no independent engine source-audio/frame monitor contract. The working
  shipped source monitor remains. See [source-preview.md](source-preview.md).
- C18: the native frame-cache lease (`pm-node/src/present/mod.rs`,
  `viewer_frame_cache`) budgets `pm-media` decode neighbourhoods. There is no
  composed preview-frame cache or renderer warm/play/clear API. `PreviewControl`
  cache-only requests prepare Blender surfaces; they do not cache composed
  frames. The existing RAM preview still uses ImageBitmaps and WebAudio and is
  not native-complete. Section 15 assigns the composed cache to Phase 4a; the
  consumer requirement cannot be met by relabeling the decode cache.
- C28: existing host conversion is functional, but the job protocol has no
  browser-rendition job/output contract. Unmarked native-only originals and
  source-audio codecs without browser support still need that media/job service.
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

Panel popout already moves live nodes in the same realm and the bridge presenter
follows `stage.ownerDocument` (DPR, color capability, resize and drawing backend).
The existing viewer/timeline/toolbar dock-only rule remains. Both refreshed native
panel-popout cases pass; broader custom-panel/frozen-fork parity remains a gate.

## Validation

The complete first default-on suite ran 691 cases: 529 passed, 112 failed and
50 existing optional-fixture cases skipped, with no quarantine exclusions. It
used the earlier app build; it is not final-HEAD acceptance. The refreshed native
consumer gate ran 99 cases: 78 passed and 21 failed. After correcting test
fixtures and active-evaluator probes, the four native Library cases and six
focused native ports passed. This is targeted follow-up evidence, not a second
complete native-suite claim. The complete default-off suite is recorded in the
consumer section of [e2e-ledger.md](e2e-ledger.md).

The refreshed native transport gate passed 4/4, including the 150-bucket waveform,
engine mixer levels, scrub grains and J/K/L audio-clock presentation. Worst
measured J/K/L frame-clock error was 3.701 ms, with zero frames outside 10 ms.
All three mixer cases passed; retune, master, mute and solo use fresh native
levels and require zero legacy voices. Restored-project audio remains positive
through quality changes after the meter's stale interval, while the separate
native-pressure and composed RAM-preview gaps remain asserted.

The 1,000-layer native profile presented 72 playback frames over 2.4 seconds;
95th-percentile presentation gap was 35.7 ms. Engine rolling mean frame costs
warmed from 40.293 ms to 9.291 ms. These are separate measurements, not per-frame
render-duration percentiles. Graph sampling observed 1,086 cold and 2 warm
native evaluator calls. The project, quality and inspector assertions passed.

Final app build and root typecheck pass (one existing IconCropper accessibility
warning). All 22 changed E2E files typecheck independently. Focus regression
units pass 92/92; presented-card/render transport units pass 37/37. Protocol
unit, interop and wire gates pass 111 cases, with the existing generator
benchmark ignored, and the native all-target clippy gate passes with warnings
denied. The rebuilt WASM packed-model path passes its six focused tests.
Screenshots, logs, traces and raw JSON remain outside Git.
