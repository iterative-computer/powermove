# Design review: `feat/na-shell2` (native shell, second pass)

Reviewer: design lead · 2026-11-01 · Evidence: `/tmp/pm-shell2/{before,after}-*.png`
(native AppKit captures, dark, 2×) and `headless-*.png`. The branch had no
pushed commits at review time; this reviews the working tree's captures.
Read against [language.md](../language.md) and the
[review checklist](../review-checklist.md).

**Result: good structure, changes required.** The arrangement matches the
Electron app, the toolbar is a real `NSToolbar` with a glass tool capsule and
an orange Export, menus are real `NSMenu`s with Title Case and correct `…`,
the Save Layout title is Title Case. The issues are colour discipline,
placeholders, sheet placement and a few HIG placements.

## Must-fix

1. **Blue system accent next to orange.** Sidebar and Inspector toggles render
   as blue filled capsules (`after-1-default.png`), the palette search field
   and the Save Layout text field have blue focus rings, and the sheet's
   default button is blue. Set the app's AccentColor (asset catalog
   `AccentColor` + `NSAccentColorName`, or `NSApp` appearance tint) to
   `colors.accent` so every system control tints orange. Sidebar/Inspector
   toggles should be **plain toolbar buttons** that show state through their
   symbol (`sidebar.leading` / `sidebar.trailing`), not a filled tint — Apple's
   own sidebar toggles are untinted.
2. **Save Layout is a centred dialog, not a sheet.** `after-10-save-layout-sheet.png`
   shows a panel centred in the window. Use a window-modal sheet
   (`beginSheet`) so it attaches below the toolbar; Return = Save (orange),
   Esc = Cancel.
3. **Developer placeholders in the UI.** `TEMPORARY · PHASE 1`, `fxbrowser`,
   `No projects in this isolated library. Use File → Open…`, `Click a track to
   seek`. Replace with empty states per language §9/§14: centred, `heading` +
   one `reading` line + one action, e.g. Project: "No Media" / "Import files or
   drag them here." [Import…]; Properties: "No Selection" / "Select a layer to
   see its properties."; Effects & Presets: the real list or "Loading effects…".
   Never show panel ids or "isolated library".
4. **Viewer has a title and no controls.** Remove the `Viewer` header text;
   the 32 pt viewer bar holds trailing pop-up buttons `Fit (54 %) ⌄` and
   `Auto ⌄` (`NSPopUpButton` or GPUI equivalents). `Fit 50% 100%` as three
   plain words reads as static text.
5. **Drag preview tints the whole panel orange** (`after-7-drag-to-left-edge.png`).
   Highlight only the **drop zone**: the header strip when joining a tab group,
   the upper or lower half when stacking, the outer 25 % when docking to an
   edge — `accent_wash` fill with a 2 pt `accent` outline at
   `control_radius`. Keep the source panel dimmed and the floating tab chip
   (good).
6. **Layout vs Workspace vocabulary.** The context menu says `Save Layout…`
   and `Reset Workspace`; the sheet says `Window ▸ Layouts`. Pick
   **Workspace** (Electron's term, used in the status bar `Default`):
   `Save Workspace…`, `Reset Workspace`, `Window ▸ Workspace`.
7. **Context menu state.** `Dock Left / Dock Right / Dock Bottom` should mark
   the current dock with a checkmark (or disable it).
8. **App name in the menu bar** shows the launching process (`T3 Code
   (Nightly)` in `after-9-tab-context-menu.png`). Set `CFBundleName` /
   process name to **Powermove** for dev runs too, so captures and menus are
   correct.

## Should-fix

9. **Panel header height** — pm-design now sets `metrics.header` = 32 (was 36);
   rebase and confirm headers, the viewer bar and the timeline header share it.
10. **Palette.** Add a `magnifyingglass` glyph in the search field; right-align
    shortcuts in a fixed trailing column with `menu_inset` padding (they sit
    ~28 pt inside the row's right edge today); shrink the panel to its
    results (empty space under 7 results); selected row `selected_fill`
    (fine) with the row's text unchanged.
11. **Status bar.** `Open a project to begin` is an instruction; the status bar
    holds facts. Show nothing (or `No project`) on the left; keep `Metal`
    (with its green dot only when it means something to users — otherwise move
    it to Settings ▸ About this Mac-style diagnostics) and `Default` workspace
    as a pop-up.
12. **Tab close glyph** (`Properties ×`) appears at the trailing side next to
    the title; Mac tabs show the close button at the leading edge on hover.
13. **Header `+` buttons** need tooltips naming the action (`Add Panel`,
    `Import…`) and a `hover` fill; they are bare glyphs at 11 pt.

## Nice-to-have

14. Title + `Edited` subtitle: good. When there is no subtitle the title must
    centre vertically in the toolbar (see viewer2 captures, where it rides
    high).
15. Dark panel/window contrast is right (neutral, `#1A1A1A` on `#111111`).
    Keep it when the token changes land; do not reintroduce borders on panels
    in dark.

## Evidence still needed

Light-mode, Increase Contrast and Reduce Transparency **native** captures of
the toolbar and palette (glass vs solid), and a Full Keyboard Access
walkthrough of panel tabs and splitters (checklist §0, §6).
