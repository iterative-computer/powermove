# Effects

Panel id `fxbrowser`, title **Effects** (owner decision 3 renamed "Effects & Presets" until a presets library exists), bottom of the right dock
under Properties (default height `pm_design::workspace::DEFAULT_EFFECTS`).
Shared rules: [README.md](README.md).

Behaviour source: `v1.2.0:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte`
(categories `All` / `Recent` / groups, search, apply by double-click / Return
/ `+`, drag payload `{kind: 'effect', id, label}`, recents). Workflow W064
(B413, B414) and the local route *Effects browser* in w1-keymap.

v1.2.0 lists effects only. **Presets** (animation/style presets from v4
built-ins) appear in this panel when the model supplies them, as their own
groups, with identical row behaviour. Until then the panel shows effects.

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `scope` | `All` · `Recent` · `Group(name)` |
| `query` | `String` (empty = browsing) |
| `items` | `Item{id, label, group, kind: Effect · Preset, installed: bool}` |
| `groups` | ordered names + `collapsed: Set` (persisted per app, not per project) |
| `recent` | ≤ 8 ids, most recent first (v1.2.0 `readRecent`) |
| `selected` | `Option<item id>` (single) |
| `can_apply` | bool — a layer that takes effects is selected |

## 2. Anatomy

```
┌ Effects ─────────────────────────────────────────┐  header (no buttons)
│ [⌕ Search effects              ] [All ⌄]         │  search row m.row
│ ⌄ Recent                                          │  group row (only under All, when recents exist)
│     ✦ Glow                                  [+]  │  item row m.row, + on hover
│ ⌄ Blur & Sharpen                                  │
│     ✦ Gaussian Blur                               │
│     ✦ Directional Blur  (selected)                │
│ › Stylize                                         │
└───────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Header | README §2, title only. No `+` (adding is the row's job). |
| Search row | height `m.row`, inset `m.inset`. `NSSearchField` (system small size), placeholder `Search effects`, fills the row minus the scope pop-up and `m.gap`. Always visible (v1.2.0 toggled it; HIG search fields stay put). |
| Scope pop-up | GPUI pop-up well (inspector Choice recipe) **borderless**: `t.body` `c.secondary` title + chevron; `NSMenu`: `All` ✓, `Recent` (only when recents exist), separator, every group name. Width fits the longest group name, max `m.value_column`. Tooltip `Show Category`. |
| Group row (scope All) | outline row, `m.row`, disclosure per README §5.4 at depth 0, name `t.body` `t.semibold_weight` `c.text`, no count. Click toggles. |
| Item row | depth 1 (indent `m.tree_indent`), symbol at `m.symbol_inline` `c.muted` (effect `wand.and.rays`, preset `sparkles`), `m.gap`, label `t.body` `c.text` truncating with tooltip. Trailing **Add** button `plus` `m.symbol_inline` `c.secondary` in an `m.hit` square at `m.inset`, shown on hover/selection/focus; tooltip `Add {label}`; disabled (`c.muted`) when `can_apply` is false, tooltip `Select a layer first`. |
| Scope = a group or Recent | flat list, depth 0, no group rows. |
| Searching | flat list of matches across all groups (label or id contains the query, case-insensitive), each row with the group name trailing in `t.minimum` `c.muted` before the Add button. Matched substring in the label is `t.semibold_weight`. |
| Selected row | README selected (neutral fill). |

## 3. States

| State | Visual |
|---|---|
| **Empty (nothing installed)** | README empty state: symbol `wand.and.rays`, `No Effects`, `Installed effects and presets appear here.` (shell copy, kept). |
| **No search results** | centred `t.reading` `c.secondary`: `No effects match “{query}”`. |
| **Empty scope** (a group with no items) | `Nothing here yet` (v1.2.0) `t.reading` `c.secondary`, centred. |
| **Loading** | N/A (registry is synchronous); extension effects registering later simply appear. |
| **Hover** | row `c.hover`; Add appears. |
| **Focus** | list is one Tab stop; focus ring on the focused row. |
| **Selected** | neutral fill; selection is browsing-only (does not apply). |
| **Disabled** | Add disabled without a target layer (above). Rows remain draggable. |
| **Error** | apply rejected: Error line under the search row (`c.warning` triangle, `t.body` `c.warning`) `Couldn’t add {label}. Try again.`, also announced; focus stays; clears on the next apply/query/scope change. Search failed: previous results stay, same line `Couldn’t search effects. Try again.`. Settings persistence failure: no UI (diagnostic only). An item whose definition failed to load: label `c.muted`, trailing `exclamationmark.triangle.fill` `c.warning`, tooltip `This effect couldn’t load.`; not draggable or addable. |
| **Drag source** | dragging an item: system drag image = a capsule (`c.floating`, `m.control_radius`) with symbol + label; payload `{kind: 'effect', id, label}`; valid targets highlight per their specs (timeline row, viewer layer, Effect Controls insertion line, Properties panel). |
| **Drop** | N/A (panel accepts no drops). |
| **Multi-select** | N/A (single selection, v1.2.0). |
| **60 k rows** | virtualised; search off-main-thread above 2 000 items (README §4). |

## 4. Interactions

| Gesture | Result |
|---|---|
| Click row | select |
| Double-click row / Return / click Add | add to the selected layer (`Add {label}`), push to recents; announce `Added {label}` — or `Select a layer first` when none |
| Drag row | drop onto a layer (timeline row, viewer, Effect Controls) |
| Click group row | collapse/expand |
| Type in search | filter live |
| Esc in search | clear the query; second Esc moves focus to the list (v1.2.0 Escape clears/closes search) |
| Right-click item | `Add to Selected Layer` (disabled without target), separator, `Show in Effect Controls` (only if the selected layer already has it) |

## 5. Keyboard and focus

Tab order: search field → scope pop-up → list. In the list (w1-keymap
*Effects browser*): ↑/↓ move, Home/End first/last, → expands a group, ←
collapses or moves to the parent group, Return adds, Esc returns focus to
search. ↓ in the search field moves to the first result. Typing a printable
character while the list is focused moves focus to the search field and
inserts it.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Search | search field | `Search effects`; results count announced on change (`12 effects`) |
| Scope | pop-up button | `Category, All` |
| List | outline `Effects` (`AXRowCount`) | group: `Blur & Sharpen, 3 effects, expanded`; item: `Gaussian Blur, effect, Blur & Sharpen` / preset `{label}, preset, {group}`; actions `Add to selected layer` |
| Add | button | `Add Gaussian Blur` |

## 7. Appearance

Tokens only; solid panel. Increase Contrast: group rows get a hairline above
(`m.border` `c.separator`) except the first.

## 8. Copy

| Where | String |
|---|---|
| Placeholder | `Search effects` |
| Scope | `All`, `Recent`, group names as registered (Title Case, e.g. `Blur & Sharpen`); tooltip `Show Category` |
| Empty | `No Effects` · `Installed effects and presets appear here.`; `No effects match “{query}”`; `Nothing here yet` |
| Tooltips | `Add {label}`, `Select a layer first`, `This effect couldn’t load.`, row: `Drag onto a layer, or click Add.` |
| Announcements | `Added {label}`, `Select a layer first`, `{n} effects` |

## 9. Don't

- Don't use a category rail beside the list (v1.2.0) — 300 pt is too narrow;
  the scope pop-up keeps the same behaviour.
- Don't show the `fx` badge or the `fxbrowser` id anywhere.
- Don't add on single click.
- Don't fill the selected row with accent.
