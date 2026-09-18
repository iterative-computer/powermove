# Powermove release review — 2026-09-17

**Remediation in progress:** all nine original findings below have source fixes and regression coverage integrated. The final native verification is still in progress; the original findings are retained as the audit record. Do not treat this as final distribution-artifact approval.

Reviewed base commit: `d81690482b877f71eca09f1320110122021a681c`. Follow-up fixes were developed in three isolated worktrees (Astra Low for extension safety, Sol High for export and project/UI), independently reviewed, then integrated into the working tree. Existing untracked test-results directories were preserved. No commit or deployment was performed.

The hands-on review used the freshly built production bundles over `app://powermove`, a separate temporary profile, and native computer use. A copy of Electron with a distinct app identifier allowed computer use to distinguish the audit session from the existing development process. This was **not** a signed/notarized distribution-artifact test.

## Remediation follow-up

- **1:** Recovery preserves untouched originals, retains backups through rollback, and limits failed restores to entries actually touched. Native startup recovered a pre-move journal without changing the original extension SHA-256.
- **2:** Runtime failures persist disabled state, quarantine the module, and preserve failure health. Native two-failure test removed the panel and its stylesheet; disk state stayed disabled. A follow-up fixes the Mods switch refresh ordering discovered during that test.
- **3:** Initial compilation completion broadcasts discovery; delayed-compilation integration coverage verifies the renderer loads the result.
- **4:** All export delivery promises are awaited and cancellation/write errors propagate. Canceling the native PNG folder picker now returns “Export cancelled” without per-frame save dialogs. Successful-folder verification stalled before the first frame. PNG now uses the existing 2D frame capture to avoid live WebGL-canvas readback; this suspected cause still needs native rechecking. The desktop permission policy also now permits the trusted app's selected directory and its descendants, with tests denying other paths, origins, windows, and subframes. Native tracing found additional Electron permission-shape mismatches: missing requesting URL, and an origin-only check that can have no WebContents. The final correction scopes such checks to a directory selected by a live trusted window and clears grants on navigation/destruction. Restricted paths reject instead of hanging. The final build and 16 targeted export/permission tests pass, but successful native PNG export is not yet signed off because macOS locked again before the retry.
- **5:** Active project persistence is flushed before trashing, and project switching skips trashed outgoing records. Native trash and restore passed without resurrection.
- **6:** Media rows explicitly depend on runtime asset restoration. Native reopening showed the thumbnail and enabled Add to timeline immediately.
- **7:** Tab file status snapshots react to save/path changes. Native Save cleared the dirty marker and showed the correct PMV path.
- **8:** Successful native project open dismisses Home; manually verified.
- **9:** Imported CSS and compiled Svelte styles are owned by extension activation and removed on disposal, including failed activation and cached-module re-enable. Native failure disposal removed the visible test style.

The final integrated full gate passed **2,214 tests, one skipped (272 passing files, one skipped)**. Build, typecheck (zero Svelte errors/warnings), and extension boundaries (10 extensions) passed after the follow-up Mods/PNG changes. After unlock, the Sol-generated 75% button changed the selected layer to 75 and Command+Z restored 100. Restart preserved both buttons, the saved project, restored media, and disabled extension state. A second failure test confirmed the live Mods switch immediately changed to Off and reported one mod needing attention. PNG success remains unresolved; macOS locked again during the next diagnostic retry. The latest full run had 2,213 passing tests and one skipped, but one test worker timed out across the machine sleep/lock interval; rerunning the affected dock-ghost file plus permission tests passed all 10 tests.

## Original findings requiring release fixes

### 1. P1 — Interrupted extension publication can delete the previous working extension

Source: [change-history.ts:195](/Users/judekim/Developer/powermove/apps/desktop/src/main/codex/change-history.ts:195).

Publication writes the pending transaction record before moving live extension directories into `before`. Rollback/recovery removes every declared live directory unconditionally, then restores it only if its `before` backup exists. An interruption before that move therefore deletes an untouched original. A failure partway through a multi-extension transaction can also affect later, untouched entries.

**Evidence:** the code-review lane ran the real recovery function against a temporary pending record representing this exact pre-move state. The original `live/safe-change/index.ts` became `ENOENT`.

**Fix direction:** persist per-entry progress or use an idempotent recovery protocol that distinguishes untouched originals from installed replacements. Never infer that an absent backup means the live directory is newly created. Check both pre-move and partially applied multi-extension recovery.

### 2. P1 — A runtime-failing extension is immediately reactivated after auto-disable

Sources: [loader.ts:326](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/kernel/loader.ts:326), [registry.ts:355](/Users/judekim/Developer/powermove/apps/desktop/src/main/extensions/registry.ts:355).

The loader disables only its local record, then reports runtime-error health. Main retains `enabled: true` and broadcasts a health event. The renderer refetches that enabled record and reconciles it; runtime-error is not a blocked activation state. The failing extension runs again instead of remaining disabled.

**Evidence:** a bounded harness using the real loader/kernel and the production-style health echo produced `activationCount: 2`, `active: ["flaky"]`, and `mainEnabled: true` after the failure threshold. Ordinary manual disable/re-enable worked in the UI; the defect is specifically automatic failure containment.

**Fix direction:** persist the disabled state through the authoritative registry or block reactivation until an explicit retry. Test the actual health-event round trip rather than a bridge that never echoes health updates.

### 3. P1 — Slow startup compilation can leave user extensions unloaded

Sources: [main/index.ts:450](/Users/judekim/Developer/powermove/apps/desktop/src/main/index.ts:450), [loader.ts:372](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/kernel/loader.ts:372).

Initial extension refresh runs in the background. Its completion starts the watcher but emits no change event, despite the comment promising one. The renderer makes one initial list request. If that request precedes completion of user-extension compilation, it sees an empty/partial registry and receives no completion signal to load the remaining mods.

**Evidence:** a real-loader harness reproduced one main-process record with zero renderer records and only one list request after delayed discovery. This is a timing-dependent code/harness finding; the small Audit Tools mod did load on the manual restart.

**Fix direction:** coordinate initial readiness or broadcast the completed refresh to subscribed renderers. Add a delayed-compile startup regression.

### 4. P1 — Canceling PNG destination selection starts a broken fallback export

Source: [exporter.ts:640](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/legacy/core/exporter.ts:640), especially the unawaited download at line 654.

**Manual reproduction:** export a 3-second, 30 fps composition as PNG sequence; cancel the directory picker. Instead of canceling the operation, Powermove opens a save dialog for frame 00000. The loop continues issuing asynchronous native saves without waiting for that dialog. Native save permits one outstanding upload, so subsequent frames fail.

**Evidence:** the 90-frame run generated **89** `file:save-upload` handler failures with “A save is already in progress.” The top-level export path can report completion despite those unhandled failures. The same unawaited native download pattern exists in realtime video delivery at [exporter.ts:633](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/legacy/core/exporter.ts:633); that branch was identified by code review, not separately reproduced.

**Fix direction:** treat canceled directory selection as cancellation, await every delivery operation, and propagate cancellation/write failures into export status. Prefer a single destination-folder workflow for sequences. Recheck cancellation plus a successful multi-frame sequence.

## Other actionable findings

### 5. P2 — Trashing the active project immediately restores it

Source: [projects.ts:255](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/legacy/ui/projects.ts:255).

**Manual reproduction:** Home → active project's actions → Move to Trash → confirm. A new Untitled project appears, but the original remains in All Projects and Trash stays at zero. The switch captures/persists the old active project after trashing it; `R.put` restores its live metadata and removes its Trash entry. Trashing the now-inactive project and restoring it both worked.

Switch/capture the active project before trashing, or suppress persistence of the project being removed.

### 6. P2 — Restored, playable media is labeled offline and cannot be reused from Media

Source: [AssetsPanel.svelte:466](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/panels/AssetsPanel.svelte:466).

**Manual reproduction:** import H.264/AAC media, save a `.pmv`, quit, and restart. The Media card says “Media offline” and “Missing,” while the same video renders and plays. Reopening the `.pmv` explicitly also reproduced the stale card. The card offers Locate instead of Add to timeline and sets `draggable` false.

**Evidence:** the PMV3 file contains the original 20,554-byte MP4; its code-export ZIP also contains that media and plays in an independent browser. This is not lost media. The keyed Svelte row derives offline state from nonreactive `PM.assets.get`; restoration changes the runtime map but retains the metadata object's identity, leaving the row's cached state stale. The review lane verified the generated Svelte dependency behavior.

Make runtime asset availability/poster changes an explicit row dependency and test asynchronous restoration after initial render.

### 7. P2 — Saved-project tabs retain an incorrect unsaved/file tooltip

Source: [Titlebar.svelte:315](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/shell/Titlebar.svelte:315).

After save/restart, the tab's accessibility label and tooltip reported “unsaved” / “Not saved to a file,” while Home displayed the correct `.pmv` association. The file association and saved-file hash were present on disk. The keyed tab reads nonreactive `PM.projectFileState`; its initial evaluation can occur before that function is installed, and same-name updates do not invalidate the tooltip. Do not treat the stale tooltip as proof of save failure.

### 8. P2 — Native Open Project leaves Home covering the opened document

Source: [app.ts:669](/Users/judekim/Developer/powermove/apps/desktop/src/renderer/src/legacy/app.ts:669).

**Manual reproduction:** while Home is visible, Command+O → choose a `.pmv` → Open. Powermove reports “Opened Release Audit.pmv” and adds its tab, but Home stays visible until the tab is clicked. The native open path does not dismiss Home, unlike the Home button's wrapper.

### 9. P2 — Imported extension CSS has no unload lifecycle

Source: [compiler.ts:193](/Users/judekim/Developer/powermove/apps/desktop/src/main/extensions/compiler.ts:193).

**Code-only finding:** CSS imports compile into unconditional `document.head.appendChild(style)` calls. Loader deactivation releases registered API resources, not these styles. Disabling/removing a mod can retain its styling; hot reload can accumulate prior rules. The tested Audit Tools mod inserted its style inside its panel, so that successful toggle test does not cover CSS imports. Give imported styles explicit extension ownership and dispose them on deactivate/reload.

## Completed checks

| Area | Result / actual exercise |
| --- | --- |
| Production desktop build | Passed |
| Desktop typecheck | Passed; zero errors and warnings |
| Unit tests | 270 files passed, one skipped; 2,187 tests passed, one skipped |
| Kernel/extension boundary check | Passed, 10 extensions |
| Fresh onboarding | Animation rendered, Welcome appeared, Begin opened empty Projects |
| Project creation and native save | Created disposable composition; saved `.pmv`; subsequent saves worked |
| Recovery and explicit reopen | Layers, keyframes, effect, take, media bytes survived; stale UI findings above |
| Media | Native H.264/AAC import, red/green frame playback, embedded delivery |
| Timeline and Properties | Scrub, play/pause, two opacity keys, curve view, effect controls |
| Content authoring | Solid creation, canvas text entry, drawing a mask on selected text |
| Effects | Gaussian Blur applied and rendered |
| Agent personalization | Real Codex **5.6 Sol / High** run created and loaded Audit Tools; no Astra agent use |
| Generated panel action | Click set selected opacity to 50%; Command+Z restored 100% |
| Extension lifecycle | Manual disable removed panel; enable restored it; mod survived restart |
| Library/workspaces | Inspected panel previews and workspace presets; switched Design to Animate |
| Takes | Saved Take 1; it survived restart |
| Settings | Composition/export/account/extension surfaces inspected; light theme applied; dark/light editor inspected |
| MP4 export | 90-frame H.264/AAC export succeeded; FFmpeg decoded all 90 frames and audio without error |
| PNG sequence cancellation | Failed; finding 4 reproduced with 89 rejected saves |
| Code export | ZIP contained scene, player, React wrapper, handoff docs and MP4; independent browser loaded it and playback advanced |
| Project Trash | Active path failed; inactive trash and restore passed |
| Shutdown/process lifecycle | Audit app and its helpers exited after normal quit; temporary web server stopped |

The Sol run took roughly four minutes, including live verification. It encountered one failed source-search command but recovered. It correctly said that it had not clicked the generated button; the reviewer performed that action and verified Undo. No broad repeated agent requests were needed.

## Limits of this review

- This is a broad release smoke test plus focused code review, not an exhaustive test of every format, effect, input combination, or extension.
- No full Playwright suite was rerun. Native computer-use tests complemented the complete unit gate.
- No fresh signing/notarization/Gatekeeper, updater-install, clean second-Mac, or final DMG/ZIP verification was performed. Release those artifacts only after their release lane passes.
- Claude, local/API providers, denied-account/auth flows, image sequences on the successful directory path, ProRes/HEVC/alpha, WebM/realtime output, external 3D assets, full shader authoring, transitions, and long editing sessions were not manually certified here.
- Notes, Shader, Performance, and Workspaces previews were inspected in Library, but not every operation in those panels was exercised. Take restore and panel popout/docking gestures were not covered.
- No sustained memory-leak claim is possible from this short session. Clean shutdown was observed; logs include ResizeObserver and GPU tile warnings without an independently established user-visible defect, so they are not reported as additional bugs.
- The existing hours-old blank dev window was excluded from findings; the fresh production-bundle launches worked.

## Evidence locations

- Audit profile and saved specimens: `/tmp/powermove-release-audit-20260917/` (`Release Audit.pmv`, `Release Audit.mp4`, `Release Audit-web.zip`, `extensions/audit-tools/`).
- UI/export failures: `/tmp/powermove-release-audit-ui.log`.
- Restart log: `/tmp/powermove-release-audit-restart.log`.
- Fresh onboarding profile/log: `/tmp/powermove-release-onboarding-20260917/`, `/tmp/powermove-release-onboarding.log`.
- Gate logs: `/tmp/powermove-desktop-unit-review.log`, `/tmp/powermove-desktop-typecheck-review.log`, `/tmp/powermove-desktop-build-review.log`.

Follow-up evidence: `/tmp/powermove-release-verify-20260917/`, `/tmp/powermove-fix-ui.log`, `/tmp/powermove-fix-unit-final.log`, `/tmp/powermove-fix-typecheck-final.log`, and `/tmp/powermove-fix-build-final.log`. Temporary evidence should be copied before system cleanup if needed. Next gate: finish the pending native checks after macOS unlock, then perform signed distribution-artifact validation.

Final permission follow-up evidence: `/tmp/powermove-fix-permissions-final.log` (16 passing tests); `/tmp/powermove-export-diagnostic2.log` (actual missing-URL check). The temporary compiled diagnostic logging was removed by a clean final build. Electron v44 source documents the null-window origin-only dispatch: https://raw.githubusercontent.com/electron/electron/v44.0.0/shell/browser/electron_permission_manager.cc .
