# W1 — workflow acceptance inventory

## Baseline and decisions

**Today is the shipped tag `v1.2.0`, commit `07ed5fa17ee640659cb075f9cad1af2095011d60`.** The two input inventories describe main at `d9a98ea5a7db1df245627eb0246713a53452a9b8`; they are useful inputs, not the shipped oracle. All shipped source references below are immutable Git objects read from this repository. The later 370-spec roster is frozen at worktree commit `212a3cc73e87aa189eb55b4daadce4c1a470a38b`; it is not the tag’s spec count.

This document defines **78 shipped workflow groups** and **5 explicit cutover-addition groups** (83 total acceptance groups). All **1,392** original rows are accounted for: **1344 rebased rows** in the workflow cases and **48 wholly post-tag claims** in the separate appendix. Mixed rows are rewritten to the tag, with later deltas separately recorded. “Absent” notes are audit boundaries, not invented shipped features. The registry map has **195 entries**, each an independent acceptance variant in [w1-keymap.md](w1-keymap.md).

Decisions follow the execution baseline v6: pro editor expectations first (N1); exact authored geometry, text, timing, color, shader coordinates and API semantics (N3); every existing project, extension, theme, state and media remains usable with safe migration/rollback (N4). No browser implementation quirk becomes a native requirement. Keep the frozen Electron reference until the replacement gates pass; prune replaced code only after its last production consumer is gone. These documents contain source contracts and proposed native acceptance tests; they do not claim the tests were executed or native parity is complete.

**Plan/source discrepancies are explicit, not waived.** J/K/L shuttle is absent from the tag and exists on the later engine branch. Persistent linked A/V and ripple delete are also absent as editing workflows in the tag. They remain named cutover requirements, with new acceptance groups below. Ordinary trim, a method named slide that moves a strip, transform “Parent/Link Layers”, and an animation named spatial ripple are not evidence of pro slide, linked A/V or ripple editing. Source slip does exist through `edit_video`; no dedicated slip pointer tool ships.

The shipped standalone web-animation export exists at the tag and was later deleted on main. **Disposition (owner decision, 2026-10-29): removed from the product; not a cutover requirement.** The @powermove/player package stays for the website. It stays listed in W1 for traceability. Existing remote browser/serve workflows are inventoried, with the native plan’s explicit out-of-scope boundary recorded. This is an explicit scope decision, not an assertion that those features did not ship.

## How to execute these acceptance contracts

Use synthetic media: visible frame numbers and distinct colors, a known audio tone/pulse, alpha frames, stills, nested compositions, locked groups, animated channels and small/large saved projects. Use temporary profiles/destinations and injected failures, never real user secrets or private project identifiers. For migration cases include cached media whose original is gone, proxy-only records and already-offline references, including references retained only by undo history.

Each W group gives a goal, preconditions, steps and expected result. Its numbered A/B cases are parameterized subtests: repeat the group steps for the named target/condition and assert that row’s result. A/B IDs are stable ordinal IDs assigned to the original checkbox rows, including duplicates; they are traceability IDs, not extra workflow counts. Preserve the source file/test provenance beside each case. Reversible edits also require cancel, Undo and Redo; persisted operations require save/reopen. A control-level unit contract is not silently promoted to an e2e pass. Shared edit variants use K007/K008 (cmd/ctrl+z), K009/K010 (cmd/ctrl+shift+z), and the owning local Escape/cancel route; those keys are inherited by every reversible-edit workflow below.

Keyboard steps refer to exact K IDs and chords in the companion file, including arguments, priority, field/repeat/loose flags. Local context keys are listed there separately. “No registered key” means use the named pointer/menu/API route; do not invent a shortcut. Per workflow, existing shipped e2e files are separated from later candidate specs. No matching e2e means add native coverage; it does not excuse the workflow.

Use the three oracles: (1) standards/f64 for engine content; (2) frozen behavioral traces for calls, returns, identities, events, Undo and errors; (3) classified frozen-reference visual differences only for extension-drawn/themed content. Geometry/text placement are exact; edge coverage tolerance is 2/255 per pixel with at most 0.5% of edge pixels beyond 1/255; color ΔE2000 ≤1 at p99 and ≤2 max except reviewed spec corrections. Shell/native chrome may change. A plumbing-only retirement waits for replacement consumers/gates and never discards its underlying safety requirement.

## Required pro-editing verdicts

| Feature | EXISTS in v1.2.0? | Exact scope and acceptance | Shipped evidence |
|---|---|---|---|
| J/K/L shuttle ±1×/±2× with decline | NO | W079; shipped J/K navigate events and L reveals audio levels. Later priority-90 engine bindings decline when unavailable/paused. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L133`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts:L372` |
| In/out points | YES, work/layer bounds; NO source-monitor marks | W052; B/N work area, I/O selected layer boundaries, Alt+[ / Alt+] ordinary trims. Source-range three-point marks are new. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L137`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts:L74` |
| Markers | YES | W053; frame-aligned ruler add/remove, duplicate protection, persistence/Undo. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.ts:L10` |
| Snapping | YES | W054; playhead/clip/key directional snap, viewer alignment and modifier bypass. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts:L689` |
| Ripple delete | NO | W080; shipped remove/delete deletes layers but does not close a downstream gap. | `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L113`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts:L256` |
| Linked A/V | PARTIAL: embedded A/V YES; persistent linked-pair edits NO | W059 and W083; separating creates an independent audio strip, not a durable edit-link pair. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts:L721`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L95` |
| Source handles | YES | W056 and W057; API insert/trim/slip bounds sourceIn + duration×rate; ordinary UI trim behavior is separately covered. | `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L59`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts:L307` |
| Frame-rate conversion | YES: source-time sampling/output-rate conversion | W071; composition and export rates may differ from source rate. Do not claim optical-flow interpolation. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts:L176`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/video-timeline.ts`, `v1.2.0:apps/desktop/src/shared/mp4-frame-rate.ts` |
| Source-monitor editing | PARTIAL: preview/scrub YES; source insert/overwrite NO | W021 and W082; edit_video insert takes explicit numbers but does not implement source-monitor three-point insert/overwrite. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts:L265`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L32` |
| Offline media/relink | YES | W022; missing versus changed originals, stable asset replacement, tag v2 fingerprint rule below. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts:L153`, `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts:L1003`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/project-file.ts` |
| Project recovery | YES | W004; library checkpoints/journal/persisted undo and renderer recovery. Native one-second durability is a stronger gate, not measured tag behavior. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/projects.ts:L78`, `v1.2.0:apps/desktop/src/main/renderer-recovery.ts` |
| Export cancellation | YES | W069; cancel chooser/progress/encoding, cleanup, resume playback and export again. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts:L307`, `v1.2.0:apps/desktop/e2e/export-mp4.spec.ts:L61` |
| Nested comps | YES | W062; precompose/open/nest/save/undo and independent timed instances. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compositions.ts`, `v1.2.0:apps/desktop/e2e/compositions.spec.ts:L14`, `v1.2.0:apps/desktop/e2e/video-frame-integrity.spec.ts:L250` |
| Retiming | YES, with constraints | W058; Speed preserves retained source span; explicit sourceTime/timeRemap animation; API guards static timing/embedded audio. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/editing.ts:L247`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L87` |
| Slip | YES via edit_video; NO dedicated mouse tool | W057; source-only offset, unchanged animation/strip, handle guard. | `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L69`, `v1.2.0:apps/desktop/src/renderer/src/legacy/__tests__/agent-video-editing.test.ts:L95` |
| Slide | NO pro slide edit | W081; timeline slide(e) moves strip; it does not compensate both neighbor trims. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts:L3496`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L24` |
| Roll | NO | W081; no paired adjacent-edge roll operation in keymap/UI/edit_video. | `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L24`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| Ripple trims | NO | W080; ordinary trim modifies clip start/duration/source without shifting downstream strips. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts:L958`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L59` |
| Three-point editing | NO | W082; source marks and insert/overwrite from source monitor are explicit cutover additions. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts:L32` |
| Every registered keymap entry | YES: 195 | W010 plus per-row workflow in w1-keymap.md; menus and local keys are also enumerated there. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/index.ts:L50`, `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts:L284` |

The negative verdicts are bounded by the complete shipped keymap, source-preview controller, edit_video operation allowlist and timeline drag/timing implementations, not by a failed text search alone. No dedicated slip tool, pro slide/roll/ripple trim or source three-point edit can be inferred from a similarly named helper.

### Relink identity and migration cases

The tag fingerprint uses up to 64 KiB from each unique offset: 0, floor((size−sample)/2), size−sample, with sample=min(64 KiB,size). Hash the UTF-8 header `powermove-media-v2\n<size>\n` followed by those samples in order with SHA-256; store `v2:<size>:<hex>`. Filename/MIME are not in the hash, so a rename or picker MIME change must still match. For legacy records without fingerprint only, matching requires media kind and normalized name plus same positive size or duration within 0.05 seconds. Candidate priority favors live assets, then references, then exact fingerprint. This is sampled identity, not a full-file collision guarantee.

Acceptance variants: renamed matching file accepted; changed sampled bytes rejected for automatic recovery; same basename with wrong fingerprint rejected; explicit Replace intentionally substitutes compatible content and remains undoable; partial sample collision is not presented as full identity. Import fingerprint is computed from the original selected file before conversion. For unplayable video, the persisted blob can be a playback proxy; preserve original fingerprint/sourcePath separately. Native migration records BLAKE3 after resolving/verifying the available original; if absent, retain proxy-only/offline state and warning, never relabel proxy pixels as originals. Already-offline and undo-only references survive migration. See `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts:L153` and `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts:L868`.

### Additional pro cutover scope beyond shipped W1

The plan’s additional native colour/format/proxy requirements are not proven by shipped UI labels. v1.2.0 has preview Auto/Full/Half/Quarter, automatic conversion proxies for unsupported video, depth controls, embedded audio/mixer and format-gated export. It does not have the full pro media management suite: explicit Proxy Preferred/Only modes, Toggle Proxies, ingest presets and per-selection proxy lifecycle, attach-existing proxies, optimized media, disk render cache/render bar/Keep Rendered, Background Tasks pause/priority and per-cache storage controls. Main’s later Storage screen is not evidence that those shipped. Camera-log vendor transforms, `.cube` apply UI and waveform/parade/vectorscope/histogram panels are also absent from the tag; later main effect/LUT support is separate. They remain additional cutover gates rather than retroactive W1 parity claims. W1 does not claim performance, all planned native codecs/HDR/display paths, system audio-device switching or native text services are already met.

## Workflow contracts

### W001 — Launch, restore and activate the last project

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A fresh profile and a profile with two windows, several projects and an older saved-session shape.
**Steps:** Launch; activate from Dock; relaunch with restoration on and off; open a document during startup.
**Expected:** Only the last active project restores; other projects remain in Recents; no startup file is lost.
**Exact registered keymap entries:** K045 cmd+o → open; K046 ctrl+o → open; K047 cmd+shift+o → open; K048 ctrl+shift+o → open; K053 cmd+p → projects; K054 ctrl+p → projects.
**Native menu key equivalents:** M008 CommandOrControl+O → open.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts`.
**Existing shipped e2e:** `boot.spec.ts`; `large-store-startup.spec.ts`; `project-save.spec.ts`; `project-startup.spec.ts`; `shell.spec.ts`.
**Later candidate e2e (not shipped evidence):** `document-host-boot.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A006 | Restore migration: Read window-tab sessions, older one-project windows, then legacy open tabs; deduplicate known IDs with 12-window/40-tab bounds. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A007 | Launch restoration: Default to reopening only the last active project from the last focused window; retain other projects in Recents. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A008 | Restoration opt-out: Start on Projects when restoreWindows is false. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A012 | Single instance: Request an application lock; route .pmv arguments and deep links from later processes into the first process. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A013 | Quiet duplicate launch: A macOS second launch with no project or link must not steal focus. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A014 | Finder/Dock documents: Queue open-file events before readiness so opening a .pmv during startup is not lost. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A015 | Deep links: Queue powermove:// open-url before ready; packaged builds claim the scheme, development builds only with explicit opt-in. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A016 | External links: Route supported external HTTP(S) navigation through the system browser rather than arbitrary editor windows. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A032 | Acceptance project-startup: Restart opens only the previously active project tab. | `v1.2.0:apps/desktop/e2e/project-startup.spec.ts:L6` |
| A033 | Acceptance project-startup: Restart restores one project from the last focused window. | `v1.2.0:apps/desktop/e2e/project-startup.spec.ts:L29` |
| A034 | Acceptance project-startup: Turning off project restoration starts on Projects without opening another recent project. | `v1.2.0:apps/desktop/e2e/project-startup.spec.ts:L51` |
| A035 | Acceptance project-startup: An old single-autosave profile restores its only project. | `v1.2.0:apps/desktop/e2e/project-startup.spec.ts:L64` |
| A289 | Acceptance large-store-startup: Opens with a large recovery tree and preserves every record. | `v1.2.0:apps/desktop/e2e/large-store-startup.spec.ts:L7` |

### W002 — Move, reorder and focus project tabs across windows

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Two open projects and two editor windows.
**Steps:** Reorder tabs; tear off a tab; drop it on another window; open the same project again; minimize and reveal its owner.
**Expected:** One owner per project, preserved edit/history state, correct focused command target and usable window drag area.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**Native menu key equivalents:** M009 CommandOrControl+W → closeTab; M029 Control+Tab → nextTab; M030 Control+Shift+Tab → previousTab; M031 CommandOrControl+Shift+N → newWindow; M032 CommandOrControl+Shift+W → closeWindow.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/windows.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/Titlebar.svelte`, `v1.2.0:apps/desktop/src/main/index.ts`.
**Existing shipped e2e:** `interface-polish.spec.ts`; `shell.spec.ts`; `window-drag-region.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A001 | Editor windows: Allow multiple independent editor windows containing ordered project tabs; one project cannot be owned by two windows. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A002 | Window focus: Menu targets and Dock activation use the most recently focused live editor; revealing restores minimized windows. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A003 | Initial dimensions: Create 1440×900 windows with a 980×640 minimum and cascade additional windows. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A004 | Keep native traffic lights and usable titlebar/drag space; active-window material follows focus. The tag’s hidden-inset/x14/y15 implementation is reference chrome, not authored content to copy pixel-for-pixel. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A005 | Fullscreen: Notify the renderer on enter/leave fullscreen so titlebar space formerly reserved for traffic lights is reclaimed. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A009 | Project ownership: Opening an already-owned project raises its owner rather than creating competing autosaves. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A010 | Tab reorder: Deduplicate requested tab order and retain omitted owned tabs at the end; never add/drop ownership through reorder. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A011 | Tab tear-off: Move a tab into a new window or another window's title strip; topmost hit window wins and strip drop band is 56 pixels. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/windows.ts` |
| A046 | Acceptance shell: Boots the Svelte chrome while the dock panels stay live. | `v1.2.0:apps/desktop/e2e/shell.spec.ts:L25` |
| A047 | Open projects as tabs and switch between those tabs; the centered title-menu design is later. | `v1.2.0:apps/desktop/src/renderer/src/shell/Titlebar.svelte`, `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A048 | Acceptance shell: Closing a project shows its neighbour, and closing the last shows Projects. | `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A049 | Acceptance shell: Relaunch opens the active document while retaining the others in Recents. | `v1.2.0:apps/desktop/e2e/shell.spec.ts:L102` |
| A050 | Acceptance shell: A tab moves to a window of its own, and one project is never a tab in two windows. | `v1.2.0:apps/desktop/e2e/shell.spec.ts:L144` |
| A051 | Drop a project tab on another window title strip to transfer ownership. | `v1.2.0:apps/desktop/src/main/windows.ts`, `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A053 | Acceptance shell: Keeps a usable window drag area with many projects and overlays. | `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A054 | Acceptance shell: Switching back to a project resumes it from memory instead of reloading it. | `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A055 | Rename from the individual tab menu; retain editability and saved name. | `v1.2.0:apps/desktop/src/renderer/src/shell/Titlebar.svelte`, `v1.2.0:apps/desktop/e2e/shell.spec.ts` |
| A056 | Acceptance window-drag-region: The agent overlay leaves the titlebar grab area draggable. | `v1.2.0:apps/desktop/e2e/window-drag-region.spec.ts:L3` |
| A258 | Titlebar local keys: F2 starts project rename; Enter commits and Escape cancels while keeping focus local to rename. | `v1.2.0:apps/desktop/src/renderer/src/shell/Titlebar.svelte` |

### W003 — Close a project or quit without losing work

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Untitled, saved, changed and unchanged projects; a pending import and a simulated write failure.
**Steps:** Close tab/window and quit; choose Save, Don’t Save and Cancel in separate runs; retry after a failed save.
**Expected:** Cancellation keeps edits open; successful saves precede close; discarded untitled projects stay discarded. On macOS v1.2.0 closing the last window keeps the app running.
**Exact registered keymap entries:** K041 cmd+s → save; K042 ctrl+s → save; K043 cmd+shift+s → saveAs; K044 ctrl+shift+s → saveAs.
**Native menu key equivalents:** M009 CommandOrControl+W → closeTab; M010 CommandOrControl+S → save; M011 CommandOrControl+Shift+S → saveAs; M032 CommandOrControl+Shift+W → closeWindow; M035 Command+Q → quit.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts`.
**Existing shipped e2e:** `project-close-unchanged.spec.ts`; `project-save.spec.ts`.
**Later candidate e2e (not shipped evidence):** `quit.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A018 | Await pending imports and recovery writes before closing; cancellation preserves editing session. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts` |
| A019 | Offer Save, Don’t Save and Cancel; cancelled Save leaves editor open. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts` |
| A020 | Skip prompts for unchanged local documents. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts` |
| A021 | Discard a never-saved project without a later flush restoring it to Recents. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts` |
| A023 | A failed durable close flush leaves edits open and retryable; implementation lives in index.ts, not later editor-close.ts. | `v1.2.0:apps/desktop/src/main/index.ts` |
| A297 | Acceptance project-close-unchanged: Closing an unchanged local project skips the save prompt, while edits still prompt. | `v1.2.0:apps/desktop/e2e/project-close-unchanged.spec.ts:L5` |
| A298 | Acceptance project-close-unchanged: Don’t Save removes a never-saved project from Recents and recovery across restart. | `v1.2.0:apps/desktop/e2e/project-close-unchanged.spec.ts:L38` |
| A299 | Acceptance project-close-unchanged: Don’t Save on window close survives the final recovery flush. | `v1.2.0:apps/desktop/e2e/project-close-unchanged.spec.ts:L60` |

### W004 — Recover edits, undo and external connections after a crash

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A library project with a persisted checkpoint, journal patches, redo entries and a connected external client.
**Steps:** Make edits; flush recovery; terminate the editor process; accept recovery; reopen; undo/redo; reconnect with the same MCP client.
**Expected:** Recovered document and retained undo match the durable checkpoint plus journal; missing editor does not trap close; external connection remains usable. Native acceptance also bounds acknowledged-edit loss to one second.
**Exact registered keymap entries:** K007 cmd+z → undo; K008 ctrl+z → undo; K009 cmd+shift+z → redo; K010 ctrl+shift+z → redo.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/renderer-recovery.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/history.ts`, `v1.2.0:apps/desktop/src/main/index.ts`.
**Existing shipped e2e:** `large-store-startup.spec.ts`; `memory-history-persistence.spec.ts`; `persistence.spec.ts`; `project-save-inactive.spec.ts`; `renderer-crash.spec.ts`.
**Later candidate e2e (not shipped evidence):** `background-autosave.spec.ts`; `engine-document-recovery.spec.ts`; `engine-render-transport.spec.ts`; `project-open-crash-recovery.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A024 | Offer crash recovery and close without waiting for dead editor. | `v1.2.0:apps/desktop/src/main/renderer-recovery.ts`, `v1.2.0:apps/desktop/src/main/index.ts` |
| A025 | External MCP connection survives editor crash and reload. | `v1.2.0:apps/desktop/src/main/renderer-recovery.ts` |
| A043 | Acceptance renderer-crash: Offers recovery after an editor process crashes and restores its latest saved project. | `v1.2.0:apps/desktop/e2e/renderer-crash.spec.ts:L4` |
| A044 | Acceptance renderer-crash: A crashed editor can close without waiting for a save from the missing process. | `v1.2.0:apps/desktop/e2e/renderer-crash.spec.ts:L39` |
| A045 | Acceptance renderer-crash: The external MCP connection survives editor crash and reload without timing out. | `v1.2.0:apps/desktop/e2e/renderer-crash.spec.ts:L52` |

### W005 — Inspect and apply app/runtime updates

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A stubbed available update and failed download, with an unsaved project.
**Steps:** Check updates; download; cancel or retry failure; request restart; inspect and dismiss version changes.
**Expected:** Visible progress and recoverable errors; document close protection; app/runtime version notices persist independently.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/app-updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/WhatsNew.svelte`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.
**Later candidate e2e (not shipped evidence):** `runtime-scratch-sweep.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A026 | App updates: Check and present update state through the shell, with download/install progress and failure handling. | `v1.2.0:apps/desktop/src/main/updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/app-updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/WhatsNew.svelte` |
| A028 | What’s New: Present version-specific release changes and persist seen/dismissed state. | `v1.2.0:apps/desktop/src/main/updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/app-updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/WhatsNew.svelte` |
| A029 | Runtime updates: Track runtime updates independently of app document content. | `v1.2.0:apps/desktop/src/main/updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/app-updates.ts`, `v1.2.0:apps/desktop/src/renderer/src/shell/WhatsNew.svelte` |

### W006 — Create, open, rename, duplicate and trash projects

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A project library, an editable .pmv, supported media and an unsupported drop.
**Steps:** Create with name, dimensions, rate, duration and background; cancel another creation; open/import/drop; rename; duplicate; trash and restore a closed project.
**Expected:** Independent projects and Recents; unsupported/cancelled opens preserve the current document; trash is recoverable.
**Exact registered keymap entries:** K045 cmd+o → open; K046 ctrl+o → open; K047 cmd+shift+o → open; K048 ctrl+shift+o → open; K053 cmd+p → projects; K054 ctrl+p → projects; K057 cmd+alt+n → newProject; K058 ctrl+alt+n → newProject.
**Native menu key equivalents:** M007 CommandOrControl+Alt+N → newProject; M008 CommandOrControl+O → open.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/projects.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts`.
**Existing shipped e2e:** `agent-project-timeline.spec.ts`; `project-save.spec.ts`; `projects-drop.spec.ts`; `shell.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A169 | Cmd/Ctrl+o: Run &#96;open&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A170 | Cmd/Ctrl+shift+o: Run &#96;open&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A173 | Cmd/Ctrl+p: Run &#96;projects&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A175 | Cmd/Ctrl+alt+n: Run &#96;newProject&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A260 | Project library: Provide project cards/list navigation, search, pagination, recent file state and Trash actions. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A261 | New project form: Ask name, resolution, custom width/height, frame rate, duration and background before creating; Cancel preserves current document. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A262 | Resolution presets: Offer 3840×2160, 1920×1080, 1280×720, 1080×1920, 1080×1350, 1080×1080 and Custom. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A331 | Acceptance projects-drop: Dropping a project opens it once and keeps the previous composition intact. | `v1.2.0:apps/desktop/e2e/projects-drop.spec.ts:L45` |
| A332 | Acceptance projects-drop: Unsupported and empty drops leave Projects and the current project unchanged. | `v1.2.0:apps/desktop/e2e/projects-drop.spec.ts:L63` |

### W007 — Save and reopen a portable project with media and history

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A project with imported video/audio, animation, layouts, large media and history; a writable temporary destination.
**Steps:** Save; edit and save again; Save As; reopen without session caches; repeat from a field and an inactive tab; inject picker cancel, write failure and external modification.
**Expected:** Correct destination reuse, editable source/media/history/layout round-trip, bounded streaming memory and retained dirty state on failure.
**Exact registered keymap entries:** K041 cmd+s → save; K042 ctrl+s → save; K043 cmd+shift+s → saveAs; K044 ctrl+shift+s → saveAs.
**Native menu key equivalents:** M010 CommandOrControl+S → save; M011 CommandOrControl+Shift+S → saveAs.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/project-file.ts`, `v1.2.0:apps/desktop/src/main/project-save.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts`.
**Existing shipped e2e:** `agent-project-persistence.spec.ts`; `editor-reliability.spec.ts`; `editor-responsiveness.spec.ts`; `menu-save.spec.ts`; `persistence.spec.ts`; `project-document-worker.spec.ts`; `project-export-fixture.spec.ts`; `project-file-media.spec.ts`; `project-files-memory.spec.ts`; `project-panel-persistence.spec.ts`; `project-save-inactive.spec.ts`; `project-save-large-history.spec.ts`; `project-save-large-media.spec.ts`; `project-save.spec.ts`; `project-streaming.spec.ts`.
**Later candidate e2e (not shipped evidence):** `background-autosave.spec.ts`; `linked-media-storage.spec.ts`; `media-store.spec.ts`; `storage-upgrade.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A167 | Cmd/Ctrl+s: Run &#96;save&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A168 | Cmd/Ctrl+shift+s: Run &#96;saveAs&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A265 | Save destination: Save writes editable .pmv, reuses its destination, and Save As changes the tracked destination. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A266 | Save safety: Cancellation, failed writes and external modification conflicts retain unsaved state; inactive project saves preserve live edits. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A267 | Portable document: Save/open preserve editable source, media, history and project layout; large media are streamed with bounded memory. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A292 | Acceptance menu-save: Routes both the renderer accelerator and native menu item to PM.cmd. | `v1.2.0:apps/desktop/e2e/menu-save.spec.ts:L6` |
| A295 | Acceptance persistence: Keeps an imported project and its edits when closed immediately. | `v1.2.0:apps/desktop/e2e/persistence.spec.ts:L8` |
| A296 | Acceptance persistence: Saves a project and restores it after relaunch. | `v1.2.0:apps/desktop/e2e/persistence.spec.ts:L26` |
| A301 | Acceptance project-file-media: Saved project files restore imported video without the original session media. | `v1.2.0:apps/desktop/e2e/project-file-media.spec.ts:L9` |
| A302 | Acceptance project-file-media: Saved project files restore imported audio without the original session media. | `v1.2.0:apps/desktop/e2e/project-file-media.spec.ts:L77` |
| A303 | Acceptance project-file-media: Leaving a local project for another and coming back keeps imported audio. | `v1.2.0:apps/desktop/e2e/project-file-media.spec.ts:L132` |
| A304 | Acceptance project-file-media: Local recovery keeps imported audio after closing and relaunching Powermove. | `v1.2.0:apps/desktop/e2e/project-file-media.spec.ts:L170` |
| A305 | Acceptance project-file-media: Media library items reveal their original file in Finder. | `v1.2.0:apps/desktop/e2e/project-file-media.spec.ts:L198` |
| A314 | Acceptance project-save-inactive: Saving an inactive tab preserves live edits after a local recovery write fails. | `v1.2.0:apps/desktop/e2e/project-save-inactive.spec.ts:L6` |
| A315 | Acceptance project-save-large-history: Large undo history survives autosave, native Save, and relaunch. | `v1.2.0:apps/desktop/e2e/project-save-large-history.spec.ts:L6` |
| A316 | Save/back up/reopen portable media beyond 256 MiB; later main changes storage/version retention. | `v1.2.0:apps/desktop/e2e/project-save-large-media.spec.ts` |
| A317 | Acceptance project-save: Command+S writes real files, reuses the destination, saves from a field, and Save As changes it. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L35` |
| A318 | Acceptance project-save: Native Open preserves editable source and subsequent saves update the opened file. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L69` |
| A319 | Acceptance project-save: Projects screen exposes file state and saves active, duplicated, and opened projects independently. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L102` |
| A320 | Acceptance project-save: Open on Home starts a blank project from media. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L166` |
| A321 | Acceptance project-save: Cancel, disk-write failures, and external modifications never clear unsaved changes. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L180` |
| A322 | Acceptance project-save: Renderer and native window close honor Cancel and cancelled Save. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L199` |
| A323 | Acceptance project-save: Save on native window close writes the document before closing. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L223` |
| A324 | Acceptance project-save: Native Quit honors Cancel without closing the editing session. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L236` |
| A325 | Acceptance project-save: A project opened from Finder while the editor is still loading is not dropped. | `v1.2.0:apps/desktop/e2e/project-save.spec.ts:L252` |
| A329 | Acceptance project-streaming: Opens and saves media beyond 1 GiB with bounded main-process memory. | `v1.2.0:apps/desktop/e2e/project-streaming.spec.ts:L10` |
| A330 | Acceptance project-streaming: Portable Export uses native streaming and honors a cancelled file picker. | `v1.2.0:apps/desktop/e2e/project-streaming.spec.ts:L94` |

### W008 — Open large projects while retaining input and tab intent

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Large synthetic projects and one already open project with undo.
**Steps:** Open; interact while loading; choose another tab; close a pending tab; repeat the same open; open an invalid file.
**Expected:** Input remains responsive, newest choice wins, duplicate requests join, cancellation is complete and parked document/history are intact.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/projects.ts`.
**Existing shipped e2e:** `editor-responsiveness.spec.ts`; `project-document-worker.spec.ts`; `project-files-memory.spec.ts`; `project-loading.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A300 | Acceptance project-document-worker: A large native document validates outside main and remains editable, undoable, and saveable. | `v1.2.0:apps/desktop/e2e/project-document-worker.spec.ts:L8` |
| A306 | Acceptance project-files-memory: Opening a large native project releases its main-process document after import. | `v1.2.0:apps/desktop/e2e/project-files-memory.spec.ts:L7` |
| A307 | Acceptance project-loading: Loading placeholders never bleed through Projects. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L3` |
| A308 | Acceptance project-loading: A hidden window finishes loading keyframe-heavy projects without timer delays. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L40` |
| A309 | Acceptance project-loading: Opening a large stored project allows input and paint between loading steps. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L71` |
| A310 | Acceptance project-loading: Closing the pending tab cancels its load and repeated clicks join the same open. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L120` |
| A311 | Acceptance project-loading: All open tabs retain their document, Undo state and cached artwork. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L148` |
| A312 | Acceptance project-loading: A newer tab choice wins over an unfinished open and failures release the editor. | `v1.2.0:apps/desktop/e2e/project-loading.spec.ts:L185` |

### W009 — Create and configure compositions and delivery defaults

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** An open project and at least two compositions.
**Steps:** Create/cancel; edit name, dimensions, duration, frame rate, depth, background and export defaults; reopen the settings.
**Expected:** Live, undoable composition edits; supported depth choices; settings persist and do not create a project unexpectedly.
**Exact registered keymap entries:** K055 cmd+n → newComposition; K056 ctrl+n → newComposition; K061 cmd+alt+k → compositionSettings; K062 ctrl+alt+k → compositionSettings.
**Native menu key equivalents:** M026 CommandOrControl+N → newComposition; M027 CommandOrControl+Alt+K → compositionSettings.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compositions.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`.
**Existing shipped e2e:** `background-color-drag.spec.ts`; `color-depth.spec.ts`; `compositions.spec.ts`; `new-composition-menu.spec.ts`; `open-media.spec.ts`; `project-settings.spec.ts`.
**Later candidate e2e (not shipped evidence):** `project-color.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A174 | Cmd/Ctrl+n: Run &#96;newComposition&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A177 | Cmd/Ctrl+alt+k: Run &#96;compositionSettings&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A263 | Project composition settings: Edit live name, resolution, size, frame rate (1–240), duration, color depth and background; unsupported depth choices are disabled. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A293 | Acceptance new-composition-menu: Right-click menus create a composition and preserve the current project on cancel. | `v1.2.0:apps/desktop/e2e/new-composition-menu.spec.ts:L6` |
| A326 | Acceptance project-settings: Settings › Project edits the live composition and its export settings. | `v1.2.0:apps/desktop/e2e/project-settings.spec.ts:L3` |
| A327 | Acceptance project-settings: New project asks for the settings that matter before creating. | `v1.2.0:apps/desktop/e2e/project-settings.spec.ts:L57` |
| A328 | Acceptance project-settings: Settings opens once, whichever entry point asks for it. | `v1.2.0:apps/desktop/e2e/project-settings.spec.ts:L132` |

### W010 — Resolve every registered shortcut and preserve text ownership

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Each binding’s target context plus an input, nested contenteditable, selected read-only text, button and sandbox panel.
**Steps:** For every K row in w1-keymap.md press the exact chord once, repeat it, add modifiers, then repeat in fields; add a higher-priority extension binding that declines and one that handles.
**Expected:** Exactly one correct command/argument dispatch; priority, exact/loose match and repeat flags apply; text, native menu and button defaults retain ownership. Unknown commands decline.
**Exact registered keymap entries:** K194 escape → blurField.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/keychord.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`.
**Existing shipped e2e:** `agent-composer-shortcuts.spec.ts`; `button-pointer-focus.spec.ts`; `media-keyboard.spec.ts`; `pro-editor-shortcuts.spec.ts`.
**Later candidate e2e (not shipped evidence):** `ae-workflow.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A246 | Arrow nudges: Bare arrows move selection 1 unit; Shift+arrows move 10 units; both repeat while held. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A247 | Primary frame keys: Cmd+Left/Right steps frames; Ctrl+Left/Right selects adjacent keyframe; Cmd/Ctrl+Shift+Left/Right steps ±10 frames. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A248 | Property reveal: P/S/R/T/A/U/M/F/E/L run timeline.revealProperty:<key>; Shift variant adds to revealed properties. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A249 | Field exceptions: Save, Save As, Settings and context Paste work in fields; Escape in a field runs blurField at priority 50 without editor deselection. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A250 | Held-key policy: Only nextFrame, prevFrame, stepFrames, nudgeSelection and nudgeKeyframes explicitly repeat in this keymap. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A251 | Loose modifier policy: Legacy MOD_CHORDS selectively accept extra modifiers; export/new-project/new-composition are exact, as are PRO_CHORDS. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A252 | Keymap customization: Built-in keymap is replaceable as an extension; runtime bind/unbind/list expose command, args, priority, inFields, repeat and looseModifiers. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A253 | Shortcut dispatch: Normalize chords and arbitrate registered bindings with focus/field/repeat policy rather than a second native shortcut executor. | `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/keychord.ts` |
| A254 | Custom shortcut settings: No dedicated user keybinding editor is established by SettingsScreen; extension API is the evidenced customization surface. | `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/keychord.ts` |
| B513 | Primary modifier: Cmd shortcuts have Ctrl equivalents; exact file chords reject extra modifiers. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B532 | Escape ownership: Escape in field blurs without deselecting; otherwise Escape deselects. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B540 | pro-editor-shortcuts contract: Command+Shift+D splits a group into two group strips. | `v1.2.0:apps/desktop/e2e/pro-editor-shortcuts.spec.ts:L24` |
| B541 | pro-editor-shortcuts contract: Professional shortcuts split, cut, paste, nudge, and respect focused fields. | `v1.2.0:apps/desktop/e2e/pro-editor-shortcuts.spec.ts:L54` |
| B542 | pro-editor-shortcuts contract: Space toggles playback while the pointer is over the viewer but stays local in text fields. | `v1.2.0:apps/desktop/e2e/pro-editor-shortcuts.spec.ts:L234` |
| B543 | pro-editor-shortcuts contract: After Effects timeline keys navigate, retime layers, and move selected keyframes. | `v1.2.0:apps/desktop/e2e/pro-editor-shortcuts.spec.ts:L270` |

### W011 — Reach editor commands through native and contextual menus

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A focused editor, text field, selection, locked selection and contributed menu item.
**Steps:** Invoke every shipped application-menu row and each contextual action with eligible and ineligible targets; delay an extension contribution.
**Expected:** Correct focused target and disabled state; no duplicate keyboard execution; asynchronous contributions keep the clicked target. Menu spelling/layout may become native.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/menu.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/menu.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts`, `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/main/text-context-menu.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte`.
**Existing shipped e2e:** `menu-save.spec.ts`; `new-composition-menu.spec.ts`; `user-feedback.spec.ts`.
**Later candidate e2e (not shipped evidence):** `ae-workflow.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A057 | View › Zoom In: CommandOrControl+=; dispatch &#96;zoomIn&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A058 | View › Zoom Out: CommandOrControl+-; dispatch &#96;zoomOut&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A059 | View › Actual Size: CommandOrControl+1; dispatch &#96;actualSize&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A060 | View › Fit Composition: CommandOrControl+0; dispatch &#96;fitComposition&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A061 | View › Show/Hide Layer Controls: CommandOrControl+Shift+H; dispatch &#96;toggleLayerControls&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A062 | Powermove › Settings…: CommandOrControl+,; dispatch &#96;settings&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A063 | File › New Project: CommandOrControl+Alt+N; dispatch &#96;newProject&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A064 | File › Open Project…: CommandOrControl+O; dispatch &#96;open&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A065 | File › Close Tab: CommandOrControl+W; dispatch &#96;closeTab&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A066 | File › Save Project: CommandOrControl+S; dispatch &#96;save&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A067 | File › Save Project As…: CommandOrControl+Shift+S; dispatch &#96;saveAs&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A068 | File › Import Media…: CommandOrControl+I; dispatch &#96;import&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A069 | File › Export…: CommandOrControl+E; dispatch &#96;export&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A070 | Edit › Undo: CommandOrControl+Z; dispatch &#96;contextUndo&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A071 | Edit › Redo: CommandOrControl+Shift+Z; dispatch &#96;contextRedo&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A072 | Edit › Cut: CommandOrControl+X; dispatch &#96;contextCut&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A073 | Edit › Copy: CommandOrControl+C; dispatch &#96;contextCopy&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A074 | Edit › Paste: CommandOrControl+V; dispatch &#96;contextPaste&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A075 | Edit › Select All: CommandOrControl+A; dispatch &#96;contextSelectAll&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A076 | Edit › Duplicate Layers: CommandOrControl+D; dispatch &#96;duplicate&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A077 | Edit › Split at Playhead: CommandOrControl+Shift+D; dispatch &#96;split&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A078 | Edit › Bring Forward: CommandOrControl+]; dispatch &#96;bringForward&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A079 | Edit › Send Backward: CommandOrControl+[; dispatch &#96;sendBackward&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A080 | Edit › Bring to Front: CommandOrControl+Shift+]; dispatch &#96;bringToFront&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A081 | Edit › Send to Back: CommandOrControl+Shift+[; dispatch &#96;sendToBack&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A082 | Composition › New Composition…: CommandOrControl+N; dispatch &#96;newComposition&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A083 | Composition › Composition Settings…: CommandOrControl+Alt+K; dispatch &#96;compositionSettings&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A084 | Pre-compose is in the Composition menu at the tag; later main adds Layer menu. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A085 | Window › Show Next Tab: Control+Tab; dispatch &#96;nextTab&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A086 | Window › Show Previous Tab: Control+Shift+Tab; dispatch &#96;previousTab&#96; to the editor (context* commands target the focused window); template has no per-document enabled predicate. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A087 | Native About role. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A088 | Powermove › Services: system role; system Services submenu. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A089 | Powermove › Hide: ⌘H; native hide role. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A090 | Powermove › Hide Others: ⌥⌘H; native hideOthers role. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A091 | No explicit Show All/unhide row exists in tag menu; do not invent a shipped entry. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A092 | Command-Q uses native quit and existing close preparation. | `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/menu.ts` |
| A093 | File › New Window: ⇧⌘N; enabled only when window actions exist. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A094 | File › Close Window: ⇧⌘W; enabled only when window actions exist. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A095 | File › Import Image Sequence…: none; dispatch importSequence. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A096 | File › Import Folder…: none; dispatch importFolder. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A106 | View › Toggle Full Screen: system role; native togglefullscreen role. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A107 | View › Reload / Toggle Developer Tools: system roles; development builds only. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A108 | Window › Minimize / Zoom: system roles; native minimize and zoom roles. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A109 | Window › Move Tab to New Window: none; enabled only when window actions exist. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A110 | Window › Bring All to Front: system role; native front role. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A112 | Menu routing: Do not let displayed native equivalents preempt renderer-owned field and repeat policy; gate matching Cmd/Ctrl chords before native dispatch. | `v1.2.0:apps/desktop/src/main/menu.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts` |
| A113 | Command availability: Native command items generally remain enabled in the template; actual selection/project/field availability is enforced by renderer command handling. | `v1.2.0:apps/desktop/src/main/menu.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts` |
| A114 | Tag top-level menu order is Powermove, File, Edit, Composition, View, Window. Layer and Help are later. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A115 | Shipped panel menu offers Pop out to window, Close panel except viewer, and extension contributions; maximize, add-as-tab and close-others are later. | `v1.2.0:apps/desktop/src/renderer/src/layout/menu.ts` |
| A117 | Panel menu extensions: Gather panel:context asynchronously with panelId/dockId and preserve clicked target through delayed results. | `v1.2.0:apps/desktop/src/renderer/src/layout/menu.ts` |
| A120 | Recent project context: Open, Open in New Window, Save, Save As…, Rename…, Duplicate, Move to Trash…; current project Open is disabled. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts` |
| A121 | Recent window eligibility: Show Open in New Window only with multiwindow support and disable moving the sole active tab. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts` |
| A122 | Trashed project context: Restore and Delete Forever… replace the normal project actions. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts` |
| A123 | Media showProjectMenu: Context menu contains New Composition…, New Folder, Import Media…, Import Folder…; action predicates and optional items follow the clicked asset/folder/composition. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A124 | Media showFolderMenu: Context menu contains Open, Rename, New Folder Inside, Import Folder Into…, Move Out of Folder, Delete Folder; action predicates and optional items follow the clicked asset/folder/composition. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A125 | Media showCompMenu: Context menu contains Open in Timeline, Add to Timeline, Composition Settings…, Duplicate, Delete Composition…; action predicates and optional items follow the clicked asset/folder/composition. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A126 | Media showAssetMenu: Context menu contains Delete Selected Media…, Download from Cloud, Add to Timeline, New Composition from Media, Reveal in Finder, Move Out of Folder, Delete Media…; action predicates and optional items follow the clicked asset/folder/composition. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A127 | Text selection menu: Non-password selections get Look Up on macOS and Search with Google; noneditable selected text gets Copy only after those services. | `v1.2.0:apps/desktop/src/main/text-context-menu.ts` |
| A128 | Editable text menu: Undo, Redo, Cut, Copy, Paste, Select All use native editFlags; composer Undo/Redo use attachment-preserving custom history. | `v1.2.0:apps/desktop/src/main/text-context-menu.ts` |
| A130 | Native text services: Open menu with the originating services frame for macOS Writing Tools, Services and Look Up; never route selection copying to layers. | `v1.2.0:apps/desktop/src/main/text-context-menu.ts` |
| A131 | Media asset optional actions: Replace Media…/Relink Missing Media… uses replacement picker; include dynamic Play/Pause preview labels and cloud state actions. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A132 | Layer context scope: Right-click layer in viewer/timeline uses shared selected-layer menu; mutating rows disable for protected/locked selection. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A133 | Duplicate, Group layers, Ungroup layers for groups, Pre-compose exist; Rename and Release Pre-comp rows are later. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A134 | Layer text and mask menus: All-text selection offers Split Text into Layers; valid multiple selection offers Use Bottom Layer as Mask; Move to Group lists Outside Groups and eligible groups. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A135 | Parent… menu offers None and valid noncyclic candidates; main renames it Link Layers. This is transform parenting, not linked audio/video. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A136 | Layer timing menu: Timing submenu offers Split at Playhead ⇧⌘D, Trim In/Out to Playhead (disabled outside relevant span), Fit to Composition. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A137 | Mute/separate video audio, hide/solo/lock and 3D layer exist; dedicated Continuously Rasterize menu row is later. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A138 | Layer deletion menu: Delete ⌫ disables when selection cannot be edited; preserve extension layer:context contributions. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/layer-menu.ts` |
| A139 | Viewer empty context: Paste, Select All Layers (disabled with no layers), Fit Composition in View, Composition Settings… plus viewer:context contributions. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| A141 | Timeline graph context: Value Graph/Speed Graph checks current mode; Fit Selected Curves disabled without graph; Merge Selected Curves requires at least two series; Show Layer Bars with Graph toggles combined view. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| A142 | Timeline key context: Edit Value…, Keyframe Velocity…, Scale Keyframes…, easing presets, Hold Keyframe(s), Copy Keyframe(s), Delete Keyframe(s). | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| A143 | Timeline track context: Add Keyframe Here, Paste Keyframes Here, Clear All Keyframes (disabled for empty track), Separate Dimensions with checked state. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| A144 | Timeline background/ruler context: Remove Marker on marker; Add Marker Here disabled if marker exists; Start/End Work Area at context position, Reset Work Area, Switch to Layer/Track Timeline. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| A145 | Easing curve context: Save Preset…, Mirror Curve, Reset Curve, Input Coordinates toggle; library background offers Import Presets…, Export Library…, Panel Settings. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| A146 | Easing action context: Copy/Paste/Apply controls expose target-section menus; preset and group context menus support library organization. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B018 | Tag empty-viewer context offers Paste and Select all layers; Fit Composition and Composition Settings rows are later. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |

### W012 — Find and change app settings

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Home and editor profiles, live extension settings and an existing focused field.
**Steps:** Open Settings from each entry point; search; navigate sections with keyboard and wheel; change restoration, preview memory and cloud-download preferences; close and reopen.
**Expected:** One Settings surface, correct visible sections, live search results, persisted preferences, restored focus and responsive reduced-motion navigation.
**Exact registered keymap entries:** K005 cmd+, → settings; K006 ctrl+, → settings.
**Native menu key equivalents:** M006 CommandOrControl+, → settings.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `design-language.spec.ts`; `project-settings.spec.ts`; `settings-motion.spec.ts`; `settings-scroll.spec.ts`.
**Later candidate e2e (not shipped evidence):** `memory-settings.spec.ts`; `storage-settings.spec.ts`; `storage-upgrade.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A149 | Cmd/Ctrl+,: Run &#96;settings&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A257 | Settings local keys: Up/Down/Home/End navigate visible sidebar sections; Escape or exact Cmd/Ctrl+, dismisses; Settings stops editor key propagation. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A268 | Tag sections: Project when open; General, Accounts, Transcription and Extensions. Memory and Storage sections are later. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A269 | Settings entry: Open one shared Settings screen from any entry point; default to Project when available, otherwise General. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A270 | Settings search: Filter live settings across sections and extension details, highlight matches and announce count/no matches. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A271 | Settings navigation: Continuous scrolling tracks active section; arrow keys/Home/End navigate sidebar; Done, Escape or Cmd/Ctrl+, close and restore focus. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A272 | General appearance: Choose Match system (default), Light or Dark. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A273 | General workspace: Bring my workspace closes Settings and hands After Effects workspace transfer to agent; failures remain retryable. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A274 | General playback: Preview memory choices Off, 128 MB, 256 MB, 512 MB, 1 GB, 2 GB; default 256 MB persisted per profile. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A275 | General media: Automatically download cloud media defaults off and persists. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A276 | General windows: Reopen last project on launch defaults on and appears only with multiwindow support. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A333 | Acceptance settings-motion: Settings section travel settles quickly and manual wheel input owns scrolling. | `v1.2.0:apps/desktop/e2e/settings-motion.spec.ts:L3` |
| A334 | Acceptance settings-motion: Settings retargeting, dismissal, trackpad deltas, and reduced motion remain responsive. | `v1.2.0:apps/desktop/e2e/settings-motion.spec.ts:L55` |
| A335 | Acceptance settings-scroll: Settings is a continuous page with section navigation and highlighted search. | `v1.2.0:apps/desktop/e2e/settings-scroll.spec.ts:L3` |
| A336 | Acceptance settings-scroll: Settings search follows live extensions into their details and works without a project. | `v1.2.0:apps/desktop/e2e/settings-scroll.spec.ts:L61` |

### W013 — Complete first-run setup and inspect About

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Fresh profile, available transcription downloads and optional connected provider.
**Steps:** Complete or skip workspace, agent, timeline and transcription setup; continue while a model download runs; open About.
**Expected:** Chosen timeline starts, background model download completes, setup remains retryable and required license notices are retained.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/onboarding/welcome.ts`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TimelineStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/AgentStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/WorkspaceStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TranscriptionStep.svelte`, `v1.2.0:apps/desktop/src/main/menu.ts`.
**Existing shipped e2e:** `animation-cleanup.spec.ts`; `onboarding-timeline-mode.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A285 | Onboarding: Gate first launch before normal startup; introduce workspace transfer, agent connection, timeline preference and transcription setup. | `v1.2.0:apps/desktop/src/renderer/src/onboarding/welcome.ts`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/AgentStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TimelineStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/WorkspaceStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TranscriptionStep.svelte` |
| A286 | Timeline onboarding: Allow choosing track timeline before entering editor. | `v1.2.0:apps/desktop/src/renderer/src/onboarding/welcome.ts`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/AgentStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TimelineStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/WorkspaceStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TranscriptionStep.svelte` |
| A287 | Onboarding model download: Start selected transcription model download and continue it after onboarding closes. | `v1.2.0:apps/desktop/src/renderer/src/onboarding/welcome.ts`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/AgentStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TimelineStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/WorkspaceStep.svelte`, `v1.2.0:apps/desktop/src/renderer/src/onboarding/TranscriptionStep.svelte` |
| A288 | Keep native About metadata and shipped font/icon license notices; do not assume later Inter font was bundled. | `v1.2.0:apps/desktop/src/main/menu.ts` |
| A294 | Acceptance onboarding-timeline-mode: Onboarding lets a new user choose the track timeline before opening the editor. | `v1.2.0:apps/desktop/e2e/onboarding-timeline-mode.spec.ts:L7` |

### W014 — Arrange, resize, hide and restore panels/workspaces

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Several registered panels, saved workspaces and one playing composition.
**Steps:** Drag panels between docks; resize splitters; cancel a drag; close/restore; apply/save workspace; relaunch and reopen .pmv.
**Expected:** Document and live hosts survive; minimum usable areas; explicit visibility and per-project layout persist. Shipped dock arrangement is the reference, not later panel-tab/maximize additions.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts`.
**Existing shipped e2e:** `area-corners.spec.ts`; `design-language.spec.ts`; `dock-layout.spec.ts`; `interface-cleanup.spec.ts`; `panel-visibility.spec.ts`; `project-panel-persistence.spec.ts`; `splitter-pairs.spec.ts`; `viewer-panel-drag.spec.ts`; `workspace-import.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A313 | Acceptance project-panel-persistence: Restores each project layout after quit and when reopening its saved file. | `v1.2.0:apps/desktop/e2e/project-panel-persistence.spec.ts:L10` |
| A364 | Dock model: Arrange panel groups across left/center/right/bottom with viewer center and resizable split areas. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A365 | Dock gestures: Drag panel headers/grips into edge splits or tab groups with drop target/ghost preview; Escape cancels. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A367 | Split constraints: Resize linked splitter pairs respecting minimums; overfilled side panels remain usable between titlebar/status bar. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A369 | Mount continuity: Retain panel/viewer hosts during layout/workspace moves so media and panel state are not reset. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A370 | Panel visibility: Remember explicit close/hide across restart and workspace changes; restore through registry/layout APIs. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A373 | Workspace library: Browse All workspaces, Built in and Saved by you; apply presets and save customized layouts as new workspaces. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts` |
| A374 | Panel library: Browse All panels and categorized shelves; open a live panel from its preview and return without replacing its document. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts` |
| A375 | Workspace persistence: Store each project's layout through file saves and app relaunch; retain custom workspace and hidden-panel state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts` |
| A378 | Acceptance dock-layout: Keeps overfilled side panels usable between the titlebar and status bar. | `v1.2.0:apps/desktop/e2e/dock-layout.spec.ts:L4` |
| A379 | Acceptance dock-layout: Keeps panel and viewer hosts alive across layout and workspace moves. | `v1.2.0:apps/desktop/e2e/dock-layout.spec.ts:L121` |
| A386 | Acceptance panel-visibility: Keeps a closed panel hidden across relaunch and workspace changes. | `v1.2.0:apps/desktop/e2e/panel-visibility.spec.ts:L5` |
| B065 | viewer-panel-drag contract: Moves the Preview canvas through the shared panel drag affordance. | `v1.2.0:apps/desktop/e2e/viewer-panel-drag.spec.ts:L3` |

### W015 — Edit in a popped-out panel and return it

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A popout-eligible panel with unsaved edits.
**Steps:** Open its native window; use controls; close it; try an unauthorized window target.
**Expected:** Same project, panel state and edits on return; unauthorized popups refused.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/layout/popout.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte`.
**Existing shipped e2e:** `panel-popout.spec.ts`.
**Later candidate e2e (not shipped evidence):** `consumer-matrix.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A371 | Floating panels: Eligible panels open in native windows, retain edits, and return to layout when closed. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A372 | Popout security: Only allow named editor-owned panel windows, not arbitrary window.open targets. | `v1.2.0:apps/desktop/src/renderer/src/layout/model.ts`, `v1.2.0:apps/desktop/src/renderer/src/layout/Dock.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A384 | Acceptance panel-popout: A panel opens in a native window, keeps edits, and returns to the layout. | `v1.2.0:apps/desktop/e2e/panel-popout.spec.ts:L3` |
| A385 | Acceptance panel-popout: Only named panel windows from the editor are allowed. | `v1.2.0:apps/desktop/e2e/panel-popout.spec.ts:L42` |

### W016 — Browse workspaces and live panel previews

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Many registered panels, including tall scrollable ones.
**Steps:** Open Library; scroll shelves; open a panel preview; return rapidly; reopen; close during queued preview work.
**Expected:** Bounded lazy preview work, live panel/document continuity, fitted previews and no orphan overlays.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts`.
**Existing shipped e2e:** `animation-cleanup.spec.ts`; `comp-thumbnails.spec.ts`; `library-performance.spec.ts`; `library-preview-fit.spec.ts`; `library-transitions.spec.ts`.
**Later candidate e2e (not shipped evidence):** `consumer-matrix.spec.ts`; `consumer-surfaces.spec.ts`; `library-card-parity.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A376 | Preview scheduling: Render nearby library previews, lazy-load while scrolling, refresh cached previews on reopen and cancel queued work on close. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts` |
| A377 | Panel previews: Contain tall scrollable panel content without changing the live panel's geometry or scroll state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/library.ts`, `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/layout/install.ts` |
| A380 | Acceptance library-preview-fit: Library previews contain tall panel content without changing the live panel. | `v1.2.0:apps/desktop/e2e/library-preview-fit.spec.ts:L5` |
| A381 | Acceptance library-preview-fit: Built-in Library previews fit their scrollable content. | `v1.2.0:apps/desktop/e2e/library-preview-fit.spec.ts:L37` |
| A382 | Acceptance library-transitions: Opening and returning from a panel keeps the shelves and restores its live panel. | `v1.2.0:apps/desktop/e2e/library-transitions.spec.ts:L5` |
| A383 | Acceptance library-transitions: Quick Back and reopen leave one editor and no abandoned flying cover. | `v1.2.0:apps/desktop/e2e/library-transitions.spec.ts:L53` |

### W017 — Use command search, modal dialogs, menus and notices

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Commands/providers, a focused field and an asynchronous action that can fail.
**Steps:** Search and execute a palette command; navigate menu rows; open modal; cycle focus; fail/retry action; dismiss toasts with mouse and keyboard.
**Expected:** Eligibility and focus restoration, readable actionable errors, no accidental project mutation, independent toast action/close state.
**Exact registered keymap entries:** K001 cmd+k → palette; K002 ctrl+k → palette.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Menu.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Palette.svelte`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/modal.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/inert.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/palette-model.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte`.
**Existing shipped e2e:** `color-picker-overlay.spec.ts`; `error-presentation.spec.ts`; `overlays.spec.ts`; `select-menu-layout.spec.ts`; `toast-error.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A147 | Cmd/Ctrl+k: Run &#96;palette&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A391 | Modal focus: Trap modal focus, make covered content inert, support dismissal and restore previous focus. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/modal.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/inert.ts` |
| A392 | Async actions: Keep dialog input and error visible if an asynchronous action fails; allow retry. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/modal.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/inert.ts` |
| A393 | Confirmations: Use native confirmation bridge where available with cancel-safe results; guard stale project-bound dialogs. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/modal.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/inert.ts` |
| A394 | Modal API: Extensions may provide title, DOM/string body, width and primary/nonprimary action buttons with close handle. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/modal.ts`, `v1.2.0:apps/desktop/src/renderer/src/overlays/inert.ts` |
| A395 | Tag menus support anchors/coordinates, separators, headers, checks, disabled rows and async contributions; nested menus use explicit follow-up menus. Structured submenu API is later. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Menu.svelte`, `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A396 | Tag menu fits a single option near lower viewport without a spurious scrollbar; menu-rules.ts is a later split. | `v1.2.0:apps/desktop/e2e/select-menu-layout.spec.ts` |
| A397 | Palette searches commands/providers, displays categories/shortcuts and keyboard-runs eligible results. | `v1.2.0:apps/desktop/src/renderer/src/overlays/palette-model.ts` |
| A398 | Async palette providers and command availability exist; later main adds AE terminology matching. | `v1.2.0:apps/desktop/src/renderer/src/overlays/palette-model.ts` |
| A399 | Toasts: Stack status/error notices, timed or persistent, with optional actions and independent trailing close control. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte` |
| A400 | Toast keyboard: Allow keyboard dismissal; action availability is independent of close-button disabled state. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte` |
| A401 | Progress tasks: Show persistent import/download/storage work with cancel and terminal success/failure cleanup. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte` |
| A402 | Project loading: Show theme-aware project/panel ghost only in editor while loading; never bleed it through Projects. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte` |
| A403 | Tooltips: Retain action titles, keyboard hints and accessible labels across shell and extension controls. | `v1.2.0:apps/desktop/src/renderer/src/overlays/Toasts.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/ProjectGhost.svelte` |
| A404 | Acceptance error-presentation: Agent errors and app failures remain readable, inspectable and dismissible. | `v1.2.0:apps/desktop/e2e/error-presentation.spec.ts:L19` |
| A405 | Acceptance error-presentation: Failed asynchronous dialog actions keep the input and allow another attempt. | `v1.2.0:apps/desktop/e2e/error-presentation.spec.ts:L66` |
| A406 | Acceptance error-presentation: Sending a message leaves the saved Takes archive untouched and keeps Undo available. | `v1.2.0:apps/desktop/e2e/error-presentation.spec.ts:L87` |
| A407 | Acceptance overlays: Runs a palette command and manages modal focus and toast lifetime. | `v1.2.0:apps/desktop/e2e/overlays.spec.ts:L6` |
| A408 | Acceptance select-menu-layout: A single-option menu fits without a scrollbar near the bottom edge. | `v1.2.0:apps/desktop/e2e/select-menu-layout.spec.ts:L3` |
| A409 | Acceptance toast-error: Notification close buttons stay at the trailing edge and dismiss by mouse or keyboard. | `v1.2.0:apps/desktop/e2e/toast-error.spec.ts:L3` |
| A410 | Acceptance toast-error: Notification actions remain usable when the separate close button is disabled. | `v1.2.0:apps/desktop/e2e/toast-error.spec.ts:L61` |
| A411 | Acceptance toast-error: The error toast matches the status toast it stacks with. | `v1.2.0:apps/desktop/e2e/toast-error.spec.ts:L85` |

### W018 — Select and organize project media

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A long asset list, nested folders, referenced and locked media.
**Steps:** Use arrows, Home/End, ranges, marquee/autoscroll; create/rename/move/delete folders; move a batch; delete referenced assets; switch project while confirmation is open; undo.
**Expected:** Stable selection, one batch Undo, no folder cycles or disk-file deletion, no stale dialog writes into another project.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`.
**Existing shipped e2e:** `interface-polish.spec.ts`; `media-bulk-selection.spec.ts`; `media-deletion-project.spec.ts`; `media-folder-drag.spec.ts`; `media-keyboard.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A259 | Media list local keys: Cmd/Ctrl+A selects media, arrows/Home/End navigate, Enter adds, Space previews, Delete/Backspace deletes, Escape clears; folder Enter opens and rename Enter/Escape commits/cancels. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A412 | Media browser: List compositions, nested media folders and assets with previews, status, search/filter and selection. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A413 | Media row keys: Arrow keys wrap selection; Home/End jump; Enter adds, Space previews, Delete/Backspace deletes, Escape clears; nested buttons keep Enter/Space. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A414 | Folder keys: Enter opens, Delete/Backspace removes folder, Escape clears; inline rename commits Enter and cancels Escape. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A415 | Media selection: Support Cmd-click toggles, Shift ranges, marquee selection with autoscroll and batch actions. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A416 | Media folders: Create/rename/move/delete folders; deleting moves contents up and never deletes original disk files. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A417 | Folder drops: Move selected media as a batch with one Undo; reject folder cycles; external folder drops import into target. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A418 | Option folder drop: Option keeps individual files in a folder rather than treating the drop as an image sequence. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A431 | Referenced deletion: Confirm once for batch deletion, block locked/unsupported references, provide one Undo and reject stale dialogs after project switch. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A432 | Reveal original: Media library reveals the original source file in Finder. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A454 | Acceptance media-bulk-selection: Drag a selection box, delete media, and undo the whole batch in both themes. | `v1.2.0:apps/desktop/e2e/media-bulk-selection.spec.ts:L20` |
| A455 | Acceptance media-bulk-selection: Command-click and Shift-click select media; referenced bulk deletion confirms once. | `v1.2.0:apps/desktop/e2e/media-bulk-selection.spec.ts:L50` |
| A456 | Acceptance media-bulk-selection: A selection box scrolls through long media lists and Escape clears selection. | `v1.2.0:apps/desktop/e2e/media-bulk-selection.spec.ts:L79` |
| A457 | Acceptance media-deletion-project: A stale media deletion dialog cannot delete media after switching projects. | `v1.2.0:apps/desktop/e2e/media-deletion-project.spec.ts:L3` |
| A458 | Acceptance media-folder-drag: Dragging selected media into a folder moves the batch and one Undo restores it. | `v1.2.0:apps/desktop/e2e/media-folder-drag.spec.ts:L3` |
| A464 | Acceptance media-keyboard: Media action buttons retain native Enter and Space activation. | `v1.2.0:apps/desktop/e2e/media-keyboard.spec.ts:L3` |

### W019 — Import durable media and cancel safely

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Supported videos/audio/stills/animated images/models, and unreadable files.
**Steps:** Import through picker, drop, Finder paste and extension URL; place on timeline; cancel during native work; switch project mid-import; remove original and relaunch.
**Expected:** Metadata, alpha and media bytes survive; progress terminates; cancel/failure is retryable; late results never enter another project.
**Exact registered keymap entries:** K037 cmd+i → import; K038 ctrl+i → import; K039 cmd+shift+i → import; K040 ctrl+shift+i → import.
**Native menu key equivalents:** M012 CommandOrControl+I → import.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts`, `v1.2.0:apps/desktop/src/main/media-proxy.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `extension-remote-media-ownership.spec.ts`; `import-animated-image.spec.ts`; `import-cloud.spec.ts`; `import-converted-formats.spec.ts`; `import-folder.spec.ts`; `import-mp4.spec.ts`; `import-progress.spec.ts`; `import-prores-alpha.spec.ts`; `import-prores.spec.ts`; `import-svg.spec.ts`; `media-paste.spec.ts`; `open-media.spec.ts`.
**Later candidate e2e (not shipped evidence):** `import-graded-footage.spec.ts`; `media-import-readiness.spec.ts`; `media-store-long-task.spec.ts`; `media-store.spec.ts`; `native-linked-rendition.spec.ts`; `native-media-sequence-parity.spec.ts`; `native-media-sequences.spec.ts`; `native-still-icc-parity.spec.ts`; `native-still-orientation-parity.spec.ts`; `native-svg-broken.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A165 | Cmd/Ctrl+i: Run &#96;import&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A166 | Cmd/Ctrl+shift+i: Run &#96;import&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A420 | Picker imports: Native media, folder and image-sequence pickers share durable importer and project-bound progress. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A422 | URL media: Extension assets.importUrl requires assets+network permissions and must remain bound to originating project. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A423 | Finder paste: Import copied Finder media and clipboard image bytes at playhead with timeline Undo/Redo. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A428 | Import cancellation: Keep Cancel visible until native work settles, clear task, allow another batch and prevent late imports landing in another project. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A429 | Durability: Imported playable media survives removal of source and app restart; converted proxies remain durable. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A430 | Supported import families: Handle video/audio, still and animated images, editable SVG, image sequences and registered model importers; preserve alpha and source metadata. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A434 | Acceptance import-animated-image: A still image in an animatable container still imports as an image. | `v1.2.0:apps/desktop/e2e/import-animated-image.spec.ts:L76` |
| A436 | Acceptance import-converted-formats: A Matroska video converts on import and plays both of its colours. | `v1.2.0:apps/desktop/e2e/import-converted-formats.spec.ts:L50` |
| A441 | Acceptance import-mp4: Keeps imported video playable after the source is removed and the app closes. | `v1.2.0:apps/desktop/e2e/import-mp4.spec.ts:L17` |
| A442 | Acceptance import-mp4: Imports H.264/AAC media and renders its green frame. | `v1.2.0:apps/desktop/e2e/import-mp4.spec.ts:L66` |
| A443 | Acceptance import-mp4: Plays an imported soundtrack and separates it from the timeline context menu. | `v1.2.0:apps/desktop/e2e/import-mp4.spec.ts:L117` |
| A444 | Acceptance import-mp4: Mutes a video soundtrack from the timeline and the inspector. | `v1.2.0:apps/desktop/e2e/import-mp4.spec.ts:L170` |
| A445 | Acceptance import-progress: Single-file imports show persistent progress through storage and clear it on success or failure. | `v1.2.0:apps/desktop/e2e/import-progress.spec.ts:L3` |
| A446 | Acceptance import-progress: Switching projects during import removes progress without adding media to the new project. | `v1.2.0:apps/desktop/e2e/import-progress.spec.ts:L35` |
| A447 | Acceptance import-progress: A successful import reads as success even when the file is named like a failure. | `v1.2.0:apps/desktop/e2e/import-progress.spec.ts:L60` |
| A448 | Acceptance import-progress: Cancel stays visible while native work settles, then allows the next batch. | `v1.2.0:apps/desktop/e2e/import-progress.spec.ts:L78` |
| A449 | Acceptance import-prores-alpha: Imports ProRes 4444 with transparency and restores playable media. | `v1.2.0:apps/desktop/e2e/import-prores-alpha.spec.ts:L9` |
| A450 | Acceptance import-prores: Converts ProRes once, presents video frames, and restores the durable proxy. | `v1.2.0:apps/desktop/e2e/import-prores.spec.ts:L8` |
| A465 | Acceptance media-paste: Pastes a clipboard image at the playhead and supports undo and redo. | `v1.2.0:apps/desktop/e2e/media-paste.spec.ts:L40` |
| A702 | Acceptance extension-remote-media-ownership: A delayed extension URL import cannot land in a different project. | `v1.2.0:apps/desktop/e2e/extension-remote-media-ownership.spec.ts:L7` |
| B019 | Drop media: place media through viewer drop interaction using composition coordinates. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B144 | File drops: timeline import uses drop time/index; Option imports as folder. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |

### W020 — Import, replace and rate-convert an image sequence

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Numbered transparent frames with a gap plus an existing sequence.
**Steps:** Choose sequence rate; inspect gap warning; cancel or import individual images; import as sequence; replace missing frames; save/reopen/export.
**Expected:** Correct ordered frames at selected rate, transparency and insertion time; cancelled replacement preserves old source.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/image-sequence.ts`.
**Existing shipped e2e:** `import-image-sequence.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A421 | Image sequences: Recognize numbered frames, choose frame rate, warn about gaps and offer still import/cancel; reimport replaces missing frames. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A439 | Acceptance import-image-sequence: Sequence import warns about gaps and permits importing individual stills or cancelling. | `v1.2.0:apps/desktop/e2e/import-image-sequence.spec.ts:L117` |
| A440 | Acceptance import-image-sequence: Reimport replaces missing frames, preserves the frame rate and insertion time, and allows cancellation. | `v1.2.0:apps/desktop/e2e/import-image-sequence.spec.ts:L135` |
| B431 | Sequence confirmation: detect sequence and offer Import sequence, Cancel and optional Individual images. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/image-sequence.ts` |
| B432 | Sequence progress: show checking frames, creating sequence and loading sequence progress with counts. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/image-sequence.ts` |
| B433 | Sequence ownership: release playback proxy on completion/failure; cancellation does not partially replace source. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/image-sequence.ts` |
| B446 | import-image-sequence contract: Sequence import warns about gaps and permits importing individual stills or cancelling. | `v1.2.0:apps/desktop/e2e/import-image-sequence.spec.ts:L117` |
| B447 | import-image-sequence contract: Reimport replaces missing frames, preserves the frame rate and insertion time, and allows cancellation. | `v1.2.0:apps/desktop/e2e/import-image-sequence.spec.ts:L135` |

### W021 — Preview source media without altering the timeline

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Available image, audio and video assets.
**Steps:** Double-click/Space from Media; scrub source; use source Left/Right, Home/End; close with Escape; switch asset.
**Expected:** Source is named and fitted; stills hide transport; audio/video use source time; composition pauses once and remains unedited. No source range insert/overwrite UI ships.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`.
**Existing shipped e2e:** `video-instance-preview.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-source-compositor.spec.ts`; `engine-source-guard.spec.ts`; `engine-source-media.spec.ts`; `engine-source-performance.spec.ts`; `engine-source-refresh.spec.ts`; `engine-source-timing-end.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A419 | Source preview: Double-click available assets previews them; offline double-click downloads cloud assets or opens replacement picker. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| B090 | source-preview contract: Names the source and offers a way back so the stage never reads as a broken composition. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L47` |
| B091 | source-preview contract: Shows the audio filename only once below the audio icon. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L69` |
| B092 | source-preview contract: Hides the transport for a still image, which has nothing to play. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L89` |
| B093 | source-preview contract: Seeks with the keyboard and keeps playback time accessible without visible timestamps. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L107` |
| B094 | source-preview contract: Pauses the composition once instead of restarting it behind the monitor. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L128` |
| B095 | source-preview contract: Replaces only the preview controller during a hot update. | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.test.ts:L158` |

### W022 — Recover offline sources or replace media in place

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Saved v2 fingerprints; absent cache; original moved, renamed, changed and missing; a referencing animated clip.
**Steps:** Reopen offline; recover from original; choose replacement/relink; try incompatible/unreadable/cancelled candidates; undo/redo and save/reopen.
**Expected:** IDs, references, folder, timing and animation survive. Automatic identity matching uses v2 sampled SHA-256; explicit Replace can intentionally change content and is not identity relink.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/project-file.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`.
**Existing shipped e2e:** `local-media-recovery.spec.ts`; `media-replacement.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-delivery.spec.ts`; `linked-media-storage.spec.ts`; `media-store.spec.ts`; `native-linked-rendition.spec.ts`; `native-offline-media.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A424 | Media replacement: Replace/relink retains asset ID, referencing layers, timing, animation and folder; validate same media kind and preserve original on failure. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A425 | Offline sources: Differentiate missing from changed source files; recover from original local source when cached bytes disappear. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A426 | Keep original sourcePath/fingerprint for missing-cache recovery and portable media saves. A project-wide link-in-place storage mode is later. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts` |
| A453 | Acceptance local-media-recovery: Distinguishes changed source files from missing files and clears errors after recovery. | `v1.2.0:apps/desktop/e2e/local-media-recovery.spec.ts:L40` |
| A466 | Acceptance media-replacement: Replacement uses the import picker, progress, and image sequence flow. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L9` |
| A467 | Acceptance media-replacement: Replacement picker remains bound to its original project. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L52` |
| A468 | Acceptance media-replacement: Right-click replacement preserves layers, renders new pixels, undoes/redoes, and survives portable save. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L91` |
| A469 | Acceptance media-replacement: Replacement picker cancellation and unreadable or incompatible files leave media intact. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L146` |
| B449 | media-replacement contract: Replacement uses the import picker, progress, and image sequence flow. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L9` |
| B450 | media-replacement contract: Replacement picker remains bound to its original project. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L52` |
| B451 | media-replacement contract: Right-click replacement preserves layers, renders new pixels, undoes/redoes, and survives portable save. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L91` |
| B452 | media-replacement contract: Replacement picker cancellation and unreadable or incompatible files leave media intact. | `v1.2.0:apps/desktop/e2e/media-replacement.spec.ts:L146` |

### W023 — Defer and download cloud media

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Cloud-only placeholders and a failed download.
**Steps:** Defer; download in place; enable automatic restoration; retry failure; reopen.
**Expected:** Placeholder status remains explicit; fresh durable bytes resolve the same asset and tasks clear.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/cloud-media.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`.
**Existing shipped e2e:** `cloud-media.spec.ts`; `import-cloud.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A427 | Cloud placeholders: Allow defer/download in place and optional automatic restoration; never silently treat placeholders as valid bytes. | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts` |
| A433 | Acceptance cloud-media: Cloud media can be deferred, downloaded in place, and automatically restored. | `v1.2.0:apps/desktop/e2e/cloud-media.spec.ts:L6` |
| A435 | Acceptance import-cloud: Cloud imports show download state, use fresh bytes, and clear failed tasks. | `v1.2.0:apps/desktop/e2e/import-cloud.spec.ts:L6` |

### W024 — Connect subscription, API and local agent providers

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Disconnected, connected and failed provider states; synthetic credentials supplied outside tracked fixtures.
**Steps:** Connect/disconnect; configure address/model/image support; stream a reply; interrupt the connection and retry; relaunch.
**Expected:** Correct identity/capabilities, durable non-secret preferences and recoverable errors; keys never enter project or extension source.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts`, `v1.2.0:apps/desktop/src/shared/compatible-provider.ts`, `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/chatgpt-settings.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`.
**Existing shipped e2e:** `agent-connection-layout.spec.ts`; `agent-models.spec.ts`; `chatgpt-connection.spec.ts`; `compatible-authoring.spec.ts`; `compatible-provider.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A280 | Accounts providers: Host ChatGPT and Claude subscriptions, API/local model connection, and Other agent apps. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A338 | Settings control Provider: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A339 | Settings control Address: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A340 | Settings control API key: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A341 | Settings control Model: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A342 | Settings control Images: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A343 | Settings control This model supports images: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A344 | Settings control Extensions: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A345 | Settings control Extension details: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A346 | Settings control Search extensions: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A347 | Settings control Filter extensions: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A348 | Settings control Extension type: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A349 | Settings control Close extension details: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A350 | Settings control Installed: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A351 | Settings control Status: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A352 | Settings control About: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A353 | Settings control Setup: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A354 | Settings control Files: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A355 | Settings control Remove: Retain named account/provider/extension control and its live connection/availability state. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A356 | Subscription connection states: Show Checking, Connect/Retry, Waiting, Connected/Disconnect and Restart needed with account identity/error detail; disable action where unavailable. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/chatgpt-settings.ts` |
| A358 | Compatible address validation: Accept HTTPS APIs and loopback HTTP; normalize bare address or chat/completions endpoint; reject credentials/query/hash in URL. | `v1.2.0:apps/desktop/src/shared/compatible-provider.ts` |
| A360 | Compatible image support: Persist This model supports images capability with address/model/key connection settings. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A471 | Agent provider: Choose Codex/ChatGPT, Claude or configured compatible API/local provider with model/capability-aware options. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A575 | Acceptance agent-connection-layout: Connection setup owns the empty panel and stays usable when resized. | `v1.2.0:apps/desktop/e2e/agent-connection-layout.spec.ts:L4` |
| A587 | Acceptance agent-models: Sol 6.1 and Astra selections reach the request and Claude selections preserve capabilities. | `v1.2.0:apps/desktop/e2e/agent-models.spec.ts:L3` |
| A627 | Acceptance chatgpt-connection: Settings reads a ChatGPT subscription through the real main-process bridge. | `v1.2.0:apps/desktop/e2e/chatgpt-connection.spec.ts:L6` |
| A628 | Acceptance chatgpt-connection: Connect opens the trusted ChatGPT browser flow and enters waiting state. | `v1.2.0:apps/desktop/e2e/chatgpt-connection.spec.ts:L54` |
| A629 | Acceptance chatgpt-connection: Large images and files cross the real renderer-to-main agent boundary. | `v1.2.0:apps/desktop/e2e/chatgpt-connection.spec.ts:L91` |
| A630 | Acceptance chatgpt-connection: Claude subscription status and structured runs cross the real hidden app boundary. | `v1.2.0:apps/desktop/e2e/chatgpt-connection.spec.ts:L133` |
| A631 | Acceptance compatible-provider: Connects a local model, streams chat, recovers after a broken stream and remembers the connection. | `v1.2.0:apps/desktop/e2e/compatible-provider.spec.ts:L4` |
| A632 | Acceptance compatible-provider: Provider models, reasoning, and flat error actions remain usable in a narrow panel. | `v1.2.0:apps/desktop/e2e/compatible-provider.spec.ts:L56` |

### W025 — Open, float, pin and resize the agent conversation

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** An existing draft and running conversation.
**Steps:** Open from launcher; drag header/grip; cancel; dock/pin; close/reopen; resize narrowly; relaunch.
**Expected:** Same conversation/draft and readable controls; saved placement; usable drag area and no horizontal overflow.
**Exact registered keymap entries:** K003 cmd+shift+k → agent; K004 ctrl+shift+k → agent.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent-shell.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`.
**Existing shipped e2e:** `agent-connection-layout.spec.ts`; `agent-header-drag.spec.ts`; `agent-markdown.spec.ts`; `agent-panel.spec.ts`; `agent-redesign.spec.ts`; `agent-response-preservation.spec.ts`; `agent-result-presentation.spec.ts`; `agent-steering-reasoning.spec.ts`; `agent-streaming.spec.ts`; `agent-subagents.spec.ts`; `agent-width.spec.ts`; `interface-polish.spec.ts`; `spatial-ripple.spec.ts`; `window-drag-region.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A148 | Cmd/Ctrl+shift+k: Run &#96;agent&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A470 | Agent placement: Titlebar launcher opens agent popover; pin/drag docks it and floating grip moves/cancels/docks the same conversation. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A592 | Acceptance agent-panel: Fills its column when docked alone and keeps its resized height beside another panel. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L4` |
| A593 | Acceptance agent-panel: Opens under the titlebar launcher, never in a dock, and follows the window. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L58` |
| A594 | Acceptance agent-panel: Dragging the launcher docks the agent; closing the panel returns it to the popover. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L83` |
| A595 | Acceptance agent-panel: Dragging re-pins a closed agent and keeps it visible after restarting. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L116` |
| A596 | Acceptance agent-panel: The floating grip moves, cancels and docks the same conversation. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L155` |
| A597 | Acceptance agent-panel: The pin button keeps the conversation in the workspace. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L203` |
| A598 | Acceptance agent-panel: Composer submit → progress → result phase, all through the Svelte UI. | `v1.2.0:apps/desktop/e2e/agent-panel.spec.ts:L228` |
| A602 | Acceptance agent-redesign: Studio agent keeps suggestions, steering, activity and narrow layouts usable. | `v1.2.0:apps/desktop/e2e/agent-redesign.spec.ts:L5` |
| A603 | Acceptance agent-result-presentation: Agent results preserve work above the reply and omit file and external activity panels. | `v1.2.0:apps/desktop/e2e/agent-result-presentation.spec.ts:L5` |
| A604 | Acceptance agent-result-presentation: A completed response stays visible when the user sends a follow-up. | `v1.2.0:apps/desktop/e2e/agent-result-presentation.spec.ts:L91` |
| A606 | Acceptance agent-streaming: Streamed text stays inline and keeps existing nodes as chunks arrive. | `v1.2.0:apps/desktop/e2e/agent-streaming.spec.ts:L3` |
| A607 | Acceptance agent-subagents: Subagents update in place, expose results and Stop, and fit native light and dark panels. | `v1.2.0:apps/desktop/e2e/agent-subagents.spec.ts:L6` |
| A626 | Acceptance agent-width: Agent content wraps without horizontal scroll at narrow and normal widths. | `v1.2.0:apps/desktop/e2e/agent-width.spec.ts:L3` |

### W026 — Write, attach and send an agent prompt

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Multiline draft with inline image/file tokens and a provider ready to accept a request.
**Steps:** Type/scroll; select/copy/paste; use slash menu; test Enter, Shift+Enter, repeat and IME; remove attachments; send text-only and file-only drafts.
**Expected:** Original bytes and token order delivered once, caret/newlines stable, local Undo/clipboard and IME preserved.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`.
**Existing shipped e2e:** `agent-attachment-preview.spec.ts`; `agent-composer-deletion.spec.ts`; `agent-composer-scroll.spec.ts`; `agent-composer-shortcuts.spec.ts`; `agent-threads.spec.ts`; `chatgpt-connection.spec.ts`; `prompt-attachments.spec.ts`; `spatial-prompt-paste.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A255 | Agent composer keys: Enter sends, Shift+Enter inserts newline; repeated Enter ignored; IME composition bypasses submission; slash menu Up/Down wraps and Enter/Tab selects, Escape dismisses. | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte` |
| A256 | Agent text focus: Cmd/Ctrl+A selects draft and Cmd/Ctrl+V stays in native paste path; Escape follows composer-specific dismissal behavior. | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte` |
| A475 | Composer: Preserve multiline draft, caret, wrapped scrolling, file-only drafts, inline attachment positions and two-step deletion. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A476 | Composer shortcuts: Slash menu supports keyboard and pointer selection/dismissal; Cmd+A and native clipboard operate on draft/text. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A477 | Attachments: Accept images and binary files, remove individually, transmit original bytes, open fitted image preview and restore focus with Escape. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A566 | Acceptance agent-attachment-preview: Agent image attachments open a fitted preview and Escape returns focus. | `v1.2.0:apps/desktop/e2e/agent-attachment-preview.spec.ts:L3` |
| A570 | Acceptance agent-composer-deletion: Clearing composer lines preserves only intentional newlines and keeps the caret in place. | `v1.2.0:apps/desktop/e2e/agent-composer-deletion.spec.ts:L3` |
| A571 | Acceptance agent-composer-shortcuts: Slash shortcuts support keyboard selection, dismissal, pointer selection, and multiline drafts. | `v1.2.0:apps/desktop/e2e/agent-composer-shortcuts.spec.ts:L43` |
| A572 | Acceptance agent-composer-shortcuts: Inline file tokens preserve their place, bytes, and two-step deletion in the desktop editor. | `v1.2.0:apps/desktop/e2e/agent-composer-shortcuts.spec.ts:L94` |
| A573 | Acceptance agent-composer-shortcuts: File-only drafts keep their caret and new text on the same line. | `v1.2.0:apps/desktop/e2e/agent-composer-shortcuts.spec.ts:L130` |
| A633 | Acceptance prompt-attachments: Floating prompt attaches images and binary files, removes them, and sends their contents. | `v1.2.0:apps/desktop/e2e/prompt-attachments.spec.ts:L5` |

### W027 — Run and revisit ordinary/background agent threads

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Two projects, multiple threads and drafts.
**Steps:** Create/switch/read/steer/watch/cancel a background conversation; switch project during run; return and relaunch.
**Expected:** Transcripts, titles, drafts and attachments stay thread-scoped; running badge/Stop accurate; streaming resumes without losing text.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/spatial.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts`.
**Existing shipped e2e:** `agent-orchestrated-threads.spec.ts`; `agent-project-persistence.spec.ts`; `agent-threads.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A473 | Conversation durability: Persist transcripts, titles, threads, drafts and attachments across switches, file opens and relaunch without cross-thread leakage. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A474 | Run continuity: Continue runs through tab/project switching; restore live streaming when returning. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A481 | Threads: Create/select background/planning threads, show compact running badge, preserve visible chat/draft and expose Stop. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A551 | MCP &#96;task_send&#96;: Steer an active subagent with follow-up instructions and/or change its provider, model or reasoning effort. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L47` |
| A552 | MCP &#96;thread_launch&#96;: Start a separate ordinary conversation in this project, visible in the thread picker. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L52` |
| A553 | MCP &#96;thread_list&#96;: List ordinary conversations in this project, with their current run IDs and settings. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L56` |
| A554 | MCP &#96;thread_read&#96;: Read paginated messages and the latest run state of an ordinary thread in this project. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L60` |
| A555 | MCP &#96;thread_send&#96;: Send a follow-up to an ordinary thread in this project. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L64` |
| A556 | MCP &#96;thread_watch&#96;: Watch a specific run of an ordinary thread. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L68` |
| A557 | MCP &#96;thread_unwatch&#96;: Remove this agent’s watch without interrupting the watched thread. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L72` |
| A558 | MCP &#96;thread_cancel&#96;: Stop the current run and its subagents in an ordinary thread in this project. | `v1.2.0:apps/desktop/src/main/agent-tools/orchestration-spec.ts:L76` |
| A588 | Acceptance agent-orchestrated-threads: Model-created threads preserve the visible chat and draft, deduplicate launches, and expose native thread controls. | `v1.2.0:apps/desktop/e2e/agent-orchestrated-threads.spec.ts:L6` |
| A589 | Acceptance agent-orchestrated-threads: A launched planning thread keeps inspection access and can be stopped from its normal composer. | `v1.2.0:apps/desktop/e2e/agent-orchestrated-threads.spec.ts:L74` |
| A590 | Acceptance agent-orchestrated-threads: The parent can read and answer a blocked background thread without interrupting it. | `v1.2.0:apps/desktop/e2e/agent-orchestrated-threads.spec.ts:L102` |
| A599 | Acceptance agent-project-persistence: Agent history follows a saved project across file opens, project switches, and relaunch. | `v1.2.0:apps/desktop/e2e/agent-project-persistence.spec.ts:L4` |
| A608 | Acceptance agent-threads: Background run badge stays compact when creating and switching threads. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L3` |
| A609 | Acceptance agent-threads: Command+A selects the full agent composer draft. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L56` |
| A610 | Acceptance agent-threads: Selected agent text copies and pastes normally without copying layers. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L70` |
| A611 | Acceptance agent-threads: Steering stays in the active run and preserves the transcript before it. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L109` |
| A612 | Acceptance agent-threads: Hidden renderer switches threads, keeps drafts, and restores history after relaunch. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L168` |
| A613 | Acceptance agent-threads: Background app is never visible or focused, but can draw and accept input. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L209` |
| A614 | Acceptance agent-threads: Thread transcripts and titles survive a hidden relaunch without leaking into another thread. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L223` |
| A615 | Acceptance agent-threads: Agent run survives clicking another project tab and resumes on return. | `v1.2.0:apps/desktop/e2e/agent-threads.spec.ts:L254` |

### W028 — Read streaming replies, reasoning, approvals and results

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A run emitting text, reasoning, tools, questions and a failure.
**Steps:** Stream chunks; steer; answer choices; inspect activity/subagent results; stop; send follow-up.
**Expected:** Text updates in place, reasoning/results retained, current approvals actionable, errors readable and work stays above the final response.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/spatial.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`.
**Existing shipped e2e:** `agent-header-drag.spec.ts`; `agent-markdown.spec.ts`; `agent-panel.spec.ts`; `agent-playhead.spec.ts`; `agent-prompt-glow.spec.ts`; `agent-redesign.spec.ts`; `agent-response-preservation.spec.ts`; `agent-result-presentation.spec.ts`; `agent-steering-reasoning.spec.ts`; `agent-streaming.spec.ts`; `agent-subagents.spec.ts`; `agent-width.spec.ts`; `error-presentation.spec.ts`; `prompt-run-glow.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A478 | Streaming: Append text in place, preserve reasoning during steering and keep completed replies above follow-up draft/result. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A479 | Markdown: Render rich prose throughout conversation, activity, questions and results in light/dark with narrow-width wrapping. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A480 | Approvals and questions: Present agent approval requests and structured questions with explicit choices and native error recovery. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A482 | Working timer: Show working duration during active turns with stable lifecycle across completion/stopping. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A483 | Activity: Display tool/thought/subagent rows in place, retain results, expose Stop and avoid horizontal overflow. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A601 | Acceptance agent-prompt-glow: Prompt halo sits outside the message bubble and stays within the panel. | `v1.2.0:apps/desktop/e2e/agent-prompt-glow.spec.ts:L5` |

### W029 — Publish and reopen interactive visual replies

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** An agent run and a self-contained generated attachment.
**Steps:** Publish; interact; expand/replay; use keyboard controls; reopen thread; submit malformed HTML and attempted host/network/tool access.
**Expected:** Durable bounded attachment, theme/focus behavior and isolation; rejected publication is clear and host capabilities never escape.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/agent/visual-reply.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`.
**Existing shipped e2e:** `agent-visual-replies.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A484 | Visual replies: Persist sandboxed interactive HTML attachments with compact preview, expand/replay/keyboard controls and live theme tokens. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A485 | Visual reply security: Block host privileges, networking, file access, forms/navigation and tool messages from generated scripts. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A502 | MCP &#96;publish_visual_reply&#96;: Publish a self-contained interactive HTML visual attachment in this run’s thread before your final text. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L29` |
| A619 | Acceptance agent-visual-replies: Visual replies publish, interact, expand, replay and reopen in their durable thread. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L63` |
| A620 | Acceptance agent-visual-replies: Generated scripts cannot access host privileges, send tool messages or navigate. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L112` |
| A621 | Acceptance agent-visual-replies: CSP blocks resource requests and external navigation from generated scripts. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L132` |
| A622 | Acceptance agent-visual-replies: Visual replies resize within bounds and malformed publication fails clearly. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L159` |
| A623 | Acceptance agent-visual-replies: The real provider run publishes through its authenticated MCP session. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L179` |
| A624 | Acceptance agent-visual-replies: Attachments preserve interaction while the response adds pages and remain compact at narrow widths. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L206` |
| A625 | Acceptance agent-visual-replies: Visual attachment chrome matches native modal tokens in light and dark and supports keyboard controls. | `v1.2.0:apps/desktop/e2e/agent-visual-replies.spec.ts:L232` |

### W030 — Let an agent inspect, edit, verify and undo project work

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Large project with protected hand edits and a concurrent user revision.
**Steps:** Inspect real state/frames; perform complete edit batch; race a user edit; review frames without moving playhead; undo/redo or guarded rollback; force capture failure.
**Expected:** Revision/lock rules apply; all intended edits land atomically; prior user work survives; failed verification keeps edits and reports limits.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/harness.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`.
**Existing shipped e2e:** `agent-canvas-capture.spec.ts`; `agent-context-review.spec.ts`; `agent-extension-recovery.spec.ts`; `agent-keyframe-easing.spec.ts`; `agent-media-handoff.spec.ts`; `agent-path-mask.spec.ts`; `agent-playhead.spec.ts`; `agent-project-timeline.spec.ts`; `agent-revision-recovery.spec.ts`; `agent-undo-redo.spec.ts`; `agent-video-editing.spec.ts`; `compatible-authoring.spec.ts`; `keyframe-simplification-proof.spec.ts`; `keyframe-simplification.spec.ts`.
**Later candidate e2e (not shipped evidence):** `document-host-contract.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A472 | Agent access: Separate planning inspection from project editing and app extension work; preserve attached project context. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A486 | Review and undo: Review real resulting frames without moving paused playhead; retain edits on capture failure and state verification limits. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A511 | MCP &#96;get_project_state&#96;: Honor the project memory index (goal, rules, open items and latest decisions); use project_memory for details. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts` |
| A512 | MCP &#96;project_memory&#96;: Read or write this project's memory: the goal, standing rules, decisions and rejected ideas that explain why the video looks as it does. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts` |
| A517 | MCP &#96;select_layers&#96;: Select existing layers by their IDs from get_project_state, or clear selection with an empty array. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L107` |
| A567 | Acceptance agent-canvas-capture: Agent frame capture preserves the displayed canvas before the next animation frame. | `v1.2.0:apps/desktop/e2e/agent-canvas-capture.spec.ts:L3` |
| A568 | Acceptance agent-canvas-capture: Offscreen captures match opaque presentation without resizing or replacing a zoomed preview. | `v1.2.0:apps/desktop/e2e/agent-canvas-capture.spec.ts:L41` |
| A569 | Acceptance agent-canvas-capture: Autosave persists during panning and captures its thumbnail after navigation settles. | `v1.2.0:apps/desktop/e2e/agent-canvas-capture.spec.ts:L85` |
| A576 | Acceptance agent-context-review: Live discovery reaches late layers and all keyframes in a large composition. | `v1.2.0:apps/desktop/e2e/agent-context-review.spec.ts:L3` |
| A577 | Acceptance agent-context-review: Composition only reviews and repairs with the generation settings after a picker change. | `v1.2.0:apps/desktop/e2e/agent-context-review.spec.ts:L35` |
| A578 | Acceptance agent-context-review: A composition edit reviews final frames with the original settings after a picker change. | `v1.2.0:apps/desktop/e2e/agent-context-review.spec.ts:L64` |
| A579 | Acceptance agent-context-review: Capture failure preserves applied edits and reports the verification limit. | `v1.2.0:apps/desktop/e2e/agent-context-review.spec.ts:L114` |
| A600 | Acceptance agent-project-timeline: Recovered strip gestures share Undo and library operations retain independent projects. | `v1.2.0:apps/desktop/e2e/agent-project-timeline.spec.ts:L3` |
| A605 | Acceptance agent-revision-recovery: A stale agent run refreshes, applies a complete batch, and preserves interleaved user Undo. | `v1.2.0:apps/desktop/e2e/agent-revision-recovery.spec.ts:L3` |
| A618 | Acceptance agent-video-editing: Agent cuts imported footage, reviews decoded frames, and undoes the run. | `v1.2.0:apps/desktop/e2e/agent-video-editing.spec.ts:L4` |

### W031 — Inspect and manage footage through agent tools

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Imported video/audio, missing media, nested references and folders.
**Steps:** Probe/sample/contact-sheet/listen/transcribe; import without layers; replace; rename/move/delete; retry failure and undo.
**Expected:** Real source evidence, durable ownership, same guards/Undo as UI and no cross-project results.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/harness.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts`.
**Existing shipped e2e:** `agent-media-handoff.spec.ts`; `agent-media-management.spec.ts`; `agent-media-tools.spec.ts`; `media-intelligence.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A488 | Agent media: List/import/replace/manage assets directly with durable verification, locks, normal Undo and explicit rollback limits. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A500 | MCP &#96;import_media_to_timeline&#96;: Import a host-local video, audio, image or model file through Powermove's media importer. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L12` |
| A506 | MCP &#96;save_project&#96;: Save the current composition as an editable .pmv using Powermove's project packer, including its media. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L59` |
| A513 | MCP &#96;list_media&#96;: List project media with stable asset IDs, missing/failed/loading/cloud status, durable storage status, original source paths, fingerprints, folders and layer references (including locks and nested compositions). | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L87` |
| A514 | MCP &#96;replace_media&#96;: Replace or relink one existing media asset from an absolute local file path. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L92` |
| A516 | MCP &#96;manage_media&#96;: Manage project media with normal editor Undo: rename one asset, move assetIds to folderId (null = root), delete assets, create_folder/rename_folder/delete_folder, or move_folder with folderId and parentId (null = root). | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L102` |
| A534 | MCP &#96;probe_media&#96;: Read a video or audio asset's technical metadata without decoding: container, duration, bitrate, and per stream codec/profile, width×height, fps, pixel format, alpha, rotation, audio channels/layout/sample rate/bitrate. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L230` |
| A535 | MCP &#96;sample_media_frames&#96;: Decode real frames from SOURCE footage (the asset's own pixels, not the composition) and return them as images. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L235` |
| A536 | MCP &#96;media_contact_sheet&#96;: One image: a grid of timecode-labelled thumbnails, the cheapest way to see a whole clip or the live composition at once. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L247` |
| A537 | MCP &#96;media_waveform&#96;: Listen to an asset's or clip's audio: silent ranges (ffmpeg silencedetect, below silenceThresholdDb for at least minSilence seconds; defaults -40 dB, 0.4 s), integrated loudness (LUFS), loudness range, true/sample peak, RMS and a clipping flag, plus a waveform image with silences shaded. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L259` |
| A539 | MCP &#96;check_project&#96;: Lint the live composition for structural problems the editor can prove: spans where no layer is visible (only the background shows), layers that never show (opacity 0 throughout, outside the composition or work area, zero duration), missing, failed, offline or cloud-only media, clips that play past the end of their media, and audio driven over full scale by its gain. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L282` |
| A581 | Acceptance agent-media-handoff: Generated PNGs reach the timeline and continuation groups and renders the actual layers. | `v1.2.0:apps/desktop/e2e/agent-media-handoff.spec.ts:L6` |
| A582 | Acceptance agent-media-management: Agent replaces missing media directly, preserves animation, verifies storage, and supports Undo/retry. | `v1.2.0:apps/desktop/e2e/agent-media-management.spec.ts:L7` |
| A583 | Acceptance agent-media-management: Agent imports without timeline layers and manages folders/deletion with native Undo and lock guards. | `v1.2.0:apps/desktop/e2e/agent-media-management.spec.ts:L57` |
| A584 | Acceptance agent-media-management: Agent replacement failures and interrupted work leave the original intact and can be retried. | `v1.2.0:apps/desktop/e2e/agent-media-management.spec.ts:L104` |
| A585 | Acceptance agent-media-tools: The agent watches and listens to imported footage through the media tools. | `v1.2.0:apps/desktop/e2e/agent-media-tools.spec.ts:L41` |

### W032 — Inspect and operate live panels or transfer a workspace

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Registered built-in/extension panel with canvas and a saved external workspace description.
**Steps:** Read layout and semantic controls; interact/capture/draw by panel target; stage an original extension; save a new named arrangement; test absent provider and retry.
**Expected:** Observed targets stay accurate, existing controls preserved, no unintended project edits and original extension work is recoverable.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/harness.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`.
**Existing shipped e2e:** `agent-computer-use.spec.ts`; `agent-panel-tools.spec.ts`; `panel-workflow.spec.ts`; `ui-placement.spec.ts`; `workspace-import.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A387 | Acceptance panel-workflow: Drawing a box notes every touched panel and sends that focus with the prompt. | `v1.2.0:apps/desktop/e2e/panel-workflow.spec.ts:L204` |
| A388 | Acceptance workspace-import: Settings workspace transfer creates, loads and arranges an original extension without editing the project. | `v1.2.0:apps/desktop/e2e/workspace-import.spec.ts:L8` |
| A389 | Acceptance workspace-import: Settings transfer from home keeps the request ready while the agent needs sign-in. | `v1.2.0:apps/desktop/e2e/workspace-import.spec.ts:L127` |
| A390 | Acceptance workspace-import: Settings transfer stays retryable when the handoff is unavailable or fails. | `v1.2.0:apps/desktop/e2e/workspace-import.spec.ts:L146` |
| A519 | MCP &#96;set_panel_layout&#96;: Save and activate a NEW named Powermove workspace with ordered panels in left, center, right and bottom docks. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L117` |
| A520 | MCP &#96;get_panel_layout&#96;: Read the live Powermove dock and panel layout, including registered panel ids and titles. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L125` |
| A522 | MCP &#96;open_panel&#96;: Open or expand a registered Powermove panel, restoring its saved position. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L135` |
| A523 | MCP &#96;get_panel_state&#96;: Read visible panel text and semantic controls with fresh refs, labels, values and select options. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L140` |
| A524 | MCP &#96;interact_panel&#96;: Use an observed panel control: click a button, fill a text field, select an option, or press a supported key. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L145` |
| A526 | MCP &#96;computer_use_panel&#96;: Send real Chromium mouse, drag, wheel or keyboard input to an observed panel, including custom canvases, sliders and outline drawing. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L155` |
| A527 | MCP &#96;get_workspace_state&#96;: Read live project identity, selection, panel layout and recent renderer errors for troubleshooting. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L160` |
| A574 | Acceptance agent-computer-use: Agent bridge captures and draws on a real extension canvas with native input. | `v1.2.0:apps/desktop/e2e/agent-computer-use.spec.ts:L8` |
| A591 | Acceptance agent-panel-tools: Agent opens and uses an extension panel through its real controls. | `v1.2.0:apps/desktop/e2e/agent-panel-tools.spec.ts:L3` |
| B539 | panel-workflow contract: Drawing a box notes every touched panel and sends that focus with the prompt. | `v1.2.0:apps/desktop/e2e/panel-workflow.spec.ts:L204` |

### W033 — Use an external/headless tool session

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** An external client and a saved project.
**Steps:** Start fixed app-owned entry; list/create/open; pin session; edit; finish commit/rollback; reload editor; disconnect and reconnect.
**Expected:** Project/session isolation, authenticated local discovery, complete tool availability, guarded rollback and continuing connection; exported side effects remain explicit.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts`.
**Existing shipped e2e:** `agent-threads.spec.ts`; `agent-tool-runtime.spec.ts`; `mcp-navigation.spec.ts`; `renderer-crash.spec.ts`.
**Later candidate e2e (not shipped evidence):** `document-host-contract.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A489 | External MCP entry: --powermove-mcp runs fixed app-owned stdio entry without Electron RunAsNode or caller-supplied scripts. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A490 | External MCP discovery: Use loopback authenticated bridge and private connection descriptor; never place discovery credential in client configuration. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A491 | External MCP sessions: Pin one project/session per connection; require finish before changing projects and drain children on close. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A492 | External MCP finish: commit:true retains normal Undo edits; false/disconnect attempts guarded rollback, while published extensions/exported files remain. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A493 | Hidden host: Allow headless editor and media imports without file pickers; tools requiring pixels still need render capability. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A494 | MCP tool union: Expose EXTERNAL_MCP_TOOLS plus POWERMOVE_AGENT_TOOLS including orchestration tools; capabilities can depend on host configuration. | `v1.2.0:apps/desktop/src/main/agent-tools/external-entry.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-server.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/external-session.ts` |
| A495 | MCP &#96;list_projects&#96;: List saved projects and the current project in this Powermove host. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L7` |
| A496 | MCP &#96;create_project&#96;: Create and open a new empty video project without a dialog. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L8` |
| A497 | MCP &#96;open_project&#96;: Open a saved project by ID from list_projects. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L9` |
| A498 | MCP &#96;start_session&#96;: Start a project or app tool session, pinned to the currently open project. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L10` |
| A501 | MCP &#96;finish_session&#96;: Finish this session: commit keeps live edits in normal Undo history; false rolls back guarded edits. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L13` |
| A503 | MCP &#96;get_timeline&#96;: Read compact layer strips in visual stack order, with selection, frame positions, timing, source ranges, group/parent links, visibility, solo and locks. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L32` |
| A504 | MCP &#96;edit_layer_strips&#96;: Atomic human-style timeline gestures on explicit layerIds. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L37` |
| A505 | MCP &#96;manage_project&#96;: Manage the project library without dialogs: list, create and open in this window, rename, duplicate, save a durable local checkpoint, trash a closed project, or restore it. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L50` |
| A528 | MCP &#96;render_frames&#96;: Render one to five frames from the live composition. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L165` |
| A529 | MCP &#96;apply_commands&#96;: simplify_keyframes {scope:project\|layers\|properties, targets?:layerIds, properties?:[{layerId,path}]} removes only verified redundant keys, returns before/after counts, and keeps locked layers and expressions. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L177` |
| A530 | MCP &#96;edit_video&#96;: Edit real video/audio timeline clips. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L194` |
| A531 | MCP &#96;rollback_changes&#96;: Roll back every live composition edit made by this agent run, without touching earlier project work. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L214` |
| A533 | MCP &#96;stage_fork_rebase&#96;: Stage a stale user fork for a three-way rebase onto the built-in version shipped by this Powermove app. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L224` |
| A616 | Acceptance agent-tool-runtime: The app-owned MCP entrypoint lists live tools without Electron Node mode. | `v1.2.0:apps/desktop/e2e/agent-tool-runtime.spec.ts:L7` |

### W034 — Use the existing browser-host/CLI workflows

**Status:** EXISTS in shipped tag; native macOS scope exclusion is explicit above.
**Preconditions:** An authorized remote host and browser device.
**Steps:** Serve/connect; edit/reconnect; import media; export/download; inspect service status/logs.
**Expected:** Shared host document/media, per-device Undo and continuing runs. Existing workflow is recorded but browser serve and its service lifecycle are explicitly outside the native macOS cutover scope.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/server/cli.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A559 | CLI serve: Run browser-accessible host; default port 4747, bind 0.0.0.0, profile ~/.powermove and exports ~/Powermove. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A560 | CLI lifecycle: install registers user launchd/systemd service, reinstall updates, status reports address, logs reads logs and uninstall removes service. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A561 | CLI options: Support --port, --host, --user-data, --exports, --token and --http; default HTTPS self-signed with remembered access token. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A562 | CLI MCP: mcp-install-browser prepares hidden browser; mcp runs stdio hidden host with ~/.powermove-headless default; --connect targets existing profile. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A563 | Remote reconnect: Reconnect interrupted browser sessions, replay pending edits and resume agent streams. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A564 | Remote documents: Host owns shared document/media/fonts, devices keep their own Undo, and host agent runs continue after browser closes. | `v1.2.0:apps/desktop/src/server/cli.ts` |
| A565 | Remote exports: Write on host and download to browser; source/npx update paths may present a manual command. | `v1.2.0:apps/desktop/src/server/cli.ts` |

### W035 — Manage account identity and remembered Store installs

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Signed-out and signed-in accounts with remembered installs.
**Steps:** Sign in; choose handle; sign out; toggle remembered installs; simulate error; request account deletion.
**Expected:** Correct profile/availability and rollback of failed toggles; deletion rules and published extension ownership preserved.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A277 | Accounts profile: Show avatar, handle/email; offer Choose Handle when missing, Sign Out, Sign In and Delete Account. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A278 | Remember installs: Account toggle defaults on; disabling deletes remembered install list, failures restore toggle. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A279 | Account deletion: Delete account keeps published extensions under the handle; display success/failure. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A634 | Cloud identity: Sign in through trusted account flow, choose handle and show account profile/sign-out/deletion states. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |

### W036 — Browse, install, update and publish extensions

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Catalog entries, an installed extension and an owned publishable extension.
**Steps:** Search/read source/details; install/update/remove with permission review; prepare publication; edit icon/metadata; cancel/reopen; test validation failures.
**Expected:** Declared trust/permissions, source provenance, recoverable errors and remembered installs. Publication remains an explicit user action.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte`.
**Existing shipped e2e:** `store-icons.spec.ts`; `store-publish-panel.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A540 | MCP &#96;store_search&#96;: Search the Powermove Store for published extensions (effects, transitions, panels, themes, commands, layers, tools). | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L288` |
| A541 | MCP &#96;store_extension&#96;: Read a Store extension's detail by handle and slug: description, declared permissions, verified publisher, install count, and its full release history (each with a releaseId and version). | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L298` |
| A542 | MCP &#96;store_source&#96;: Read a published release's source before installing it. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L303` |
| A543 | MCP &#96;store_library&#96;: List the store extensions installed on this Mac: local id, name, version, permissions, whether enabled, any available update, and whether each is yours to publish. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L308` |
| A544 | MCP &#96;store_install&#96;: Download and install a Store extension into this Mac. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L313` |
| A545 | MCP &#96;store_update&#96;: Update an installed store extension to its latest release. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L318` |
| A546 | MCP &#96;store_uninstall&#96;: Remove an installed store extension by its local id (from store_library). | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L323` |
| A547 | MCP &#96;store_publish_prepare&#96;: Dry-run a publish of a local extension without publishing: returns the coordinate, suggested version, tree hash, file count/size, whether it is a first publish, and any blocking or waivable secret/permission findings. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L328` |
| A548 | MCP &#96;store_publish&#96;: Publish a local extension to the Powermove Store. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L333` |
| A635 | Store browse: Browse/search extension catalog with artwork fallback and extension detail disclosures. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A636 | Store installation: Install/update extensions with declared permission/trust review; optionally remember installed list on account. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A637 | Store publication: Select extension and show full publish panel through preparation, icon crop, metadata, cancellation and reopening. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A638 | Publish validation: Check ownership, provenance, sandbox compatibility and staged source before publishing; keep errors actionable. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A639 | Store forks: Preserve upstream identity and fork metadata; full-access installs remain off until accepted and can revoke trust later. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A640 | Sync boundary: Account remembered installs and Store publication are evidenced; remote-host live sync is separate from a claimed general cloud project-sync product. | `v1.2.0:apps/desktop/src/renderer/src/cloud/account.ts`, `v1.2.0:apps/desktop/src/main/cloud/auth.ts`, `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/store/PublishSheet.svelte` |
| A717 | Acceptance store-icons: Loads Store icons in the built desktop app and keeps artwork when an image fails. | `v1.2.0:apps/desktop/e2e/store-icons.spec.ts:L6` |
| A718 | Acceptance store-publish-panel: Selecting an extension opens a full publish panel through preparation, cancellation, and reopening. | `v1.2.0:apps/desktop/e2e/store-publish-panel.spec.ts:L5` |

### W037 — Install, fork, rebase, reload and disable extensions

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Local/project/Store fixtures at apiVersion 1–3, old forks and an invalid newer version.
**Steps:** Discover/compile; activate/override; fork/rebase; hot reload; disable/remove; fail activation; inspect health notices and undo a rebase.
**Expected:** Owned registrations clean up and restore predecessors; project/window/layout/custom edits survive; old versions work; unsupported versions fail clearly.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/install.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts`.
**Existing shipped e2e:** `agent-extension-recovery.spec.ts`; `agent-mod-capabilities.spec.ts`; `compatible-authoring.spec.ts`; `effect-authoring.spec.ts`; `extension-layers.spec.ts`; `extension-remote-media.spec.ts`; `extensions.spec.ts`; `generated-panel.spec.ts`; `interface-cleanup.spec.ts`; `performance-warning.spec.ts`; `svelte-panel.spec.ts`; `svelte5-showcase.spec.ts`; `viewer-recovery.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-customization-corpus.spec.ts`; `engine-private-setup-compat.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A361 | Extension settings categories: Group Effects, Panels, Appearance, Layer types, Media, Editor tools and Other extensions; provide search/status/type filters and Clear filters. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A362 | Extension status states: Show Off, Active, Replaced, Update needed, Needs setup, Needs full access and Needs attention with detail. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A363 | Extension details actions: Show Installed/Status/About/Setup/Files/Remove sections; allow enable toggle, setup, Show in Finder and inline Delete confirmation/cancel/error. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| A487 | Extension repair: Stage changes, validate and publish locally; automatic repair/reverification is bounded and undoable. | `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, `v1.2.0:apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` |
| A499 | MCP &#96;publish_extensions&#96;: Validate, compile, secret-scan and atomically promote declared staged extension changes, then reload them. | `v1.2.0:apps/desktop/src/main/agent-tools/external-spec.ts:L11` |
| A510 | MCP &#96;fork_builtin_extension&#96;: Copy a shipped built-in extension into this run's isolated extension staging directory, rewrite it as a user fork, and retain a pristine merge base. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L74` |
| A515 | MCP &#96;import_media&#96;: Import a local media file into durable project media without adding timeline layers or packaging bytes into extension code. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L97` |
| A518 | MCP &#96;inspect_creative_workspace&#96;: Read a saved After Effects workspace on this computer: panel names, tab groups, relative bounds, visibility and an inventory of user and third-party script panels, CEP extensions and compiled plugins. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L112` |
| A521 | MCP &#96;inspect_creative_extension&#96;: Inspect one installed After Effects tool by its opaque toolId from inspect_creative_workspace. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L130` |
| A525 | MCP &#96;capture_panel&#96;: Capture a real image of any visible built-in or extension panel, including canvases. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L150` |
| A532 | MCP &#96;validate_effect&#96;: Check a complete proposed EffectDefinition using the actual kernel registration validator before returning an effect extension. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L219` |
| A580 | Acceptance agent-extension-recovery: Agent extension changes are isolated, recorded, and recoverable through the app bridge. | `v1.2.0:apps/desktop/e2e/agent-extension-recovery.spec.ts:L15` |
| A586 | Acceptance agent-mod-capabilities: A real mod supplies import defaults and Properties controls with automatic cleanup. | `v1.2.0:apps/desktop/e2e/agent-mod-capabilities.spec.ts:L5` |
| A641 | Extension versions: Accept apiVersion 1, 2 and 3; newer versions remain unloaded with needs-update state. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A642 | Extension locations: Discover built-in, user and project extensions; built-ins are read-only and must be forked for editing. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A643 | Extension manifests: Retain id/name/version/entry/contributes/replaces/dependsOn/forkedFrom/author/features/integrates plus vars/permissions/links rules. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A644 | Compilation: Load relative TS/JS/MJS/Svelte/CSS and allowed Svelte client modules; reject npm imports/path escapes. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A645 | Activation lifetime: All registrations are owned/disposable, overrides by same ID restore previous registration on disable and dependency/replacement order is respected. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A646 | Extension reload: Watch user source, compile and hot-reload panels in place without replacing document/window/docking. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A647 | Fault containment: Activation errors disable/report extension; two runtime errors in 10 seconds auto-disable; sandbox hangs/crashes are isolated. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A655 | Fork updates: Three-way rebase preserves custom edits, validates before install, offers Undo and hands conflicts to agent; failures retain previous copy. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A656 | Update compatibility: Persist app-version extension health/dismissals and show Review extensions notices; never silently rewrite/delete custom work. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A710 | Acceptance extensions: Replaces panel source in place without changing the document, window, or docking. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L29` |
| A711 | Acceptance extensions: A user extension activates, contributes a command and an effect, and survives disable. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L54` |
| A712 | Acceptance extensions: Timeline replacements swap with the built-in timeline without corrupting the live panel. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L99` |
| A713 | Acceptance extensions: Settings explains an override and deletes it while restoring the built-in panel. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L161` |
| A714 | Acceptance extensions: A broken extension is contained: the app boots and reports the failure. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L193` |
| A715 | Acceptance extensions: Built-in mods panel is registered. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts` |
| A716 | Acceptance extensions: Turning the built-in timeline off and on restores its live panel in place. | `v1.2.0:apps/desktop/e2e/extensions.spec.ts:L239` |

### W038 — Use Store extensions under permission and crash isolation

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Two Store extensions, a permitted one and a denied one.
**Steps:** Render/react to project; contribute menu; import URL/media; copy with genuine gesture; open declared/external links; spin/crash one runtime.
**Expected:** Other extension/app work survives; permission and recent-input gates hold; no clipboard read, host escape or project mutation without permission.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/sandbox-host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/sandbox-schemas.ts`, `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts`.
**Existing shipped e2e:** `extension-remote-media-ownership.spec.ts`; `extension-remote-media.spec.ts`; `extension-sandbox-clipboard.spec.ts`; `extension-sandbox-isolation.spec.ts`; `extension-sandbox-menus.spec.ts`; `extension-sandbox-perf.spec.ts`; `extension-sandbox.spec.ts`; `panel-popout.spec.ts`; `security.spec.ts`; `svelte5-showcase.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A648 | Sandbox processes: Untrusted Store extensions use separate processes/opaque views with host panel chrome, theme and keyboard integration. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A649 | Sandbox permissions: Preserve network, clipboard, assets, project:write and full-access; versions 1/2 cannot declare permissions. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A650 | Read access: Project snapshots/selection/time/transport reads need no permission; without network data cannot leave sandbox. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A651 | Clipboard gate: Write only, focused extension panel with recent genuine user input; max 500000 characters and once per second; no clipboard read API. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A652 | External links: Up to five declared HTTPS origins with network and recent user action can open without prompt; other destinations use consent rules. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A703 | Acceptance extension-sandbox-isolation: Each store extension gets its own process and a spinning one is killed without touching the rest. | `v1.2.0:apps/desktop/e2e/extension-sandbox-isolation.spec.ts:L7` |
| A704 | Acceptance extension-sandbox-menus: A sandboxed context-menu item acts on the layer that was right-clicked. | `v1.2.0:apps/desktop/e2e/extension-sandbox-menus.spec.ts:L6` |
| A705 | Acceptance extension-sandbox-perf: Store extensions that do not read the project cost playback nothing, and readers share one snapshot per change. | `v1.2.0:apps/desktop/e2e/extension-sandbox-perf.spec.ts:L91` |
| A706 | Acceptance extension-sandbox: Store code runs in an opaque iframe and network permission controls fetch. | `v1.2.0:apps/desktop/e2e/extension-sandbox.spec.ts:L7` |
| A707 | Acceptance extension-sandbox: A store extension’s Svelte panel renders in its own view iframe with host chrome and keys. | `v1.2.0:apps/desktop/e2e/extension-sandbox.spec.ts:L52` |
| A708 | Acceptance extension-sandbox: A sandboxed Svelte panel follows the playhead and the project through reactive api reads. | `v1.2.0:apps/desktop/e2e/extension-sandbox.spec.ts:L158` |
| A709 | Acceptance extension-sandbox: A sandboxed panel submits its form, fetches data: URLs, and survives blocked loads. | `v1.2.0:apps/desktop/e2e/extension-sandbox.spec.ts:L261` |

### W039 — Configure extension variables and durable storage

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A v2/v3 extension with declared secret/non-secret vars and stored values.
**Steps:** Set a subset; reload; read declared keys; remove extension with optional variable deletion; reopen.
**Expected:** Only declared variables exposed, optional missing values do not block activation, secrets stay outside source and storage retains its contract.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts`, `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A653 | Variables: apiVersion 2+ declares up to 32 values; masked secrets stored outside extension source and delivered only for declared keys. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A654 | Variable editing: Set Up/Variables saves any subset, reloads extension, and optional deletion accompanies extension removal; missing values do not block activation. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |

### W040 — Run legacy extension APIs and contributions unchanged

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Trusted and sandboxed apiVersion 1–3 fixtures exercising each public API member and legacy PM/service access.
**Steps:** Register/run/dispose each original API; read after write; observe identities/events/errors; mutate through history/edit/workspace; replace service/provider; render/snapshot in sync and async calls.
**Expected:** Frozen behavioral traces match calls/returns/identities/undo/errors. Authored geometry, text, timing, colors, shader coordinates and extension drawing remain correct. No silent compatibility exception.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/host.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`.
**Existing shipped e2e:** `agent-mod-capabilities.spec.ts`; `expression-csp.spec.ts`; `extension-sandbox-clipboard.spec.ts`; `extension-sandbox-isolation.spec.ts`; `extension-sandbox-menus.spec.ts`; `extension-sandbox-perf.spec.ts`; `extension-sandbox.spec.ts`; `status-polling.spec.ts`; `svelte-panel.spec.ts`; `typewriter-raster.spec.ts`.
**Later candidate e2e (not shipped evidence):** `channel-operators.spec.ts`; `document-host-boot.spec.ts`; `document-host-contract.spec.ts`; `engine-agent-observation.spec.ts`; `engine-color-depth-capture.spec.ts`; `engine-color-depth-expanded.spec.ts`; `engine-compat-context.spec.ts`; `engine-compat-moat.spec.ts`; `engine-customization-corpus.spec.ts`; `engine-definitions-export.spec.ts`; `engine-doc-boot.spec.ts`; `engine-document-recovery.spec.ts`; `engine-expression-performance.spec.ts`; `engine-extension-compat.spec.ts`; `engine-frame-painter.spec.ts`; `engine-headless-playback.spec.ts`; `engine-large-project.spec.ts`; `engine-present-recovery.spec.ts`; `engine-private-setup-compat.spec.ts`; `engine-render-transport.spec.ts`; `engine-shader-first-use.spec.ts`; `engine-svg-maps.spec.ts`; `engine-svg.spec.ts`; `engine-video-superwhite.spec.ts`; `engine-viewer-canvas.spec.ts`; `engine-viewport-compat.spec.ts`; `layer-input-effects.spec.ts`; `native-engine-mount-measure.spec.ts`; `native-engine-p3a-measure.spec.ts`; `native-engine-tracer-measure.spec.ts`; `native-engine-tracer.spec.ts`; `set-channels.spec.ts`; `source-owner-effects.spec.ts`; `source-raster-inputs.spec.ts`; `typed-effect-references.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A658 | PanelsAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;list&#96;, &#96;open&#96;, &#96;close&#96;, &#96;isOpen&#96;, &#96;refresh&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A659 | CommandsAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;run&#96;, &#96;has&#96;, &#96;list&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A660 | KeybindingsAPI: Preserve extension-facing contract: &#96;bind&#96;, &#96;unbind&#96;, &#96;list&#96;, &#96;chordOf&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A661 | EffectsAPI at tag: register, list, get(id). supports* capability flags, parameterDefault and versioned get are post-tag. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A662 | TransitionsAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;list&#96;, &#96;get&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A663 | ExtensionLayersAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;list&#96;, &#96;get&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A664 | Scene3DAPI: Preserve extension-facing contract: &#96;isGroup&#96;, &#96;getView&#96;, &#96;setView&#96;, &#96;onViewChange&#96;, &#96;getNavigationMode&#96;, &#96;setNavigationMode&#96;, &#96;onNavigationChange&#96;, &#96;createNavigation&#96;, &#96;frame&#96;, &#96;getMode&#96;, &#96;setMode&#96;, &#96;getSpace&#96;, &#96;setSpace&#96;, &#96;onGizmoChange&#96;, &#96;createGizmo&#96;, &#96;prepareImport&#96;, &#96;convert&#96;, &#96;createScene&#96;, &#96;createObject&#96;, &#96;createLight&#96;, &#96;edit&#96;, &#96;describe&#96;, &#96;selection&#96;, &#96;select&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A665 | AssetsAPI: Preserve extension-facing contract: &#96;pick&#96;, &#96;import&#96;, &#96;importUrl&#96;, &#96;get&#96;, &#96;readText&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A666 | ThemeAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;activate&#96;, &#96;active&#96;, &#96;list&#96;, &#96;scheme&#96;, &#96;setScheme&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A667 | PaletteAPI: Preserve extension-facing contract: &#96;registerProvider&#96;, &#96;open&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A668 | MenusAPI: Preserve extension-facing contract: &#96;contribute&#96;, &#96;collect&#96;, &#96;gather&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A669 | StatusAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;list&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A670 | ProjectAPI: Preserve extension-facing contract: &#96;get&#96;, &#96;latest&#96;, &#96;revision&#96;, &#96;apply&#96;, &#96;selection&#96;, &#96;select&#96;, &#96;time&#96;, &#96;setTime&#96;, &#96;play&#96;, &#96;pause&#96;, &#96;playing&#96;, &#96;undo&#96;, &#96;redo&#96;, &#96;snapshot&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A671 | AnimAPI: Preserve extension-facing contract: &#96;ev&#96;, &#96;evP&#96;, &#96;active&#96;, &#96;findProp&#96;, &#96;allProps&#96;, &#96;hasKeyAt&#96;, &#96;setKey&#96;, &#96;setKeyOn&#96;, &#96;removeKey&#96;, &#96;applyEaseTo&#96;, &#96;wouldCycle&#96;, &#96;resolveContent&#96;, &#96;expressionErrors&#96;, &#96;version&#96;, &#96;touch&#96;, &#96;worldMatrix&#96;, &#96;localMatrix&#96;, &#96;transformParentMatrix&#96;, &#96;mul&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A672 | ModelAPI: Preserve extension-facing contract: &#96;P&#96;, &#96;CH&#96;, &#96;KF&#96;, &#96;BLENDS&#96;, &#96;TYPE_META&#96;, &#96;MASK_SHAPES&#96;, &#96;mkLayer&#96;, &#96;mkMask&#96;, &#96;mkProject&#96;, &#96;cloneLayer&#96;, &#96;normalizeFill&#96;, &#96;layerDefinition&#96;, &#96;curComp&#96;, &#96;layer&#96;, &#96;byName&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A673 | SelectionAPI: Preserve extension-facing contract: &#96;get&#96;, &#96;layers&#96;, &#96;first&#96;, &#96;keys&#96;, &#96;chan&#96;, &#96;set&#96;, &#96;select&#96;, &#96;resolveSelectedKeys&#96;, &#96;keySelectionActive&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A674 | GroupsAPI: Preserve extension-facing contract: &#96;ancestors&#96;, &#96;transformRoots&#96;, &#96;bounds&#96;, &#96;span&#96;, &#96;expand&#96;, &#96;normalizeStack&#96;, &#96;moveToGroup&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A675 | TransportAPI: Preserve extension-facing contract: &#96;time&#96;, &#96;setTime&#96;, &#96;play&#96;, &#96;pause&#96;, &#96;toggle&#96;, &#96;playing&#96;, &#96;step&#96;, &#96;quality&#96;, &#96;perf&#96;, &#96;invalidate&#96;, &#96;previewResolution&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A676 | HistoryAPI: Preserve extension-facing contract: &#96;do&#96;, &#96;begin&#96;, &#96;commit&#96;, &#96;cancel&#96;, &#96;undo&#96;, &#96;redo&#96;, &#96;external&#96;, &#96;selection&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A677 | EditAPI: Preserve extension-facing contract: &#96;apply&#96;, &#96;begin&#96;, &#96;commit&#96;, &#96;cancel&#96;, &#96;dispatch&#96;, &#96;mutate&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A678 | InspectorAPI: Preserve extension-facing contract: &#96;registerSection&#96;, &#96;sections&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A679 | MediaAPI: Preserve extension-facing contract: &#96;registerImportDefaults&#96;, &#96;getImportDefaults&#96;, &#96;timing&#96;, &#96;importFiles&#96;, &#96;commandForAsset&#96;, &#96;audio&#96;, &#96;assets&#96;, &#96;fonts&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A680 | RenderAPI includes gl, raster, textLayout, renderFrameTo, snapshot AND shipped async renderFrameToAsync, snapshotAsync, rasterAsync. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A681 | UIStateAPI: Preserve extension-facing contract: &#96;timelineVersion&#96;, &#96;getLayerCollapsed&#96;, &#96;setLayerCollapsed&#96;, &#96;getGroupCollapsed&#96;, &#96;setGroupCollapsed&#96;, &#96;getKeyHandles&#96;, &#96;setKeyHandles&#96;, &#96;getFxOpen&#96;, &#96;setFxOpen&#96;, &#96;getReveal&#96;, &#96;setReveal&#96;, &#96;getShaderMeta&#96;, &#96;setShaderMeta&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A682 | ControlsAPI: Preserve extension-facing contract: &#96;Segmented&#96;, &#96;NumField&#96;, &#96;ColorField&#96;, &#96;FillField&#96;, &#96;RampField&#96;, &#96;PointField&#96;, &#96;SliderField&#96;, &#96;Disclosure&#96;, &#96;FontField&#96;, &#96;SelectField&#96;, &#96;TextField&#96;, &#96;ToggleField&#96;, &#96;Row&#96;, &#96;Section&#96;, &#96;binding&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A683 | UIAPI: Preserve extension-facing contract: &#96;controls&#96;, &#96;toast&#96;, &#96;confirm&#96;, &#96;openExternal&#96;, &#96;copy&#96;, &#96;menu&#96;, &#96;modal&#96;, &#96;icon&#96;, &#96;drag&#96;, &#96;closeMenus&#96;, &#96;showLayerMenu&#96;, &#96;showParentMenu&#96;, &#96;beginParentPick&#96;, &#96;openShaderEditor&#96;, &#96;gesture&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A684 | DndAPI: Preserve extension-facing contract: &#96;ASSET_MIME&#96;, &#96;FX_MIME&#96;, &#96;startAssetDrag&#96;, &#96;mediaDrag&#96;, &#96;hasAssetDrag&#96;, &#96;hasFileDrag&#96;, &#96;hasMediaDrag&#96;, &#96;readAssetDrag&#96;, &#96;hasFxDrag&#96;, &#96;readFxDrag&#96;, &#96;applyFxDrop&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A685 | WorkspaceAPI: Preserve extension-facing contract: &#96;current&#96;, &#96;mutate&#96;, &#96;hasPanel&#96;, &#96;addPanel&#96;, &#96;movePanel&#96;, &#96;removePanel&#96;, &#96;hidePanel&#96;, &#96;restorePanel&#96;, &#96;refresh&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A686 | UtilAPI: Preserve extension-facing contract: &#96;round&#96;, &#96;clamp&#96;, &#96;lerp&#96;, &#96;snapF&#96;, &#96;tc&#96;, &#96;parseTc&#96;, &#96;uid&#96;, &#96;hex2rgb&#96;, &#96;rgb2hex&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A687 | EaseAPI is data in the API definition; preserve its exposed curves/functions, not a fictitious empty member list. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A688 | Space3DAPI: Preserve extension-facing contract: &#96;CHANNELS_3D&#96;, &#96;local3D&#96;, &#96;parent3D&#96;, &#96;world3D&#96;, &#96;is3DLayer&#96;, &#96;perspectiveAmount&#96;, &#96;planeMatrix&#96;, &#96;projectPoint&#96;, &#96;inversePlane&#96;, &#96;planeContains&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A689 | ServicesAPI: Preserve extension-facing contract: &#96;register&#96;, &#96;get&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A690 | StorageAPI: Preserve extension-facing contract: &#96;get&#96;, &#96;set&#96;, &#96;delete&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A691 | EventsAPI: Preserve extension-facing contract: &#96;on&#96;, &#96;emit&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A692 | ExtensionsAPI: Preserve extension-facing contract: &#96;list&#96;, &#96;fork&#96;, &#96;setEnabled&#96;, &#96;remove&#96;, &#96;reload&#96;, &#96;reveal&#96;, &#96;requestFix&#96;, &#96;rebase&#96;, &#96;setUp&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A693 | VarsAPI: Preserve extension-facing contract: &#96;get&#96;, &#96;has&#96;, &#96;keys&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A694 | HostAPI: Preserve extension-facing contract: &#96;pm&#96;, &#96;state&#96;, &#96;mount&#96;, &#96;haptic&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A695 | Contribution locations: titlebar:right, panel:context, layer:context, timeline:context and viewer:context accept synchronous or asynchronous menu contributions. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A696 | Inspector contribution: Register conditional sections after content/transform/effects with build and disposal lifecycle. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A697 | Panel metadata includes component/build/header, size/min, flush/noscroll/headless, move-handle/slot, icon/library metadata; fitContent is post-tag. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A698 | Status contribution: Register left/right computed text items with title and click handler. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A699 | Svelte compatibility: Retain Svelte 5 component mount/reactivity and DOM build contracts; a Rust-only reimplementation cannot execute third-party panels without a compatibility runtime. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A700 | Services and hooks: Preserve owned service override registry and typed event subscriptions/emits; sandbox custom events are namespaced. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| A701 | Trusted host bridge: Existing trusted built-ins/forks can access host.pm, state, Svelte mount and alignment haptics; compatibility includes these escape hatches. | `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts` |
| B084 | index contract: Reattaches the renderer canvas after a module update with a new service facade. | `v1.2.0:apps/desktop/src/extensions/viewer/index.test.ts:L81` |
| B085 | index contract: Does not repeat hover hit testing while a viewer drag owns the cursor. | `v1.2.0:apps/desktop/src/extensions/viewer/index.test.ts:L234` |
| B086 | index contract: Keeps the original WebGL host when a panel rebuild attempts to attach a second stage. | `v1.2.0:apps/desktop/src/extensions/viewer/index.test.ts:L250` |
| B087 | index contract: Disposes listeners and rebinds replacement closures to the same live WebGL stage. | `v1.2.0:apps/desktop/src/extensions/viewer/index.test.ts:L266` |
| B088 | index contract: Replaces typed event subscriptions without installing duplicate stage listeners. | `v1.2.0:apps/desktop/src/extensions/viewer/index.test.ts:L299` |
| B089 | path-editing contract: Removes a newly drawn mask and restores the active path on cancellation. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.test.ts:L81` |

### W041 — Download and manage local speech models

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Available catalog with word- and phrase-timed models and unsupported-host response.
**Steps:** Filter language; inspect/download/cancel/remove/select; generate captions requiring words; dismiss request; continue download past onboarding.
**Expected:** Local service used, accurate language/model capability, no project change on dismiss and no late result in an ended agent run.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/captions/generate.ts`, `v1.2.0:apps/desktop/src/main/transcription/service.ts`, `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`.
**Existing shipped e2e:** `media-intelligence.spec.ts`; `transcription.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A719 | Transcription catalog: Show downloaded/available models with language filtering, model detail meters and download state/progress. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A720 | Model lifecycle: Download, select/use, cancel and remove local models; report unavailability on unsupported hosts. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A721 | Spoken language: Choose detection or supported explicit language; English-only models expose English rather than false autodetection. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A722 | Model storage: Show downloaded-model usage and local storage controls. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A723 | Word timestamps: Caption generation asks for a word-timed model when active model only supplies phrase timestamps. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A724 | Model request sheet: Focused editor presents model picker with reason for agent/caption requests; dismissing must not alter project. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A725 | Transcription engine: Run transcription on this Mac in utility process; agent and captions use shared service/cache. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A726 | Caption file settings: Retain SRT/WebVTT import/export and caption generation model/language choices; detailed caption editing is companion inventory. | `v1.2.0:apps/desktop/src/renderer/src/transcription/TranscriptionSettings.svelte`, `v1.2.0:apps/desktop/src/renderer/src/transcription/ModelSheet.svelte`, `v1.2.0:apps/desktop/src/main/transcription/service.ts` |
| A741 | Acceptance transcription: The download sheet downloads a model, closes itself, and Settings manages it. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L94` |
| A742 | Acceptance transcription: Captions ask for a word-timed model when the one in use only times phrases. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L199` |
| A743 | Acceptance transcription: Main asks the focused editor for a model with a reason. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L270` |
| A744 | Acceptance transcription: A real Parakeet Unified EN model transcribes speech in the utility process. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L281` |
| A745 | Acceptance transcription: Onboarding starts the chosen model download and it finishes after onboarding. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L327` |
| B456 | transcription contract: Captions ask for a word-timed model when the one in use only times phrases. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L199` |
| B457 | transcription contract: Main asks the focused editor for a model with a reason. | `v1.2.0:apps/desktop/e2e/transcription.spec.ts:L270` |

### W042 — Apply appearance and themes without losing customization

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** System/light/dark modes, two themes and later user appearance changes.
**Steps:** Activate; change system appearance; reload/disable/remove; switch workspace and relaunch; inspect extension views.
**Expected:** Owned CSS/tokens/root attributes reverse completely without removing later user choices; authored theme content matches visual oracle. Shell appearance may translate to native tokens.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts`, `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts`, `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/theme.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/theme-apply.ts`.
**Existing shipped e2e:** `theme-lifecycle.spec.ts`; `theme-toggle.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-customization-corpus.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A657 | Theme cleanup: Disable/remove/reload fully reverses owned CSS/tokens/root attributes/workspace appearance while preserving later user changes. | `v1.2.0:apps/desktop/src/main/extensions/index.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/loader.ts` |
| A735 | System/light/dark appearance propagates through renderer and native window material; system-accent.ts is later. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/main/index.ts` |
| A736 | Preserve shipped public CSS token semantics and theme-authored content; main token/font changes are not the frozen tag. | `v1.2.0:apps/desktop/src/main/theme.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/theme-apply.ts` |
| A738 | Theme-owned attributes/CSS/workspace appearance fully unapply across reload, workspace switch and restart; system-accent.ts is not the tag implementation. | `v1.2.0:apps/desktop/e2e/theme-lifecycle.spec.ts` |
| A739 | Acceptance theme-lifecycle: Theme appearance fully un-applies across workspaces, toggles, reload and restart. | `v1.2.0:apps/desktop/e2e/theme-lifecycle.spec.ts:L5` |
| A740 | Acceptance theme-toggle: System appearance repaints the timeline and respects an explicit preference. | `v1.2.0:apps/desktop/e2e/theme-toggle.spec.ts:L50` |
| A746 | Token family bg: Keep public names and light/dark or shared values: &#96;--bg-window&#96;, &#96;--bg-rail&#96;, &#96;--bg-panel&#96;, &#96;--bg-panel-2&#96;, &#96;--bg-sunken&#96;, &#96;--bg-row&#96;, &#96;--bg-row-hi&#96;, &#96;--bg-field&#96;, &#96;--bg-float&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A747 | Token family tx: Keep public names and light/dark or shared values: &#96;--tx&#96;, &#96;--tx-2&#96;, &#96;--tx-3&#96;, &#96;--tx-4&#96;, &#96;--tx-label&#96;, &#96;--tx-value&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A748 | Token family accent: Keep public names and light/dark or shared values: &#96;--accent&#96;, &#96;--accent-hover&#96;, &#96;--accent-dim&#96;, &#96;--accent-tx&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A749 | Token family line: Keep public names and light/dark or shared values: &#96;--line&#96;, &#96;--line-2&#96;, &#96;--line-strong&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A750 | Token family r-: Keep public names and light/dark or shared values: &#96;--r-base&#96;, &#96;--r-xs&#96;, &#96;--r-sm&#96;, &#96;--r-md&#96;, &#96;--r-lg&#96;, &#96;--r-xl&#96;, &#96;--r-pill&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A751 | Token family fs: Keep public names and light/dark or shared values: &#96;--fs-xs&#96;, &#96;--fs-sm&#96;, &#96;--fs-md&#96;, &#96;--fs-lg&#96;, &#96;--fs-xl&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A752 | Token family f-: Keep public names and light/dark or shared values: &#96;--f-ui&#96;, &#96;--f-mono&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A753 | Token family dur: Keep public names and light/dark or shared values: &#96;--dur-1&#96;, &#96;--dur-2&#96;, &#96;--dur-3&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A754 | Token family ease: Keep public names and light/dark or shared values: &#96;--ease&#96;, &#96;--ease-io&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| A755 | Token family shadow: Keep public names and light/dark or shared values: &#96;--shadow-raise&#96;, &#96;--shadow-panel&#96;, &#96;--shadow-lift&#96;, &#96;--shadow-float&#96;. | `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |

### W043 — Navigate with keyboard, text services and accessibility preferences

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Keyboard/assistive navigation with reduced motion, transparency and contrast preferences.
**Steps:** Navigate dialogs/controls; inspect names and states; use native selection services; toggle accessibility preferences during motion.
**Expected:** Reachable named controls, visible focus and proper text ownership; motion stops promptly. Native controls may replace web chrome.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/text-context-menu.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/keychord.ts`, `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`.
**Existing shipped e2e:** `animation-cleanup.spec.ts`; `button-pointer-focus.spec.ts`; `native-panel-text.spec.ts`; `native-text-selection-menu.spec.ts`; `panel-text-entry.spec.ts`; `settings-motion.spec.ts`; `settings-scroll.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A731 | Accessible names: Preserve semantic dialogs, buttons, navigation/regions, selection/pressed/current state and progressbar values with meaningful labels. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A732 | Keyboard focus: Maintain visible focus, tab order, modal focus restoration and native text services without editor shortcut leakage. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A733 | Reduced motion: Respect prefers-reduced-motion in settings travel/library/agent motion; stop animations promptly when preference changes. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| A734 | Localization baseline: UI strings are predominantly English; locale-aware dates/formatting exist, but no translation catalog/language preference is established. | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`, `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| B535 | button-pointer-focus contract: Clicking a button does not leave focus or assign Space to its action. | `v1.2.0:apps/desktop/e2e/button-pointer-focus.spec.ts:L3` |
| B538 | panel-text-entry contract: Command+V pastes text into a panel search field without pasting layers. | `v1.2.0:apps/desktop/e2e/panel-text-entry.spec.ts:L5` |

### W044 — Receive completion notifications, warnings and alignment haptics

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Finished run, agent question, slow extension and snapping gesture.
**Steps:** Complete/question a run; activate notification; trigger snap repeatedly; fail optional haptic provider; disable a slow extension.
**Expected:** Relevant work is revealed, feedback is paced, warning actionable and optional device failures do not stop editing.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/main/index.ts`, `v1.2.0:apps/desktop/src/main/haptics.ts`, `v1.2.0:apps/desktop/src/main/agent-notifications.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/notification-preferences.ts`.
**Existing shipped e2e:** `performance-warning.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A727 | Alignment haptic: Use native macOS alignment feedback for snap/alignment events, trusted senders only, at least 80 ms apart. | `v1.2.0:apps/desktop/src/main/haptics.ts` |
| A728 | Optional hardware: Haptic failures never interrupt gestures; native addon is optional where unavailable. | `v1.2.0:apps/desktop/src/main/haptics.ts` |
| A729 | Agent completion: Play bundled Little Victory (Deep) notification sound for completed runs. | `v1.2.0:apps/desktop/src/main/agent-notifications.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/notification-preferences.ts` |
| A730 | Agent questions: Send notification with the actual question and completion sound; native notification activation must return to relevant work. | `v1.2.0:apps/desktop/src/main/agent-notifications.ts`, `v1.2.0:apps/desktop/src/renderer/src/panels/agent/notification-preferences.ts` |

### W045 — Navigate zoom, pan, fit and recover the composition view

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Small and oversized compositions, fractional Retina zoom and a selected layer.
**Steps:** Wheel/pinch/hand/middle-drag; anchor zoom at cursor; choose Fit/actual/custom presets; pan fully away and recover; resize/reload.
**Expected:** Stable cursor anchor, last frame and correct visible pixels; no document edits; bounded allocation at edges.
**Exact registered keymap entries:** K081 cmd+= → zoomIn; K082 ctrl+= → zoomIn; K083 cmd+shift++ → zoomIn; K084 ctrl+shift++ → zoomIn; K085 cmd+- → zoomOut; K086 ctrl+- → zoomOut; K087 cmd+0 → fitComposition; K088 ctrl+0 → fitComposition; K089 cmd+1 → actualSize; K090 ctrl+1 → actualSize; K145 . → zoomIn; K146 , → zoomOut; K147 / → actualSize; K148 shift+/ → fitComposition; K156 h → toolHand; K157 shift+h → toolHand; K158 z → toolZoom; K159 shift+z → toolZoom.
**Native menu key equivalents:** M001 CommandOrControl+= → zoomIn; M002 CommandOrControl+- → zoomOut; M003 CommandOrControl+1 → actualSize; M004 CommandOrControl+0 → fitComposition.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `effect-zoom.spec.ts`; `high-zoom-performance.spec.ts`; `pan-presentation.spec.ts`; `region-pan-performance.spec.ts`; `timeline-keyframe-navigation.spec.ts`; `viewer-recovery.spec.ts`; `viewer-tools.spec.ts`; `zoom-gestures.spec.ts`; `zoom-performance-benchmark.spec.ts`; `zoom-raster.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A187 | Cmd/Ctrl+=: Run &#96;zoomIn&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A188 | Cmd/Ctrl+shift++: Run &#96;zoomIn&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A189 | Cmd/Ctrl+-: Run &#96;zoomOut&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A190 | Cmd/Ctrl+0: Run &#96;fitComposition&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A191 | Cmd/Ctrl+1: Run &#96;actualSize&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A208 | h / Shift+h: Run &#96;toolHand&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A209 | z / Shift+z: Run &#96;toolZoom&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A235 | .: Run &#96;zoomIn&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A236 | ,: Run &#96;zoomOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A237 | /: Run &#96;actualSize&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A238 | shift+/: Run &#96;fitComposition&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B001 | Tag viewer: mouse wheel zoom, trackpad pan, pointer-centered pinch, Shift horizontal redirect. Keep authored navigation goal; web event mechanics may change. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B002 | Zoom tool: Z selects; click magnifies 1.25×, Option-click reduces to 0.8×; drag a rectangle to frame it. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B003 | Fit: Cmd+0 or Shift+/ fits composition; Cmd+1 or / selects actual size; . and , zoom in/out. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B004 | Hand: H selects Hand; middle-button drag pans; temporary Hand preserves the chosen tool. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B020 | Lost canvas recovery: show one-click return only after explicit pan fully loses the composition. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B021 | Zoom continuity: preserve point beneath cursor and last frame while resizing the drawing buffer. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B054 | pan-presentation contract: High-zoom pans move the presented picture before another render is available. | `v1.2.0:apps/desktop/e2e/pan-presentation.spec.ts:L5` |
| B055 | pan-presentation contract: Paused zoom refines after settling and content or time changes bypass deferral. | `v1.2.0:apps/desktop/e2e/pan-presentation.spec.ts:L60` |
| B056 | pan-presentation contract: Fractional pans reuse covered pixels while crop changes and independent redraws still render. | `v1.2.0:apps/desktop/e2e/pan-presentation.spec.ts:L104` |
| B057 | pan-presentation contract: Native outward pans keep GPU allocation stable at every composition edge. | `v1.2.0:apps/desktop/e2e/pan-presentation.spec.ts:L154` |
| B066 | viewer-recovery contract: Keeps rendered pixels on the visible canvas across extension reloads. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L18` |
| B067 | viewer-recovery contract: Offers a one-click return only after explicit panning fully loses the composition. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L48` |
| B068 | viewer-recovery contract: Keeps the same composition point under the cursor through small-to-oversized zoom. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L75` |
| B069 | viewer-recovery contract: Keeps the last composition frame visible while zoom resizes its drawing buffer. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L128` |
| B070 | viewer-recovery contract: Zooms with a mouse wheel, pans with trackpad scroll, pinches to zoom, and middle-drags. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L172` |
| B071 | viewer-recovery contract: Double-clicks selected text through a full-frame top layer and selects all its source. | `v1.2.0:apps/desktop/e2e/viewer-recovery.spec.ts:L222` |
| B109 | Preview resolution: Auto, Full, Half and Quarter select preview scale; change relayouts viewer and reflects current transport quality. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B110 | Zoom menu: Fit plus 5/12.5/25/50/100/200/400/800%; custom zoom displays current percentage; clamp displayed zoom 2–800%. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B115 | Viewer overlay coverage: dedicated ruler, user-guide, safe-area and grid toggles were not located in committed viewer sources; preserve actual alignment/anchor/path overlays rather than assume AE features exist. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B116 | Preview Auto/Full/Half/Quarter exists in viewer.ts; no per-source proxy-status badge. viewer-chrome.ts is not a tag path. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |

### W046 — Select and move canvas layers and groups

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Overlapping layers, nested groups, a common selection box and locked members.
**Steps:** Hover/click; deep Cmd/Ctrl pick; Shift toggle; marquee; drag common box or covered selected layer; cancel and undo.
**Expected:** Resolved targets, group boundaries and locks correct; one completed gesture = one Undo; cancelled gesture restores state.
**Exact registered keymap entries:** K033 cmd+a → selectAll; K034 ctrl+a → selectAll; K035 cmd+shift+a → deselect; K036 ctrl+shift+a → deselect; K067 cmd+shift+h → toggleLayerControls; K068 ctrl+shift+h → toggleLayerControls; K069 cmd+l → lockSelectedLayers; K070 ctrl+l → lockSelectedLayers; K071 cmd+shift+l → unlockAllLayers; K072 ctrl+shift+l → unlockAllLayers; K073 cmd+up → selectPreviousLayer; K074 ctrl+up → selectPreviousLayer; K075 cmd+down → selectNextLayer; K076 ctrl+down → selectNextLayer; K077 cmd+shift+up → extendSelectionPreviousLayer; K078 ctrl+shift+up → extendSelectionPreviousLayer; K079 cmd+shift+down → extendSelectionNextLayer; K080 ctrl+shift+down → extendSelectionNextLayer; K101 left → nudgeSelection [-1, 0]; K102 shift+left → nudgeSelection [-10, 0]; K103 right → nudgeSelection [1, 0]; K104 shift+right → nudgeSelection [10, 0]; K105 up → nudgeSelection [0, -1]; K106 shift+up → nudgeSelection [0, -10]; K107 down → nudgeSelection [0, 1]; K108 shift+down → nudgeSelection [0, 10]; K122 escape → deselect; K154 v → toolSelect; K155 shift+v → toolSelect.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `pro-editor-shortcuts.spec.ts`; `user-feedback.spec.ts`; `viewer-selection.spec.ts`; `viewer-tools.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A163 | Cmd/Ctrl+a: Run &#96;selectAll&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A164 | Cmd/Ctrl+shift+a: Run &#96;deselect&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A180 | Cmd/Ctrl+shift+h: Run &#96;toggleLayerControls&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A181 | Cmd/Ctrl+l: Run &#96;lockSelectedLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A182 | Cmd/Ctrl+shift+l: Run &#96;unlockAllLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A183 | Cmd/Ctrl+up: Run &#96;selectPreviousLayer&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A184 | Cmd/Ctrl+down: Run &#96;selectNextLayer&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A185 | Cmd/Ctrl+shift+up: Run &#96;extendSelectionPreviousLayer&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A186 | Cmd/Ctrl+shift+down: Run &#96;extendSelectionNextLayer&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A206 | escape: Run &#96;deselect&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A207 | v / Shift+v: Run &#96;toolSelect&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B005 | Canvas picking: ordinary click selects the group; Cmd/Ctrl-click picks deeper; hover previews the resolved target. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B006 | Additive picking: Shift-click toggles selected targets; empty unmodified click clears selection. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B007 | Marquee: drag empty canvas to select enclosed layers; Shift toggles the enclosed set. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B008 | Common selection box: drag empty space inside the common box to move the selection as one gesture. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B009 | Locked selection members: keep locked members in common chrome; do not transform only the unlocked subset. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B016 | Canvas drag cancellation: Escape restores original state; pointer release commits one undo operation. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B076 | viewer-selection contract: Drags the timeline-selected layer beneath a full-frame top layer. | `v1.2.0:apps/desktop/e2e/viewer-selection.spec.ts:L4` |
| B077 | viewer-selection contract: Moves a multi-selection from the empty gap inside its common box as one undoable gesture. | `v1.2.0:apps/desktop/e2e/viewer-selection.spec.ts:L69` |
| B078 | viewer-selection contract: Keeps locked members in common chrome without transforming the unlocked subset. | `v1.2.0:apps/desktop/e2e/viewer-selection.spec.ts:L143` |
| B079 | viewer-selection contract: Clicks select groups first, Cmd goes deeper, and hover previews the target. | `v1.2.0:apps/desktop/e2e/viewer-selection.spec.ts:L209` |
| B518 | Layer selection: Cmd+A selects all; Cmd+Shift+A deselects; Cmd+Up/Down selects previous/next; Shift extends. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B529 | Layer controls: Cmd+Shift+H toggles canvas layer controls. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B530 | Layer lock: Cmd+L locks selection; Cmd+Shift+L unlocks all. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B533 | Delete ownership: Backspace/Delete removes context selection; selected keyframes take precedence over layer deletion. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W047 — Resize, rotate, align and change anchors

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Parented/rotated shapes, text, media and multi-selection.
**Steps:** Resize edges/corners from local opposite corner or center; constrain aspect; rotate with angle modifiers; drag/center anchor; align union to composition; cancel and undo.
**Expected:** Exact geometry and hit bounds; text box resizing does not change font/scale; parent transforms and common pivot correct; locked members protected.
**Exact registered keymap entries:** K091 cmd+alt+home → centerAnchor; K092 ctrl+alt+home → centerAnchor; K160 w → toolRotate; K161 shift+w → toolRotate; K162 y → toolAnchor; K163 shift+y → toolAnchor.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/align.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `font-anchor.spec.ts`; `group-transforms.spec.ts`; `scale-aspect-ratio.spec.ts`; `size-anchor.spec.ts`; `text-layout-resize.spec.ts`; `viewer-alignment.spec.ts`; `viewer-anchor-indicators.spec.ts`; `viewer-resize.spec.ts`; `viewer-tools.spec.ts`.
**Later candidate e2e (not shipped evidence):** `history-gesture-press.spec.ts`; `native-edit-kernels.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A192 | Cmd/Ctrl+alt+home: Run &#96;centerAnchor&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A210 | w / Shift+w: Run &#96;toolRotate&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A211 | y / Shift+y: Run &#96;toolAnchor&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B010 | Resize: opposite edge/corner stays fixed in local space; Option resizes from center; Shift locks aspect; image/video lock aspect by default. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B011 | Text resize: resize paragraph width/box without rewriting font Size or layer Scale X/Y. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B012 | Rotation: W selects rotation; Shift quantizes 15°, Cmd/Ctrl quantizes 45°; common selection rotates around common pivot. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B013 | Anchor: Y selects Pan Behind; double-click toolbar centers anchor; Cmd+Option+Home centers anchor. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B014 | Anchor drag: ordinary drag preserves layer appearance; Option moves the layer; Cmd/Ctrl enables anchor snapping. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B060 | size-anchor contract: Resized shapes render and hit-test from their anchor in preview and export. | `v1.2.0:apps/desktop/e2e/size-anchor.spec.ts:L3` |
| B063 | viewer-anchor-indicators contract: Selected layers show their transformed anchors at a constant screen size. | `v1.2.0:apps/desktop/e2e/viewer-anchor-indicators.spec.ts:L3` |
| B064 | viewer-anchor-indicators contract: Clicking away commits canvas text and returns to Selection while explicit tool choices win. | `v1.2.0:apps/desktop/e2e/viewer-anchor-indicators.spec.ts:L79` |
| B072 | viewer-resize contract: Resizes a shape parent and its selected child together around the common fixed corner. | `v1.2.0:apps/desktop/e2e/viewer-resize.spec.ts:L4` |
| B073 | viewer-resize contract: Resizes the selected layer in local space with a fixed opposite edge and correct cursors. | `v1.2.0:apps/desktop/e2e/viewer-resize.spec.ts:L76` |
| B074 | viewer-resize contract: Resizes the text box without changing typography Size or Scale X/Y. | `v1.2.0:apps/desktop/e2e/viewer-resize.spec.ts:L203` |
| B075 | viewer-resize contract: Resizes and rotates a multi-selection around the common pivot with one-step undo. | `v1.2.0:apps/desktop/e2e/viewer-resize.spec.ts:L292` |
| B385 | align contract: Moves a box so the chosen edge or center meets the frame. | `v1.2.0:apps/desktop/src/extensions/inspector/align.test.ts:L9` |
| B386 | align contract: Aligns a multi-selection by moving its union bounds against the composition. | `v1.2.0:apps/desktop/src/extensions/inspector/align.test.ts:L18` |
| B387 | align contract: Expresses a world delta in a scaled or rotated parent space. | `v1.2.0:apps/desktop/src/extensions/inspector/align.test.ts:L28` |

### W048 — Draw and edit vector shapes, paths and masks

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Empty comp plus existing transformed/animated shape and mask paths.
**Steps:** Draw constrained/from-center shapes; add/tangent/edit/delete Pen vertices; close path; split tangent; edit through nested transforms; cancel and undo.
**Expected:** Correct path or mask target, constant-size handles, layer-local animation time and locked-ancestor protection.
**Exact registered keymap entries:** K011 cmd+y → newSolid; K012 ctrl+y → newSolid; K013 cmd+shift+y → newShape; K014 ctrl+shift+y → newShape; K059 cmd+alt+y → newAdjustment; K060 ctrl+alt+y → newAdjustment; K164 q → toolShape; K165 shift+q → toolShape; K166 g → toolPen; K167 shift+g → toolPen.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`; `agent-path-mask.spec.ts`; `figma-paste.spec.ts`; `import-svg.spec.ts`; `pen-group-locks.spec.ts`; `viewer-tools.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A152 | Cmd/Ctrl+y: Run &#96;newSolid&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A153 | Cmd/Ctrl+shift+y: Run &#96;newShape&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A176 | Cmd/Ctrl+alt+y: Run &#96;newAdjustment&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A212 | q / Shift+q: Run &#96;toolShape&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A213 | g / Shift+g: Run &#96;toolPen&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A451 | Acceptance import-svg: @viewer imports SVG as durable editable Shape paths and preserves its source. | `v1.2.0:apps/desktop/e2e/import-svg.spec.ts:L7` |
| B015 | Shape drawing: Q cycles rectangle, rounded rectangle, ellipse, polygon and star; Shift constrains; Option draws from center. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B022 | Pen vertices: G selects Pen; click adds vertex; drag creates mirrored incoming/outgoing tangents. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B023 | Path closure: double-click closes path; clicking first vertex closes an active path with at least three vertices. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B024 | Delete vertex: Cmd/Ctrl-click a hit vertex deletes it with Undo. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B025 | Independent tangents: Option-drag splits tangent behavior; ordinary tangent drag mirrors the opposite tangent. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B026 | Path targets: shape layers receive vector paths; supported non-shape layers receive mask paths; otherwise create a Path layer. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B027 | Path transforms: hit-test and drag vertices in inverse layer and nested path-group transforms. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B028 | Animated vertices: editing animated coordinates writes keys at layer-local playhead time. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B029 | Path overlay: draw vertices, tangents and handles at constant screen size while Pen is active. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B030 | Path locks: refuse edits through locked ancestor groups. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B031 | Path cancellation: restore original paths, masks and active path; refresh inspector on completion/cancel. | `v1.2.0:apps/desktop/src/extensions/viewer/path-editing.ts` |
| B058 | pen-group-locks contract: Pen edits protect nested locked groups and work after unlocking with one Undo. | `v1.2.0:apps/desktop/e2e/pen-group-locks.spec.ts:L3` |
| B080 | viewer-tools contract: Keeps tool buttons clear of native window drag regions after resizing. | `v1.2.0:apps/desktop/e2e/viewer-tools.spec.ts:L28` |
| B081 | viewer-tools contract: Keeps tools distinct from creation commands and draws an undoable shape. | `v1.2.0:apps/desktop/e2e/viewer-tools.spec.ts:L52` |
| B082 | viewer-tools contract: Marquee-selects enclosed layers and Shift toggles the enclosed set. | `v1.2.0:apps/desktop/e2e/viewer-tools.spec.ts:L103` |
| B083 | viewer-tools contract: Rotation, Pan Behind, Type, Zoom, and temporary Hand use professional gesture semantics. | `v1.2.0:apps/desktop/e2e/viewer-tools.spec.ts:L140` |
| B448 | import-svg contract: @viewer imports SVG as durable editable Shape paths and preserves its source. | `v1.2.0:apps/desktop/e2e/import-svg.spec.ts:L7` |

### W049 — Create and edit point and paragraph text on canvas

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Existing, locked and covered text plus an empty comp.
**Steps:** Type-tool click/drag; type/select/multiline/paste; click away or Escape/Cmd+Enter; delete all text; save during edit.
**Expected:** Live authored text layout and selection; complete edit is one Undo; empty fresh layer leaves no entry; correct explicit tool precedence.
**Exact registered keymap entries:** K015 cmd+t → toolText; K016 ctrl+t → toolText; K017 cmd+shift+t → toolText; K018 ctrl+shift+t → toolText; K195 enter → text.editSelected.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/index.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`; `canvas-text.spec.ts`; `text-color-drag.spec.ts`; `text-layout-resize.spec.ts`; `text-raster-efficiency.spec.ts`; `text-render-parity.spec.ts`; `typewriter-raster.spec.ts`; `viewer-anchor-indicators.spec.ts`.
**Later candidate e2e (not shipped evidence):** `native-engine-text-app-parity.spec.ts`; `native-engine-text-level2.spec.ts`; `native-engine-text-typing.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A154 | Cmd/Ctrl+t: Run &#96;toolText&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A155 | Cmd/Ctrl+shift+t: Run &#96;toolText&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B032 | Enter text edit: double-click text or Enter on selected text opens in-place editor; selected text remains editable beneath a covering layer. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B033 | Point text: Cmd+T then click creates point text; type live; click away commits and returns to Selection. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B034 | Paragraph text: drag Type tool creates fixed-width paragraph with automatic height. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B035 | Text selection: click moves caret; double-click selects word; Cmd+A selects text inside editor. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B036 | Multiline text: Enter inserts line breaks while editing; clipboard and selection stay local to text editor. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B037 | Empty text: leaving a new text layer empty removes it without a history entry. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B038 | Text history: a complete in-place edit commits as one undo step; save preserves editor focus. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B039 | Text tool precedence: click-away returns to Selection unless another tool was explicitly chosen. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| B045 | canvas-text contract: Locked group text refuses canvas editing and works after unlocking with one Undo. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L44` |
| B046 | canvas-text contract: Edits existing text in place: caret, selection, clipboard, multiline, commit and one-step undo. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L71` |
| B047 | canvas-text contract: Text tool: click creates point text, typing renders live, clicking away commits and returns to Select. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L108` |
| B048 | canvas-text contract: Text tool: leaving a new layer empty removes it without a history entry. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L139` |
| B049 | canvas-text contract: Text tool: dragging creates a fixed-width paragraph with automatic height. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L156` |
| B050 | canvas-text contract: Clicking inside the edited text moves the caret; double-click selects a word. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L176` |
| B051 | canvas-text contract: Enter edits the selected text layer; Command+A selects all inside the editor; focus survives a save. | `v1.2.0:apps/desktop/e2e/canvas-text.spec.ts:L200` |
| B096 | text-editor contract: Opens one transaction, writes live and commits on Escape without losing text. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L71` |
| B097 | text-editor contract: Commits on Command+Enter and lets other shortcuts bubble to the field-aware keymap. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L92` |
| B098 | text-editor contract: Removes a fresh layer with no history when it ends empty, and names a kept one after its text. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L109` |
| B099 | text-editor contract: Deletes an existing layer whose text was emptied. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L126` |
| B100 | text-editor contract: Returns the Text tool to Select when the session ends. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L135` |
| B101 | text-editor contract: Places the caret from a click, selects words on double click and drags a range. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L144` |
| B102 | text-editor contract: Selects everything on entry when asked and mirrors external text changes. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L164` |
| B103 | text-editor contract: Commits on clicks outside the viewer but keeps editing through the inspector. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L176` |
| B104 | text-editor contract: Commits before an inspector value field starts its own transaction. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L194` |
| B105 | text-editor contract: Closes a transaction left open elsewhere instead of failing to start. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L205` |
| B106 | text-editor contract: Handles Command+A itself and offers style shortcuts. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L213` |
| B107 | text-editor contract: Takes focus back when nothing else holds it. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts:L235` |
| B108 | text-editor contract: Refuses locked layers. | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.test.ts` |

### W050 — Build and navigate editable 2.5D/3D scenes

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Models with textured material, lights/camera, nested groups and flat 3D children.
**Steps:** Import model bundle; convert; edit material/light/camera/environment and keyframes; use gizmo Global/Local; orbit/pan/dolly/frame; save/export and undo.
**Expected:** Durable assets, precise animated scene pixels, correct parent/occlusion; editor view leaves render camera and export unchanged.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/GizmoToolbar.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts`.
**Existing shipped e2e:** `extension-layers.spec.ts`; `groups-25d-occlusion.spec.ts`; `groups-25d.spec.ts`; `layers-25d.spec.ts`; `scene3d-idle.spec.ts`; `scene3d.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-scene3d-chrome.spec.ts`; `scene3d-budget-parity.spec.ts`; `scene3d-compat-parity.spec.ts`; `scene3d-gltf-round7-parity.spec.ts`; `scene3d-import-parity.spec.ts`; `scene3d-normal-matrix.spec.ts`; `scene3d-short-attributes.spec.ts`; `scene3d-warnings.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A508 | MCP &#96;get_3d_scene&#96;: Read the composition’s individual 3D model, light and camera layers with IDs, geometry, textured PBR materials, camera, environment, and keyframe channel paths. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L63` |
| A509 | MCP &#96;edit_3d&#96;: Create and edit individual 3D model, light and camera layers in the normal timeline. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L68` |
| B040 | Editor camera: orbit, pan, dolly and framing change editor view without changing render camera or saved project. | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| B041 | Orbit gesture: middle button or Option-drag orbits; add Shift to pan. | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| B042 | Fine navigation: Shift slows non-pan navigation to one tenth. | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| B043 | 3D framing: F frames selection; Home frames composition; Escape cancels gesture or returns to selection navigation. | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| B044 | 3D navigation ownership: navigation consumes gestures only when available and navigation tool/Option applies. | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| B111 | 3D transform controls: choose Move/Rotate/Scale; choosing transform returns navigation to Select. | `v1.2.0:apps/desktop/src/extensions/viewer/GizmoToolbar.svelte` |
| B112 | 3D axis space: Global/Local toggle changes transform basis. | `v1.2.0:apps/desktop/src/extensions/viewer/GizmoToolbar.svelte` |
| B113 | 3D viewpoint: Camera returns to rendered shot; Editor exposes editor navigation view. | `v1.2.0:apps/desktop/src/extensions/viewer/GizmoToolbar.svelte` |
| B114 | 3D view tools: Select, Orbit, Pan, Dolly, Frame selection and Frame all available from dropdown. | `v1.2.0:apps/desktop/src/extensions/viewer/GizmoToolbar.svelte` |
| B394 | 3D Color: color field (color); ; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B395 | 3D Roughness: num field (roughness); step: 0.01, min: 0.02, max: 1; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B396 | 3D Metalness: num field (metalness); step: 0.01, min: 0, max: 1; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B397 | 3D Emission: color field (emissive); ; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B398 | 3D Emission strength: num field (emissiveIntensity); step: 0.05, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B399 | 3D Opacity: num field (opacity); step: 0.01, min: 0, max: 1; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B400 | 3D Color: color field (color); ; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B401 | 3D Intensity: num field (intensity); step: 0.05, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B402 | 3D Range: num field (distance); step: 0.1, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B403 | 3D Falloff: num field (decay); step: 0.05, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B404 | 3D Cone angle: num field (angle); step: 1, min: 1, max: 89, unit: '°'; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B405 | 3D Softness: num field (penumbra); step: 0.01, min: 0, max: 1; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B406 | 3D Field of view: num field (fov); step: 0.5, min: 1, max: 150, unit: '°'; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B407 | 3D Zoom: num field (zoom); step: 0.01, min: 0.01; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B408 | 3D Clip start: num field (near); step: 0.01, min: 0.001; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B409 | 3D Clip end: num field (far); step: 1, min: 0.01; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B410 | 3D Ambient: num field (ambient); step: 0.01, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B411 | 3D Ambient color: color field (ambientColor); ; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |
| B412 | 3D Exposure: num field (exposure); step: 0.01, min: 0; evaluated channel supports SceneField animation controls. | `v1.2.0:apps/desktop/src/extensions/layers-3d/scene-model.ts` |

### W051 — Play, pause, step and navigate visible events

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A composition with video/audio, work area, keys, markers and selected layers.
**Steps:** Play/pause; frame-step and hold repeat keys; jump boundaries; J/K and Shift+J/K; scrub across cuts, reverse seeks and loops.
**Expected:** Exact project-frame time and selected-event scope; no wrap for adjacent keys; incoming clip never flashes stale outgoing pixels; audio continues through quality changes.
**Exact registered keymap entries:** K113 space → play; K114 home → gotoStart; K115 end → gotoEnd; K118 pageup → prevFrame; K119 pagedown → nextFrame; K123 shift+pageup → stepFrames [-10]; K124 shift+pagedown → stepFrames [10]; K125 cmd+left → prevFrame; K126 cmd+right → nextFrame; K127 cmd+shift+left → stepFrames [-10]; K128 cmd+shift+right → stepFrames [10]; K129 ctrl+left → timeline.adjacentKeyframe:prev; K130 ctrl+right → timeline.adjacentKeyframe:next; K131 ctrl+shift+left → stepFrames [-10]; K132 ctrl+shift+right → stepFrames [10]; K133 j → prevVisibleEvent; K134 k → nextVisibleEvent; K135 shift+j → timeline.adjacentKeyframe:prev; K136 shift+k → timeline.adjacentKeyframe:next; K116 shift+home → gotoWorkIn; K117 shift+end → gotoWorkOut.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/video-timeline.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `composited-video-sync.spec.ts`; `frame-preparation.spec.ts`; `gif-preview-scrubbing.spec.ts`; `keyframe-paused-frame.spec.ts`; `playback-audio.spec.ts`; `playback-performance-benchmark.spec.ts`; `playback-source-warmup.spec.ts`; `pro-editor-shortcuts.spec.ts`; `project-performance-benchmark.spec.ts`; `project-playback-profile.spec.ts`; `timeline-keyframe-navigation.spec.ts`; `timeline.spec.ts`; `transparent-sequence-scrubbing.spec.ts`; `video-cut-recovery.spec.ts`; `video-frame-integrity.spec.ts`; `video-playback-regression.spec.ts`; `video-preview-io.spec.ts`; `video-scrubbing.spec.ts`; `video-texture-readiness.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-source-timing-end.spec.ts`; `engine-video-end.spec.ts`; `native-engine-transport.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A197 | space: Run &#96;play&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A198 | home: Run &#96;gotoStart&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A199 | end: Run &#96;gotoEnd&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A202 | pageup: Run &#96;prevFrame&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A203 | pagedown: Run &#96;nextFrame&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A220 | shift+pageup: Run &#96;stepFrames&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A221 | shift+pagedown: Run &#96;stepFrames&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A223 | j: Run &#96;prevVisibleEvent&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A224 | k: Run &#96;nextVisibleEvent&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A225 | shift+j: Run &#96;timeline.adjacentKeyframe:prev&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A226 | shift+k: Run &#96;timeline.adjacentKeyframe:next&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B261 | Tag time readout uses api.util.tc at project FPS; main introduces aeTimecode hours:minutes:seconds:frames formatting. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B521 | Playback: Space toggles playback outside text fields, including pointer over viewer. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B522 | Frame step: PageUp/PageDown or Cmd+Left/Right step one frame; Shift variants step ten. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B524 | Visible events: J previous visible event and K next; Shift+J/K previous/next real keyframe; Ctrl+Left/Right also navigate keys. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W052 — Set preview/export in and out points

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Composition and selected layers with known spans.
**Steps:** B/N set work bounds; drag handles; use work-area menu/reset; I/O navigate selected layer bounds; jump with Shift+Home/End.
**Expected:** At least one frame in work area; frame-aligned/clamped ranges; layer I/O navigation does not mark source monitor ranges.
**Exact registered keymap entries:** K116 shift+home → gotoWorkIn; K117 shift+end → gotoWorkOut; K137 i → gotoLayerIn; K138 o → gotoLayerOut; K168 b → workIn; K169 shift+b → workIn; K170 n → workOut; K171 shift+n → workOut.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A200 | shift+home: Run &#96;gotoWorkIn&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A201 | shift+end: Run &#96;gotoWorkOut&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A214 | b / Shift+b: Run &#96;workIn&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A215 | n / Shift+n: Run &#96;workOut&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A227 | i: Run &#96;gotoLayerIn&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A228 | o: Run &#96;gotoLayerOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B133 | Work-area bounds: B sets work in and N work out; handles keep at least one frame span. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B134 | Work-area menu: start/end here or at playhead and reset to entire composition. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B523 | Composition boundaries: Home/End jump start/end; Shift+Home/End jump work in/out. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W053 — Add, persist, undo and remove composition markers

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Known frame rate with empty and occupied marker times.
**Steps:** Right-click ruler; add here; repeat same frame; save/reopen; undo/redo; remove via marker menu.
**Expected:** Single frame-aligned marker, bounds clamped, correct target after delayed menu and persistent IDs/legacy compatibility.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/markers.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `timeline-markers.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B135 | Marker menu: right-click ruler adds marker here; marker context menu removes it; duplicate-time marker disabled. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B167 | timeline-markers contract: Native timeline markers support ruler menus, Undo/Redo and save/reopen. | `v1.2.0:apps/desktop/e2e/timeline-markers.spec.ts:L5` |
| B242 | markers contract: Adds frame-aligned markers without duplicate pins and clamps the composition bounds. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.test.ts:L10` |
| B243 | markers contract: Resolves a marker when its menu runs, including legacy markers without IDs. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.test.ts:L26` |
| B244 | markers contract: Hits only visible ruler pins, leaving work-area handles and track interactions alone. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.test.ts:L41` |
| B245 | markers contract: Draws only visible pins and neutral guides, restoring the canvas state. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.test.ts:L53` |
| B246 | markers contract: Adds and removes through the real edit boundary, persists in .pmv, and restores Undo/Redo. | `v1.2.0:apps/desktop/src/extensions/timeline/markers.test.ts:L63` |

### W054 — Snap playhead, clips and selected keys without changing spacing

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Dense clip edges/keys and selection with multiple eligible edges.
**Steps:** Cross edges in both directions; enable Shift during drag; reverse; move away; remove target; fast drag; Cmd-bypass viewer alignment.
**Expected:** Directional latch only after crossing, stable target and release; rigid selection spacing; no far-backward pull; visible guides clear on release.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/track-view.ts`, `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`.
**Existing shipped e2e:** `directional-snapping.spec.ts`; `viewer-alignment.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B017 | Move snapping: ordinary moves align both axes; Cmd bypasses snapping; show alignment feedback. | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| B052 | directional-snapping contract: Playhead latches only after crossing an edge and releases for fine movement. | `v1.2.0:apps/desktop/e2e/directional-snapping.spec.ts:L39` |
| B061 | viewer-alignment contract: Renders the composition center as full-axis guides with a fixed bullseye. | `v1.2.0:apps/desktop/e2e/viewer-alignment.spec.ts:L11` |
| B062 | viewer-alignment contract: Aligns both axes during an ordinary layer drag and allows Command to bypass snapping. | `v1.2.0:apps/desktop/e2e/viewer-alignment.spec.ts:L53` |
| B119 | Playhead snapping: Shift held or pressed during scrub latches when crossing clip boundaries and keys. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B199 | directional-snap contract: Does not attract a pointer near an edge, and acquires only when crossing it. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L57` |
| B200 | directional-snap contract: Works in both directions and releases immediately when reversing. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L65` |
| B201 | directional-snap contract: Does not reacquire a latch in the sample that reverses across its edge. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L74` |
| B202 | directional-snap contract: Does not snap when toggled at rest or acquire an edge passed while disabled. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L79` |
| B203 | directional-snap contract: Moves a rigid selection from any eligible edge without changing its spacing. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L87` |
| B204 | directional-snap contract: Ignores incompatible edges and never jumps to a distant target crossed by a fast drag. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L94` |
| B205 | directional-snap contract: Keeps the same latch among dense targets and releases a removed target. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L99` |
| B206 | directional-snap contract: Chooses the last crossed target and does not let a weighted target pull backwards. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L106` |
| B207 | directional-snap contract: Does not latch while moving away from an initially aligned edge. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L110` |
| B208 | directional-snap contract: Matches exhaustive snapping through large mixed-edge drags, reversals, toggles and target edits. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L117` |
| B209 | directional-snap contract: Limits target reads for a large selection while choosing the same edge. | `v1.2.0:apps/desktop/src/extensions/timeline/directional-snap.test.ts:L159` |

### W055 — Arrange layers/tracks, disclosures and viewport

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Many grouped/animated layers, several tracks and negative-time strips.
**Steps:** Switch Layers/Tracks; scroll/pan/zoom; expand properties/groups; rename; change switches; move between tracks; cancel; relaunch collapsed state.
**Expected:** Correct stacking, selection and live preview; preserved view state; no playback-time scan of unrelated rows; graph/strip focus survives.
**Exact registered keymap entries:** K192 cmd+&#96; → timeline.revealAll; K193 ctrl+&#96; → timeline.revealAll.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/track-view.ts`.
**Existing shipped e2e:** `agent-project-timeline.spec.ts`; `group-timeline-interactions.spec.ts`; `layer-reorder.spec.ts`; `timeline-gesture-performance.spec.ts`; `timeline-graph-paint-cache.spec.ts`; `timeline-keyframe-rows.spec.ts`; `timeline-layer-rows.spec.ts`; `timeline-modes.spec.ts`; `timeline-negative-time.spec.ts`; `timeline-paint-cache.spec.ts`; `timeline-panel-collapse.spec.ts`; `timeline-playback-viewport.spec.ts`; `timeline-refresh-cadence.spec.ts`; `timeline-scroll-integrity.spec.ts`; `timeline-shift-selection.spec.ts`; `timeline-track-live-drag.spec.ts`; `timeline.spec.ts`.
**Later candidate e2e (not shipped evidence):** `ae-workflow.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B117 | Time ruler: click/drag ruler changes playhead; empty timeline clicks preserve playhead. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B118 | Timeline pan: middle-button drag pans timeline horizontally. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B123 | Hide/solo/lock/3D switches exist; preserve actual tag controls rather than later column design. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B124 | Timeline rename: inline layer-name edit commits Enter and cancels Escape; ignore blank names. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B137 | Timeline modes: switch Layers/Tracks from mode control or context menu; preserve animation access. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B140 | Layer disclosure: show existing keyframed properties immediately; collapse/reopen without losing selection. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B141 | Layer strip selection: strip clicks retain the graph-focused curve. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B159 | timeline-gesture-performance contract: Moving a group in a large timeline expands selection once and remains undoable. | `v1.2.0:apps/desktop/e2e/timeline-gesture-performance.spec.ts:L3` |
| B166 | timeline-layer-rows contract: Compact layer rows keep names centered and parenting in dedicated controls. | `v1.2.0:apps/desktop/e2e/timeline-layer-rows.spec.ts:L22` |
| B169 | timeline-modes contract: The track timeline packs layers onto Premiere-style tracks and moves clips between them. | `v1.2.0:apps/desktop/e2e/timeline-modes.spec.ts:L42` |
| B170 | timeline-modes contract: Animated clips unfold their keyframes inside the track timeline. | `v1.2.0:apps/desktop/e2e/timeline-modes.spec.ts:L85` |
| B171 | timeline-panel-collapse contract: Keeps the timeline usable after double-clicks and a saved collapsed state. | `v1.2.0:apps/desktop/e2e/timeline-panel-collapse.spec.ts:L3` |
| B172 | timeline-playback-viewport contract: Playback and scrubbing draw only the active timeline viewport, including after reload. | `v1.2.0:apps/desktop/e2e/timeline-playback-viewport.spec.ts:L3` |
| B176 | timeline-refresh-cadence contract: Timeline motion retains subpixel updates independently of composition frame rate. | `v1.2.0:apps/desktop/e2e/timeline-refresh-cadence.spec.ts:L11` |
| B177 | timeline-refresh-cadence contract: Scrub indicator follows subframe pointer motion while selected frames stay accurate. | `v1.2.0:apps/desktop/e2e/timeline-refresh-cadence.spec.ts:L201` |
| B178 | timeline-scroll-integrity contract: Large timeline stays usable after vertical scrolling, panning, zooming and scrubbing. | `v1.2.0:apps/desktop/e2e/timeline-scroll-integrity.spec.ts:L3` |
| B179 | timeline-track-live-drag contract: Moving a clip in time updates the composition before release and creates one undo step. | `v1.2.0:apps/desktop/e2e/timeline-track-live-drag.spec.ts:L40` |
| B180 | timeline-track-live-drag contract: Moving between tracks previews stacking and Escape restores the composition without an edit. | `v1.2.0:apps/desktop/e2e/timeline-track-live-drag.spec.ts:L64` |
| B181 | timeline-track-live-drag contract: A paused video seeks to its new local time during a clip drag and restores on pointer cancellation. | `v1.2.0:apps/desktop/e2e/timeline-track-live-drag.spec.ts:L83` |
| B184 | timeline contract: Scale keys move outside the layer, delete together, and never delete their layer. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L63` |
| B185 | timeline contract: The easing grid applies a curve to both Scale dimensions and preserves linear defaults. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L109` |
| B186 | timeline contract: The disclosure opens and collapses the selected layer strip. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L139` |
| B187 | timeline contract: Command Shift D selects the new segment to the right of the playhead. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L163` |
| B188 | timeline contract: Editing one Bézier handle keeps the neighboring handle visible. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L245` |
| B189 | timeline contract: Continuous Bézier handles stay joined unless Option-drag splits them. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L275` |
| B190 | timeline contract: Marquee-selected graph keyframes move together from inside their transform box. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L329` |
| B191 | timeline contract: A Bézier handle adjusts paired Scale axes without changing other selected times. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L382` |
| B192 | timeline contract: Layer-strip clicks do not replace the curve focused in the Graph Editor. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L432` |
| B193 | timeline contract: Project rename is available from the title menu and persists the live document. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts` |
| B194 | timeline contract: Empty timeline clicks preserve the playhead and marquee selection still works. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L488` |
| B195 | timeline contract: Clicking the ruler moves the playhead. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L522` |
| B196 | timeline contract: Middle-button dragging pans the timeline viewport horizontally. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L543` |
| B197 | timeline contract: Shift pressed during a playhead drag latches when crossing clip edges and keyframes. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L576` |
| B198 | timeline contract: Clicking the transport button pauses playback. | `v1.2.0:apps/desktop/e2e/timeline.spec.ts:L624` |
| B216 | index contract: Binds M to open mixed layer strips together and collapse an open selection. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L96` |
| B217 | index contract: Closes selected group hierarchies when M collapses their layer strips. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L112` |
| B218 | index contract: Selects only the new right-hand segments after splitting. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L125` |
| B219 | index contract: Preserves source time when splitting timed media. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L147` |
| B220 | index contract: Keeps caption cues in place on both halves of a split. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L161` |
| B221 | index contract: Re-bases caption cues from the gesture start while the In edge is dragged. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L176` |
| B222 | index contract: Uses the M command to close and reopen nested groups with every strip. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L211` |
| B223 | index contract: Owns the property-reveal and adjacent-keyframe command ids. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L233` |
| B224 | index contract: Builds the exact canvas skeleton and rebinds the runtime to replacement hosts. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L283` |
| B225 | index contract: Refreshes timecode and graph state when switching projects or changing frame rate. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L295` |
| B226 | index contract: Scrubs time from the initial position instead of accumulating pointer offsets. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L313` |
| B227 | index contract: Places only the essential controls in the ruler gutter. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L330` |
| B228 | index contract: Switches between the layer and track timelines from the panel and the command palette. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L351` |
| B229 | index contract: Preserves the live panel move handle when the extension rebuilds in place. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L415` |
| B230 | index contract: Renders the move slot synchronously before the layout injects its handle. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L426` |
| B231 | index contract: Registers and runs a disposer for kernel reload or disable. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L437` |
| B232 | index contract: Reactivates in place with fresh handlers and preserved view state. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L450` |
| B233 | index contract: Shift-moving a strip snaps its edges to the playhead exactly and holds through the release radius. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L538` |
| B234 | index contract: Releases an external text field when the timeline starts a pointer gesture. | `v1.2.0:apps/desktop/src/extensions/timeline/index.test.ts:L605` |
| B247 | professional-animation contract: Snaps a multi-keyframe group by whichever selected key reaches a target. | `v1.2.0:apps/desktop/src/extensions/timeline/professional-animation.test.ts:L8` |
| B248 | professional-animation contract: Holds a group snap until the anchored key clears the release radius. | `v1.2.0:apps/desktop/src/extensions/timeline/professional-animation.test.ts:L15` |
| B249 | professional-animation contract: Distinguishes Shift-click toggle from Shift-drag movement. | `v1.2.0:apps/desktop/src/extensions/timeline/professional-animation.test.ts:L23` |
| B250 | professional-animation contract: Jumps to clamped work-area boundaries without editing the work area. | `v1.2.0:apps/desktop/src/extensions/timeline/professional-animation.test.ts:L39` |
| B251 | professional-animation contract: Keeps selected-event navigation scoped to selected layer events. | `v1.2.0:apps/desktop/src/extensions/timeline/professional-animation.test.ts:L51` |
| B257 | property-values contract: Scrubs within limits and supports precise and fast adjustment. | `v1.2.0:apps/desktop/src/extensions/timeline/property-values.test.ts:L3` |

### W056 — Move, split and trim clips with source continuity

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Timed media with trim, speed, animation, groups and captions; locked targets.
**Steps:** Move brackets/boundary commands; split at interior frame; trim in/out or extend using handles; cancel drag; undo/redo.
**Expected:** Right tail selected, source time continues at cut, caption cues/keyframes rebased, valid minimum span and one edit. Ordinary trims do not move downstream clips.
**Exact registered keymap entries:** K021 cmd+shift+d → split; K022 ctrl+shift+d → split; K063 cmd+b → split; K064 ctrl+b → split; K139 [ → moveLayerIn; K140 ] → moveLayerOut; K141 alt+[ → trimIn; K142 alt+] → trimOut; K143 alt+home → moveLayerInToStart; K144 alt+end → moveLayerOutToEnd.
**Native menu key equivalents:** M021 CommandOrControl+Shift+D → split.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `agent-video-editing.spec.ts`; `editor-reliability.spec.ts`; `group-timeline-interactions.spec.ts`; `pro-editor-shortcuts.spec.ts`; `timeline-track-live-drag.spec.ts`; `timeline.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A157 | Cmd/Ctrl+shift+d: Run &#96;split&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A178 | Cmd/Ctrl+b: Run &#96;split&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A229 | [: Run &#96;moveLayerIn&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A230 | ]: Run &#96;moveLayerOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A231 | alt+[: Run &#96;trimIn&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A232 | alt+]: Run &#96;trimOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A233 | alt+home: Run &#96;moveLayerInToStart&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A234 | alt+end: Run &#96;moveLayerOutToEnd&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B138 | Live clip preview: moving clip updates composition before release; Escape restores track stacking and local video time. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B139 | Group strips: moving/trimming group strip applies to members; reveal ancestors when selecting grouped descendants. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B515 | Split: Cmd+B or Cmd+Shift+D splits at playhead; select new right segment. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B525 | Layer timing: I/O jump layer in/out; [/] moves layer in/out to playhead; Option+[ or ] trims. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B526 | Boundary retime: Option+Home/End moves layer in to composition start or out to end. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W057 — Slip a source range through the shipped editing API

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Static-timing clip at from=2, dur=3, trim=1, rate=2, source duration=12 with local animation.
**Steps:** Call edit_video slip sourceIn=5; sample start/end and animation; try sourceIn=8; undo/redo.
**Expected:** Valid slip changes only source trim; strip/key times unchanged; 5+3×2=11 fits; 8+3×2=14 is rejected atomically. No dedicated mouse slip tool ships.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/__tests__/agent-video-editing.test.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B258 | Slip coverage: source slip preserves strip and animation while respecting source handles; this is a tested editing operation, not evidence of a dedicated mouse slip tool. | `v1.2.0:apps/desktop/src/renderer/src/legacy/__tests__/agent-video-editing.test.ts` |

### W058 — Retime video while preserving its retained source interval

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Video with trim, speed and animation; embedded and separated audio variants.
**Steps:** Change Speed in Properties; use edit_video speed; key sourceTime/timeRemap explicitly; try unsupported animated-timing structural edit; undo.
**Expected:** Constant-rate duration follows oldRate/newRate, trim retained; API retimes keys, guards composition/min-frame limits and declines embedded-audio retiming until separated.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/editing.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/video-timeline.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte`.
**Existing shipped e2e:** `agent-video-editing.spec.ts`; `video-speed-duration.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B325 | ContentSection / Trim start: NumField edits selected content; step={0.05}; precision={2}; unit="s"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B326 | ContentSection / Speed: NumField edits selected content; step={0.05}; precision={2}; min={0.05}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |

### W059 — Play embedded A/V, separate audio and mix it

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Video with soundtrack and independent audio.
**Steps:** Play; mute/unmute video; Separate Audio; adjust gain/fades; undo; move separated strip independently.
**Expected:** Synchronous embedded soundtrack, copied trim/span/gain/mute and one-step separate/recombine Undo. Separation creates independent strips; persistent linked-pair editing is absent.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte`.
**Existing shipped e2e:** `playback-audio.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B327 | ContentSection / Mute audio: ToggleField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B328 | ContentSection / Audio gain: NumField edits selected content; step={0.05}; precision={2}; min={0}; max={4}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B329 | ContentSection / Trim start: NumField edits selected content; step={0.05}; precision={2}; min={0}; unit="s"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B330 | ContentSection / Gain: NumField edits selected content; step={0.05}; precision={2}; min={0}; max={4}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B331 | ContentSection / Fade in: NumField edits selected content; step={0.05}; precision={2}; min={0}; max={layer.dur}; unit="s"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B332 | ContentSection / Fade out: NumField edits selected content; step={0.05}; precision={2}; min={0}; max={layer.dur}; unit="s"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |

### W060 — Create, reveal, edit and move keyframes

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Animated numeric/color/toggle channels, paired Scale axes, keys outside strip and locked layers.
**Steps:** Stopwatch/diamond; reveal properties; select/marquee; type value/time; drag/nudge and stagger; delete final key; undo.
**Expected:** Exact frame times, axis pairing and scoped selection; collisions/invalid times rejected; final-key removal keeps evaluated value.
**Exact registered keymap entries:** K109 alt+left → nudgeKeyframes [-1]; K110 alt+right → nudgeKeyframes [1]; K111 alt+shift+left → nudgeKeyframes [-10]; K112 alt+shift+right → nudgeKeyframes [10]; K172 p → timeline.revealProperty:p; K173 shift+p → timeline.revealProperty:p [true]; K174 s → timeline.revealProperty:s; K175 shift+s → timeline.revealProperty:s [true]; K176 r → timeline.revealProperty:r; K177 shift+r → timeline.revealProperty:r [true]; K178 t → timeline.revealProperty:t; K179 shift+t → timeline.revealProperty:t [true]; K180 a → timeline.revealProperty:a; K181 shift+a → timeline.revealProperty:a [true]; K182 u → timeline.revealProperty:u; K183 shift+u → timeline.revealProperty:u [true]; K184 m → timeline.revealProperty:m; K185 shift+m → timeline.revealProperty:m [true]; K186 f → timeline.revealProperty:f; K187 shift+f → timeline.revealProperty:f [true]; K188 e → timeline.revealProperty:e; K189 shift+e → timeline.revealProperty:e [true]; K190 l → timeline.revealProperty:l; K191 shift+l → timeline.revealProperty:l [true]; K192 cmd+&#96; → timeline.revealAll; K193 ctrl+&#96; → timeline.revealAll.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-navigation.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `editor-reliability.spec.ts`; `expression-csp.spec.ts`; `keyframe-paused-frame.spec.ts`; `keyframe-simplification-proof.spec.ts`; `keyframe-simplification.spec.ts`; `keyframe-value-dialog.spec.ts`; `property-animation.spec.ts`; `property-reveal.spec.ts`; `text-animators.spec.ts`; `timeline-gesture-performance.spec.ts`; `timeline-graph-paint-cache.spec.ts`; `timeline-keyframe-drag.spec.ts`; `timeline-keyframe-navigation.spec.ts`; `timeline-keyframe-rows.spec.ts`; `timeline-layer-rows.spec.ts`; `timeline-marquee-scroll.spec.ts`; `timeline-modes.spec.ts`; `timeline-negative-time.spec.ts`; `timeline-paint-cache.spec.ts`; `timeline-panel-collapse.spec.ts`; `timeline-playback-viewport.spec.ts`; `timeline-quick-offset.spec.ts`; `timeline-refresh-cadence.spec.ts`; `timeline-scroll-integrity.spec.ts`; `timeline-shift-selection.spec.ts`; `timeline-track-values.spec.ts`; `timeline.spec.ts`; `variable-font-sliders.spec.ts`.
**Later candidate e2e (not shipped evidence):** `channel-operators.spec.ts`; `history-gesture-press.spec.ts`; `set-channels.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A216 | alt+left: Run &#96;nudgeKeyframes&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A217 | alt+right: Run &#96;nudgeKeyframes&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A218 | alt+shift+left: Run &#96;nudgeKeyframes&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A219 | alt+shift+right: Run &#96;nudgeKeyframes&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A244 | cmd+&#96;: Run &#96;timeline.revealAll&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A245 | ctrl+&#96;: Run &#96;timeline.revealAll&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B125 | Key marquee: Shift or Cmd adds marquee-selected keys; graph marquee retains original targets while panning. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B126 | Key drag: move selected keys together; paired Scale dimensions move/delete together without deleting layer. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B127 | Quick stagger: Cmd+Option drag offsets layers in selection order at subframe precision; key groups preserve each span. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B131 | Keyframe value editor: double-click value opens typed editor; Enter commits and Escape cancels. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B132 | Property reset: clearing final key preserves evaluated value and disables current-key diamond; Undo restores. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B136 | Property menu: Add Keyframe Here, Paste Keyframes Here and Clear All Keyframes operate on hit property. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B157 | property-reveal contract: AE property toggles and global M disclosure use actual timeline rows. | `v1.2.0:apps/desktop/e2e/property-reveal.spec.ts:L6` |
| B158 | property-reveal contract: Timeline diamond toggles playhead keys without clearing other times. | `v1.2.0:apps/desktop/e2e/property-reveal.spec.ts:L51` |
| B160 | timeline-keyframe-navigation contract: Timeline shortcuts navigate real keyframes with a compact transport. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-navigation.spec.ts:L4` |
| B161 | timeline-keyframe-navigation contract: Clicking away clears row selection and deleting final keys preserves value with Undo. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-navigation.spec.ts:L57` |
| B162 | timeline-keyframe-navigation contract: Removing the last current key disables its diamond and Undo restores it. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-navigation.spec.ts:L114` |
| B163 | timeline-keyframe-navigation contract: Composition zoom dropdown follows presets, shortcuts, pan and Fit without changing the project. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-navigation.spec.ts:L169` |
| B164 | timeline-keyframe-rows contract: Expanding a layer strip immediately shows its existing keyframed properties. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-rows.spec.ts:L4` |
| B165 | timeline-keyframe-rows contract: Disclosure shows only keyframed properties and refreshes animation changes. | `v1.2.0:apps/desktop/e2e/timeline-keyframe-rows.spec.ts:L46` |
| B168 | timeline-marquee-scroll contract: Graph selection origin stays at the same time and value while panning. | `v1.2.0:apps/desktop/e2e/timeline-marquee-scroll.spec.ts:L58` |
| B173 | timeline-quick-offset contract: Stagger layers in selection order with a subframe Cmd+Option drag and one undo. | `v1.2.0:apps/desktop/e2e/timeline-quick-offset.spec.ts:L8` |
| B174 | timeline-quick-offset contract: Stagger selected keyframe groups per layer without changing each group span. | `v1.2.0:apps/desktop/e2e/timeline-quick-offset.spec.ts:L61` |
| B175 | timeline-quick-offset contract: One Graph Editor handle adjusts selected same-property keyframes across layers. | `v1.2.0:apps/desktop/e2e/timeline-quick-offset.spec.ts:L113` |
| B235 | keyframe-navigation contract: Uses real keys, offsets, finite composition times and both Scale channels. | `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-navigation.test.ts:L18` |
| B236 | keyframe-navigation contract: Does not wrap at endpoints, fall through an unanimated selection, or mutate keys. | `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-navigation.test.ts:L27` |
| B237 | keyframe-navigation contract: Does not scan unrelated layers or reread row keyframes during playback. | `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-navigation.test.ts:L35` |
| B238 | keyframe-value-dialog contract: Still rejects collisions on the neighboring keyframe. | `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-value-dialog.test.ts:L72` |
| B239 | keyframe-value-dialog contract: Snaps composition times to the nearest frame. | `v1.2.0:apps/desktop/src/extensions/timeline/keyframe-value-dialog.test.ts:L81` |
| B252 | property-reveal contract: Isolates static properties and Shift adds and removes rows. | `v1.2.0:apps/desktop/src/extensions/timeline/property-reveal.test.ts:L17` |
| B253 | property-reveal contract: M with no selection opens and closes every strip on consecutive taps. | `v1.2.0:apps/desktop/src/extensions/timeline/property-reveal.test.ts:L31` |
| B254 | property-reveal contract: Single shortcut toggles closed when its rows are already shown. | `v1.2.0:apps/desktop/src/extensions/timeline/property-reveal.test.ts:L37` |
| B280 | Property animation: stopwatch enables/removes animation; diamond adds/removes current-time key; axis fields expose independent diamonds. | `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte` |
| B281 | Tag channel menu supports keyframe, graph, expression and reset paths; Separate Dimensions row and per-axis presentation are later additions. | `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte` |
| B283 | Expressions: Add/Edit Expression opens modal with Cancel/Apply; Remove Expression clears expression. | `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte` |
| B343 | keyframe-value-dialog contract: Numeric keyframe times extend beyond the layer strip, Undo, and reject invalid times. | `v1.2.0:apps/desktop/e2e/keyframe-value-dialog.spec.ts:L3` |
| B344 | property-animation contract: Properties and timeline expose editable content animation, scrubbing, and one-step Undo. | `v1.2.0:apps/desktop/e2e/property-animation.spec.ts:L6` |
| B345 | property-animation contract: Dragging a timeline value adjusts the same property on every selected layer in one undo step. | `v1.2.0:apps/desktop/e2e/property-animation.spec.ts:L127` |
| B519 | Layer nudge: arrows move one unit; Shift+arrows ten; held arrows repeat. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B520 | Key nudge: Option+Left/Right moves one frame; Option+Shift+Left/Right ten; held keys repeat. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B528 | Property reveal: P/S/R/T/A/U/M/F/E/L reveal property groups; Shift adds to existing reveal; Cmd+backtick reveals all. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B534 | Repeat safety: only frame steps and nudges repeat; destructive creation commands ignore key repeat. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W061 — Edit value/speed curves and Bézier handles

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Several animated properties with paired axes and multiple layers.
**Steps:** Switch graph modes; fit/merge; marquee/move/scale; independently drag incoming/outgoing handles; Option split; edit velocity; cancel and undo.
**Expected:** Only intended handles/keys change, paired axes follow, unrelated times fixed; correct curve identity, negative ranges and combined splitter.
**Exact registered keymap entries:** K149 shift+f3 → graph; K150 f9 → easyEase; K151 shift+f9 → easyEaseIn; K152 cmd+shift+f9 → easyEaseOut; K153 ctrl+shift+f9 → easyEaseOut.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`; `agent-keyframe-easing.spec.ts`; `easing-panel.spec.ts`; `graph-editor-review.spec.ts`; `graph-handle-independence.spec.ts`; `graph-picking.spec.ts`; `graph-reference.spec.ts`; `timeline-marquee-scroll.spec.ts`; `timeline-quick-offset.spec.ts`; `timeline.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A239 | shift+f3: Run &#96;graph&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A240 | f9: Run &#96;easyEase&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A241 | shift+f9: Run &#96;easyEaseIn&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A242 | cmd+shift+f9: Run &#96;easyEaseOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A243 | ctrl+shift+f9: Run &#96;easyEaseOut&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B120 | Shift+F3 graph toggle exists; tag chrome offers Dope sheet / combined / graph buttons rather than later single-toggle chrome. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B128 | Graph types: choose Value Graph or Speed Graph; fit selected curves; merge selected curves; optionally show layer bars. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B129 | Graph handles: incoming/outgoing handles remain separately hittable; Option splits continuous handles. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B130 | Graph multi-edit: same-property selected keys across layers accept handle adjustment; preserve unrelated key times. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B145 | graph-editor-review contract: Graph editor exposes its state and keeps value and speed navigation usable. | `v1.2.0:apps/desktop/e2e/graph-editor-review.spec.ts:L3` |
| B146 | graph-handle-independence contract: Dragging an incoming handle leaves another selected key handle fixed and supports Undo. | `v1.2.0:apps/desktop/e2e/graph-handle-independence.spec.ts:L5` |
| B147 | graph-picking contract: A Weight handle edits Weight while Scale is focused, and Undo restores it. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L32` |
| B148 | graph-picking contract: Vertical scrolling pans the graph without scrolling rows or editing animation. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L92` |
| B149 | graph-picking contract: Selected curves expose all their keys and remain editable after clearing selection. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L109` |
| B150 | graph-picking contract: Marquee keeps its original graph targets and selected points still drag together. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L131` |
| B151 | graph-picking contract: Drawing an empty graph marquee preserves selected keyframes. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L156` |
| B152 | graph-picking contract: A graph marquee selects a subset and moves only enclosed points. | `v1.2.0:apps/desktop/e2e/graph-picking.spec.ts:L172` |
| B153 | graph-reference contract: Reference graph supports combined editing, persistent handles, and group scaling with undo. | `v1.2.0:apps/desktop/e2e/graph-reference.spec.ts:L3` |
| B210 | graph-performance contract: Indexes neighbours once for a 10,000-key move and scale. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-performance.test.ts:L4` |
| B211 | graph-presentation contract: Keeps axis identity through reordered selections. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-presentation.test.ts:L5` |
| B212 | graph-presentation contract: Labels readable intervals across negative, fractional, and large ranges. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-presentation.test.ts:L9` |
| B213 | graph-presentation contract: Distinguishes held, linear, and curved keys. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-presentation.test.ts:L15` |
| B214 | graph-presentation contract: Reserves usable space on both sides of the adjustable split. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-presentation.test.ts:L21` |
| B215 | graph-sample-cache contract: Reuses redraw samples and refreshes them after an edit, mode or project change. | `v1.2.0:apps/desktop/src/extensions/timeline/graph-sample-cache.test.ts:L5` |
| B260 | Combined graph splitter: drag graph/layer boundary vertically; keep split ratio between 0.2 and 0.65. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B527 | Easing shortcuts: F9 Easy Ease; Shift+F9 Ease In; Cmd/Ctrl+Shift+F9 Ease Out. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W062 — Group, parent, reorder and precompose without changing authored output

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Nested opacity/mask/blend/effect groups and animated external parent rigs.
**Steps:** Group/ungroup; move into/out of group; pickwhip/parent menu; reorder; precompose partial/whole group; open nested comp tabs; save/undo.
**Expected:** Rendered appearance, timing, rigs, references and group boundaries preserved; unsafe partial moves rejected visibly; nested comp instances keep independent media time.
**Exact registered keymap entries:** K025 cmd+shift+c → precompose; K026 ctrl+shift+c → precompose; K027 cmd+g → groupLayers; K028 ctrl+g → groupLayers; K029 cmd+shift+g → ungroupLayers; K030 ctrl+shift+g → ungroupLayers; K093 cmd+] → bringForward; K094 ctrl+] → bringForward; K095 cmd+[ → sendBackward; K096 ctrl+[ → sendBackward; K097 cmd+shift+] → bringToFront; K098 ctrl+shift+] → bringToFront; K099 cmd+shift+[ → sendToBack; K100 ctrl+shift+[ → sendToBack.
**Native menu key equivalents:** M022 CommandOrControl+] → bringForward; M023 CommandOrControl+[ → sendBackward; M024 CommandOrControl+Shift+] → bringToFront; M025 CommandOrControl+Shift+[ → sendToBack; M028 CommandOrControl+Shift+C → precompose.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/layer-groups.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/compositions.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/parent-pickwhip.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`; `comp-thumbnails.spec.ts`; `compositions.spec.ts`; `figma-paste.spec.ts`; `group-effects.spec.ts`; `group-timeline-interactions.spec.ts`; `group-transforms.spec.ts`; `groups-25d-occlusion.spec.ts`; `groups-25d.spec.ts`; `layer-reorder.spec.ts`; `precomp.spec.ts`; `precompose-group-appearance.spec.ts`; `split-text.spec.ts`; `ungroup-opacity.spec.ts`; `ungroup-parent-rigs.spec.ts`; `ungroup-solo.spec.ts`; `ungroup-visibility.spec.ts`.
**Later candidate e2e (not shipped evidence):** `ae-workflow.spec.ts`; `native-edit-kernels.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A159 | Cmd/Ctrl+shift+c: Run &#96;precompose&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A160 | Cmd/Ctrl+g: Run &#96;groupLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A161 | Cmd/Ctrl+shift+g: Run &#96;ungroupLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A193 | Cmd/Ctrl+]: Run &#96;bringForward&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A194 | Cmd/Ctrl+[: Run &#96;sendBackward&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A195 | Cmd/Ctrl+shift+]: Run &#96;bringToFront&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A196 | Cmd/Ctrl+shift+[: Run &#96;sendToBack&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B154 | group-timeline-interactions contract: M closes and reopens nested group layers with no selection. | `v1.2.0:apps/desktop/e2e/group-timeline-interactions.spec.ts:L6` |
| B155 | group-timeline-interactions contract: Selecting grouped layers reveals ancestors without hiding siblings. | `v1.2.0:apps/desktop/e2e/group-timeline-interactions.spec.ts:L40` |
| B156 | group-timeline-interactions contract: Both edges of a selected group strip freely trim its members. | `v1.2.0:apps/desktop/e2e/group-timeline-interactions.spec.ts:L99` |
| B240 | layer-drop contract: Group center nests, edges place outside the entire group. | `v1.2.0:apps/desktop/src/extensions/timeline/layer-drop.test.ts:L9` |
| B241 | layer-drop contract: Properties resolve to their owner and child gaps preserve membership. | `v1.2.0:apps/desktop/src/extensions/timeline/layer-drop.test.ts:L15` |
| B262 | Composition tabs: show open compositions and activate selected composition through shared project state. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B263 | Composition tab lifetime: preserve valid tabs when compositions change and repair active composition references. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B264 | Parent pickwhip: drag dedicated parenting control to target layer; parenting control also offers menu selection. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/parent-pickwhip.ts` |
| B265 | compositions contract: Pre-compose keeps every rendered pixel, and the new comp opens, nests and survives a save. | `v1.2.0:apps/desktop/e2e/compositions.spec.ts:L14` |
| B266 | precomp contract: Grouping preserves rendered pixels, time, parent rigs, save data and Undo. | `v1.2.0:apps/desktop/e2e/precomp.spec.ts:L13` |
| B267 | precomp contract: Canvas and timeline open the same compact layer menu. | `v1.2.0:apps/desktop/e2e/precomp.spec.ts:L46` |
| B268 | precomp contract: Pickwhip parents to a timeline row with no pose jump and one Undo. | `v1.2.0:apps/desktop/e2e/precomp.spec.ts:L74` |
| B269 | precompose-group-appearance contract: Partial precomposition preserves overlapping group alpha, animated rigs and Undo. | `v1.2.0:apps/desktop/e2e/precompose-group-appearance.spec.ts:L8` |
| B270 | precompose-group-appearance contract: Nested shared blur, mask and blend remain on their original group boundary. | `v1.2.0:apps/desktop/e2e/precompose-group-appearance.spec.ts:L46` |
| B271 | precompose-group-appearance contract: Offcanvas transformed styled groups reject partial moves and preserve whole-group pixels. | `v1.2.0:apps/desktop/e2e/precompose-group-appearance.spec.ts:L81` |
| B272 | comp-tabs contract: Coalesces project-switch notifications and retains unchanged tab DOM and focus. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.test.ts:L29` |
| B273 | comp-tabs contract: Switches nested composition tabs by pointer and keyboard without scrolling parent panels. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.test.ts:L40` |
| B274 | comp-tabs contract: Cancels deferred work and unsubscribes on disposal. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.test.ts:L52` |
| B275 | Tab close: close affordance exists only when more than one composition tab is open; close click does not activate tab. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B276 | Tab keyboard: active tab has tab stop; Enter/Space opens composition. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B277 | Tab menu: Composition Settings acts on clicked tab; New Composition Cmd+N and conditional Close are offered. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B278 | Tab reveal: scroll selected tab into strip view only when active composition changes; do not scroll surrounding panel. | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| B279 | Outliner ownership: layer hierarchy/disclosures and parenting are implemented in timeline rather than an additional independent built-in layers panel. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B517 | Grouping: Cmd+G groups; Cmd+Shift+G ungroups; Cmd+Shift+C precomposes. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B531 | Stack order: Cmd+[ or ] moves backward/forward; Shift sends to back/front. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |

### W063 — Edit selected content, transforms and composition properties

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Mixed-type selection, locked group members and animated properties.
**Steps:** Type/scrub shared and private fields; switch layers; align; toggle link Scale; cancel and undo; edit while playing then pause.
**Expected:** Applicable members only, exact stopped value, correct mixed state and defaults; locked targets protected; one gesture Undo and retained per-layer scroll.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.svelte`, `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte`, `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** `bulk-property-edit.spec.ts`; `generated-panel.spec.ts`; `inspector-alignment.spec.ts`; `inspector-effects.spec.ts`; `inspector-group-locks.spec.ts`; `inspector-scroll.spec.ts`; `property-animation.spec.ts`; `scale-aspect-ratio.spec.ts`; `timeline-track-values.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B059 | scale-aspect-ratio contract: Aspect ratio lock stays in sync and preserves proportions in both panels. | `v1.2.0:apps/desktop/e2e/scale-aspect-ratio.spec.ts:L3` |
| B142 | Property values: timeline readouts support typed edits, axis-specific scrubs, cancellation, Undo and locks. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B143 | Linked Scale: aspect lock stays synchronized with inspector; paired keys are created when linked. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| B182 | timeline-track-values contract: Track property values support typing, axis-specific dragging, cancellation, Undo, and locks. | `v1.2.0:apps/desktop/e2e/timeline-track-values.spec.ts:L23` |
| B183 | timeline-track-values contract: A track readout edits the clip at the playhead and creates paired Scale keys. | `v1.2.0:apps/desktop/e2e/timeline-track-values.spec.ts:L140` |
| B282 | Scale linking: link toggle preserves aspect during edits; unlinked axes edit separately. | `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte` |
| B284 | Multi-edit locks: disable writes for locked layers and locked ancestor groups. | `v1.2.0:apps/desktop/src/extensions/inspector/ChannelRow.svelte` |
| B315 | ContentSection / Fill: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B316 | ContentSection / Shape: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B317 | ContentSection / Width: NumField edits selected content; step={1}; min={1}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B318 | ContentSection / Height: NumField edits selected content; step={1}; min={1}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B319 | ContentSection / Corner smoothing: NumField edits selected content; step={1}; min={0}; max={100}; unit="%"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B320 | ContentSection / Corner radius: NumField edits selected content; step={1}; min={0}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B321 | ContentSection / Stroke: NumField edits selected content; step={0.5}; min={0}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B322 | ContentSection / Stroke: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B323 | ContentSection / Points: NumField edits selected content; step={1}; min={3}; max={24}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B324 | ContentSection / Fit: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B334 | bulk-property-edit contract: A shared dimension edit reaches mixed types while type-private fields stay put. | `v1.2.0:apps/desktop/e2e/bulk-property-edit.spec.ts:L40` |
| B338 | inspector-effects contract: A copied effect pastes onto the layer selected after copying. | `v1.2.0:apps/desktop/e2e/inspector-effects.spec.ts:L5` |
| B339 | inspector-effects contract: Effect rows select, copy, paste, delete, and undo as editable source transactions. | `v1.2.0:apps/desktop/e2e/inspector-effects.spec.ts:L37` |
| B340 | inspector-effects contract: Timeline controls occupy the ruler gutter without legacy navigation buttons. | `v1.2.0:apps/desktop/e2e/inspector-effects.spec.ts:L98` |
| B341 | inspector-group-locks contract: Inspector edits skip selected layers inside locked groups and Undo restores editable siblings. | `v1.2.0:apps/desktop/e2e/inspector-group-locks.spec.ts:L3` |
| B342 | inspector-scroll contract: Properties remembers the scroll position of each layer. | `v1.2.0:apps/desktop/e2e/inspector-scroll.spec.ts:L3` |
| B351 | InspectorPanel contract: Limits playback property evaluation and shows the exact stopped value immediately. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L423` |
| B352 | InspectorPanel contract: Checks a multi-selection for mixed values on the readout clock, never on a playback frame. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L453` |
| B353 | InspectorPanel contract: Updates animated values on time events without rebuilding parenting choices. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L478` |
| B354 | InspectorPanel contract: Exposes the compact footprint of a transparent null object. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L507` |
| B355 | InspectorPanel contract: Offers parent selection when a group layer is selected. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L518` |
| B356 | InspectorPanel contract: Exposes effects, masks, blend, motion blur and track mattes for groups. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L527` |
| B357 | InspectorPanel contract: Offers a group as a transform parent without changing membership. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L551` |
| B358 | InspectorPanel contract: Activates by registering the Properties and Layer Effects panel contributions. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L566` |
| B359 | InspectorPanel contract: Caps multi-selection rendering to the first layer and keeps the header consistent. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L617` |
| B360 | InspectorPanel contract: Does not repeat the Properties heading above composition controls. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L627` |
| B361 | InspectorPanel contract: Presents imported SVG paths as Motioner-style property sections. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L634` |
| B362 | InspectorPanel contract: Updates a channel value on the values tick without remounting its row. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L655` |
| B363 | InspectorPanel contract: Keeps both axes in the same Scale row when linking or unlinking. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L674` |
| B364 | InspectorPanel contract: Edits both linked axes proportionally and only the chosen axis when unlinked. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L717` |
| B365 | InspectorPanel contract: Toggles only playhead keys with the property diamond. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L790` |
| B366 | InspectorPanel contract: Selects a property without opening its timeline rows. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L804` |
| B367 | InspectorPanel contract: Resets an animated effect parameter to its defined default. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L833` |
| B368 | InspectorPanel contract: Derives keyframe-diamond state from keyframes and playhead time. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L875` |
| B369 | InspectorPanel contract: Sends the exact legacy remove-effect command. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts` |
| B370 | InspectorPanel contract: Reorders effects by drag and by option-arrow. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L928` |
| B371 | InspectorPanel contract: Selects multiple effects and routes copy, paste, and delete before layer shortcuts. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L963` |
| B372 | InspectorPanel contract: Keeps a copied effect active while a different destination layer is selected. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1018` |
| B373 | InspectorPanel contract: Routes composition fields through an exact set_composition command. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1040` |
| B374 | InspectorPanel contract: Routes transform, effect, layer, and content fields through exact typed commands. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1057` |
| B375 | InspectorPanel contract: Exposes direct keyframe controls for numeric, color, and toggle effect parameters. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1105` |
| B376 | InspectorPanel contract: Keeps paired visual effect coordinates precise and independently animatable. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1141` |
| B377 | InspectorPanel contract: Falls back to available effect fields when an older instance lacks widget properties. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1159` |
| B378 | InspectorPanel contract: Uses set_effect for toggles and exposes one controlled expansion button. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1178` |
| B379 | InspectorPanel contract: Selects effect rows without changing disclosure and reserves disclosure for the chevron. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1199` |
| B380 | InspectorPanel contract: Clears effect selection when clicking anywhere outside it but not from nested controls. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1219` |
| B381 | InspectorPanel contract: Labels every numeric spinbutton and gives every button an accessible name. | `v1.2.0:apps/desktop/src/extensions/inspector/InspectorPanel.test.ts:L1288` |
| B390 | multi-edit-content contract: Applies a fill change to every selected layer that has a colour, regardless of type. | `v1.2.0:apps/desktop/src/extensions/inspector/multi-edit-content.test.ts:L46` |
| B391 | multi-edit-content contract: Skips selected layers whose type has no such content field. | `v1.2.0:apps/desktop/src/extensions/inspector/multi-edit-content.test.ts:L56` |
| B392 | multi-edit-content contract: Shares dimension edits across mixed types. | `v1.2.0:apps/desktop/src/extensions/inspector/multi-edit-content.test.ts:L66` |
| B393 | multi-edit-content contract: Reports mixed fills across a cross-type selection. | `v1.2.0:apps/desktop/src/extensions/inspector/multi-edit-content.test.ts:L75` |

### W064 — Browse, apply, animate and reorder effects or mattes

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Selected layers, registered effects, copied effect stack and two matte-compatible layers.
**Steps:** Search/browse; apply/drop; edit parameters; select/copy/paste/delete/reorder; toggle/reset; set matte source/mode and undo.
**Expected:** Correct selected destination, parameter/default/animation values and compositing pixels; contextual clipboard wins over layer clipboard. Later effect-version/input APIs are separate.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte`, `v1.2.0:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte`.
**Existing shipped e2e:** `adjustment-layer.spec.ts`; `ae-audit-implementation.spec.ts`; `effect-authoring.spec.ts`; `effect-backdrop.spec.ts`; `effect-zoom.spec.ts`; `gradient-effect.spec.ts`; `group-effects.spec.ts`; `inspector-alignment.spec.ts`; `inspector-effects.spec.ts`; `inspector-group-locks.spec.ts`; `inspector-scroll.spec.ts`; `shader-colors.spec.ts`.
**Later candidate e2e (not shipped evidence):** `color-correction-effects.spec.ts`; `color-space-effects.spec.ts`; `compound-blur.spec.ts`; `effects-idle-cost.spec.ts`; `input-shutter.spec.ts`; `layer-compositing.spec.ts`; `layer-input-effects.spec.ts`; `shader-color-uniform.spec.ts`; `shadow-ramp-effects.spec.ts`; `source-owner-effects.spec.ts`; `source-raster-inputs.spec.ts`; `spatial-effects.spec.ts`; `stroke-lut-effects.spec.ts`; `temporal-effects.spec.ts`; `typed-effect-references.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B285 | Effect selection: Shift selects contiguous range; Cmd/Ctrl toggles; selection has its own keyboard/clipboard context. | `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte` |
| B286 | Effect stack: add, toggle, reset, remove and reorder effects; changes apply through inspector history. | `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte` |
| B287 | Effect keyboard order: Option+Up/Down reorders selected effects. | `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte` |
| B288 | Shipped matte source/mode editing exists; later main adds remove-matte/toggle and reorganized stack chrome. | `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte` |
| B413 | Effect browsing: search registered definitions by group; Escape clears search; keyboard moves through tree. | `v1.2.0:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte` |
| B414 | Apply effect: double-click, Enter or Add applies to selected layer; drag effect payload onto layer. | `v1.2.0:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte` |

### W065 — Edit and organize easing/vector preset libraries

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Selected keys and saved .json/.kease presets with groups.
**Steps:** Edit graph/numeric handles; local undo; auto-apply; copy/paste easing/values by section; save/import/export/move presets; mirror/duplicate keys; clean paths.
**Expected:** Local curve history separate from project Undo, one applied gesture, durable presets and correct vector playback.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte`.
**Existing shipped e2e:** `easing-panel.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B416 | Curve workspace: show Graph + Library or toggle views; preserve panel layout preferences. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B417 | Curve editing: drag handles; numeric X/Y inputs; mirror/reset curve; local curve undo/redo separate from project history. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B418 | Auto apply: option applies graph changes to selected keys in one undoable gesture. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B419 | Paste easing: Cmd+V pastes easing; Cmd+Shift+V pastes values; context targets incoming/middle/outgoing sections. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B420 | Preset management: save, rename, color, duplicate, move to group, export or delete preset; manage groups. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B421 | Preset import: accept .json/.kease files and folders; export preset, group or entire library. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B422 | Animation utilities: duplicate/mirror keys at playhead; linear/continuous/auto Bezier; overshoot/bounce expressions; clean path/create controller. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B423 | Preset presentation: configure columns, size 32/44/56/72, labels, palette and preview. | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| B437 | easing-panel contract: Graph editing auto-applies as one undoable gesture and imports extended presets. | `v1.2.0:apps/desktop/e2e/easing-panel.spec.ts:L45` |
| B438 | easing-panel contract: Vector presets and curved path cleanup work through the native panel and playback. | `v1.2.0:apps/desktop/e2e/easing-panel.spec.ts:L77` |

### W066 — Change typography, variable axes and text animators

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Installed static/variable fonts, wrapped text and stagger presets.
**Steps:** Search/select face; edit real discovered axes/size/tracking/leading/alignment; animate and scrub; apply word/character/line split and stagger band; undo/save.
**Expected:** Exact authored glyph layout/anchor, no invented axes, live rendered changes, blank/locked handling and persistent animation.
**Exact registered keymap entries:** K011 cmd+y → newSolid; K012 ctrl+y → newSolid.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte`, `v1.2.0:apps/desktop/src/extensions/inspector/VariableFontHarness.svelte`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/text-animation.ts`, `v1.2.0:apps/desktop/src/renderer/src/controls/FontField.svelte`.
**Existing shipped e2e:** `font-anchor.spec.ts`; `font-registry-cache.spec.ts`; `split-text.spec.ts`; `text-animators.spec.ts`; `variable-font-sliders.spec.ts`.
**Later candidate e2e (not shipped evidence):** `native-edit-kernels.spec.ts`; `native-engine-font-census-perf.spec.ts`; `native-engine-font-seeds.spec.ts`; `native-engine-text-app-parity.spec.ts`; `native-engine-text-level2.spec.ts`; `native-engine-text-typing.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B053 | font-anchor contract: Typography preserves its anchor while changed alignment restores the natural origin. | `v1.2.0:apps/desktop/e2e/font-anchor.spec.ts:L6` |
| B311 | ContentSection / Size: NumField edits selected content; step={1}; min={4}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B312 | ContentSection / Text color: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B313 | ContentSection / Tracking: NumField edits selected content; step={0.5}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B314 | ContentSection / Leading: NumField edits selected content; step={0.02}; precision={2}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/ContentSection.svelte` |
| B347 | variable-font-sliders contract: Variable font numeric fields scrub, animate, undo, and persist. | `v1.2.0:apps/desktop/e2e/variable-font-sliders.spec.ts:L4` |
| B348 | variable-font-sliders contract: Installed variable font exposes its axes before editing and changes real glyph pixels. | `v1.2.0:apps/desktop/e2e/variable-font-sliders.spec.ts:L52` |
| B349 | variable-font-sliders contract: First-pass keyframe playback renders every width sample without delayed font swaps. | `v1.2.0:apps/desktop/e2e/variable-font-sliders.spec.ts:L88` |
| B350 | variable-font-sliders contract: SF Pro weight scrubbing reuses decoded faces without crashing the renderer. | `v1.2.0:apps/desktop/e2e/variable-font-sliders.spec.ts:L129` |
| B382 | VariableFontInspector contract: Shows real discovered axes without creating project values or duplicate weight fields. | `v1.2.0:apps/desktop/src/extensions/inspector/VariableFontInspector.test.ts:L140` |
| B383 | VariableFontInspector contract: Distinguishes static fonts and keeps saved custom axes without invented sliders. | `v1.2.0:apps/desktop/src/extensions/inspector/VariableFontInspector.test.ts:L159` |
| B384 | VariableFontInspector contract: Shows the current alignment and writes changes through the animated content binding. | `v1.2.0:apps/desktop/src/extensions/inspector/VariableFontInspector.test.ts:L175` |
| B454 | split-text contract: Wrapped lines, locked layers and blank text. | `v1.2.0:apps/desktop/e2e/split-text.spec.ts:L57` |
| B455 | text-animators contract: A stagger preset plays without keyframes and its timeline band edits timing with one Undo each. | `v1.2.0:apps/desktop/e2e/text-animators.spec.ts:L34` |
| B549 | FontField: searchable font sheet previews each face, checks current face, pages 80 results and restores trigger focus on dismiss. | `v1.2.0:apps/desktop/src/renderer/src/controls/FontField.svelte` |
| B587 | components contract: FontField applies a choice matching the stale readout when the live font has changed. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L326` |
| B591 | components contract: FontField toggles closed and can open again. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L379` |
| B592 | components contract: FontField uses the font list, invalidates all views, and exposes a dialog/listbox. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L397` |

### W067 — Generate, edit, style and exchange captions

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** SRT/WebVTT plus spoken clips with trim/speed/time remapping and CJK text.
**Steps:** Generate/import; select/edit/move/split/merge/delete cues; search list; change style; trim/split clip; export sidecar and burned video; undo.
**Expected:** Word timing maps to composition time; nonoverlapping valid cues and CJK spacing/wrapping; export round-trips and ended runs cannot add late captions.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/captions/commands.ts`, `v1.2.0:apps/desktop/src/renderer/src/captions/formats.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`, `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte`.
**Existing shipped e2e:** `captions-ui.spec.ts`; `captions.spec.ts`; `media-intelligence.spec.ts`; `transcription.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A459 | Acceptance media-intelligence: Generate Captions puts the spoken words on the caption track in composition time. | `v1.2.0:apps/desktop/e2e/media-intelligence.spec.ts:L161` |
| A460 | Acceptance media-intelligence: The agent watches, listens to and captions the clip through the live tool bridge. | `v1.2.0:apps/desktop/e2e/media-intelligence.spec.ts:L259` |
| A461 | Acceptance media-intelligence: Without a model, Generate Captions asks for one and dismissing changes nothing. | `v1.2.0:apps/desktop/e2e/media-intelligence.spec.ts:L375` |
| A462 | Acceptance media-intelligence: An imported SRT edits, round-trips as SRT and WebVTT, burns into a frame and rides along an export. | `v1.2.0:apps/desktop/e2e/media-intelligence.spec.ts:L464` |
| A463 | Acceptance media-intelligence: The Mixer shows the speech clip, its meters move while it plays and its fader is undoable. | `v1.2.0:apps/desktop/e2e/media-intelligence.spec.ts:L550` |
| A538 | MCP &#96;transcribe_media&#96;: Transcribe speech on this Mac. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L271` |
| A549 | MCP &#96;generate_captions&#96;: Transcribe the speech of audio/video layers on this Mac and add the result as a new captions layer on top, mapped through each clip's trim, speed and time remapping and split into readable cues. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L346` |
| A550 | MCP &#96;export_captions&#96;: Read a captions layer as SubRip (srt) or WebVTT (vtt) text in composition time, for a sidecar file. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L356` |
| B289 | CaptionsSection / Caption weight: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B290 | CaptionsSection / Caption size: NumField edits selected content; step={1}; min={4}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B291 | CaptionsSection / Caption color: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B292 | CaptionsSection / Caption outline: NumField edits selected content; step={0.5}; min={0}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B293 | CaptionsSection / Outline color: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B294 | CaptionsSection / Text case: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B295 | CaptionsSection / Background: ToggleField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B296 | CaptionsSection / Background color: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B297 | CaptionsSection / Background opacity: NumField edits selected content; step={1}; min={0}; max={100}; unit="%"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B298 | CaptionsSection / Background padding: NumField edits selected content; step={1}; min={0}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B299 | CaptionsSection / Background corner radius: NumField edits selected content; step={1}; min={0}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B300 | CaptionsSection / Caption position: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B301 | CaptionsSection / Caption alignment: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B302 | CaptionsSection / Horizontal offset: NumField edits selected content; step={1}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B303 | CaptionsSection / Vertical offset: NumField edits selected content; step={1}; unit="px"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B304 | CaptionsSection / Safe margin: NumField edits selected content; step={0.5}; min={0}; max={45}; unit="%"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B305 | CaptionsSection / Maximum width: NumField edits selected content; step={1}; min={10}; max={100}; unit="%"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B306 | CaptionsSection / Maximum lines: NumField edits selected content; step={1}; min={1}; max={6}; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B307 | CaptionsSection / Word highlight: ToggleField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B308 | CaptionsSection / Highlight color: ColorField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B309 | CaptionsSection / Highlight size: NumField edits selected content; step={1}; min={50}; max={300}; unit="%"; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B310 | CaptionsSection / Word animation: SelectField edits selected content; use component keyboard/gesture rules in area 7. | `v1.2.0:apps/desktop/src/extensions/inspector/CaptionsSection.svelte` |
| B434 | captions-ui contract: Every preset renders a caption. | `v1.2.0:apps/desktop/e2e/captions-ui.spec.ts` |
| B435 | captions contract: Subtitles import onto a caption track, edit in place, burn into frames and export back. | `v1.2.0:apps/desktop/e2e/captions.spec.ts:L52` |
| B436 | captions contract: Timeline cue gestures keep neighbours apart and Delete removes selected cues. | `v1.2.0:apps/desktop/e2e/captions.spec.ts:L137` |
| B458 | agent-generate contract: Returns a job for long transcriptions and applies the result in the run. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L53` |
| B459 | agent-generate contract: Answers in one call when the transcript is ready in time. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L70` |
| B460 | agent-generate contract: Cancels the job when the run ends, so no captions land afterwards. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L77` |
| B461 | agent-generate contract: Refuses to apply into a run whose transaction is gone, even without the abort. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L89` |
| B462 | agent-generate contract: Refuses a second generation while one is running in the same run. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L100` |
| B463 | agent-generate contract: Keeps jobs private to their run and reports a missing model cleanly. | `v1.2.0:apps/desktop/src/renderer/src/captions/agent-generate.test.ts:L111` |
| B464 | cjk contract: Joins transcript words without spaces where Chinese or Japanese meet. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L14` |
| B465 | cjk contract: Breaks between characters; closing punctuation and small kana stay with the character before. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L23` |
| B466 | cjk contract: Wraps a long Chinese cue across lines with no spaces between characters. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L30` |
| B467 | cjk contract: Spreads word timings over the characters they cover, for highlight. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L42` |
| B468 | cjk contract: Segments transcripts into unspaced cues sized by display width. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L47` |
| B469 | cjk contract: Splits and merges Chinese cues without inventing spaces. | `v1.2.0:apps/desktop/src/renderer/src/captions/cjk.test.ts:L57` |
| B470 | clip-timing contract: Re-bases cues when the In point moves, clipping the cue spanning it. | `v1.2.0:apps/desktop/src/renderer/src/captions/clip-timing.test.ts:L31` |
| B471 | clip-timing contract: Splits a captions clip so both halves keep their cues where they were heard. | `v1.2.0:apps/desktop/src/renderer/src/captions/clip-timing.test.ts:L38` |
| B472 | clip-timing contract: Splits captions in a blade-all split with nothing selected. | `v1.2.0:apps/desktop/src/renderer/src/captions/clip-timing.test.ts:L54` |
| B473 | clip-timing contract: Keeps cues in place when the In point is trimmed to the playhead. | `v1.2.0:apps/desktop/src/renderer/src/captions/clip-timing.test.ts:L62` |
| B474 | commands contract: Adds a captions layer on top from SRT text, sized to the composition. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L22` |
| B475 | commands contract: Normalises set_content on a captions layer, so raw patches cannot corrupt the cue list. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L36` |
| B476 | commands contract: Stacks a second captions layer showing at the same time instead of covering the first. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L61` |
| B477 | commands contract: Uses composition time in commands and layer time in storage. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L84` |
| B478 | commands contract: Edits cues as one undo step each and undoes them. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L93` |
| B479 | commands contract: Moves, deletes, inserts, restyles and re-imports. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L109` |
| B480 | commands contract: Fails cleanly and leaves the project untouched. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L127` |
| B481 | commands contract: Respects layer locks for generated origins. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L144` |
| B482 | commands contract: Merges a live cue drag into one recorded command. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L153` |
| B483 | commands contract: Does not regrow a trimmed Out point when an earlier cue is edited. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L167` |
| B484 | commands contract: Keeps a live cue drag cheap on a long word-timed transcript. | `v1.2.0:apps/desktop/src/renderer/src/captions/commands.test.ts:L182` |
| B485 | formats contract: Reads BOM, CRLF, wrong numbering and markup. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L7` |
| B486 | formats contract: Accepts missing hours, dot separators, no numbers and stray whitespace lines. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L17` |
| B487 | formats contract: Sorts out-of-order cues and resolves overlaps. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L22` |
| B488 | formats contract: Skips unreadable blocks and empty cues. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L34` |
| B489 | formats contract: Writes numbered cues with comma milliseconds and an offset. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L40` |
| B490 | formats contract: Round-trips through the parser. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts` |
| B491 | formats contract: Reads headers, NOTE/STYLE/REGION blocks, identifiers and cue settings. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L59` |
| B492 | formats contract: Turns inline timestamps into word timings. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L72` |
| B493 | formats contract: Writes a valid file with escaped text and an optional language. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L82` |
| B494 | formats contract: Clips to a range and rebases to its start. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L88` |
| B495 | formats contract: Detects by name, then by header, and falls back across formats. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L99` |
| B496 | formats contract: Parses timestamps and strips markup. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L107` |
| B497 | formats contract: Keeps ids unique through normalisation. | `v1.2.0:apps/desktop/src/renderer/src/captions/formats.test.ts:L114` |
| B498 | generate-toast contract: Shows no progress while the model download sheet is open and nothing runs. | `v1.2.0:apps/desktop/src/renderer/src/captions/generate-toast.test.ts:L39` |
| B505 | Caption keyboard edit: Enter commits cue text, Shift+Enter allows newline, Escape cancels. | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |
| B506 | Caption search: filter cue list; Escape clears and blurs; ArrowDown enters list at first visible cue. | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |
| B507 | Caption list navigation: Up/Down selects cue, Enter edits, Delete/Backspace removes selected cues; list supports multi-selection. | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |
| B508 | Caption layer menu: choose caption layer and synchronize layer selection. | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |

### W068 — Mix audible nested layers and inspect meters

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Nested audio/precomp, muted/solo strips and a locked layer.
**Steps:** Open mixer; play; move strip/master fader; mute/solo/Option-solo; animate level; latch/clear clipping; pause; undo.
**Expected:** Audible-layer strips, actual sound changes, meters only during sound/playback, correct lock and single-step fader Undo.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/MixerPanel.svelte`, `v1.2.0:apps/desktop/src/renderer/src/mixer/MixerStrip.svelte`.
**Existing shipped e2e:** `media-intelligence.spec.ts`; `mixer.spec.ts`.
**Later candidate e2e (not shipped evidence):** `native-engine-transport.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B388 | audio contract: Finds sound at any depth. | `v1.2.0:apps/desktop/src/extensions/inspector/audio.test.ts` |
| B389 | audio contract: Is false for silent, missing and cyclic compositions. | `v1.2.0:apps/desktop/src/extensions/inspector/audio.test.ts:L19` |
| B453 | mixer contract: Mute and solo change what plays, and meters run only while sound plays. | `v1.2.0:apps/desktop/e2e/mixer.spec.ts:L165` |
| B509 | Mixer strip: one strip per audible layer plus master; click name selects layer; locked strips disable gain edits. | `v1.2.0:apps/desktop/src/renderer/src/mixer/MixerStrip.svelte` |
| B510 | Mixer solo: solo is listening control; Option-click listens only to this strip. | `v1.2.0:apps/desktop/src/renderer/src/mixer/MixerStrip.svelte` |
| B511 | Mixer clipping: clip indicator latches and clicking clears it. | `v1.2.0:apps/desktop/src/renderer/src/mixer/MixerStrip.svelte` |
| B512 | Mixer animation: current-time level diamond adds/removes key; mute control indicates timeline animation. | `v1.2.0:apps/desktop/src/renderer/src/mixer/MixerStrip.svelte` |

### W069 — Configure, start and cancel rendered delivery

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Composition with audio, alpha, captions and several supported formats.
**Steps:** Open Export; set format/size/rate/range/quality/audio/alpha/captions; save/select preset; choose destination; cancel picker; render; cancel while frames encode; render again.
**Expected:** Format gating and defaults; destination before work; actual pixel/frame progress; cancel releases encoder so playback and next export work; correct output frames/audio.
**Exact registered keymap entries:** K049 cmd+e → export; K050 ctrl+e → export; K051 cmd+shift+e → export; K052 ctrl+shift+e → export.
**Native menu key equivalents:** M013 CommandOrControl+E → export.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts`, `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts`.
**Existing shipped e2e:** `alpha-async-export.spec.ts`; `alpha-export-performance.spec.ts`; `export-capabilities.spec.ts`; `export-destination.spec.ts`; `export-dialog.spec.ts`; `export-mp4.spec.ts`; `export-streaming.spec.ts`; `export-video-frames.spec.ts`; `export-webm.spec.ts`; `native-export-performance.spec.ts`; `png-export-performance.spec.ts`; `project-export-fixture.spec.ts`; `static-effects-export.spec.ts`; `static-export-performance.spec.ts`; `static-image-export.spec.ts`; `static-masks-export.spec.ts`; `webm-export-performance.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-delivery.spec.ts`; `engine-export-progress.spec.ts`; `engine-export-warmth.spec.ts`; `engine-export.spec.ts`; `engine-source-guard.spec.ts`; `engine-video-end.spec.ts`; `export-baseline.spec.ts`; `export-encoding-parity.spec.ts`; `export-qos.spec.ts`; `export-sequential-decode.spec.ts`; `legacy-blurred-video-export.spec.ts`; `opaque-effects-export.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A171 | Cmd/Ctrl+e: Run &#96;export&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A172 | Cmd/Ctrl+shift+e: Run &#96;export&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A264 | Project export defaults: Persist format, scaled/custom output size, export frame rate, range, quality, motion blur, audio and transparent background. | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/project-settings.ts` |
| A507 | MCP &#96;export_video&#96;: Render the current project to MP4 (default) or ProRes with its audio. | `v1.2.0:apps/desktop/src/main/agent-tools/spec.ts:L60` |
| B424 | Export entry: Cmd+E or Cmd+Shift+E opens Export; Cancel dismisses; action label follows format. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B425 | Format gating: format controls availability of size, frame rate, quality, audio and alpha options. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B426 | Output size: scale presets show dimensions; Custom size accepts explicit dimensions. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B427 | Export range: choose full composition or work area; unset work area is identified. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B428 | Caption sidecars: choose None, SubRip .srt or WebVTT .vtt. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B429 | Render presets: name/save settings excluding output name; select saved preset. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B430 | Export progress: show Preparing then actual frame/pixel progress; Cancel releases export; completion permits another export. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |
| B439 | export-capabilities contract: Reports the hidden packaged-renderer video delivery capabilities. | `v1.2.0:apps/desktop/e2e/export-capabilities.spec.ts:L3` |
| B440 | export-destination contract: Every export asks for its destination before rendering or showing progress. | `v1.2.0:apps/desktop/e2e/export-destination.spec.ts:L5` |
| B441 | export-destination contract: Still and sequence exports use the selected file or folder without another prompt. | `v1.2.0:apps/desktop/e2e/export-destination.spec.ts:L38` |
| B442 | export-dialog contract: Export inherits Settings spacing and rounded cards and controls in both themes. | `v1.2.0:apps/desktop/e2e/export-dialog.spec.ts:L5` |
| B443 | export-dialog contract: Export dialog gates fields by format and accepts a custom size. | `v1.2.0:apps/desktop/e2e/export-dialog.spec.ts:L44` |
| B444 | export-dialog contract: Export button shows rendered frames, finishes, and can export again. | `v1.2.0:apps/desktop/e2e/export-dialog.spec.ts:L116` |
| B445 | export-dialog contract: Progress displays actual pixels and cancelling releases the export. | `v1.2.0:apps/desktop/e2e/export-dialog.spec.ts:L146` |
| B504 | Export render options: Motion blur, Include audio, Transparent background and Burn in captions have format/capability-dependent availability. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts` |

### W070 — Render queued composition snapshots and retry failures

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Two queued projects, a failed/cancelled job and active editor project/time.
**Steps:** Enqueue; change source project; relaunch with rendering job persisted; render pending serially; retry/remove jobs.
**Expected:** Queued settings/project snapshot retained; interrupted job becomes pending; shipped queue stops at first non-complete result and restores original editor/time.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-delivery.spec.ts`; `engine-extension-compat.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B499 | Queue snapshot: enqueue captures project plus render options, assigns pending status and shows Added to render queue. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts` |
| B500 | Queue recovery: persist queue; jobs left rendering reopen as pending. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts` |
| B501 | Queue rows: show name, uppercase format, status and error; empty list shows empty-state message. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts` |
| B502 | Queue retry/remove: failed/cancelled jobs show Retry and reset to pending; others Remove; rendering job button disabled. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts` |
| B503 | Render pending: process pending jobs serially; stop at first non-complete result; restore original project/playhead and reopen queue. | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/render-queue.ts` |

### W071 — Convert source/composition/output frame rates

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** 24/30/fractional-rate sources, nested instances and image sequence rate metadata.
**Steps:** Import/probe; place at nonzero trim; set composition and export rates; step at cut/end; export all frames at different rate.
**Expected:** Media time selected at composition/output timestamps; no accidental duration/speed conversion, stale cut frame or missing terminal frame; exact rational output metadata where supported.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/video-timeline.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts`, `v1.2.0:apps/desktop/src/shared/mp4-frame-rate.ts`.
**Existing shipped e2e:** `ae-audit-implementation.spec.ts`; `export-video-frames.spec.ts`; `import-image-sequence.spec.ts`.
**Later candidate e2e (not shipped evidence):** `engine-video-end.spec.ts`.

### W072 — Export the shipped standalone web animation

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Project containing text, effects, images and an inactive missing effect.
**Steps:** Choose web/code output; export; open standalone artifact; compare authored frames and preserved effect animation/source.
**Expected:** Standalone playback and editable/exported definitions as shipped. This tag-only workflow must not disappear silently merely because main later removed it.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/player/export-web.ts`.
**Existing shipped e2e:** `export-code.spec.ts`; `export-missing-effect.spec.ts`.

### W073 — Use numeric, choice, toggle and point controls with keyboard

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Control fixtures with mixed values, disabled state, bounds and live animated getters.
**Steps:** Use every control type; numeric arithmetic/arrows/wheel/infinite drag; point X/Y; ramps/sliders/dials; select typed unknown option; Escape/blur/Enter/Tab; destroy during gesture.
**Expected:** Finite correct values/units, documented clamp-vs-typed semantics, live getter, single transaction, cancelled preview rollback and no listener leaks.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/PointField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.ts`, `v1.2.0:apps/desktop/src/renderer/src/controls/BackTitle.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/Disclosure.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/Row.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/Section.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/Segmented.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/SelectField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/SliderField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/TextField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/ToggleField.svelte`.
**Existing shipped e2e:** `design-language.spec.ts`; `dropdown-toggle.spec.ts`; `infinite-sliders.spec.ts`; `legacy-control-lifecycle.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B337 | infinite-sliders contract: Numeric scrubbing handles relative motion beyond display bounds and undoes as one edit. | `v1.2.0:apps/desktop/e2e/infinite-sliders.spec.ts:L3` |
| B545 | BackTitle: back navigation heading exposes its action and title within inspector subpages. | `v1.2.0:apps/desktop/src/renderer/src/controls/BackTitle.svelte` |
| B547 | Disclosure: expand/collapse button exposes expanded state and controls visibility of child content. | `v1.2.0:apps/desktop/src/renderer/src/controls/Disclosure.svelte` |
| B550 | NumField: drag right/up increases and left/down decreases; 3 px threshold, infinite drag; Shift ×10, Option ×0.1; click/Enter/F2 edits. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B551 | PointField: 2D pad plus X/Y fields; arrows step, Shift ×10, Home resets to clamped zero; drag respects per-axis bounds. | `v1.2.0:apps/desktop/src/renderer/src/controls/PointField.svelte` |
| B553 | Row: shared label associates with child control; supports paired and full-width content and consistent alignment. | `v1.2.0:apps/desktop/src/renderer/src/controls/Row.svelte` |
| B554 | Section: shared section heading and optional action slot preserve inspector hierarchy. | `v1.2.0:apps/desktop/src/renderer/src/controls/Section.svelte` |
| B555 | Segmented: tablist choices support keyboard navigation and animated active indicator. | `v1.2.0:apps/desktop/src/renderer/src/controls/Segmented.svelte` |
| B556 | SelectField: typed option values preserved; mixed/unknown values represented; shared enhanced dropdown supplies listbox behavior. | `v1.2.0:apps/desktop/src/renderer/src/controls/SelectField.svelte` |
| B557 | SliderField: visual slider or angle dial plus numeric value; arrows step, Shift ×10, Home/End choose min/max. | `v1.2.0:apps/desktop/src/renderer/src/controls/SliderField.svelte` |
| B558 | TextField: Enter blurs/commits; live edit Escape cancels; editor shortcuts stay within input. | `v1.2.0:apps/desktop/src/renderer/src/controls/TextField.svelte` |
| B559 | ToggleField: Left/Up false, Right/Down true, Space/Enter toggles; mixed and disabled state supported. | `v1.2.0:apps/desktop/src/renderer/src/controls/ToggleField.svelte` |
| B560 | Numeric arithmetic: accept arithmetic; +5, *2, /4 and -=3 are relative; -2 means absolute negative two. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B561 | Numeric commit: Enter/blur commits changed finite draft; Escape cancels; unchanged draft creates no edit. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B562 | Numeric arrows: Up/Down changes one step, Shift ten; while editing arrows update draft. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B563 | Numeric wheel: horizontal scrub is transactional; Shift ×10 and Option ×0.1; normal vertical scrolling remains available. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B564 | Numeric display: show unit and precision; trim trailing zeroes; Mixed for multi-selection differences. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B565 | Numeric drag cancel: Escape rolls back gesture and releases drag listeners; destroy cancels active scrub. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B566 | Numeric feedback: haptic alignment on changed displayed value, limited to 40 ms intervals. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B567 | Numeric range distinction: scrubs/arrows clamp to min/max; typed finite values follow text commit path without scrub clamp. | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| B568 | binding contract: Reads a live API time source for channel edits. | `v1.2.0:apps/desktop/src/renderer/src/controls/binding.test.ts:L19` |
| B569 | binding contract: Maps layer and content fields to typed commands. | `v1.2.0:apps/desktop/src/renderer/src/controls/binding.test.ts:L29` |
| B570 | binding contract: Clamps work-area endpoints through project and utility APIs. | `v1.2.0:apps/desktop/src/renderer/src/controls/binding.test.ts:L36` |
| B576 | components contract: Refreshes playback readouts at 15 Hz while edits and paused scrubs use live values. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L98` |
| B577 | components contract: Adjusts from horizontal trackpad movement and commits one gesture. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L132` |
| B578 | components contract: Keeps a negative value when the editor is opened and left, and treats a bare minus as a number. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L148` |
| B579 | components contract: Aborts a scrub cleanly when another edit transaction is already live. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L180` |
| B580 | components contract: Scrubs on both axes: right or up raises, left or down lowers. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L194` |
| B581 | components contract: Scrubs through begin → writes → commit and opens editing after a click/cancel. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L211` |
| B582 | components contract: Supports Tab-focus keyboard editing and commits arrows through once. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L256` |
| B583 | components contract: Formats precision/unit, clamps scrubs, and preserves legacy zero-option defaults. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L268` |
| B584 | components contract: Associates a visible Row label with the spinbutton. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L288` |
| B585 | components contract: Re-derives its getter after a document value tick. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L298` |
| B586 | components contract: ToggleField uses the live animated value between playback readout ticks. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L312` |
| B588 | components contract: ToggleField applies the inverted getter value once. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L340` |
| B589 | components contract: SelectField applies the selected typed option once. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L352` |
| B590 | components contract: SelectField displays an unmatched raw value instead of the first option. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L367` |
| B593 | components contract: Starts typing from the live value between playback readout ticks. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L413` |
| B594 | components contract: Uses begin/write/commit and cancels an Escape edit. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L426` |
| B604 | components contract: Commits a continuous slider drag once, then cancels a second drag with Escape. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L695` |
| B605 | components contract: Edits a ramp midpoint by keyboard and restores local state when a stop drag is cancelled. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L716` |
| B606 | components contract: Writes both point axes in a single command batch and supports exact fields. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L730` |
| B607 | components contract: Moves smoothly through normalized point ranges without an authored step. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L738` |
| B608 | components contract: Uses independent point steps anchored to each range minimum. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L749` |
| B609 | components contract: Wraps a positive full-turn dial and uses a track for a narrow angle. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L764` |
| B610 | gesture contract: Command mode drives a typed edit transaction with the label and origin. | `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.test.ts:L23` |
| B611 | gesture contract: Click-without-drag still begins and cancels (edit.cancel fast path). | `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.test.ts:L39` |
| B612 | gesture contract: Once() applies one-shot with the default interface origin. | `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.test.ts:L52` |
| B613 | gesture contract: Local mode never touches edit or history. | `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.test.ts:L66` |
| B614 | gesture contract: Legacy set mode wraps in history. | `v1.2.0:apps/desktop/src/renderer/src/controls/gesture.test.ts` |
| B615 | horizontal-scrub contract: Groups horizontal movement, preserves vertical scrolling, and cancels on disposal. | `v1.2.0:apps/desktop/src/renderer/src/controls/horizontal-scrub.test.ts:L4` |
| B616 | horizontal-scrub contract: Paces haptics and stays quiet when the value cannot change. | `v1.2.0:apps/desktop/src/renderer/src/controls/horizontal-scrub.test.ts:L26` |
| B628 | linked-scale contract: Keys the unchanged axis too when editing an unlinked animated Scale, and cancels both together. | `v1.2.0:apps/desktop/src/renderer/src/controls/linked-scale.test.ts:L16` |
| B629 | linked-scale contract: Keeps the starting scale ratio through zero, keys both axes, and undoes once. | `v1.2.0:apps/desktop/src/renderer/src/controls/linked-scale.test.ts:L36` |
| B630 | num-field-drag contract: Scrubs through the real component and drag helper, locks after the threshold, and undoes one edit. | `v1.2.0:apps/desktop/src/renderer/src/controls/num-field-drag.test.ts:L62` |
| B631 | num-field-drag contract: Cancels the whole scrub when pointer lock is lost, and a click still opens text editing. | `v1.2.0:apps/desktop/src/renderer/src/controls/num-field-drag.test.ts:L77` |
| B632 | pointer-focus contract: Releases a pointer-clicked button, including clicks on its icon, before its action. | `v1.2.0:apps/desktop/src/renderer/src/controls/pointer-focus.test.ts:L11` |

### W074 — Pick, sample and edit solid/gradient colors

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Opaque/translucent, mixed and out-of-sRGB colors; clipped panel.
**Steps:** Open shared picker; edit pad/hue/alpha/notation/space; sample screen; adjust gradient stops/midpoint; copy; restore; Escape/click outside/Enter.
**Expected:** Live preview and alpha, normalized stable saved values, intentional gamut conversion, one Undo and correct overlay/focus restoration.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/FillField.svelte`, `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.ts`, `v1.2.0:apps/desktop/src/renderer/src/controls/RampField.svelte`.
**Existing shipped e2e:** `background-color-drag.spec.ts`; `color-alpha.spec.ts`; `color-picker-overlay.spec.ts`; `text-color-drag.spec.ts`; `user-feedback.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B333 | background-color-drag contract: New project background follows dragging and restores on Escape. | `v1.2.0:apps/desktop/e2e/background-color-drag.spec.ts:L3` |
| B335 | color-alpha contract: Colour alpha from the picker renders see-through and survives Undo. | `v1.2.0:apps/desktop/e2e/color-alpha.spec.ts:L16` |
| B336 | color-picker-overlay contract: Color picker stays above adjacent panels and inside the viewport. | `v1.2.0:apps/desktop/e2e/color-picker-overlay.spec.ts:L4` |
| B346 | text-color-drag contract: Text color follows the pointer throughout a drag. | `v1.2.0:apps/desktop/e2e/text-color-drag.spec.ts:L3` |
| B546 | ColorField: color swatch opens shared picker; mixed values supported; edits use gesture transactions. | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |
| B548 | FillField: edit solid/gradient fill through picker with fallback color and fill transaction. | `v1.2.0:apps/desktop/src/renderer/src/controls/FillField.svelte` |
| B552 | RampField: drag/select color stops and midpoint; arrows ±1 or Shift ±10; Home/End choose endpoints; midpoint clamped 1–99. | `v1.2.0:apps/desktop/src/renderer/src/controls/RampField.svelte` |
| B571 | color-space contract: Parses the notations a person pastes, with their alpha. | `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.test.ts:L5` |
| B572 | color-space contract: Stores six digits while opaque so existing documents keep their values. | `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.test.ts:L16` |
| B573 | color-space contract: Keeps Display P3 colours outside sRGB and clips only when writing hex. | `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.test.ts:L25` |
| B574 | color-space contract: Formats each notation the way the value field shows it. | `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.test.ts:L33` |
| B575 | color-space contract: Holds lightness while reducing chroma into gamut. | `v1.2.0:apps/desktop/src/renderer/src/controls/color-space.test.ts:L40` |
| B595 | components contract: ColorField previews saturation and brightness continuously, then cancels the preview. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L457` |
| B596 | components contract: ColorField removes confirmation buttons and commits the normalized choice when clicking outside. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L499` |
| B597 | components contract: ColorField samples a screen color through the system eyedropper. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L520` |
| B598 | components contract: Mounts color and fill picker overlays above clipped panel contents. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L536` |
| B599 | components contract: ColorField picks in Display P3 and writes the clipped sRGB hex. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L559` |
| B600 | components contract: FillField previews color continuously across pointer moves. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L586` |
| B601 | components contract: FillField shares the redesigned editor, preserves opacity, and cancels its transaction. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L626` |
| B602 | components contract: FillField applies a normalized fill rather than its mutable draft. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L646` |
| B603 | components contract: Keeps the last valid gradient-stop color when the text buffer is invalid. | `v1.2.0:apps/desktop/src/renderer/src/controls/components.test.ts:L668` |
| B617 | legacy-color contract: Unmounts a control added and removed before mutation delivery. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L52` |
| B618 | legacy-color contract: Detects an ancestor added and removed in the same delivery. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L62` |
| B619 | legacy-color contract: Cleans up a same-turn removal through a real MutationObserver. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L69` |
| B620 | legacy-color contract: Preserves fields moved or reattached before delivery and disposes after their final removal. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L76` |
| B621 | legacy-color contract: Waits for initially detached controls to be attached. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L90` |
| B622 | legacy-color contract: Ignores mutations that happened before a field was created inside a detached container. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L99` |
| B623 | legacy-color contract: Preserves a never-connected field added to a container after its document removal. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L111` |
| B624 | legacy-color contract: Distinguishes real ephemeral connection from later detached insertion in the same batch. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L126` |
| B625 | legacy-color contract: Shares one observer and one callback delivery across 200 fields and releases it when empty. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L139` |
| B626 | legacy-color contract: Continues cleaning other controls when one component cleanup throws. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L156` |
| B627 | legacy-color contract: Preserves live color/fill reads, writes and sync updates while attached. | `v1.2.0:apps/desktop/src/renderer/src/controls/legacy-color.test.ts:L166` |
| B633 | Fill modes: Solid, Linear, Radial and None; preview uses a gesture, cancellation restores prior fill, close restores trigger focus. | `v1.2.0:apps/desktop/src/renderer/src/controls/FillField.svelte` |
| B634 | Color picker axes: saturation/brightness pad, hue and opacity controls support keyboard and pointer edits. | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |
| B635 | Color picker tools: screen color sampling, picker settings, color-space choices, notation choice, per-row Copy and restore previous color. | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |
| B636 | Color picker gamut: out-of-sRGB draft shows Clipped and saves nearest sRGB color. | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |
| B637 | Color picker focus: Escape cancels preview; Enter on slider commits/closes; Tab remains inside non-embedded popover. | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |

### W075 — Undo/redo, clipboard and live edit transactions across surfaces

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Project edits, external history closures, open gesture, focused fields and selected effects/keys/layers.
**Steps:** Begin/write/commit/cancel each API; keep begin open while another writer edits; copy/cut/paste/duplicate; undo/redo; pressure/relaunch history.
**Expected:** Per-project stack and API cancel semantics preserved; selection alone does not lose redo; history.cancel keeps edits, edit.cancel reverts owner operations. Native cancellation never removes another writer’s acknowledged change.
**Exact registered keymap entries:** K007 cmd+z → undo; K008 ctrl+z → undo; K009 cmd+shift+z → redo; K010 ctrl+shift+z → redo; K019 cmd+d → duplicate; K020 ctrl+d → duplicate; K023 cmd+c → copyLayers; K024 ctrl+c → copyLayers; K031 cmd+v → contextPaste; K032 ctrl+v → contextPaste; K065 cmd+x → cutLayers; K066 ctrl+x → cutLayers; K120 backspace → delete; K121 delete → delete.
**Native menu key equivalents:** M018 CommandOrControl+V → contextPaste; M020 CommandOrControl+D → duplicate.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/history.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/editing.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`, `v1.2.0:apps/desktop/src/extensions/inspector/effect-clipboard.ts`.
**Existing shipped e2e:** `agent-revision-recovery.spec.ts`; `agent-undo-redo.spec.ts`; `error-presentation.spec.ts`; `figma-paste.spec.ts`; `layer-paste.spec.ts`; `media-paste.spec.ts`; `memory-history-persistence.spec.ts`; `native-panel-text.spec.ts`; `panel-text-entry.spec.ts`; `pro-editor-shortcuts.spec.ts`; `spatial-prompt-paste.spec.ts`.
**Later candidate e2e (not shipped evidence):** `history-gesture-press.spec.ts`.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| A150 | Cmd/Ctrl+z: Run &#96;undo&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A151 | Cmd/Ctrl+shift+z: Run &#96;redo&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A156 | Cmd/Ctrl+d: Run &#96;duplicate&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A158 | Cmd/Ctrl+c: Run &#96;copyLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A162 | Cmd/Ctrl+v: Run &#96;contextPaste&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A179 | Cmd/Ctrl+x: Run &#96;cutLayers&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A204 | backspace: Run &#96;delete&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A205 | delete: Run &#96;delete&#96; in editor context; priority 100; field/repeat/loose-modifier exceptions are listed below. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A290 | Acceptance memory-history-persistence: Compact history preserves pixels, every action, and pending redo through pressure and relaunch. | `v1.2.0:apps/desktop/e2e/memory-history-persistence.spec.ts:L5` |
| A617 | Acceptance agent-undo-redo: Selecting a layer after Undo keeps the completed agent run available to Redo. | `v1.2.0:apps/desktop/e2e/agent-undo-redo.spec.ts:L3` |
| B514 | Undo/redo: Cmd+Z undo and Cmd+Shift+Z redo; editor commands defer to focused editable fields. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B516 | Layer clipboard: Cmd+C copies layers; Cmd+X cuts; Cmd+V pastes contextually; Cmd+D duplicates. | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| B536 | figma-paste contract: Native copy and paste creates editable grouped layers, preserves rendered placement and undoes once. | `v1.2.0:apps/desktop/e2e/figma-paste.spec.ts:L14` |
| B537 | layer-paste contract: Keyboard paste places copied layers at the playhead and undo restores the stack. | `v1.2.0:apps/desktop/e2e/layer-paste.spec.ts:L3` |
| B544 | Effect clipboard clones tag effect type, parameters, enabled state/animation and open state; version/typed-input remapping is later. | `v1.2.0:apps/desktop/src/extensions/inspector/effect-clipboard.ts` |

### W076 — Render, capture and recover authored content correctly

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Synthetic shapes/text/shaders/effects/masks/3D/media at 8/16/32-bit and different scales.
**Steps:** Render paused/playback/export; capture sync/async/nested override; change source/quality while capture pending; exhaust/release caches; lose/recover GPU.
**Expected:** Correct geometry/text/color/timing and shader logical resolution; no stale revisions, deadlocks, leaks or missing extension drawing. Compare standards references; pixels against Electron only for extension/theme content with classified differences.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/compositor.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/raster.ts`, `v1.2.0:apps/desktop/src/renderer/src/kernel/api.ts`.
**Existing shipped e2e:** `adjustment-layer.spec.ts`; `agent-canvas-capture.spec.ts`; `agent-path-mask.spec.ts`; `alpha-async-export.spec.ts`; `alpha-export-performance.spec.ts`; `automatic-preview-memory.spec.ts`; `automatic-preview-preparation.spec.ts`; `boot.spec.ts`; `color-alpha.spec.ts`; `color-depth.spec.ts`; `composited-video-sync.spec.ts`; `composition-playback-profile.spec.ts`; `continuous-rasterization.spec.ts`; `effect-backdrop.spec.ts`; `export-capabilities.spec.ts`; `export-destination.spec.ts`; `export-dialog.spec.ts`; `export-mp4.spec.ts`; `export-streaming.spec.ts`; `export-webm.spec.ts`; `frame-preparation.spec.ts`; `gif-preview-scrubbing.spec.ts`; `gpu-recovery.spec.ts`; `gradient-effect.spec.ts`; `heavy-project-profile.spec.ts`; `large-project-performance.spec.ts`; `native-export-performance.spec.ts`; `opaque-capture-parity.spec.ts`; `playback-performance-benchmark.spec.ts`; `playback-source-warmup.spec.ts`; `png-export-performance.spec.ts`; `precompose-group-appearance.spec.ts`; `preview-resolution.spec.ts`; `preview-responsiveness.spec.ts`; `rendering-performance.spec.ts`; `scene3d-idle.spec.ts`; `scene3d.spec.ts`; `shader-colors.spec.ts`; `size-anchor.spec.ts`; `startup-performance.spec.ts`; `static-effects-export.spec.ts`; `static-export-performance.spec.ts`; `static-image-export.spec.ts`; `static-masks-export.spec.ts`; `static-preview-performance.spec.ts`; `text-render-parity.spec.ts`; `transparent-sequence-scrubbing.spec.ts`; `typewriter-raster.spec.ts`; `video-cut-recovery.spec.ts`; `video-frame-integrity.spec.ts`; `video-instance-preview.spec.ts`; `video-playback-regression.spec.ts`; `video-preview-io.spec.ts`; `video-scrubbing.spec.ts`; `video-texture-readiness.spec.ts`; `webm-export-performance.spec.ts`.
**Later candidate e2e (not shipped evidence):** `color-correction-effects.spec.ts`; `color-space-effects.spec.ts`; `compound-blur.spec.ts`; `consumer-matrix.spec.ts`; `engine-agent-observation.spec.ts`; `engine-color-depth-capture.spec.ts`; `engine-color-depth-expanded.spec.ts`; `engine-compat-context.spec.ts`; `engine-compat-moat.spec.ts`; `engine-definitions-export.spec.ts`; `engine-doc-boot.spec.ts`; `engine-export.spec.ts`; `engine-expression-performance.spec.ts`; `engine-extension-compat.spec.ts`; `engine-frame-painter.spec.ts`; `engine-headless-playback.spec.ts`; `engine-large-project.spec.ts`; `engine-present-recovery.spec.ts`; `engine-render-transport.spec.ts`; `engine-scene3d-chrome.spec.ts`; `engine-shader-first-use.spec.ts`; `engine-source-compositor.spec.ts`; `engine-source-media.spec.ts`; `engine-source-refresh.spec.ts`; `engine-svg-maps.spec.ts`; `engine-svg.spec.ts`; `engine-video-superwhite.spec.ts`; `engine-viewer-canvas.spec.ts`; `engine-viewport-compat.spec.ts`; `export-baseline.spec.ts`; `export-encoding-parity.spec.ts`; `export-sequential-decode.spec.ts`; `import-graded-footage.spec.ts`; `input-shutter.spec.ts`; `layer-compositing.spec.ts`; `legacy-blurred-video-export.spec.ts`; `library-card-parity.spec.ts`; `media-import-readiness.spec.ts`; `native-engine-font-census-perf.spec.ts`; `native-engine-font-seeds.spec.ts`; `native-engine-mount-measure.spec.ts`; `native-engine-p3a-measure.spec.ts`; `native-engine-text-app-parity.spec.ts`; `native-engine-text-level2.spec.ts`; `native-engine-text-typing.spec.ts`; `native-engine-tracer-measure.spec.ts`; `native-engine-tracer.spec.ts`; `native-engine.spec.ts`; `native-media-sequence-parity.spec.ts`; `native-media-sequences.spec.ts`; `native-offline-media.spec.ts`; `native-still-icc-parity.spec.ts`; `native-still-orientation-parity.spec.ts`; `native-svg-broken.spec.ts`; `opaque-effects-export.spec.ts`; `path-mask-playback-measure.spec.ts`; `perfbench-rig.spec.ts`; `project-color.spec.ts`; `scene3d-budget-parity.spec.ts`; `scene3d-compat-parity.spec.ts`; `scene3d-gltf-round7-parity.spec.ts`; `scene3d-import-parity.spec.ts`; `scene3d-normal-matrix.spec.ts`; `scene3d-short-attributes.spec.ts`; `scene3d-warnings.spec.ts`; `shader-color-uniform.spec.ts`; `shader-fallback-readback.spec.ts`; `shadow-ramp-effects.spec.ts`; `spatial-effects.spec.ts`; `stroke-lut-effects.spec.ts`; `temporal-effects.spec.ts`.

### W077 — Keep launch, editing and long playback responsive

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** Public synthetic large/long project fixtures, fixed machine/power/display and controlled run conditions.
**Steps:** Measure paired cold launch/open, random/cached scrub, playback, zoom, gestures, idle, memory and export while retaining correctness assertions.
**Expected:** Native performance gates are met with headroom; caches stay bounded and never change authored output. Existing implementation counters are diagnostic, not an obligation to preserve transport.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/core/projects.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/gl/compositor.ts`.
**Existing shipped e2e:** `automatic-preview-memory.spec.ts`; `automatic-preview-preparation.spec.ts`; `composition-playback-profile.spec.ts`; `editor-responsiveness.spec.ts`; `heavy-project-profile.spec.ts`; `high-zoom-performance.spec.ts`; `large-project-performance.spec.ts`; `pan-presentation.spec.ts`; `preview-resolution.spec.ts`; `preview-responsiveness.spec.ts`; `project-performance-benchmark.spec.ts`; `project-playback-profile.spec.ts`; `region-pan-performance.spec.ts`; `rendering-performance.spec.ts`; `startup-performance.spec.ts`; `static-preview-performance.spec.ts`; `status-polling.spec.ts`; `text-raster-efficiency.spec.ts`; `zoom-gestures.spec.ts`; `zoom-performance-benchmark.spec.ts`; `zoom-raster.spec.ts`.
**Later candidate e2e (not shipped evidence):** `consumer-surfaces.spec.ts`; `effects-idle-cost.spec.ts`; `engine-source-performance.spec.ts`; `export-qos.spec.ts`; `media-store-long-task.spec.ts`; `native-engine-font-census-perf.spec.ts`; `native-engine-font-seeds.spec.ts`; `path-mask-playback-measure.spec.ts`; `perfbench-rig.spec.ts`.

### W078 — Write and compile a custom shader

**Status:** EXISTS in shipped tag (with the scope/corrections stated below).
**Preconditions:** A selected shader layer with uniform controls and an invalid program.
**Steps:** Edit source; indent/outdent with Tab/Shift+Tab; compile with Cmd/Ctrl+Enter; Escape arms focus exit; Tab leaves; animate parameters and undo.
**Expected:** Successful shader draws with authored logical resolution/u_res; compile errors keep editable source and actionable diagnostics; keyboard focus can leave the editor.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/panels/shader/ShaderEditor.svelte`.
**Existing shipped e2e:** `shader-colors.spec.ts`.
**Later candidate e2e (not shipped evidence):** `shader-color-uniform.spec.ts`.

### W079 — Cutover addition: state-dependent J/K/L shuttle

**Status:** CUTOVER ADDITION — absent from shipped tag.
**Preconditions:** Native engine clock available, paused/playing variants and a legacy/unsupported transport fixture.
**Steps:** While playing use J/L for -1/+1 and again -2/+2; K stops; Control-J starts reverse while paused; Control-Shift-Space plays around; repeat under unsupported/paused states.
**Expected:** Commands decline to shipped AE event/property bindings when state disallows shuttle. Never execute both bindings. This is absent in v1.2.0 but explicitly required at cutover.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.
**Later candidate e2e (not shipped evidence):** `native-engine-transport.spec.ts`.

### W080 — Cutover addition: ripple delete and ripple trim

**Status:** CUTOVER ADDITION — absent from shipped tag.
**Preconditions:** Three adjacent clips, a locked downstream clip, source handles and optional linked audio.
**Steps:** Ripple-delete middle; ripple-trim first edge shorter/longer; test blocked locked/handle boundaries; undo/redo.
**Expected:** Delete/trim closes/opens gap by exact delta, intended downstream members move together and rejected operations are atomic. Absent in shipped tag.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/track-view.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`, `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

| Case | Rebased expected result / variant | Tag source or existing contract |
|---|---|---|
| B259 | Ripple coverage: no dedicated ripple-edit timeline tool/shortcut was located in timeline/keymap sources; do not infer it from spatial ripple animation. | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |

### W081 — Cutover additions: roll and slide edits

**Status:** CUTOVER ADDITION — absent from shipped tag.
**Preconditions:** Adjacent A/B/C clips with known handles and animation.
**Steps:** Roll A/B boundary; slide B while compensating neighbors; exhaust each handle; cancel and undo.
**Expected:** Roll preserves combined duration; slide preserves B duration/source while compensating A/C; handles/locks enforced and one Undo. Neither pro operation ships.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

### W082 — Cutover addition: source-monitor three-point editing

**Status:** CUTOVER ADDITION — absent from shipped tag.
**Preconditions:** Source in/out and timeline playhead/range; tracks with existing clips.
**Steps:** Set three points; insert then overwrite video/audio/both; test inconsistent range and missing source; undo/redo.
**Expected:** Resolve fourth point exactly; insert ripples and overwrite replaces interval; preserve A/V sync, source handles and existing authored timing. No shipped source insert/overwrite workflow.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

### W083 — Cutover addition: persistent linked A/V edits

**Status:** CUTOVER ADDITION — absent from shipped tag.
**Preconditions:** Video and its separated audio with explicit native pair identity.
**Steps:** Link/unlink; select/move/split/trim/delete one member; temporarily bypass linking; cancel/undo and save/reopen.
**Expected:** Pair timing stays synchronized, bypass is intentional and linkage survives save; distinct from transform-parent Link Layers. Absent in v1.2.0.
**Exact registered keymap entries:** None assigned; use the listed menu/pointer/API route and the local-key table in w1-keymap.md.
**v1.2.0 implementation/contracts:** `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/shortcuts.ts`, `v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`.
**Existing shipped e2e:** None directly mapped; add a native e2e acceptance test. Unit contracts below are supporting evidence only.

## E2e triage and coverage limits

[e2e-triage.csv](e2e-triage.csv) has one row for each file in the immutable rosters: **396 total**, including **all 370 files in the requested worktree roster**, plus **26 main-only files** needed to rebase the inputs. The tag has **289** e2e specs and main has **313**. All tag specs remain in the 370 roster; its remaining **81** files are later branch tests. Do not equate 370 with shipped coverage.

| Roster | Product | Mixed product + host assertions | Plumbing-only retirement candidates | Total |
|---|---:|---:|---:|---:|
| Requested worktree | 339 | 29 | 2 | 370 |
| Complete union | 365 | 29 | 2 | 396 |

CSV columns record membership in each roster, source revision, workflow IDs, classification, disposition, source test declarations with line numbers and conditional/skip presence. `evidence_revision` pins the full commit hash: tag when available, otherwise main, otherwise worktree snapshot `212a3cc73e87aa189eb55b4daadce4c1a470a38b`. Parameterized test names retain source expressions. File-level mapping means the file contains relevant assertions, not every acceptance variant or a successful run. A skipped environment/corpus-gated test is unproven until its required fixture runs. “Conditional present” is a conservative source flag and does not mean every test is skipped.

Product assertions remain even when setup uses Electron, IPC, DOM selectors or real Chromium input. Mixed specs retain user-visible results and replace obsolete process/transport/counter assertions. Only the two files below are plumbing/quirk-only retirement candidates; even these wait for replacement validation.

- `native-engine.spec.ts`: Only asserts opt-in Electron utility-process/port lifecycle. Retire after in-process replacement and its resource tests pass.
- `scene3d-short-attributes.spec.ts`: Chromium/Three active-attribute, GL error and JS exception probe. Keep as frozen diagnostic evidence only; replace with safe bounds/standards tests, never imitate malformed-input quirks.

Important gaps: the tag has no direct source-preview e2e suite (video-instance-preview provides only part of the interaction); source slip/handle rejection are proved by unit contracts and need native interaction coverage. No tag e2e establishes shuttle, ripple editing, persistent linked pairs, slide/roll or source three-point editing. Later `native-engine-transport.spec.ts` covers shuttle; `spatial-ripple.spec.ts` covers an agent overlay only. Web-export specs remain shipped obligations despite deletion on main.

## Appendix — post-v1.2.0 on main

These rows are removed from the shipped acceptance cases. Preserve the claim and provenance so later behavior can be reconciled without quietly widening “today”. A wholly later test file does not prove the same behavior was absent through every other path; where the tag already has the user goal, its corrected workflow remains above.

| Original case | Main inventory claim | Why it is not a shipped acceptance claim | Main provenance |
|---|---|---|---|
| A017 | Last window: Closing the last editor exits the process; closing one of multiple windows leaves the others running. | On macOS the tag keeps the app alive after the last window; main changes exit behavior. | `main:apps/desktop/src/main/index.ts`, `main:apps/desktop/src/main/windows.ts` |
| A022 | Multi-window quit: Cancelling a later window does not discard an earlier window's approved but still-open project. | New close coordinator protects earlier approvals during later cancellation. | `main:apps/desktop/src/main/editor-close.ts`, `main:apps/desktop/src/main/renderer-recovery.ts`, `main:apps/desktop/src/main/project-save.ts` |
| A027 | Update restart: Coordinate update installation with editor close/save preparation. | Main coordinates update restart with the new close coordinator. | `main:apps/desktop/src/main/updates.ts`, `main:apps/desktop/src/renderer/src/shell/app-updates.ts`, `main:apps/desktop/src/renderer/src/shell/WhatsNew.svelte` |
| A030 | Acceptance background-autosave: Untitled recovery, named background save, conflicts, retry and safe version copy. | New background autosave behavior/spec. | `main:apps/desktop/e2e/background-autosave.spec.ts` |
| A031 | Acceptance background-autosave: Two heavy projects retain pending writes through tab switches, undo, redo and crash. | New background autosave behavior/spec. | `main:apps/desktop/e2e/background-autosave.spec.ts` |
| A036 | Acceptance quit: Quit exits the process after saving the editing session. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A037 | Acceptance quit: Closing the last window exits the process. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A038 | Acceptance quit: Quit exits after discarding unsaved edits. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A039 | Acceptance quit: Closing one of two windows keeps the other window open. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A040 | Acceptance quit: Repeated Quit can be cancelled while preparation waits for imports. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A041 | Acceptance quit: A failed recovery flush keeps edits open and allows Quit to be retried. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A042 | Acceptance quit: Cancelling a later window preserves an earlier approved unsaved project. | New quit spec/close coordinator behavior; no tag spec exists. | `main:apps/desktop/e2e/quit.spec.ts` |
| A052 | Acceptance shell: Tools lead on the left and the project is titled in the middle, however many are open. | Main replaces tab strip with centered single-project title and left tools. | `main:apps/desktop/e2e/shell.spec.ts` |
| A097 | Layer › New › Text: none displayed; dispatch newText. | Layer > New submenu added after tag. Creation commands themselves already exist. | `main:apps/desktop/src/main/menu.ts` |
| A098 | Layer › New › Solid: none displayed; dispatch newSolid. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A099 | Layer › New › Shape Layer: none displayed; dispatch newShape. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A100 | Layer › New › Null Object: none displayed; dispatch newNull. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A101 | Layer › New › Adjustment Layer: none displayed; dispatch newAdjustment. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A102 | Layer › New › Camera: none displayed; dispatch 3d.add-camera. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A103 | Layer › New › Light: none displayed; dispatch 3d.add-light. | Layer > New submenu added after tag. | `main:apps/desktop/src/main/menu.ts` |
| A104 | Layer › Release Pre-comp: none; dispatch releasePrecomp. | Release Pre-comp command/menu is new. | `main:apps/desktop/src/main/menu.ts` |
| A105 | Edit › Delete: none displayed; dispatch contextDelete. | Native Edit > Delete entry/contextDelete is new. | `main:apps/desktop/src/main/menu.ts` |
| A111 | Help › Powermove Help: none; open https://trypowermove.com; Help role supplies system menu search. | Native Help menu/search entry is new. | `main:apps/desktop/src/main/menu.ts` |
| A116 | Panel menu eligibility: Exclude headless/viewer tabs, toolbar, library:false and popped-out panels from Add Panel as Tab; hide Close Panel for viewer and show Close Others only for groups. | Panel tab-adding/close-others eligibility is new. | `main:apps/desktop/src/renderer/src/layout/menu.ts` |
| A118 | Project title menu: List checked open projects, New Project (⌥⌘N), All Projects… (⌘P), Rename Project… (F2), Move to New Window, Close Project (⌘W), Close Other Projects. | Single centered project title menu replaces per-project tabs. | `main:apps/desktop/src/renderer/src/shell/Titlebar.svelte` |
| A119 | Title menu availability: Disable rename while opening; Move to New Window and Close Other Projects require at least two open projects. | New centered title-menu predicates. | `main:apps/desktop/src/renderer/src/shell/Titlebar.svelte` |
| A129 | Spelling menu: Prepend up to five suggestions or disabled No Guesses Found, then Learn Spelling for editable misspellings. | Spelling suggestions/Learn Spelling menu added after tag. | `main:apps/desktop/src/main/text-context-menu.ts` |
| A140 | Timeline column context: Layer Name and Link Layers checked/disabled; Layer Numbers and Switches toggle; Show All After Effects Columns enables both. | Timeline column context and AE column headers added after tag. | `main:apps/desktop/src/extensions/timeline/timeline.ts` |
| A222 | &#96;: Run &#96;maximizePanel&#96;; explicit args in source preserve step direction, frame count or reveal behavior. | Backtick maximizePanel binding is new. | `main:apps/desktop/src/extensions/keymap-default/keymap.ts` |
| A281 | Memory enable: Use project memory defaults on; turning off retains notes but hides them from agents and autosaves without a revision/Undo step. | Settings Memory section is new; project_memory tool already ships. | `main:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A282 | Memory sections: Show Goal, Rules, Open items, Decisions, Rejected ideas and Feedback sorted newest first with author/date/resolved state. | Settings Memory notes display is new. | `main:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| A283 | Storage summary: Show calculation/error states, total bytes, per-category usage and storage folder. | Storage settings/service absent in tag. | `main:apps/desktop/src/renderer/src/settings/StorageSettings.svelte`, `main:apps/desktop/src/main/storage-management.ts` |
| A284 | Storage cleanup: Preview reclaimable storage before cleanup; preserve project data and persist automatic-cleanup and older-backup-budget preferences. | Storage cleanup preferences/preview absent in tag. | `main:apps/desktop/src/renderer/src/settings/StorageSettings.svelte`, `main:apps/desktop/src/main/storage-management.ts` |
| A291 | Acceptance memory-settings: Settings › Memory lists the project notes and switches agent access on and off. | Memory-settings spec absent in tag. | `main:apps/desktop/e2e/memory-settings.spec.ts` |
| A337 | Acceptance storage-settings: Storage previews cleanup, preserves project data, persists preferences and works in both themes. | Storage-settings spec absent in tag. | `main:apps/desktop/e2e/storage-settings.spec.ts` |
| A357 | Compatible provider presets: Offer Ollama, LM Studio, llama.cpp server, Jan, OpenRouter, OpenAI, Anthropic, Google Gemini, Groq, DeepSeek, xAI, Mistral and Other compatible service. | Named provider preset catalog added after tag; compatible connection already ships. | `main:apps/desktop/src/shared/compatible-provider.ts` |
| A359 | Compatible model discovery: Fetch provider models, hide models without tools, retain manually entered model, refresh list, show missing-server/download actions and saved-key state. | Model discovery/catalog controls added after tag. | `main:apps/desktop/src/renderer/src/legacy/ui/compatible-settings.ts` |
| A366 | Panel stacks: Tabbed groups retain active tab, permit reordering, and can close one panel or others in group. | Panel tab stacks/reordering/close-others added after tag. | `main:apps/desktop/src/renderer/src/layout/model.ts`, `main:apps/desktop/src/renderer/src/layout/Dock.svelte`, `main:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `main:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A368 | Maximize panel: Backtick maximizes the panel under pointer and restores its previous layout. | Maximize panel added after tag. | `main:apps/desktop/src/renderer/src/layout/model.ts`, `main:apps/desktop/src/renderer/src/layout/Dock.svelte`, `main:apps/desktop/src/renderer/src/layout/DockLayout.svelte`, `main:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| A437 | Acceptance import-graded-footage: HDR footage is tone mapped once so the editor shows SDR levels. | Graded-footage import correction/spec added after tag. | `main:apps/desktop/e2e/import-graded-footage.spec.ts` |
| A438 | Acceptance import-graded-footage: BT.2020 log footage keeps its code values for grading. | Graded-footage log handling/spec added after tag. | `main:apps/desktop/e2e/import-graded-footage.spec.ts` |
| A452 | Acceptance linked-media-storage: Links originals through import, save, restart, missing-source recovery and relinking; exports remain portable. | Linked-media storage mode/spec added after tag. Original-path cache recovery already ships. | `main:apps/desktop/e2e/linked-media-storage.spec.ts` |
| A737 | System accent: Follow system accent plumbing while retaining explicit/custom theme overrides. | System-accent plumbing added after tag. | `main:apps/desktop/src/main/theme.ts`, `main:apps/desktop/src/main/system-accent.ts`, `main:apps/desktop/src/renderer/src/kernel/theme-apply.ts` |
| B121 | Layer gutter: drag divider to resize layer list. | User-resizable layer gutter added after tag. | `main:apps/desktop/src/extensions/timeline/timeline.ts` |
| B122 | Column menu: Layer Name and Link Layers remain present; Layer Numbers and Switches can toggle; restore all AE columns. | AE column headings/menu added after tag. | `main:apps/desktop/src/extensions/timeline/timeline.ts` |
| B255 | property-sections contract: Names each property after its Properties panel section. | Property-section headings helper/test added after tag. | `main:apps/desktop/src/extensions/timeline/property-sections.test.ts` |
| B256 | property-sections contract: Orders sections like the Properties panel and heads each one once. | Property-section ordering helper/test added after tag. | `main:apps/desktop/src/extensions/timeline/property-sections.test.ts` |
| B415 | Track Matte browser item: special Matte item sets matte source rather than adding shader effect. | Special Track Matte browser item added after tag; layer-menu matte editing already ships. | `main:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte` |

### Mixed-row deltas kept out of shipped behavior

| Case | Tag correction / excluded main delta |
|---|---|
| A084 | Pre-compose is in the Composition menu at the tag; later main adds Layer menu. |
| A091 | No explicit Show All/unhide row exists in tag menu; do not invent a shipped entry. |
| A114 | Tag top-level menu order is Powermove, File, Edit, Composition, View, Window. Layer and Help are later. |
| A115 | Shipped panel menu offers Pop out to window, Close panel except viewer, and extension contributions; maximize, add-as-tab and close-others are later. |
| A133 | Duplicate, Group layers, Ungroup layers for groups, Pre-compose exist; Rename and Release Pre-comp rows are later. |
| A135 | Parent… menu offers None and valid noncyclic candidates; main renames it Link Layers. This is transform parenting, not linked audio/video. |
| A137 | Mute/separate video audio, hide/solo/lock and 3D layer exist; dedicated Continuously Rasterize menu row is later. |
| A268 | Tag sections: Project when open; General, Accounts, Transcription and Extensions. Memory and Storage sections are later. |
| A288 | Keep native About metadata and shipped font/icon license notices; do not assume later Inter font was bundled. |
| A316 | Save/back up/reopen portable media beyond 256 MiB; later main changes storage/version retention. |
| A395 | Tag menus support anchors/coordinates, separators, headers, checks, disabled rows and async contributions; nested menus use explicit follow-up menus. Structured submenu API is later. |
| A398 | Async palette providers and command availability exist; later main adds AE terminology matching. |
| A426 | Keep original sourcePath/fingerprint for missing-cache recovery and portable media saves. A project-wide link-in-place storage mode is later. |
| A661 | EffectsAPI at tag: register, list, get(id). supports* capability flags, parameterDefault and versioned get are post-tag. |
| A697 | Panel metadata includes component/build/header, size/min, flush/noscroll/headless, move-handle/slot, icon/library metadata; fitContent is post-tag. |
| A736 | Preserve shipped public CSS token semantics and theme-authored content; main token/font changes are not the frozen tag. |
| B018 | Tag empty-viewer context offers Paste and Select all layers; Fit Composition and Composition Settings rows are later. |
| B120 | Shift+F3 graph toggle exists; tag chrome offers Dope sheet / combined / graph buttons rather than later single-toggle chrome. |
| B123 | Hide/solo/lock/3D switches exist; preserve actual tag controls rather than later column design. |
| B261 | Tag time readout uses api.util.tc at project FPS; main introduces aeTimecode hours:minutes:seconds:frames formatting. |
| B281 | Tag channel menu supports keyframe, graph, expression and reset paths; Separate Dimensions row and per-axis presentation are later additions. |
| B288 | Shipped matte source/mode editing exists; later main adds remove-matte/toggle and reorganized stack chrome. |
| B544 | Effect clipboard clones tag effect type, parameters, enabled state/animation and open state; version/typed-input remapping is later. |

Other main additions relevant to reconciliation: Memory/Storage Settings; provider preset/model discovery; Layer/Help menus and spelling items; panel stacks/maximize; AE timeline column headings/property-section rows; richer layer menus; versioned effects, temporal/spatial/source/typed input capabilities and LUT assets; graded-footage interpretation and linked media storage mode. Shipped keymap source has no maximize binding; shipped EffectsAPI lacks the later supports* feature flags. Existing shaders retain their logical resolution/u_res; a later processing improvement must prove geometry and sampling correctness rather than copy browser quirks.

### Tag-only workflows removed on main

`export-code.spec.ts`, `export-missing-effect.spec.ts` and `src/renderer/src/player/export-web.ts` exist in v1.2.0 but were removed on main. The standalone web export goal remains W072; deletion is not a rebase reason to drop a shipped workflow.

### Later native-engine branch evidence (not main and not shipped)

The 370 roster contains 81 post-tag branch specs, including the J/K/L transport suite, media-store/linking changes, native rendering/extension/source consumers, document hosts, process/port lifecycle and engine delivery/recovery. CSV membership distinguishes these from main. Use these as candidate native tests where their product contract is valid. The branch-only process bridges are replacement details; browser/GL error quirks remain subject to standards correction.

## Traceability and review checklist

- Original A001–A755 and B001–B637: 1392 unique rows, exactly one rebased workflow case or full post-tag appendix row each; duplicate source claims remain separately traceable.
- Keybindings: K001–K195, exactly 194 default expansion rows plus viewer Enter, with flags/args and workflow IDs. Native menu table has 35 explicit equivalents; local key table has 63 contexts.
- E2e: 370 of 370 requested roster paths and 289 of 289 tag paths mapped; no helper/fixture/script counted as a spec.
- Source research only. No builds, app launches, e2e executions or performance measurements were run for W1. Native execution, frozen trace capture, classified visual differences and lane review are still required gates.
- Frozen references include public source contracts only; isolated user fixtures stay outside tracked artifacts.
