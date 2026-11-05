# Design review: `feat/na-cloud-s-media`

Reviewer: design lead · 2026-11-01 · Lane head `457892a7f`, read against [surfaces/media.md](../surfaces/media.md), media-browser.md and README §1–4
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 80 pass; `bun test surfaces/media` 5 pass.

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-media` (`77b83ae1b`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S006 | Mismatch alert, ≤3 difference lines, `Use Anyway`·`Cancel`·`Choose Another…`; folder relink; proxy-only status/meta/export row | APPROVE |
| 2 | S006 | Chosen-has-audio case pushed an empty sentence and blocked the alert as `NeedsDesign` | **Fixed** — `It has audio; the original didn’t.` (added to spec) |
| 3 | S007 | Consent copy, 8 names + `…and {n} more`, suppression checkbox, per-file retry fencing, `Not Now` banner once per open | APPROVE |
| 4 | S024/S038/S039/S040 | Source errors, import rows/reasons, sequence sheet, Figma alerts exact | APPROVE |
| 5 | Copy | `Couldn’t import 1 files`, `Importing 1 files`, `Downloading 1 files…`, `Relink 1 other missing files…` | **Fixed** — singular forms |

## DESIGN-TODO(opus) answers

1. **{Object}** — README §2 now lists the nouns; media sheets use `The media` (or `The project` when the project closed).
2. **Sheet tokens** — exist now (kit2 fix).
3. **Positive audio** — item 2.

## Sol follow-ups

1. Use `The media` / `The project` in the stale banner.
2. AppKit sheet geometry with the new tokens.
