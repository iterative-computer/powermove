# Design review: `feat/na-cloud-s-editing`

Reviewer: design lead · 2026-11-01 · Lane head `d8cd60b3b`, read against [surfaces/editing.md](../surfaces/editing.md), viewer-chrome.md and inspector.md §7
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 70 pass; `bun test surfaces/editing` 19 pass.

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-editing` (`eb2d2e81c`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S025 | Path/vertex/handle/rubber-band states, pen cursors, text point/paragraph/overset, vertex AX names | APPROVE (plan + adapters; drawing needs gizmo tokens, now present) |
| 2 | S034 | Section, header row, Range/Properties groups, collapsed half diamond, disabled muted header, Style Selected Text, empty line | APPROVE |
| 3 | S034 | Shuffle button had no glyph | **Fixed** — `shuffle`, tooltip/AX `Shuffle Order` (spec updated) |
| 4 | S035 | None fill, previous/new swatch, Picker Settings pull-down, gamut warning, midpoint diamond, recents | APPROVE |
| 5 | S051 | Text-service settings per field kind; Esc returns focus to the previous panel | APPROVE |

## DESIGN-TODO(opus) answers

1. **Shuffle glyph** — item 3.
2. **`m.gizmo_handle`** — exists now (kit2 fix): 7.

## Sol follow-ups

1. After kit2: draw vertices/handles with `metrics.gizmo_handle`, `metrics.tangent_end`, `metrics.gizmo_keyline` via kit `ViewerGizmo`.
