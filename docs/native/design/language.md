# Powermove Native design system

One system, in `apps/native/crates/design` (values) and
`apps/native/crates/shell/src/kit.rs` (controls). A view never writes a colour,
size, radius, duration or font; it asks for a role. The generated value table
is [tokens.md](tokens.md).

The bar: next to Final Cut Pro, Finder and Xcode it reads as a first-class Mac
app, and it is better than v1 on every surface side by side.

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
  `Elevation::Raised`) or a neutral ink fill.
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
| Heading | 15 | semibold | Sheet and section headings |
| Display | 24 | semibold | A screen's title (Home, Settings) |
| Timecode | 13 | medium, tabular | Timecode and numeric readouts |

Text sizes use the font's own tracking table; Heading and Display are set
tighter (`typography.*_tracking`). Counts, sizes and times use tabular figures
(`kit::tabular`). Weight is not the hierarchy: a panel title is medium, never
semibold.

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

- Toolbar and title bar controls: the window's `NSToolbar`.
- Sidebars: `Host::sync_sidebar` puts an `NSGlassEffectView` (clear style, a
  light tint of the window colour) in a column under the GPUI view, which
  leaves that column unpainted.
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

`Kit::new(&tokens, &motion, access, window, cx)` once per frame, then:
`button` (primary, neutral, quiet, destructive), `icon_button`, `segmented`
(pill track or flat tabs, gliding selection), `search`, `switch`, `check`
(checkbox or radio), `slider`, `disclosure`, `row`, `menu` and `menu_row`,
`card` and `form_row` (the Settings anatomy sheets reuse), `progress`,
`empty_state`, `banner`, `tooltip`. Text and numeric fields are
`pm_shell::text::TextField`. Right-click menus are `NSMenu`
(`Host::context_menu`); a menu opened from a button is `menu`.

Every control: arrow cursor, hover wash, pressed fill, neutral keyboard-only
focus ring (`Tokens::focus_ring`), one symbol size per context
(`metrics.symbol_toolbar` 14, `symbol_panel` 14, `symbol_inline` 12).
Lists are quiet text for recall by name: no glyph tiles, no dividers.

## Timeline strips

`Tokens::strip(layer colour, StripState)`: a slab of tinted glass on the solid
timeline canvas. A saturated body in the layer's hue, lighter toward the
bottom, slightly translucent so the time grid reads through it; a 1 px rim of
light inside the edge instead of an outline; corner `strip_radius`; an even
`strip_gap`. Hover brightens the rim, selection makes it white, dragging lifts
it, disabled drops chroma and opacity. Labels and waveforms are tints of the
same hue. `Tokens::playhead` is a hairline with a round head; `trim_grip` is a
short pill inside the hovered edge. It is drawn by the timeline renderer,
never a system view.

## Three mistakes to avoid

1. A border or an outer shadow on a translucent surface. Use the elevation
   recipe; on panels the hairline is inside.
2. An accent selected state or focus ring. Selection is neutral and raised.
3. A literal size, colour or duration, or a second hover or spring
   implementation. Ask the tokens and the view's `Motion`.
