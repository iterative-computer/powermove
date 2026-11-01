# Native UI review checklist

Every lane that changes what Powermove Native looks like or how it is operated
passes this gate before merge into `feat/native-engine`. The design lead
reviews; a lane may not self-certify visual items. Spec:
[language.md](language.md). Values: [tokens.md](tokens.md).

Mark each item ✅, ❌ (blocks merge) or N/A with a reason. Paste the filled
list into the lane's PR or `docs/native/design/reviews/<lane>.md`.

## 0. Evidence (required, or the review does not start)

- [ ] **Screenshots** of every changed surface in **light and dark**, at 2×,
      saved under `/tmp/<lane>/` (never committed). Native AppKit captures
      (`screencapture -l <window>`), not only GPUI headless renders, for any
      surface with system chrome, glass or menus.
- [ ] The same surfaces with **Increase Contrast** and **Reduce Transparency**
      on (headless renders are acceptable for solid GPUI surfaces).
- [ ] **Before/after** pairs when changing an existing surface.
- [ ] One capture of each **interaction state** that changes appearance:
      hover, pressed, focused (keyboard), selected, disabled, editing, empty,
      loading, error.
- [ ] A short **keyboard walkthrough** (text log is fine): which keys reach
      which controls.
- [ ] `cargo test -p pm-design` passes; no token literals were added to views.

## 1. Tokens and colour

- [ ] Every colour, size, radius, duration and font comes from
      `pm_design::Tokens` (grep the diff for `0x`, `rgb(`, `px(` literals in
      view code; layout math on token values is fine).
- [ ] Dark surfaces are neutral grey; nothing is tinted blue or warm.
- [ ] The accent appears only as a mark (playhead, selection ring, set
      keyframe, active tool, primary button, focus). No large accent fills,
      no accent-filled selected clips or rows.
- [ ] System controls are tinted Powermove orange (AccentColor), not the
      user's system blue. No blue next to orange anywhere.
- [ ] Floating surfaces are lighter than panels; panels lighter than window;
      the viewer surround is the deepest neutral.
- [ ] Semantic colours (danger/warning/success) carry meaning only and are
      paired with a symbol and words.
- [ ] Text uses the three steps (text / secondary / muted), not opacity.

## 2. Materials (HIG)

- [ ] Glass only on: toolbar, tool capsule, viewer overlays, popovers,
      sheets (frame), menus, palette/field menu (`Tokens::material`).
- [ ] No glass under dense content (inspector, lists, timeline, agent
      conversation, code, task lists). No glass on glass.
- [ ] Reduce Transparency turns every glass surface solid.
- [ ] Shadows only on floating surfaces, drawn by the system.

## 3. Layout, alignment and density

- [ ] Arrangement matches the Electron app (left Project / Effect Controls,
      centre viewer, timeline under both, full-height right dock, status bar).
- [ ] Panel gaps `gap`, insets `inset`, header `header`, rows `row`.
- [ ] Inspector values share **one** value column (`value_column`), aligned
      right edge at the inset, left-aligned values, units in `muted`.
- [ ] Nothing is clipped by a panel edge; long text truncates with `…` and has
      a tooltip.
- [ ] Baselines align across a row and across the status bar (left and right
      items share one baseline).
- [ ] Every metric on the 2 pt grid; radii concentric (control = panel − inset).
- [ ] Hit targets ≥ `hit` (24 pt), even for 11 pt glyphs.
- [ ] Empty states are centred in their panel, `heading`/`reading` sizes,
      with one clear action; no developer placeholders (`TEMPORARY`, panel ids,
      `fxbrowser`).

## 4. Typography and numbers

- [ ] SF Pro (`TEXT_FONT`); SF Mono only for code/expressions/logs.
- [ ] No text under 11 pt; sizes are token roles.
- [ ] Values, timecode and counters use tabular figures; they do not shift
      width while scrubbing or playing (record a short capture).
- [ ] Units after a space (`34 px`, `100 %`), multiplication sign in sizes.
- [ ] No all-caps labels, no letter-spacing on UI text.

## 5. HIG placement and platform behaviour

- [ ] Window chrome is system: traffic lights, NSToolbar, title/subtitle
      (`Edited`), full screen, native tabs.
- [ ] Document title is single-line-centred when there is no subtitle.
- [ ] App name in the menu bar is **Powermove** (bundle name), never the
      launching process's name.
- [ ] Sheets attach below the toolbar (system sheet), not centred dialogs.
- [ ] Menus and context menus are real `NSMenu`s; state shown with checkmarks;
      every command also reachable from the main menu.
- [ ] Settings via ⌘, only; no Settings gear in the toolbar.
- [ ] Pop-up buttons for value choices (zoom, quality, font, blend), not chip
      rows.
- [ ] Popovers anchor to their control, apply live, dismiss on Esc/outside.
- [ ] One prominent (accent) button per sheet/toolbar; default button named
      for its action; destructive default is Cancel.

## 6. Keyboard

- [ ] Every Electron shortcut for the surface still works (compare
      [w1-keymap.md](../w1-keymap.md)); no Mac convention silently replaced one.
- [ ] Full Keyboard Access: Tab reaches every control in visual order; the
      focus ring is visible on every focused element (GPUI draws `focus`).
- [ ] Esc cancels the innermost transient (field edit → popover → sheet →
      drag); Return activates the default button.
- [ ] Menus show the shortcuts the resolver actually uses.
- [ ] Text fields keep IME, dictation, spelling and Emoji & Symbols.

## 7. Copy

- [ ] Title Case: menu items, buttons, window/sheet titles, panel/tab names,
      command, property and effect names.
- [ ] Sentence case: explanations, placeholders, tooltips' detail line, status,
      errors, empty states, descriptive section headings.
- [ ] `…` (U+2026) on commands that ask for more input; `▸` in prose menu paths;
      no `→`, no `...`.
- [ ] No internal or developer words (`Writable`, `isolated library`,
      `fxbrowser`, error codes) in visible text; errors say what happened and
      what to do.
- [ ] Mac vocabulary: Settings, Move to Trash, Show in Finder.

## 8. Motion

- [ ] Only state changes animate; springs/fades use `motion` tokens.
- [ ] Reduce Motion replaces springs and scales with the `reduced` cross-fade.
- [ ] Interrupting an animation does not jump.

## 9. Accessibility

- [ ] Contrast spot-check of any new colour pair (the pm-design test covers
      tokens; new pairings in views need a manual check ≥ 4.5:1 text, 3:1 marks).
- [ ] Increase Contrast captures show stronger separators and borders.
- [ ] VoiceOver: every control has a role and label; values read with units;
      custom GPUI controls expose value and actions. Record the VO rotor
      output or an Accessibility Inspector screenshot.
- [ ] Information never conveyed by colour alone.

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| | | | |
