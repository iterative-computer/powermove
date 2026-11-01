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

## Local Mac verification — 2026-11-01

Independent test pass on native-engine base
`ff42d751fbf6a9266cd2ba1a8e25345b60baa5d7`, macOS 27.2 (26B5086k), ARM64.
This records observations and failed/unverified rows, **not design sign-off**.
No visual implementation was changed. Existing review items outside the rows
below retain their earlier status; this is not a complete shortcut, IME, motion,
loaded-project or supported-OS review.

Built `pm-native-app` with Rust 1.96.1, `--locked`, `engine/env.sh`,
`CARGO_TARGET_DIR=~/Developer/pm-target` and `flock /tmp/pm-build.lock cargo …`.
Used the dev bundle and separate `/tmp/pm-macverify/profile-{dark,light}` profiles.
No user project, production profile or Keychain access. Screen-state script
reported unlocked. This lane exclusively created/held `/tmp/pm-shots/gui.lock`
during UI work and removed only that lock after quitting its test apps.

### Requested checklist rows

| Checklist row | Result | Evidence / observation |
| --- | --- | --- |
| §0 Screenshots, light/dark, native chrome | ❌ Partial: all three requested surfaces captured in both appearances; full 2× evidence is not established for the palette composite. | `dark-contrast-{main,palette,sheet}.png` and `light-contrast-{main,palette,sheet}.png`, all under `/tmp/pm-macverify/`. Native app captures via the Mac UI tool, not headless GPUI. Main PNGs are 2880×1864; sheet 960×420; palette composites 1040×668. |
| §0 Same surfaces with Increase Contrast + Reduce Transparency ON | ✅ | Both switches were read OFF initially, enabled and verified ON before the six captures; rechecked ON before restoration. `display-settings-on.{png,txt}`. |
| §0 Before/after evidence | N/A for code changes; supplemental settings baseline captured. | No UI code changed. `light-normal-{main,palette}.png` show the same build after restoring both switches OFF. |
| §0 Interaction-state evidence | ❌ Partial, not the full checklist state set. | Focused sheet field, focused palette/search and arrow-selected result, plus zoom menu/value captured. Hover/pressed/disabled/loading/error states were outside this follow-up. |
| §0 Short keyboard walkthrough | ✅ Evidence recorded; functional failures below block a pass. | Walkthrough below. |
| §0 pm-design tests / no view token literals added | ✅ | Rust 1.96.1: 8 spec + 2 token tests passed (zero failures); `design-test.log`. No view source changed. |
| §2 Reduce Transparency makes glass solid | ✅ for requested surfaces only | Palette and sheet appear solid with both accessibility switches ON; light normal palette baseline also retained. No claim about every other glass surface in the app. |
| §5 Pop-up for viewer zoom | ✅ | Click opened native menu: Fit (41 %), 25 %, 50 %, 100 %, 200 %, 400 %. Clicking 100 % changed the visible header and AX label to `Zoom: 100 %`. `dark-zoom-menu.png`, `dark-zoom-100.png`. No media was loaded, so image scaling is not certified. |
| §6 Full Keyboard Access / Tab order / visible rings | ❌ | Standard Keyboard navigation was already ON. Accessibility Full Keyboard Access was OFF with its switch exposed as disabled; unchanged. Main Tab/Shift-Tab/arrows produced no observable toolbar/panel focus traversal. Sheet Tab/Shift-Tab stayed in Workspace Name in both appearances. Palette Tab dismissed it. A complete toolbar → panels → palette → sheet keyboard-only route was not achieved. |
| §6 Esc / Return | ✅ for tested transients | Esc dismissed palette and cancelled sheet. Return in a sheet with `Mac Verification` entered saved that workspace to the temporary light profile and dismissed the sheet. Other transient types were not tested. |
| §9 Stronger separators/borders with Increase Contrast | ✅ observed | Main-window captures show prominent panel outlines and timeline separators; the restored light baseline shows their normal treatment. Design approval remains with the reviewer. |
| §9 VoiceOver roles, labels, values and actions / actual announcements | ❌ Incomplete | VoiceOver was enabled, but no reliable spoken/caption output could be captured. Details below; AX labels alone are not a VoiceOver pass. |
| Restore original accessibility settings | ✅ | Increase Contrast OFF, Reduce Transparency OFF and VoiceOver OFF verified in System Settings afterwards. `display-settings-restored.{png,txt}`, `voiceover-restored-off.txt`. Caption-panel preference was restored to its original ON value. |
| Quit launched apps / release GUI lock | ✅ | Native test instances, VoiceOver, its welcome process and VoiceOver Utility exited; only this lane's GUI lock removed. System Settings was already running and left open. |

### Keyboard walkthrough and findings

1. Main window: Tab twice, Shift-Tab, Right and Down did not expose a focus
   ring or an AX focused control in toolbar/panels. Control-F5/Control-F2
   attempts did not establish toolbar/menu keyboard focus either. Evidence:
   `dark-main-tab1.png`, `dark-toolbar-keyboard.png`, `light-normal-tab.png`.
   These are observed failures under this automation/system setup; their cause
   was not diagnosed or fixed.
2. Command-K opened the palette with Search focused. Down visibly moved the
   selected row to New Project while the search field retained AX focus
   (`dark-palette-arrow.png`). Tab dismissed the palette instead of walking
   the results. Esc also dismissed it. Typing filtered results; no complete
   palette-to-sheet keyboard route was available in this pass.
3. Opened Window ▸ Workspace ▸ Save Workspace… with the menu to inspect the
   otherwise unreachable sheet. Workspace Name received initial focus and an
   orange ring. Repeated Tab and Shift-Tab did not advance to Cancel/Save
   (`dark-sheet-tab-focus.png`). Esc cancelled. In a second light sheet,
   keyboard text entry followed by Return saved `Mac Verification` in the
   disposable profile; no user workspace was written.
4. Viewer zoom was tested separately by clicking its control, then clicking
   100 %. Its native menu and changed value passed. This does not repair the
   failed keyboard reachability row.

### VoiceOver attempt and available labels

Command-F5 (including a held-key retry) did not start VoiceOver through the
automation path. Used Accessibility ▸ VoiceOver instead, then selected
**Use VoiceOver** in the first-run welcome window. System Settings subsequently
reported VoiceOver ON and the VoiceOver process was running.

The caption-panel preference was already ON. Refreshing it OFF/ON restored that
original setting, but reading the VoiceOver app repeatedly failed with
`Computer Use server error -10005: timeoutReached`; Control-Option navigation
did not yield capturable announcements. No announcement transcript or successful
rotor capture is claimed. Command-F5 also did not turn it off in this setup;
the System Settings switch was used and verified OFF. A reliable VoiceOver
announcement pass remains required.

Supplemental AX inspection (`voiceover-ax-labels.txt`, `voiceover-attempt.png`)
exposed toolbar Sidebar/Inspector buttons (Hide help), Agent, Export, and Tools
radio buttons Select (V), Hand (H), Zoom (Z), Rotate (W), Anchor (Y), Shape (Q),
Pen (G), Text (⌘T). Panel containers were labelled Project, Viewer, Timeline,
Properties and Effects & Presets; panel tabs, Add Panel, Resize dock, Play,
Workspace: Default and Zoom: 100 % were present. The sheet exposed Workspace
Name, Cancel and Save. These are accessibility-tree labels, **not words confirmed
as announced by VoiceOver**.

All screenshots/raw UI evidence remain exclusively under `/tmp/pm-macverify/`
and are not committed. The keyboard and VoiceOver rows remain open; this lane
does not approve shell 3 for merge.
