# Design review: `feat/na-cloud-s-dialogs`

Reviewer: design lead · 2026-11-01 · Lane head `6343622c8`, read against [surfaces/dialogs.md](../surfaces/dialogs.md) and README §1–2
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 85 pass (the AppKit alert probe prints SKIP off the main thread — a known test-harness limit, not a pass).

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-dialogs` (`1c79800d9`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S021/S022 | Composition sheets, Pre-compose options/descriptions/checkboxes exact; stale target closes | APPROVE |
| 2 | S022/S043 | Delete Composition, Delete Preset and Replace Preset added the action first (default slot) | **Fixed** — `Cancel` first; Rename Preset keeps `Rename` default; spec wording corrected |
| 3 | Copy | `1 other compositions`, `1 layers extend…`, `1 keyframes` | **Fixed** — singular forms |
| 4 | S029 | Keyframe Value/Velocity/Scale copy, Mixed placeholder, errors | APPROVE |
| 5 | S036 | Expression sheet: mono editor, substitutions off, `Insert ⌄`, result line, ⌘Return/⌃Tab/Esc | APPROVE |
| 6 | README §1 | Esc on a default Cancel relies on NSAlert's title convention after the key equivalent is overwritten | Verify on macOS (follow-up 2) |

## DESIGN-TODO(opus) answers

All answered in dialogs.md *Answers*: `Leave all attributes in the layer`; Time `Enter a percentage greater than 0.` / Value `Enter a number.`; line-less diagnostic shows `{message}` alone; preview pending keeps the last line, unavailable `No value at the playhead.`; rename empty → disabled, unchanged → Cancel, duplicate → `A preset with that name already exists.`. Sheet tokens incl. `metrics.sheet_width_wide` now exist.

## Sol follow-ups

1. Implement the *Answers* list.
2. On macOS (main-thread test), confirm Esc reaches a default Cancel; if not, set `\u{1b}` on Cancel and keep Return on the default cell (authority's adapter does this).
