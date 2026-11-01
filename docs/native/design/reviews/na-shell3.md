# Lane record: `feat/na-shell3` (shell 3, review fixes)

Lane self-check for the design lead; visual items are **not** self-certified.
Base: `feat/na-shell2` + `feat/native-engine`. Evidence (not committed):
`/tmp/pm-shell3/{before,after}-{dark,light}-*.png`, captured with temp profiles
through the perf gate (`redraw`, and the new test-only `shell` op that drives
the palette, drags, the sheet and tab menus) while the screen was locked;
child windows and menus are composited onto the window capture.
`after` runs use the dev bundle (`apps/native/scripts/bundle-dev.sh`).

| Capture | What it shows |
|---|---|
| `1-main` | Main window with a project (`-empty-1-main`: no project) |
| `1b-edited`, `1c-saved` (after only) | `Edited` under the title; title alone, centred, after the save |
| `2-palette` | Command palette |
| `3-drag-edge`, `4-drag-tab`, `5-drag-split` | Timeline dragged to the left edge, onto the Properties header, onto its lower half |
| `6-save-sheet` | Save Workspace sheet |
| `7-tab-menu` | Properties tab context menu |
| `8-restored-tabs` | Two windows restored as one native tab group |

## Review items

### `na-shell2` must-fix

| # | Item | Status |
|---|---|---|
| 1 | Blue system accent | Done. Dev bundle declares `NSAccentColorName` with an `AccentColor` asset (`#F0580A` light / `#FF6019` dark); unbundled runs set the same dynamic colour on NSApp. The perf gate reads `controlAccentColor` back: `#F0580A` / `#FF6019`. Sidebar/Inspector are plain toolbar items (`sidebar.leading` / `sidebar.trailing`), tooltip Show/Hide. Focus rings and the sheet's default button are orange. |
| 2 | Save Layout is a centred dialog | Done in part. It is now a real window-modal sheet (`beginSheet`): `heading` title, one `reading` line, Name field, Cancel + orange default **Save**; Return = Save, Esc = Cancel; panel material. **Placement:** on this OS (macOS 27.2, 26B5086k) AppKit centres every sheet in its window, ignoring `window:willPositionSheet:usingRect:` and frame changes, even for a plain AppKit window (probe output: `/tmp/pm-shell3/sheet-placement.txt`). "Below the toolbar" needs a design decision: accept system placement, or a non-system attached panel. |
| 3 | Developer placeholders | Done. Shell empty states (`layout::empty_state`): heading + reading line + action (`No Media` / Import…, `No Selection`, `No Effects`, …); no panel ids. App: Properties `No Selection`, Project `No Recent Projects`; status says `No project` instead of an instruction. |
| 4 | Viewer title, no controls | Done for the title and zoom pop-up (`Fit (41 %) ▾` in the 32 pt header). Quality pop-up still blocked on an engine preview-resolution control (see na-viewer2 #5). |
| 5 | Drag preview tints the whole panel | Done. Header strip (join), half (stack), outer 25 % (edge); `accent_wash` fill, 2 pt `accent` outline, `control_radius`. Source tab ghosted, chip kept. |
| 6 | Layout vs Workspace | Done. `Window ▸ Workspace` (presets with checkmark, saved, Save Workspace…, Delete Workspace, Reset Workspace), palette `Workspace: …`, tab menu, sheet, status and errors all say workspace. |
| 7 | Checkmark on the current dock | Done (`Dock Right ✓`). The viewer (centre column) has none. |
| 8 | App name | Done. Process name and bundle dictionary name are `Powermove` for unbundled runs; the dev bundle's `CFBundleName` is `Powermove`. Window title is `Untitled`, not `… — Powermove Native`. |

### `na-shell2` should-fix / nice-to-have

| # | Item | Status |
|---|---|---|
| 9 | 32 pt headers | Done: `metrics.header` 32 for panel headers, viewer bar and timeline header (one row each now). |
| 10 | Palette | Done: `NSSearchField` glyph, fixed trailing shortcut column at `menu_inset`, panel shrinks to its results. |
| 11 | Status bar | Done: no Metal dot, no instructions; workspace is a pop-up (`Default ⌃⌄`). |
| 12 | Tab close at the leading edge | Done (SF `xmark`, on hover, tab groups). |
| 13 | Header `+` | Done: SF `plus` at `symbol_panel`, `hit` target, hover fill, `secondary` → `text`, tooltip `Add Panel`. |
| 14 | Title centring | Done (also viewer2 #11). |

### `na-viewer2` notes for this lane

| Item | Status |
|---|---|
| Panels supply their own header | Done: `Integration::header`. Viewer: zoom pop-up trailing, no title. Timeline: play (SF `play.fill`/`pause.fill`), native timecode, fps. The app's extra bars are gone; `fit_scale` centres the image in the body below the bar with an `inset` margin. Tab groups keep their strip. |
| `Saving…` subtitle | Done: `document_subtitle(edited, saving)`; observed `Saving…` during a perf-gate save, then nothing. |
| SF Symbol element | Done: `pm_shell::symbol` (AppKit-rasterised, tinted, cached per scale); kind badges in Project and Timeline rows use `kind_symbol`. |
| Lock badge for copy-opened projects | Not done (P3 banner work). |

### Other lane items

| Item | Status |
|---|---|
| `Tokens::apply_patch` on theme load | Done: `pm_shell::theme` applies a `nativePatch` on every token build (launch, appearance and accessibility changes); `PM_NATIVE_THEME` file or the `shellTheme` command; `theme::native_tokens` gives `environment.nativeTokens`; pm-design gained `Tokens::field`. |
| Native tab bar for restored tab groups | Done. Cause: GPUI-CE's `addTitlebarAccessoryViewController:` override hides AppKit's tab bar at 0 pt (Zed draws its own tabs). The shell re-adds the accessory through NSWindow's own implementation and joins tabs on the next run-loop turn. The perf gate reports `tabs: 2, tab_bar: true`; before: no bar. `Window ▸ Show Tab Bar` on a lone window still goes through GPUI's override. That needs a vendor patch row. |

## Checklist (lane self-check)

- §0 Evidence: light/dark native captures and before/after pairs ✅. Increase Contrast / Reduce Transparency captures ❌: not taken; that needs a system setting change on the owner's Mac. Keyboard walkthrough ❌ (screen locked; no real input). `cargo test -p pm-design` ✅.
- §1 Tokens/colour: new view code uses tokens; `DROP_OUTLINE` (2 pt, spec value) and `TAB_BAR_HEIGHT` (AppKit metric) are named constants ✅. No blue next to orange ✅. Accent only as a mark (drop wash is `accent_wash`) ✅.
- §2 Materials: palette glass unchanged; sheet content on panel material ✅. Light palette over a black viewer reads dark grey (glass shows the backdrop); review this.
- §3 Layout: header 32, `hit` targets for `+`/play, empty states centred ✅.
- §5 HIG: system sheet ✅ (placement: see #2); menus with checkmarks ✅; app name ✅; title centring ✅; pop-ups for zoom/workspace ✅.
- §7 Copy: Title Case headings, sentence-case lines, `…` on commands that ask for input ✅; no internal words ✅.
- §9 Accessibility: new controls have roles and labels (`Add Panel`, `Close …`, `Workspace: Default`, `Workspace Name`) ✅. VoiceOver capture ❌.
