# Main delta against the frozen native reference

## Scope and decisions — 2026-11-04

This is a source audit, not a change to the parity oracle or an implementation
plan sign-off. Frozen behavior remains **v1.2.0**
(`07ed5fa17ee640659cb075f9cad1af2095011d60`). The audited main tip is
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d` (2026-11-03).
`git log v1.2.0..origin/main` contains **138 commits**, including merge commits
and test-only changes. Native coverage is evaluated against integration commit
`ed5482622f41c533199a724dcb8c8738dd4dcc93`, with working-lane evidence identified
separately. These hashes pin the audit even if either branch advances.

Read together with [HANDOFF.md](HANDOFF.md), [p0-decisions.md](p0-decisions.md),
[C-MATRIX](c-matrix.md), [D-STATE](d-state.md), [W1](w1-workflows.md),
[keymap](w1-keymap.md), [traces](traces.md), [surfaces](surfaces.md),
[MCP](mcp.md), [engine service](engine-service.md) and
[first launch](first-launch.md). Source paths below are repository-relative;
inspect immutable evidence with `git show COMMIT:PATH` or `git show COMMIT -- PATH`.
The commit ledger accounts for every commit; expanded rows split broad commits
into independently owned user-visible changes. Merge ancestry and tests are not
counted as additional shipped features. Main source availability does not mean
a release containing it shipped.

| Decision | Selecting rule |
| --- | --- |
| Keep the v1.2.0 oracle; record subsequent behavior as additive deltas, including removed/superseded behavior | Recorded P0 reference decision; N3 exact authored intent. |
| Flag document fields, media bytes/references, saved workspaces, undo and profile formats as **HIGH** | N4 every project opens, preserve user data; N3 exact authored state. HIGH is an audit priority, not a new phase approval. |
| Count native primitives as partial coverage; require real application and physical evidence for completion | N1 pro-grade first, N2 better than shipped, N3 correctness. Linux models and documentation cannot prove Mac UI/rendering. |
| Route extension logic and synchronous services to V8/native recorder, panel presentation to remote DOM; headless/MCP to Rust | Recorded architecture; N6 v1–3 contract, N7 id-driven automation, N8 no Node/Chromium or main-thread JS waits. |
| Preserve data and old consumers even when main now cleans up or removes a feature | N4 archive, never delete; prune only after no consumer remains. Web-animation delivery has a separate recorded owner removal. |
| Record visual changes as evidence for Opus, without adopting colors, dimensions, copy or interaction decisions | User design ownership instruction; existing pm-design/specs govern native presentation. |

Coverage vocabulary: **Partial** means a named native primitive/spec exists but
complete behavior is unverified; **Gap** means the audited delta has no complete
native implementation/evidence identified; **Disposition** means a recorded scope
or design decision applies; **N/A** means no additional native runtime behavior.
No row asserts full native/macOS parity. Owner names below designate native areas,
not permission to edit their files or other branches.
