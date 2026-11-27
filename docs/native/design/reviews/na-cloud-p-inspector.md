# Design review: `feat/na-cloud-p-inspector`

Reviewer: design lead · 2026-11-01 · Lane head `e5d7bb0d1` (diff vs
`origin/feat/native-engine`). Read against [panels/inspector.md](../panels/inspector.md),
[panels/README.md](../panels/README.md), [ui-kit.md](../../ui-kit.md),
[review-checklist.md](../review-checklist.md) and
`panels/inspector/test/__snapshots__/panel.test.tsx.snap`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-inspector` (`ef817d98d`), not the lane head.**
(Updated 2026-11-01, second pass: F0 done by the reviewer. The snapshot is
regenerated; 43/43 inspector Bun tests and the `pm-panel-inspector` Rust tests
pass on macOS.) The fix branch is the lane plus the **Fixed** items, merged cleanly
with `origin/feat/native-engine`. It passes TypeScript and Prettier.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §2 header | Title `Properties`; Add Effect only for layers that take effects; New Layer menu items exact | APPROVE |
| 2 | §2 header | Buttons sat right after the title | **Fixed**: header `space-between`, buttons grouped trailing |
| 3 | §2 identity line | `{name} · {type}` / `{n} Layers · Mixed|{type}`; AX `Headline, text layer` / `3 layers selected` | APPROVE |
| 4 | §3 locked | The `lock` glyph after the type was missing (tooltip only on rows) | **Fixed**: `lock` `m.symbol_inline` `c.muted` + `Locked layers can’t be edited.` |
| 5 | §3 empty | `No Composition` / `Open a project to start editing.` had no symbol | **Fixed**: `sidebar.trailing` |
| 6 | §3 no selection | Composition section + hint `Select a layer to edit its properties.`, not an empty state | APPROVE |
| 7 | §2 align strip | Two groups, roving Tab stop with ←/→, tooltips exact, hidden for audio/no selection | APPROVE (detail line per D1) |
| 8 | §2 rows | One value column via `InspectorRow`; diamond leading, link trailing outside the column | APPROVE |
| 9 | §2 colour | The hex field reused the property name, so VoiceOver heard `Color` twice | **Fixed**: `{Property} Hex` |
| 10 | §2 point pad | The pad button reused the property name | **Fixed**: `{Property} Pad` |
| 11 | §3 expression | `c.accent_text` value; expression row; `Expression is valid` / error tooltip + `t.minimum` `c.danger` line | APPROVE (multi-line F2) |
| 12 | §3 errors | `Enter a number.` / `Enter a value from {min} to {max}.` under the row | APPROVE |
| 13 | §4 menu | Row context menu items and order exact, `Link Scale Axes` checked | APPROVE |
| 14 | §3 drop | Whole content area `c.accent_wash` while a valid effect drags over a layer that takes effects | APPROVE (stroke `m.drop_stroke`, F3) |
| 15 | §2 section heading | `DisclosureGroup` default; chevron is not trailing / hover-only; section `+` sits inside the content | CHANGES REQUIRED (kit F1) |
| 16 | §6 VoiceOver | Diamond `Keyframe, {Property}` + three-state value; link `Lock aspect ratio`; two-axis labels `Position X` | APPROVE (mixed value F4) |
| 17 | Checklist §1, §7 | Tokens only; copy verbatim | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

- **D1: multi-layer alignment target.** v1.2.0 wins on behaviour (README
  precedence). With several layers, the union box aligns to the
  **composition** and relative offsets are kept, as in `AlignmentStrip.svelte`
  and the kernel. The tooltip detail line reads `To composition` in every
  case. The spec's `To selection` is withdrawn. Align-to-selection may come
  back later as an explicit menu option. It is not a silent behaviour change.
- **D2: failure copy.**
  - **Edit rejected** (actor/binding commit failed): the value reverts to the
    last acknowledged one. A `t.minimum` `c.danger` line under that row reads
    `Couldn’t change {Property}. Try again.` and clears on that row's next
    accepted edit.
  - **Subscription lost**: keep the last values. Disable every well. Show one
    Error line at the top of the content: `exclamationmark.triangle.fill`
    `c.warning`, `t.body` `c.warning`, text
    `Properties aren’t updating right now.` Announce it once. The host
    reconnects on its own, and the line goes away when publications resume.
  - **Native menu failed to open**: no UI. The button stays usable. Report to
    the diagnostic callback.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F0 | — | **Done** (`ef817d98d`). |
| F1 | kit | Section heading recipe: trailing chevron (`chevron.down`/`chevron.right`, `m.symbol_inline` `c.muted`, visible on hover/focus/collapsed), optional trailing heading button before it (`Add Mask`, `Add Text Animator`), whole heading clickable; IC hairline above. |
| F2 | kit | Multi-line `TextArea` (2–6 lines) and monospaced expression field (1–6 lines) with validity glyph slot. |
| F3 | kit + pm-design | Symbol slot on `ToolbarItem`/text (diamond states, align glyphs, `link`/`link.slash`, `lock`); `metrics.drop_stroke`; token arithmetic for the header optical inset and the Scale link overlap. |
| F4 | kit | Mixed number AX value `mixed`; colour well value `F5F3EF hex` (+ `, 80 percent opacity`); split swatch for mixed colours. |
| F5 | inspector lane | D1 tooltip detail `To composition`; D2 error lines. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-inspector | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-inspector` `ef817d98d`; F1–F5 open |
