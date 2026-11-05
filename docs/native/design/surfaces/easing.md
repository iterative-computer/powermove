# Easing panel and preset library

Rows S094 S095. Grammar: [README](README.md) §1–5. Sources:
`R/panels/EasingPanel.svelte`, `R/panels/easing/{CurveEditor.svelte,curve,library,
operations,position,preset-files}.ts`. The timeline graph is a different surface
([timeline.md](../panels/timeline.md)); this panel shapes one curve and applies
it to selected keyframes.

No new tokens. One rule change from v1.2.0: `Curve Color` loses its blue, green
and violet choices, because timeline.md rules out per-axis hues and language §1
reserves colour for the work. A stored `blue`, `green` or `violet` preference is
kept in the file and read as `Accent`. User-chosen **preset** colours stay: those
are the user's own marks, like label colours.

## S094 Easing panel

Dockable panel `easing`, title `Easing`, default extent 320 pt as v1.2.0.

```
┌ Easing ───────────────────────────────── [⋯] ┐ header
│ [Copy] [Paste Easing] [Paste Values] [⇄]     │ tools row, m.row
│ ┌ graph ────────────────────────────┐        │ square-fit, c.panel_secondary
│ │        ╭───●                      │        │
│ │   ●───╯                           │        │
│ └───────────────────────────────────┘        │
│ [↶][↷] [xy] [Mirror] [+]     [Auto] [Apply]  │ graph tools, m.row
│ Outgoing   [ 0.33 ][ 0.00 ]                  │ coordinates, when on
│ (1) (2) (3) [+]                              │ preset group tabs
│ ▢ ▢ ▢ ▢ ▢ ▢                                  │ standard presets
│ ▢ ▢ ▢                                        │ multi-keyframe presets
│ 3 keyframes copied                           │ status line, t.minimum
└──────────────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Header `⋯` | `More Easing Tools` menu, below |
| Tools row | toolbar `Easing tools`. Borderless `m.hit` buttons: `Copy Keyframes` (`doc.on.doc`, with a `t.minimum` count badge when the clipboard holds keys), `Paste Easing` (`doc.on.clipboard`), `Paste Values` (`clock`), and in Toggle layout `Toggle Graph and Library` (`arrow.left.arrow.right`). Paste buttons disable with an empty clipboard |
| Graph | `Curve Editor` group. `c.panel_secondary`, `m.control_radius`, unit-square grid hairlines `c.separator`, diagonal `c.separator`. Curve 1.5 pt, `c.accent` for `Accent`, `c.text` for `Mono`. Key points `m.gizmo_handle` squares `c.text`; active point `c.accent`; tangent lines `m.gizmo_keyline` `c.secondary`, ends `m.tangent_end` circles. Overshoot extends beyond the square with `c.muted` guides at 0 and 1 |
| Graph tools | toolbar `Graph tools`: `Undo Curve`, `Redo Curve`, `Input Coordinates` (toggle), `Mirror Curve`, `Save Easing Preset`; trailing `Auto Apply` (toggle) and `Apply Curve` |
| Toggle buttons | pressed = `c.selected_fill` + `c.text` glyph, never accent fill |
| Coordinates | inspector rows; `Point` pop-up when the curve has more than two points; `Outgoing`, `Incoming` as X/Y pairs, 3 decimals, X 0–1. AX `outgoing handle X` |
| Preset groups | small tab group (panels/README tab recipe), then `New Preset Group` `plus`. ←/→/Home/End move between tabs |
| Preset tile | `prefs.size` square, `m.control_radius`, `c.field`; miniature curve in the preset's colour, else `c.secondary`; label below per preference. Tooltip = name, AX `Apply {name}`. Hover `c.hover`; Preview on: hovering plays a small dot along the curve (static under Reduce Motion) |
| Grids | `Standard presets`, then `Multi-keyframe presets`, each its own AX group. Columns from preference; 0 fits the width |
| Status | one line `t.minimum` `c.muted`, polite live region, replaces itself |

**Modes.** A two-point curve is standard. A curve with extra points is multi-keyframe,
and applying it writes those keys. Selected keys drive the graph: `Load Selected
Curve` reads them. `Auto Apply` on means every curve change applies at once to the
selected keys as one undo step `Adjust Easing`; off means `Apply Curve` does it,
named `Apply Easing`.

**Targets.** `Apply Curve`, a preset click and `Paste` affect all sections. Holding
modifiers narrows them, as v1.2.0's `sections(event)`: ⇧ incoming, ⌥ middle, ⌘
outgoing, in any combination. Right-click on `Apply Curve` shows the section menu: `All Sections`, `Incoming`, `Middle`,
`Outgoing`, `Incoming + Middle`, `Incoming + Outgoing`, `Middle + Outgoing`.
Applying with several properties selected shows `Apply Easing to…` with one row
per `{layer} · {property}`.

**Graph context menu**: `Reset Curve`.

**More Easing Tools** (`NSMenu`, v1.2.0 order):
`Load Selected Curve` · `Overshoot Expression` · `Bounce Expression` · `Remove
Expressions` · separator · `Linear Interpolation` · `Continuous Interpolation` ·
`Auto Bezier Interpolation` · `Clean Motion Path` · `Create Position Controller` ·
separator · `Import Presets…` · `Import Preset Folder…` · `Export Library…` ·
`Panel Settings` ✓.

Copy context menu: `Copy Keyframes`, `Duplicate at Playhead`, `Mirror at Playhead`.

**Panel Settings** (replaces the library area while on):

| Row | Control |
|---|---|
| `Layout` | pop-up `Graph and Library`, `Toggle` |
| `Columns` | number, 0–12, AX `Columns, 0 means automatic` |
| `Preset Size` | pop-up `32 px`, `44 px`, `56 px`, `72 px` |
| `Labels` | pop-up `None`, `Name`, `Influence` |
| `Curve Color` | pop-up `Accent`, `Mono` |
| `Preview` | checkbox |

| State | Spec |
|---|---|
| No keys selected | graph stays editable; `Apply Curve`, paste and tools needing keys disable with tooltip `Select keyframes first` |
| Empty group | `Save a curve or import presets` centred `t.reading` `c.secondary` |
| Operation refused | status line in `c.warning` with symbol and the operation's own message, for example `Select keys from one property, or use Copy Keyframes` |
| Copied | `{n} keyframes copied` |
| Path tools | `Path cleaned · Path progress keys selected`, `Path progress keys selected` |
| Library write failed | status line `c.danger` `Library couldn’t be saved. Export a backup.`; edits keep working in memory |

Keyboard (panel focused): ⌘C copy, ⌘V paste easing, ⇧⌘V paste values, ⌘Z and ⇧⌘Z
undo and redo **the curve**, not the project, as v1.2.0. Graph: Return cycles points,
arrows adjust the active handle, ⌥-click adds or removes a point. Dragging a handle
then Esc restores it.

VoiceOver: graph label `Easing graph. Drag handles. Option-click to add or remove
points. Return cycles points; arrows adjust the active handle.` Value reads the
active point's coordinates.

## S095 Preset and group management

Library data is `{groups, presets}` version 1, preserved byte-compatibly.

**Preset context menu** (`NSMenu`): `Apply Easing to…` · `Load into Graph` · separator
· `Rename…` · `Color…` · `Duplicate` · `Move to Group ▸` · `Export Preset…` ·
`Delete`.

**Group context menu**: `Rename Group…` · `Export Group…` · `Delete Group` (disabled
with one group).

**Library area context menu**: `Import Presets…`, `Import Preset Folder…`.

Name prompts are one small sheet (`m.sheet_width`, one row `Name`):

| Prompt | Title | Prefill | Default button |
|---|---|---|---|
| New group | `New Preset Group` | next number | `Create` |
| Save preset | `Save Easing Preset` | `Custom Easing` | `Save` |
| Rename preset | `Rename Preset` | current | `Rename` |
| Rename group | `Rename Group` | current | `Rename` |

Maximum 100 characters; empty disables the button; Return and Esc per sheet grammar.
A duplicate is named `{name} copy`.

**Color** — `Preset Color`: `NSPopover` from the tile with the kit colour well, plus
`Use Accent` (borderless) to clear the colour. Applies live per language §9 (v1.2.0's
Save and Cancel go away).

**Delete** — v1.2.0 removes at once with no prompt, and native keeps that speed:
`Delete` and `Delete Group` apply immediately, then the status line reads
`Deleted “{name}”` with a borderless `Undo` for as long as the panel stays open.
The removed items are archived in the library file (N4). There's no restore UI after
that; see [README §6](README.md#6-owner-questions-raised-by-these-specs).

**Move and reorder**: drag a tile within a grid (insertion line) or onto a group tab
(Drop target).

**Import** — `NSOpenPanel`, `.json` and `.kease`; a folder picks every supported
file, each folder becoming a group named after it.

| Result | Status line |
|---|---|
| Success | `Presets imported` |
| Too many | `Choose up to 5,000 preset files` |
| Too large | `Preset imports must be smaller than 10 MB` |
| Nothing usable | `No supported preset files found. Choose JSON or .kease files.` |
| Vector preset | `Use the component importer for vector presets` |
| Corrupt | `Couldn’t read {file}.` with detail to the Error Log |

Imported presets merge by id; conflicts get new ids, so nothing is overwritten.

**Export** — `NSSavePanel` sheet, default `Easing presets.json`. Result `Presets
exported`, or the Error copy in the status line.

## Don't

- Don't make the panel's ⌘Z touch project history.
- Don't tint the graph or the panel with preset colours.
- Don't add a confirmation to preset deletion.
