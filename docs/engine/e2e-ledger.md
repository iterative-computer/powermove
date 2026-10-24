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

## Phase 3c consumers serial integration run

The historical quarantine tables above describe Phase 0. The consumer gate
uses `playwright.consumers.config.ts` and runs every spec, including historical
quarantine entries, with one worker. It adds no exclusions or retries and leaves
individual assertion and test budgets unchanged. The app and native modules are
built before taking the shared Electron lock; the four-hour global budget allows
the complete serial suite to finish. Existing optional-fixture skips remain
reported separately. The gate sets `POWERMOVE_NEW_ENGINE=1` or `0` as the default;
specs that explicitly exercise a particular engine retain that declared mode.

### Current ownership decisions (2026-10-23)

The orchestrator has re-scoped C4/C19 (Assets posters, filmstrips and inline
media), C5 (the source monitor), and C28 (job-engine web renditions) out of
consumers to the new `mediaframes` lane. That lane owns the missing engine
contracts and their qualification. The existing browser compatibility paths
remain; recording their coverage does not claim the new contracts exist.

C18 RAM preview belongs to Phase 4a and is done on `feat/ne-phase4a`. This
consumer branch does not claim that work is merged. C18 failures in the run
tables below are historical results from the tested builds, with ownership
transferred to Phase 4a; their counts and observed symptoms are preserved.

`X.addToQueue` continues to accept a queue addition while a foreground export
is busy. The preparation UI's queue-only busy rejection is reverted; queue
preparation uses operation-local cancellation and does not alter the export's
busy/cancel state. No prior queue bug was proven by that guard. The
`projectThumb` retry at the end of the five-second capture window is retained
as an accepted beta bug fix.

To reproduce without rebuilding while holding the Electron slot:

```sh
source engine/env.sh
nice -n 15 bun run --cwd apps/desktop build:engine
nice -n 15 bun run --cwd apps/desktop build:wasm
nice -n 15 bun run --cwd apps/desktop build
cd apps/desktop
consumer_reports=$(mktemp -d)
POWERMOVE_NEW_ENGINE=1 POWERMOVE_E2E_OUTPUT_DIR="$consumer_reports/on" \
  PLAYWRIGHT_JSON_OUTPUT_NAME="$consumer_reports/on.json" \
  ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  nice -n 15 bunx playwright test --config playwright.consumers.config.ts --workers=1
POWERMOVE_NEW_ENGINE=0 POWERMOVE_E2E_OUTPUT_DIR="$consumer_reports/off" \
  PLAYWRIGHT_JSON_OUTPUT_NAME="$consumer_reports/off.json" \
  ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  nice -n 15 bunx playwright test --config playwright.consumers.config.ts --workers=1
```

### Current serial runs (2026-10-24, after the main merge)

App and release native modules from `dc1418d4d` (the merged
`feat/native-engine` 84bcb32f8 plus this lane); one worker, no quarantine or
retries. Later commits revert the WebGL resolve emulation and change only
the C3 fixture; their C3 results are in [consumers.md](consumers.md).
Other lanes kept the machine loaded: ON ran at a load average of 11 to 17,
OFF at 40 to 90 (2.7 hours), so timeout and forced-shutdown results in OFF
are load-sensitive.

| Setting | Cases | Passed | Failed | Skipped | Time |
|---|---:|---:|---:|---:|---:|
| ON | 1,041 | 772 | 197 | 72 | 3,851.8 s |
| OFF | 1,041 | 832 | 138 | 71 | 9,561.6 s (1 global error) |

The merged suite has 244 more cases than round 6. 78 failures are ON-only,
119 fail in both settings and 19 are OFF-only. "Round 6" owners are the
owners recorded for the same spec; "new since merge" rows are specs main
added or changed, not yet rerun on the integration build, so their owner is
unverified. All 32 C3 cases passed in the ON run.

ON-only failures:

| Spec and line | Cases | Owner | Observed |
|---|---:|---|---|
| `ae-audit-implementation.spec.ts:51` | 1 | phase4a (C18) (round 6) | TimeoutError: page.waitForFunction: Timeout 10000ms exceeded. |
| `agent-context-review.spec.ts:114` | 1 | new since merge; unverified | Error: expect(received).toContain(expected) // indexOf  Expected substring: "Preview capture failed" Received string:    "" |
| `agent-playhead.spec.ts:6` | 1 | new since merge; unverified | Error: expect(received).toBeGreaterThan(expected)  Expected: > 0 Received:   0 |
| `agent-visual-replies.spec.ts:63` | 1 | new since merge; unverified | Error: expect(locator).toContainText(expected) failed  Locator: locator('iframe[title="Launch storyboard"]').contentFrame().locator('output') Expected substring: "Reveal" |
| `automatic-preview-memory.spec.ts:153` | 1 | phase4a (C18) (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 2 Received:   0  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `automatic-preview-preparation.spec.ts:72` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false |
| `composited-video-sync.spec.ts:9` | 1 | mediaclose (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 20 Received:   0 |
| `continuous-rasterization.spec.ts:4` | 1 | extcompat / renderer (round 6) | Error: expect(received).toBeGreaterThanOrEqual(expected)  Matcher error: received value must be a number or bigint  Received has value: undefined |
| `document-host-contract.spec.ts:23` | 1 | new since merge; unverified | Error: 34: __mcp_context in the worker  expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1  @@ -1,9 +1,9 @@   Object {     "content": Ar |
| `effect-zoom.spec.ts:3` | 1 | new since merge; unverified | Error: expect(received).not.toBeNull()  Received: null |
| `engine-compat-moat.spec.ts:278` | 1 | new since merge; unverified | Error: expect(received).toBeLessThanOrEqual(expected)  Expected: <= 4 Received:    223 |
| `engine-customization-corpus.spec.ts:411` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false |
| `engine-customization-corpus.spec.ts:445` | 1 | new since merge; unverified | Error: fork: today's reference must work: {"importColorSpace":null,"context":true,"connected":true,"presenting":true,"shown":false,"white":23152,"sourceWhite":23153,"erro |
| `engine-extension-compat.spec.ts:736` | 1 | new since merge; unverified | Error: expect(received).toBeLessThanOrEqual(expected)  Expected: <= 84 Received:    335 |
| `engine-svg.spec.ts:452` | 1 | derouter / vectors (round 6) | Test timeout of 30000ms exceeded. |
| `export-destination.spec.ts:38` | 1 | derouter / export (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Object { -   "cancelled": false, +   "error": "A file with that name alread |
| `export-dialog.spec.ts:150` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: undefined |
| `extension-layers.spec.ts:4` | 1 | threed (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 1 + Received  + 1  @@ -1,9 +1,9 @@   Object {     "changedPixels": true,     "colorAfterUndo": "#C7C4FF", - |
| `extension-layers.spec.ts:60` | 1 | threed (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 1 + Received  + 1  @@ -10,11 +10,11 @@     "colorAfterEditUndo": "#C7C4FF",     "compileError": null,     " |
| `extension-sandbox-perf.spec.ts:91` | 1 | extcompat (round 6) | Error: Electron shutdown exceeded 45000 ms before relaunch. |
| `frame-preparation.spec.ts:8` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: false Received: true |
| `gpu-recovery.spec.ts:3` | 1 | bridge (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 0 Received:   0  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `group-timeline-interactions.spec.ts:99` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 8 + Received  + 8    Object {     "history": Array [ -     "Trim clip", -     "Trim clip", +     |
| `import-mp4.spec.ts:117` | 1 | mediaclose (round 6) | Test timeout of 30000ms exceeded. |
| `import-mp4.spec.ts:17` | 1 | mediaclose (round 6) | Error: page.evaluate: Error: Restored video did not seek     at eval (eval at evaluate (:311:30), <anonymous>:19:47)     at wrapped (app://powermove/host/bootstrap-BllfMM |
| `import-mp4.spec.ts:170` | 1 | mediaclose (round 6) | Test timeout of 30000ms exceeded. |
| `import-prores-alpha.spec.ts:10` | 1 | mediaclose (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false |
| `import-prores.spec.ts:8` | 1 | mediaclose (round 6) | Error: page.evaluate: Error: {"liveProxy":false,"durableProxy":false,"frames":0}     at eval (eval at evaluate (:311:30), <anonymous>:31:15)     at async <anonymous>:337: |
| `legacy-blurred-video-export.spec.ts:27` | 1 | derouter / mediaclose (round 6) | Error: {"source":"prores","mblur":false,"levels":[20,30,40,50,61,70,80,90,100,109,120,130,140,150,160,170,180,190,200,200],"expected":[20,30,40,50,60,70,80,90,100,110,120 |
| `local-media-recovery.spec.ts:40` | 1 | mediaclose (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 2 + Received  + 2    Object { -   "error": StringContaining "source file has changed", -   "ready": false,  |
| `mixer.spec.ts:484` | 1 | consumers (1.0.8 light timeout; isolated rerun pending) | TimeoutError: page.waitForFunction: Timeout 20000ms exceeded. |
| `native-edit-kernels.spec.ts:32` | 1 | dochost (round 6) | Error: native evaluator mode of the build  expect(received).toBe(expected) // Object.is equality  Expected: "off" Received: "on" |
| `native-engine-tracer.spec.ts:185` | 1 | new since merge; unverified | TypeError: Cannot read properties of undefined (reading 'bounds') |
| `native-export-performance.spec.ts:213` | 2 | derouter / export (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Object { -   "error": "The GPU could not capture the export frame", +   "ca |
| `native-export-performance.spec.ts:324` | 1 | derouter / export (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 3 + Received  + 3    Object {     "active": 0, -   "bytes": 98304, -   "maxActive": 1, -   "writes": 4, +   |
| `native-export-performance.spec.ts:402` | 2 | derouter / export (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `native-export-performance.spec.ts:474` | 1 | derouter / export (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `native-export-performance.spec.ts:84` | 1 | derouter / export (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 2 + Received  + 2    Object {     "busy": false, -   "reads": 1, +   "reads": 0,     "result": O |
| `native-export-performance.spec.ts:9` | 2 | derouter / export (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Object { -   "cancelled": false, +   "cancelled": true,   } |
| `pan-presentation.spec.ts:104` | 1 | bridge (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 5 + Received  + 5    Object { -   "edit": true, -   "exact": true, -   "externalRedraw": true, + |
| `pan-presentation.spec.ts:5` | 1 | bridge (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Object {     "dx": 160,     "dy": -24, -   "newViewport": true, +   "newVie |
| `pan-presentation.spec.ts:60` | 1 | bridge (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 2 + Received  + 2    Object { -   "deferred": true, +   "deferred": false,     "duringGesture":  |
| `playback-audio.spec.ts:16` | 1 | phase4a (C18) (round 6) | Test timeout of 30000ms exceeded. |
| `playback-audio.spec.ts:73` | 1 | phase4a (C18) (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false |
| `playback-source-warmup.spec.ts:127` | 1 | mediaclose (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: 80 Received: 0  Call Log: - Timeout 1800ms exceeded while waiting on the predicate |
| `playback-source-warmup.spec.ts:3` | 1 | mediaclose (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: 25 Received: 0  Call Log: - Timeout 3000ms exceeded while waiting on the predicate |
| `playback-source-warmup.spec.ts:72` | 4 | mediaclose (round 6) | Error: expect(received).toBeGreaterThanOrEqual(expected)  Expected: >= 200 Received:    0  Call Log: - Timeout 1800ms exceeded while waiting on the predicate |
| `preview-resolution.spec.ts:3` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 2 + Received  + 2    Array [ -   1920, -   1080, +   1464, +   824,     1,     false,   ]  Call  |
| `project-file-media.spec.ts:77` | 1 | mediaclose (round 6) | Test timeout of 30000ms exceeded. |
| `project-file-media.spec.ts:9` | 1 | mediaclose (round 6) | Error: page.evaluate: Error: Restored video did not seek     at eval (eval at evaluate (:311:30), <anonymous>:19:47)     at wrapped (app://powermove/host/bootstrap-BllfMM |
| `project-loading.spec.ts:71` | 1 | new since merge; unverified | Error: expect(received).toBeLessThan(expected)  Expected: < 250 Received:   410.69999999925494 |
| `settings-motion.spec.ts:3` | 1 | new since merge; unverified | Error: expect(received).toBeLessThan(expected)  Expected: < 220 Received:   258.30000000074506 |
| `status-polling.spec.ts:3` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: 0 Received: 1 |
| `timeline-track-live-drag.spec.ts:40` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Array [     0,     0, -   255, +   0,     255,   ]  Call Log: - Timeout 100 |
| `timeline-track-live-drag.spec.ts:64` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Array [ -   255, +   0,     0,     0,     255,   ]  Call Log: - Timeout 100 |
| `timeline-track-live-drag.spec.ts:83` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: "green" Received: "other"  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `timeline.spec.ts:488` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: 2 Received: 0 |
| `video-cut-recovery.spec.ts:7` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `video-frame-integrity.spec.ts:130` | 1 | mediaclose (round 6) | Error: {"time":1.4,"level":0,"seeks":0}  expect(received).toBeLessThan(expected)  Expected: < 4 Received:   118 |
| `video-frame-integrity.spec.ts:161` | 3 | mediaclose (round 6) | Error: {"level":160,"viewerLevel":0,"seeks":0,"before":0,"after":0,"time":0.8,"memoryBefore":{"entries":0,"bytes":0,"maxBytes":134217728},"memoryAfter":{"entries":0,"byte |
| `video-frame-integrity.spec.ts:215` | 1 | mediaclose (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 20 Received:   0 |
| `video-frame-integrity.spec.ts:251` | 1 | mediaclose (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 3 Received:   0 |
| `video-frame-integrity.spec.ts:43` | 1 | mediaclose (round 6) | TimeoutError: page.waitForFunction: Timeout 15000ms exceeded. |
| `video-playback-regression.spec.ts:128` | 2 | mediaclose (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 48 Received:   0 |
| `video-playback-regression.spec.ts:6` | 1 | mediaclose (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: false Received: true |
| `video-playback-regression.spec.ts:79` | 1 | mediaclose (round 6) | Error: frame at 0.5: 254,0,0,255  expect(received).toBeGreaterThan(expected)  Expected: > 220 Received:   0 |
| `video-preview-io.spec.ts:10` | 1 | new since merge; unverified | Error: expect(received).toBeGreaterThan(expected)  Expected: > 0 Received:   0 |
| `video-scrubbing.spec.ts:7` | 2 | mediaclose (round 6) | Error: expect(received).toBeGreaterThan(expected)  Expected: > 20 Received:   0 |

Failing in both settings:

| Spec and line | Cases | Owner | Observed |
|---|---:|---|---|
| `ae-audit-implementation.spec.ts:105` | 1 | phase4a (C18) (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 1 + Received  + 1    Object {     "name": "Alpha master",     "options": Object {       "alpha": true, -    |
| `agent-attachment-preview.spec.ts:62` | 1 | new since merge; unverified | Error: expect(locator).not.toBeFocused() failed  Locator:  getByRole('button', { name: 'View reference.png', exact: true }) Expected: not focused Received: focused Timeou |
| `agent-media-handoff.spec.ts:6` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 4  - Array [] + Array [ +   "frame.png", +   "screen.png", + ] |
| `agent-media-tools.spec.ts:42` | 1 | agents (round 6) | Error: expect(received).toBe(expected) // Object.is equality  Expected: "host" Received: "cache" |
| `alpha-async-export.spec.ts:262` | 6 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `alpha-async-export.spec.ts:296` | 2 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `alpha-async-export.spec.ts:323` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `alpha-async-export.spec.ts:371` | 2 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `alpha-async-export.spec.ts:400` | 2 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"error": "The GPU could not capture the export frame"} Received: undefined |
| `alpha-async-export.spec.ts:429` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `alpha-async-export.spec.ts:454` | 2 | new since merge; unverified | Error: page.evaluate: Error: Builtin blur output changed     at eval (eval at evaluate (:311:30), <anonymous>:11:50)     at UtilityScript.evaluate (<anonymous>:313:16)    |
| `alpha-export-performance.spec.ts:170` | 2 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 6 + Received  + 4    Object {     "busy": false, -   "captures": 1, -   "disposed": 1, -   "inte |
| `alpha-export-performance.spec.ts:40` | 10 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `automatic-preview-preparation.spec.ts:6` | 1 | new since merge; unverified | Test timeout of 30000ms exceeded. |
| `chatgpt-connection.spec.ts:6` | 1 | agents (round 6) | Error: expect(locator).toBeEnabled() failed  Locator: getByRole('dialog', { name: 'Settings', exact: true }).getByRole('button', { name: 'Disconnect', exact: true }) Expe |
| `editor-responsiveness.spec.ts:69` | 1 | dochost (round 6) | Error: expect(received).toBeLessThan(expected)  Expected: < 100 Received:   124.90000000037253 |
| `engine-blender-png-transport.spec.ts:15` | 2 | new since merge; unverified | Error: page.evaluate: TypeError: window.PM.Kernel.api(...).scene3d.blenderStatus is not a function     at eval (eval at evaluate (:311:30), <anonymous>:1:67)     at Utili |
| `engine-blender-preview-api.spec.ts:148` | 1 | new since merge; unverified | Error: page.evaluate: TypeError: window.PM.Kernel.api(...).scene3d.blenderStatus is not a function     at eval (eval at evaluate (:311:30), <anonymous>:1:63)     at Utili |
| `engine-blender-preview-api.spec.ts:7` | 1 | new since merge; unverified | Error: page.evaluate: TypeError: window.PM.Kernel.api(...).scene3d.blenderStatus is not a function     at eval (eval at evaluate (:311:30), <anonymous>:1:58)     at Utili |
| `engine-customization-corpus.spec.ts:307` | 1 | new since merge; unverified | Error: api-probe-v1  expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 3  - Array [] + Array [ +   "scene3d reads: "threw: s.getRenderin |
| `engine-definitions-export.spec.ts:101` | 1 | new since merge; unverified | Error: drawn by the job engine, not routed  expect(received).not.toMatch(expected)  Expected pattern: not /^Lavf/ Received string:      "Lavf63.1.102" |
| `engine-definitions-export.spec.ts:28` | 1 | new since merge; unverified | Error: drawn by the job engine, not routed  expect(received).not.toMatch(expected)  Expected pattern: not /^Lavf/ Received string:      "Lavf63.1.102" |
| `engine-delivery.spec.ts:201` | 1 | derouter / export (round 6) | Error: expect(locator).toContainText(expected) failed  Locator: getByRole('dialog', { name: 'Render Queue', exact: true }).locator('.render-queue-job').nth(1) Expected su |
| `engine-doc-boot.spec.ts:56` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 1    Object {     "enabled": true, -   "projectId": "P6cdi19o", +   "projectId": |
| `engine-export.spec.ts:18` | 1 | derouter / export (round 6) | Error: the native viewer leaves the legacy WebGL context unallocated  expect(received).toBe(expected) // Object.is equality  Expected: true Received: false |
| `engine-headless-playback.spec.ts:47` | 1 | new since merge; unverified | Error: expect(received).toMatchObject(expected)  - Expected  - 1 + Received  + 1    Object { -   "frames": 0, +   "frames": 1,     "headless": true,   } |
| `engine-scene3d-cancel-model.spec.ts:8` | 1 | new since merge; unverified | Error: page.evaluate: TypeError: window.PM.Kernel.api(...).scene3d.blenderStatus is not a function     at eval (eval at evaluate (:311:30), <anonymous>:1:59)     at Utili |
| `engine-scene3d-chrome.spec.ts:42` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: 16 Received: undefined  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `engine-svg.spec.ts:359` | 1 | derouter / vectors (round 6) | Error: Command failed: /Users/judekim/.local/share/fnm/node-versions/v24.18.0/installation/bin/node /Users/judekim/Developer/powermove/.claude/worktrees/ne-consumers/pack |
| `engine-viewport-compat.spec.ts:12` | 4 | new since merge; unverified | TypeError: Cannot read properties of undefined (reading 'split') |
| `export-encoding-parity.spec.ts:202` | 4 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: "ready/ready" Received: "ready/stopped"  Call Log: - Timeout 45000ms exceeded while waiting on the |
| `extensions.spec.ts:161` | 1 | new since merge; unverified | Test timeout of 30000ms exceeded. |
| `frame-preparation.spec.ts:8` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: false Received: true |
| `import-cloud.spec.ts:6` | 1 | new since merge; unverified | Error: expect(locator).toBeVisible() failed  Locator: getByRole('button', { name: 'Downloading from iCloud… · Cloud artwork.png', exact: true }) Expected: visible Timeout |
| `import-image-sequence.spec.ts:7` | 2 | new since merge; unverified | Error: expect(locator).toContainText(expected) failed  Locator: getByRole('status', { name: 'Importing image sequence', exact: true }) Expected substring: "Importing imag |
| `media-replacement.spec.ts:9` | 1 | derouter / mediaclose (legacy capture after reload) (round 6) | Error: expect(locator).toContainText(expected) failed  Locator: getByRole('status', { name: 'Importing image sequence', exact: true }) Expected substring: "Importing imag |
| `native-edit-kernels.spec.ts:200` | 1 | dochost (round 6) | Error: page.evaluate: TypeError: api.scene3d.createMaterial is not a function     at eval (eval at evaluate (:311:30), <anonymous>:88:32)     at UtilityScript.evaluate (< |
| `native-export-performance.spec.ts:324` | 1 | derouter / export (round 6) | Error: expect(received).toMatchObject(expected)  - Expected  - 3 + Received  + 3    Object {     "active": 0, -   "bytes": 98304, -   "maxActive": 1, -   "writes": 4, +   |
| `native-linked-rendition.spec.ts:11` | 1 | new since merge; unverified | TypeError: The "path" argument must be of type string. Received undefined |
| `native-still-orientation-parity.spec.ts:264` | 1 | derouter (round 6) | Error: {"file":"still-cicp-9-1-9.avif","expectDrawn":false,"route":"engine","reason":null,"legacySize":[96,64],"nativeSize":[96,64],"max":1,"over1":0,"pixels":16384,"alph |
| `png-export-performance.spec.ts:104` | 2 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `png-export-performance.spec.ts:126` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `png-export-performance.spec.ts:50` | 3 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `png-export-performance.spec.ts:7` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `precomp.spec.ts:46` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 1 + Received  + 0  @@ -13,8 +13,7 @@     "Lock",     "",     "Delete",     "",     "New Composit |
| `project-color.spec.ts:7` | 1 | new since merge; unverified | Error: expect(locator).toHaveValue(expected) failed  Locator: getByRole('dialog', { name: 'Settings', exact: true }).getByRole('region', { name: 'Project', exact: true }) |
| `projects-drop.spec.ts:45` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: "Dropped project" Received: "E2E composition"  Call Log: - Timeout 10000ms exceeded while waiting  |
| `projects-drop.spec.ts:6` | 2 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: "Dropped image" Received: "Untitled"  Call Log: - Timeout 10000ms exceeded while waiting on the pr |
| `scene3d-budget-parity.spec.ts:157` | 2 | new since merge; unverified | Error: electronApplication.evaluate: Error: UnknownVizError |
| `scene3d-warnings.spec.ts:5` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 0 + Received  + 2    Array [     Object {       "missing": false,       "warnings": Array [ +    |
| `settings-scroll.spec.ts:3` | 1 | new since merge; unverified | Error: expect(locator).toHaveCount(expected) failed  Locator:  getByRole('dialog', { name: 'Settings', exact: true }).locator('.sg-page:not([hidden])') Expected: 6 Receiv |
| `shader-color-uniform.spec.ts:6` | 1 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: 0 Received: 2 |
| `static-effects-export.spec.ts:164` | 4 | new since merge; unverified | Error: page.evaluate: Error: Builtin blur output changed     at eval (eval at evaluate (:311:30), <anonymous>:18:51)     at UtilityScript.evaluate (<anonymous>:313:16)    |
| `static-effects-export.spec.ts:291` | 3 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `static-effects-export.spec.ts:64` | 5 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `static-export-performance.spec.ts:37` | 2 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 4 + Received  + 2    Object { -   "captures": 3, -   "result": Object { -     "cancelled": false |
| `static-export-performance.spec.ts:6` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `static-image-export.spec.ts:142` | 2 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `static-image-export.spec.ts:45` | 5 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `static-masks-export.spec.ts:190` | 2 | new since merge; unverified | Error: expect(received).toBe(expected) // Object.is equality  Expected: true Received: false  Call Log: - Timeout 10000ms exceeded while waiting on the predicate |
| `static-masks-export.spec.ts:79` | 5 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `timeline-refresh-cadence.spec.ts:11` | 1 | new since merge; unverified | Error: expect(received).toBeGreaterThanOrEqual(expected)  Expected: >= 115 Received:    0 |
| `timeline-refresh-cadence.spec.ts:201` | 1 | new since merge; unverified | Error: expect(received).toBeGreaterThanOrEqual(expected)  Expected: >= 115 Received:    0 |
| `viewer-resize.spec.ts:295` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 0 + Received  + 4  @@ -8,10 +8,14 @@       "text": Array [         450,         180,       ],    |
| `viewer-resize.spec.ts:76` | 1 | new since merge; unverified | ReferenceError: testInfo is not defined |
| `webm-export-performance.spec.ts:46` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |
| `webm-export-performance.spec.ts:6` | 1 | new since merge; unverified | Error: expect(received).toEqual(expected) // deep equality  Expected: {"cancelled": false} Received: undefined |

OFF-only failures:

| Spec and line | Cases | Owner | Observed |
|---|---:|---|---|
| `engine-extension-compat.spec.ts:838` | 1 | new since merge; unverified | Error: page.evaluate: Target page, context or browser has been closed Browser logs:  <launching> /Users/judekim/Developer/powermove/.claude/worktrees/ne-consumers/node_mo |
| `engine-extension-compat.spec.ts:924` | 1 | new since merge; unverified | Test timeout of 240000ms exceeded. |
| `engine-shader-parity.spec.ts:84` | 1 | new since merge; unverified | Test timeout of 30000ms exceeded. |
| `engine-svg.spec.ts:289` | 1 | derouter / vectors (round 6) | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 2 + Received  + 2    Object {     "control": Object { -     "differ": 0, -     "max": 0, +     " |
| `export-video-frames.spec.ts:12` | 1 | new since merge; unverified | Test timeout of 30000ms exceeded. |
| `extension-sandbox.spec.ts:158` | 1 | extcompat (round 6) | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `figma-paste.spec.ts:14` | 1 | new since merge; unverified | Error: electronApplication.evaluateHandle: TypeError: Failed to construct 'ClipboardItem': at least one MIME type is required.     at new ClipboardItem (node:electron/js2 |
| `figma-paste.spec.ts:52` | 2 | new since merge; unverified | Error: electronApplication.evaluateHandle: TypeError: Failed to construct 'ClipboardItem': at least one MIME type is required.     at new ClipboardItem (node:electron/js2 |
| `graph-picking.spec.ts:65` | 1 | new since merge; unverified | TypeError: Cannot read properties of undefined (reading '_object') |
| `history-gesture-press.spec.ts:318` | 1 | new since merge; unverified | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `inspector-alignment.spec.ts:4` | 2 | new since merge; unverified | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `library-card-parity.spec.ts:163` | 1 | consumers: shelf-ledge raster history under load; fixed by `ea47354cd` | Error: expect(received).toEqual(expected) // deep equality  - Expected  - 2 + Received  + 2    Object { -   "differing": 0, -   "max": 0, +   "differing": 10973, +   "max |
| `native-text-selection-menu.spec.ts:15` | 1 | bridge lifecycle (test body passed) (round 6) | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `svelte5-showcase.spec.ts:216` | 1 | new since merge; unverified | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `svelte5-showcase.spec.ts:222` | 1 | new since merge; unverified | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `timeline-layer-rows.spec.ts:22` | 1 | new since merge; unverified | Error: Electron shutdown exceeded 45000 ms; terminated the test process for cleanup. |
| `variable-font-sliders.spec.ts:4` | 1 | new since merge; unverified | Test timeout of 60000ms exceeded. |
