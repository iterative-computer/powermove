# Powermove Native design language

Status: **governing spec** for every visual lane of the native rewrite.
Owner: design lead. Values live in `pm-design` (see [tokens.md](tokens.md));
the gate is [review-checklist.md](review-checklist.md); the reference picture is
[editor-mockup-v3.html](editor-mockup-v3.html). Per-panel build specs live in
[panels/](panels/README.md) (pictures: [panels-mockup.html](panels-mockup.html));
where a panel spec is more specific than this page, the panel spec wins.

Powermove Native is the **same Powermove**, refined so that it is at home on
the Mac. It is not a redesign. Where the two disagree:

- **Arrangement** follows the Electron app: Project / Effect Controls on the
  left, the viewer in the centre, the timeline beneath both, Properties over
  Effects & Presets in a full-height right dock, status bar at the bottom.
- **Placement and look** follow the Apple Human Interface Guidelines (HIG):
  window chrome, toolbar, menus, sheets, popovers, focus, materials, type.
- **Behaviour and shortcuts** follow the Electron app. A Mac convention never
  silently replaces a Powermove shortcut; if the two conflict, the Powermove
  binding stays and the Mac binding is added only where it is free.

Sources: `packages/tokens/tokens.css` and `apps/desktop/css/app.css` on
`main` (tokens win the cascade), the shipped `v1.2.0` layout, the Nov 2026
design commits (sheets on the panel material, Title Case dialog names, aligned
value columns, neutral-grey dark), the owner-approved
[editor-mockup-v2.html](editor-mockup-v2.html), [T1](../t1-themes.md) and the
running Electron app.

---

## 1. Principles

1. **The work is the colour.** The UI is neutral grey; footage, artwork and
   the one orange accent carry all the colour. No tinted chrome, no gradients
   on work surfaces, no decorative colour.
2. **One accent, used as a mark.** Orange marks *the thing that matters now*:
   the playhead, the selection ring, set keyframes, the primary button, the
   active tool. It is never a large fill behind content.
3. **Solid where you work, glass where you reach.** Panels, the viewer, the
   timeline, the inspector and the agent conversation are opaque. Liquid Glass
   is reserved for controls that float above content: the toolbar, viewer
   overlays, popovers, sheets, menus and the palette.
4. **Dense, aligned, calm.** A pro editor shows a lot at once. Density comes
   from tight, consistent rhythm (2 pt grid, shared columns, one row height),
   never from smaller type. 11 pt is the floor.
5. **Numbers never jump.** Every value, timecode and counter uses tabular
   figures and a fixed column, so scrubbing and playback read as motion, not
   jitter.
6. **System first.** Use the AppKit control, menu, sheet, popover, focus ring
   and SF Symbol before drawing our own. Draw our own only for surfaces AppKit
   does not have (timeline, viewer gizmos, scrubbable values), and then match
   the system's metrics.
7. **Same behaviour, quieter surface.** Every Electron behaviour and shortcut
   survives. What changes is chrome: fewer borders, fewer custom buttons,
   system placement.
8. **Accessible by construction.** Contrast, Increase Contrast, Reduce
   Transparency, Reduce Motion, full keyboard access and VoiceOver labels are
   token- and test-enforced, not afterthoughts.

---

## 2. Window anatomy

```
┌ traffic lights · Sidebar · Title/Edited ·· [ tool capsule ] ·· Agent · Tasks · Export · Inspector ┐  52 pt unified toolbar (glass)
│┌ Project | Effect Controls ┐┌ viewer bar (Fit (54 %) ⌄)(Auto ⌄)┐┌ Properties ────────┐│
││                            ││                               ││                       ││  panels: one material, 8 pt radius,
│└────────────────────────────┘│                               ││                       ││  6 pt gaps and window edge
│┌ Timeline ─────────────────────────────────────────────────┐│├ Effects & Presets ───┤│
││ ▶ 00:01:25:13 · 30 fps            header (38 pt)          │││                       ││
│└───────────────────────────────────────────────────────────┘│└───────────────────────┘│
└ status bar (24 pt): Saved · 133 layers · 11 keyframes        Paused · 1920×1080 · 30 fps ┘
```

- **Toolbar** is an `NSToolbar` in a unified title bar (`metrics.toolbar` =
  52 pt). Leading: system traffic lights, Sidebar toggle, document title with
  `Edited` subtitle (title menu gives Rename, Move, Duplicate, Revert, as in
  any document app). Centre: the tool capsule (Select V, Hand H, Zoom Z,
  Crop/Anchor, Orbit, Shape, Pen, Text — Electron order and keys). Trailing:
  Agent, Background Tasks (only while a task runs), **Export** (the one
  prominent orange button), Inspector toggle.
- **No Settings gear, no Home button** in the toolbar: Settings is
  `Powermove ▸ Settings…` (⌘,) and the project library is
  `File ▸ Project Library` plus the Welcome window, as Mac document apps do.
  Workspaces (Default / Animation / Effects …) move from toolbar tabs to a
  `Window ▸ Workspace` submenu and a toolbar pop-up the user can add via
  Customize Toolbar; their Electron shortcuts are kept.
- **Docks** keep the Electron arrangement and default sizes (left 300 pt,
  right 300 pt, timeline 344 pt, Effects 268 pt; see `pm_design::workspace`).
  Panels are separate cards on the window ground with `metrics.gap` (6 pt)
  between them and to the window edge; `metrics.inset` (12 pt) is the
  content inset inside a panel.
- **Status bar** (`metrics.status` = 24 pt) sits on the window ground, not in
  a panel: quiet `muted` text, left = document facts, right = playback and
  format facts. Nothing in it is a primary control.
- **No panel touches a window corner** (toolbar above, status bar below), so
  panel radii only need to be concentric with their own contents.

---

## 3. Materials

| Surface | Material | Why |
|---|---|---|
| Toolbar, tool capsule, toolbar buttons | Liquid Glass (`NSGlassEffectView`, macOS 26); `NSVisualEffectView` `.titlebar` before 26 | HIG: glass is the control layer above content |
| Viewer overlays (zoom/quality pop-ups in Focus, 3D status capsule, Light panel frame) | Glass, small, clear variant | Floats over the image; content scrolls/plays beneath |
| Popovers (colour, easing, gradient, zoom, layer blend) | System `NSPopover` (glass on 26) | Transient, anchored to a control |
| Menus, context menus, palette, field menu | System menus; palette/field menu are glass child panels | System behaviour |
| Sheets (Save Layout, New Composition, Export, Composition Settings) | System sheet frame (glass); content on **panel** material in cards | Electron's Nov 2026 rule: dialogs are cut from the panel material. The system owns the frame; we own the content. |
| Alerts (destructive confirmations, Full Access) | `NSAlert` | Exact system look and keyboard behaviour |
| Window ground, status bar | `translucency.ground` at `translucency.window` over a behind-window blur (`NSVisualEffectView`, `.underWindowBackground`); solid `colors.window` under Reduce Transparency (`Tokens::ground`) | The owner's demo: a heavy tint, so only a hint of the desktop comes through. The tint is darker than `colors.window` because the blur lightens it, so panels still step lighter than the ground |
| Panels, inspector, lists, timeline, agent conversation | One shared material: `colors.panel` at `translucency.panel`, a `text` hairline edge (`translucency.edge`) | Not glass: no refraction, and the tint keeps dense text legible. Panel bodies never repeat the fill |
| Viewer surround | Solid `colors.viewer`, the deepest neutral | Footage must read true; no material tint |

Rules:

- **Never** glass under dense content (inspector rows, lists, timeline,
  conversation, code). If a floating surface holds dense content (Background
  Tasks list, a large popover), its content area is solid `colors.floating`
  inside the glass frame.
- **Never** stack glass on glass. A popover from a toolbar item is fine (it is
  one surface); a glass button inside a glass popover is not — use a plain
  borderless button there.
- **Reduce Transparency** → every glass surface becomes solid
  (`colors.floating`, or `colors.window` for the toolbar band), and the window
  ground and panel material become opaque (`translucency.*` = 1, no blur).
  Enforced by `Tokens::material` and `Tokens::translucent`.
- **Increase Contrast** → glass gets the system's high-contrast border; GPUI
  borders double (`metrics.border` = 2), panel edges and dividers become solid
  `separator` (`Tokens::edge`, `Tokens::line`) and secondary text steps up.
- Elevation in solid surfaces is tonal, not shadowed: window → panel → field →
  floating each step lighter in dark. Floating surfaces (`colors.floating`)
  stay **lifted above panels** in both schemes; never set them equal to the
  panel colour. Only floating surfaces cast a shadow, and the system draws it.

---

## 4. Colour

All values: [tokens.md](tokens.md). Two schemes; the app follows the system
appearance unless the user picks one in Settings ▸ General.

**Dark is neutral grey**, R = G = B, from the deepest viewer surround
(`#0E0E0E`) through window `#111111`, panel `#1A1A1A`, field `#262626` to
floating `#2C2C2C`. No warm Stone, no blue-grey. (Test: `dark_is_neutral_grey`.)

**Light keeps Powermove's cool ink** (`#1B1D23` text on `#FDFDFE` panels over
an `#EEEEF1` window), as the Electron light theme does.

**Text** has three steps: `text` (primary), `secondary` (row labels),
`muted` (captions, units, metadata, placeholders). All three pass 4.5:1 on
every surface they appear on; secondary passes 7:1 under Increase Contrast.
Do not dim text with opacity — use the step.

**Accent.** `#FF6019` in dark, `#F0580A` in light (the Electron light accent;
white bold labels on it pass 3:1 for large text). Use:

| Use | Token |
|---|---|
| Fill of the primary button (Export, default button of a sheet) | `accent` + `selected_text` |
| Playhead, selection ring, set keyframes, active tool glyph, snapping guide | `accent` |
| Accent as text or a 1 pt mark on a panel (links, "Edited" badge, active tab underline) | `accent_text` (≥ 4.5:1) |
| Drop target, active guide bed | `accent_wash` (opaque) |
| Keyboard focus ring | GPUI surfaces draw the neutral `focus` ring; never the accent (owner rule, 2026-11-08) |

The app's **AccentColor asset is Powermove orange**, so system controls
(focused field ring, default button, checkboxes, switches, slider fill)
tint orange — never the user's blue system accent next to our orange
Export button. List and table **selection uses `selected_fill`** (neutral),
because an orange row fill would compete with the artwork; the selected
item's marks (ring, diamond, thumbnail outline) carry the accent.

**Semantic**: `danger` (offline media, errors, destructive buttons),
`warning` (missing-but-recoverable, proxy-only), `success` (rendered, online).
Never use them decoratively, and never alone: pair with an SF Symbol and words.

**Over the image.** Anything drawn *on top of* footage or artwork — viewer
gizmo keylines and labels, the Focus/full-screen overlay capsule, viewer
banners, the source-preview transport, badges and hover buttons on Project
thumbnails — uses the **dark-scheme** tokens in both appearances (plus
`media_badge` for badge fills). Media is dark on average, and light chrome on
a black frame reads as a hole; this is how QuickTime and Photos treat
controls over content. The panel chrome around the image (viewer bar,
surround) follows the app appearance.

**Light viewer surround** is pure neutral (`viewer` `#E4E4E4`, R = G = B),
so footage reads true in light mode too; the composition frame keeps its
1 px `separator` outline when its background is transparent or close to the
surround. (Changed 2026-11-01 from the cool `#E6E6EA`; see
[panels/README.md §6](panels/README.md#6-token-additions-and-changes-requested).)

**Labels** (clips and layers): eight muted fills (`labels.*`) — blue, violet,
orange, red, yellow, green, teal, gray — tuned so `text` on them is ≥ 7:1 and
so a timeline full of clips stays quieter than the viewer. Defaults by layer
kind are in the token roles; users can relabel (Electron behaviour).

---

## 5. Typography

SF Pro via `.SystemUIFont` (automatic optical sizes); SF Pro Display only for
≥ 20 pt; SF Mono only for code, expressions, shader source and logs.

| Role | Size / weight | Where |
|---|---|---|
| `minimum` | 11 / regular | Captions, ruler labels, status bar, units, thumbnails metadata. Nothing smaller, ever. |
| `body` | 12 / regular | Dense UI: values, list rows, layer names, effect names, menus drawn in panels |
| `label` | 12 / regular, `secondary` | Inspector row labels |
| `title` | 13 / semibold | Panel titles and tabs, inspector section headings, document title |
| `reading` | 13 / regular, 1.45 leading | Prose: agent replies, sheet explanations, Settings descriptions |
| `timecode` | 13 / medium, tabular | Transport timecode, current-time field |
| `heading` | 15 / semibold | Sheet titles in custom sheets, Settings pane titles, empty-state headings |
| `display` | 22 / semibold, SF Pro Display | Welcome window, onboarding, large empty states |

- **Numbers** (values, timecode, frame counts, sizes, percentages, hex colours)
  use SF Pro with **tabular figures** (`NSFont.monospacedDigitSystemFont`,
  CSS `font-variant-numeric: tabular-nums` in compat surfaces). This replaces
  the Electron app's Geist Mono numbers: same steadiness, at home on the Mac.
- **Units** follow the number after a space, in `muted`, inside the well:
  `34 px`, `100 %`, `0°`, `1.2` (leading is a unitless multiplier).
- Leading: `dense_leading` 1.3 for UI, `reading_leading` 1.45 for prose.
- Weights: regular 400, medium 500 (tabs, timecode, a value being edited),
  semibold 600 (titles, headings, primary buttons). No bold, no light.
- Never letter-space UI text; never all-caps labels (the Electron
  `TEMPORARY · PHASE 1`/`SAVED` style caps go away — see §15).

---

## 6. Spacing, grid and density

- **2 pt grid** for every metric; most rhythm in 4s.
- `gap` 6 between panels and to the window edge; `inset` 12 inside panels.
- `header` 38: panel header, viewer bar and timeline header share one height
  (the owner's demo), with a `line` divider under panel headers.
- `row` 28: inspector rows, list rows; a `field` (22) sits centred in it.
- `track_row` 28 / `property_row` 22 / `ruler` 24 in the timeline.
- `control` 28 for standalone buttons and pop-ups; toolbar capsules 34
  (`capsule_radius` 17 = height ÷ 2).
- `hit` 24: any glyph smaller than 24 × 24 gets a padded hit area.
- **Density** (Settings ▸ General ▸ Density: Compact / Normal / Comfortable,
  kept from Electron) moves lengths relative to these natives through the
  theme pipeline (compact takes 4 pt off rows and 2 pt off gaps). It never
  changes type below `minimum`.

---

## 7. Corner radii (concentric)

| Element | Radius |
|---|---|
| Window | System (macOS 26 toolbar windows) |
| Panel | `panel_radius` 8 |
| Control inside a panel (field, button, segmented track, thumbnail) | `control_radius` 6; content sits `inset` 12 in, clear of the panel corner |
| Control inside a control (segment, chip in a field) | `inner_radius` 4 |
| Toolbar capsule, toolbar button group, tags | height ÷ 2 (capsule) |
| Popovers, sheets, menus | System |

A shape's radius equals its container's radius minus the inset between them.
If an inset differs, compute the radius; do not reuse a token that no longer
nests. Thumbnails in the Project grid use `control_radius`; timeline clips use
`inner_radius` (they nest in 28 pt rows with 3–4 pt insets).

---

## 8. Iconography

- **SF Symbols** everywhere AppKit or GPUI draws a glyph. Custom glyphs only
  for Powermove concepts SF Symbols lacks (Powermove mark, keyframe diamond,
  easing curves, pen/anchor tools), drawn on the SF Symbols template grid and
  exported as custom symbols so they scale and weight-match.
- Sizes: `symbol_toolbar` 15 (regular), `symbol_panel` 13 (medium) for header
  buttons, `symbol_inline` 11 (regular) inside rows. Weight matches adjacent
  text.
- Rendering: monochrome, `secondary` at rest, `text` on hover, `accent` when
  the glyph represents an active state (active tool, keyframe set, record).
  Hierarchical rendering for status symbols (`checkmark.circle`,
  `exclamationmark.triangle`) in their semantic colour.
- Suggested mapping: Select `cursorarrow`, Hand `hand.raised`, Zoom
  `plus.magnifyingglass`, Sidebar `sidebar.leading`, Inspector
  `sidebar.trailing`, Agent `sparkles`, Export `square.and.arrow.up`,
  Background Tasks `circle.dashed` progress, visibility `eye`/`eye.slash`,
  lock `lock`/`lock.open`, audio `speaker.wave.2`, add `plus`, search
  `magnifyingglass`, effect `fx` → `wand.and.rays` (keep the Electron `fx`
  mono badge only in compat surfaces).
- Layer-kind badges in the timeline and Project list use symbols, not
  letters: text `textformat`, shape `square.on.circle`, media `film`,
  image `photo`, audio `waveform`, adjustment `slider.horizontal.3`,
  null `circle.dashed`, camera `camera`, group `square.stack`.

---

## 9. Controls

### Inspector rows (Properties, Effect Controls, composition sheets)

```
◇  Label ............  [ value          unit ]   ← one shared value column
   12 pt 'secondary'      field 22 pt in a 28 pt row, width value_column
```

- Leading 9 pt **keyframe diamond** (`metrics.keyframe`) in its own column:
  outline `muted` = not animated, filled `accent` = keyframe at playhead,
  half-filled = animated, off-key. Click toggles (Electron behaviour).
- **Label column** flexible, truncates with an ellipsis; tooltips show the
  full name.
- **Value column** `value_column` (128 pt) wide, right edge at the panel
  inset, **shared by every row** of the panel and by composition sheets. Two-
  part values (Scale X/Y, Position X/Y) split the column into equal wells
  with a 4 pt gap, link toggle trailing *outside* the column.
- Values are **left-aligned in the well**, tabular, unit trailing in `muted`
  (Electron Nov 2026 rule).
- Section headings: `title` weight, 12 pt top / 6 pt bottom padding, a
  disclosure chevron on hover; Electron's collapse state is kept per project.

### Scrubbable numbers

Behaviour is Electron's `legacy/ui/controls.ts`, unchanged:

- Hover: the well lightens to `hover`, pointer becomes the horizontal resize
  cursor. A drag starts after 3 pt (`drag_threshold`); scrub speed is the
  property's step × 0.5 per point, ⇧ ×10, ⌥ ×0.1. The pointer is unbounded
  during the drag (Electron `infinite`); on release it reappears at the well.
  The whole drag is one undo step named for the property.
- Click without dragging: becomes an AppKit text field (P-TEXT dense field)
  with the system focus ring, contents selected. Return or clicking away
  commits, Esc restores. Arithmetic is accepted: `90/2`, and a leading
  operator applies to the current value (`+10`, `*2`).
- ↑/↓ step by the property's step (⇧ ×10) while editing.
- Scroll wheel over a hovered well adjusts it (⇧ ×10, ⌥ ×0.1), as in
  Electron — but only after the pointer has rested on the well, so scrolling
  the inspector past a value never changes it.

### Buttons and pop-ups

- **Push buttons**: system `NSButton` at `control` height. One prominent
  (accent) button per sheet or toolbar; secondary buttons are the system
  default style. Destructive actions use `danger` text, never a red fill.
- **Pop-up buttons** for choices (font, blend mode, quality, zoom):
  `NSPopUpButton` showing the current value and the system chevron
  (`Fit (54 %) ⌄`), not a row of three chips.
- **Borderless icon buttons** in panel headers (`+`, search, filter):
  `symbol_panel`, `secondary`, `hover` fill on hover, tooltip with the command
  name and shortcut.

### Segmented controls

- System `NSSegmentedControl` in sheets and toolbars. In GPUI inspector rows
  (Align, axis selectors) draw the same shape: `field` track, 2 pt padding,
  `inner_radius` segments, selected segment `floating` with `text` glyph.
- Panel tab groups (Project | Effect Controls) are quiet text tabs in the
  panel header: `title` size, medium weight, the active title in `text`, the
  rest in `muted`, a `hover` pill only under the pointer. Titles never
  truncate; tabs that do not fit fold into the trailing More Panels menu
  (2026-11-08, replaces the stretched segmented strip).

### Popovers

- `NSPopover`, anchored to the control that opened it, arrow towards it.
  Default width `popover_width` (260). Transient: dismiss on click outside or
  Esc. Used for colour picker, easing editor, gradient stops, zoom presets,
  layer blend/opacity quick edit.
- A popover never contains its own OK/Cancel — edits apply live and undo as one
  step on close.

### Sheets and alerts

- Sheets for any task that needs several fields: Save Layout, New
  Composition, Composition Settings, Export, Pre-compose, Replace Media.
  Window-modal system sheets (`NSWindow.beginSheet`). **Placement belongs to
  the system:** on macOS 26 and later (including macOS 27) the system
  presents a sheet as a floating glass card **centred in the window**; on
  earlier systems it slides down below the toolbar. Powermove never
  positions, offsets or re-parents a sheet, and never fakes one with a
  centred child window. A save/open panel started from a sheet is a sheet on
  that sheet. Width `sheet_width` (480) unless content needs more
  (`sheet_width_wide` for Export).
- Layout: `heading` title (Title Case), one `reading` sentence of
  explanation if needed, fields in inspector grammar (label left, shared value
  column), footer buttons trailing: Cancel, then the accent default button
  named for the action (`Save`, `Export`, `Create`) — never `OK`.
- Alerts (`NSAlert`) only for confirmations and errors. Destructive default is
  Cancel. Full Access consent is an alert with a warning symbol.

### Menus

- The main menu is the single complete command surface (every toolbar,
  context and palette command is also in a menu, with its shortcut).
- Context menus: system `NSMenu`; group with separators; current state shown
  with a checkmark (e.g., current dock side), not by omission.
- The palette (⌘K) and field menu are glass child panels: search field with a
  `magnifyingglass` icon, `menu_row` rows, shortcuts right-aligned in a fixed
  column in `muted`, height shrinks to content (max `palette_height`).

### Lists and tabs

- Project grid: thumbnails `control_radius`, duration badge top-left in a
  dark capsule, name below (`body`), metadata (`minimum`, `muted`). Selected =
  2 pt `accent` ring on the thumbnail + name in `text` on `selected_fill`.
- Lists (layers, effects, Settings): `row` height, full-width `selected_fill`
  for selection, `hover` on hover, disclosure triangles at leading.
- Panel tabs close with a hover `×` at the **leading** edge of the tab (Mac
  tab convention), and via the context menu.

### Progress, tasks and status

- Determinate progress: system bar or circular indicator; indeterminate:
  system spinner. Never a fake percentage.
- Background work appears as a toolbar Background Tasks item (circular
  progress) only while something runs; it opens the Background Tasks
  popover/window (see [reviews/na-cloud-promedia.md](reviews/na-cloud-promedia.md)).
- Toasts become quiet transient banners at the bottom of the viewer, or
  nothing: prefer state that is visible where the change happened.

---

## 10. Viewer

- Surround `colors.viewer`; the composition frame is drawn at its real
  bounds with a 1 px `separator` outline when the comp background is
  transparent (checkerboard) or matches the surround.
- No panel title. The viewer bar (`header` 32) holds, trailing: the zoom
  pop-up (`Fit (54 %) ⌄`), then the preview-resolution pop-up (`Auto ⌄`, or
  `Auto (Half) ⌄` when Auto has lowered it). Both titles have fixed widths so
  they never jump. Electron put them bottom-left as pills; native puts them
  in the bar so they never cover the image. There is no guides/safe-area
  toggle (v1.2.0 has none). In Focus or full-screen viewer modes the same two
  controls become a glass `ViewerOverlay` capsule (dark scheme) that fades in
  on pointer movement. Full spec: [panels/viewer-chrome.md](panels/viewer-chrome.md).
- Gizmos: bounding box 1 pt `accent`, handles 7 pt squares `accent` with
  1 pt `viewer` keyline; anchor point circle; rotation handle above. Gizmos
  scale with the display, not with zoom.
- Rulers/guides use `accent_text` at 60 %; snapping lines `accent`.
- The viewer never adds a material, tint or rounded mask to the image; the
  preview-corner preference (`square` / `rounded`) affects only the surround.

---

## 11. Timeline

- Header (`header` 32): play button, **timecode** (`timecode` 13 medium,
  tabular, `text`; Electron's orange timecode becomes neutral — orange is the
  playhead's), fps in `muted`, composition tabs, then trailing: Layers/Tracks
  mode, the Dope Sheet / Dope Sheet and Graph / Graph Editor view control,
  graph options and the snap toggle. Full spec:
  [panels/timeline.md](panels/timeline.md).
- Layer column `layer_column` (232) on `panel_secondary`: index (tabular,
  `muted`), kind symbol in a label-colour badge, name (`body`), trailing
  visibility/lock/solo glyphs at `symbol_inline` shown on hover or when set.
- Ruler `ruler` 24: tick labels in `minimum`, tabular, `muted`; major ticks
  `secondary`.
- Clips: label-colour fill, `inner_radius`, 20 pt tall in a 28 pt row, name
  inside in `body` (`text`), trimmed with an ellipsis. Selection is a 1.5 pt
  `accent` ring plus brighter fill — **never a solid accent fill** (that is
  how a selected clip would hide its keyframes). The selected row is filled
  in the layer column only (the owner's demo, 2026-11-08).
- Property sub-rows `property_row` 22: keyframe navigator (‹ ◆ ›, three
  `hit` squares from the inset), label `secondary` after it, and the value
  readout trailing in the layer column when it fits.
- Keyframes: 9 pt diamonds; unselected `text` at 90 %, selected `accent`
  with a 1 pt `panel` keyline; easing shown by shape (diamond linear, circle
  ease, hourglass hold) as in Electron.
- Playhead: `playhead` 1.5 pt `accent` line through ruler and tracks with an
  11 pt rounded head in the ruler.
- Render bar (Keep Rendered): a 3 pt strip under the ruler — see promedia
  review for the state colours.
- Work-area / in-out range: `accent_wash` bed in the ruler with `accent`
  handles.

---

## 12. Agent panel

- Opens from the toolbar Agent button as a **popover** (glass frame, solid
  `floating` content), as Electron's launcher does; **Pin** docks it as a
  solid panel (`Surface::AgentPanel`, default `agent_width` 360) in the right
  dock, and closing the docked panel returns it to the popover. Full spec:
  [panels/agent-panel.md](panels/agent-panel.md).
- Messages in `reading` 13; the user's messages right-aligned on
  `floating` with `control_radius`; agent replies are plain text on the panel
  (no bubble), markdown rendered with the language's type scale, code in SF
  Mono 12 on `field`.
- Work steps (tool calls) collapse into a single disclosure line in
  `muted` (`› 4 tool calls`), as Electron does; expanded steps are list rows.
- Composer: multi-line native text view on `field`, `control_radius`, send
  button `arrow.up.circle.fill` in `accent` when enabled. It is not glass: the
  composer sits on solid panel material (dense text).
- Questions from the agent appear inline as a card with system buttons; they
  never open a modal alert.

---

## 13. Motion

- Surfaces appear with a spring (`motion.enter` 0.32 s, `bounce` 0.15) from
  `start_scale` 0.96 and fade out in `exit` 0.12 s. Hover, press and selection
  feedback take `interaction` 0.075 s, linear. Nothing else animates by
  default.
- Motion explains a change of place (a dock moving, a panel opening); it never
  decorates. No looping animation outside an active progress indicator and
  playback.
- **Reduce Motion**: springs and scales become a `reduced` 0.15 s cross-fade;
  drag previews snap; the playhead still moves (it is content).
- Interrupted animations continue from their presentation value (shell
  already seeds springs from the presentation layer).

---

## 14. Copy style

- **Title Case** (HIG title-style capitalisation) for: menu items, menu
  titles, button labels, window and sheet titles, toolbar item labels,
  tab/panel names, command names in the palette, property and effect names.
  Examples: `Save Layout…`, `Reveal All Animated Properties`, `Effects &
  Presets`, `Anchor Point`.
- **Sentence case** for everything descriptive: section headings that
  describe (`Type settings`), explanations, placeholder text, tooltips' second
  line, empty-state explanations, status bar facts, errors, notifications.
  Empty-state **titles** are Title Case (`No Media`), like the system's
  content-unavailable view.
- Ellipsis `…` (U+2026, not three dots) when a command needs more input before
  it acts. Use `▸` only in prose paths (`Window ▸ Layouts`) and `→` never.
- Never all caps, never internal names (`fxbrowser`, `TEMPORARY · PHASE 1`,
  `Writable`) in the UI. Status reads like a person: `Saved`, `Paused`,
  `Edited`.
- Button labels are verbs naming the result (`Export`, `Replace`, `Trust`), not
  `OK`/`Yes`.
- Use Mac words: Settings (not Preferences), Move to Trash / Put Back (not
  Archive / Restore), Show in Finder (not Reveal in Explorer).
- Numbers and units: `1920×1080` (multiplication sign), `30 fps`, `34 px`,
  `100 %`, `0:12` durations, `00:01:25:13` timecode.

---

## 15. What changes from the Electron look, and why

| Electron (main, Nov 2026) | Native | Why |
|---|---|---|
| Custom traffic lights, tools inline in a flat title bar | System traffic lights; `NSToolbar` with a glass tool capsule | HIG window chrome; automatic full-screen, tabs, customisation |
| Home button, Settings gear, workspace tabs in the toolbar | File ▸ Project Library + Welcome window; ⌘, Settings; Window ▸ Workspace (+ optional toolbar pop-up) | HIG placement: app settings and navigation live in menus; toolbar holds document actions |
| `1 task running` text pill | Background Tasks toolbar item with circular progress, only while running | Quieter; standard progress affordance |
| Inter (`Powermove Inter`) UI, Geist Mono numbers | SF Pro with optical sizes; tabular SF Pro numbers; SF Mono only for code | At home on the Mac; same steady numbers |
| Units glued to numbers (`34px`) | `34 px`, unit in `muted` | Readability, HIG unit spacing |
| Orange timecode | Neutral timecode; orange reserved for the playhead | One accent, used as a mark |
| Uppercase status (`SAVED`) | `Saved` | Copy style |
| Zoom/quality pills over the bottom-left of the viewer | Pop-up buttons in the viewer bar; glass overlay only in Focus | Never cover the image; system pop-up |
| Dialogs centred, panel material, custom buttons | System sheets (system-placed: centred on macOS 26+), panel-material content, system buttons, orange default | HIG sheets; same material rule as Electron |
| Electron-drawn menus imitating macOS | Real `NSMenu` | Exact system behaviour, accessibility |
| Blue system focus ring possible beside orange accent | AccentColor = Powermove orange | One accent |
| Letter badges for tracks (`V1`, `A1`) and `fx` badges | SF Symbol kind badges in label colours (track names stay) | System iconography |
| 38/30 px headers/rows, 10 px gaps | 32 pt headers, 28 pt rows, 6 pt gaps, 12 pt inset | Owner-approved v2 density; Electron's own latest header height |
| Warm Stone dark (v1.2.0) | Neutral grey dark (R = G = B) | Owner rule, Nov 2026 |
| Selected clip in solid accent (seen in native lanes) | Ring + brighter label fill | Keeps keyframes visible; accent as a mark |

Unchanged on purpose: panel arrangement and sizes, every shortcut and drag
behaviour, inspector grammar (label + shared value column), keyframe diamond
semantics, effect and property names, light-mode palette, the orange accent.

---

## 16. Accessibility

- Contrast floors (tested): text 7:1; secondary/muted 4.5:1; accent text and
  semantic colours 4.5:1; accent marks 3:1; under Increase Contrast secondary
  7:1 and separators 3:1.
- Never colour alone: status pairs a symbol and words; selected keyframes
  change shape keyline as well as colour; render-bar states differ in pattern.
- Full Keyboard Access: every control reachable by Tab; panels cycle with
  ⌃Tab / ⌃⇧Tab (Electron behaviour kept where bound); splitters focusable and
  arrow-resizable (`keyboard_step`). The focus ring is always visible on the
  focused element; a theme cannot hide it.
- VoiceOver: every GPUI element exposes a role, label and value; values read
  with units (`Opacity, 100 percent`); keyframe diamonds read their state;
  the timeline reads clips as `Headline, text layer, 0:00 to 0:20`.
- Reduce Transparency, Increase Contrast, Reduce Motion and Differentiate
  Without Colour are observed live (shell rebuilds tokens on the system
  notification).
- Dynamic type: macOS has none system-wide; the Density setting and the 11 pt
  floor cover it. Text never truncates without a tooltip.

---

## 17. Decisions for the owner

These are made in this document; reverse any of them here and in pm-design.

**Approved by the owner (Chike, 2026-11-01):** all five, as written below.

1. Values left-aligned in wells (Electron Nov 2026) rather than right-aligned
   (mockup v2). Chosen: left, to match the shipped app.
2. Timecode neutral, not orange. Chosen: neutral.
3. List selection neutral (`selected_fill`), not accent. Chosen: neutral.
4. Workspace switcher leaves the toolbar for Window ▸ Workspace plus an
   optional toolbar item. Chosen: menu + optional item.
5. Panel header 32 pt (Electron latest) rather than 36 pt (mockup v2).
   Superseded 2026-11-08 by the owner's demo: 38 pt headers, 8 pt panel
   radius, 24 pt status line, translucent window and panel material.

**Design-lead resolutions (2026-11-01).** Three open design items, decided
within the rules above (reverse here if the owner disagrees):

- *Sheet placement on macOS 27*: the system centres sheets; we use system
  sheets and never place them (§9 *Sheets and alerts*).
- *Light-mode palette over the black viewer*: chrome follows the
  appearance; everything drawn over the image uses dark-scheme tokens; the
  light surround becomes neutral `#E4E4E4` (§4 *Over the image*).
- *Quality pop-up placement*: viewer bar, trailing, right after zoom, titled
  `Auto (Half)` when Auto lowers it (§10, [panels/viewer-chrome.md](panels/viewer-chrome.md)).

