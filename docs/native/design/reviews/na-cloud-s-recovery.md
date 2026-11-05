# Design review: `feat/na-cloud-s-recovery`

Reviewer: design lead · 2026-11-01 · Lane head `d9a9dd0aa`, read against [surfaces/recovery.md](../surfaces/recovery.md) and README §1–4
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 65 pass; `bun test surfaces/recovery` 6 pass.

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-recovery` (`3293acd0a`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S001 | Reload alert, recovered banners, Ask alert (Esc → Open as Copy, Discard omitted when it can't archive), Unrecoverable alert — copy exact, never "crash" | APPROVE |
| 2 | S013 | Panel failure table exact, one AXGroup `{Title}. {Line}`, `.high` announcement, not-responding alert | APPROVE |
| 3 | S013 | `WaitingToRestart` with no Failure rendered a placeholder | **Fixed** — same as after `Crash` (spec updated) |
| 4 | S053 | Error Log window, columns, Details mono, Copy/Clear Log, empty state | APPROVE |
| 5 | S054 | Status items; indicator AX said `1 issues` | **Fixed** — `Editor slowed down, 1 issue` |

## DESIGN-TODO(opus) answers

1. **Esc on the reload alert** — none (spec updated); neither button leaves the failed window as it is.
2. **WaitingToRestart without Failure** — item 3.
3. **SF Symbol primitive** — kit2 adds the `EmptyState` symbol slot; use it once kit2 merges (`exclamationmark.triangle`, `c.warning`).

## Sol follow-ups

1. After kit2: bind the EmptyState symbol slot and remove the `requiredSymbol` placeholder.
