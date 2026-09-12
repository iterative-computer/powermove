# Large project rendering performance

Measured September 12, 2026 against an isolated copy of the 670-layer “Powermove ad” project: 588 shapes, 69 text layers, 9 images, 3 audio layers, and 1 group. The original project and media were not edited by this investigation.

## Changes

- Build matte-source membership once per composition render instead of scanning every layer for every layer. The lookup is rebuilt for each pass, including nested compositions and matte passes, so edits cannot leave stale membership.
- Let uploaded shape/text textures survive eviction of their CPU canvases. The existing evaluated raster signature identifies their pixels and geometry. Ordinary bitmap consumers still get a canvas; anchored typography keeps its measurement path. Explicit source invalidation clears both caches.
- Replace the 96-entry raster limit with a 4,096-entry guard while retaining byte budgets. GPU textures also have a 4,096-entry guard, and cache maintenance avoids sorting entries when no eviction is needed. CPU and GPU memory pressure continue to release their respective resources.

## Measurements

Built Electron app, disposable user profile, 25 frame samples with 50 ms gaps to allow normal idle cache maintenance. Times measure synchronous preview rendering, not end-to-end playback FPS. Both baseline and optimized runs used the same copied project and CPU profiler. Values vary with machine load.

| Measurement | Before | After |
| --- | ---: | ---: |
| 1280×720 median redraw | 13.4 ms | 2.4 ms |
| 1280×720 p95 redraw | 20.4 ms | 5.0 ms |
| 1280×720 canvas allocations | 3,549 | 0 |
| 1280×720 texture uploads | 2,731 | 1 |
| 1920×1080 median redraw | 7.7 ms | 3.0 ms |
| 1920×1080 p95 redraw | 62.6 ms | 6.1 ms |
| 1920×1080 canvas allocations | 2,730 | 336 |
| 1920×1080 texture uploads | 2,325 | 365 |

Allocation/upload counts include the final capture redraw. Preview median improved approximately 5.6×; full-resolution median improved approximately 2.6×. Full resolution still needs some cache turnover because its source bitmaps exceed the existing budgets. No rendering-quality reduction or budget increase was used.

Decoded RGBA bytes matched exactly before/after at both output sizes. Selection plus explicit timeline/overlay redraw work on the copied project took approximately 1–3 ms per sample. The clean copied project was mostly idle when paused. The status bar's paused redraw count is not a playback benchmark.

## Validation

- Build succeeded.
- 49 unit tests passed across rasterization, compositor behavior, memory management, animation, 3D groups, and font anchoring.
- Four Electron regression tests passed: 670-layer texture reuse and CPU/GPU eviction recovery; content edits and Undo; animated text/shape rendering at three export sizes; tiny-texture count limits; alpha/inverted mattes, detachment, Undo, and nested compositions.
- Final benchmark runs passed with no renderer page errors.
- Full typecheck still reports the same 15 pre-existing errors in seven unrelated control/layout/viewer/test files; none are in this change. This is targeted regression evidence, not a claim that the entire application is defect-free.

Reproduce with a local JSON project copy (either a raw project or the stored `{ v, proj }` wrapper):

```sh
PM_PERF_PROJECT=/absolute/path/project-copy.json PM_PERF_WIDTH=1280 \
  bun run --cwd apps/desktop test:e2e project-performance-benchmark.spec.ts --output=/tmp/powermove-perf
bun run --cwd apps/desktop test:e2e large-project-performance.spec.ts
bun run --cwd apps/desktop test src/renderer/src/legacy/gl src/renderer/src/legacy/core/memory.test.ts src/renderer/src/legacy/core/groups-3d.test.ts src/renderer/src/legacy/core/font-anchor.test.ts src/renderer/src/legacy/core/anim.test.ts
```

The opt-in benchmark saves metrics, CPU profiles, composition pixels, and an editor screenshot in its Playwright output directory. Set `PM_PERF_WIDTH=1920` for the full-resolution comparison. The user's project data is not included in the repository.

## Zoom follow-up

The user then reproduced lag while zooming. Profiling the same copied project at 200% showed 856,005,228 bytes of CPU raster cache content: the composition canvas was cropped to the viewport, but its shape sources were still rasterized in full.

The renderer now plans a conservative visible source window for ordinary 2D primitive shapes. Fully offscreen primitives are skipped. Uniform rectangle interiors use their fill directly; partially visible shapes rasterize on the original source pixel grid into a bounded window. Source windows use 256-pixel buckets to reduce churn during small pans. Shape selection bounds are calculated from geometry without creating a bitmap. Paths, text, effects, masks, mattes, 3D content, and export keep their existing source rendering paths.

At 200% in the copied project, raster bytes fell to 87,236,420 (approximately 90% less), and draw calls fell from 658 to 354. The project image matched the reference RGBA bytes exactly at 50%, 100%, and 200%. A 12-step zoom/pan sweep had a worst synchronous step of 35.1 ms in the reference run and approximately 17.9 ms in the final isolated run. These are short local measurements, not a guaranteed frame rate; other loaded runs showed occasional timing outliers. Both centered and edge views were exercised at 400% and 800% without restoring the giant full-source allocations.

Validation: 55 unit tests passed; seven Electron tests passed including the copied-project benchmark and the earlier large-project regressions. New image checks cover 60 primitive/rotation/pan combinations, rounded corners, strokes, reflected scales, selection bounds, fill edits, Undo and export. The synthetic edge cases differ by at most one 8-bit channel level in a small number of antialiased pixels (142 channels out of 307,200 in the largest observed case); there is no quality downsampling. The real project comparison is exact. Build succeeds; full typecheck retains the same 15 unrelated pre-existing errors.

```sh
PM_PERF_PROJECT=/absolute/path/project-copy.json \
  bun run --cwd apps/desktop test:e2e zoom-performance-benchmark.spec.ts --output=/tmp/powermove-zoom
# Same renderer with full source bitmaps for the reference comparison:
PM_PERF_PROJECT=/absolute/path/project-copy.json PM_REFERENCE_SOURCES=1 \
  bun run --cwd apps/desktop test:e2e zoom-performance-benchmark.spec.ts --output=/tmp/powermove-zoom-reference
bun run --cwd apps/desktop test:e2e zoom-raster.spec.ts large-project-performance.spec.ts
```

### Follow-up: sustained 800% panning on Retina

The short DPR-1 zoom checks above did not expose the remaining interaction stall. A copied-project test at 1728×1117 with DPR 2 now pans for 60 frames at 800%, both at the center and near the upper-left content. It records layout, GL submission, a `gl.finish()` call, and time until the next animation callback. Chromium's `finish()` returns almost immediately here; it is not treated as reliable standalone GPU-completion timing. The animation callback measurements expose delays missed by the synchronous submission timer.

Before this follow-up, all 69 ordinary text sources were still rasterized at high density even when offscreen, repeatedly overflowing the raster/texture budgets. The 95th-percentile time from layout through the next animation callback was 153.3 ms in the center and 144.6 ms near the edge. Synchronous work alone was much smaller (median 6.7 ms), explaining why the earlier numbers underestimated perceived lag.

The compositor now conservatively excludes offscreen ordinary text before creating its bitmap. Font geometry is measured independently of raster density and cached in a bounded 512-entry metadata cache, invalidated alongside the existing font/bitmap caches. Visible text retains its original full source bitmap: experimentally cropping text changed a few glyph-edge pixels, so that approach was discarded. Animated/styled glyphs, typography anchoring, masks, effects, mattes, 3D layers, and exports retain their existing rendering paths. Memory budgets and preview quality are unchanged.

In the final matching test, 95th-percentile animation callback waits were **14.6 ms / 9.2 ms** (center / edge), and median synchronous work was 2.7 ms / 2.0 ms. Across the 60 samples, text raster calls fell from approximately 4,140 to 283 / 134. A separate 60-event test of the actual wheel handler and normal scheduled renderer moved the view by (-420, -180) at 800%, rendered 61 frames, and had animation intervals below 10 ms in that run. These are local measurements on this project, not a universal frame-rate guarantee.

Validation: 38 focused unit tests and 10 Electron tests passed (eight regressions plus two copied-project benchmarks). All 72 new text comparisons were pixel-exact, covering 16× raster density, left/center/right alignment, wrapped and multiline text, italics, tracking, rotations, reflected scale, fractional pans, and offscreen layers. Earlier project captures remain byte-identical at 50%, 100%, and 200%. Editing, Undo, font anchoring, animation, scaled exports, matte changes, nested compositions, and cache eviction checks pass. Build succeeds; full typecheck still reports the same 15 pre-existing errors in seven unrelated files.

```sh
PM_PERF_PROJECT=/absolute/path/project-copy.json \
  bun run --cwd apps/desktop test:e2e high-zoom-performance.spec.ts --output=/tmp/powermove-800
bun run --cwd apps/desktop test:e2e zoom-raster.spec.ts large-project-performance.spec.ts font-anchor.spec.ts
```

### Follow-up: distinguish playback FPS from paused redraws

The engine's old FPS sample counted demand-driven paused redraws and divided by elapsed time that included idle periods. It also retained the last sampled number indefinitely while paused. This can show single-digit FPS even though no animation is running, or show the previous playback session's FPS after playback stops. Two deterministic engine tests reproduced these failures before the fix.

FPS now samples only during playback, resets its sampling window when playback starts, and clears on pause. The status bar explicitly shows `Playback paused`, `Measuring FPS…`, measured preview FPS, `Preparing preview`, or `Cached preview` as appropriate. Cached preview does not display the unrelated live renderer's stale FPS. The render-time field remains available.

The separate playback benchmark exercises the normal engine for four seconds per view at DPR 2, with automatic quality disabled and quality fixed at 1. Before the counter fix it measured about 117–120 preview redraws/s at 800% (center and edge), with a 95th-percentile interval around 10.7 ms; paused readouts were demonstrably stale. These are preview redraws, not distinct authored animation frames (the composition is 30 fps). This follow-up fixes measurement/presentation; it does not claim an additional rendering speedup or rule out a different interaction-specific stall.

Validation: 32 engine, shell, contribution, and runtime tests pass, including pause/resume sampling and status-state transitions. The opt-in Electron playback benchmark passes and verifies paused FPS is zero and the live status is `Playback paused` after playback. The final run measured 117.4 / 105.5 preview redraws per second at 800% (center / edge), with 95th-percentile intervals of 10.1 / 13.9 ms. This variance reinforces that the counter fix is not a rendering-speed claim.

```sh
PM_PERF_PROJECT=/absolute/path/project-copy.json \
  bun run --cwd apps/desktop test:e2e playback-performance-benchmark.spec.ts --output=/tmp/powermove-playback
```

### Follow-up: reproduce and fix visible gesture stuttering

The user clarified that the picture visibly stutters. The FPS-counter correction did not address that, and fast GL submission timings were insufficient evidence of smooth presentation.

A new presentation regression test reproduced a separate visual bug: with the renderer temporarily unavailable, a 32-pixel horizontal / -24-pixel vertical pan moved the displayed canvas by **(0, 0)**. Each layout moved the crop origin along with the composition, pinning the old picture to the screen until a new bitmap arrived. The viewport's existing 128-CSS-pixel overscan is now retained while it covers a pan. When a new crop is required, the viewer keeps the presented picture in its own source coordinates and commits replacement geometry with the GL presentation. The same mechanism scales the presented picture immediately during zoom. Tests check buffer-refresh and zoom transitions for less than one CSS pixel of visual drift.

The expanded copied-project test also exercises 72 pinch events around 668–800% over the upper-left UI content at DPR 2. This exposed real frame spikes: an initial run had median 11 ms, p95 53.6 ms, maximum 62.3 ms. Instrumentation confirmed that the canvas did not resize during these events. Sources that already hit the 8192-pixel bitmap limit were nevertheless keyed by requested zoom density, causing redundant rasterization/uploads. Shape and ordinary-text keys now use effective density; paths and animated text retain their prior keys. This reduced p95 to 27.2 ms in the intermediate run.

Paused zoom gestures now immediately transform valid presented pixels while leaving a full-quality redraw pending. The engine refines after 80 ms without another zoom event, or sooner if the visible region leaves the presented buffer. This applies only to the safe 2D viewport path, with unchanged project identity, animation revision, time, and quality. Playback, content edits, and scrubbing bypass the delay. It does not lower the chosen preview resolution or change export rendering. The final matching pinch run measured median 8.3 ms, p95 9.2 ms, maximum 33.1 ms; the remaining maximum is reported rather than hidden. These local timings do not guarantee every interaction on every project will be smooth.

Validation: 83 focused unit tests and 15 Electron regressions pass, plus the copied-project gesture benchmark. New checks prove immediate pan/zoom presentation with the renderer blocked, stable buffer replacement, deferred redraw completion, exact settled pixels after fresh rasterization, unchanged quality, and immediate handling of edits/time changes. Existing pixel, cache eviction, Undo, export, native wheel/pinch/middle-drag, text editing, recovery, and saved layout checks pass. Viewer recovery fixtures now explicitly open a project: the previous empty-profile Projects overlay intercepted native input and caused unrelated test failures. Full typecheck retains the previously documented unrelated failures.

```sh
bun run --cwd apps/desktop test:e2e pan-presentation.spec.ts zoom-raster.spec.ts large-project-performance.spec.ts viewer-recovery.spec.ts
PM_PERF_PROJECT=/absolute/path/project-copy.json \
  bun run --cwd apps/desktop test:e2e high-zoom-performance.spec.ts --output=/tmp/powermove-motion
```

### Follow-up: panning the user's current timeline and waveform region

Captured the live camera before editing: zoom 4.371588852276498, pan [298.525348951279, -471.9744386396628], time 0, quality 1, DPR 2, viewer 681×502 CSS pixels, window 1448×949. The copied-project benchmark matches that camera and panel size (Chromium reports 501 pixels of laid-out client height) and pans back and forth using fractional wheel deltas. It uses the normal engine, including autosave.

The initial 180-event run measured median 8.3 ms, p95 9.7 ms, maximum 318.3 ms, with 182 preview renders. Thus the good median hid two large, visible stalls. Avoiding redundant preview renders reduced submissions to 13 but did not remove the stalls. CPU profiling and resize instrumentation identified a background project-thumbnail capture: it resized the live 1874×1514 preview to 320×180 and back. One profiled restoration alone blocked for 590 ms. Offscreen rendering eliminated those resizes, but synchronous thumbnail GPU readback still blocked a gesture for about 412 ms.

Changes:

- Covered, paused wheel/hand pans now move the existing picture without submitting the layer stack again. The optimization only suppresses that pan's own redraw request. Already-pending draws, edits, media/font invalidations, time changes, playback and newly exposed regions still render normally.
- `renderFrameTo` now captures through an offscreen RGBA8 presentation target, preserving the live canvas dimensions, displayed pixels, viewport metadata and retained framebuffer. Row order and opaque black-backed transparency match the existing capture API. Failure cleanup restores GL binding and releases temporary resources.
- Autosave continues to persist the document during navigation. Thumbnail capture retries after navigation has been quiet for at least 250 ms, and also waits for playback/export/capture to finish. Retries check project identity so closing or switching the document cannot write an old thumbnail into another project.

The final matching run measured median 8.3 ms, p95 12.0 ms, maximum 32.3 ms, **10 preview renders instead of 182**, and **zero live-canvas resizes**. Another post-fix run had maximum 32.2 ms. These measurements establish removal of the reproduced hundreds-of-milliseconds stalls, not a promise that every frame will hit 120 Hz. The native live-app hand drag was checked and reversed, leaving Selection active at the captured camera.

Validation: 84 focused unit tests pass. Across the regression runs, 26 Electron checks pass, including exact capture pixel equivalence for translucent artwork over solid/transparent backgrounds, capture failure preservation, deferred thumbnail completion without delaying project persistence, fractional pan reuse, pending external redraws, edits, scrubbing, viewer recovery, source pixel comparisons, video import/relaunch/capture, audio playback, grouping and Undo. Five older import/group tests initially stopped at the empty-profile Projects screen; explicitly opening their fixture project fixes their setup and all nine capture/import/group tests pass. Both copied-project region and 800% gesture benchmarks pass. Full typecheck still reports the same 15 existing errors in seven unrelated files; no new errors were introduced.

```sh
PM_PERF_PROJECT=/absolute/path/project-copy.json \
  bun run --cwd apps/desktop test:e2e region-pan-performance.spec.ts high-zoom-performance.spec.ts
bun run --cwd apps/desktop test:e2e agent-canvas-capture.spec.ts pan-presentation.spec.ts viewer-recovery.spec.ts zoom-raster.spec.ts large-project-performance.spec.ts import-mp4.spec.ts adjustment-layer.spec.ts precomp.spec.ts
```

### Follow-up: media card and window shadow at 312.2%

Captured the next reported camera: zoom 3.1223372814366486, pan [2142.9624206302624, 1263.2672596867947], time 0, full quality, DPR 2. At this view the same normal-engine pan test reproduced p95 31.1 ms and a 106.4 ms maximum. Freezing renderer submissions removed the recurring spikes, isolating the remaining cost to refreshed composition frames rather than input handling. The document builds its window shadow from 32 large translucent rounded rectangles, much of which is subsequently covered by opaque UI plates.

The preview now identifies a conservative opaque interior of axis-aligned filled rectangles above each layer. It renders lower layers only in the uncovered scissor regions and clips their source rasterization to those regions. Rounded corners, strokes and sampling margins are excluded from the opaque interior. The optimization is disabled for export, nested compositions, solo rendering, 3D, effects, masks, mattes, transitions, motion blur, non-normal blends and unsupported layer types. It does not modify project layers or reduce preview resolution.

The matching post-fix runs measured p95 13.9 / 13.7 ms, with maximum 55.9 / 58.1 ms. Median remained 8.3 ms. Occasional pauses remain, so this is a measured improvement, not a universal smoothness guarantee. A smaller crop-bucket experiment was slower and was discarded.

Validation: 24 new pixel comparisons spanning fractional pan positions, rounded/stroked occluders, rotation and opacity changes are exact. Both actual copied-project regions also match the renderer with occlusion disabled pixel-for-pixel. All 30 selected Electron regressions pass, including error presentation, asynchronous modal recovery, message checkpoints, agent capture, autosave, pan/zoom, raster caching, video import/relaunch/audio playback, adjustment layers, grouping and Undo. Three copied-project pan/pinch benchmarks pass. The full unit run has 1,771 passing, 18 failing and one skipped test; the exact same 18 test names fail in a separate unchanged HEAD checkout (50 other tests in those eight files pass). Existing unrelated renderer typecheck errors remain documented above.

Before committing, the exact staged patch was applied to a separate HEAD checkout, excluding unrelated local control/layout edits. Its production build, 270 focused unit tests and 13 Electron capture/error/presentation/pixel checks pass. The two private-project benchmarks are opt-in and were run separately in the working checkout as described above. The staged renderer typecheck reports the same 15 existing errors and no warnings.

### Follow-up: outward panning beside the top edge at 251.2%

Captured the latest reported camera: zoom 2.51184505912942, pan [1625.9500044340953, 966.7594946944603], time 0, quality 1, DPR 2. Extending the copied-project benchmark to pan outward first reproduced 164 live-canvas resizes and 166 scene renders in 180 wheel events. The overscan margins were independently clipped to the composition, so moving toward an edge changed the backing-buffer dimensions on almost every event. Earlier inward-first cases missed this path.

The viewport now keeps its allocation bounded by the stage plus overscan and shifts the source window inward at composition edges. Its dimensions stay fixed throughout a pan, including when only part of the composition is visible. Existing retained-image presentation, selected quality, export and out-of-view recovery behavior remain intact.

The matching reproduction dropped from **164 resizes to zero** and from **166 scene renders to 10**. Median / p95 frame intervals changed from 12.9 / 26.1 ms to 8.3 / 15.2 ms; total CPU rendering time fell from 495.2 to 48.6 ms. Maximum interval remained approximately 47 ms (46.5 before, 46.7 after), so this removes the repeated allocation stalls without claiming every occasional hitch is eliminated.

Validation: 82 focused unit tests and 21 Electron regressions pass. New coverage checks all four edges at three zoom levels, two device scales and two preview qualities, plus native middle-button dragging at every edge with zero canvas resizes. Existing pixel comparisons, edits/Undo/export, capture, deferred thumbnails, raster caches, nested/matte passes, pan/pinch presentation, text editing, saved layouts and view recovery pass. The copied-project outward benchmark passes with exact pixels against the reference renderer. The private project is not included in the repository.
