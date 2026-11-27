# Design review: `feat/na-cloud-p-effects`

Reviewer: design lead · 2026-11-01 · Lane head `5a873bed9` (diff vs
`origin/feat/native-engine`). Read against [panels/effects-presets.md](../panels/effects-presets.md),
[panels/README.md](../panels/README.md) (owner decision 3: title **Effects**),
[ui-kit.md](../../ui-kit.md), [review-checklist.md](../review-checklist.md) and
`panels/effects/__snapshots__/panel.test.tsx.snap`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-effects` (`81e2cdd38`), not the lane head.** That
branch is the lane plus the **Fixed** items, merged with
`origin/feat/native-engine`. The `Cargo.toml` members conflict is resolved as
a union. `bun test panels/effects` passes there (65/65, snapshots regenerated,
conformance executable built). TypeScript and Prettier are clean.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | Owner decision 3 | Title, group and list labels are `Effects`; nothing visible says `Effects & Presets` or `fxbrowser` (ids are internal) | APPROVE |
| 2 | §2 header | Title only, no `+` | APPROVE |
| 3 | §2 search row | Always visible; label `Search effects`; Esc/↓ routes. `TextField` stands in for `NSSearchField` and has no placeholder | APPROVE as interim (F1) |
| 4 | §2 scope pop-up | `Category, {scope}`, tooltip `Show Category`, max width `m.value_column`; `Recent` only when recents exist | APPROVE |
| 5 | §2 group rows | Semibold name, no visible count, AX `{name}, {n} effects, expanded`; IC hairline except the first | APPROVE |
| 6 | §2 item rows | Group name and Add button ran straight on from the label | **Fixed**: label leading; failure glyph, group name (searching) and Add trailing, row `space-between` |
| 7 | §2 item symbol and indent | No `wand.and.rays` / `sparkles` glyph; no `m.tree_indent` at depth 1 | CHANGES REQUIRED (F2) |
| 8 | §2 Add button | `Add {label}` / `Select a layer first`, disabled without target, shown on hover/selection/focus, single-click only | APPROVE |
| 9 | §2 search match | Matched run `t.semibold_weight` | APPROVE |
| 10 | §3 empty | `No Effects` / `Installed effects and presets appear here.` had no symbol | **Fixed**: `wand.and.rays` |
| 11 | §3 no results / empty scope | `No effects match “{query}”`, `Nothing here yet`, centred `t.reading` `c.secondary` | APPROVE |
| 12 | §3 error | Failed definition: label muted, tooltip `This effect couldn’t load.`, not draggable or addable, but no triangle | **Fixed**: trailing `exclamationmark.triangle.fill` `c.warning` |
| 13 | §3 selected | Neutral `c.selected_fill`; IC outline; browsing-only selection | APPROVE |
| 14 | §4 | Double-click / Return / Add apply; single click never applies; recents ≤ 8 | APPROVE |
| 15 | §6 VoiceOver | Presets spoke only their label | **Fixed** (TODO 1): `{label}, preset, {group}` |
| 16 | §3 empty placement | Empty and no-result states render *inside* the `Tree` (an outline containing a heading) | CHANGES REQUIRED (F3) |
| 17 | Checklist §1, §7 | Tokens only; copy verbatim with curly quotes | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

1. **Preset spoken kind.** `{label}, preset, {group}`, the same shape as
   effects. Done.
2. **Failures.**
   - **Apply rejected**: nothing visible changed where the user acted, so
     tell them in place. One Error line under the search row:
     `exclamationmark.triangle.fill` `c.warning`, `t.body` `c.warning`, text
     `Couldn’t add {label}. Try again.` The same string is announced
     (`.medium`). Focus stays on the row. The line clears on the next apply,
     query change or scope change. Recents are not updated.
   - **Search failed** (worker error/timeout): keep the previous results, and
     show the same Error line with `Couldn’t search effects. Try again.`. The
     next keystroke retries.
   - **Settings persistence** (recents, collapsed groups): no UI. The
     in-memory state stays as the user left it. Report to the host
     diagnostic callback only.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F1 | kit | `SearchField` (system small `NSSearchField`, placeholder, progress in trailing edge per README §4.5); the search field fills the row minus the pop-up and `m.gap` (needs a flex-fill recipe). |
| F2 | kit + effects lane | SF Symbol slot (`m.symbol_inline` `c.muted`: effect `wand.and.rays`, preset `sparkles`) + `m.gap`; depth-1 indent `metrics.tree_indent`; drag capsule image. |
| F3 | effects lane | Render the empty / no-results / empty-scope states as siblings of the `Tree`, centred in the content area. When nothing is installed the `Tree` is absent and the search row is disabled. |
| F4 | effects lane | Implement TODO 2 (error line, announcement, clearing rules). |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-effects | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-effects` `81e2cdd38`; F1–F4 open |
