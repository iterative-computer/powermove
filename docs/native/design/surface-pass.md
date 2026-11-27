# Surface pass (2026-11-08)

Short notes on the per-surface design pass: what changed and which tokens it
uses. The governing rules stay in [language.md](language.md).

## 1. Editor shell

- **Tabs.** Quiet text tabs replace the stretched segmented strip: `title`
  size, `medium_weight`, active `text`, others `muted` (`secondary` on hover),
  a `hover` pill at `control_radius` and `hit` height only under the pointer.
  Each tab is as wide as its title; tabs that do not fit fold into More
  Panels and the active tab always stays. The leading close slot is `inset`
  and tab groups start flush, so the first title sits on `inset` like a single
  title; the hover × is `symbol_inline − 3`.
- **Focus.** `colors.focus` is neutral grey in every scheme (owner rule: no
  accent focus rings). Increase Contrast raises it. The palette, field menu
  and sheet text fields turn the system focus ring off (it is drawn in the
  orange AccentColor); the bezel and caret show focus, as Spotlight's field.
- **Translucency.** Light mode lets more of the desktop through
  (`translucency.window` 0.62, `panel` 0.66, was 0.8/0.72); dark keeps the
  owner's demo values.
- **Status line.** Trailing format facts (`1920×1080 · 30 fps`, tabular,
  `muted`) before the workspace pop-up, through
  `Integration::status_trailing`.
- **Panel windows.** Settings and Export get a transparent title bar band
  (`metrics.titlebar` 28) on their own material, so the window reads as one
  surface; the agent popover has no title bar at all.

## 2. Timeline outline

- Layer names stay on one line and end in an ellipsis; the tooltip has the
  full name.
- The disclosure column is always reserved (`hit`), so kind badges and names
  line up whether or not a layer has children. Index, disclosure and badge
  never shrink; the name takes the rest (`flexGrow`). Rows are inset by `gap`.
- Selection and hover fill the layer column only; in the tracks the clip's
  accent ring marks the selection (owner's demo; language §11 updated).
- The timeline's selection count and announcements are status labels for
  VoiceOver, not text drawn over the rows (v1 shows the count in the
  inspector, which still does).

## 5. Project list

- The host's one-column layout renders quiet source-list rows: `row` height,
  name in `body`/`text`, the duration trailing in `minimum`/`muted` with
  tabular figures, or the media status (`Loading…`, `Media offline`) in its
  tone. No thumbnail box, badge or glyph. Selection is `selected_fill`, inset
  by `gap`. Hover actions take the trailing slot. The full metadata line is
  the row's tooltip.
- Missing files read `Missing` in `danger` at `minimum`; the path is in the
  tooltip and the relink flow, never in the row. Index and trailing never
  shrink; the name truncates.
- A multi-column layout keeps the card grid (thumbnail, duration badge, name,
  metadata), for when the host supplies posters.

## 2b. Timeline header

- The view switch (Dope Sheet / Dope Sheet and Graph / Graph Editor) is a
  three-glyph segmented control (`chart.bar.doc.horizontal`,
  `rectangle.split.1x2`, `point.topleft.down.to.point.bottomright.curvepath`)
  with the names as tooltips, so the header reads as transport + tabs, not a
  row of long labels. Layers | Tracks stays text.

## 2c. Timeline tracks

- The track area no longer paints an opaque `colors.panel` ground: it sits on
  the shell's panel material like every other panel (the `gutter` keeps
  `panel_secondary`).
- Ruler ticks are `text` ink at 0.14 (minor) and 0.3 (major); a 0.04 grid
  line runs under the tracks at each labelled time. Increase Contrast keeps
  solid `separator`/`secondary` ticks and no grid.
- Unselected clips get a hairline keyline (`Tokens::edge` at twice its
  alpha; solid `separator` under Increase Contrast); selection is the
  `accent` ring.
- The playhead head is the rounded pin (top 3, bottom 6) the render op
  already described; the painter drew it as a diamond.

## 3. Viewer bar

- The zoom and preview-resolution pop-ups are borderless title pop-ups
  sized to their title (`ControlSpec::inline`, viewer panel only), as in the
  owner's demo; inspector and sheet pop-ups keep the bezel at `value_column`.
- The Fit row is titled `Fit (54 %)`, so the closed pop-up names the zoom.

## 4. Properties and Effect Controls

- Number wells draw their unit after the value in `muted`, `inner_radius`
  apart (`1920 px`, `73.53 s`, `100 %`), in both the leased native field and
  the static well; long values truncate, the unit stays.
- Every well in a row's value slot fills the shared `value_column`; two-part
  values split it with a 4 pt (`inner_radius`) gap. A trailing control
  (Scale's link, now drawn with `link`/`link.slash`) sits outside the column
  and the wells give up its width, so the column's leading edge never moves.
- Host-menu triggers in the value column (Font, Fill) are wells: `field`
  height, `field` fill, `control_radius`, text from the leading edge.
- Section headings start on the row-label inset; the chevron is trailing,
  `muted`, and shows on hover or while collapsed.
- The layer heading (`clip.mp4 · Video`, `42 Layers · Mixed`) gets `gap`
  above it, clear of the header divider.

## 6. Agent popover

- The Agent window is a borderless popover framed natively
  (`pm_shell::popover_frame`): `Surface::Popover` material under a clear
  GPUI view (glass on macOS 26, `NSVisualEffectView` popover before, solid
  `floating` with a `separator` edge under Reduce Transparency), corners at
  `metrics.popover_radius` (16), system shadow. The window no longer paints
  `colors.panel`. Plan §3.10 puts the agent composer on glass.
- No title bar or traffic lights. It enters with the half-bounce spring
  (`motion.enter`, `bounce`, from `start_scale`), the palette's animator.
- The conversation takes the free height and the composer sits at the foot;
  its choices (access, provider, model) are borderless title pop-ups after a
  `plus` attach button, as in the owner's demo.

## 7. Export and 9. Settings

- Both keep the Settings-sheet anatomy (grouped cards, label + description,
  trailing controls) in a window with the seamless title band. Export is
  still a window, not a system sheet: presenting a GPUI window as a sheet
  changes modality and its end path, which belongs to the behaviour lanes.

## 8. Project Library window

- Full-size content under a transparent title bar. The sidebar is a source
  list on the window ground (`Tokens::ground` over the behind-window blur,
  solid `window` under Reduce Transparency) with a `line` edge, starting
  below the traffic lights (`toolbar`). Section rows are `row` high.
- The content column is solid `panel`: the selected section's name as a
  `heading` in the title band, then 28 pt rows, name truncating, facts in
  `minimum`/`muted` tabular figures.
- GPUI's own `MacosWindowBackground::Blurred` panics in its layer update
  (`blurred_view_update_layer`, a non-unwinding panic that aborts the app).
  The Library uses a transparent window over the shell's behind-window blur
  (`pm_shell::window_backdrop`) instead; nothing should use `Blurred`.

## Review fixes (Sol review, 2026-11-08)

- `role="status"` is live in every v4 panel: a status that appears or changes
  after the first frame is posted as an NSAccessibility announcement and maps
  to the status role. Timeline refusals (key collision, outside the
  composition, locked layer) also stay visible as a `floating` capsule at the
  bottom-left of the tracks; confirmations are spoken only.
- The timeline's `ground` op is the track material: the timeline tokens'
  `colors.panel` (a theme's track colour, swapped by reversed surfaces) at
  `translucency.panel`, opaque under Reduce Transparency. The shell fills only
  the header of a panel that paints its own ground.
- Narrow docks: the header's own tools (one hit, or two hits and a gap for a
  panel's actions) and More Panels keep their room; the More Panels chevron
  sits outside the clipped tab strip; only an active tab wider than the room
  truncates (`TabTrack::active_limit`).
- Selected rows use `colors.selected_secondary` and `colors.selected_danger`
  (≥ 4.5:1 on `selected_fill`, spec-tested in every variant).
- Ruler ticks and grid read `translucency.tick_minor`, `tick_major`, `grid`.
- Layer column: native default `metrics.layer_column` 320 with an edge
  hairline (drag it; double-click resets). v1 wrote its default `gutterWidth`
  224 into every workspace, so 224 means "default"; any other width is kept.
  Unset switches show on hover or focus only.
