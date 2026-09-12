# AE fundamentals implementation

Implementation for the twelve findings in the September 4 audit, plus the reported playback stutter/audio-sync loss after switching windows. Implementation and hidden-app verification are complete for the checks listed below. The updated desktop runtime was opened with user authorization on September 5; new controls were verified in the original workspace. Exact live A/V sync after switching remains inconclusive because concurrent user interaction changed playback and project state during the check. No commit has been made.

| Area | Implemented behavior | Evidence |
| --- | --- | --- |
| Parenting | Preserve current world pose through rotated/scaled parent changes; independent child opacity; retain keyframes/expressions; reject singular transforms atomically | `ae-fundamentals.behavior.test.ts`: pose, expressions, locks, Undo |
| Precompose | Preserve external parent chains inside the nested composition and outside dependents with editable rig nulls | Rendered frames before/after at three times in `ae-audit-implementation.spec.ts` |
| Properties | Shared selection edits, mixed values, relative numeric scrubbing, locked-layer exclusions, selected stopwatches | Inspector/control tests; relative shape-width and per-layer keyframe/Undo tests; visible mixed fill assertion |
| Graphs | Multiple chosen curves; value/speed modes; numerical incoming/outgoing velocity/influence; fit; time/value scaling; units; responsive toolbar | Multi-curve interaction, velocity application, Undo and narrow dark-theme screenshot |
| Vectors/masks | Pen vertices/tangents, shape/mask context, path groups and transforms, trim/repeat, alpha/luma/inverted track mattes | Pen/mask clicks, editable channel serialization, alpha/luma pixel comparisons, matte-cycle rejection |
| Text | Canvas text editing and native selection; editable range styles; live character/word/line animator selectors | Text-entry interaction, animator visibility, selector pixel change and Undo, selector unit tests |
| Expressions | Syntax/identifier diagnostics, supported-name completion/reference, runtime error state | Expression/inspector suite and bounded interpreter tests |
| Timeline | Solo, Show Shy, layer/property search, selected-layer parent/mode/matte row | Timeline/inspector suite, rendered matte behavior and visual review |
| Motion blur | Evaluate animated toggle state for timeline display and menu actions | Timeline and inspector regression suites |
| Retiming | Integrated animated speed, keyframed source time, freeze/reverse, explicit remapped-audio policy, decoded offline frames per media instance | Source-time/Undo tests and two copies of one video showing different source frames |
| Preview | Explicit resolution/rate, bounded frame cache and cached playback; absolute elapsed transport clock, loop overshoot and gap resynchronization | Cache/play/stop interaction, resolution assertion, 9 engine tests including background gaps and audio seek |
| Delivery | Persistent queue and reusable presets; native H.264/AAC and ProRes 4444 alpha; fractional rates; explicit sRGB/BT.709 output tags | Native encoder decode/alpha test, playable MP4 with audio, queued ProRes plus close/reopen persistence, preset close/reopen persistence |

## Validation

- Production build passes.
- TypeScript and Svelte checks pass: 0 errors, 0 warnings.
- Latest full unit run: 1,517 passed, 1 skipped, 1 failed (the calibrated expression-performance threshold described below).
- Subsequent inspector/control/timeline checks after the final mixed-content correction: 112 passed.
- Hidden Electron: 13 combined audit/export/viewer tests passed; the latest eight audit tests passed again after the final mixed-content correction. Native dialogs were replaced only inside isolated hidden test sessions.
- Extension compiler now supports the shared `powermove` model helpers without importing outside an extension's directory; all source-heavy built-in extension compilation checks pass.
- `git diff --check` passes.

The recorded expression-performance threshold also fails on the pre-implementation source reconstructed from HEAD plus the original tracked patch. That baseline measured 112.32 ms against its calibration-derived limit; the optimized implementation measured 93.75 ms in an isolated run, but still missed the old ratio threshold narrowly. Full-suite timings vary. The threshold has not been loosened or re-recorded; this remains a reported failed check rather than a clean-suite claim. The newly introduced repeated per-frame temporal conversion was removed, and evaluation uses cached native curve data.

## Runtime and visual evidence

With explicit restart authorization, computer use saved the original project to `/Users/chike/Downloads/Powermove commercial — restart backup 2026-09-05.pmv` and quit normally. The first launch used the default profile; it was closed normally and the original profile `/private/tmp/powermove-build-open-profile.ozzZhm` was restored. Computer use confirmed the original project with two layers and six keys plus the new Pen, preview, text-animation, and matte controls. Playback and window switching were exercised, but concurrent user interaction changed the project and transport, so exact live A/V sync remains unverified. User changes were not reverted. Runtime log: `/private/tmp/pm-live-correct-profile-20260905.log`.

Hidden-renderer screenshots were reviewed in light and dark themes, including a 1100-pixel-wide window. They exposed and led to repairs for overlapping timeline controls, path raster/vertex alignment, an empty Content section, preview control styling, and mixed-content display.

Evidence directory: `/private/tmp/powermove-ae-implementation-visual/` (`paths.png`, `text-animator.png`, `multi-graph.png`, `multi-graph-dark-narrow.png`).

Logs: `/private/tmp/pm-final-tests.log`, `/private/tmp/pm-final-types-2.log`, `/private/tmp/pm-mixed-tests.log`, `/private/tmp/pm-e2e-final.log`, `/private/tmp/pm-audit-latest.log`, `/private/tmp/pm-queue-fixed.log`, `/private/tmp/pm-baseline-perf.log`.

Existing unrelated working-tree changes remain intact. The original tracked-diff baseline is `/private/tmp/powermove-full-audit-baseline-20260905/tracked.patch`; the source archive in that directory was written later and contains intermediate implementation work, so it must not be treated as an untouched original snapshot. Ask before committing and stage only the audit changes.

## User-requested interface refinements (September 5)

Restored the compact timeline transport within the ruler gutter; removed the full-width toolbar, its search/Shy controls and relation row. Removed the audio Source field and visibility stopwatch; new visibility keyframes are rejected and ordinary visibility edits store a boolean. Older saved visibility animation remains readable for compatibility. Removed the preview frame-rate and cache buttons, moved resolution beside Fit, and gave those menus and timeline buttons consistent subtle backgrounds. Graph velocity and scale remain available from the keyframe context menu.

These panels were applied in the existing process through the supported user-extension replacement mechanism: `/private/tmp/powermove-build-open-profile.ozzZhm/extensions/interface-cleanup` contains source copies of timeline, inspector, and viewer and replaces those bundled panels. Future changes to those source panels must also refresh this fork, or remove it after the matching compiled source is loaded. No whole-window reload or restart occurred. The hidden reload test verifies the same project and WebGL canvas survive, the simplified layout aligns, and visibility toggling undoes. Computer use confirmed the compact timeline, adjacent Auto/Fit menus, audio Source removal, and plain Visible switch in the open window.

Final button styling uses a 5% foreground tint over the panel color, without borders or shadows. Latest focused checks: 60 unit tests passed and the hidden interface-cleanup reload test passed after waiting for the initial canvas boot. The wider audit rerun encountered two missing Pen-button selectors after concurrent toolbar changes and one session-start timeout; its initial reload assertion also ran before canvas boot and was corrected and passed independently. Typecheck encountered four errors in the concurrently edited toolbar entry. Those unrelated toolbar edits were left intact. Logs: `/private/tmp/pm-ui-cleanup-tests.log`, `/private/tmp/pm-ui-cleanup-targeted-final.log`, `/private/tmp/pm-ui-cleanup-types.log`, `/private/tmp/pm-ui-cleanup-final-e2e.log`.
