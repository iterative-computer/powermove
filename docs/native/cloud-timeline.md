# Cloud timeline integration

## 2026-11-05 — native action admission

Reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d` (main).
Base: `5bcd425dd` (native-engine). The read-only `feat/na-cloud-tl-render`
head is already an ancestor of this base; its renderer is retained.

Decisions:

- N3/N4: join Position only when `separateDimensions` is explicitly false.
  Presentation groups channels without changing stored keys, values, expressions,
  unknown fields or exact legacy subframe times. Group selection expands only
  keys at exactly equal stored times. Axis diamonds still address one channel.
- N3/N7/N8: admit semantic gesture/action intents through the existing v4 worker
  dispatch and real kernel actor. Use replacement gesture steps and actor-owned
  commit/cancel/history. Native geometry supplies seconds/values and stable ids;
  JS never owns document state or converts a coordinate into an authored value.
- N3/N7: decimal document/revision stamps may fence native driver actions. Reject
  malformed ids, nonfinite numbers and missing required inputs before a lease.
- N1/N3: preserve errors and remaining platform boundaries. Headless actor tests
  do not qualify native hit testing, screen output, menu/field presentation,
  transport delivery or Opus review. No coverage row is fully closed by adding
  an action endpoint.

New native action types are declared beside the Timeline panel contract and
implemented in `pm-panel-timeline/src/v4_actions.rs`. They reuse the existing
commands and drag implementation. No extension contract or frozen MCP tool
catalog changes, and no consumer is pruned.

Verification and application-host boundaries are appended after integration.

## 2026-11-05 — scoped pro gestures and session admission

The native action route also admits linked/scoped move, trim, slip, ripple trim,
ripple delete, roll and slide. Continuous pro edits use the same actor gesture
lease and replacement steps; pro planners validate the supplied tracks, source
bounds, lock state and caller origin before a lease opens. Relative pro edits
refuse a concurrently changed revision. A gesture from a closed/reopened document
cannot write a new document reusing its layer ids. Disposal retires the stale
local gesture without touching the replacement document.

N1/N3 selects the existing pro planners and their exact validation, not a second
approximation of edit semantics. N4 selects selective cancellation and retention
of opaque source/key data. N7 selects stable ids and inspectable decimal stamps.
Native numeric key editing follows main's layer-local frame snapping (including
subframe layer origins), composition bounds and neighbour collision guard.
Simplification is an explicit action, never a drag-finishing side effect.

Linux actor regressions cover every pro-edit dispatch variant, replacement ripple
steps, cancellation, locked downstream refusal, joined/per-axis diamonds, exact
subframe membership, graph/handle/property/work-area gestures, stale documents,
numeric collision/validation, opaque key preservation and document Undo.

## 2026-11-05 — production worker and Easing storage

`Builtins::timeline_action` queues the existing bundled v4 worker, and
`timeline_key` now uses the same grouped-key admission. Disposal awaits actual
preview cancellation. Native effects receive nonzero request ids and the host
ack remains pending until native replies arrive; unavailable effects reject.
Main only deposits a receipt into the reply mailbox. The panel worker consumes
it and finishes its own action (N3/N8). The Mac host must still substantiate
actual transport completion rather than only acknowledging command enqueueing.

The production presenter retains every negative/subframe/coincident key identity
rather than dropping off-composition keys or deduplicating against an invented
2048-point grid (N3/N7). Joined readouts evaluate each actual axis; navigation
compares composition times. Outline, native scene and evaluated values share
one immutable publication, even if the actor advances during presentation.

The existing `pm-services` Easing library now has a worker adapter for explicit
import, save, duplicate, rename, colour, group creation, move and archive. Export
is exact source inspection. Storage is the admitted `ext.builtin-easing`
extension namespace, containing the original `pm.easing.library.v1` member and
an archive of every prior value. Unknown fields are retained. Archive and
publication use one revision-checked kernel state transaction (N3/N4/N6).
Missing, malformed or future-version source is never replaced with defaults.
The original localStorage source still needs the migration owner's import;
this adapter does not claim to discover it or provide a file chooser.

No visual tokens, layout or product copy are introduced. No new
`DESIGN-TODO(opus)` placeholder is introduced. All rendered output still awaits
Mac captures and Opus review.

## 2026-11-05 — final integration evidence and blocked handoff

Implementation commits: `5fbeeb5ff` (native actions and joined Position),
`dfb9ee065` (scoped pro gestures/session guards), `21c7536a5` (production worker,
coherent projection, effect receipts and Easing service storage).

Fetched and merged `origin/feat/native-engine` at
`5bcd425ddc158cbd891ee4b515cf9993fad29a66`: already up to date, no conflicts.
Read-only render input `ce10963fae2eb4467492e2083b9aa15e29d9ddf6` is an ancestor
of that base; its implementation and snapshot fixtures remain in place.
No dependency changes were needed and no lockfile changed. Every Cargo test ran
with `--locked`. No secrets/user projects, generated captures or private plan
files were used or committed.

Post-merge commands, from the repository root after activation:

| Command | Result |
| --- | --- |
| `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-panel-timeline -p pm-builtins` | 193 passed: Timeline 114 behavior + 10 render + 36 v4; built-ins 33. 0 failed, 0 ignored. |
| `cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-panel-timeline -p pm-builtins --all-targets -- -D warnings` | Passed. |
| `cargo fmt --manifest-path apps/native/Cargo.toml -p pm-panel-timeline -p pm-builtins -- --check` | Passed. |
| `cd apps/native/js/v4 && bun run typecheck` | Passed. |
| `cd apps/native/js/v4 && bun test panels/timeline` | 54 passed, 0 failed; 18 snapshots. Uses locked Cargo example builds. |
| `cd apps/native/js/v4 && bun node_modules/prettier/bin/prettier.cjs --check panels/timeline/actions.ts panels/timeline/index.ts panels/timeline/model.ts` | Passed. |

Gitleaks staged scans and the configured repository hook run before every
commit, including the final signal. The entire cross-app acceptance suite was
not run and its aggregate is not changed by these targeted results.

**Status: BLOCKED for the complete requested lane.** No U04/U05/U06/U07/U12/U13
row is declared closed. Passing worker/kernel tests do not imply complete
on-screen timeline operation. Remaining work is concrete:

| Boundary / row | Required work outside the completed integration |
| --- | --- |
| U04/U05 and HANDOFF 7/8 | `apps/native/crates/app/src/v4_view.rs` needs native track/graph hit tests, pointer capture, autoscroll, drag/trim/pickwhip/marquee/resize/pan/zoom routing into `Builtins::timeline_action`. Generic raw `pointer*`, wheel and drop intents still reject where native geometry is absent. Wire the reconciled renderer to the actual viewport rather than the existing app's separate proportional track drawing. This file is outside this lane's ownership. |
| U04/U05/U07 | App native menus, rename/property/key-time fields, per-axis affordances, columns and section headings need their specified bindings/presentation. `selectProperty`, `editKeys`, parent changes, diamonds and dimension joining are real actor routes, but those endpoints alone do not qualify the controls. Prove save/reopen with real project storage. |
| U06 | Capture Timeline and Graph selected/unselected/locked states in light, dark and Increase Contrast on macOS, then obtain Opus review. No visual acceptance is claimed. |
| U12 | Register/present S094/S095 Easing in the application and feed migrated `pm.easing.library.v1` source into the explicit import adapter. Native file/folder import/export, the full local curve editor, multikey/vector section application, graph fit/merge, Clean Motion Path and Create Position Controller remain incomplete. The existing worker explicitly rejects unavailable effects. App entry/shell registration and migration changes are outside the owned paths. |
| U13 timeline captions | `apps/native/crates/kernel/src/commands/captions.rs` still returns `NotImplemented` for add/edit. The existing Timeline regression preserves this as a rejection. Cue move/trim/split/delete cannot be claimed working until that owning crate supplies the body. |
| Host and macOS | `apps/native/crates/app/src/builtins.rs` must return actual transport/service completion or rejection, rather than only command enqueueing. Verify AppKit/GPUI focus arbitration, pointer cancellation across window lifecycle, IME/AX/VoiceOver, menus, fields and native presentation. Linux cannot certify any of these. |

The requested changes in the app, kernel and migration areas are intentionally
not made under this lane's path ownership. No existing consumer is removed.
No new design placeholder or design decision was introduced.
