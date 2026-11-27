# E2E ledger

Phase 0 baseline for the desktop Playwright suite (plan section 25, "E2E ledger").
Every spec in `apps/desktop/e2e` is green, quarantined with an owner and a reason,
or skipped because the machine lacks something it needs. This file and
`apps/desktop/e2e/quarantine.json` are owned by the Parity and QA lane.

## How to run

```sh
cd apps/desktop
bun run build
bun run test:e2e:ledger          # the suite minus quarantine.json
bun run test:e2e                 # everything, quarantined tests included
```

`test:e2e:ledger` uses `playwright.ledger.config.ts`, which extends
`playwright.config.ts` and changes only three things: it skips the tests listed in
`e2e/quarantine.json`, rejects malformed or stale entries (unknown spec file,
missing reason, owner that is not a section 27 lane), and raises the global timeout
to two hours because one worker needs about 16 minutes for the unquarantined suite
(38 minutes for everything with retries), past the base config's 10 minutes. No spec
file is modified. Every run uses the existing hidden harness: windows are created
with `show: false` and `focusable: false` (`POWERMOVE_BACKGROUND_TEST=1`), and each
launch gets its own temporary `POWERMOVE_USER_DATA` profile.

Rules for `quarantine.json`:

- One entry per test, with the exact test title. An entry without `tests` skips the
  whole file; none do today.
- `status` is `fail` (fails every time), `flaky` (passes sometimes), or `env`
  (fails only on machines missing something).
- `owner` is a lane from plan section 27. `reason` says what fails and why.
- The change that fixes a test removes its entry and its row below.

## Baseline run

| | |
|---|---|
| Date | 2026-10-12 |
| Code under test | `feat/native-engine` at `b2931251`. The ledger branch adds only `playwright.ledger.config.ts`, the `test:e2e:ledger` script, `e2e/quarantine.json`, and this ledger, so application code is identical |
| Machine | Reference machine (decision 11): M4 Max, 128 GB, macOS 27; other lanes were compiling Rust at the same time, and the e2e run was niced |
| Command | `playwright test --shard=i/8 --workers=1 --retries=1 --global-timeout=0`, shards 1 to 8 run one after another, one Electron instance at a time |
| Tests | 547 tests in 228 spec files |
| Passed | 466 |
| Failed | 57, all on both the first attempt and the retry |
| Flaky | 0 by retry; 1 passing test reclassified as flaky by follow-up runs (below) |
| Skipped | 24 tests in 16 files, all environment-dependent |
| Wall time | 2,297 s (38 min 17 s) for the 8 shards, including a production build in each shard's global setup |

Follow-up runs on the same code:

- **Comparison with the commit before the Phase 0 legacy fixes.** The 42 spec files
  with failures were run once (no retries) at `b1d3e7f7`, the integration branch
  just before the `legacyfix` lane merged (export of that tree, same dependencies).
  All 57 failures also fail there, so the Phase 0 legacy fixes introduced none of
  them. One extra test failed there: `project-file-media.spec.ts:9`, which passed in
  the baseline run.
- **`project-file-media.spec.ts:9`** then failed 5 of 5 isolated repeats and 3 of 3
  whole-file repeats on `feat/native-engine`. It passed once in 10 runs, so it is
  quarantined as `flaky`.
- **Blender.** `blender-motion.spec.ts` (3 tests) skipped in the baseline run because
  no Blender was at a discovered path when shard 2 ran (`POWERMOVE_BLENDER_PATH`,
  `/Applications/Blender.app`, `~/Applications/Blender.app`, `PATH`); the only copy was
  a versioned bundle, `~/Applications/Blender-4.5.14.app`. With `POWERMOVE_BLENDER_PATH`
  pointing at it, all 3 passed (30 s). `/Applications/Blender.app` (Blender 4.5.14 LTS)
  appeared on the machine at 00:30 during the baseline run, and the full ledger run
  below found it and passed all 3 without the variable.
- **Ledger run.** `bun run test:e2e:ledger` over the 42 previously failing files
  ran 69 tests: 68 passed, 1 skipped (environment), 0 failed, in 113 s. The full
  ledger run is recorded at the end of this file.

## Failures by cause

Counts after the Phase 0 amendment 14 fixes (section "Amendment 14 rerun" below);
the baseline count is in brackets.

| Cause | Tests | Notes |
|---|---|---|
| Spec reads the sealed `window.powermove` bridge | 0 [21] | Fixed by the e2e bridge seam: hidden test launches publish the same bridge as `window.__powermoveTestBridge` (`src/shared/test-bridge.ts`); the specs use it. `window.powermove` stays sealed for page code |
| Relaunch does not restore the last active project | 0 [5] | Fixed: quitting answered the close prompt's Don't Save, which since `61425cc8` discarded every never-saved project, so the next launch had nothing to restore. Don't Save at quit now keeps the local project; closing a window or tab still discards a never-saved one. `timeline.spec.ts:459` was in this family too (it renames, relaunches, and reads the tab) |
| Spec predates a UI change | 6 [6] | Stale selectors or expectations: text animator presets, Settings tabs, color picker anatomy, toast details button. `effect-authoring.spec.ts` (renamed effect id) and `compatible-authoring.spec.ts` (staging prompt wording) were updated |
| UI behavior differs from the spec | 16 [18] | Some are regressions, some are intended changes the spec missed; each needs a decision before its test is rewritten or the product fixed |
| Agent requests carry extra images | 2 [2] | Requests include viewer-capture JPEGs the specs do not expect |
| Rendering, evaluation, text, and persistence | 2 [5] | Precompose pixels, the text anchor, and history on disk are resolved (below). Left: save responsiveness and Studio Cube property controls |
| Flaky | 1 [1] | `project-file-media.spec.ts:9` |

By owner (27 quarantined): Viewer and settings UI 21, Export and agents 2, Text and
vectors 1, Renderer write path 1, Proxies and storage 1, 3D 1. Lanes without engine
scope for a test (for example a toast layout) get it through the closest UI owner;
Parity and QA tracks all of them.

## Failing and flaky tests

Each test below failed on `feat/native-engine` as-is: on the first attempt, on the
retry, and in the comparison run at `b1d3e7f7`, and again in the amendment 14 rerun.
The flaky test is described in its row. Rows for tests fixed by amendment 14 were
removed; they are listed in the next section.

| Spec | Test | Status | Owner | Error and reason |
|---|---|---|---|---|
| `ae-audit-implementation.spec.ts:9` | editable Pen, canvas text, graph and preview controls render coherently | fail | Text and vectors | Timeout waiting for the "Text animator presets" group and its Rise button; that group no longer exists in src (removed in a5e3fbb6, 2026-10-05). Stale spec; the text animator UI it drives was redesigned. |
| `agent-attachment-preview.spec.ts:3` | agent image attachments open a fitted preview and Escape returns focus | fail | Viewer and settings UI | Escape closes the preview but focus does not return to the "View reference.png" trigger. Pointer-clicked buttons are blurred since 352f1d1e (2026-10-09, "Fix pointer-clicked buttons retaining keyboard focus"), so the trigger the spec clicks has no focus to restore. Product and spec disagree; decide which behavior is intended. |
| `agent-media-tools.spec.ts:41` | the agent watches and listens to imported footage through the media tools | fail | Export and agents | The final model request carries 8 data:image attachments; the spec expects 4 (2 frames + 1 sheet + 1 composition sheet). Suspected extra visual-context frames from e7e3ccb1 ("Improve agent visual context"). Agent snapshots move to the engine, so the count matters for the port. |
| `captions-ui.spec.ts:42` | captions surfaces in light mode | fail | Viewer and settings UI | Escape does not close the Export dialog, so Export Captions opens on top of it and locator('.export-modal') matches two dialogs (strict mode). Suspected cause: pointer-clicked buttons are blurred since 352f1d1e (2026-10-09, "Fix pointer-clicked buttons retaining keyboard focus"), after which the dialog no longer holds focus for Escape. |
| `captions-ui.spec.ts:42` | captions surfaces in dark mode | fail | Viewer and settings UI | Same failure as the light-mode case: two .export-modal dialogs (Export and Export Captions) are open at once. |
| `color-picker-overlay.spec.ts:4` | color picker stays above adjacent panels and inside the viewport | fail | Viewer and settings UI | The first open passes; after Escape the inspector Fill row no longer contains a .color-field trigger and the loop times out (30 s). Suspected stale selector after the color picker unification (f8734eb3, 2026-09-25). |
| `design-language.spec.ts:3` | flat controls remain consistent across home, editor, settings, library, and export in both themes | fail | Viewer and settings UI | In the dark editor the selected timeline mode button ("tl-mode-button on", Layers) draws a 1 px box-shadow ring, which the flat-controls rule forbids. The mode switch arrived in 19c910e4 (2026-10-03). |
| `dropdown-toggle.spec.ts:6` | native dropdowns reopen and custom pickers toggle on repeated clicks | fail | Viewer and settings UI | The 19th menu button, "Caption actions" (captions panel, 416e3a45, 2026-10-04), sits outside the viewport in the default layout, so the click never lands and the test times out. |
| `editor-responsiveness.spec.ts:65` | saving a large undo history yields to input and preserves the file | fail | Renderer write path | Longest main-thread gap while saving a 36 MB undo history was 220.6 ms (limit 100 ms) on both attempts and on the baseline run. Measured on the reference M4 Max while other lanes were compiling at nice 15; not yet re-measured on an idle machine. Amendment 14 rerun: 132.9 ms (limit 100 ms), machine shared with other lanes. |
| `extension-sandbox-clipboard.spec.ts:9` | a sandboxed panel imports a 5 MB image, and copies only with the clipboard permission | fail | Viewer and settings UI | Clicking inside the sandboxed panel never arms the gesture (outcome.armed stays false for 15 s). Same family as extension-sandbox.spec.ts: pointer input does not reach or focus the sandboxed view iframe in the hidden harness. |
| `extension-sandbox.spec.ts:52` | a store extension’s Svelte panel renders in its own view iframe with host chrome and keys | fail | Viewer and settings UI | Clicking the empty panel area leaves document.activeElement with no class (expected ext-panel-frame), so the sandboxed view never takes focus. |
| `media-bulk-selection.spec.ts:20` | drag a selection box, delete media, and undo the whole batch in both themes | fail | Viewer and settings UI | Dragging across the media grid never shows .asset-selection-box (10 s). The selection box still exists in src; the drag start point or the library layout changed under the spec. |
| `panel-popout.spec.ts:3` | a panel opens in a native window, keeps edits, and returns to the layout | fail | Viewer and settings UI | getByRole(textbox, "Project notes") matches two textareas: the docked Notes panel and the Library's "Edit Notes" card preview. Strict-mode violation; spec or Library card markup needs to disambiguate. |
| `panel-workflow.spec.ts:19` | library shows a panel grid, adds panels to the workspace, and edits panels with the agent | fail | Viewer and settings UI | The Library shows 14 panel cards; PM.PANELS (minus toolbar) has 15. One registered panel is missing from the Library grid. |
| `precomp.spec.ts:46` | canvas and timeline open the same compact layer menu | fail | Viewer and settings UI | The timeline layer menu has "Import Captions…" and the canvas layer menu does not (captions, 416e3a45). The two menus are meant to match. |
| `project-file-media.spec.ts:9` | saved project files restore imported video without the original session media | flaky | Proxies and storage | After reopening a saved .pmv, the metadata-only re-save returns false. Passed once in the baseline suite; failed 9 of 9 later baseline runs and 4 of 4 runs after the amendment 14 fixes. Unrelated to the restart-restore fix (the spec reopens the file explicitly). |
| `project-settings.spec.ts:3` | Settings › Project edits the live composition and its export settings | fail | Viewer and settings UI | Settings tabs are project, general, accounts, transcription, extensions; the spec predates the Transcription tab (d63f9e55, 2026-10-04). Stale expectation. |
| `project-settings.spec.ts:57` | New project asks for the settings that matter before creating | fail | Viewer and settings UI | The Background color dialog has no .fill-sv plane; the color picker anatomy changed in f8734eb3 ("unify color pickers", 2026-09-25). Stale selector. |
| `prompt-attachments.spec.ts:5` | floating prompt attaches images and binary files, removes them, and sends their contents | fail | Export and agents | CodexBridge.request receives 4 images (the attached PNG plus three 480x270 JPEG viewer captures); the spec expects only the attachment. Suspected visual-context frames from e7e3ccb1 bypassing the stubbed AgentHarness.observe. |
| `property-animation.spec.ts:68` | text, shader, extension, blend, and mask stopwatches animate their real values | fail | 3D | The Studio Cube extension layer's "x.objectColor" stopwatch resolves but is never visible (collapsed or hidden section), so the click times out. |
| `shell.spec.ts:260` | keeps a usable window drag area with crowded tabs and overlays | fail | Viewer and settings UI | The Library screen top is 42.32 px; the spec requires ≥ 44 px so it clears the titlebar drag area. |
| `timeline-layer-rows.spec.ts:3` | compact layer rows keep names centered and parenting in dedicated controls | fail | Viewer and settings UI | Clicking the row disclosure for the nested composition never reveals its child row "Prompt" (30 s timeout). |
| `timeline.spec.ts:100` | the easing grid applies a curve to both Scale dimensions and preserves linear defaults | fail | Viewer and settings UI | Choosing "Ease out" in the keyframe easing grid leaves both Scale keys at ei [1, 1] (expected [0.58, 1]); the menu item click does not apply the curve. |
| `toast-error.spec.ts:61` | the error toast matches the status toast it stacks with | fail | Viewer and settings UI | The error toast has no "Technical details" button; it was removed in dbd1c23a ("Redesign agent errors", 2026-09-22). Stale spec. |
| `user-feedback.spec.ts:5` | background picker keeps every channel above the hex field | fail | Viewer and settings UI | The Background color dialog has no .color-dialog-value element (removed in f8734eb3, "unify color pickers"). Stale selector. |
| `viewer-resize.spec.ts:4` | resizes the selected layer in local space with a fixed opposite edge and correct cursors | fail | Viewer and settings UI | Dragging the east handle leaves scale.x at 100. The handle hit-test works (the resize cursor matches) but the drag does not resize. |
| `viewer-resize.spec.ts:199` | resizes and rotates a multi-selection around the common pivot with one-step undo | fail | Viewer and settings UI | The multi-selection resize drag leaves the shape scale at 100, the same failure as the single-layer case. |

## Amendment 14 rerun

Plan section 26, amendment 14: the legacy fixes that precede the corpus v1 capture.
Branch `feat/ne-legacy2` (from `feat/native-engine` at `bb013c14`), 2026-10-12, same
reference machine, other lanes building at the same time, e2e niced.

**Fixes and conclusions**

| Item | Cause | Change | Tests |
|---|---|---|---|
| Restart restore | Quitting runs the same Save / Cancel / Don't Save prompt as closing a window. Since `61425cc8` (2026-10-02) Don't Save discards every never-saved project, so the restored window session named a project that no longer existed and the app booted to Projects with a fresh Untitled. The e2e harness answers every message box with Don't Save, which is how all relaunch specs hit it; a user who quits and picks Don't Save (the prompt promises "Local recovery is kept separately") hits the same thing | Main tells the editor when a close is part of a quit (`prepareToClose({ quitting: true })`); Don't Save then skips only the file write. Closing a window or a tab still discards a never-saved project. Boot publishes `PM.projectAssetsReady` like a project switch | `app.test.ts` (quit keeps, window close still discards); `project-close-unchanged.spec.ts` new case; 6 relaunch specs pass |
| Mixer after restore | A restored project mounts the Mixer before its panel is in the rendered page; the palette read returns empty strings and the first meter draw throws in `addColorStop` (surfaced as page errors in `playback-audio.spec.ts:100` once restore worked) | Empty palette entries keep the previous colour; the panel reads the palette again when shown | `meter-loop.test.ts` |
| Bridge seam | Since `bfcd2c88` the kernel seals `window.powermove` at boot (intended) | Hidden test launches (`POWERMOVE_BACKGROUND_TEST=1`, isolated profile) start editor windows with `--powermove-test-bridge`; only then does the preload also publish the same bridge as `window.__powermoveTestBridge`. Normal launches never pass the switch | `test-bridge.test.ts`, `background-testing.test.ts`; 21 specs updated; `security.spec.ts` now checks the seal and that the seam carries no Node or Electron |
| Gradient Tint id | The shipped sample registers `gradient-tint.effect` since `512b5ead` (registration ids carry the extension id); `compatible-authoring.spec.ts` also matched system-prompt wording that changed | Specs updated | `effect-authoring.spec.ts:5`, `compatible-authoring.spec.ts:6` pass |
| Blender discovery | Only `Blender.app` was looked for | Every `Blender*.app` in `/Applications` and `~/Applications` (unversioned first, then newest first), every install folder under `Blender Foundation` on Windows; `POWERMOVE_BLENDER_PATH` stays first; an older Blender found first no longer hides a supported one | `blender-service.test.ts`; `blender-motion.spec.ts` 3 of 3 pass |
| `ae-audit-implementation.spec.ts:42` | Real bug. Since `f5f993d3` pre-compose moves layers into a nested composition and cuts parent links to layers left outside; pre-composing a child without its animated rig moved it (mean difference 12.17 CV at t = 0.4, 1, 1.8) | When every moved root hangs from the same outside layer, the precomp layer takes that parent (its own transform is the identity). Mixed outside parents are still cut. Known limit: the precomp is the host's size and sits in the rig's local space, so moved content that lies outside `[0, w] x [0, h]` in the rig's local coordinates (a child at a negative offset from its null, for example) is clipped by the precomp frame. The spec's child sits at (+100, +100); a negative-offset case is not covered | Mean difference now 0.0010 to 0.0015 CV; `compositions.test.ts` |
| `font-anchor.spec.ts:6` | Intended behaviour. Since `c2139713` an alignment change drops the preserved anchor bounds so the pivot follows the alignment origin (unit-tested in `font-anchor.test.ts`); every sample after `align: 'right'` drifted because the spec expected the old rule | Spec checks that alignment releases the anchor, re-centres, and keeps the 0.001 px limit for font and animated size changes | Passes |
| `memory-history-persistence.spec.ts:5` | Real bug. History entries hold composition paths (`['comps', id, 'layers']`) since `f5f993d3`, but record sharing and the on-disk dictionary only recognised `['layers']` and `['edits']`, so undo history was neither shared in memory nor packed on disk | Both accept the composition paths; unpacking validates references with the same rule. The dictionary stays version 1: builds before this fix stored these histories raw, and a downgraded build reading a newly packed one rejects it (`Invalid history reference`), so a downgrade loses undo history. A version bump would not change that, since older builds reject unknown versions too | `history-memory.test.ts`; spec passes (format, cursor, fewer than 24 records) |
| Corpus: unlit glTF | `renderSnapshot` read `emissive` from a `MeshBasicMaterial` | `sourceMaterialSnapshot`: standard materials unchanged; unlit ones become emission; other types get the worker defaults | `rendering.test.ts` |
| Corpus: `vec3` colour `@param`s | The compositor took `@param` definitions from UI state, which the Inspector fills only for a layer it has shown; other shader layers bound colours with `uniform1f` (GL_INVALID_OPERATION, colour 0) | Definitions are parsed from the layer's code, cached by program key. `int`, `vec2`, and `vec4` scalar params keep today's binding (they stay 0 and raise GL_INVALID_OPERATION); known legacy behaviour, recorded in `docs/engine/corpus.md` | `shader-color-uniform.spec.ts` (fails with 1282 before, passes after) |

**Rerun.** Every previously quarantined spec file plus `blender-motion`,
`project-close-unchanged`, and `shader-color-uniform`: 46 files, 133 tests, one
worker, no retries, one Electron instance, 7.7 min. 105 passed, 27 failed, 1 skipped
(`editor-responsiveness.spec.ts:6`, needs `PM_PERF_PROJECT`; run below).

- 31 of the 58 quarantined tests now pass and left `quarantine.json`: the 21 bridge
  tests (one of them, `compatible-authoring.spec.ts:6`, also needed the Gradient Tint
  update), the 5 relaunch tests plus `timeline.spec.ts:459` (same family),
  `effect-authoring.spec.ts:5` (Gradient Tint), and the three investigated specs.
- The 27 failures are the remaining quarantined tests, each failing as described in
  its row. No test outside the quarantine failed.
- `project-file-media.spec.ts:9` failed again (4 of 4 including 3 repeats).
- `timeline.spec.ts:459` passed 3 of 3 repeats.
- `editor-responsiveness.spec.ts:65` measured 132.9 ms (limit 100 ms).

**Performance specs with Jude's project.** The decision 13 project was copied to
`~/Library/Application Support/Powermove Engine Corpus/perf/` outside the repository;
the original's hash was checked before and after and did not change.
`PM_PERF_FILE` is the `.pmv` copy; `PM_PERF_PROJECT` is a JSON document of its
`proj` and `ws` only (1.4 MB): the full decoded document is 134 MB because of its
undo history, and passing it to `page.evaluate` ran the Playwright worker out of
heap. All 9 tests passed (2.4 min, one Electron instance), and `project-playback-profile` with
`PM_PLAYBACK_PROJECT` set to the `.pmv` copy passed (2.1 min). Measured, machine shared
with other lanes:

| Spec | Result |
|---|---|
| `composition-playback-profile.spec.ts:6` | Playback render p95 4.3 ms over 240 frames; scrub render p95 1.6 ms over 51 |
| `editor-responsiveness.spec.ts:6` (`.pmv` opened through Open) | Scrub sample p50 8.2 ms, p95 15.8 ms, max 22.6 ms (80); render p95 8.0 ms (limit 16.7), max 15.3 ms (limit 100) |
| `high-zoom-performance.spec.ts:7` | 800% wheel pan frames about 8 to 9 ms; pinch median 8.3, p95 9.0, max 9.2 ms; high-zoom render p95 1.4 ms, frame p95 11.6 ms |
| `playback-performance-benchmark.spec.ts:7` | 29.98 to 29.99 fps actual at 30 fps (zoom 1 and 8), paused 0 fps, frame interval p95 34.1 ms |
| `project-performance-benchmark.spec.ts:5` | 884 layers at 1280x720: render p50 4.5, p95 5.9, max 6.2 ms (25); interactions 9 to 19.9 ms |
| `region-pan-performance.spec.ts:9` (3) | Median 8.3 ms, p95 9.3 to 10.4 ms, max 39.5 to 40.8 ms; 3 to 6 renders; pixels identical |
| `zoom-performance-benchmark.spec.ts:5` | First render at 50% 25.4 ms, at 100% 11.9 ms, then 0.8 to 1.6 ms; gesture 0.9 to 5.4 ms |
| `project-playback-profile.spec.ts:6` (`PM_PLAYBACK_PROJECT` = the `.pmv` copy) | 29.90 fps actual at fit and at 100%; render median 1.0 to 1.1 ms, p95 3.9 to 4.2 ms, max 11.8 to 18.5 ms |

## Skipped (environment-dependent)

These tests skip themselves when the machine lacks a private fixture, a model, a
font, a live account, or Blender. They are not quarantined: they never fail when
the requirement is missing, and they run when it is present. Private projects stay
outside the repository; per decision 13, Jude's "Powermove ad" project is the
candidate for `PM_PERF_PROJECT` on the reference machine.

| Spec | Tests | Requirement | Owner |
|---|---|---|---|
| `agent-panel.spec.ts:188` | 1 | `POWERMOVE_E2E_LIVE=1` and a signed-in Codex account (live run) | Export and agents |
| `blender-motion.spec.ts:5, 265, 452` | 3 | Blender at a discovered path or `POWERMOVE_BLENDER_PATH`. Discovery now also finds versioned bundles such as `~/Applications/Blender-4.5.14.app` (amendment 14); all 3 pass with Blender 4.5.14 LTS | 3D |
| `composited-video-sync.spec.ts:73` | 1 | `PM_SYNC_PROJECT`, a private layered cutout project | Media |
| `composition-playback-profile.spec.ts:6` | 1 | `PM_PERF_PROJECT`, a copied project; passes with the copies above (amendment 14) | Parity and QA |
| `editor-responsiveness.spec.ts:6` | 1 | `PM_PERF_PROJECT` or `PM_PERF_FILE`; passes with the copies above (amendment 14) | Parity and QA |
| `high-zoom-performance.spec.ts:7` | 1 | `PM_PERF_PROJECT`; passes with the copies above (amendment 14) | Parity and QA |
| `media-intelligence.spec.ts:161, 259` | 2 | `POWERMOVE_E2E_PARAKEET_DIR`, a downloaded Parakeet model | Audio |
| `playback-performance-benchmark.spec.ts:7` | 1 | `PM_PERF_PROJECT`; passes with the copies above (amendment 14) | Parity and QA |
| `project-export-fixture.spec.ts:11` | 1 | `POWERMOVE_EXPORT_FIXTURE`, a project file | Export and agents |
| `project-performance-benchmark.spec.ts:5` | 1 | `PM_PERF_PROJECT`; passes with the copies above (amendment 14) | Parity and QA |
| `project-playback-profile.spec.ts:6` | 1 | `PM_PLAYBACK_PROJECT`; passes with the `.pmv` copy above (amendment 14) | Parity and QA |
| `region-pan-performance.spec.ts:9` | 3 | `PM_PERF_PROJECT`; passes with the copies above (amendment 14) | Parity and QA |
| `transcription.spec.ts:281` | 1 | `POWERMOVE_E2E_PARAKEET_DIR` | Audio |
| `variable-font-sliders.spec.ts:4, 52, 88` | 3 | Geist Width variable font installed | Text and vectors |
| `variable-font-sliders.spec.ts:129` | 1 | SF Pro installed | Text and vectors |
| `video-playback-regression.spec.ts:26` | 1 | `PM_VIDEO_FIXTURE`, a private 4K source | Media |
| `zoom-performance-benchmark.spec.ts:5` | 1 | `PM_PERF_PROJECT`; passes with the copies above (amendment 14) | Parity and QA |

Conditional skips that did not trigger on this machine: `boot.spec.ts` (WebGPU
present), `gpu-recovery.spec.ts`, `native-export-performance.spec.ts`, and
`rendering-performance.spec.ts` (GPU reset extension available),
`extension-remote-media.spec.ts` (network available), `security.spec.ts` sandbox host
present, and the `test.fixme` guards in `export-webm.spec.ts` and
`persistence.spec.ts`.

## Ledger run

`bun run test:e2e:ledger` on the same code, after the quarantine list landed: one
worker, no retries, no sharding.

| | |
|---|---|
| Tests run | 489 (547 minus 58 quarantined) |
| Passed | 468 |
| Failed | 0 |
| Flaky | 0 |
| Skipped | 21 (the 24 environment skips above, minus the 3 Blender tests, which passed) |
| Wall time | 945 s (15 min 45 s) |

After the amendment 14 fixes (`feat/ne-legacy2`, 2026-10-12): the same command,
one worker, no retries.

| | |
|---|---|
| Tests run | 522 (549 minus 27 quarantined; the suite gained 2 tests: Don't Save at quit in `project-close-unchanged.spec.ts`, and `shader-color-uniform.spec.ts`) |
| Passed | 501 (Blender found through discovery) |
| Failed | 0 |
| Flaky | 0 |
| Skipped | 21 (environment; the `PM_PERF_PROJECT` and `PM_PLAYBACK_PROJECT` specs were run separately, above) |
| Wall time | 996 s (16 min 36 s) |

## Gate E — native engine integration, 2026-11-01

**OPEN.** This section is the complete capability/work ledger, not a claim
that the engine gates passed. The earlier Electron E2E measurements above
remain historical evidence. Native plan v6, N1–N8 and
`docs/native/p0-decisions.md` supersede older browser-fallback assumptions.
No capability failure authorizes a silent legacy render or loss of source data.

Base `ff42d751f`; the five author branches were merged **in order, --no-ff**.
[etrack-integration.md](etrack-integration.md) records all conflict paths and
rule decisions (19 L4, 2 L5, 44 Phase 4a, 67 bigrender, 22 SVG conflicts).
Only owned engine/documentation changes were installed. Existing native
implementations supersede much of the branches' old Chromium/Electron wiring;
those paths remain archived in ancestry. Phase 4a backend integration alone
is not end-to-end native-service integration.

### Exhaustive category and producer inventory

[gate-e.json](gate-e.json) records each enum category, its condition, evidence
and every production `Feature::…` reference in pm-render (a conservative
superset of constructors, including diagnostic consumers). It distinguishes
implementation from qualification: `native` does not mean an unrun Metal gate
passed. Run `python3 engine/tools/gate_e_inventory.py`; a new category or
reference fails the audit until reviewed. `--require-green` deliberately fails
while evidence/work remains unqualified. The audit is not a runtime corpus test.

**Before/after: 17 enum categories → 17.** Removing diagnostic variants while
consumers remain would violate the prune rule and hide failures. No new legacy
route was added. EXR/DPX admission and static JPEG/AVIF/EXR/DPX recognition now
use native readers; H.273 logarithmic tags no longer silently read as sRGB.
The outstanding runtime unsupported set is **not empty and not measured here**.

| Reason | Disposition | Condition and evidence |
|---|---|---|
| LayerType | native | Shipped layer types have handlers; unknown types remain explicit. `headless::capability_check_lists_what_the_renderer_cannot_draw`, `every_feature_is_supported_and_frames_are_deterministic` (Mac run required). |
| ExtensionDefinition | native / host binding required | Fragment/mesh definitions must be installed. Raster overrides need V8/recorder producers. `shader_layers`, `compat_rasters`; native export children still need definition/font replay. |
| Effects | native | Requires definitions/EffectHost. `effects`, `effect_proof`, corpus. Missing configuration remains a named failure. |
| Masks | native | No constructor remains. `composite`, `path_masks`; retain variant while referenced. |
| Transition | native | Native definition/host chains. `effects`, `effect_proof`; shader-resource gaps below also apply. |
| MotionBlur | native | All shipped visual types sampled. `motion`, `headless`; unknown future types remain explicit. |
| TwoAndAHalfD | native | Shipped plane types supported. `space25d_picks`, `headless`. |
| Adjustment | native | Native accumulator effects; definitions required. `effects`, `composite`. |
| CompatLevel2 | native | No constructor remains. `text_level2_semantics`, `sampling`. |
| ProjectColor | blocked-by | Standard, linear blending and saved depth are implemented (`linear`, `color_depth`); future wide-gamut/HDR/ACES project modes need the professional-color implementation. Preserve unknown modes; never reinterpret them as Standard. |
| RequiresEngine | native | Current stamp fields are drawn or checked independently. `capability::tests::every_stamping_field_is_classified` and the adjacent classification tests; retain the diagnostic for future fields. |
| ImageSequence | blocked-by | Persisted WebM rendition is native video. Direct original animation/sequence runtime providers are still missing. `sequence_rendition`, `pm-media::image::animation`, `pm-proxy/tests/sequence.rs` are pieces, not a playback proof. |
| Media | needs-macOS / specific blockers below | Native decode, ColorSync and Metal source pass. `stills`, `stills2`, `footage`, `video_reading`, `source_snapshot`; detailed subreasons below. |
| Request | needs-macOS | Real adapter precision/target limits and configured fonts. `headless::requests_are_validated`, `render_into_a_callers_target_equals_render`; invalid requests remain errors. |
| Definition | native / host binding required | Absent definition is an input/lifecycle diagnostic. `effects`, `shader_layers`; native child serialization remains required outside this lane. |
| ShaderFallback | blocked-by | S4 still has 13 WebGL-only cases. Native sampler/resource bindings and Metal compilation must replace that need; an old Chromium runner cannot satisfy N8. `shader_gate`, `pm-glsl::const_tables`, `uniform_types`, `docs/native/s4-glsl.md`. |
| Text | needs-macOS | CoreText/project fonts and execution-time paint diagnostics. `text_corpus`, `text_level2_semantics`, `text_gpu`, `text_raster`; a configured-font boolean alone is not the glyph-paint gate. |

All cited render gates need rerunning against the merged tree. Neither Linux
CPU tests nor historical author-branch measurements qualify them.

### Media subreasons (every producer family)

| Producer / subreason | Native work or precise blocker |
|---|---|
| `capability::check_media`, `plan` sequence branches | Persisted renditions work. Original animated GIF/APNG/WebP/AVIF and original sequence playback need an engine timestamp/frame provider and source registration. This is missing implementation, not merely unavailable hardware. |
| `media::still_facts`, `stills::check/static_format` | Native EXR/DPX admission added; native readers already existed. New `native_exr_dpx_and_png_metadata_are_supported` compares every sample with an independent standards conversion. Run on Mac (OpenEXRCore/FFmpeg + Metal). JPEG/AVIF static-format refusals removed because native handling is present. |
| `stills` header/layout, orientation, WebP ICC, HEIF HDR | Unsupported decoder layouts, malformed/oversized metadata and mismatched transforms remain explicit. WebP ICC admission needs reader-to-ColorSync/float verification and any missing implementation; never call unverified reading correct. HEIF PQ/HLG and gain-map stills (HEIC, JPEG, AVIF) are read with their transfer and drawn as HDR sources (`hdr_stills`). Run `stills`, `stills2`, `hdr_stills`, `heif_source`, `png_gamma`, source snapshots and material maps. |
| `media::still_reading`, `stills::avif` unknown color descriptions | H.273 Log100/Log316 supported through probe/interpretation/CPU/shader. Other unspecified or unsupported primaries/transfer/matrix readings remain named; no invented curve. Test `h273_log_png_tags_decode_in_float_at_every_sample` on Mac. |
| `media::video_supported` | Removed: native has no other renderer. Interlaced, anamorphic, cropped-aperture, wide-gamut and HDR sources draw by default (`footage::every_source_with_a_reading_draws_without_an_interpretation`, `hdr_delivery`). |
| `media::video_probe_drawable` | Off-quarter-turn display rotation needs native geometry support. Dolby Vision without a decodable compatible base layer needs an approved native decoder/reading. Unknown primaries need explicit interpretation. Hardware alone does not implement these. |
| `media/prewarm` and execution-time media errors | The same decode/reading checks apply at execution and ahead-of-time preparation. Prove refusal leaves caller targets untouched (`headless::exec_time_unsupported_media_leaves_the_target_untouched`) and file replacement invalidates caches (`a_video_replaced_on_disk_is_reopened`). Offline/unreadable media stays named and original data retained. |

### Remaining E-track work and exact Mac handoff

| Item | Current evidence and remaining work |
|---|---|
| L7 / AUDIO-DEV-1 | Portable `pm-audio::device_policy` has 5 passing Linux tests and a CoreAudio `SystemDeviceProbe`. **Not wired into automatic stream reopening yet.** The native audio owner must commit a snapshot only after successful stream/schedule adoption, invalidate epochs, and bind actual reopen for device/rate/channel changes. Test unplug/replug, same-id 48↔44.1 kHz and stereo↔multichannel, Bluetooth/USB/default changes, no device, latency/buffer changes and callback sizes through 8192. Existing `pm-audio` playback/clock/mixer/stretch/source suites and physical speaker/display sync remain required. AUDIO-2 cold-start bound, allocation ratchet and listening evidence remain open in pm-audio.md. |
| N3/N8 audio | Existing Chromium `libffmpeg` decoder dependency remains a blocker. Export now mixes with the playback mixer, so authored speed and remaps are heard (2026-11-06, `an_export_mix_*`). Do not claim N8 because Node itself is absent. |
| Log interpretation | H.273 transfer 9/10 is wired natively; CPU analytic/monotonic/round-trip proof and production WGSL validation pass. Proprietary camera log/gamut pairs are still outside the public source-reading contract; add exact curve+gamut definitions and test vectors before admitting them. Do not guess from codec/filename. No UI decisions made. |
| Standards | CHROMA-420-1's decimation quality remains open (export.md). The current inverse preserves the round trip but is not the complete antialiasing fix. Mac: f64 lossless encoder-input bars, BT.709/sRGB tags, range, alpha, PNG depth, GPU-vs-CPU conversion and matched reference charts. |
| WebM settings | Native libvpx realtime settings retained, no RGBA8 canvas round trip restored. Run `pm-export` WebM/Opus/format/delivery tests, check decoded alpha, every frame, tags, priming/presented duration, and matched-quality throughput against shipped Electron. A realtime setting is not itself a throughput pass. |
| SHADER-RES-1 | Preserve source, logical resolution, `u_res`, coordinates and uniform calls. Constant-table hoisting is integrated; native compiler prewarming is source-specific and retired on device release. Run S4 inventory and `shader_gate` on Metal. Implement missing sampler/resource bindings; no Chromium fallback can close this row. |
| Phase 4a | `pm-proxy` policy/store/manifest and `pm-cache` backend are integrated. Generation is optional for Linux tests; default Mac feature uses VideoToolbox/Apple media. Native service must bind `GenerateProxy`, store capabilities, cancellation/contention/disk pauses, policy at clip boundaries, original-only normal export/analysis, explicit draft export, and exact preview-cache identities. Native service paths are outside ownership (`apps/native/**`). Then run `pm-proxy` codec/chart/disk/sequence tests; original-vs-proxy export byte identity; moves/relinks/relaunch; paused original upgrade ≤150/300 ms; ΔE chart; hour-long memory/playback and disk guard. |
| Bigrender | Prepared tracks coexist with channel memo, release on evaluator reset and survive counter wrap; native programs can prewarm on a worker. Actual peak resource accounting, pending corpus fingerprints, cold/warm open/seek/playback/export, shader compilation cancellation, hot reload/device retirement and required 60/120 Hz machine rows remain Mac gates. |
| Frozen corpus / whole engine | Run every corpus/performance/local-extension project with original media, fonts and definitions present; require empty unsupported set. Run preview=export, source monitor/filmstrip, GPU loss, native child exports, audio timing and full engine clippy/tests. External captures and Apple hardware are unavailable here. |

Recommended Mac commands, under the Mac lane's single-build lock and configured
FFmpeg/native library environment:

```sh
source engine/env.sh
cd engine
cargo test --locked -p pm-media -p pm-gpu -p pm-render -p pm-export -p pm-audio -p pm-node -p pm-proxy -p pm-vector -p pm-eval
cargo clippy --locked --all-targets -p pm-media -p pm-gpu -p pm-render -p pm-export -p pm-audio -p pm-node -p pm-proxy -- -D warnings
```

The unfiltered Linux protocol suite has one failing test:
`wire::typescript_regeneration_is_a_no_op`. The protocol source hash changed;
the generated desktop `index.ts` needs `protocol-f1801e99aec59477`.
[etrack-desktop-protocol.patch](etrack-desktop-protocol.patch) is the exact
one-line consumer update, prepared without editing an unowned path. The desktop
owner must apply it with `git apply --unidiff-zero docs/engine/etrack-desktop-protocol.patch` or regenerate normally. The original test and draft
handshake remain strict. The broad passing replay explicitly skips only that
known cross-lane failure; it is not an all-green full-suite result.

The Linux validation counts and exact command limitations are recorded in
[etrack-integration.md](etrack-integration.md). **Gate E stays open until the
above missing implementations and physical/corpus proofs close.**


### macOS integration evidence — base Apple M5 (2026-11-01)

The native engine integration now also includes the export-parity lane. Its
first complete macOS replay passed **139 pm-export tests**, with **7 existing
ignored measurements/manual cases**. The three reported export failures were
reproduced and repaired: AAC startup/flush attribution, archive-aware atomic
replacement, and the AVFoundation chart reader's packed RGB conversion.
The numeric bounds, decoded lengths, rates and channel checks remain intact.
See [export.md](export.md#macos-integration-replay-base-apple-m5-2026-11-01).

The generated desktop protocol id is updated and the strict regeneration test
passes. `cargo check -p pm-native-app` also passes after the export merge. The
complete native replay passed **pm-native-app 28 tests (1 ignored)** and
**pm-promedia 116 tests**. Engine totals and remaining gate results are recorded
in [etrack-integration.md](etrack-integration.md).

The original frozen corpus and the original real project are unavailable on
this Mac. Fresh captures use the documented tool with `--label v1`, synthetic
projects only, a temporary Electron profile, and
`--root /private/tmp/pm-engine-corpus`. **These were recaptured on the build Mac
(base M5); they are not the original frozen capture.** Capture and parity run
one category at a time; captures are deleted after their results are recorded.
No original pending fingerprint or numerical bound is replaced by these new
measurements.

The first category captured 12 layer projects, 34 frames (3 float frames),
and 34 evaluation dumps; verification found no missing or unlisted files.
The renderer compared 30 frames and found a binding-quirk/reference mismatch.
The initial replay also reported unvisited pending entries outside that single
category; subsequent category runs use the explicit category filter and retain
the global qualification requirement. Gate E remains **OPEN**.


G-V2 was also recaptured on this M5 using all **54** synthetic mask cases and
the current legacy coverage source. The Metal gate passed every case against
those new witnesses and the independent f64 coverage/Gaussian reference,
including HD/UHD, subtraction, feathering and exact unfeathered geometry.
Its precision and parity bounds are unchanged. Category captures were removed
after recording results. This closes the Mac mask execution check, not the
whole-engine frozen-corpus or physical/performance qualification.

The Canvas/vector witness was also recaptured on this M5: **556 synthetic
cases captured**, **474 compared** by G-V1, and **18 Rust tests passed**. The
vector
gate's edge max/p99.9/mean checks and exact stroke regions passed unchanged;
its capture was deleted after results were recorded.

The Mac S4 inventory ran the bounded native compiler and Metal draw, including
MSL byte-equivalence checking: **812 source cases / 2,143 variants**, **656
successful Metal draws**. The other cases stopped at ES validation (115),
resource binding (33), source limits (5), or Naga validation (3). This set
includes deliberate negative probes. It establishes execution evidence, not a
claim that all input sources are supported or that the historical WebGL pixel
gaps are qualified. Private shader sources, diagnostics and pixel digests are
excluded from this ledger.

G-V3 also passed against fresh M5 witnesses: **383 Chromium semantic probe
captures**, **374 probe rasters compared** (existing documented intrinsic-size
and radius decisions retained), plus **43 actual legacy-app frame captures**
for common placements, 25–800% scale and animated scale. The latter replay
passed **8 Rust tests**, including both frame gates. Source and frame hashes,
versions, dimensions and premultiplied row encoding were checked. Engine
frames stayed RGBA32F, independent f64 references and all existing numerical
bounds stayed unchanged, and each capture set was removed after recording its
results. These are new build-Mac witnesses, not the original frozen captures.

The final replay of the 15 requested crates' library targets passed **908
tests, 0 failed, 2 existing ignored**. The extra Mac replay passed
**pm-node 254 tests (21 existing ignored)** and **pm-vector 73 tests**.

The seven-crate Mac `cargo clippy --locked --all-targets ... -- -D warnings`
check passes. Mechanical lint repairs retain the existing conversions and
assertions; the two reference painters now share one SVG reference module.
The Mac encoding-core replay with `pm-export --no-default-features` also
passes **69 tests, 0 failed**. Its QoS callback installer is available to the
Mac unit tests even when the renderer integration feature is disabled.

### Frozen-corpus run on the owner's Mac, 2026-11-07

Head `d324a46f4` (lane `feat/na-w15-engine`), M4 Max (12P + 4E), macOS 27,
shared with other lanes (load 15 to 90). Original frozen witnesses under
`PM_ENGINE_CORPUS_ROOT`; same-checkout `pm-glsl-compiler`. The 17 HANDOFF step 3
packages (plus `pm-text` and `pm-perfbench` in the second run), every test
binary run to completion with `RUST_TEST_THREADS=3`, as `--no-fail-fast` does.

| Run | Binaries | Pass | Fail | Ignored | Harness `main` failing |
|---|---:|---:|---:|---:|---|
| Head, before this lane | 180 | 2,633 | 68 | 74 | `text_corpus` |
| After this lane's fixes | 188 | 2,815 | 6 | 74 | `text_corpus`, perf `ratchet` |

**Missing captures no longer pass.** On a machine with the corpus, a missing
capture family fails the gate; with no corpus it prints `NOT RUN`. The same
holds for the scene3d, glTF and motion witnesses and the generated media
corpus (`pm-render` `color`, `retime`, `scene3d*`, `pm-export` `bars`). The newest
capture is chosen by manifest `createdAt`: `threed-*` had resolved by name to a
one-project capture instead of the three-project roles/maps capture. Tests that
still did not run in the second run, counted as not run: 4 `pm-audio`
`chromium` tests (no Electron `libffmpeg` in a native checkout) and
`effects_webgl` perturbation (no perturbed WebGL capture).

**Failures fixed (63, harness or setup):**

- 60 `advanced_effects`, 2 `effect_versions`, 1 `pm-node` `compose`: the
  two-slot GLSL compiler answered busy to parallel test fixtures. A test-only
  `pm-glsl` feature makes the default pool wait for a slot within the deadline;
  shipped admission still never queues.
- `pm-glsl` `source_shader`: the test left out the camera log module that the
  source pass includes.

**Open:**

| Test | Class | Detail |
|---|---|---|
| `corpus_render_parity` | Real, unattributed | 362 frames compared, 488 failing checks. All 221 pending rows changed (180) or resolved (41). Measurements got worse, mostly "resampled image content" on transparent float frames (mean 0.06 → 6.4 CV, p99.9 1 → 107); opaque 8-bit frames of the same projects stay within 2 CV. At `40f9ee7c9` (the commit that recorded the rows) `render/fx-blur` passes; at `80bc541dd` and head it fails. Commits from the cloud etrack integration between them do not build, so bisection stops there. Not re-pinned. |
| `text_corpus` (harness `main`) | Real plus missing witness | 48 frames: unowned text reference gaps and pending rows changed as above; 5 rows "not measured" because no `text-proj-*` capture exists. |
| `scene3d`, `scene3d_gltf` G-D1 | Setup | Need `PM_SCENE3D_FLOAT_REF` / `PM_GLTF_FLOAT_REF` from `examples/scene3d_float_reference.ts`, which needs a Chromium bundle. Not generated. |
| `playback` AUDIO-2 and scheduled-play line | Timing under load | AUDIO-2 passed in run 1 (7.3 to 8.1 ms, load 40) and failed in run 2 (13.7 to 15.7 ms, load 86, exports running). The 10.667 ms bound is unchanged. A shorter fresh-start grid cut the work but worsened fresh-start fidelity (3.58 dB LSD over the 3 dB bound), so it was not kept. |
| `pm-audio` `legacy_parity` `conformed`, `animated-trim` | Real, unattributed | Without today's FFmpeg both now leave a −110.9 dBFS residual against their −240 dBFS ceiling, like `embedded`'s AAC decode. Which merge made them decode the AAC soundtrack through our FFmpeg is not found; the ceiling was not raised. |
| perf `ratchet` | Real, pre-existing | `drag_500` heap bytes +0.8% and `blend_stack_cold` heap bytes +3.7% over their baselines on the merged head before this lane. `text_cold` passes again (see perfbench.md). |

**Real projects (private copies; names stay out of Git).** Three local
projects, 28.8 s to 87.4 s at 1080p, two of them drawing 39 user extensions'
effects through `pm-cli render --contributed` (the definitions
`pm-compat-runner` reads from the installed extensions). Every project
exported its full duration in ProRes 422 HQ, ProRes 4444 with alpha, H.264,
HEVC Main 10 and PNG: exact frame counts, profiles and bit depths, every frame
decodes, 24-bit PCM or AAC audio. Decoded frames against the same render as a
still: ProRes 4444 within 2 CV on opaque pixels with transparent background
transparent; 4:2:2 and 4:2:0 deliveries 43 to 77 dB PSNR (their maximum errors
sit on sharp chroma edges). The one project with an Electron capture (v1
`real/`, 12 sample times) matches it within 1 CV on 98.5 to 99.6% of pixels;
the rest are antialiased text and line edges. Open: one project's audio ends
400 samples (8 ms) before its video when the composition duration is not a
whole frame; a local fork of a built-in could not draw saved instances until
this lane's fork rule (now drawn); one project needs an extension that is not
on this Mac.

**Playback (`pm-engine-service --bench --project`, 1080p output, 60 Hz
synthetic ticks, Auto).** 87 s project: 0 skipped targets and 0 stand-ins over
25 s, Auto dropped to half resolution; frame cost p50 7.7 ms, p95 25 ms (after
removing a per-frame whole-document walk: p50 was 9 to 11 ms). The section 5
headroom (p95 ≤ 8.3 ms) is **not met**. Random seeks to a full-quality frame:
p50 3 to 5 ms, p95 27 to 35 ms, max 49 ms.

**On screen, 2026-11-08 (release app, `PM_ENGINE_TRACE`, private copies).**
Review found playback presenting no new frame after the first and a paused
viewer at 260% CPU. Cause: the layer's drawable no longer matched the
configured output, and the engine dropped the drawable and asked for the same
frame again at once; the flood also overflowed the effect watchdog, whose
dropped frame left a mapped readback that wgpu then refused. Both fixed
(drawn at the drawable's size; a dropped frame unmaps its readback). Paused
CPU is now 1.4 to 1.7%. Playback presents every frame it draws:

| Project | Rate | Frames | Late (completed after target) | Hitches (gap ≥ 3 vsyncs) | Frame cost p50 / p95 / max |
|---|---|---|---|---|---|
| 87 s, 30 fps | 60 Hz display | 2,836 over ~95 s | 223 | ~35 | 4.3 / 20.1 / 160 ms |
| 74 s, 24 fps, 4K video | 60 Hz display | 1,322 | 330 | 15 target gaps | 3.0 / 47.4 / 132 ms |

Not 0 dropped frames. The hitches repeat each loop at the same media times
(4.0 to 5.8 s on the 30 fps project, 9 to 11 s and 18 to 22 s on the 24 fps
one). Sampled there: text coverage geometry rebuilt for every animated
transform (`CoveragePathCache::prepare`, keyed by exact linear transform, about
40% of the engine thread), one command buffer submission per vector raster
miss (`VectorCache::render_scene`, about 45% at 4 to 6 s), and plan building.
Planning media ahead of the playhead no longer delays the frame it follows.
A frame missing a layer (an effect no installed extension provides) now says
so in the viewer status line.

The review's compiler-busy export case does not occur in the app: every export
(app and MCP) runs in its own `--engine-job` process with its own compiler
pool, and one renderer compiles at a time there.

### Wave 16 perf lane, 2026-11-08

M4 Max shared with five other lanes (load 25 to 118 throughout); private
copies of the three real projects; `pm-engine-service --bench --project`
(release, 1080p output, 60 Hz synthetic ticks, Auto), 30 s from the start.
CPU = render thread from frame start to submit; GPU waits include other
lanes' GPU work.

| Project | Before (head) | After | Notes |
|---|---|---|---|
| 30 fps, 867 layers | p50 12.3, p95 44.1 ms; 695/720 frames | p50 9.9, p95 31.2 ms; 720/720; CPU p95 34 → 18 ms; worst in-play frame 190 → 50 ms | heaviest window (9–11 s, keyframed shape paths) CPU 50 → 17–25 ms |
| 24 fps, 4K video | p95 14.0 ms, max 182 ms | p50 1.2, p95 13.2 ms, max 31 ms in play | the 182 ms was a text pipeline compiled mid-play |
| WithMedia | p50 1.6, p95 2.4 ms | unchanged | |

Removed per frame (sampled): text glyph geometry flattening and nonzero
resolution under animated transforms (22% of the frame), per-glyph GPU
uploads, text reshaping, shape path evaluation and a deep copy of every
vertex, still decodes and still probes on the frame, a first-play pipeline
compile, and the 190 ms stall of a video's first frame: `prepare_media`
refused the whole look-ahead when any layer was unsupported (a missing
extension effect). Section 5's headroom (p95 ≤ 50% of the 16.7 ms display
interval) is **not met** on the 867-layer project under this load; the
remaining cost there is keyframed shape paths (per-vertex channel
evaluation and outline transforms) and GPU queueing.

**Scrub** (bench sweep at 9 s): repeats of recently drawn frames (the
service's frame cache) 0.31 ms p95, within the 9.333 ms endpoint;
uncached frame-by-frame drags 25–37 ms on the 867-layer project's heaviest
window (animated paths), 2–3 ms p50 and 17 ms p95 on the 4K video project.
Random seeks to a completed frame: p50 0.8–1.9 ms, p95 31–35 ms.

**Ratchet.** `text_cold` passes again after this lane's changes (heap
2,332,538 of 2,339,455; 6 textures and 7 uploads instead of 9 and 10).
`drag_500` (94.6 MB heap, +0.8%) and `blend_stack_cold` (1.62 MB, +4.1%)
fail as they did on the merged head before the lane (94,565,111 and
1,609,021 there); not attributed, baselines unchanged.

**`corpus_render_parity`, `render/fx-blur` (effects capture).** Classified
real, not harness: every failing pixel is *unreferenced* resampled image
content. Interior pixels match the f64 reference (excess ≤ 0.07 CV). The
excess over today's capture sits on a rotated, minified high-frequency
test chart and on the edges of a rotated square (8-bit frame: max 34,
p99.9 14 CV over 366 pixels; transparent float frames: max 2.49 over a
bound of 1). Frames are visually identical. Between the passing
`40f9ee7c9` and failing `80bc541dd` the still minification path changed
(`3351e2185` per-still mip chains, `625c444f9` one-to-one texel copies,
`13e77ff11` pyramid tails in one pass), which is where to look. Owner: L4
sampling, to model the declared minification kernel for rotated stills so
these pixels get a reference. No bound changed; the pending rows stay
failing for owner review.

**Not run here:** `text_corpus`, the scene3d and glTF float references
(they need the Chromium reference generator) and AUDIO-2 (a timing bound
that needs a quiet Mac). Their state is as in the entry above.

### Wave 18 perf lane, 2026-11-08

Release builds on the owner's M4 Max, shared with other lanes (load 6 to 42),
private copies of the real projects with the co-founder's 39 extensions,
launched in the background.

**Export.** Every app export submitted one 16-row band per grant, so a 1080p
frame cost 68 composition renders wherever a pass renders whole for any
region (precomps, multi-pass effects). Agent export of the 867-layer project's
work range (1,765 frames, H.264): 0.37 fps before, 152 s (11.6 fps) after
(whole frames once the viewer has been quiet 1 s). In whole frames
(`pm-cli`, ProRes 422 HQ) a frame costs about 66 ms: 26 ms of CPU, then a GPU
wait of 35 to 180 ms, about 97% of it one user effect (an 8-pass glow on four
1080p layers at the project's declared 32-bit precision, authored cost). CPU
and GPU of consecutive frames do not overlap yet. Electron was not measured.

**Playback on screen** (60 Hz display, Auto, Fit):

| Project | Before | After |
|---|---|---|
| 867 layers, 24 fps | 680 content frames skipped in one pass: 23 s and 4 s freezes waiting on raster override answers, then the realm's 8 s "not responding" prompt | override playback left to the compat lane (this lane's change was withdrawn); with it: 1,779 of 1,779 shown, 11 skipped; cost p50 15, p95 65 ms (9 to 22 s: CPU 17 to 42 ms, GPU wait about 22 ms) |
| 24 fps, 4K video | 39 skipped in the first 3.2 s (refusals backed off to 1 s) | 21 skipped, all in one 0.9 s hold at 3.07 s where an extension override fails (frames fail, as on main); cost p50 9.3, p95 17 ms |

Section 5 (0 dropped, p95 at most 8.3 ms) is **not met**. Remaining: a
first-use composite pipeline compile (70 ms) in the first play, keyframed
shape paths and per-frame texture creation on the CPU, GPU queueing.

**Render cache.** View › Render Selection of a 4.1 s layer in the heaviest
stretch rendered 99 frames in under 10 s in an `--engine-job` child; played
back through the read-ahead thread they cost p50 2.8, p95 3.3 ms against 50 to
75 ms rendered.

**Not run here:** AUDIO-2 and the `drag_500` / `blend_stack_cold` heap
attribution need a quiet Mac; they stand as in the entries above.

### Wave 19 perf lane, 2026-11-09

**Export "VideoToolbox dropped a frame".** Not the band switch: a viewer
playing through an export defers every frame, and the hardware H.264
encoder drops the B-frames it holds once the next frame is late by 30 to
75 s (system log: `AVE ERR ... encoding error -1004`, `DROPPING frame N
type 4`). `an_export_held_for_a_long_playback_never_drops_a_frame` (80 s
hold, `--ignored`) fails with the app's message without the fix and passes
with it.

**Playback, 867-layer project, 6 to 23 s** (`pm-engine-service --bench`,
release, `--quality=half`, render thread CPU time): p50 14.9, p95 28.5 ms
(19 to 21 s the heaviest); before the wave p50 15.3, p95 28.8 ms. The
machine's GPU was 40 to 80% busy with other processes, so frame time from
worker to completion (p50 35 to 50 ms) is mostly queueing behind them; the
frame's own GPU time is p50 1.5, p95 3 ms. Section 5's headroom is not met.
CPU left per heavy frame: text rasterised in device space under moving
parents (about 24 misses a frame, 28%), planning 786 layers (26%), wgpu
encode and submit (20%), and the viewer waiting on video decode (10% of the
wall time). Changes: animated shapes no longer submit a command buffer
each (48 a frame), effect uniforms are pooled, and effect programs build on
the prewarm worker (42 of the project's programs build there; the bench,
which plays right after opening, still met 2 compiles mid-play instead of 4,
the worker not having reached them yet).

**Export overlap** (`pm-cli render`, ProRes 422 HQ, 6 to 23 s, best of
three under load): 32.9 s instead of 37.5 s; still bound by the project's
8-pass glow on the GPU.

**Render cache** (`--fill`, 19 to 22 s, 72 frames at 1080p): 3.9 MB a frame
instead of 16.6 MB, fill 25 s, read (file, checksum, decode) p50 14, p95
18 ms; playback from it p50 2.5, p95 3.0 ms.

### WIP owner handoff

Validation was stopped at the orchestrator's request before the full engine
integration replay finished. Jude takes over with the original frozen corpus
and projects. The exact failure register, verified subsets, environment and
next steps are in [the Mac engine handoff](../native/handoff/engine-macos.md).
The branch is not all green; Gate E and the cold distinct-voice AUDIO-2 gate
remain open. The last TIFF palette integration repair also needs a full replay.
