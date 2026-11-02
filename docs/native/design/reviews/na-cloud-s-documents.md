# Design review: `feat/na-cloud-s-documents`

Reviewer: design lead · 2026-11-01 · Lane head `17e98b0e9`, read against [surfaces/documents.md](../surfaces/documents.md) and README §1–4
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 82 pass; `bun test surfaces/documents` 7 pass.

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-documents` (`8fb0920fe`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S002 | Close/quit alerts, `Don’t Save` ⌘D leftmost, review walk, Finishing Up sheet after 1 s, save-failed alert | APPROVE |
| 2 | S003/S004 | Changed-on-disk, recovery copy banner/notice, uncertain save, open errors, all eight copy reasons, `{name} copy` Save As | APPROVE |
| 3 | S014 | Two failed restores read `“a”, “b” and 0 more…` | **Fixed** — `“a” and “b” may have been moved or deleted.` |
| 4 | S016/S020/S055 | Popout window, pickers, opening phases + announcements | APPROVE |
| 5 | Test | JS replay test ran `ui_kit_wire` from the default target dir | **Fixed** — honours `CARGO_TARGET_DIR` |

## DESIGN-TODO(opus) answers

1. **Notice symbol** — README §3 now specifies it by severity: info `info.circle` `c.secondary`, warning `exclamationmark.triangle.fill` `c.warning`, error `xmark.octagon.fill` `c.danger`, `m.symbol_inline`.
2. **Sheet tokens** — exist now (kit2 fix).

## Sol follow-ups

1. After kit2: replace the hand-built notice with the kit `Banner` and the symbols above.
