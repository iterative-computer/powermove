# Phase 2 gate

## Rerun after the gate fixes (2026-10-17)

Every item of the gate measured again on `feat/ne-regate`: the integration branch
`feat/native-engine` at `c4c1c5f5` (waves 1 to 4 through "Merge lane longgop (wave
3d)"; `feat/ne-correct`, `feat/ne-bridgefix`, `feat/ne-perf`, and `feat/ne-longgop`
had nothing left to merge) plus this lane's measurement scripts. No engine code was
changed. Same machine as the first run (M4 Max, 128 GB, macOS 27, built-in 120 Hz XDR
panel), Electron 44.0.0, release `pm_node.node` and `pm_bridge.node` built in the
worktree. Shared with other lanes: the 1-minute load average is recorded with every
measurement. Every Electron run was hidden and unfocused, one at a time under the
machine lock, `--workers=1`, no retries. `apps/desktop/scripts/gate-rerun.sh` reruns
each step (Electron steps under the lock, presentation steps only with the display
awake and the session unlocked) and records the load average before and after;
`scripts/gate-summaries.mjs` reads the harness summaries. Harness summaries of this
rerun are in `phase2-gate-measurements/*-regate.json`; raw Playwright results and logs
stay outside the repository.

| Gate line | First run (2026-10-15) | Rerun (2026-10-17 and 18) | Result | What changed |
|---|---|---|---|---|
| Layer drag, 500 layers | engine p95 46.2 idle, 125.0 under load (today 34.9, 65.7) | Idle (load 5.2): engine p50 49.8, p95 58.4, p99 58.6 ms; today 24.8, 25.4, 33.2 ms. Under load (6.3): engine 50.6, 99.9, 125.2 ms, 203 of 2,401 never presented; today 25.5, 58.8, 75.8 ms, 139 unpresented | **fail** | worse than the first run and than `perf.md`'s `perf7` (29.8 / 38.0 / 38.4 ms idle): D6 |
| Inspector edit during playback, 1,000 layers | apply and send p95 31.5 ms; 381 of 1,796 dropped | Harness (an Opacity edit every 250 ms, load 7.1): apply and send p50 11.5, p95 13.6, p99 14.6, max 16.0 ms; 0 of 1,794 frames dropped; edit to presented p50 106.7, p95 128.4 ms. Page (60 edits, load 10.3): apply and send p50 8.8, p95 13.1 ms, edit to draw p50 78.3, p95 90.8 ms, presenter drops 0 (89 frames replaced by newer revisions). Use New Engine off: harness apply p95 13.3 ms, 6 of 1,793 dropped; page p95 9.7 ms | pass | the perf lane's fixes; edit to presented is slower than `perf.md` measured (not a gate line) |
| Scrub, intra | p95 8.8 ms | ProRes 422 HQ 1080p seek to draw p50 6.3, p95 7.5, max 8.9 ms (load 11.8) | pass | |
| Scrub, long-GOP | 1080p p95 103.9; 2160p p95 347.5 ms | 1080p p50 19.4, p95 58.1 ms; 2160p p50 63.5, p95 200.1, max 210.9 ms (load 9.3 to 10.5) | pass | the long-GOP lane (`scrub.md`) |
| A/V sync ±10 ms | page 900 of 900; presentation p95 11.9 to 20.6 ms | Page: ProRes clip with audio 449 of 449 within, line minus frame start p95 3.83 ms; 2160p long-GOP 450 of 450 (the silent 500-layer row is blocked by D5). Presentation, \|offset\| p95: idle 5.9 (30 fps), 7.8 (25), 7.3 (23.976), 6.8 ms (1,000 layers), every frame but 3 within; under the gate load 10.2 (30), 10.8 (25), 10.1 ms (23.976) | **fail under load** | the bridge lane's prediction (`bridge.md`); idle now passes |
| Cadence | irregular 794 (23.976), 321 (25), 529 (30, load) | 120 Hz only, 0 drops in every run. Irregular intervals idle: 25 fps 308 (ideal 299: 290 are the 5-5-5-5-4 pattern, 18 presentation lag), 23.976 fps 108 (ideal about 7), 30 fps 87 (ideal 0); under load 528, 302, 447 | **fail** | fewer irregular intervals; the ideal is reached at no rate but 25 fps idle |
| Agent snapshots during playback | 0 drops | Frame bridge test pattern at 60 fps, 60 s each (load 4.4 to 5.9): baseline 0 of 3,587 dropped; 4 frames at 720 px every 2 s: 0 of 3,587, 30 batches, batch p50 184, p95 220, max 362 ms; back to back: 0 of 3,587, 1,504 batches (6,016 frames), batch p95 44.5 ms | pass | |
| Export plus playback memory, base 8 GB | deferred | deferred (base machine) | deferred | |
| Cold start | connection 634 to 675 ms, project 704 to 740 ms | not measured: the spec's synthetic project never reaches the engine (D5) | **not measured** | new defect D5 |
| S1 gate, real viewer | engine runs did not start (H2) | Playback, perf project: engine 0 of 1,793 dropped idle and under load, \|jitter\| p99 8.5 and 11.0 ms; today 1 and 100 dropped, 12.1 and 37.8 ms. Pan p50 / p99: engine 13.9 / 30.6 ms idle (load 17.8), 21.1 / 87.8 ms under load; today 21.6 / 29.9 and 20.9 / 70.4 ms | **fail** (pointer-to-pixel p99 and layer drag) | H2 fixed: the engine path is measured |
| Color invariant (1) | pass | 4:2:0 luma and chroma 0 CV (259,200 samples); 4:4:4 and 4:2:2 0 of 1,724,760 differ | pass | |
| Color invariant (2) | pass | sRGB PNG exact, 10/16-bit RGB 0, 8-bit limited ≤ 2 CV, 10/12-bit ≤ 1 CV | pass | |
| Color invariant (3) | float max ΔE2000 0.718 (H.264), 0.063 (ProRes) | float max 0.311 (H.264), 0.063 (ProRes); 8-bit BGRA delivery of H.264 1.162 | pass (as S2 defined it) | H.264 float delivery closer |
| Pipeline transparency | 0 differ | 1,148,760 and 576,000 samples through the export job, 688,500 and 345,600 in pm-render: 0 differ | pass | |
| Codec generation | ProRes 4444 alpha 1.603 CV within 8 px of α = 0 | ProRes 422 0.000 CV; ProRes 4444 alpha 1.603 CV within 8 px of α = 0, Δ −0.184 dB; H.264 Y′ engine 43.038, Apple 41.643 dB (Δ +1.396) | **fail** (ProRes 4444 alpha, decision) | H.264 Y′ up from Δ +0.480 |
| Time | incomplete | frame math to ±10⁸ at 16 rates (3,200,000,016 frames, 9.4 s); 91,800 exported frames at 9 rates; 29.97 non-drop now written (Phase 3b) but not in the time bar; burned-in timecode string and per-frame `tmcd` not covered | **incomplete** | 29.97 non-drop exists |
| 3D primitive in the 3D class | 12 scenes, max 1 CV | 74 outputs of the sweep 100% within 2 CV, max 1 CV, SSIM ≥ 0.99984; 265 draws and 23,831 uniforms equal to three.js; 34 corpus 3D frames within the bar | pass | larger sweep (3D lanes) |
| Crash and GPU-loss recovery | pass; harness clock defect (H1) | Engine kill and GPU loss resync (tracer spec); GPU process crash (`engine-present-recovery`); both bridge modes recover from an engine crash in 380 and 402 ms; `engine-host-integration.mjs` 5 of 5 (interactive 314.8 to 336.5 ms, job 312.8 to 334.3 ms) once it builds again (H3); pm-node, pm-engine, pm-host 139 tests pass | pass | H1 fixed; H3 found and fixed in this lane's scripts |
| IndexedDB copy, relink, agent imports, deleting a project | e2e pass | engine specs 28 of 28 | pass | |
| Engine-switch round trip | 131 of 131 corpus projects | fixture specs pass; corpus 130 of 131 in 2 of 2 runs: `3d/legacy-scene` gains keyframe defaults when opened with the flag off (R3) | **fail** | new (R3) |
| `test-mcp.mjs --desktop` | pass | pass, 4 verified exports (load 3.4) | pass | |
| E2E ledger, flag off | 560 run: 527 pass, 3 fail | 578 run: 541 pass, 2 fail, 35 skipped (19.5 min, load 8.8) | **fail**: 1 environment, 1 stale spec (R2) | R1 fixed |
| Engine specs, flag on | 27 of 27 | 28 of 28 (1.6 min); the engine specs added since: 11 of 12 (`native-engine-transport.spec.ts:91` failed), then that spec 8 of 9 with `--repeat-each=3` (`:184` failed once) | pass; **intermittent** in `native-engine-transport.spec.ts` | |
| Engine-related specs, flag on for every launch | 100 pass, 25 fail | 124 tests: 98 pass, 23 fail, 3 skipped (5.5 min, load 8.7): 7 tests from one new defect (D4), 11 that read the resting legacy engine, 4 by-design differences, 1 quarantined | **fail** (D4) | D1, D2, D3 fixed |

Also measured: unit tests over the media store, project files, the engine host and
connection, the barrier, the write path, `legacy/gl`, and agent tools: 496 passed, 2
skipped, 0 failed (49 files, load 9.5; `project-file.test.ts` passes). `pm-render`
`tests/corpus.rs` compares 256 frames, 0 failed. `pm-cli parity render` compares 189
frames (91 projects routed, 3 skipped for playback proxies): 3 fail, all
`composition/metadata` (2 to 6 pixels at a rotated rectangle's corners, max 43 CV),
the vector lane's MSAA residual that `pm-render`'s corpus test pins
(`KNOWN_RESIDUALS`, `vector.md`) and `pm-cli` does not.

The gate does not pass as rerun. The Phase 1 gate does not pass either
(`phase1-gate.md`: T1, T2, two test-code audit failures).

### Decisions for Jude

| # | Decision | Evidence (this rerun) |
|---|---|---|
| T1 | Phase 1: how level 1 measures `system-ui` at the sizes the app's UI text seeds in Chromium's font cache | `phase1-gate.md`: 1,260 corpus layouts up to 18.21 px off through pm-wasm, bit-identical natively with the capture's font cache |
| T2 | Phase 1: a variable-font ink class in the bounds tolerance, or a port of CoreText's variation arithmetic | 810 values, at most 1.191e-4 px against 1e-6 |
| ProRes 4444 alpha | Accept that a premultiplied pipeline cannot keep color under α = 0 (Apple's encoder does), or ask for a straight-alpha path for ProRes 4444 | max 1.603 CV at 10 bits within 8 px of α = 0, 0.000 beyond; Y′CbCr PSNR Δ −0.184 dB; unchanged since S2 |
| Base machine | The base-machine rows (memory on the 8 GB Air, every row's base form) stay deferred | not measured; only this M4 Max |
| 60 Hz | The cadence bar's 60 Hz half needs a 60 Hz display (or an external panel set to 60 Hz) | not measured; the built-in panel runs at 120 Hz |
| Cadence on the Chromium path | The ideal cadence is not reached through Chromium's compositor (`bridge.md`: which of two frame sinks reaches viz first, and the WindowServer's presentation lag); C′ (a remote Core Animation layer, private API) is the only path measured to reach it. Accept the Chromium path's cadence, or take C′ | Idle: 25 fps 308 irregular against the pattern's 299, 23.976 fps 108 against about 7, 30 fps 87 against 0; under load 302 to 528; 0 drops everywhere. Of the 30 fps idle run's 87, 67 changed the draw lag (the sink race) and 20 the presentation lag |
| Layer drag | The engine path is slower than today's viewer at every percentile, idle and under load (D6 below); the gate's "p95 ≤ 33 ms, at most a frame worse than today" is not met. Decide whether the gate waits for the render cache and evaluation reuse (`perf.md`, Gaps) | Idle p50 / p95 / p99: engine 49.8 / 58.4 / 58.6 ms against today 24.8 / 25.4 / 33.2 ms; under load 50.6 / 99.9 / 125.2 against 25.5 / 58.8 / 75.8 ms |
| A/V sync under load | Idle passes ±10 ms at p95 at every rate; under the gate's load the p95 is 10.1 to 10.8 ms, from frames shown a refresh early or late during live window resizes (`bridge.md`). Accept the load profile's resizes as outside the bar, or keep it a fail | \|offset\| p95 under the gate load: 10.2 ms (30 fps, 1,669 of 1,793 within), 10.8 ms (25 fps, 1,409 of 1,494), 10.1 ms (23.976 fps, 1,358 of 1,433); idle 5.9 to 7.8 ms with at most 2 frames outside |
| `composition/metadata` | Accept a pinned MSAA residual in the Standard class (2 to 6 pixels at a rotated rectangle's corners, max 43 CV, one flipped sample each; `vector.md`), or require it exact | `pm-cli parity render`: 3 of 189 frames fail, all this project |
| Time bar | Whether 29.97 non-drop, the burned-in timecode string, and a per-frame `tmcd` check must be in the time bar for Phase 2, or move to Phase 3b's delivery gate | 29.97 non-drop exports exist and their writer test passes; the time bar exports 9 rates with drop-frame labels at 29.97 and 59.94 |
| Still failing, no decision needed | D4, D5, D6, R2, R3, T-1, A-1 below have owners; the H.264 chroma gap (`export.md`) is open | below |

### Defects and harness problems found by the rerun

| # | Finding | Evidence | Owner |
|---|---|---|---|
| D4 | With Use New Engine on, an export started right after an import asks "1 clip is offline" and, answered Cancel (the hidden test window), returns `{ cancelled: true }`. Main's offline check (`engine/index.ts` `checkOffline`, section 14.8) asks the media store (`resolveAsset`) before the import's binding exists | 7 flag-on tests: `export-mp4.spec.ts:7`, `export-webm.spec.ts:8`, `import-animated-image.spec.ts:15` (GIF, APNG, WebP stills), `import-converted-formats.spec.ts:21` (TIFF, HEIC stills). A probe (the export-mp4 setup, main's message box recorded): exported at once, the box reads "1 clip is offline … tone.wav" and the result is cancelled; the same export 8 s after the import shows no box and returns `{ cancelled: false }` | Export and agents (delivery), Media |
| D5 | With Use New Engine on, one of two images imported together (`PM.importFiles`, the synthetic project of `native-engine-tracer-measure.spec.ts`) never becomes resolvable for the engine: the viewer logs "Drawn by the current engine: media offline: synthetic image a.png; … b.png", then the same with one of them, and the composition stays on the current engine for the spec's 90 to 120 s | 2 of 2 runs of the spec, 7 test runs (the routing test, the silent A/V sync row, cold start, the page edit row in the second run). The frame-bridge harness builds the same project through its own path and reaches the engine | Media, tracer (engine connection) |
| D6 | Layer drag on the engine path regressed after the perf lane: the frame's way from the handled move to its arrival on the page is p50 26.1 ms (17.8 of it before the engine starts rendering; the render takes 8.0 ms), against 14.5 ms in `perf.md`'s `perf7` run of the perf lane's branch | `tracer-drag-engine-500-idle-regate.json` (load 5.2): p50 49.8 ms against 29.8 ms in `perf7` (load 9.6); input to handled unchanged (1.0 ms), received to presented 16.4 against 11.3 ms. Merged after the perf lane's measurement: gltf, blender, shaders, longgop (wave 3d and 4); not bisected (the A/B against `ce1541b8` did not run, see the performance rows) | GPU and render, perf, engine and platform |
| R2 | `ae-audit-implementation.spec.ts:102` expects a saved render preset's format `prores`; the delivery lane's format ids save `prores-4444` (`shared/delivery.ts`, by design) | fails with the flag off, in `audit`, and on | Export and agents (the spec) |
| R3 | `3d/legacy-scene` of the corpus: opened with the flag off, the loader fills in default keyframe fields on the scene object's `ry` keyframes (`autoBezier: false`, `continuous: false`, `inInterp`/`outInterp: "linear"`, ease influence 33.3, speed 0) that the stored baseline omits; nothing is removed or changed. Order-dependent: in 2 of 2 whole-corpus runs, not when the project runs alone (1 of 1) | `phase2-gate-measurements/engine-switch-corpus-regate.json` (`differences`) | 3D |
| T-1 | `native-engine-transport.spec.ts` (added since the gate) is intermittent: `:91` (J/K/L, 2× reverse: 20 of the run's frames 8.8 to 12.5 ms off the audio line, against at most 1) failed 1 of 4 runs at load 10.3; `:184` (audio scrub: no grain at the scrub point) failed 1 of 3 runs at load 7 | the two runs' logs | Audio, tracer |
| H3 | The committed engine harnesses (`engine-host-integration.mjs`, `engine-fonts-integration.mjs`, `frame-bridge-integration.mjs`, `frame-bridge-measure.mjs`) no longer bundle on this tip: esbuild has no loader for main's `worker.py?raw` import (`engine/blender.ts`, Blender lane, 2b5ab30e). Fixed in this lane: `scripts/esbuild-raw-text.mjs` loads `?raw` as text, as electron-vite does | each script's build failed before the fix and passes after | fixed here (Engine and platform to review) |
| A-1 | Two auditor missed writes, both by test code in specs added since the Phase 1 gate (`phase1-gate.md`) | `engine-delivery.spec.ts:199`, `native-engine-tracer.spec.ts:429` | Export and agents, tracer |

### Items

#### Performance rows (section 25, Phase 2)

Harness rows from `scripts/frame-bridge-measure.mjs` (viz presentation timing), page
rows from `e2e/native-engine-tracer-measure.spec.ts` (the page's clock), the same
synthetic projects and clips as the first run. Round 1: 2026-10-18 00:13 to 00:29
(perf) and 00:49 to 01:00 (page), the display awake and the session unlocked before and
after every run. Load average per run as before → after.

These rows were measured once. The machine's quietest periods of the night (load 2.3
to 4.5 around 23:58 and from 05:15 to 07:45) fell while the display was asleep and the
session locked, when the harness refuses to measure; the display stayed asleep from
about 04:00 until at least 10:54, when this lane stopped waiting. A second round of
the drag, edit, and cadence rows and of S1, and an A/B of the engine drag between the
perf lane's merge (`ce1541b8`, built and ready) and this tip, were queued for the next
awake display and did not run. So every number here is at load 4 to 19, against the
first run's 8 to 24.

| Run (`-regate`) | Load | Measured |
|---|---|---|
| `tracer-drag-engine-500-idle` | 5.2 → 6.0 | pointer-to-pixel p50 49.8, p95 58.4, p99 58.6 ms, 2,401 of 2,401 presented. Hops p50 / p99: input → handled 1.0 / 3.3; handled → render start 17.8 / 26.1; render 8.0 / 9.8; sent → arrival 1.6 / 3.3; received → presented 16.4 / 24.7 |
| `tracer-drag-today-500-idle` | 4.9 → 5.2 | 24.8, 25.4, 33.2 ms, 2,400 of 2,401 |
| `tracer-drag-engine-500-load` | 6.3 → 8.3 | 50.6, 99.9, 125.2 ms, 2,198 of 2,401 (with the unpresented 58.3, 117.1, 159.4 ms); handled → arrival p99 74.6 ms |
| `tracer-drag-today-500-load` | 6.0 → 6.3 | 25.5, 58.8, 75.8 ms, 2,262 of 2,401 (with them 25.5, 68.0, 100.3 ms) |
| `tracer-edit-playback-engine-1000` | 7.1 → 6.9 | apply and send p50 11.5, p95 13.6, p99 14.6, max 16.0 ms (239 edits); edit to presented p50 106.7, p95 128.4 ms; 0 of 1,794 dropped; \|offset\| p95 2.4 ms |
| `tracer-edit-playback-today-1000` | 6.9 → 6.1 | apply p50 10.6, p95 13.3 ms; 6 of 1,793 dropped, jitter p99 24.9 ms |
| `tracer-playback-engine-1000` | 8.3 → 7.1 | 0 of 1,793 dropped; 424 irregular; \|offset\| p95 6.8 ms, 1,793 within |
| `tracer-playback-engine-25` | 6.1 → 7.1 | 0 of 1,494; 308 irregular (draw lag 0, presentation lag 18, pattern 290); \|offset\| p95 7.8 ms, 1,492 within |
| `tracer-playback-engine-23976` | 7.1 → 5.7 | 0 of 1,433; 108 irregular (85 draw, 16 presentation, 7 neither); p95 7.3 ms, 1,432 within |
| `tracer-playback-engine-30` | 5.1 → 6.8 | 0 of 1,793; 87 irregular (67 draw, 20 presentation); p95 5.9 ms, 1,793 within |
| `tracer-playback-engine-30-load` | 5.7 → 5.1 | 0 of 1,793; 447 irregular; p95 10.2 ms, 1,669 within |
| `tracer-playback-engine-25-load` | 6.8 → 4.0 | 0 of 1,494; 528 irregular; p95 10.8 ms, 1,409 within |
| `tracer-playback-engine-23976-load` | 4.0 → 5.1 | 0 of 1,433; 302 irregular; p95 10.1 ms, 1,358 within |
| `playback-engine-idle` (test pattern, 60 fps) | 4.4 → 5.9 | 0 of 3,587 dropped, jitter p99 12.7 ms |
| `playback-engine-agent-snapshots` | 5.9 → 5.4 | 0 of 3,587; 30 batches of 4 frames at 720 px, batch p50 184.3, p95 220.1, max 362.1 ms, 0 failures |
| `playback-engine-agent-snapshots-busy` | 5.4 → 4.9 | 0 of 3,587; 1,504 batches, batch p50 37.9, p95 44.5, max 447.9 ms, 0 failures |

Page timing (load 8.2 to 14.7):

| Row | Engine | Today |
|---|---|---|
| Scrub, ProRes 422 HQ 1080p (seek to draw) | p50 6.3, p95 7.5, max 8.9 ms; drags 362 (361 exact), 362 (361), 362 (361), 361 (361) | p50 18.9, p95 52.0 ms; drags 156, 163, 167, 162 |
| Scrub, H.264 GOP 250 1080p | p50 19.4, p95 58.1, max 63.7 ms; drags 362 (362), 358 (358), 362 (361), 320 (303) | p50 19.2, p95 94.2 ms; drags 170, 174, 173, 170 |
| Scrub, H.264 GOP 250 2160p | p50 63.5, p95 200.1, p99 206.6, max 210.9 ms, first frame p95 9.7 ms; drags 363 (362), 316 (315), 174 (1), 184 (128) | p50 18.9, p95 268.6, max 351.8 ms (today's 1280-pixel editing copy); drags 129, 137, 135, 131 |
| A/V sync, ProRes clip with audio | 449 of 449 within; line minus frame start p50 0.47, p95 3.83, max 4.44 ms; 0 skipped, 0 repeated | |
| A/V sync, 2160p long-GOP | 450 of 450 within; p95 3.79 ms | |
| A/V sync, 500 silent layers; cold start | not measured: the project stays on the current engine (D5; 2 of 2 runs) | |
| Edits during playback, 1,000 layers | apply and send p50 8.8, p95 13.1 ms; edit to draw p50 78.3, p95 90.8 ms; 0 dropped (the second run hit D5) | apply p50 8.5, p95 9.7 ms (8.2 / 9.0 in the second run) |

Drags are frames shown of about 360 asked (exact frames in parentheses) for 1, −1, 7,
and −7 frames per animation frame.

#### S1 gate against the real viewer (amendments 5 and 18)

`scripts/frame-bridge-measure.mjs` app runs on the 884-layer perf project, the engine
path through the kernel's own viewer and transport (H2's fix, `bridge.md`), round 1
00:29 to 00:49. The load moved a lot during this round; each run's load is given.

| Line | Today (real WebGL viewer) | Engine path | |
|---|---|---|---|
| Playback idle: drops, \|jitter\| p99 | 1 of 1,793, 12.1 ms (load 4.1 → 19.2) | 0 of 1,793, 8.5 ms (load 11.5 → 9.1); \|offset\| p95 5.7 ms | not worse |
| Playback under load | 100 of 1,793, 37.8 ms (load 19.2 → 11.5) | 0 of 1,793, 11.0 ms (load 9.1 → 7.6) | not worse |
| Pan idle, pointer-to-pixel p50 / p99 | 21.6 / 29.9 ms (load 7.6) | 13.9 / 30.6 ms (load 17.8) | p99 0.7 ms worse |
| Pan under load, p50 / p99 | 20.9 / 70.4 ms (with dropped inputs 21.0 / 70.9) | 21.1 / 87.8 ms (with dropped 21.1 / 87.8) | worse at p99 |
| Layer drag (engine draws the document) | the drag row above | the drag row above | worse |
| 3D orbit and walk | not measurable (no engine-drawn 3D gesture harness) | | not measured |
| Quality lines | `frame-bridge-integration.mjs`, both modes (load 8.7): every frame code of 3 s read back (180 of 180), `capturePage` ramp max error 0, 10-bit ramp of 1,024 codes monotonic with max error 0.4 CV, patches 0.25 to 4.0 in `rgbaf16` | | pass |

Today's drag on the perf project (`app-latency-today-*`): idle 31.7 / 39.7 ms, under
load 30.3 / 96.8 ms (p50 / p99).

#### E2E ledger, Use New Engine off

`POWERMOVE_NEW_ENGINE=0`, `playwright.ledger.config.ts`, 21:09 to 21:29 (load 8.8 at the
start, 9.1 at the end): 578 tests, 541 passed, 2 failed, 35 skipped (the ledger's
environment skips and the measurement tests that run on request). The suite gained 18
tests since the first run.

| Test | Failure | Triage |
|---|---|---|
| `chatgpt-connection.spec.ts:5` | two "Disconnect" buttons in Settings › Accounts | environment, unchanged (the hidden profile imports the developer's Claude settings) |
| `ae-audit-implementation.spec.ts:102` | the saved preset's format is `prores-4444`, the spec expects `prores` | R2, a stale spec |

R1 is fixed: `import-image-sequence.spec.ts` passed all four tests in the ledger, with
the flag on, and in `audit`.

#### Engine specs, Use New Engine on

`native-engine.spec.ts`, `native-engine-tracer.spec.ts`, `media-store.spec.ts`,
`engine-export.spec.ts`: **28 of 28** (1.6 min, load 9.1 to 10.3), including "a killed
interactive engine and a lost GPU recover and resync" and the section 21 tests below.
The engine specs added since the gate (`engine-present-recovery`,
`native-engine-transport`, `engine-blender`, `engine-delivery`, `native-edit-kernels`):
11 of 12, `native-engine-transport.spec.ts:91` failed (T-1); the transport spec then
`--repeat-each=3` (load 7.9 to 6.4): 8 of 9, `:184` failed once (T-1).

#### Engine-related specs with Use New Engine on

The first run's 40 files were not named in its report; `scripts/gate-rerun.sh`
(`FLAG_ON_SPECS`) lists the 40 reconstructed from its categories and failures. 124
tests: 98 passed, 23 failed, 3 skipped (5.5 min, load 8.7 to 9.1).

| Class | Tests | Change since the first run |
|---|---|---|
| Defect D4 (an export right after an import is cancelled as offline) | `export-mp4:7`, `export-webm:8`, `import-animated-image:15` ×3, `import-converted-formats:21` ×2 | new; the TIFF and HEIC cases now fail here, before the by-design MIME check |
| Read the resting legacy engine (WebGL compositor state or pixels, `<video>` elements, Web Audio voices) | `video-frame-integrity:33`, `:63`, `:99`, `video-playback-regression:6`, `:79`, `:128` ×2, `video-scrubbing:7` (HD), `import-mp4:166`, `playback-audio:15`, `import-svg:7` (`PM.GL.previewViewport` at high zoom) | `composited-video-sync:9`, `viewer-recovery:18`, `:128`, `project-file-media:9`, `:65` pass now; `video-frame-integrity:33`, `:63` and `import-svg:7` joined (the engine now draws motion-blurred video and imported SVG shapes) |
| By design with the flag on (the store keeps originals) | `import-converted-formats:50`, `import-prores:7`, `import-prores-alpha:10`, `local-media-recovery:38` | as before |
| Quarantined, fails with the flag off too | `precomp:46` | as before |

D1 (`export-video-frames.spec.ts`, 9 of 9), D2 (`export-mp4.spec.ts:61`), and D3
(`viewer-tools.spec.ts:140`) pass.

#### Section 2 quality bars

| Bar | Rerun (no Electron; load 5.9 to 9.1) |
|---|---|
| Color invariant (1) | `pm-render` `tests/color.rs`: H.264 4:2:0 → `420v` 259,200 samples, luma and chroma 0 CV. `pm-export` `tests/bars.rs`: ProRes 4444 XQ → `y416` 1,148,760 and ProRes 422 HQ → `v216` 576,000 samples through the export job, 0 differ (3,240 under α = 0 not compared) |
| Color invariant (2) | 512 solids at 16 and 32 bits: sRGB PNG exact, 10/16-bit RGB 0 CV, 8-bit limited Y′CbCr ≤ 2 CV, 10/12-bit ≤ 1 CV |
| Color invariant (3) | `gate-quicktime-color.mjs` (`phase2-gate-measurements/quicktime-color-regate.json`, load 2.4): float (`RGhA`) delivery H.264 max ΔE2000 0.311, mean 0.155 (worst patch 193/90/99); ProRes 422 HQ and 4444 max 0.063, mean 0.010. 8-bit `BGRA`: H.264 1.162 (grey 32), ProRes 0.425. The same twice |
| Pipeline transparency | as invariant (1)'s export rows; `pm-render`: 688,500 and 345,600 samples, 0 differ |
| Codec generation | ProRes 422 from 422 HQ max 0.000 CV, PSNR Δ 0.000 dB. ProRes 4444 with alpha: max 1.603 CV, 0.000 beyond 8 px of α = 0; Y′CbCr PSNR Δ −0.184 dB. H.264 at a matched rate: Y′ engine 43.038, Apple 41.643 dB (Δ +1.396); chroma, not judged, 34.395 against 41.676 dB |
| Time | `pm-time` exhaustive: 3,200,000,016 frames at 16 rates in 9.4 s on 4 threads. `time_bar_full`: 91,800 exported frames at 9 rates each carry their frame code; `tmcd` names each window's first frame (382 s). 29.97 non-drop is now an export option (Phase 3b; `videotoolbox.rs` `h264_in_mov_carries_timecode_and_pcm`: a 29.97 non-drop `tmcd` at 01:00:00:00 = frame 108,000, passes) but the time bar does not export it; the burned-in timecode string and a per-frame `tmcd` check are still not covered |
| 3D primitive | `pm-render` `tests/scene3d.rs`: 265 draws matched, 23,831 uniform values equal as f32, programs from the disk cache; the sweep's 74 outputs 100% within 2 CV, max 1 CV, at most 1 CV outside edges, SSIM ≥ 0.99984. `tests/corpus.rs`: 34 3D frames 100% within the 3D bar, SSIM ≥ 0.99995 |

#### Crash and GPU-loss recovery

| Test | Rerun |
|---|---|
| `native-engine-tracer.spec.ts` "a killed interactive engine and a lost GPU recover and resync" | pass |
| `engine-present-recovery.spec.ts` (D3's duplicate delivery; GPU process crash) | 2 of 2 |
| `frame-bridge-integration.mjs`, both presentation modes (load 8.7) | pass: an engine crash gets a new name and a verified check-in and the viewer presents again in 380 ms (main) and 402 ms (worker); the old launch's 32 surfaces released |
| `engine-host-integration.mjs` | as committed it does not build (H3). With `esbuild-raw-text.mjs`, 5 of 5 runs pass (load 2.3 to 3.5): the engines read main's clock (H1 fixed); interactive engine killed and resynced in 314.8, 333.1, 327.1, 330.8, 336.5 ms; job engine 329.6, 312.8, 332.7, 334.3, 325.2 ms; reconnects racing a dying engine dropped; no engine process left |
| `cargo test --release -p pm-node -p pm-engine -p pm-host` | 139 passed, 5 ignored, 0 failed (load 4.7 to 9.8) |

#### Media management and switching engines (section 21)

The section 21 tests in the engine specs pass (IndexedDB copy verified by hash and
never deleted; relink by content hash; agent imports and workspace files copied;
deleting a project never touches linked originals; the engine-switch round trips of
the tracer and store specs).

`gate-engine-switch-corpus.mjs` (now records the differing values,
`phase2-gate-measurements/engine-switch-corpus-regate.json`): 130 of 131 corpus
projects round-trip, twice (load 3.3 to 5.1 and 3.7); `3d/legacy-scene` fails both
times (R3) and passes alone. The 16 effect projects change their keyframe ids on any
reopen, as before.

#### `test-mcp.mjs --desktop`

Pass (load 3.4): `render_frames` through the job engine, `export_video` H.264 (wait)
and ProRes (async), a text project routed to the current exporter with the job engine
still serving after it, 4 exports verified with our FFmpeg.

#### Reproducing the rerun

```sh
source engine/env.sh
cd apps/desktop && bun install && node scripts/build-engine-addon.mjs \
  && node ../../engine/crates/pm-wasm/scripts/build.mjs && bun run build
# the harnesses load pm-bridge with its harness feature; the addon build replaces it
(cd ../../engine && cargo build --release --locked -p pm-bridge-addon --features harness)
scripts/gate-rerun.sh ledger-off engine-specs engine-specs-new flag-on-specs audit-suite \
  engine-host-integration test-mcp engine-switch-corpus pm-wasm-bench
scripts/gate-rerun.sh perf s1 tracer-measure frame-bridge-integration transport-repeat
node scripts/gate-summaries.mjs ../../docs/engine/tracer-measurements/*-regate.json
# the bars and parity runs without Electron are the first run's commands below
```

`gate-rerun.sh` writes Playwright JSON, logs, and `steps.tsv` (each step's start, end,
load before and after, exit status) to `~/Library/Caches/powermove-engine/regate`
(`GATE_OUT`); `GATE_TAG` names the harness summaries, `GATE_RUNS` narrows the harness
runs.

The first run's report follows unchanged.

---

The Phase 2 gate (plan section 26, "Phase 2: Tracer bullet"; section 25's performance
rows for Phase 2; the section 2 quality bars Phase 2 names; section 21's gate; section
18's S1 gate with amendments 5, 12, and 18), measured on `feat/ne-p2gate`: the
integration branch `feat/native-engine` at `3d37d8b4` (waves 1 to 3; `feat/ne-parity`,
`feat/ne-tracer`, `feat/ne-jobexport`, and `feat/ne-store` had nothing left to merge)
plus this lane's measurement scripts. No engine code was changed here.

Measured 2026-10-15 between 21:55 and 23:40 on Jude's MacBook Pro (Apple M4 Max,
128 GB, macOS 27, built-in 120 Hz XDR panel, AirPods Pro as the output device at
160 ms latency), Electron 44.0.0 (Chrome 152), release `pm_node.node` and
`pm_bridge.node` built in the worktree. The Mac was shared with other lanes: the
1-minute load average was 8 to 24 during the runs (every harness summary records
it). Every Electron run was hidden and unfocused, one at a time under the machine
lock. This is the Pro-class reference machine of decision 11; the base-machine rows
are deferred by Jude's decision and are marked so. Numbers are as measured; nothing
is passed on a looser tolerance than the plan's.

> Gate: Phase 2 performance rows; Standard color invariants, pipeline transparency,
> codec generation, time, and cadence bars; the 3D primitive within the 3D class; crash
> and GPU-loss recovery; the IndexedDB copy verified by hash and never deleted;
> content-hash relink; agent workspace and agent-imported files always copied;
> deleting a project never touches a linked original; the engine-switch round trip;
> `test-mcp.mjs --desktop` export through the job engine.

## Summary

| Gate line | Measured | Result |
|---|---|---|
| Layer drag, 500 layers | p95 46.2 ms idle, 125.0 ms under load (today 34.9, 65.7) | **fail** |
| Inspector edit during playback, 1,000 layers | apply and send p95 31.5 ms (page); edit to presented p95 223.7 ms; 381 of 1,796 frames dropped with edits | **fail** |
| Scrub, intra (ProRes 422 HQ 1080p) | seek to draw p95 8.8 ms | pass |
| Scrub, long-GOP | 1080p p95 103.9 ms; 2160p p95 347.5 ms | 1080p pass, **2160p fail** |
| A/V sync ±10 ms | page timing 900 of 900 frames within; presentation p95 11.9 to 20.6 ms | **fail** at p95 by presentation |
| Cadence | 0 drops at 23.976, 25, 30 fps; irregular intervals 794, 321, 529 against ideal 7, 299, 0 | **fail** |
| Agent snapshots during playback | 0 drops of 3,587 with 4 frames every 2 s and back to back | pass |
| Export plus playback memory, base 8 GB | | deferred (base machine) |
| Cold start | connection ready 634 to 675 ms, project ready 704 to 740 ms | pass |
| S1 gate, real viewer (amendments 5, 18) | engine app runs do not start (harness); document drag worse than today; quality lines pass | **fail** (pointer-to-pixel), playback not comparable |
| Color invariant (1) | 4:2:0 luma and chroma exact; 4:4:4 and 4:2:2 at 32-bit 0 of 1,724,760 differ | pass |
| Color invariant (2) | sRGB PNG exact, 8-bit limited ≤ 2 CV, 10/12-bit ≤ 1 CV, greys neutral | pass |
| Color invariant (3) | float delivery max ΔE2000 0.718 (H.264), 0.063 (ProRes); 8-bit BGRA delivery of H.264 1.220 | pass (as S2 defined it) |
| Pipeline transparency | 1,148,760 and 576,000 samples, 0 differ | pass |
| Codec generation | ProRes 422 0.000 CV, Δ 0.000 dB; ProRes 4444 alpha 1.603 CV within 8 px of α = 0, Δ −0.184 dB; H.264 Y′ +0.480 dB | **fail** (ProRes 4444 alpha edges) |
| Time | frame math to ±10⁸ at 16 rates; 91,800 exported frames at 9 rates read back; 29.97 NDF, the timecode string, and per-frame `tmcd` not covered | **incomplete** |
| 3D primitive in the 3D class | 12 scenes and 7 corpus projects: 100% within 2 CV, max 1 CV, SSIM ≥ 0.99998 | pass |
| Crash and GPU-loss recovery | app kill and GPU loss resync; both engines killed and back in 310 to 323 ms; the integration harness's own clock check fails | pass, harness defect (H1) |
| IndexedDB copy verified by hash, never deleted | e2e pass | pass |
| Content-hash relink | e2e pass | pass |
| Agent workspace and agent imports copied | e2e pass | pass |
| Deleting a project never touches a linked original | e2e pass | pass |
| Engine-switch round trip | fixture projects pass; every corpus project (131 of 131) round-trips | pass |
| `test-mcp.mjs --desktop` | 4 verified exports through the job engine | pass |
| E2E ledger, flag off | 560 run: 527 pass, 3 fail, 30 skipped | **fail**: 1 intermittent regression (2 tests, R1), 1 environment |
| Engine specs, flag on | 27 of 27 | pass |
| Engine-related specs with the flag on for every launch | 100 pass, 25 fail | **fail**: 4 tests from 2 defects, 1 intermittent defect; 13 tests that observe the resting legacy engine, 6 by-design differences, 1 quarantined |

The gate does not pass as measured. The Phase 2 gate also requires the Phase 1 gate,
which did not pass as `phase1-gate.md` records it (T1, T2, T5, T6).

What stands between this tip and a pass, by owner:

1. **pm-render** (GPU and render): the last frames of a video clip are lost in exports
   (D1); layer-drag and edit-during-playback latency (the profile in `tracer.md`
   puts the time in the page's pm-wasm queries, pm-eval lookups, and one pass per
   layer).
2. **Engine and platform** (frame bridge): pointer-to-pixel and cadence (bridge.md's
   decision is still open); a shared-texture release race (D3); the S1 app harness
   no longer starts its engine runs (H2); the engine-host harness's clock check (H1).
3. **Media**: long-GOP UHD scrub (2160p p95 347.5 ms against 300 ms).
4. **Export and agents** (pm-export): ProRes 4444 alpha generation near α = 0 (known
   since S2; needs Jude's decision or a different alpha path), the remaining time-bar
   pieces (29.97 non-drop, burned-in timecode string once text renders, `tmcd` per
   frame).
5. **Tracer** (engine connection): a project installed by assigning `PM.proj` never
   plays on the engine clock (D2); Play takes about 300 ms to move the playhead on a
   Bluetooth output.
6. **Proxies and storage**: the image-sequence export regression (R1).
7. **Parity and QA**: 13 legacy tests that read the WebGL compositor, video elements,
   or Web Audio voices need rewriting against engine frames before they can run with
   the flag on (section 25, "Infrastructure").

## Performance rows (section 25, Phase 2)

Harness rows come from `scripts/frame-bridge-measure.mjs` (viz presentation timing;
summaries in `phase2-gate-measurements/*-p2gate.json`). Page rows come from
`e2e/native-engine-tracer-measure.spec.ts` (the page's clock; seek and edit rows end at
the presentation worker's draw, about two refreshes before the panel;
`phase2-gate-measurements/tracer-measure-page.jsonl`). The synthetic projects are
`src/main/engine/integration/synthetic-project.ts` (500 layers: 432 solids, 35 images,
19 groups, 10 precomps, 4 nulls, 301 keyframed, 1920 × 1080 at 30 fps; 1,000 layers
for the playback rows); the clips are the ones `tracer.md` generates. Every run
checked the display awake and the session unlocked before and after.

| Row | Gate | Measured | Result | Owner |
|---|---|---|---|---|
| Layer drag, 500 layers of slice features | p95 ≤ 33 ms pointer-to-pixel; at most 1 frame worse than today | Idle: engine p50 38.0, p95 46.2, p99 51.0 ms (2,384 of 2,401 presented); today (WebGL viewer, same project) p50 26.6, p95 34.9, p99 37.0 ms. Under load: engine p50 41.8, p95 125.0, p99 175.0 ms (205 of 2,401 never presented); today p50 24.4, p95 65.7, p99 74.5 ms | **fail** both forms | pm-render, pm-wasm, frame bridge (`handledToArrival` p50 21.5 ms idle, p99 129.7 ms under load: view state and edit out, engine render, descriptor back) |
| Inspector edit during playback, 1,000 layers | p95 ≤ 16 ms, no dropped frames | Page: apply and send p50 24.5, p95 31.5 ms (Use New Engine off: p50 29.5, p95 39.3 ms); edit to draw p50 136.3, p95 155.3 ms. Harness, an edit every 250 ms: apply p95 157.7 ms, edit to presented p50 156.5, p95 223.7 ms, 381 of 1,796 frames dropped. The same playback without edits: 0 of 1,793 dropped | **fail** | tracer (edits wait behind 4 frames rendered ahead), pm-render (compose cost), pm-wasm |
| Scrub, intra codecs | p95 ≤ 50 ms | ProRes 422 HQ 1080p29.97: random seek to draw p50 8.2, p95 8.8, max 9.8 ms (60 seeks); playhead drag 360 of 360 frames shown, p95 15.4 ms | pass | |
| Scrub, long-GOP, no generated media | p95 ≤ 300 ms | H.264 High GOP 250, 3 B-frames. 1080p: p50 33.6, p95 103.9, max 111.1 ms. 2160p: p50 125.4, p95 347.5, max 736.3 ms; a playhead drag shows 242 of 360 asked frames | 1080p pass, **2160p fail** | pm-media (decode from the GOP start) |
| A/V sync | ±10 ms | Page (the audio clock's line at each frame's display instant against the frame's interval): 450 of 450 within (500 silent layers) and 450 of 450 (a ProRes clip with audio); line minus frame start p50 4.0, p95 7.6, max 8.3 ms. Presentation (viz time minus the frame's start on the audio line): 25 fps p50 6.7, p95 11.9, p99 13.4; 23.976 fps p50 5.6, p95 11.9, p99 13.1; 30 fps under load p50 8.4, p95 20.6, p99 21.1; 1,000 layers p50 11.6, p95 11.9, p99 19.8 ms | **fail** at p95 by presentation | frame bridge (the viewer's predicted display time, `tracer.md` gaps) |
| Cadence (section 2) | 23.976 and 25 fps on 60 and 120 Hz: ideal cadence, zero irregular repeats over 60 s | 120 Hz only (no 60 Hz display here). 25 fps: 0 drops, 321 irregular intervals of 1,493 (the ideal 5-5-5-5-4 pattern has 299), jitter p99 39.8 ms. 23.976 fps: 0 drops, 794 of 1,431 (ideal about 7), jitter p99 7.5 ms. 30 fps under load: 0 drops, 529 of 1,792 (ideal 0) | **fail** | frame bridge (worker canvas alternates one and two refreshes; no presentation feedback in a worker, bridge.md) |
| Agent snapshots during playback | 4 frames per agent turn, no dropped playback frames | Frame bridge test pattern at 60 fps, 60 s each (amendment 12's 2-frame deadline). Baseline: 0 of 3,587 dropped, jitter p99 8.49 ms. 4 frames at 720 px every 2 s on a 24-layer 4K project: 0 of 3,587 dropped, 30 batches (120 frames), batch p50 244, p95 307, max 428 ms, jitter p99 8.53 ms. Back to back: 0 of 3,587 dropped, 590 batches (2,360 frames), batch p95 102 ms, jitter p99 8.42 ms | pass | |
| Export plus playback memory, base 8 GB | No memory-pressure warnings over 10 minutes | Not measured: a base-machine row | deferred | |
| Cold start | Engine ready before the window finishes loading the last project | 3 launches of a 500-layer project: engine connected 634, 662, 675 ms after launch; project ready 704, 726, 740 ms; first engine frame 1,024 to 1,059 ms (about 320 ms after the project) | pass (connection); the first frame comes later | |

Compared with `tracer.md`'s runs (2026-10-15, earlier wave 3 tip): playback of 1,000
layers no longer skips frames (0 of 1,793 against 384), the 25 fps irregular count fell
(321 against 488), drag p95 fell (46.2 against 113.8 ms idle), and edits during
playback are as before. The baseline of the agent-snapshot runs now alternates
refreshes (jitter p99 8.49 ms, 1,290 irregular intervals; r4 and r5 had 0.24 ms).

## S1 gate against the real viewer (section 18, amendments 5 and 18)

Amendment 5 makes the latency and drop lines relative (no worse than today's viewer at
p50 and p99, idle and under the load profile) and keeps the quality lines absolute;
amendment 18 makes "today" the real WebGL viewer and asks that the gate report the
comparison as measured once its three steps are done (they are, bridge.md).

| Line | Measured | Result |
|---|---|---|
| Playback, perf project (884 layers), today | Idle: 2 of 1,793 dropped, jitter p99 8.49 ms. Under load: 89 dropped, jitter p99 41.6 ms | today's reference |
| Playback, perf project, engine path | Not measured: the `app-playback-engine-*` and `app-latency-engine-*` runs time out (H2) | **not measured** |
| Playback, engine drawing the document | 1,000 layers idle: 0 of 1,793 dropped, jitter p99 8.29 ms; 500 layers at 30 fps under load: 0 of 1,793, jitter p99 15.5 ms. Not like for like with today's perf-project runs (the perf project routes to the current engine: effects, text, masks) | measured |
| Pointer-to-pixel, layer drag (engine draws the document) | See the drag row: worse than today at p50 and p99, idle and under load | **fail** |
| Pointer-to-pixel, pan (perf project) | Today idle p50 22.3, p99 30.7 ms; under load p50 23.3, p99 73.4 ms. Engine not measured (H2) | not measured |
| 3D orbit and walk | Not measurable: no 3D corpus scene routes to the engine with a gesture harness | not measured |
| Quality lines | `scripts/frame-bridge-integration.mjs`, both presentation modes: every frame code of 3 s of playback read back, `capturePage` ramp max error 0, a 1,024-code 10-bit ramp monotonic with max error 0.4 CV, patches 1.25 to 4.0 survive in `rgbaf16` on the EDR panel | pass |

## Section 2 quality bars

| Bar | Measured | Result |
|---|---|---|
| Color invariant (1) | `pm-render` `tests/color.rs`: H.264 4:2:0 → `420v`, 259,200 samples, luma and chroma 0 CV. Through the export job (`pm-export` `tests/bars.rs`): ProRes 4444 XQ → `y416` 1,148,760 samples and ProRes 422 HQ → `v216` 576,000 samples, 0 differ (3,240 color samples under α = 0 not compared) | pass |
| Color invariant (2) | `tests/color.rs`: 512 solids at 16- and 32-bit: sRGB PNG exact, 10/16-bit RGB 0 CV, 8-bit limited Y′CbCr ≤ 2 CV, 10- and 12-bit ≤ 1 CV, greys neutral | pass |
| Color invariant (3) | New measurement (`scripts/gate-quicktime-color.mjs`, below). A 40-patch chart exported by `pm-cli render` (the job engine's pipeline) with "Rec.709 (QuickTime and Safari)" (1-13-1, limited), played through `AVPlayerItemVideoOutput`, against ColorSync of the viewer's codes, for the main display profile and Display P3. Float (`RGhA`) delivery: H.264 max ΔE2000 0.718, mean 0.228 (worst patch 122/122/121); ProRes 422 HQ and ProRes 4444 max 0.063, mean 0.010. 8-bit `BGRA` delivery: H.264 max 1.220 (grey 24), ProRes 0.425. Equal to S2's prototype figures | pass on the float delivery, as S2 defined the bar; the 8-bit delivery of an 8-bit H.264 rounds once more |
| Pipeline transparency | As invariant (1)'s export rows, and `pm-render`'s own: 688,500 and 345,600 samples, 0 differ | pass |
| Codec generation | `tests/bars.rs`. ProRes 422 from 422 HQ: max 0.000 CV at 10 bits, Y′CbCr PSNR 63.549 dB both. ProRes 4444 with alpha (premultiplied, amendment 8): max 1.603 CV overall, 0.000 beyond 8 px of α = 0; Y′CbCr PSNR Δ −0.184 dB, Y′ Δ −0.004 dB. H.264 at a matched rate (amendment 8): Y′ PSNR engine 42.123, Apple 41.643 dB (Δ +0.480); chroma, not judged, engine 34.337 against Apple 41.676 dB | **fail** for ProRes 4444 alpha within 8 px of α = 0 (Apple keeps color under α = 0, a premultiplied pipeline cannot; S2). The H.264 chroma gap is `export.md`'s open item |
| Time | `pm-time`: frame math exhaustive to ±10⁸ frames at all 16 rates (3,200,000,016 frames, 44.6 s), the section 2 vectors, invalid drop-frame strings rejected. `pm-export` `time_bar_full`: 91,800 exported frames at 9 rates (10,000 from frame 0, 100 at 10⁶ and 10⁷) each carry their burned-in binary frame code; `tmcd` names each window's first frame (447 s) | **incomplete**: 29.97 non-drop exports (exports label 29.97 and 59.94 drop-frame until presets choose), the burned-in timecode string (text is not in the Phase 2 renderer), and `tmcd` per frame are not covered (pm-export) |
| Cadence | The cadence row above | **fail** |
| 3D (the Phase 2 primitive) | `pm-render` `tests/scene3d.rs`: 12 primitive scenes against three.js r180 with Metal safe math, 100% of pixels within 2 CV, max 1 CV, SSIM ≥ 0.99998; 858 uniforms equal in f32; 12 of 12 programs byte-identical; `pm-geom3d` primitives bit-identical. `tests/corpus.rs`: the 7 render-corpus 3D projects within the 3D bar (3D-in-a-precomp judged by the stricter Standard class, max 1 CV). Fast math: one box 33 CV at an MSAA edge (inside the bar's edge exclusion) | pass |

`pm-render` `tests/corpus.rs` (70 frames) and `pm-cli parity render` (95 frames: 126
projects routed, 3 skipped because the legacy frame shows a playback proxy) both
report 0 failures.

## Crash and GPU-loss recovery

| Test | Result |
|---|---|
| `e2e/native-engine-tracer.spec.ts` "a killed interactive engine and a lost GPU recover and resync" | pass: after SIGKILL the next launch's session resyncs and an edit reaches a frame; after `simulateGpuLoss` the presenter rebuilds its device and pool (new generation) and draws an edit |
| `scripts/frame-bridge-integration.mjs`, both modes | pass: an engine crash gets a new bootstrap name and a verified check-in, the viewer presents again, the old launch's surfaces are released |
| `scripts/engine-host-integration.mjs` | **fails its first check** (H1). With the harness's clock changed to the engines' clock (a temporary copy, not committed), 5 of 5 runs pass: the interactive engine killed, restarted, and resynced in 309.6 to 320.1 ms; the job engine in 313.9 to 322.7 ms (250 ms of each is the first backoff step), reconnects racing a dying engine dropped, leases re-granted, no engine process left |
| `cargo test --release -p pm-node`, `pm-engine`, `pm-host` | pass (GPU loss and routing child-process test included) |

## Media management and switching engines (section 21)

`e2e/media-store.spec.ts`, `e2e/native-engine-tracer.spec.ts`,
`e2e/native-engine.spec.ts`, and `e2e/engine-export.spec.ts`: 27 of 27 pass in 1.6 min.

| Gate line | Test | Result |
|---|---|---|
| IndexedDB copy verified by hash, never deleted | "IndexedDB media is copied to the store, verified by hash, and never deleted"; "opening a project file on another Mac copies its media into the store in main, verified" | pass |
| Content-hash relink | "relinks a moved and renamed original by content hash"; `pm-node` "a relinked asset is resolved again in the same session" | pass |
| Agent workspace and agent imports always copied | "agent imports are copied unless they ask to link; agent workspace files are always copied" | pass |
| Deleting a project never touches a linked original | "deleting a project never touches its linked originals" | pass |
| Engine-switch round trip | "switching engines on, off, and on again loses nothing" (tracer); "Use New Engine on → off → on loses nothing; a linked project is refused cleanly while off" (store); every corpus project (below) | pass |

**Every corpus project** (`scripts/gate-engine-switch-corpus.mjs`, new): one profile,
the 131 synthetic projects of corpus v1 opened as the capture opens them (60 with media,
65 assets imported through the app), stored with Use New Engine on, then reopened on,
on again (a control), off, and on. 131 of 131 round-trip with no loss: every stored
document equal to the baseline (keys sorted, open and save timestamps aside) and every
asset loaded in every phase. 16 effect projects change on any reopen, flag on both
times: the loader reassigns effect-parameter keyframe ids on every open
(`fx[].p.*.kf[].i`; the corpus capture records the same as `keyIdRewrites`), and the
switch changes nothing beyond that. No corpus project is stamped (the corpus was
captured by the current engine); the stamped refusal is the store spec's case.
Summary: `phase2-gate-measurements/engine-switch-corpus.json`.

Unit tests over the media store, project files, the engine host, the engine
connection, the barrier, and agent tools: 346 of 349 pass, 2 skipped; the one failure is
`project-file.test.ts` "restores ranges as Blobs that own their bytes" timing out at
vitest's 5 s with the machine at load 17 to 20 (5.2 to 7.8 s in 4 runs). It passes at
load 9 (2 of 2). Owner: Proxies and storage (a 3 MB element-wise comparison in the test).

## `test-mcp.mjs --desktop`

Pass (15 s): `render_frames` returns the job engine's frames; `export_video` H.264 (wait)
and ProRes (async, through `get_render_status`) through the job engine; a text project
routed to the current exporter and the job engine still serving after it; 4 exports
verified with our FFmpeg.

## E2E ledger, Use New Engine off

`POWERMOVE_NEW_ENGINE=0 bun run test:e2e:ledger` with `--workers=1`, no retries: 560
tests run (quarantine.json's 27 entries excluded), 527 passed, 3 failed, 30 skipped
(the ledger's 21 environment skips and the 9 measurement tests that run on request),
29.3 min. The ledger's last run (amendment 14) had 522 run and 0 failed; the suite
gained 38 tests since.

| Test | Failure | Triage | Owner |
|---|---|---|---|
| `import-image-sequence.spec.ts:7` (12 fps and 120 fps) | The restored sequence's 2048 × 2048 alpha still export throws "The GPU could not capture the export frame" (`frame-capture.ts`: `renderToPixels` returned no frame) | **R1, a regression, intermittent.** At the ledger base (`feat/ne-legacy2`) 11 of 11 runs pass; at this tip 10 of 18 fail. A bisect over the integration branch's merges with 6 runs per step: Merge lane render (wave 3) 6 of 6 pass, Merge lane store (wave 3) `9060f251` 1 of 6 fail, then present 1 of 6 and audio 3 of 6 fail. The store lane's restore path is the first suspect (`legacy/core/project-file.ts`: 22f939d0, df0877ee; `legacy/gl/raster.ts`) | Proxies and storage |
| `chatgpt-connection.spec.ts:5` | Two "Disconnect" buttons in Settings › Accounts (strict-mode violation) | **Environment, not code.** The hidden profile's isolated Claude home copies the developer's `~/.claude/settings.json` (`prepareIsolatedClaudeHome`), which on this Mac now carries an `env` block with an auth token (file changed 2026-10-15 11:43, after the ledger runs), so Claude Code reports logged in and Claude shows a second Disconnect. Fails 7 of 7, also with the session's own auth variables unset; `claude auth status` in a fresh config directory reports logged out | Export and agents (the e2e harness should not import the developer's Claude settings) |

## Engine-related specs with Use New Engine on

The engine specs above pass (27 of 27). Then 40 engine-related legacy spec files with
`POWERMOVE_NEW_ENGINE=1` for every launch (boot, saves, project files, imports, media
recovery and replacement, agent media, exports, video playback and scrubbing, viewer,
GPU recovery, precomps, compositions, 3D, capture): 100 passed, 25 failed, 2 skipped,
6.6 min. These specs predate the engine; section 25 has the WebGL readers rewritten
against engine frames. The failures, by cause:

**Defects**

| Test | Failure | Triage | Owner |
|---|---|---|---|
| `export-video-frames.spec.ts:12` (3 cases with MP4 output at 30 fps) | The exported last frame repeats the one before it (level 200 instead of 210) | **D1.** Reproduced outside the app with `pm-cli render` (a 20-frame H.264 source in a project of its length): 10, 24, and 25 fps keep every frame, 30 fps loses the last frame, 60 fps the last two, for H.264 and ProRes outputs alike. `pm-render` `plan.rs` `playback_time` clamps every source time to `dur − 0.04` (the legacy compositor's preview padding); today's export path (`frame-preparation.ts`) passes `1 / fps` as the padding, so its exports keep the last frame | pm-render (GPU and render) |
| `export-mp4.spec.ts:61` | After a cancelled export, 500 ms of Play moves the playhead 0 s (> 0.2 s expected) | **D2.** Not about the cancel: a probe in the built app shows a project installed by assigning `PM.proj = PM.mkProject(...)` never advances (0 s in 500 ms, route `engine`), with or without an export; a project opened through `pm-open-project` advances 0.19 s in 500 ms (the run-start hold on a 160 ms Bluetooth output); with the flag off both advance 0.50 s | tracer (`engine-connection/session.ts`, `transport.ts`) |
| `viewer-tools.spec.ts:140` | A page error: `IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN`: "Shared texture with id … not found", after a WebGL context loss in the run | **D3, intermittent** (1 of 4 runs): a release reaches main after main dropped the texture | Engine and platform (frame bridge release path, `extensions/viewer/engine-present/`) |

**Specs that observe the resting legacy engine** (with the engine presenting, the WebGL
compositor, the video elements, and Web Audio rest by design, `tracer.md`):

| Tests | What they read | Owner |
|---|---|---|
| `composited-video-sync:9`, `video-frame-integrity:99`, `video-scrubbing:7`, `video-playback-regression:79`, `:128` (×2) | Hooks on `PM.GL.render` or `gl.readPixels` | Parity and QA |
| `viewer-recovery:18`, `:128` | Pixels of the `#gl` WebGL canvas | Parity and QA |
| `video-playback-regression:6`, `project-file-media:9` | The asset's `<video>` element (paused, seeking) | Parity and QA |
| `import-mp4:166`, `playback-audio:15`, `project-file-media:65` | Web Audio voices (`dataset.audioVoices`) | Audio, Parity and QA |

**Behavior that differs by design with the flag on** (the media store keeps originals,
`media-store.md`); each needs a decision on the spec:

| Tests | Difference | Owner |
|---|---|---|
| `import-converted-formats:21` (TIFF, HEIC) | The stored asset keeps `image/tiff` and `image/heic` (kept originals with a rendition) | Proxies and storage |
| `import-converted-formats:50` (Matroska), `import-prores:7`, `import-prores-alpha:10` | `playbackProxy` is false: the original is kept and the engine decodes it | Proxies and storage |
| `local-media-recovery:38` | A changed source file no longer makes the asset unavailable: the store holds the imported copy | Proxies and storage |

`precomp.spec.ts:46` fails with the flag off too (quarantined).

## Harness defects

- **H1**, `src/main/engine/integration/electron-main.ts`: the integration harness passes
  `hostNowUs` (libuv's clock, which counts sleep) as main's host clock, while main
  itself uses the engines' `CLOCK_UPTIME_RAW` through pm-bridge since `c2236f89`
  (`engineClock` in `src/main/engine/index.ts`). After the Mac has slept the two differ
  by the sleep (28,893 s here), so "the interactive engine shares main's host clock"
  fails before any crash step runs. Owner: Engine and platform.
- **H2**, `src/main/engine/integration/measure-app-page.ts` (`engine` mode): the S1 app
  harness's page and the kernel's engine connection (`engine-connection/install.ts`,
  tracer lane) both send their hello on the same interactive port; the engine refuses
  the second ("The handshake already happened on this connection.") and closes it, and
  the run times out waiting for the viewer. Every `app-*-engine-*` run fails this way on the
  integration tip; the `tracer-*` runs use the kernel's connection and work. Owner:
  Engine and platform (frame bridge), with the tracer lane.

## Not done here

- Base-machine rows (memory on the 8 GB base Air, and every row's base-machine form):
  deferred by Jude's decision.
- No 60 Hz display: the cadence bar's 60 Hz half.
- S1 playback and pan through the engine path on the perf project (H2), 3D orbit and
  walk pointer-to-pixel (no harness for an engine-drawn 3D scene).
- Opening the 884-layer perf project took 74 to 76 s in the harness's engine runs at
  load 9 to 10 (today's runs started their line about 77 s after launch too); not a
  gate line, not investigated.

## Reproducing

```sh
source engine/env.sh
cd apps/desktop && bun install && node scripts/build-engine-addon.mjs \
  && node ../../engine/crates/pm-wasm/scripts/build.mjs && bun run build
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock

# Ledger, flag off; engine specs; engine-related specs with the flag on
POWERMOVE_NEW_ENGINE=0 $L bunx playwright test --config playwright.ledger.config.ts --workers=1
$L bunx playwright test e2e/native-engine.spec.ts e2e/native-engine-tracer.spec.ts e2e/media-store.spec.ts e2e/engine-export.spec.ts --workers=1
POWERMOVE_NEW_ENGINE=1 $L bunx playwright test <the 40 files listed above> --workers=1

# Measurements (the harness needs pm-bridge with its harness feature)
(cd ../../engine && cargo build --release --locked -p pm-bridge-addon --features harness)
$L node scripts/frame-bridge-measure.mjs --skip-build --tag p2gate --only tracer-drag-today-500-idle,...
PM_TRACER_MEASURE=1 $L bunx playwright test e2e/native-engine-tracer-measure.spec.ts --workers=1
$L node scripts/frame-bridge-integration.mjs --skip-build
$L node scripts/test-mcp.mjs --desktop
$L node scripts/gate-engine-switch-corpus.mjs --json switch.json

# Bars (no Electron)
cd ../../engine
cargo test --release -p pm-time --test exhaustive -- --ignored
cargo test --release -p pm-render --test color --test scene3d --test corpus -- --nocapture
cargo test --release -p pm-export --test bars --test time_bar -- --include-ignored --nocapture --test-threads=1
cargo build --release -p pm-cli && node ../apps/desktop/scripts/gate-quicktime-color.mjs
target/release/pm-cli parity render
```

`gate-quicktime-color.mjs` renders the chart through `pm-cli render` (H.264, ProRes 422
HQ, ProRes 4444; `--color quicktime`) and measures each movie with
`gate-quicktime-color.swift` (AVPlayer and `AVPlayerItemVideoOutput` offscreen, 64-bit
half-float and 8-bit BGRA deliveries, the frame's attached color space to the main
display's profile and to Display P3 through ColorSync, CIEDE2000 on 32 × 32 patch-center
means, with Sharma's vectors checked before measuring). Neither opens a window.

## Fixed after the gate

Notes appended on 2026-10-16 (`feat/ne-correct`, `docs/engine/gate-fixes.md`); the
measured results above are as they were.

- **D1, fixed after the gate (2026-10-16).** `pm-render` pads video source times per
  output kind as today does (`RenderRequest::video_end`: 0.04 s for the viewer, scrubs,
  and thumbnails; `1 / fps` for exports and agent stills, `prepareFrame`'s padding).
  `pm-render/tests/video_end.rs` renders a 20-frame clip at 10, 24, 25, 30, and 60 fps
  for both kinds against today's rule (fails at 30 fps without the fix). With
  `POWERMOVE_NEW_ENGINE=1`, `export-video-frames.spec.ts` passes 9 of 9.
- **D2, fixed after the gate (2026-10-16).** A project installed by assigning `PM.proj`
  now lays the viewer out after the engine loads it, as opening a project does, so the
  viewer's state names the new composition and it plays on the engine clock. The Play
  start: the engine opened the audio device at the first Play (about 150 ms) and held
  every run, silent ones too, for the output's first buffer (160 ms on the AirPods);
  today's transport warms its device while idle and starts its playhead at Play. A
  composition with nothing to hear now plays on the host clock from Play (run start
  50 ms after Play, the frame pre-roll), and the device opens ahead of the first Play
  once the composition is audible (first Play 181 to 198 ms, the output latency plus
  pre-roll, kept for A/V sync). With the flag on, `export-mp4.spec.ts` passes 2 of 2
  and `native-engine-tracer.spec.ts` 10 of 10.
- **R1, fixed after the gate (2026-10-16).** Not the store lane's restore path itself:
  the legacy export never waited for the compositor's WebGL context, which the viewer
  makes when it mounts; after the relaunch the open and the frame preparation used to
  finish after the viewer had mounted, and from the store wave 3 merge on they often
  finish first (instrumented runs at the render merge, the store merge, and the tip in
  `gate-fixes.md`). `prepareFrame` now waits for the context. `import-image-sequence.spec.ts:7`
  `--repeat-each=12`: 24 of 24 with the flag off, 24 of 24 with it on.
