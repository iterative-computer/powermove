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

| Cause | Tests | Notes |
|---|---|---|
| Spec reads the sealed `window.powermove` bridge | 21 | Since `bfcd2c88` (2026-09-23) the renderer captures the preload bridge at boot and replaces `window.powermove` with `undefined`. These specs call `powermove.store`, `.media`, `.extensions`, `.codex`, or `.compatible` directly. They need a test seam that does not reopen the bridge to extensions, or a rewrite through `PM` APIs |
| Relaunch does not restore the last active project | 5 | A probe on the same build: before relaunch `PM.proj.id` was `Pol5olf7` with one imported layer; after relaunch the tab strip still listed it, but the app booted to Projects with a fresh `Untitled` (`Py3jztv7`). Every spec that checks persistence across `session.relaunch()` without opening a file fails this way |
| Spec predates a UI change | 6 | Stale selectors or expectations: text animator presets, Settings tabs, color picker anatomy, toast details button |
| UI behavior differs from the spec | 18 | Some are regressions, some are intended changes the spec missed; each needs a decision before its test is rewritten or the product fixed |
| Agent requests carry extra images | 2 | Requests include viewer-capture JPEGs the specs do not expect |
| Rendering, evaluation, text, and persistence | 5 | Directly relevant to engine parity: precompose pixels, text anchor, history on disk, save responsiveness, Studio Cube property controls |
| Flaky | 1 | `project-file-media.spec.ts:9` |

By owner: Viewer and settings UI 29, Proxies and storage 7, Export and agents 6,
Engine and platform 6, Renderer write path 5, Text and vectors 2, Model and
evaluation 1, Media 1, 3D 1. Lanes without engine scope for a test (for example a
toast layout) get it through the closest UI owner; Parity and QA tracks all of them.

## Failing and flaky tests

Each test below failed on `feat/native-engine` as-is: on the first attempt, on the
retry, and in the comparison run at `b1d3e7f7`. The flaky test is described in its
row.

| Spec | Test | Status | Owner | Error and reason |
|---|---|---|---|---|
| `ae-audit-implementation.spec.ts:9` | editable Pen, canvas text, graph and preview controls render coherently | fail | Text and vectors | Timeout waiting for the "Text animator presets" group and its Rise button; that group no longer exists in src (removed in a5e3fbb6, 2026-10-05). Stale spec; the text animator UI it drives was redesigned. |
| `ae-audit-implementation.spec.ts:42` | precompose preserves cross-boundary animated rig pixels at several frames | fail | Model and evaluation | PM.precompose of a parented, rotating rig changes rendered pixels: the mean absolute difference at t = 0.4, 1, 1.8 is not below 1 CV for every sample (expected all true, got false). Evaluation parity across a precomposition boundary. |
| `agent-attachment-preview.spec.ts:3` | agent image attachments open a fitted preview and Escape returns focus | fail | Viewer and settings UI | Escape closes the preview but focus does not return to the "View reference.png" trigger. Pointer-clicked buttons are blurred since 352f1d1e (2026-10-09, "Fix pointer-clicked buttons retaining keyboard focus"), so the trigger the spec clicks has no focus to restore. Product and spec disagree; decide which behavior is intended. |
| `agent-media-tools.spec.ts:41` | the agent watches and listens to imported footage through the media tools | fail | Export and agents | The final model request carries 8 data:image attachments; the spec expects 4 (2 frames + 1 sheet + 1 composition sheet). Suspected extra visual-context frames from e7e3ccb1 ("Improve agent visual context"). Agent snapshots move to the engine, so the count matters for the port. |
| `agent-mod-capabilities.spec.ts:5` | a real mod supplies import defaults and Properties controls with automatic cleanup | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions.setEnabled), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `captions-ui.spec.ts:42` | captions surfaces in light mode | fail | Viewer and settings UI | Escape does not close the Export dialog, so Export Captions opens on top of it and locator('.export-modal') matches two dialogs (strict mode). Suspected cause: pointer-clicked buttons are blurred since 352f1d1e (2026-10-09, "Fix pointer-clicked buttons retaining keyboard focus"), after which the dialog no longer holds focus for Escape. |
| `captions-ui.spec.ts:42` | captions surfaces in dark mode | fail | Viewer and settings UI | Same failure as the light-mode case: two .export-modal dialogs (Export and Export Captions) are open at once. |
| `chatgpt-connection.spec.ts:90` | large images and files cross the real renderer-to-main agent boundary | fail | Export and agents | Stale spec: reads the raw window.powermove bridge (powermove.codex), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `chatgpt-connection.spec.ts:124` | Claude subscription status and structured runs cross the real hidden app boundary | fail | Export and agents | Stale spec: reads the raw window.powermove bridge (powermove.codex), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `color-picker-overlay.spec.ts:4` | color picker stays above adjacent panels and inside the viewport | fail | Viewer and settings UI | The first open passes; after Escape the inspector Fill row no longer contains a .color-field trigger and the loop times out (30 s). Suspected stale selector after the color picker unification (f8734eb3, 2026-09-25). |
| `compatible-authoring.spec.ts:6` | an API connection creates a keyframeable effect and verifies it through live tools | fail | Export and agents | Stale spec: reads the raw window.powermove bridge (powermove.compatible), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `compatible-provider.spec.ts:56` | provider models, reasoning, and flat error actions remain usable in a narrow panel | fail | Export and agents | Stale spec: reads the raw window.powermove bridge (powermove.compatible), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `design-language.spec.ts:3` | flat controls remain consistent across home, editor, settings, library, and export in both themes | fail | Viewer and settings UI | In the dark editor the selected timeline mode button ("tl-mode-button on", Layers) draws a 1 px box-shadow ring, which the flat-controls rule forbids. The mode switch arrived in 19c910e4 (2026-10-03). |
| `dropdown-toggle.spec.ts:6` | native dropdowns reopen and custom pickers toggle on repeated clicks | fail | Viewer and settings UI | The 19th menu button, "Caption actions" (captions panel, 416e3a45, 2026-10-04), sits outside the viewport in the default layout, so the click never lands and the test times out. |
| `editor-reliability.spec.ts:60` | cancelled Save never reports a saved file; successful Save writes a reopenable project | fail | Renderer write path | Stale spec: reads the raw window.powermove bridge (powermove.store.flush), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `editor-responsiveness.spec.ts:65` | saving a large undo history yields to input and preserves the file | fail | Renderer write path | Longest main-thread gap while saving a 36 MB undo history was 220.6 ms (limit 100 ms) on both attempts and on the baseline run. Measured on the reference M4 Max while other lanes were compiling at nice 15; not yet re-measured on an idle machine. |
| `effect-authoring.spec.ts:5` | the shipped effect sample validates, registers, animates and undoes without creating a panel | fail | Viewer and settings UI | docs/samples/gradient-tint copied into the profile's extensions folder is not registered as an effect after relaunch (Kernel.effects.get('gradient-tint') stays empty for 30 s). Cause not isolated (unverified guess: local extensions now need enabling or trust since Sandbox v2). The effect contract itself belongs to GPU and render. |
| `extension-sandbox-clipboard.spec.ts:9` | a sandboxed panel imports a 5 MB image, and copies only with the clipboard permission | fail | Viewer and settings UI | Clicking inside the sandboxed panel never arms the gesture (outcome.armed stays false for 15 s). Same family as extension-sandbox.spec.ts: pointer input does not reach or focus the sandboxed view iframe in the hidden harness. |
| `extension-sandbox.spec.ts:52` | a store extension’s Svelte panel renders in its own view iframe with host chrome and keys | fail | Viewer and settings UI | Clicking the empty panel area leaves document.activeElement with no class (expected ext-panel-frame), so the sandboxed view never takes focus. |
| `extensions.spec.ts:29` | replaces panel source in place without changing the document, window, or docking | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `extensions.spec.ts:54` | a user extension activates, contributes a command and an effect, and survives disable | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `extensions.spec.ts:103` | timeline replacements swap with the built-in timeline without corrupting the live panel | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `extensions.spec.ts:243` | turning the built-in timeline off and on restores its live panel in place | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `font-anchor.spec.ts:6` | font changes and animation preserve a centered text anchor | fail | Text and vectors | Centered text anchor drifts after font changes and animation: maximum position error 630.87 px (limit 0.001); samples 1 to 10 and the last two are exact; samples 11 to 24 are off by 70 to 631 px. Anchor bounds do persist. |
| `import-mp4.spec.ts:17` | keeps imported video playable after the source is removed and the app closes | fail | Engine and platform | After session.relaunch() the app restores the tab strip but boots to Projects with a fresh Untitled placeholder instead of the last active project (probe: id before relaunch Pol5olf7, after Py3jztv7, ProjectsScreen open). Suspected window session restore (main EditorWindows / renderer loadBootProject). |
| `import-prores-alpha.spec.ts:9` | imports ProRes 4444 with transparency and restores playable media | fail | Media | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `import-prores.spec.ts:7` | converts ProRes once, presents video frames, and restores the durable proxy | fail | Engine and platform | After session.relaunch() the app restores the tab strip but boots to Projects with a fresh Untitled placeholder instead of the last active project (probe: id before relaunch Pol5olf7, after Py3jztv7, ProjectsScreen open). Suspected window session restore (main EditorWindows / renderer loadBootProject). |
| `large-store-startup.spec.ts:6` | opens with a large recovery tree and preserves every record | fail | Renderer write path | Stale spec: reads the raw window.powermove bridge (powermove.store), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores tone.wav from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores tone.mp3 from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores h264-aac.mp4 from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores converted.mkv from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores still.heic from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `local-media-recovery.spec.ts:7` | restores animated.gif from its local source when cached bytes are missing | fail | Proxies and storage | Stale spec: reads the raw window.powermove bridge (powermove.media), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `media-bulk-selection.spec.ts:20` | drag a selection box, delete media, and undo the whole batch in both themes | fail | Viewer and settings UI | Dragging across the media grid never shows .asset-selection-box (10 s). The selection box still exists in src; the drag start point or the library layout changed under the spec. |
| `memory-history-persistence.spec.ts:5` | compact history preserves pixels, every action, and pending redo through pressure and relaunch | fail | Renderer write path | store/projectHistory.<id>.json on disk has no format field (expected "powermove-history-dictionary", src/shared/history-memory.ts). The compact history dictionary is not what reaches disk after PM.flushProject(). |
| `panel-popout.spec.ts:3` | a panel opens in a native window, keeps edits, and returns to the layout | fail | Viewer and settings UI | getByRole(textbox, "Project notes") matches two textareas: the docked Notes panel and the Library's "Edit Notes" card preview. Strict-mode violation; spec or Library card markup needs to disambiguate. |
| `panel-workflow.spec.ts:19` | library shows a panel grid, adds panels to the workspace, and edits panels with the agent | fail | Viewer and settings UI | The Library shows 14 panel cards; PM.PANELS (minus toolbar) has 15. One registered panel is missing from the Library grid. |
| `performance-warning.spec.ts:3` | identifies a slow extension and lets the user turn it off | fail | Viewer and settings UI | Stale spec: reads the raw window.powermove bridge (powermove.extensions), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `persistence.spec.ts:26` | autosaves a project and restores it after relaunch | fail | Renderer write path | Stale spec: reads the raw window.powermove bridge (powermove.store.flush), which the renderer seals at boot since bfcd2c88 (2026-09-23, sandbox X1b); window.powermove is undefined in the page. |
| `playback-audio.spec.ts:100` | the restored project keeps playing audio through quality changes | fail | Engine and platform | After session.relaunch() the app restores the tab strip but boots to Projects with a fresh Untitled placeholder instead of the last active project (probe: id before relaunch Pol5olf7, after Py3jztv7, ProjectsScreen open). Suspected window session restore (main EditorWindows / renderer loadBootProject). |
| `precomp.spec.ts:46` | canvas and timeline open the same compact layer menu | fail | Viewer and settings UI | The timeline layer menu has "Import Captions…" and the canvas layer menu does not (captions, 416e3a45). The two menus are meant to match. |
| `project-file-media.spec.ts:158` | local recovery keeps imported audio after closing and relaunching Powermove | fail | Engine and platform | After session.relaunch() the app restores the tab strip but boots to Projects with a fresh Untitled placeholder instead of the last active project (probe: id before relaunch Pol5olf7, after Py3jztv7, ProjectsScreen open). Suspected window session restore (main EditorWindows / renderer loadBootProject). |
| `project-file-media.spec.ts:9` | saved project files restore imported video without the original session media | flaky | Proxies and storage | After reopening a saved .pmv, the metadata-only re-save returns false. Passed once in the full suite; failed 9 of 9 later runs (baseline file run, 5 isolated repeats, 3 whole-file repeats). |
| `project-settings.spec.ts:3` | Settings › Project edits the live composition and its export settings | fail | Viewer and settings UI | Settings tabs are project, general, accounts, transcription, extensions; the spec predates the Transcription tab (d63f9e55, 2026-10-04). Stale expectation. |
| `project-settings.spec.ts:57` | New project asks for the settings that matter before creating | fail | Viewer and settings UI | The Background color dialog has no .fill-sv plane; the color picker anatomy changed in f8734eb3 ("unify color pickers", 2026-09-25). Stale selector. |
| `prompt-attachments.spec.ts:5` | floating prompt attaches images and binary files, removes them, and sends their contents | fail | Export and agents | CodexBridge.request receives 4 images (the attached PNG plus three 480x270 JPEG viewer captures); the spec expects only the attachment. Suspected visual-context frames from e7e3ccb1 bypassing the stubbed AgentHarness.observe. |
| `property-animation.spec.ts:68` | text, shader, extension, blend, and mask stopwatches animate their real values | fail | 3D | The Studio Cube extension layer's "x.objectColor" stopwatch resolves but is never visible (collapsed or hidden section), so the click times out. |
| `security.spec.ts:4` | blocks traversal, popups, and Node/Electron escape hatches | fail | Engine and platform | Expects the bridge to be present on window (present: true); since bfcd2c88 the renderer seals window.powermove at boot, so present is false. Stale expectation; the security property it checks still holds. |
| `shell.spec.ts:260` | keeps a usable window drag area with crowded tabs and overlays | fail | Viewer and settings UI | The Library screen top is 42.32 px; the spec requires ≥ 44 px so it clears the titlebar drag area. |
| `timeline-layer-rows.spec.ts:3` | compact layer rows keep names centered and parenting in dedicated controls | fail | Viewer and settings UI | Clicking the row disclosure for the nested composition never reveals its child row "Prompt" (30 s timeout). |
| `timeline.spec.ts:55` | Scale keys move outside the layer, delete together, and never delete their layer | fail | Engine and platform | After session.relaunch() the app restores the tab strip but boots to Projects with a fresh Untitled placeholder instead of the last active project (probe: id before relaunch Pol5olf7, after Py3jztv7, ProjectsScreen open). Suspected window session restore (main EditorWindows / renderer loadBootProject). Here PM.L(id) is null after relaunch. |
| `timeline.spec.ts:100` | the easing grid applies a curve to both Scale dimensions and preserves linear defaults | fail | Viewer and settings UI | Choosing "Ease out" in the keyframe easing grid leaves both Scale keys at ei [1, 1] (expected [0.58, 1]); the menu item click does not apply the curve. |
| `timeline.spec.ts:459` | project rename is available from the tab menu and persists the live document | fail | Viewer and settings UI | Pressing Enter in "Rename project" does not commit: the tab keeps "E2E composition". |
| `toast-error.spec.ts:61` | the error toast matches the status toast it stacks with | fail | Viewer and settings UI | The error toast has no "Technical details" button; it was removed in dbd1c23a ("Redesign agent errors", 2026-09-22). Stale spec. |
| `user-feedback.spec.ts:5` | background picker keeps every channel above the hex field | fail | Viewer and settings UI | The Background color dialog has no .color-dialog-value element (removed in f8734eb3, "unify color pickers"). Stale selector. |
| `viewer-resize.spec.ts:4` | resizes the selected layer in local space with a fixed opposite edge and correct cursors | fail | Viewer and settings UI | Dragging the east handle leaves scale.x at 100. The handle hit-test works (the resize cursor matches) but the drag does not resize. |
| `viewer-resize.spec.ts:199` | resizes and rotates a multi-selection around the common pivot with one-step undo | fail | Viewer and settings UI | The multi-selection resize drag leaves the shape scale at 100, the same failure as the single-layer case. |

## Skipped (environment-dependent)

These tests skip themselves when the machine lacks a private fixture, a model, a
font, a live account, or Blender. They are not quarantined: they never fail when
the requirement is missing, and they run when it is present. Private projects stay
outside the repository; per decision 13, Jude's "Powermove ad" project is the
candidate for `PM_PERF_PROJECT` on the reference machine.

| Spec | Tests | Requirement | Owner |
|---|---|---|---|
| `agent-panel.spec.ts:188` | 1 | `POWERMOVE_E2E_LIVE=1` and a signed-in Codex account (live run) | Export and agents |
| `blender-motion.spec.ts:5, 265, 452` | 3 | Blender at a discovered path or `POWERMOVE_BLENDER_PATH`; all 3 pass with Blender 4.5.14 LTS, including in the full ledger run | 3D |
| `composited-video-sync.spec.ts:73` | 1 | `PM_SYNC_PROJECT`, a private layered cutout project | Media |
| `composition-playback-profile.spec.ts:6` | 1 | `PM_PERF_PROJECT`, a copied project | Parity and QA |
| `editor-responsiveness.spec.ts:6` | 1 | `PM_PERF_PROJECT` or `PM_PERF_FILE` | Parity and QA |
| `high-zoom-performance.spec.ts:7` | 1 | `PM_PERF_PROJECT` | Parity and QA |
| `media-intelligence.spec.ts:161, 259` | 2 | `POWERMOVE_E2E_PARAKEET_DIR`, a downloaded Parakeet model | Audio |
| `playback-performance-benchmark.spec.ts:7` | 1 | `PM_PERF_PROJECT` | Parity and QA |
| `project-export-fixture.spec.ts:11` | 1 | `POWERMOVE_EXPORT_FIXTURE`, a project file | Export and agents |
| `project-performance-benchmark.spec.ts:5` | 1 | `PM_PERF_PROJECT` | Parity and QA |
| `project-playback-profile.spec.ts:6` | 1 | `PM_PLAYBACK_PROJECT` | Parity and QA |
| `region-pan-performance.spec.ts:9` | 3 | `PM_PERF_PROJECT` | Parity and QA |
| `transcription.spec.ts:281` | 1 | `POWERMOVE_E2E_PARAKEET_DIR` | Audio |
| `variable-font-sliders.spec.ts:4, 52, 88` | 3 | Geist Width variable font installed | Text and vectors |
| `variable-font-sliders.spec.ts:129` | 1 | SF Pro installed | Text and vectors |
| `video-playback-regression.spec.ts:26` | 1 | `PM_VIDEO_FIXTURE`, a private 4K source | Media |
| `zoom-performance-benchmark.spec.ts:5` | 1 | `PM_PERF_PROJECT` | Parity and QA |

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
