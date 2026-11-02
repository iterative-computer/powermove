# Design review: `feat/na-cloud-p-effectcontrols`

Reviewer: design lead · 2026-11-01 · Lane head `3eeb5e949` (diff vs
`origin/feat/native-engine`). Read against [panels/effect-controls.md](../panels/effect-controls.md),
[panels/inspector.md](../panels/inspector.md) (row grammar),
[panels/README.md](../panels/README.md), [ui-kit.md](../../ui-kit.md),
[review-checklist.md](../review-checklist.md) and the golden trees in
`panels/effect-controls/snapshots/*.json`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-effectcontrols` (`37bbcc9f2`), not the lane head.**
(Updated 2026-11-01, second pass: F0 done by the reviewer. The goldens are
regenerated, and all 39 Effect Controls tests pass on macOS. The test model
wire blocked on the kernel from the main thread, which the kernel forbids on
macOS; it now runs on a worker thread.)
The fix branch is the lane plus the **Fixed** items, merged with
`origin/feat/native-engine`. The `Cargo.toml` workspace-members conflict is
resolved as a union (`packaging` + the harness). It passes TypeScript and
Prettier.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §2 header | `[Project \| Effect Controls]` tab group + `+` (`Add Effect`), hidden for NoSelection/Unsupported | APPROVE |
| 2 | §2 header | `+` sat right after the tab group | **Fixed**: header `space-between` |
| 3 | §2 identity line | Name `t.title` semibold + ` · {type}` muted; `lock` + `Locked layers can’t be edited.` | APPROVE |
| 4 | §2 effect header | Chevron, checkbox, name, `Not installed` and Reset/Remove were one unaligned run; Reset/Remove weren't at the trailing inset | **Fixed**: leading cluster + trailing cluster, row `space-between` |
| 5 | §2 disclosure | Chevron button's AX label was the bare effect name, the same as the row text | **Fixed**: `Expand {effect}` / `Collapse {effect}` |
| 6 | §2 parameters | One panel-wide value column (`valueColumn: "panel"`); indent carried as `metrics.tree_indent` metadata pending the token | APPROVE (indent F3) |
| 7 | §2 between effects | `m.gap` above every header after the first; Increase Contrast hairline above each | APPROVE |
| 8 | §2 trailing add | `Add Effect…` is a ToolbarItem placeholder, not centred | CHANGES REQUIRED (F1) |
| 9 | §3 empty states | Copy exact, and owner decision 3 applied (`…from Effects, or add one.`) | APPROVE |
| 10 | §3 empty symbols | Only No Composition / No Effects had symbols | **Fixed** (TODO 5): `wand.and.rays` on all four |
| 11 | §3 disabled effect | Checkbox off + name `c.muted`; parameter labels `c.muted`, still editable | APPROVE |
| 12 | §3 collapsed / B379 | Row click selects only; chevron toggles only; checkbox stops propagation | APPROVE |
| 13 | §3 locked | Everything `c.muted` and inert; list leaves the tab order | APPROVE |
| 14 | §3 drag | 50 % opacity and insertion slot carried as metadata; accent insertion line | APPROVE as interim (F2) |
| 15 | §4 diamonds | `diamond` muted / half-filled accent / filled accent; ⌥-click = remove animation | APPROVE |
| 16 | §4 raw values (TODO 4) | Values without a registry control were shown as `JSON.stringify` | **Fixed**: scalars as plain text (`On`/`Off` for booleans); structured values `Custom value` in `c.muted`; AX `{label}, {value}, read only` |
| 17 | §5 keyboard | Arrow/⌥-arrow/⌘C/⌘V/⌫/→/←/Space/Return/Esc routed before layer keys; nested fields keep their keys | APPROVE |
| 18 | §6 VoiceOver | Row label `Glow effect, enabled, expanded, 2 of 3`; `Enable Glow`, `Reset Glow`, `Remove Glow` | APPROVE (projection F4) |
| 19 | Checklist §1 | No literal colours/sizes in styles; `dragOpacity: 0.5` is metadata only (token F3) | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

- **T1: initial row.** When Tab enters the list with no focused effect, the
  focus ring goes on the **first** effect header and the selection is left
  alone. The first ↑/↓ then selects the focused row without moving. After that
  the arrows move as usual. If effects are selected, focus lands on the anchor
  row. This is Finder's list convention.
- **T2: announcements.** `Removed {n} effects`, `Moved {effect} down`,
  `Moved {n} effects up`, `Moved {n} effects down`, `Pasted {effect}` (one) /
  `Pasted {n} effects`.
- **T3: projection and admission failures.** Keep the last accepted content.
  Show one Error line under the identity line: `exclamationmark.triangle.fill`
  in `c.warning`, `t.body` `c.warning`.
  - Rejected edit: `Couldn’t change the effects. Try again.`
  - Registry unavailable: `Effect controls aren’t available right now.`
    Parameters drop to read-only raw values, as T4.

  The line clears on the next accepted publication and is announced once
  (`.medium`). Host/actor details go to the diagnostic callback, never on
  screen.
- **T4: opaque/custom widgets.** Read-only in the value column: scalar text
  `t.body` `c.secondary` (tabular when numeric), else `Custom value`
  `c.muted`. No editor, no hover, not a Tab stop. AX
  `{label}, {value}, read only`. Done on the fix branch.
- **T5: empty-state symbols.** `wand.and.rays` for every Effect Controls empty
  state. Done.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F0 | — | **Done** (`37bbcc9f2`). |
| F1 | kit | `PushButton` (shared request): trailing `Add Effect…` regular, centred, `m.inset` below the last effect. |
| F2 | kit/host | Drag source opacity token `metrics.drag_opacity` (0.5, added to README §6), insertion line `m.border × 2` + `metrics.status_dot` circle, pointer-leave events. |
| F3 | pm-design | `metrics.tree_indent` (requested), `metrics.drag_opacity`; per-kind virtual-list row sizes so the `m.gap` before headers is real geometry. |
| F4 | kit | Project `multiselectable`, `rowCount`, custom actions (Move Up/Move Down/Reset) and the three-state diamond value to AccessKit. |
| F5 | effect-controls lane | T1 initial-row rule; T2 strings; T3 error line. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-effectcontrols | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-effectcontrols` `37bbcc9f2`; F1–F5 open |
