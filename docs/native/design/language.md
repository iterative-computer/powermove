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
`TreeItem`) are rounded. Nothing was renamed or removed.

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
| Display | 24 | semibold (`display_weight`) | A screen's title (Home, Settings) |
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
  unified style, at the system's own height and spacing. Its items are Home,
  the project tabs (the window's tab group drawn by `Host::sync_tabs`; AppKit's
  own tab bar row stays hidden), the tools, Agent and Export. Docks are shown
  and hidden from the menus and by their tabs, not from the toolbar.
- The project strip (Home and the open projects, in the toolbar's leading
  glass) is one control drawn in cells. Every cell stands 4 pt inside the
  glass on all four sides, so a pill's corner is the glass's corner less the
  inset. Tabs share one width: the widest while there is room, then narrower
  together down to a readable least; past that the strip shows the run that
  holds the selected tab and a pull-down lists every project. A title stands
  in the middle of its tab and is cut in the middle, keeping the start and
  the end of a name. The leading slot holds the close button (under the
  pointer) or the edited dot and is always kept, so the title never moves.
  The selected cell is a raised neutral pill; the one under the pointer a
  wash of the same shape; hairlines part resting tabs and vanish beside
  either. The pull-down and New Project are round targets after the last
  rule, each in a cell that shows the strip's wash under the pointer. A tab is carried along the strip to reorder it.
- Sidebars: `Host::sync_sidebar` puts an `NSGlassEffectView` (clear style, a
  heavy tint of the window colour) in a column under the GPUI view, which
  leaves that column unpainted. The glass runs on under the content sheet's
  leading corners, so its rim is hidden. In both appearances the sidebar
  recedes into the window (a shade under the window ground) and the content
  sheet stands out from it: the change of value is the only line between
  them.
- Popovers with their own window: `popover_frame`. The palette and field menu
  are pooled child windows (`native::Popup`).
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
not animate. Reduce Motion snaps anything that moves and turns fades into a
0.15 s cross-fade.

## Controls

Real controls first. Wherever AppKit has the control and the system draws it
(liquid glass on macOS 26 and later), host the real one:
`pm_shell::native_control::NativeControl` with a `ControlSpec` of kind `Push`
(optionally tinted, the view's one accent action), `Popup`, `Segmented`,
`Switch`, `Checkbox`, `Radio`, `Slider` or `Search` (the system search
field, with no focus ring: Return, Escape and Down hand the keyboard back to
the window's content and reach the view as keys; ⌘F on Home calls `focus`).
A view creates them once (an entity
each), places them like any element, and subscribes to their values. Each
sits in its own clipping wrapper above the GPUI view and is staged with the
element's bounds and the content mask of its scroll region, so it scrolls and
clips with the panel. Focus, disabled state, Reduce Transparency and both
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

Every drawn control: arrow cursor, hover wash, pressed fill, neutral
keyboard-only focus ring (`Tokens::focus_ring`), one symbol size per context
(`metrics.symbol_toolbar` 14, `symbol_panel` 14, `symbol_inline` 12, and
`symbol_action` 20 for a surface's one primary action glyph, a composer's
Send). A value in a well starts on the same inset with or without a unit, in
the text font, so it sits on its row label's baseline. A section heading in a
panel is the Label role, a step under the panel's Title.
Lists are quiet text for recall by name: no glyph tiles, no dividers.

## Home

A sidebar for navigation only (Recents, Trash, Store, the account) on the
window's glass, and one inset sheet. The sheet's header is two rows on the
sheet's margins: the section title, the one wide search field in the middle
(the large system field, its shortcut at the trailing end while it is empty)
and Open… with the one accent action; then the count and the quiet sort and
layout controls. A project is its poster: the frame in the composition's own
shape with a continuous corner, a hairline and a soft shadow, the name and
one line of facts under it, and a small dot when it is open (the accent for
this window's project). A project with no poster yet is a quiet ground of the
same shape that says its size. Under the pointer a card rises on the hover
springs; pressed, it sits back down; a sort or a search slides cards to their
new places. The list is the dense alternative under the same header.

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

## Timeline strips

`Tokens::strip(layer colour, StripState)`: a slab of tinted glass on the solid
timeline canvas, the same in both appearances (a clip is an object with its
own colour). The band a name stands on is deep and saturated; light gathers in
the foot below it (`glow`, from `GLOW_FROM` of the way down), so the slab is
luminous without costing the name its contrast. Labels are the palest tint of
the hue that holds 4.5:1 on the body under them (`under_label`). A 1 px rim of
light in the strip's hue runs inside the edge, brightest along the top and
round its corners, faint down the sides, with no outline; corners are
continuous and about a quarter of the strip's height (`strip_radius`); an even
`strip_gap` parts neighbours; the ruler's grid reads through the body as a
top-lit highlight: brightest at the strip's top edge and gone by the middle
of its height (`grid`, `GRID_FADE`), with the faint full-height line showing
only between rows. The foot gathers as much light as the hue allows while a
label still holds 4.5:1 under a name (a pale layer colour takes less). A waveform is a soft filled shape in a paler tone standing on the
foot; silence draws nothing. A collapsed layer's keys are small marks in the
foot, under the name. Hover brightens and whitens the rim and shows the trim
grip (`trim_grip`) inside the nearer edge, with the name starting clear of it.
Selection is a 2 px white ring: at the edge on a dark canvas, inside a band of
the strip's own colour on a light one (`rim_inset`). Dragging lifts it;
disabled drops chroma and opacity. Rows are 30 pt (v1's height), so a strip is
26 pt. Ruler labels stand over a small dot with a faint line down the tracks.
`Tokens::playhead` is a hairline with a round head. Strips are drawn by the
timeline renderer, never system views.

## Three mistakes to avoid

1. A border or an outer shadow on a translucent surface. Use the elevation
   recipe; on panels the hairline is inside.
2. An accent selected state or focus ring. Selection is neutral and raised.
3. A literal size, colour or duration, or a second hover or spring
   implementation. Ask the tokens and the view's `Motion`.
