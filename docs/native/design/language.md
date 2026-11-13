# Powermove Native design system

One system, in `apps/native/crates/design` (values) and
`apps/native/crates/shell/src/kit.rs` (controls). A view never writes a colour,
size, radius, duration or font; it asks for a role. The generated value table
is [tokens.md](tokens.md).

The bar: next to Final Cut Pro, Finder and Xcode it reads as a first-class Mac
app, and it is better than v1 on every surface side by side.

Kit API, additive changes since the first hand-back: `Kit::disabled`;
`pm_shell::native_control` (the real AppKit control host, moved from the app,
with `Radio`, `Segmented` and `Slider` kinds, `tint` and `symbol`);
`typography.display_weight`; `Strip::glow`, `Strip::grid`, `Strip::under_label`
and `Strip::rim_inset`; `translucency.glass_tint`; `pm_shell::request_close`;
the surface host's native `SegmentedControl`; the `flexWrap` panel style;
the `Search` control kind (`focus`, `placeholder`, `host_text`) and a panel
text field's `metadata.search`; `metrics.symbol_action`;
`pm_shell::native::selected_color`; `ControlSpec::quiet` and the panel
variant `quiet`; `pm_shell::sidebar_frame`; source-list rows (`ListItem`,
`TreeItem`) are rounded; `pm_shell::resize_from_top`;
`NativeControl::stretch` (a `SegmentedControl` with `flexGrow`); a sheet's
`data-pm-slot="body"`; a `Switch` in a sheet; `translucency.glass_menu`;
`Kit::menu_light` and `Kit::menu_heading`; a `transient::Choice` has a
`category` and a `checked` state (`Choice::new(..).within(..).checked(..)`).
One change of meaning: `transient::Picker::selected` is gone. A menu has no
selected row; it has the keyboard's row (`keyed`) and the pointer's (`hover`),
and `lit()` is the one that shows. `Kit::menu_row` no longer washes itself.

## Colour

Seven roles per appearance, chosen in OKLCH (`Colors::of`): an ink, four
grounds (window, panel, recess, viewer), a floating surface and the accent.
Dark is warm stone (hue about 55°, chroma under 0.01); light is cool paper
(hue 286°). Everything else neutral is the ink over a ground at an alpha step:

| Step | Role |
|---|---|
| `text` | Primary text and glyphs |
| `secondary` | Row labels, quiet buttons |
| `muted` | Metadata, captions, units (4.5:1 on every ground) |
| `faint` | Placeholders, shortcut hints, disabled |
| `fill` / hover, `selected`, `pressed` | Washes and soft wells |
| `hairline`, `separator` | Edges and dividers |

- The accent marks one action or one state per view (the primary button, a
  drop target, progress). It is never a selected fill and never a focus ring.
- Selected is the raised neutral surface (`colors.raised` with
  `Elevation::Raised`) or a neutral ink fill. An active window gives a
  system "on" control the accent, so a hosted segmented control takes
  `selected_color()` as its bezel (the selected tab's pill: paper on a light
  ground, a lift on a dark one) and draws each segment's title or glyph
  itself in the label ink, because AppKit would paint them white on a custom
  bezel. Judge selection in an active window, not only in a background
  capture.
- Elevation is tonal: viewer below window below panel below floating.
- Increase Contrast lifts the text steps to 7:1 and 4.5:1, separators to 3:1
  and doubles strokes; a theme patch cannot lower them.

## Type

The system font through CoreText (`TEXT_FONT`), one role per job
(`Tokens::text(Text::…)`, `Recipes::text_role`):

| Role | Size | Weight | Use |
|---|---|---|---|
| Caption | 11 | regular | Metadata, counts, shortcut hints |
| Body | 12 | regular | Rows, fields, menus |
| Label | 12 | medium | Button labels, the name in a row |
| Title | 13 | medium | Panel and window titles |
| Reading | 13 | regular | Running prose |
| Heading | 15 | semibold (`display_weight`) | Sheet and section headings |
| Headline | 17 | semibold (`display_weight`) | The title in a screen's header row (Home) |
| Display | 24 | semibold (`display_weight`) | A screen's title (Settings) |
| Timecode | 13 | medium, tabular | Timecode and numeric readouts |

Text sizes use the font's own tracking table; Heading and Display are set
tighter (`typography.*_tracking`). Counts, sizes and times use tabular figures
(`kit::tabular`). Weight is not the hierarchy: a panel title is medium, never
semibold. Panels ask for `typography.semibold_weight` for their titles and
section headings; it is the medium weight, so every panel matches the kit.

## Space and shape

- One spacing scale (`Tokens::space`): 2, 4, 6, 8, 12, 16, 24, 32.
- One radius scale from a base of 4 (`Tokens::radius`): 4, 6, 8, 12, 16.
  Controls are 8, wells and segments 6, panels and menus 12, popovers and
  sheets 16.
- Nested corners are concentric: child = parent − inset, never below 6
  (`Radii::nested`). Surfaces of 12 and more use continuous corners
  (`Recipes::corner`); small controls stay circular so nested corners match.
- Docked panels are borderless surfaces `metrics.dock_gap` apart.

## Edges and elevation

Edges are hairline shadows, never borders (`Tokens::elevation`,
`Recipes::elevated`): `Well`, `Raised`, `Panel`, `Menu`, `Sheet`. GPUI draws
outer shadows under the whole element, so translucent surfaces (`Panel`,
`Well`) carry their hairline inside the edge.

## Materials

Real system material through AppKit, for what the system itself makes glass:

- Toolbar and title bar controls: the window's `NSToolbar` in the regular
  unified style, at the system's own height and spacing. Its items are the
  project strip (Home and the window's tab group drawn by `Host::sync_tabs`;
  AppKit's own tab bar row stays hidden), the tools, Agent and Export. Docks are shown
  and hidden from the menus and by their tabs, not from the toolbar.
- The project strip (Home, the open projects, New Project) is not glass: it
  is an unbordered toolbar item of quiet cells straight on the bar, 4 pt
  apart, each the control height with an 8 pt corner. A tab is as wide as
  its title asks (14 pt inside each end, at most 180); short of room the
  long titles narrow together, cut in the middle so the start and the end
  of a name stay; past the narrowest the strip shows the run that holds the
  selected tab and a quiet pull-down lists every project. One neutral ink
  fills the showing project's cell (8 %) and the cell under the pointer;
  nothing else has a fill, an edge or a shadow. The showing title is the
  primary ink, the others muted, all in one size and weight. An edited
  project carries a 6 pt dot before its title. Under the pointer the close
  button slides in from the trailing end while the title's end fades out
  under it (a mask), so the title never moves. Home is a glyph cell before
  the first tab, New Project one after the last. A tab is carried along the
  strip to reorder it and changes places with a neighbour once its leading
  edge passes that tab's middle.
- Sidebars: `Host::sync_sidebar` puts an `NSGlassEffectView` (clear style, a
  heavy tint of the window colour) in a column under the GPUI view, which
  leaves that column unpainted. The glass runs on under the content sheet's
  leading corners, so its rim is hidden. In both appearances the sidebar
  recedes into the window (a shade under the window ground) and the content
  sheet stands out from it: the change of value is the only line between
  them.
- Popovers with their own window: `popover_frame`. The palette and field menu
  are pooled child windows (`native::Popup`) on glass, with the floating
  colour laid under their rows (`translucency.glass_menu`): glass alone takes
  its value from what is behind it, so dark ink over a dark viewer in the
  light appearance would not read, and a glass tint does not hold it.
- The window ground is a tint over a behind-window blur.

The viewer, the timeline and dense panels are solid. Glass has no hover of its
own and no drawn bloom or rim. Reduce Transparency makes every material solid;
before macOS 26 glass falls back to the matching visual effect.

## Motion

`pm_design::motion`: substepped, velocity-carrying springs; retargeting keeps
value and velocity, so everything is interruptible. A view owns one
`pm_shell::motion::Motion`, calls `frame` at the top of `render`, and nothing
ticks while nothing moves.

| Preset | Use |
|---|---|
| `Layout` | A gliding selection, rows and cards sliding to new places |
| `HoverIn` / `HoverOut` / `HoverSwitch` | Washes: fast in, slow out, gone at once when the pointer moves to a neighbour |
| `Enter` / `Exit` | Menus, popovers, sheets (half-bounce 0.15 in, none out) |
| `Show`, `Pop` | A row appearing; a mark confirming an action |

Press answers on pointer-down with no animation. Keyboard-driven changes do
not animate, with one exception: a menu's highlight glides one row for one
key press, and steps without the glide while a key repeats. Reduce Motion
snaps anything that moves and turns fades into a 0.15 s cross-fade.

## Controls

Real controls first. Wherever AppKit has the control and the system draws it
(liquid glass on macOS 26 and later), host the real one:
`pm_shell::native_control::NativeControl` with a `ControlSpec` of kind `Push`
(optionally tinted, the view's one accent action; with `glass`, the system's
glass button at the large size, a surface's one raised action), `Popup`, `Segmented`,
`Switch`, `Checkbox`, `Radio`, `Slider` or `Search` (the system search
field, with no focus ring: Return, Escape and Down hand the keyboard back to
the window's content and reach the view as keys; ⌘F on Home calls `focus`).
A view creates them once (an entity
each), places them like any element, and subscribes to their values. Each
sits in its own clipping wrapper above the GPUI view and is staged with the
element's bounds and the content mask of its scroll region, so it scrolls and
clips with the panel. A pop-up's rows follow its panel: rows that stand for
the same values keep their menu items and take new titles, even while the
menu is open; any other change waits until an open menu has closed and sent
its action. Focus, disabled state, Reduce Transparency and both
appearances are the system's own. Extension panels get the same controls from
the surface host (`PopUpButton`, `PushButton`, `Checkbox`, `Toggle`, `Switch`,
`SegmentedControl`, and a `TextField` with `metadata.search`). A borderless
pop-up in a bar of controls (`data-pm-variant="inline"`: the viewer's zoom
and resolution) keeps the primary ink; one in an option row under the content
it qualifies (`"quiet"`: a composer's options, Home's sort) is titled in the
secondary ink. A panel's
header is the child it marks
`data-pm-slot="header"`: the dock shows it as the panel's one title row. A
composer is the kit's multi-line `TextField` with `metadata.submitOnReturn`,
written on a surface its panel draws.

Drawn controls (`pm_shell::kit`) are for where a view per control cannot work:
long scrolling lists, the timeline, anything over the viewer, and windows with
no AppKit view (tests). `Kit::new(&tokens, &motion, access, window, cx)` once
per frame, then: `button` (primary, neutral, quiet, destructive),
`icon_button`, `segmented` (pill track or flat tabs, gliding selection),
`search`, `switch` (the system switch's shape at the small size), `check`
(checkbox or radio), `slider`, `disclosure`, `row`, `menu` and `menu_row`,
`card` and `form_row` (the Settings anatomy sheets reuse), `progress`,
`empty_state`, `banner`, `tooltip`, `disabled`. Text and numeric fields are
`pm_shell::text::TextField` (a native field in a drawn well). Right-click
menus are `NSMenu` (`Host::context_menu`).

Menus. The system's menu (`NSMenu`) is for a right-click and for a pop-up
button. A menu a button of ours opens, and the command palette, are ours and
share their parts, in AppKit (`native::Popup`) and drawn (`Kit::menu`):

- Nothing is lit when it opens. One highlight (`Kit::menu_light`) shows the
  lit row: the row under the pointer while the pointer is over one, else the
  keyboard's. It appears on the row it lands on, glides between rows on the
  layout spring and leaves with the pointer. `transient::Picker` holds that
  state for both.
- Rows are the Body role with a check column when any row has a state,
  section headings in the Caption role and the muted ink
  (`Kit::menu_heading`; the palette lists a command under its menu), and
  shortcut hints right-aligned in the faint ink.
- Keys: the arrows, Home and End move the highlight (wrapping), Return
  chooses the lit row and does nothing when none is lit, Escape closes. The
  palette and the field menu filter as you type in the system's search field
  and light the best match; a section stays whole and stands where its best
  match does. A menu with no field lights the first row a typed letter
  starts.
- Corners are concentric: the palette is 16 with rows 6 inside it (row corner
  10), a menu is 12 with rows 4 inside (row corner 8).
- It opens on the enter spring and closes on the exit fade.
- A list longer than the palette fades out at the bottom edge, so a row is
  never cut there, and scrolls a fade's length past its last row.

Every drawn control: arrow cursor, hover wash, pressed fill, neutral
keyboard-only focus ring (`Tokens::focus_ring`), one symbol size per context
(`metrics.symbol_toolbar` 14, `symbol_panel` 14, `symbol_inline` 12, and
`symbol_action` 20 for a surface's one primary action glyph, a composer's
Send). A value in a well starts on the same inset with or without a unit, in
the text font, so it sits on its row label's baseline. A section heading in a
panel is the Label role, a step under the panel's Title (see Properties).
Lists are quiet text for recall by name: no glyph tiles, no dividers.

## Home

A sidebar for navigation only (Recents, Trash, Store, the account) on the
window's glass, and one inset sheet. The sheet's header is drawn, quiet and
borderless: nothing in it is a bezelled system control (hosted AppKit
controls are for forms: Settings, sheets, panel option rows).

- One inset (24) for the title, both header rows and the cards; one text
  size for the header's controls (Body); one ink in alpha steps (wash 4.5 %,
  a step deeper under the pointer or when on, deeper again while pressed).
- First row, 48 high: the title in the Headline role (17, semibold); the
  search in the middle of the sheet, a soft pill 32 high with no edge (a
  magnifier, the bare native text field, the shortcut in the faint ink while
  it is empty), at most 360 wide; at the trailing edge Open… as an icon
  alone (32 square, no fill at rest, its name and shortcut in the tooltip)
  and New Project, the header's one raised action: the system's glass
  button (a plus and its name), the same glass as the toolbar's Export.
  ⌘F puts the keyboard in the search; typing filters; Escape clears the
  search and leaves the field in one step
  (`TextField::set_escape_cancels`).
- Second row, 28 high: the count in the muted ink; the sort as a text action
  (an icon and the current order) that opens our menu; then the grid/list
  switch, the header's one raised element (a small pill, two 30 × 24 cells,
  the on fill gliding between them).
- A project is a framed page: a 4 pt frame of ink around the page, corners
  concentric (page 16, frame 20, continuous), in one shape for every card.
  The poster fills the page when it has the page's shape and stands whole on
  the viewer's ground otherwise; a project with no poster yet is an empty
  page that says its size. Under the pointer the frame deepens a step and
  a round "more" button stands at the page's top trailing corner, on a
  scrim so it reads over any poster: it opens our menu with the row's
  actions (a right-click keeps the system's menu). Selected, the frame
  carries a ring of ink inside its edge. The name and one line
  of facts stand under the card on the page's edge, with a small dot when
  the project is open (the accent for this window's).
- Cards are 240 wide when four or more fit, and one to three wider ones on
  a narrow sheet; a sort or a search slides them to their new places. The
  list is the dense alternative under the same header.

## Settings

Its own window in Home's shape (`pm_shell::sidebar_frame`): the sections on
the window's glass below the window buttons (the system search field, quiet
group headings, rows whose selected state is a rounded neutral wash) and one
content sheet. A page is a title and a line of description, then cards
(`FormGroup`) under section headings. Every row of a card is the same
anatomy: a title over its description, and a control of its natural width at
the trailing edge (the real pop-up, switch, push button; a text or number
field in its well). The page title, its description, every section heading
and every row title start on one x: a card bleeds past that column by the
row inset. Lists that are not settings (speech models, installed extensions,
the Store, memory notes) are rows of the same cards, never loose text. A
search lists every section with a match and dims the others in the sidebar.

## Sheets

A form in a sheet is a Settings page in a sheet: the title, then cards of
rows under quiet headings, then Cancel and the default action at the trailing
edge of the bottom (no rule above them). A row is its title over its notes
(a description, or what is wrong with the value), with a real control of its
natural width at the trailing edge: the pop-up, the system switch (never a
checkbox or Off | On segments for a yes-or-no row), a text or number field, a
colour well. Numbers end before one unit column, so their right edges line
up whatever the unit. Radios keep their own titles, one to a row.

One text column: the sheet's title, a destination's description, section
headings, row titles, notes and the summary all start on
`sheet_inset + inset` (32 pt); cards start on `sheet_inset`. Measure it in a
capture before showing a sheet.

Two builders, one anatomy: a panel sheet is `SheetAnchor` with `FormGroup`
and `FormRow` (Export), and an AppKit sheet is `surfaces::sheet_layout`
(`Sheet`: `text`, `number`, `pair`, `control`, `toggle`, `swatch`, `option`,
`note`, `heading`, `line`, laid out by `footer`), which every `Surface` form
goes through (composition settings, new composition and project, pre-compose,
keyframe value, velocity and scale, captions, sequence import, sign-in and
Store forms). Confirmations are the system's alert sheets.

A sheet whose content changes height marks the part between its title and
its actions `data-pm-slot="body"`. Its window follows the body's height on
the layout spring from a fixed top edge (`pm_shell::resize_from_top`; AppKit
would centre the sheet again on every frame, so the window class hangs it
from the top it was first given), the actions ride the bottom edge, and
nothing else moves. Hosted controls keep their distance from the window's
top between a resize and the next paint.

## Properties

One grid for every panel of rows (Properties, Effect Controls, extension
panels built from `InspectorRow`), from the kit, so a panel never sets it:

- One text edge at the panel inset: the panel's title, the layer's name,
  section headings, and the leading point of a row's keyframe mark, which
  hangs in a gutter of its own. A row without a mark keeps the gutter, so
  every label starts on one x. An effect's heading puts its disclosure in
  the same gutter.
- A label keeps `metrics.label_column` and gives way first in a narrow
  panel; the value column takes the rest and is never under
  `metrics.value_column`. A trailing control (a scale's link) stands
  outside the value column.
- Three ranks, one size apart at most: the panel's title (Title), then the
  layer's name and section headings (Label: 12 medium, text ink; the kind
  after the name in the muted ink), then row labels (Body, secondary ink)
  and values (Body, tabular, text ink; units muted). Rows are 28 with 22
  wells; sections stand a gap apart with no rule between them.
- A section's own action (Refresh Fonts, Add Mask, Add Text Animator) is a
  quiet text action at the trailing edge of its heading row.
- One well for every value: the ink at the wash step (never an opaque
  field colour, which vanishes on a translucent panel), the well corner, a
  6 pt text inset, a step deeper under the pointer, and while edited a
  hairline ring inside its edge in the neutral focus ink. A value keeps its
  baseline when it is clicked into. A pop-up is the real `NSPopUpButton`
  with no bezel on that well; a font or a fill that opens a menu is the
  same well with the pop-up's chevrons; a colour is one well holding its
  chip (a keyline inside its edge) and its hex; a multi-line text is as
  tall as its lines, from two to six. Yes-or-no rows are checkboxes;
  segmented rows are the small system control.
- The keyframe mark is one glyph whose shape and ink say its state, as the
  timeline's (outline muted, half accent, filled accent), with no lit
  ground. A scale's link is one glyph whose ink says linked or not.
- An expression stands under its row, from the label's x to the panel's
  inset, in the fixed-width face, as many lines as it has; a problem with
  it is one line under it in the danger ink, and a valid one says nothing.
  The value it drives is in the accent ink. Its sheet is an editor that is
  the card itself: seven lines, the fixed-width face, nothing that rewrites
  what is typed, the parser's answer under it and the names an expression
  can use under that; Return is a new line and ⌘Return applies.

## Timeline strips

`Tokens::strip(layer colour, StripState)` (`crates/design/src/strip.rs`): a
slab of tinted glass on the solid timeline canvas, derived from the layer's
hue for the appearance it stands in. Only the hue is shared; every value
below lives in pm-design, none in the timeline's code.

- Dark canvas (the body of `Tokens::strip`): a deep, saturated band a name
  stands on (L 0.39, chroma 0.15) opening into a luminous foot (L 0.52 plus
  `glow` from `GLOW_FROM` of the way down, as much as the hue allows while a
  white-ish name still holds 4.5:1). A 1 px rim of light in the hue, brightest
  along the top and round its corners, no outline. Names are the palest tint
  of the hue that holds 4.5:1 on the body under them (`under_label`). Hover
  brightens and whitens the rim; selection is a 2 px white ring at the edge
  on a body a step brighter.
- Light canvas (`Tokens::strip_light`, values in `LIGHT`): a pale tint of
  the hue the canvas shows through (top L 0.84, foot L 0.895, chroma 0.10,
  90 % opaque), lit toward white in the foot. A 1 px rim in a deeper shade
  of the hue (L 0.58) with a light edge along the top. Names are a dark
  shade of the hue (from L 0.38 down until 4.5:1 on the body's deepest
  part), waveforms and marks a mid shade (L 0.54). Hover deepens the rim;
  selection is a 2 px ring in the hue's deepest shade (L 0.42) on a body a
  step deeper and fuller. A neutral layer (null, camera) is light grey glass.
- Both: disabled (hidden, muted, offline) drops chroma and opacity, so the
  strip fades toward the canvas; outside the work area the tracks are veiled
  with the canvas's own second step (`Tokens::work_area_veil`), so strips
  there fade toward the ground in either appearance, and names are painted
  over the veil, so a name reads there as it does inside; dragging lifts
  the strip on a shadow. Corners are
  continuous and about a quarter of the strip's height (`strip_radius`); an
  even `strip_gap` parts neighbours; the ruler's grid reads through the body
  as a line strongest at the strip's top edge and gone by the middle of its
  height (`grid`, `GRID_FADE`): light on the dark canvas, the rim's shade on
  the light one. The trim grip (`trim_grip`) is white on dark strips and the
  ink on light ones. Increase Contrast flattens the body and outlines it
  (white on a deep body in dark, the ink on a pale body in light).
- A waveform is a soft filled shape standing on the foot; silence draws
  nothing. A collapsed layer's keys are small marks in the foot, under the
  name. Rows are 30 pt (v1's height), so a strip is 26 pt. `Tokens::playhead`
  is a hairline in the ink with a round head. The kind badge in the layer
  column takes `colors.labels.*`, which are per-appearance too. Strips are
  drawn by the timeline renderer, never system views.

## Timeline

One solid ground carries the layer column, the ruler, the tracks and the
bottom bar; hairlines part them (the ruler's foot runs across the whole
panel, the column's edge from top to bottom, the bar's top edge), and the
layer column is a step of value, not a box. Everything is on a 2 pt grid:
ruler 24, layer rows 30 (strip 26), property rows 22, bottom bar 24.

- Layer row: its number (tabular, muted), the disclosure (a small muted
  chevron), the kind badge in the layer's colour, the name. Switches stand at
  the trailing edge in fixed columns (parent, solo, lock, visibility, the last
  at the edge): a set switch always shows, the rest come with the pointer,
  keyboard focus or a single selection, and a set switch keeps the columns
  after it open so each kind stays in its own column down the list.
- Property row: the key navigator (previous, the key mark, next) stands in
  the columns the number, disclosure and badge take, so the property's name
  starts where its layer's does; the value is tabular at the trailing edge.
  The key mark carries its state in its colour (accent once animated, muted
  before), with no lit ground.
- Ruler: a label over a dot at each step, smaller fainter dots counting the
  steps between (`minor_step`: the finest whole division that leaves 7 pt),
  a faint line down the tracks at each label. The work area is the lit
  stretch of the ruler (ink at 9 %) between two short bars at its foot, in
  the ink, never the accent; the tracks outside it are veiled. The playhead
  is the ruler's one bright mark.
- Bottom bar (`pm_panel_timeline::footer`): under the layer column, zoom (a
  minus, a slider on a logarithmic scale, a plus; twice on the slider fits
  the composition) and the composition's length; under the tracks, the
  navigator: a thumb as long as the time on screen, dragged to scroll, a
  press beside it bringing the thumb there. Rows scroll with a thumb of the
  same recipe at the tracks' trailing edge. Thumbs are ink at 22 %, 38 %
  under the pointer or held, and are absent when there is nothing to scroll.
- A property row's value ends at the row's trailing edge in every row; a
  scale's aspect link stands before its value, one glyph whose ink says
  linked or not.
- The graph is the same piece: the tracks' ground, the ruler's time lines
  and the markers' lines running on through it, curves and keys in the ink,
  value labels in the muted ink. Beside the dope sheet it takes the lower
  half from the first row edge there, so it never cuts a strip or a row of
  keys in two. With no curve to show it says how to get one, in its middle.
- A strip too short for three letters of its name shows its colour alone.
- Playing changes the playhead layer and the timecode only: the canvas, the
  bar and the rows are not repainted.

## Three mistakes to avoid

1. A border or an outer shadow on a translucent surface. Use the elevation
   recipe; on panels the hairline is inside.
2. An accent selected state or focus ring. Selection is neutral and raised.
3. A literal size, colour or duration, or a second hover or spring
   implementation. Ask the tokens and the view's `Motion`.
