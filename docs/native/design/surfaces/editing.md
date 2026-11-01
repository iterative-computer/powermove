# Editing — path/text tools, text animators, colour parity, text services

Rows S025 S034 S035 S051. Extends [panels/viewer-chrome.md](../panels/viewer-chrome.md)
(gizmos, over-the-image rule) and [panels/inspector.md](../panels/inspector.md)
(rows, §7 colour popover). Everything drawn over the image uses the
**dark-scheme** token set (language §4 *Over the image*).

## S025 Path, mask and text tool states

Ports `E/viewer/path-editing.ts`, `E/viewer/text-editor.ts`,
`E/viewer/text-geometry.ts`. Behaviour (clicks, modifiers, snapping) is
v1.2.0's; this fixes the look and the state set.

| Element | Spec |
|---|---|
| Path stroke (editing) | 1 pt `c.accent` (mask paths: the mask's label colour at full strength, 1 pt), 2 pt under Increase Contrast |
| Vertex | square, side `m.keyframe − 2` (7); unselected: `c.floating` fill + 1 pt `c.accent` stroke; selected: `c.accent` fill + 1 pt `c.selected_text` stroke; hover: side +2 |
| First vertex while drawing | as vertex plus a `m.keyframe` ring when the pointer can close the path |
| Bézier handle | 1 pt `c.secondary` line; end = circle `m.keyframe − 3` (6) `c.floating` fill + 1 pt `c.secondary` stroke; only for selected vertices and their neighbours |
| Rubber band (pen preview) | 1 pt `c.accent` dashed (4 on, 3 off) from last vertex to pointer |
| Cursors | system arrow for selection; pen cursors are template images from SF Symbols: `pencil.tip` (draw), `pencil.tip.crop.circle.badge.plus` (add point), `…badge.minus` (delete point), `…badge.arrow.forward` (close path), `arrow.triangle.branch` (convert point). Hotspot at the tip. |
| Text: point vs paragraph | click with the Text tool = point text; drag = paragraph box (1 pt `c.secondary` dashed rectangle with the transform handles of viewer-chrome.md) |
| Text: editing | system insertion point and selection (`NSTextInputClient`; system `selectedTextBackgroundColor`, which follows AccentColor); the layer's bounds show as 1 pt `c.secondary` dashed while editing |
| Text: overset (paragraph) | trailing-bottom handle replaced by a `m.keyframe` square with a `+` in `c.warning`; tooltip `More text than fits` |
| Enter / exit | double-click a text layer or Text tool click enters; Esc or clicking outside commits and selects the layer; ⌘Return commits |

States: hover, selected, dragging (vertex follows pointer; handles mirror
per modifier), locked layer (no vertices; viewer `Layer is locked` banner),
IME composition (system marked-text underline; no app drawing).
VoiceOver: during path editing each vertex is an `AXButton`
`Point {i} of {n}, {x}, {y} pixels`, selected state exposed; the text
editor is an `AXTextArea` with the layer name.

Don't: draw handles for every vertex; colour vertices by type; draw a custom
caret or selection.

## S034 Text animators

The `animators` section of Properties (inspector.md §2). Rows follow inspector
grammar; this adds the states the row list lacked.

| Element | Spec |
|---|---|
| Section heading | `Text Animators` + trailing `plus` borderless, tooltip `Add Text Animator` (menu of the shipped animator presets, labels in Title Case) |
| Animator header row | disclosure chevron · checkbox (enable, AX `{name} enabled`) · name (`t.body`, double-click renames in place) · trailing `ellipsis.circle` menu: `Rename…`, `Duplicate`, `Move Up`, `Move Down`, separator, `Remove` (danger text) |
| Selector group | sub-heading `Range` with rows `Start`, `End`, `Offset` (`%` or characters per `Unit` pop-up), `Shape` pop-up, `Softness`, `Direction`, `Order` (incl. `Shuffle`), `Stagger`, `Mode` |
| Properties group | sub-heading `Properties` + trailing `plus` (`Add Animator Property`, menu of animatable properties not yet added); each property row has a trailing `minus.circle` (`Remove {property}`) on hover |
| Collapsed animator | header row only; keyframe diamond column shows a half-filled diamond if anything inside is animated |
| Disabled animator | header text `c.muted`; content rows stay editable (shipped behaviour) |
| Style selected text | when text is selected on the canvas the section shows a first row button `Style Selected Text`; otherwise the button is disabled with tooltip `Select text on the canvas first` |
| Empty | no animators: one `t.reading` `c.muted` line `Add an animator to move, fade or style characters over time.` |

Reorder by drag of the header row (panels/README drop line). VoiceOver: each
animator is an `AXGroup` `{name}, animator {i} of {n}`.

## S035 Colour popover — shipped capabilities kept

inspector.md §7 stays the base. These v1.2.0 capabilities are **kept** (N3)
and placed as follows; nothing else is added:

| Capability | Placement |
|---|---|
| `None` fill | Fill type segmented becomes `None` · `Solid` · `Linear` · `Radial` (Fill properties only); `None` hides the picker body |
| Previous / new swatch | the bottom-row swatch splits diagonally: top-left previous (click restores, tooltip `Restore Previous Colour`), bottom-right new |
| Colour space (`sRGB` / `Display P3`) and notation (`Hex`, `RGB`, `HSL`, `OKLCH`, `OKLab`, `P3`) | trailing `ellipsis.circle` pull-down in the bottom row, tooltip `Picker Settings`: section `Colour Space` (checkmarked) and section `Notation` (checkmarked); the hex field becomes a notation field showing that notation |
| Copy colour | same pull-down: `Copy as {notation}` (copies the current notation string) |
| Gamut warning | when the colour is outside the project's space: `exclamationmark.triangle.fill` `m.symbol_inline` `c.warning` before the notation field, tooltip `Outside sRGB; saved as the nearest sRGB colour` |
| Hue mapping | the same pull-down, section `Hue Mapping`, with the shipped options (Title Case, checkmarked) |
| Gradient midpoint | a small diamond (`m.keyframe − 2`) between adjacent stops on the gradient bar, draggable; ←/→ ±1 % when focused |
| Eyedropper | as §7 (`NSColorSampler`) |
| Recent colours | §7 row stays (native addition, marked as such in surfaces.md) |

## S051 Text context menu and text services

All text fields and text views are AppKit (`NSTextField`/`NSTextView`, P-TEXT)
or `NSTextInputClient` (canvas text), so the **system** supplies the context
menu (Cut, Copy, Paste, Spelling and Grammar, Substitutions, Transformations,
Speech, Services, AutoFill), Writing Tools, dictation, emoji and IME
candidates. Powermove adds nothing to it and removes nothing except:

| Field kind | Change |
|---|---|
| Number wells, timecode, hex/notation | spelling, grammar, substitutions and Writing Tools off |
| Expression editor, shader/code views | spelling and all substitutions off; smart quotes off |
| Canvas text, agent composer, layer names, notes | system defaults (spelling on as you type per the user's system setting) |

Focus transitions: clicking a text field takes key focus from the canvas/
timeline; Esc returns focus to the panel that had it before (not to the
canvas by default); editor shortcuts never fire while a text field is first
responder (panels/agent-panel.md rule applies app-wide).
