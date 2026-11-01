# Design review: `feat/na-viewer2` (viewer + save)

Reviewer: design lead · 2026-11-01 · Evidence: `/tmp/pm-viewer2/after-0-*.png`,
`headless-*.png`. The branch had no pushed commits at review time.
Read against [language.md](../language.md) §9–§11, §14.

**Result: functional evidence is good (open → MCP edit → save round-trip);
visual changes required before merge.**

## Must-fix

1. **Selected clip is a solid accent bar** (`after-0-3-during-save.png`:
   "Flight edit" track is a full orange fill). The accent is a mark: draw the
   clip in its label colour (`labels.*`, text layers default blue) and show
   selection as a 1.5 pt `text` ring with a slightly brighter fill. A solid
   orange clip hides keyframes and competes with the playhead.
2. **Inspector values overflow the panel.** `100.00` in Opacity runs past the
   right panel edge; wells have different widths (`100.00` vs `0.00`). Use one
   `value_column` (128 pt) ending at `metrics.inset` from the panel edge;
   values left-aligned, tabular figures.
3. **Units belong to values.** `Opacity %` → label `Opacity`, value `100 %`;
   Position shows `0 px`, not `0.00`. Show the property's display precision
   (Electron rounds to its step: integers for px, 1 decimal for %, unless the
   value has more).
4. **Viewer surround is tinted.** The empty viewer in `after-0-3` is a
   blue-grey (`≈#1C1C21`), and in `after-0-1` pure black edge to edge. The
   surround is `colors.viewer` (`#0E0E0E` dark / `#E6E6EA` light); the
   composition frame is drawn at its real bounds (checkerboard or comp
   background) with a 1 px `separator` outline. Never a coloured surround.
5. **Zoom chips over the image.** `Fit` / `50%` / `100%` are three filled
   chips overlaid on the top-right of the image. Replace with one zoom pop-up
   (`Fit (54 %) ⌄`) and a quality pop-up in the viewer bar (language §10).
   Remove the `Viewer` title.
6. **Timeline needs its anatomy.** No ruler, no timecode in the timeline
   header, playhead without a head, track label column without a background.
   Implement §11: 32 pt header with play button and timecode (`timecode`
   13 medium tabular), 24 pt ruler, layer column on `panel_secondary`, 1.5 pt
   playhead with an 11 pt head. Timecode leaves the status bar (it is
   `00:00:00:00` bottom-right today, on a different baseline from the left
   status text).
7. **Status copy.** `Opened · Writable` is developer language. Status bar:
   `Saved` / `Edited`, then layer and keyframe counts; nothing about
   writability (a read-only project gets a lock badge in the title and an
   explanation in a banner).

## Should-fix

8. **Project list selection** hugs the text width (`Flight edit` chip).
   Rows are full width, `selected_fill`, `row` height, with a kind symbol
   (`film`, `textformat`, …) instead of the generic lines glyph.
9. **Inspector helper text** `Drag a value to adjust. Click to type.` should
   be a tooltip on the wells (first-run hint at most), not permanent text.
10. **Selected layer header** in Properties (`Flight edit` in a filled bar)
    should follow the v2/v3 pattern: name in `title` weight with the kind in
    `muted` (`Flight edit · Video`), no fill.
11. **Title alignment.** With no `Edited` subtitle the document title rides
    high in the toolbar; centre it vertically.

## Nice-to-have

12. During save, show progress in the title subtitle (`Saving…`) rather than
    only the status bar; it reverts to nothing on success.
13. Capture the viewer with real media in light mode too; the surround and
    frame outline need checking against `#E6E6EA`.

## Viewer 3 — 2026-11-01

Implemented by the design lead on `feat/na-design2` in the temporary
panels (`apps/native/crates/app`), on top of the merged viewer-2 lane
(virtualised rows kept). Evidence (not committed):
`/tmp/pm-design2/{before,after}-{dark,light}-{1-opened,2-selected}.png`,
captured with temp profiles through the perf gate's new `redraw` op
(paints while the screen is locked).

| # | Item | Status |
|---|---|---|
| 1 | Selected clip | Done. Clips use `labels.*` by kind (tokens.md mapping), 20 pt in a 28 pt row, `inner_radius`, name inside; selection is a 1.5 pt `text` ring with fill mixed 12 % towards `text`. No accent fill anywhere. |
| 2 | Inspector overflow | Done. One `value_column` (128) well per row ending at `inset`, left-aligned, tabular. |
| 3 | Units on values | Done. `Opacity` / `100 %`, `Position X` / `0 px`; % to 1 decimal, px to 2, trailing zeros trimmed. |
| 4 | Viewer surround | Done. Surround stays `colors.viewer` (shell). Fit now leaves a margin so the composition reads at its real bounds; a 1 pt `separator` outline is drawn outside the frame when the comp background is transparent or within 24/255 of the surround. The blue-grey/black "surround" was the composition filling the viewer edge to edge. |
| 5 | Zoom chips | Done in part. One native pop-up `Fit (37 %) ▾` (Fit, 25–400 %) in a 32 pt viewer bar. Quality pop-up **not done**: the engine has no preview-resolution control (`ViewerConfig`); add one, then a `Auto ▾` pop-up next to zoom. `Viewer` title removal is shell-owned (below). |
| 6 | Timeline anatomy | Done. Header with play button, native timecode (`timecode` 13 medium, tabular, `text`) and fps in `muted`; 24 pt ruler (major ticks `secondary` with `minimum` labels, minor `muted`, click/drag to scrub); layer column 232 on `panel_secondary` with index and label badge; 1.5 pt accent playhead with an 11 pt rounded head in the ruler. Timecode left the status bar. |
| 7 | Status copy | Done. `Saved · 3 layers · 2 keyframes` / `Edited · …`; copy-open reasons in user language (`Opened as a copy because the file is read-only.`) until the P3 banner. |
| 8 | Project rows | Done except symbols: full-width `row` height rows, `selected_fill`, label badge per kind. SF Symbol kind glyphs need a symbol element in GPUI (shell/vendor). |
| 9 | Helper text | Done: tooltip on the value wells. |
| 10 | Selected layer header | Done: `Flight edit · Shape`, title semibold + `muted`, no fill. |
| 11 | Title alignment | Not done: shell-owned. |
| 12 | `Saving…` subtitle | Not done: shell-owned (`set_document_metadata` only takes `edited`). |
| 13 | Light capture | Done (`after-light-*`). |

Needed from the shell lane (`feat/na-shell3`; not edited here):

- The viewer and timeline panels draw the shell's tab row *and* the app's
  own 32 pt bar. Let `Integration` supply header content for a panel
  (viewer: zoom/quality pop-ups trailing, no title; timeline: play,
  timecode, fps), then the app drops its extra bars. Once the viewer bar
  is the header, centre the image in the rect below it so `fit_scale` can
  stop reserving the bar's height at the bottom too.
- Title centring with no subtitle (item 11) and a subtitle string API for
  `Saving…` (item 12); a lock badge for copy-opened projects.
- An SF Symbol element usable from GPUI panels (item 8, and every list).
