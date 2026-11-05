# Properties (inspector)

Panel id `inspector`, title **Properties**, top of the right dock (Effects
below it). Shared rules: [README.md](README.md). Look:
[language.md §9](../language.md#9-controls).

Behaviour source: `v1.2.0:apps/desktop/src/extensions/inspector/`
(`InspectorPanel.svelte`, `InspectorHeader.svelte`, `AlignmentStrip.svelte`,
`CompositionSection.svelte`, `ContentSection.svelte`, `TypeSettings.svelte`,
`TransformSection.svelte`, `ChannelRow.svelte`, `PropertyStopwatch.svelte`,
`StructuredSection.svelte`, `MasksSection.svelte`, `RetimingSection.svelte`,
`LayerOptions.svelte`, `TextAnimators.svelte`, `multi-edit.ts`) and the
controls in `v1.2.0:apps/desktop/src/renderer/src/controls/`. Workflows
W063, W066, W073, W074, W075.

Effects are **not** in this panel: they live in
[Effect Controls](effect-controls.md) (v1.2.0 "Layer Effects").

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `InspectorTarget` | `NoProject` · `Composition` · `Layers{ids, first}` |
| `sections` | ordered `Section{id, title, rows, collapsed}`; ids `composition`, `content`, `type`, `transform`, `structured`, `shader`, `extension`, `masks`, `time`, `layer`, `animators`, plus contributed sections (`after: content|transform|effects`) |
| `Row` | `{id, label, control: Number{units, step, min, max, axes} · Text · Choice · Toggle · Colour · Fill · Point · Font · Segmented, anim: Static · Animated · KeyAtPlayhead, expression: Option<Expr>, mixed: bool, locked: bool}` |
| `scroll_by_selection` | per selection key, restored on reselect (B342) |

## 2. Anatomy

```
┌ Properties ─────────────────────────── [✦] [+] ┐  header m.header
│ Headline · Text                                 │  identity line
│ [⊢][⊣|][⊣] [⊤][⊥-][⊥]                           │  align strip (not for audio)
│ Content                                       ⌄ │  section heading
│ ◇ Font ............... [SF Pro Display      ⌄] │  row m.row, field m.field
│ ◆ Size ............... [164             px ]   │  value column m.value_column
│ ◇ Color .............. [■][#F5F3EF          ]  │
│ Transform                                       │
│ ◈ Position ........... [960  px][540  px]      │
│ ◆ Scale .............. [100   %][100   %] ⛓    │  link outside the column
└─────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Header | README §2. Title `Properties`. Trailing buttons: **Add Effect** (`wand.and.rays`, tooltip `Add Effect`, opens the effect menu — same as Effect Controls' Add Effect; hidden for layers that take no effects) and **New Layer** (`plus`, tooltip `New Layer`, opens menu `Text`, `Shape`, `Solid`, `Adjustment Layer`, `Null`, separator, `Import Media…`). |
| Identity line | height `m.row`, inset `m.inset`. Layer name `t.title` `t.semibold_weight` `c.text`, then ` · ` and the type (`Text`, `Shape`, `Video` …) in `t.title` regular `c.muted`. Multi-select: `{n} Layers` + ` · ` + `Mixed` or the shared type. Truncates the name first; tooltip full. |
| Align strip | one row `m.row`; two GPUI segmented-look groups (README tab-group recipe but momentary — no selected segment): Left, Horizontal Centre, Right │ Top, Vertical Centre, Bottom. Glyphs `align.horizontal.left`, `align.horizontal.center`, `align.horizontal.right`, `align.vertical.top`, `align.vertical.center`, `align.vertical.bottom` at `m.symbol_inline`. Each segment ≥ `m.hit` wide. `m.gap` between groups. Hidden for audio layers and for no selection. Tooltips `Align Left`, `Align Horizontal Centres`, `Align Right`, `Align Top`, `Align Vertical Centres`, `Align Bottom` (v1.2.0 `AlignmentStrip.svelte`: one layer or several, the union box aligns to the **composition**, keeping relative offsets — detail line `To composition`). |
| Section heading | `t.title` `t.semibold_weight` `c.text`; padding top `m.inset`, bottom `m.gap`; inset `m.inset`. Disclosure chevron (`chevron.down`, rotates to `chevron.right` collapsed) at the **trailing** inset, `m.symbol_inline` `c.muted`, shown on heading hover, on keyboard focus, and always while collapsed. Click anywhere on the heading toggles. Collapsed state persists per project per section id. Contributed sections use the same heading. Optional trailing heading button (Masks `+` `Add Mask`, Text animators `+` `Add Text Animator`) sits before the chevron. |
| Row | README/language §9 inspector row: keyframe diamond column (`m.keyframe` glyph in an `m.hit` square starting at `m.inset − (m.hit − m.keyframe) ÷ 2`), label (flexible, `t.label` `c.secondary`, truncating, tooltip), value column `m.value_column` wide ending at `m.inset` from the panel edge. One value column for the whole panel. Row height `m.row`; fields `m.field`, centred. |
| Number well | `c.field`, `m.control_radius`, padding `m.gap` each side, value `t.body` tabular `c.text` **left-aligned**, unit `c.muted` after a space, trailing in the well. Two axes: two equal wells, `m.gap − 2` (4 pt) between. |
| Link toggle (Scale) | `link` / custom `link.slash`, `m.symbol_inline`, `c.secondary` / `c.muted`. The value column never moves: the glyph is centred in the `m.inset` margin to the right of the column (centre `m.inset ÷ 2` from the panel edge) and its `m.hit` square overlaps the column's last pixels; the second Scale well is shortened by `m.hit − m.inset` so the two never collide. Tooltip `Lock Aspect Ratio` / `Unlock Aspect Ratio`. |
| Choice | GPUI pop-up well (`c.field`, value `t.body` `c.text`, `chevron.up.chevron.down` at `m.symbol_inline` `c.muted` trailing) opening a real `NSMenu` positioned so the current item sits over the well (system pop-up behaviour). |
| Font | Choice well whose menu is the **field menu** (glass child panel, `m.field_menu_width` × ≤ `m.field_menu_height`): search field on top (`Search fonts`), families in their own face at `t.body`, current family checked; Refresh at the bottom (`Refresh Fonts`). Weight is a separate Choice row. |
| Colour | swatch `m.field` × `m.field` (square), `m.control_radius`, 1 pt inner keyline `c.separator`, checkerboard under alpha; then a well with the hex (`#F5F3EF`, uppercase, tabular) filling the rest of the column (`m.gap − 2` gap). Click the swatch → colour popover (§7). Click the hex → text edit. |
| Fill (solid/gradient) | same as Colour; a gradient shows a gradient swatch and the well reads `Linear Gradient` / `Radial Gradient`. Popover has the Solid/Gradient segmented control (§7). |
| Toggle | checkbox at the value column's leading edge (README §5.5). |
| Segmented (Align text, Animate On/Off …) | GPUI segmented filling the column; icons `m.symbol_inline` or labels `t.minimum` `t.medium_weight`. |
| Multi-line text (Text content) | AppKit text view, `c.field`, `m.control_radius`, `t.body`, min 2 lines, max 6 lines then scrolls; row grows; label top-aligned with the first line. |
| Point pad (2-D point controls in effects/content) | two number wells (X, Y) + `scope` button opening a popover pad (W073 point pad keys). |
| Expression row | appears under its property when an expression exists: label `Expression` `c.muted`; full-width (label column + value column) SF Mono `t.body` field on `c.field`, min 1 line, max 6; trailing status glyph `checkmark.circle` `c.success` (`Expression is valid`) or `exclamationmark.triangle.fill` `c.danger` with the error as tooltip and as a `t.minimum` `c.danger` line under the field. |
| Composition hint | under the Composition section: `Select a layer to edit its properties.` `t.minimum` `c.muted`, inset `m.inset`, top padding `m.gap`. |

### Section order (v1.2.0 `InspectorPanel.svelte`)

No selection → **Composition** (`Width`, `Height`, `Frame Rate`,
`Duration`, `Background`) + hint.
Layer(s) → identity line, align strip, then: **Content** (type-specific; for
groups none; captions → **Captions**) → contributed `after: content` →
**Transform** (not for audio) → contributed `after: transform` →
**structured** sections (Fill, Stroke, Trim Paths, Repeater, Path Transform,
Mask Paths) → **Shader** / extension params (by type) → contributed
`after: effects` → **Masks** → **Time** (video, precomp) → **Layer**.
**Type settings** and **Text animators** belong to text layers and follow
Content.

Property names (exact, Title Case): Content — `Text`, `Font`, `Weight`,
`Size`, `Tracking`, `Leading`, `Align`, `Color`, `Fill`, `Shape`, `Width`,
`Height`, `Corner Radius`, `Corner Smoothing`, `Independent Corners`,
`Stroke`, `Stroke Color`, `Points`, `Fit`, `Source`, `Speed`, `Trim Start`,
`Gain`, `Fade In`, `Fade Out`, `Mute Audio`. Transform — `Anchor Point`,
`Position`, `Scale`, `Rotation`, `Skew`, `Opacity` (3-D layers add
`Orientation`, `X Rotation`, `Y Rotation`, `Perspective`; everything else about 3D
layers is the `3D` section in [3d-studio.md §8](../surfaces/3d-studio.md#8-3d-section-in-properties)). Layer — `Blend Mode`, `Motion Blur`, `Track Matte`, `Matte Mode`,
`Parent`, `Visible`, `Solo`, `Label Color`. Time — `Time Remap`, `Source
Time`, `Reverse`, `Freeze Frame`. Masks — `Mask Shape`, `Mode`, `Enabled`.

Values: lengths `px`, percentages `%` (Opacity, Scale, Corner Smoothing),
angles `°`, leading unitless (`1.2`, language §5), time `0:00:12` or
`HH:MM:SS:FF` for composition time, sizes `1920×1080`. Display precision =
the property's step (integers for px; one decimal for % unless the value has
more; two decimals for unitless).

## 3. States

| State | Visual |
|---|---|
| **Empty (no project)** | README empty state: symbol `sidebar.trailing`, `No Composition`, `Open a project to start editing.` No button. |
| **No selection** | Composition section + hint (above). This is **not** an empty state (v1.2.0 behaviour). |
| **Loading** | Not applicable: values are synchronous from the model. A font list still loading shows the font menu's spinner row `Loading fonts…`. |
| **Hover** | well → `c.hover` + `resizeLeftRight` cursor (number) / arrow (others); heading → chevron appears; diamond → `c.text`. |
| **Focus** | Tab focus ring on wells, diamonds (ring around the diamond's hit square, `m.control_radius`), checkboxes (system), headings (ring around the heading row). |
| **Editing** | Number becomes an AppKit field (P-TEXT): `c.field` stays, system focus ring (orange), contents selected, value `t.medium_weight`. |
| **Scrubbing** | well stays `c.hover`; value updates live, tabular, no width change; pointer hidden and unbounded; label turns `c.text` for the duration. |
| **Animated** | diamond: outline `c.muted` static; half-filled `c.accent` animated; filled `c.accent` key at playhead (language §9). |
| **Expression-driven** | value well text in `c.accent_text` (the value is computed), diamond as animated state; typing into the well asks nothing — it edits the expression's base value (v1.2.0). |
| **Mixed (multi-select)** | well shows `—` (em dash) in `c.muted`, no unit; scrubbing starts from the first layer's value and applies the same delta to all; typing sets all. Toggle shows the system mixed state. Colour swatch shows a diagonal split of the first two colours. |
| **Not applicable to some selected layers** | row is shown if ≥ 1 selected layer has it (B391); edits skip the others. |
| **Locked layer / locked ancestor** | wells `c.muted` text, no hover, not editable; identity line gets `lock` `m.symbol_inline` `c.muted` after the type; tooltip `Locked layers can’t be edited.` (B284). |
| **Disabled row** (e.g. Track Matte with no layer above) | label and value `c.muted`, no hover. |
| **Error** | edit rejected by the model: value reverts, `t.minimum` `c.danger` line under the row `Couldn’t change {Property}. Try again.` (clears on the row's next accepted edit). Updates lost: wells disabled, one Error line at the top of the content `Properties aren’t updating right now.` (`c.warning` triangle), cleared when publications resume. Native menu failure: no UI. Typed value invalid → field shakes nothing; it stays open with a `t.minimum` `c.danger` line under the row (`Enter a number.` / `Enter a value from {min} to {max}.`) and the previous value restored on Esc. |
| **Drop** | dropping an effect onto the panel adds it to the first selected layer (drop target = whole content area). |
| **Playing** | values of animated properties update on the readout clock (B351–B353), at most once per display frame, without re-laying out rows. |
| **60 k rows** | N/A for rows; a selection of 60 000 layers renders the first layer's rows (B359) and computes mixed state lazily per visible row. |

## 4. Interactions

| Gesture | Result |
|---|---|
| Drag well | scrub (language §9: step × 0.5 per pt, ⇧ ×10, ⌥ ×0.1, threshold `m.drag_threshold`, one undo named for the property) |
| Click well | type (arithmetic, relative `+10`, `*2`) |
| ↑/↓ while editing (⇧ ×10) | step |
| Scroll wheel on a rested well | README §5.2 |
| Click diamond | static → set animated and add key at playhead; animated → toggle key at playhead only (B365) |
| ⌥-click diamond | remove animation (all keys) — v1.2.0 stopwatch semantics; keeps the current value |
| Right-click row | `NSMenu`: `Add Keyframe at Playhead`, `Remove Animation`, separator, `Add Expression…` / `Edit Expression…`, `Remove Expression`, separator, `Reset`, `Show in Graph Editor`, and for Scale `Link Scale Axes` ✓ |
| Click link | toggle aspect lock (shared with the timeline, B143) |
| Click heading | collapse/expand |
| Click swatch | colour popover |
| Drag Parent pick whip (`link` glyph at the leading edge of the Parent value well) | as timeline pick whip; Esc cancels (`Drag to a layer to parent · Esc cancels`) |
| Click align button | align (one undo `Align`) |

## 5. Keyboard and focus

Tab order: header buttons (Add Effect, New Layer) → align strip (one stop,
←/→ between buttons, Space/Return activates) → for each section: heading →
each row's diamond → its control(s) left to right → link → next row.
Collapsed sections contribute only the heading. W1 local routes: numeric
readout (Return/F2 edit, ↑/↓ ±step, ⇧ ×10), numeric draft (Return commits,
Esc cancels), point pad, toggle, segmented, select, font picker, colour
picker — exactly as w1-keymap *Focus-local keyboard routes*. Editor shortcuts
do not fire while a field is editing (W010). Esc in a field cancels the edit;
a second Esc returns focus to the timeline.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Panel | group | `Properties` |
| Identity line | heading | `Headline, text layer` / `3 layers selected` |
| Section heading | disclosure triangle + heading | `Transform`, expanded/collapsed |
| Diamond | toggle button | `Keyframe, Opacity`; value `not animated` / `animated` / `keyframe at playhead` |
| Number well | slider-like text field (`AXIncrementor` semantics) | `Opacity, 100 percent`; increment/decrement = step; two axes = two elements `Position X, 960 pixels`, `Position Y, 540 pixels` |
| Mixed | same | value `mixed` |
| Colour | colour well | `Color, F5F3EF hex` (+ `, 80 percent opacity` if alpha < 1) |
| Link | toggle | `Lock aspect ratio`, on/off |
| Align buttons | buttons | tooltip names |
| Expression | text area | `Expression for Opacity`; status `valid` / error text |

## 7. Colour popover

`NSPopover` anchored to the swatch, arrow towards it, width
`m.popover_width`; glass frame, content on solid `c.floating` (language §3).
Edits apply live; closing (Esc reverts, outside-click commits) is one undo
step named `Change {property}`.

```
┌──────────────────────────────┐
│ [ Solid | Linear | Radial ]  │  only for Fill properties
│ ┌──────────────────────────┐ │
│ │  saturation × brightness │ │  height = m.popover_width ÷ 2
│ └──────────────────────────┘ │
│ ═══════ hue ═══════════════  │  track m.keyframe + 3
│ ═══════ opacity ═══════════  │  checkerboard under
│ [■] [#F5F3EF        ] [100 %] [⊙] │
│ ● ● ● ● ● ● ● ●  recent      │  document colours
└──────────────────────────────┘
```

| Element | Spec |
|---|---|
| Padding | `m.inset` on all sides; every element spans the content width |
| Type segmented (Fill only) | `NSSegmentedControl` small: `Solid` · `Linear` · `Radial` |
| SB square | height `m.popover_width ÷ 2`, `m.control_radius`; thumb 12 pt circle with 2 pt white ring + 1 pt `#000` 40 % outer ring (drawn on the image, not a token colour — it must read on any hue; implement with `c.selected_text` ring and `c.media_badge` outer ring) |
| Hue and opacity sliders | track height `m.keyframe + 3` (12), capsule; thumb `m.track_row ÷ 2 + 2` (16) circle `c.floating` with system shadow; `m.gap + 4` (10) between rows |
| Bottom row | swatch `m.field` square; hex field (`t.body` tabular); opacity number well (`%`, scrubbable); eyedropper borderless button `eyedropper` (`NSColorSampler`), tooltip `Pick a Colour From the Screen` |
| Gradient (Linear/Radial) | a gradient bar (height `m.field`) with stop handles (W073 ramp keys: ←/→ ±1, ⇧ ±10, Home/End); click the bar adds a stop; drag a stop off the bar removes it; the SB/hue/opacity controls edit the selected stop; angle well (`°`) for Linear |
| Recent colours | row of up to 8 circles `m.field` diameter, document colours most recent first; click applies |
| Keyboard | Tab cycles inside the popover (focus trapped only in popover mode, W074); arrows adjust the focused axis (⇧ ×10); Return in hex commits; Esc cancels the preview and closes |
| VoiceOver | SB square = 2-D slider `Saturation and brightness` (value `saturation 40 percent, brightness 96 percent`); sliders `Hue`, `Opacity`; eyedropper button |

## 8. Appearance

- **Dark / Light**: tokens only. Wells `c.field` on `c.panel`; diamond
  colours as language.
- **Increase Contrast**: wells get a 1 pt `c.separator` border; section
  headings get a hairline above (`m.border` `c.separator`).
- **Reduce Motion**: section collapse without animation.
- **Reduce Transparency**: popover frame solid `c.floating`.

## 9. Copy

| Where | String |
|---|---|
| Composition section rows | `Width`, `Height`, `Frame Rate`, `Duration`, `Background` |
| Composition hint | `Select a layer to edit its properties.` |
| Locked tooltip | `Locked layers can’t be edited.` |
| Invalid number | `Enter a number.` · `Enter a value from {min} to {max}.` |
| Expression valid | `Expression is valid` |
| Font menu | `Search fonts` (placeholder) · `Refresh Fonts` · `Loading fonts…` · `No fonts match “{query}”` |
| Parent | `None` · `Drag to a layer to parent · Esc cancels` |
| Undo names | `Change {Property}` (e.g. `Change Opacity`), `Align`, `Add Keyframe`, `Remove Animation`, `Reset {Property}`, `Link Scale Axes`, `Set Parent` |

## 10. Don't

- Don't right-align values, don't vary well widths between rows, don't let a
  value overflow the panel edge (na-viewer2 review).
- Don't use switches in the inspector; don't put the unit in the label
  (`Opacity %`).
- Don't show the effects stack here.
- Don't show the "No Selection" empty state when a project is open — show
  the composition.
- Don't open the colour picker as a sheet or `NSColorPanel` by default; the
  popover is the picker (the eyedropper uses `NSColorSampler`).
