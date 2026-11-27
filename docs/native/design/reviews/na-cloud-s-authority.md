# Design review: `feat/na-cloud-s-authority`

Reviewer: design lead · 2026-11-01 · Lane head `66ca88a6e`, read against [surfaces/authority.md](../surfaces/authority.md) and README §1–4
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-native-app` 80 pass (78 app incl. 48 authority).

**Verdict: APPROVE — merge `origin/feat/na-design-fix-s-authority` (`efd72713b`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | S010 | Full Access (critical, Cancel default) and per-run alert copy exact; 3-line summary clamp; session-only standing grant | APPROVE |
| 2 | S011 | Setup rows, secret set/re-entry/saving/error states, Reveal `Done`·`Copy` (concealed type), immediate Remove, empty state copy | APPROVE (headless) |
| 3 | S012 | Delete Workspace and Delete Immediately added `Delete` first, so the destructive button sat in the default (rightmost) slot with Cancel holding Return beside it | **Fixed** — Cancel added first, Delete to its left; spec wording corrected (README §1 now says so explicitly) |
| 4 | S012 | Uninstall `Keep`/`Delete`, no-values bypass, archive-before-erase ordering | APPROVE |
| 5 | Safety | `PrivateValue` redacts Debug, never on a surface/v4 tree; `try_send` bridge, no UI-thread waits | APPROVE |

## DESIGN-TODO(opus) answers

1. **Sheet tokens** — `metrics.sheet_inset` 20 and `metrics.sheet_value_column` 240 now exist (kit2 fix). Render the sheet with them.
2. **Failure copy** — added to authority.md S011/S012: Reveal unreadable → row goes to *Needs re-entry*; other Reveal failure → `Couldn’t show {label}. {informative}`; Remove failure → `Couldn’t remove {label}. {informative}` (row stays set); post-uninstall deletion failure → alert `{extension} was removed, but its values couldn’t be deleted.` + informative, `OK`.

## Sol follow-ups

1. Implement the failure copy above (typed codes already exist).
2. Native setup sheet with the new tokens; app routing/worker binding as the lane lists (items 1–5 of its "outside this lane" list).
