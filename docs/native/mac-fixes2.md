# Composition navigation and Studio catalog reconciliation

The viewer now binds Tab to `composition.flowchart` with main's priority 0,
field admission, exact modifiers and no key-repeat admission. The AppKit focus
route lets canvas Tab reach that resolver; controls retain focus movement, as
required by Full Keyboard Access. The command can decline so a field receives
its original key. View → Composition Flowchart also opens the chart.

The native popover follows [the existing specification](design/surfaces/main-delta-ui.md#3-timeline-layers-column-u04-u05-u06-e12):
Used In, the current composition and Contains, with the specified empty states,
semantic tokens and system `rectangle.stack` symbols. Tab or Escape dismisses
it; clicking a composition performs a kernel-validated navigation call. Direct
precomp links are deduplicated and missing composition links are omitted.

Composition navigation changes the canonical `compId` without replacing any
composition content or creating an undo entry. It refuses while an edit is open,
pauses playback, restores valid layer selections when revisiting, and rebinds
both viewers to the destination's dimensions, duration and frame rate. The app
retains playhead, timeline and viewer navigation state per live composition.

Source behavior is pinned at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`:
`extensions/viewer/index.ts`, `extensions/viewer/viewer-chrome.ts` and
`renderer/src/legacy/core/compositions.ts`, under `apps/desktop/src/`.

The MCP contract is pinned main **plus** the two previously selected Studio
contracts (`edit_3d` and `get_3d_scene`) from `b81b94c66`. Production already
publishes this composition. Acceptance now compares that exact contract, while
keeping the bare main source snapshot unchanged. Additional assertions retain
all main operations, base schema properties, unrelated tools, allowlists,
instructions, command fields and easing presets. The Studio fixture is also
checked against its source by `mcp/scripts/scene3d-catalog.mjs`. Studio rendering
is outside this change.

Validation is headless. Native display/capture acceptance remains in the existing
recorded integration blockers; no new visual qualification is claimed. No
performance tolerance, timing deadline or blocker classification is changed.

Strict acceptance completed with **1,682 passes, zero failures and 2,662
recorded blockers**, from 1,672 / 10 / 2,662. Configured legacy compatibility
remains 1,467 / 0 / 3,136. Strict exit 2 reflects the existing blockers. All
140 Rust prerequisite tests and eight Python tests pass, as do the affected
kernel, shell, MCP, builtins and app suites and all-target Clippy. The existing
audio cold-start timing miss still requires a quiet Mac against the unchanged
deadline; this change does not qualify that timing.
