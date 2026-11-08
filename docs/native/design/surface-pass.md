# Surface pass (2026-11-08)

Short notes on the per-surface design pass: what changed and which tokens it
uses. The governing rules stay in [language.md](language.md).

## 1. Editor shell

- **Tabs.** Quiet text tabs replace the stretched segmented strip: `title`
  size, `medium_weight`, active `text`, others `muted` (`secondary` on hover),
  a `hover` pill at `control_radius` and `hit` height only under the pointer.
  Each tab is as wide as its title; tabs that do not fit fold into More
  Panels and the active tab always stays. The leading close slot is
  `inset − gap/2`, so the first title sits on `inset` like a single title.
- **Focus.** `colors.focus` is neutral grey in every scheme (owner rule: no
  accent focus rings). Increase Contrast raises it.

## 2. Timeline outline

- Layer names stay on one line and end in an ellipsis; the tooltip has the
  full name.
- The disclosure column is always reserved (`hit`), so kind badges and names
  line up whether or not a layer has children.

## 5. Project list

- The host's one-column layout renders quiet source-list rows: `row` height,
  name in `body`/`text`, the duration trailing in `minimum`/`muted` with
  tabular figures, or the media status (`Loading…`, `Media offline`) in its
  tone. No thumbnail box, badge or glyph. Selection is `selected_fill`, inset
  by `gap`. Hover actions take the trailing slot. The full metadata line is
  the row's tooltip.
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
  alpha; solid `separator` under Increase Contrast); selection stays the
  `text` ring.
- The playhead head is the rounded pin (top 3, bottom 6) the render op
  already described; the painter drew it as a diamond.

## 3. Viewer bar

- The zoom and preview-resolution pop-ups are borderless title pop-ups
  sized to their title (`ControlSpec::inline`, viewer panel only), as in the
  owner's demo; inspector and sheet pop-ups keep the bezel at `value_column`.
- The Fit row is titled `Fit (54 %)`, so the closed pop-up names the zoom.
