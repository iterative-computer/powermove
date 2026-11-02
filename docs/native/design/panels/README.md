# Panel specs — shared rules

Status: **build specs** for the Powermove Native panels. Owner: design lead.
Governing documents: [language.md](../language.md) (look),
[tokens.md](../tokens.md) (values), [review-checklist.md](../review-checklist.md)
(gate), [w1-keymap.md](../../w1-keymap.md) and
[w1-workflows.md](../../w1-workflows.md) (behaviour), and
[panels-mockup.html](../panels-mockup.html) (pictures of the key states).

A lane builds a panel from its spec **without making design decisions**. If
a spec is silent or contradicts the code, the lane stops and writes
`DESIGN-TODO(opus): <question>` in its PR. It does not invent a value, a
string or a behaviour.

| Spec | Panel id (`pm-shell` `PANELS`) | Title |
|---|---|---|
| [timeline.md](timeline.md) | `timeline` | Timeline |
| [inspector.md](inspector.md) | `inspector` | Properties |
| [effect-controls.md](effect-controls.md) | `layer-effects` | Effect Controls |
| [viewer-chrome.md](viewer-chrome.md) | `viewer` | Viewer (no visible title) |
| [media-browser.md](media-browser.md) | `assets` | Project |
| [effects-presets.md](effects-presets.md) | `fxbrowser` | Effects (owner decision 3) |
| [agent-panel.md](agent-panel.md) | agent surface | Agent |
| [settings.md](settings.md) | Settings window | Settings |
| [export-sheet.md](export-sheet.md) | Export sheet | Export |

Precedence when sources disagree: **v1.2.0 wins on behaviour and shortcuts;
HIG wins on placement and look; language.md wins over both on Powermove's own
look** (accent, density, neutral grey). Every spec cites the v1.2.0 file it
ports.

---

## 1. How to read a spec

- **Token names** are `pm_design::Tokens` paths without the `Tokens.` prefix:
  `colors.panel`, `metrics.row`, `typography.body`, `motion.interaction`.
  Short forms in tables: `c.` = `colors.`, `m.` = `metrics.`, `t.` =
  `typography.`, `mo.` = `motion.`.
- **Layout math on tokens is allowed** and is written out (`m.track_row −
  2 × m.inner_radius`). A bare number in a spec is either a count, a ratio,
  or a value listed in §6 *Token additions* — never a licence to hard-code.
- **Copy** in `code` is exact, including capitalisation, the ellipsis
  character `…` (U+2026), the multiplication sign `×`, curly quotes `“ ”`
  and the middle dot `·` used as a separator with a space either side.
  `{name}` is a substitution.
- **Model state names** (`TimelineMode::Layers`, …) are the contract between
  the panel and its model lane. The model branches (`feat/na-cloud-m-*`) had
  no commits on 2026-11-01, so the names here are proposals; if the model
  lane ships different names, the model wins and the lane updates the spec in
  the same PR.

---

## 2. Common anatomy

Every docked panel is a card: `c.panel` fill, `m.panel_radius` corners,
`m.gap` from neighbours and the window edge, no border in normal contrast
(Increase Contrast adds a `m.border` `c.separator` edge).

```
┌──────────────────────────────────────────────┐
│ Title / tabs                    [⌕] [+] [⋯] │ ← header, m.header (32)
├──────────────────────────────────────────────┤   (no separator line; the
│ content, inset m.inset (12)                  │    content starts below)
```

| Element | Tokens |
|---|---|
| Header height | `m.header` |
| Header leading inset | `m.inset` |
| Header trailing inset | `m.inset − (m.hit − m.symbol_panel) ÷ 2` so the glyph's optical edge sits on the inset |
| Title text | `t.title`, `t.semibold_weight`, `c.text` |
| Tab group (two or more panels in one dock group) | segmented control: track `c.field`, padding `m.control_radius − m.inner_radius`, segments `m.inner_radius`, label `t.body` `t.medium_weight`; selected segment `c.floating` + `c.text`; others `c.secondary` |
| Header icon button | `m.hit` square, glyph `m.symbol_panel` `c.secondary`; hover fill `c.hover` with `m.control_radius`; pressed fill `c.selected_fill`; tooltip = command name + shortcut |
| Content inset | `m.inset` left and right; first content row starts directly under the header |
| Scroll bars | system overlay scrollers (`NSScroller` overlay style); never always-visible legacy bars unless the user's System Settings asks for them |

Panel header buttons appear **in this order, trailing**: search, filter or
view options, add (`+`), more (`ellipsis.circle`). A panel shows only the
buttons its spec lists.

### Panel context menu

Right-click on a header (title or tab) shows the **dock menu** owned by
`pm-shell` (Close Panel, Move to Left Dock / Right Dock / Bottom Dock with a
checkmark on the current one, Pop Out). Panels do not add to it.

---

## 3. Shared state vocabulary

Every spec has a **States** table with these rows (N/A rows say why). The
visuals below are the defaults; a spec only restates a row when it differs.

| State | Default visual |
|---|---|
| **Empty** | `EmptyState` centred in the content area: SF Symbol at 2 × `m.symbol_panel` in `c.muted`, title `t.heading` `c.text` (Title Case, like the system's content-unavailable view), one line `t.reading` `c.secondary` (sentence case), optional one push button (`m.control`). Vertical centre is the content area's centre, not the panel's. |
| **Loading** | Under 0.5 s: nothing changes (no flash). After 0.5 s: system indeterminate spinner (small) centred where content will appear, with the reason in `t.minimum` `c.muted` below it. Rows that are individually loading show a placeholder well in `c.field` at the row's real geometry — never a spinner per row. |
| **Hover** | Rows and borderless buttons: `c.hover` fill, `mo.interaction` linear. Text never changes colour on hover. Wells: `c.hover`. |
| **Focus (keyboard)** | Focus ring `c.focus`, 2 pt outside the focused shape, following its radius; AppKit controls use the system ring (tinted orange by AccentColor). Lists: the focused row draws the ring inset 1 pt inside the row's selection shape. Only shown for keyboard focus (Full Keyboard Access or Tab). |
| **Selected** | List rows: `c.selected_fill` full row (inset `m.gap` ÷ 2 horizontally, `m.control_radius`), text stays `c.text`. Accent only on the item's *mark* (thumbnail ring 2 pt `c.accent`, keyframe diamond `c.accent`, clip ring per timeline.md). |
| **Selected, panel not key** | Same fill, unchanged (a neutral fill does not need an inactive variant). Accent marks become `c.secondary` when the window is not key. |
| **Disabled** | Text and glyphs `c.muted`; wells keep `c.field`; no hover; the system disabled appearance for AppKit controls. Never lower opacity of a whole region. Disabled items stay in focus order only when HIG does so for that control (menus: yes; buttons: no). |
| **Error** | Inline, at the place of the failure: `exclamationmark.triangle.fill` at `m.symbol_inline` in `c.danger` (or `c.warning` for recoverable), message `t.body` `c.danger`/`c.warning` text with the fix in the same sentence. Never a red fill, never colour alone. |
| **Drag source** | The dragged item renders at `m.drag_opacity` (50 %) in place (system drag image is the item at 100 %, system shadow). |
| **Drop target** | `c.accent_wash` fill + `m.drop_stroke` (1.5 pt) `c.accent` inner stroke on the target shape (`m.control_radius`); for a gap between rows, a 2 pt `c.accent` insertion line with an `m.status_dot` circle at its leading end. Invalid target: no highlight and the system "not allowed" cursor. |
| **Multi-select** | Every selected row gets the selected fill; the anchor row (last clicked) has the focus ring when keyboard-focused. A count appears where the spec says (never as a toast). |
| **Large data (60 k rows)** | §4. |

### Transitions

Only state changes animate. Hover/press/select: `mo.interaction`. Rows
appearing or disappearing (disclosure): height animates with the
`mo.enter`/`mo.bounce` spring; under Reduce Motion they appear at once with a
`mo.reduced` cross-fade. Scrolling is never animated by the app except
"scroll to reveal" (system `scrollRectToVisible`, animated only when the
target is within two viewport heights, otherwise jump).

---

## 4. Large data (60 000 rows)

Every list and the timeline must stay responsive with 60 000 rows (layers,
media, effects, thread messages). Rules:

1. **Virtualise.** Lay out and paint only rows intersecting the viewport plus
   one viewport of overscan above and below. Row heights are fixed tokens
   (`m.row`, `m.track_row`, `m.property_row`) so offsets are arithmetic; a
   variable-height list (agent conversation) caches measured heights and
   estimates unmeasured ones from the row kind.
2. **Never measure all text.** Column widths that depend on content (the
   timeline index column) are computed from the digit count of the largest
   value, not by measuring rows.
3. **Selection is a set of ids,** not row flags; select-all is O(1) ("all
   except") until the user deselects one.
4. **Scroll position survives** model updates: anchor on the first visible
   row's id, not on a pixel offset.
5. **Search/filter** runs off the main thread when the list exceeds 2 000
   rows; while it runs the previous results stay visible and the search field
   shows the system's progress inside its trailing edge. No "Searching…"
   overlay.
6. **Accessibility** exposes the full row count (`AXRowCount`) and the
   visible rows; VoiceOver's row navigation scrolls the virtual list.
7. **Scroll bars** reflect the full logical height; dragging the scroller to
   the middle of 60 000 rows lands there in one frame (placeholder wells
   permitted for one frame, never a blank panel).

---

## 5. Resolutions of the UI-kit DESIGN-TODOs

From [ui-kit.md](../../ui-kit.md) on `feat/na-cloud-uikit`. These are now
decided; the kit lane implements them.

1. **Segmented-track padding** = `m.control_radius − m.inner_radius` (2 pt
   today). It is concentric math, not a new token: a segment of radius
   `inner_radius` nested in a track of radius `control_radius` sits at that
   inset.
2. **Wheel editing of numbers.** A scroll-wheel/trackpad event adjusts a
   `NumberField` only when **all** hold: the pointer has rested on that well
   for `mo.wheel_rest` (new token, 0.5 s) without moving more than
   `m.drag_threshold`; no scroll of the containing scroll view happened in
   the last `mo.wheel_rest`; the event is a discrete mouse-wheel event *or*
   a trackpad event with `⌥` held. Step = property step per notch, ⇧ ×10,
   ⌥ ×0.1 (⌥ alone on a trackpad means ×1). Affordance: the well already
   shows `c.hover` and the horizontal-resize cursor; when wheel editing
   becomes armed the cursor stays the same — no extra chrome. A wheel edit
   sequence is one undo step that closes `mo.wheel_rest` after the last
   event.
3. **Colour-picker contents** — specified in
   [inspector.md §7](inspector.md#7-colour-popover).
4. **Tree indentation** = `m.tree_indent` (new token, 16 pt, AppKit's
   outline default). Disclosure glyph `chevron.right` (rotates 90° to open,
   `mo.interaction`) at `m.symbol_inline`, `c.secondary`, centred in an
   `m.hit` square whose leading edge is at `m.inset − (m.hit −
   m.symbol_inline) ÷ 2 + depth × m.tree_indent`. The row's content starts
   at the hit square's trailing edge.
5. **Checkbox and switch placement in a value column.** Inspector rows use a
   **checkbox** (`NSButton` checkbox, regular size), never a switch. It sits
   at the **leading** edge of the value column, vertically centred in the
   row, with no title (the row label is its label; AX label = row label).
   Switches appear only in Settings (trailing-aligned, mini size not used).
   A two-state option with named states (On/Off with different meanings,
   e.g. "Animate") is a segmented control filling the column, not a toggle.
6. **`SheetAnchor` placement.** ui-kit.md says "system placement below
   toolbar". Replace with "system placement" — on macOS 26+ (including 27)
   the system centres sheets in the window; the kit sets no position
   (language §9 *Sheets and alerts*).

---

## 6. Token additions and changes requested

The pm-design lane adds these with role sentences (tokens.md §How to change
a token). Specs reference them by name; until they land, a lane may not
substitute literals — it blocks on the token.

| Token | Value (both schemes, all contrasts unless noted) | Role |
|---|---|---|
| `motion.wheel_rest` | 0.5 | Pointer rest before the scroll wheel edits a hovered value (s) |
| `metrics.tree_indent` | 16 | Indent per outline level |
| `metrics.kind_badge` | 18 | Layer-kind badge square in the timeline and Project list |
| `metrics.thumbnail_min` | 120 | Minimum Project grid thumbnail width |
| `metrics.sheet_inset` | 20 | Content inset inside sheets and Settings panes |
| `metrics.sheet_value_column` | 240 | Shared value column in sheets |
| `metrics.sheet_width_wide` | 600 | Width of the Export sheet |
| `metrics.composer_max_lines` | 10 | Agent composer grows to this many lines, then scrolls |
| `colors.media_badge` | `#000000` at 55 % alpha | Fill of badges drawn over media (duration, offline) — always dark, see language §4 *Over the image* |
| `colors.viewer` (light **change**) | `#E4E4E4` (was `#E6E6EA`) | Viewer surround is pure neutral in light too, so footage reads true (language §4) |
| `metrics.group_radius` | 10 (= `control_radius + inner_radius`) | Settings grouped-form card corner (reviews/na-cloud-p-settings.md) |
| `metrics.form_row` | 40 (= `row + inset`) | Minimum Settings row height |
| `metrics.section_gap` | 40 (= 2 × `sheet_inset`) | Space above each Settings section after the first |
| `metrics.gizmo_handle` | 7 (= `keyframe − 2`) | Transform handle and path vertex square (viewer-chrome.md §2) |
| `metrics.gizmo_keyline` | 1 | Gizmo stroke and keyline width over the image |
| `metrics.rotation_stem` | 18 | Rotation handle stem length |
| `metrics.anchor_mark` | 11 | Anchor point crosshair circle |
| `metrics.tangent_end` | 6 | Bézier tangent end circle |
| `metrics.checker` | 8 | Transparency checkerboard square |
| `metrics.pasteboard_opacity` | 0.35 | Selected layers drawn outside the frame (opacity) |
| `motion.overlay_idle` | 2.0 | Still time before the Focus overlay capsule fades (s) |
| `metrics.status_dot` | 6 | Attention/unread dot and the drop insertion-line end circle (reviews/na-cloud-p-agent.md) |
| `metrics.text_padding` | 8 (= `gap + 2`) | Vertical padding of text in bubbles, the agent composer and code blocks |
| `metrics.drop_stroke` | 1.5 (Increase Contrast 2) | Drop-target inner stroke (§3 *Drop target*) |
| `metrics.drag_opacity` | 0.5 | Opacity of a drag source left in place (§3 *Drag source*) |

---

## 7. Accessibility defaults

- Every panel content root is an `AXGroup` labelled with the panel title.
  Lists are `AXList`/`AXOutline`/`AXTable` with `AXRowCount`; rows expose
  `AXSelected`.
- Labels are the visible text; icon-only buttons use the tooltip's command
  name (`Add Effect`, not `plus`).
- Values read with units in words: `Opacity, 100 percent`,
  `Position X, 960 pixels`, `Rotation, 0 degrees`, `Duration, 12 seconds`.
- Status messages that v1.2.0 put in a `role="status"` live region are posted
  as `NSAccessibility` announcements with `.medium` priority (they are
  listed in each spec's Copy section).
- Increase Contrast: `m.border` becomes 2, separators/secondary step up (pm-
  design handles values). Panels draw their edge. Selected rows add a
  `m.border` `c.separator` outline so selection is not fill-only.
- Differentiate Without Colour: every state that uses `c.danger`, `c.warning`,
  `c.success` or `c.accent` already pairs a symbol or shape; the spec says
  which.
- Reduce Transparency: glass surfaces become solid (`Tokens::material`); no
  panel content changes.

## 8. Global don'ts

- No literal colours, sizes, radii, durations or font names in view code.
- No accent fills behind rows, clips, chips or sections.
- No glass inside a panel. No glass on glass.
- No toasts for things that are visible where they happened.
- No custom-drawn copies of AppKit controls (pop-up, checkbox, push button,
  search field, segmented control in sheets/toolbars, menus) inside AppKit
  surfaces. GPUI draws the inspector/timeline equivalents only where the spec
  says, matching system metrics.
- No all-caps, no letter-spacing, no text under `t.minimum`.
- No new shortcut that shadows a w1-keymap binding.

## Owner decisions (Chike, 2026-11-01)

1. Timeline layer-list default width is **266 pt** (as v1.2.0), so Position and Scale values show at the default width. `metrics.layer_column` = 266.
2. The agent opens as a **popover** under its toolbar button and can be **pinned** into the dock (as specified in agent-panel.md).
3. "Effects & Presets" is renamed **Effects** until a presets library exists. Specs, menus, palette rows and VoiceOver labels use "Effects".
4. The "Code (web animation)" export option is **removed**, not hidden (2026-10-29 decision).

