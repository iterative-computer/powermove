# Design review: `feat/na-cloud-s-library`

Reviewer: design lead · 2026-11-01 · Lane head `601f69441`, read against [surfaces/library.md](../surfaces/library.md) and README §1–4
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 76 pass.

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-library` (`bd68e20b3`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S017 | Window, sidebar, toolbar, grid/list, context menus, states and VoiceOver copy exact; no pagination | APPROVE |
| 2 | S018 | Presets, ranges and error copy exact; Create disabled while admitted | APPROVE |
| 3 | S018 a11y | Field AX names (`Project name`, `Frame rate`, `Duration in seconds`, `Background colour`…) differed from the visible labels | **Fixed** — `Name`, `Resolution`, `Width`, `Height`, `Frame Rate`, `Duration`, `Background` |
| 4 | S005 | Delete Immediately added `Delete` first (default slot) | **Fixed** — `Cancel` (default) · `Delete`, response index updated |
| 5 | README §2 | Stale banner `Project was changed, …` | **Fixed** — `The project was changed, so the dialog closed.` (noun table added to README §2) |
| 6 | S019 | Rename in place, Duplicate `{name} copy` + rename | APPROVE |

## DESIGN-TODO(opus) answers

All seven answered in library.md *Answers*: tokens now exist (`thumbnail_min`, `sheet_inset`, `sheet_value_column`, `colors.media_badge`); size link = inspector *Link* toggle, starts on; empty-state symbols `square.grid.2x2` / `magnifyingglass` / `trash`, Loading AX `Loading Projects`; while creating both buttons disabled, `Creating…`; drop-frame `;` only at 29.97/59.94 else `Use colons for timecode at this frame rate.`; sort directions longest/largest/highest first; omit unknown facts; Duplicate clears a non-matching search first.

## Sol follow-ups

1. Implement the *Answers* list and drop the `GAPS` entries.
2. AppKit window/grid rendering with the new tokens.
