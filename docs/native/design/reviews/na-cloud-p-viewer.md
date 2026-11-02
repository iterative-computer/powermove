# Design review: `feat/na-cloud-p-viewer`

Reviewer: design lead · 2026-11-01 · Lane head `17f87f117` (diff vs
`origin/feat/native-engine`). Read against
[panels/viewer-chrome.md](../panels/viewer-chrome.md), [panels/README.md](../panels/README.md),
the UI-kit rules and the op-tree snapshots
(`panels/viewer/tests/__snapshots__/panel.test.tsx.snap`).

**Verdict: APPROVE as an integration candidate — merge
`origin/feat/na-design-fix-p-viewer` (`4b0908b67`), not the lane head.** The
lane head lost the zoom and resolution controls in Focus/full screen and
never drew the lost-canvas button; both are fixed there. Gizmos, banners and
the overlay's glass look remain recipe work (items 9–11).

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §2 bar | No title; source leading content; zoom then resolution trailing; `c.panel` bar | APPROVE |
| 2 | §2 copy | `Fit (54 %)`, `54 %`, `12.5 %`, `Auto (Half)`, menu items exact | APPROVE |
| 3 | §2 Overlay | Focus/FullScreen rendered **no** zoom/resolution controls | **Fixed** — same pop-ups (same ids) in a `ViewerOverlay` group |
| 4 | §2 Lost canvas | `canvasLost` was metadata only; no button | **Fixed** — `Return to Composition` (`viewfinder`) → `viewer.fit` |
| 5 | §7 VoiceOver | `Back to composition` AX label ≠ visible `Back to Composition`; no symbol | **Fixed** |
| 6 | README §2 | Bar had no trailing inset | **Fixed** — `metrics.gap` (see D1) |
| 7 | §3 empty | Bar hidden, `c.viewer` surround, `No Composition` / `Open a project to start editing.` / `Open…` | APPROVE (push-button recipe pending, as Settings) |
| 8 | AX value | Canvas value `{name}, {zoom} percent, {res} resolution, {n} layers selected` | APPROVE (+ D2/D3 fixes) |
| 9 | §2 menus | Zoom menu lacks separators and key equivalents; `Auto` lacks its detail line | CHANGES REQUIRED — kit `Option` gains `separator`, `keyEquivalent`, `detail`; lane fills them |
| 10 | §2 Gizmos, Banners | Not rendered (metadata `pendingRecipe`) | CHANGES REQUIRED — kit viewer-overlay/gizmo recipes; `Banner` must enter the Rust publication (`ViewModel` has no banner field yet) — [surfaces/](../surfaces/README.md#3-banner-and-notice) now relies on it |
| 11 | §2 Overlay look | Group is structural; no glass, idle fade or dark scheme | CHANGES REQUIRED — kit `ViewerOverlay` capsule recipe |
| 12 | Fixed widths | Pop-up widths jump with the title | CHANGES REQUIRED — kit `PopUpButton.widthSample` (D1) |

## DESIGN-TODO(opus) answers

- **D1 Tokens.** Added to [panels/README §6](../panels/README.md#6-token-additions-and-changes-requested):
  `metrics.gizmo_handle` 7, `metrics.gizmo_keyline` 1,
  `metrics.rotation_stem` 18, `metrics.anchor_mark` 11,
  `metrics.tangent_end` 6, `metrics.checker` 8,
  `metrics.pasteboard_opacity` 0.35, `motion.overlay_idle` 2.0. Fades use the
  existing `motion.enter` / `motion.exit` / `motion.reduced`. Keylines on the
  image use `colors.media_badge`; marquee and text selection over the image
  use `colors.selected_text` / `colors.accent` with the kit's alpha parameter,
  not new colours. Fixed pop-up widths are **measured**, not tokens: the kit
  gets `widthSample: "Fit (100 %)"` / `"Auto (Quarter)"` and sizes to that
  string + chevron. Bar trailing inset = `metrics.gap` (the exact optical
  formula gives 6.5 pt, off the 2 pt grid; `m.gap` is within 0.5 pt). Light
  `colors.viewer` → `#E4E4E4` is owed by pm-design (README §6), not this lane.
- **D2 Intermediate quality.** Named steps keep names; any other effective
  scale shows as a percentage: title `Auto (75 %)`, spoken
  `Preview resolution, Auto, currently 75 percent`. Implemented.
- **D3 Unnamed composition.** Spoken `Untitled Composition`; the authored
  empty name is never rewritten. Implemented.
- **D4 Playing source.** One toggle button: `Play` while paused, `Pause`
  while playing (label flips; no separate value), Space toggles.
- **D5 Focus across presentations.** The controls keep their ids, so focus
  stays on the same control when it moves between the docked bar and the
  overlay. While keyboard focus is inside the overlay it does not idle-fade.
  `Back to Composition` has no overlay counterpart: if it had focus, focus
  goes to `viewer.canvas`. Implemented structurally; the shell must preserve
  focus by id across the presentation change.

## Not design (for the lane)

On macOS, `service.test.tsx` (`UI -> v4 host data -> Rust model -> actual
kernel ack`) fails: the `v4_wire` example calls the kernel from the process
main thread and `pm-kernel` panics (`never block the main thread on the
kernel`, `actor.rs:157`). Run the example's kernel calls on a worker thread.
Pre-existing on the lane head; Linux CI does not see it. The same
`/private/tmp` eval-path failure as the Settings lane also appears.
