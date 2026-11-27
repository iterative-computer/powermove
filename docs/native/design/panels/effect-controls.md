# Effect Controls

Panel id `layer-effects`, title **Effect Controls**, left dock, tabbed with
**Project** (`Project | Effect Controls` segmented tab group, README §2).
Shared rules: [README.md](README.md). Row grammar:
[inspector.md](inspector.md) (identical rows, diamonds, wells, popovers).

Behaviour source: `v1.2.0:apps/desktop/src/extensions/inspector/EffectsPanel.svelte`,
`EffectsSection.svelte`, `ParameterList.svelte`, `parameter-layout.ts`,
`effect-clipboard.ts`, `actions.ts` (`showFxMenu`); main's later
"Add Effect button" commit (`0e1af9e60`). Workflow W064 (B285–B288, B338,
B339, B367–B380).

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `EffectsTarget` | `NoProject` · `NoSelection` · `Layer{id}` · `Unsupported{id}` (layer type takes no effects) |
| `effects` | ordered `Effect{id, definition, label, enabled, expanded, params: [Row], missing: bool}` (Row = inspector Row) |
| `effect_selection` | `Set<effect id>` + anchor; its own clipboard context (B285) |
| `clipboard` | effect payloads; survives selecting another layer (B372) |
| `drag` | `None` · `Reorder{ids, insert_index}` · `DropNew{definition, insert_index}` |

## 2. Anatomy

```
┌ [Project | Effect Controls] ─────────────────── [+] ┐  header
│ Headline · Text                                      │  identity line
│ ⌄ ☑ Glow ................................... ↺  ×   │  effect header row (m.row)
│   ◈ Radius ............... [24            px]        │  parameter rows (inspector grammar,
│   ◆ Intensity ............ [0.65            ]        │   indented one m.tree_indent)
│   ◇ Color ................ [■][#FF7A3C      ]        │
│ ⌄ ☑ Film Grain ............................. ↺  ×   │
│ › ☐ Gaussian Blur .......................... ↺  ×   │  disabled effect: name c.muted
│                                                      │
│               [ Add Effect… ]                        │  trailing add button
└──────────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Header | README §2; tab group `Project` · `Effect Controls`. Trailing button **Add Effect** (`plus`, tooltip `Add Effect`) → effect menu (§4). Hidden for `NoSelection`/`Unsupported`. |
| Identity line | as inspector: `{layer name}` `t.title` semibold + ` · {type}` `c.muted`. |
| Effect header row | height `m.row`, full width; selected → `c.selected_fill` (README selected-row shape). Leading: disclosure (`chevron.right`, rotates open) in an `m.hit` square at `m.inset − (m.hit − m.symbol_inline) ÷ 2`; enable **checkbox** (system, small hit padded to `m.hit`); name `t.title` `t.semibold_weight` `c.text` (disabled effect → `c.muted`, and the checkbox unchecked — two cues). Trailing, right edge at `m.inset`: **Reset** (`arrow.counterclockwise`, tooltip `Reset Effect`) and **Remove** (`xmark`, tooltip `Remove Effect`), both `m.symbol_inline` `c.secondary` in `m.hit` squares, shown on row hover, on selection and on keyboard focus. |
| Missing effect definition | name followed by `exclamationmark.triangle.fill` `c.warning` and `t.minimum` `c.warning` text `Not installed` on the row; parameters show raw values read-only (`F` double-tap in the timeline reveals these, w1-keymap). |
| Parameter rows | inspector rows; the diamond column is indented by `m.tree_indent` (labels shift with it). **The value column does not move** — it is the same panel-wide column as every other row. |
| Parameter group (main E04) | inspector section disclosure inside the effect, one `m.tree_indent` in; state kept per effect. Rows inside keep the panel's value column. |
| Action row (`Swap Colors`, main E04) | push button (`m.field` tall) at the leading edge of the value column, label column empty, one undo named for the action. |
| Between effects | no separators; `m.gap` extra space above each effect header after the first. |
| Add Effect button (trailing) | system push button `Add Effect…` (regular, `m.control`), centred, `m.inset` below the last effect. Opens the effect menu. Always present when the layer takes effects. |
| Drop insertion line | README drop-target insertion line between effect headers. |

## 3. States

| State | Visual |
|---|---|
| **Empty (no project)** | README empty state: symbol `wand.and.rays`, `No Composition`, `Open a project to start editing.` |
| **No selection** | Empty state: symbol `wand.and.rays`, `No Selection`, `Select a layer to see its effects.` (v1.2.0 copy). No button. |
| **Layer has no effects** | Empty state inside the content (below the identity line): symbol `wand.and.rays`, `No Effects`, `Drag an effect here from Effects, or add one.`, button `Add Effect…`. |
| **Unsupported layer** (audio, camera) | Empty state: symbol `wand.and.rays`, `No Effects`, `This kind of layer can’t have effects.` |
| **Loading** | N/A (synchronous). |
| **Hover** | effect header `c.hover`; trailing Reset/Remove appear. |
| **Focus** | effect list is one Tab stop; focused header row gets the README list focus ring; parameter controls are separate stops in order. |
| **Selected** | header row `c.selected_fill`; multi-select by ⌘-click / ⇧-click (B285). |
| **Disabled effect** | checkbox off, name `c.muted`; parameter rows remain editable (v1.2.0) with labels `c.muted`. |
| **Collapsed** | only the header row; disclosure `chevron.right`. Expanded state persists per effect. Selecting a row never changes disclosure; disclosure only via the chevron (B379). |
| **Locked layer** | everything `c.muted`, non-interactive; identity line shows `lock`. |
| **Error** | missing definition (above). Rejected edit / registry unavailable: one Error line under the identity line (`c.warning` triangle, `t.body` `c.warning`): `Couldn’t change the effects. Try again.` / `Effect controls aren’t available right now.`; last accepted content stays; clears on the next accepted publication. Values with no registry control: read-only text (`c.secondary`, `On`/`Off` for booleans) or `Custom value` (`c.muted`), AX `{label}, {value}, read only`. Paste with an empty clipboard: the menu item is disabled with the title `Paste Effect` and tooltip `Copy an effect first`. |
| **Drag reorder** | dragged headers at 50 %; insertion line; Esc cancels; drop = one undo `Reorder Effect`. |
| **Drop new effect** (from Effects or the effect menu drag) | insertion line at the pointer; drop over a header inserts before it; drop below the last effect appends. |
| **Multi-select** | headers selected; ⌘C/⌘V/⌫ act on all (B371). |
| **60 k rows** | N/A in practice; the list is still virtualised by header + expanded params (README §4). |

## 4. Interactions

| Gesture | Result |
|---|---|
| Click header | select effect (⌘ toggle, ⇧ range); clicking anywhere outside the list clears effect selection, but not from nested controls (B380) |
| Click chevron | expand/collapse |
| Click checkbox | `Toggle Effect` (undo `Enable Effect` / `Disable Effect`) |
| Click Reset | reset all parameters to definition defaults (B367), undo `Reset Effect` |
| Click Remove | remove; undo `Remove Effect` |
| Drag header | reorder (B370) |
| Add Effect (header `+` or trailing button) | `NSMenu` of effect groups as submenus (`Blur & Sharpen ▸ Gaussian Blur`, …) with a `Recent` submenu at top when there are recents; choosing adds to the end and expands it |
| Right-click header | `Copy`, `Paste` (`Paste Effect` / `Paste Effects`), separator, `Move Up`, `Move Down`, separator, `Reset Effect`, `Remove Effect` (`Remove Effects` for several) |
| Parameter rows | exactly inspector.md §4 |

## 5. Keyboard and focus

Tab order (entering the list with no focused effect puts the focus ring on the first header without selecting it; the first ↑/↓ selects the focused row; with a selection, focus lands on the anchor): tab group → Add Effect (`+`) → effect list (one stop) → for each
expanded effect, its parameter controls in row order → trailing `Add
Effect…` button.

In the focused effect list (w1-keymap *Effect rows*, handled **before**
layer shortcuts): ↑/↓ move selection (⇧ extends), ⌥↑/⌥↓ reorder selected
effects, ⌘C/⌘V copy/paste effects, ⌫/⌦ remove, → expand, ← collapse, Space
toggles enable, Return moves focus to the first parameter control of the
focused effect. Esc clears effect selection and returns focus to the list.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Effect list | list `Effects`, multiselectable | rows: `Glow effect, enabled, expanded, 2 of 3`; actions `Press` (select), `Expand`/`Collapse`, `Delete` (remove), custom `Move Up`, `Move Down`, `Reset` |
| Checkbox | checkbox | `Enable Glow` |
| Reset / Remove | buttons | `Reset Glow`, `Remove Glow` |
| Parameters | as inspector, label prefixed by nothing (the effect is the containing group `Glow`) | |

Announcements: `Added {effect}`, `Removed {effect}`, `Removed {n} effects`,
`Moved {effect} up`, `Moved {effect} down`, `Moved {n} effects up`,
`Moved {n} effects down`, `Pasted {effect}`, `Pasted {n} effects`.
Disclosure button: `Expand {effect}` / `Collapse {effect}`.

## 7. Appearance

Tokens only. Increase Contrast: a hairline (`m.border` `c.separator`) above
each effect header after the first. Reduce Motion: no expand animation.

## 8. Copy

| Where | String |
|---|---|
| Empty states | `No Selection` / `Select a layer to see its effects.` · `No Effects` / `Drag an effect here from Effects, or add one.` / `Add Effect…` · `No Effects` / `This kind of layer can’t have effects.` |
| Buttons and tooltips | `Add Effect`, `Add Effect…`, `Reset Effect`, `Remove Effect`, `Copy an effect first` |
| Missing definition | `Not installed` |
| Undo names | `Add {Effect}`, `Enable Effect`, `Disable Effect`, `Reset Effect`, `Remove Effect`, `Remove Effects`, `Paste Effect`, `Paste Effects`, `Reorder Effect` |

## 9. Don't

- Don't nest an inner value column for effect parameters; the panel has one.
- Don't use a switch for enable; don't hide the enable state behind hover.
- Don't select an effect when its disclosure is clicked, or toggle
  disclosure when the row is clicked.
- Don't move the effect stack into Properties.
