# W1 native acceptance harness

`apps/native/crates/acceptance` (`pm-acceptance`) is a Linux-runnable acceptance
runner over the real `pm-kernel` and `pm-mcp` automation core. The first measured
report below uses native-engine base `5412ab207` on 2026-11-01. It is a coverage
report, **not a native parity or cutover pass**.

## Inventory and execution

The catalog compiles the authoritative [W1 workflows](w1-workflows.md),
[keymap](w1-keymap.md) and [e2e triage](e2e-triage.csv) directly into the runner.
It preserves 83 workflow contracts, all 1,344 rebased A/B variants and their
source provenance, all 195 bindings (arguments, priority, field/repeat/loose
flags), and all 396 triage rows, including the frozen 370-spec roster. The 48
wholly post-tag appendix claims are not promoted into shipped requirements.
Existing W1 menu/local-key/property-reveal contracts remain in each workflow's
compiled contract; their interaction coverage is blocked pending the shell.

Each original A/B case has a result. A supported case executes its actual MCP
body through `McpSession` and the host's single kernel. Unimplemented variants
return `blocked-by` with a named dependency and reason. A W-group also has a
`Wnnn.contract` case retaining its complete integration gate. Passing a subset
never makes the full group pass. Unknown IDs and empty/mismatched filters fail
as configuration errors; assertion failures and panics become **fail**, never
blockers. There are no ignored/skip tests in this crate.

Seven original cases execute: A495 list projects, A496 create, A497 open, A498
start session, A501 finish commit/rollback, A517 selection/add/clear, and A531
guarded rollback preserving earlier work. Other executed probes cover library
rename/duplicate/trash/restore, PMV4/persisted history, journal recovery, atomic
batches, stale revisions, interleaved writers, locks, disconnect, composition
settings, selection, work bounds, markers, strip moves, split/source continuity,
slip handles, keys/easing, groups, Unicode/paragraph text, masks, exact f64
properties and actor Undo/Redo/source-edit cancellation. These supporting probes
are separately labelled; they do not certify the original UI/pixel assertions.
The slip fixture uses W057's exact from=2, duration=3, trim=1, rate=2, source=12
and checks valid 5+3×2 versus refused 8+3×2, unchanged animation, Undo and Redo.

The macOS-only `pm-shell` crate cannot be linked here. The harness compiles its
**existing pure `keyboard.rs` source in place**, without a copied/reimplemented
dispatcher or edits to shell ownership. Every K row is compared with the actual
shipped shell table, then run through its resolver under seven isolated routing
contexts: normal, held, extra modifier, input, contenteditable, prior decline
and key-up. These 1,365 checks prove routing admission and exact command/args;
they do not execute the command or establish AppKit focus semantics. Each binding
also has nine separate, blocked integration variants: normal, held, extra
modifier, input, contenteditable, selected read-only text, focused button, prior
extension decline and menu single execution. All 1,755 carry `needs_macos=true`.

Engine/surface doubles implement the actual MCP `EngineHook`/`SurfaceHook`
traits. They exercise revision-bound capture requests and stable-ID inspection/
drive, including stale/unknown-ID refusal. Their two passes are explicitly
`test-double-probe` with `needs_macos=true`; they produce no pretend pixels and
cannot clear rendering or real-input gates. Absent render/panel/video MCP bodies
are also checked for explicit errors with backing information.

## Fixtures and public-data safety

Each executed scenario creates its own disposable profile, exports, library,
journal and run workspace. The CLI runs on a worker, including actor waits and
Drop; it never blocks a macOS main thread. All document writes use kernel
transactions; actor probes use `ActorAutomation` snapshots and actor messages.
The runner does not discover an installed profile or launch Electron/Node.

Three public engine container fixtures are embedded at compile time:
`engine/crates/pm-model/tests/fixtures/containers/{json-plain,pmv3,pmv4}.pmv`.
The runner copies them before opening, saves to a second private destination and
reopens through the production readers. It checks exact document values
(including unknown data/lone surrogates), opaque envelope fields, workspace,
imported history/cursor or retained opaque history, media IDs/revisions/types
and every embedded media byte. Missing workspace and explicit null both mean
no authored workspace; a populated workspace must remain exact. Media copying
and byte comparison stream with bounded buffers. Any omitted corpus media,
including an unreachable source record, is a failure requiring review.

`--corpus /absolute/public-synthetic-corpus` adds sorted immediate `.pmv`/`.json`
files. No recursion or implicit corpus discovery occurs. Each is copied with
`create_new` before opening; file symlinks are refused. Source files are never
saved, locked, renamed or deleted. Reports use ordinal corpus IDs and generic
faults, never source names/paths or authored contents. Only runner-owned scratch
is removed. Never point this public regression workflow at a user profile.
Actual video/audio decode, media availability/migration closure and project
formats beyond these fixtures remain explicit gaps.

## Running and exit status

From the native workspace, after the coordinator adds the lockfile entry below:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-acceptance
cargo fmt -p pm-acceptance -- --check
cargo clippy --locked -p pm-acceptance --all-targets --no-deps -- -D warnings
cargo run --locked -p pm-acceptance -- --format json
cargo run --locked -p pm-acceptance -- --workflow W033 --format markdown
cargo run --locked -p pm-acceptance -- --case A517
cargo run --locked -p pm-acceptance -- --case K001.route-normal
cargo run --locked -p pm-acceptance -- --case K001.normal --strict
cargo run --locked -p pm-acceptance -- --format e2e-csv
cargo run --locked -p pm-acceptance -- --format catalog
```

JSON format `powermove.acceptance`, version 1, includes per-case status, kind,
reason, blocker, macOS requirement, per-workflow aggregates and all legacy spec
mappings. `--format catalog` supplies complete expected/source contracts.
Case-filtered workflow aggregates say explicitly that they are not full workflow
verdicts. Exit 0 means no failed assertions, with blockers allowed; exit 1 means
an executed assertion failed; `--strict` returns 2 for blockers when there are no
failures; invalid configuration returns 3. The current full run exits **1**.

The root `apps/native/Cargo.lock` is outside this lane's ownership. It was not
edited. Tests ran in an isolated source-tree copy, with only this additional
package record resolved; no registry versions changed. The coordinator must
regenerate the shared lock before root-workspace `--locked` commands can run:

```toml
[[package]]
name = "pm-acceptance"
version = "0.1.0"
dependencies = [
 "pm-kernel",
 "pm-mcp",
 "pm-model",
 "serde",
 "serde_json",
]
```

## First measured coverage

| Evidence class | Pass | Fail | Blocked-by |
|---|---:|---:|---:|
| Original A/B variants | 7 | 0 | 1,337 |
| Complete W-group contracts | 0 | 0 | 83 |
| K binding integration variants (195 × 9) | 0 | 0 | 1,755 |
| Actual shell resolver probes (195 × 7) | 1,365 | 0 | 0 |
| Kernel/MCP supporting probes | 22 | 1 | 0 |
| Engine/surface adapter doubles | 2 | 0 | 0 |
| Public container copies | 3 | 0 | 0 |
| **Total** | **1,399** | **1** | **3,175** |

There are 4,575 reported cases. Twenty-one workflows have executed non-routing
supporting evidence. **Zero full workflows pass: W071 fails and 82 are blocked.**
Neither inventory counts nor routing-only probes measure product parity.

**Measured failure — W071.fractional-rate:** `apply_commands` with
`set_composition {patch:{fps:29.97}}` reads back `fps=30`.
`pm-kernel/src/commands/comps.rs`, `CompCommand::Set`, rounds the input, while its
Settings path preserves fractional rates. The acceptance assertion remains
29.97 exactly, under N3/N1; this is not an approved imitation of the tag. Fixing
the kernel is outside this lane. The harness tests verify that the defect is
reported as fail; they do not mark it passed or skipped. Remove that measured
baseline expectation once the kernel fix lands, preserving the acceptance
assertion. Source/composition/output-rate conversion still needs decoded output
verification on macOS.

Validation on Linux, Rust 1.96.1: **47 harness tests pass** (4 catalog, 6 runner,
37 scenario/fixture tests), formatting passes and strict Clippy reports no
warnings in `pm-acceptance`. Regression checks: **207 kernel tests pass, one
pre-existing ignored; 25 MCP tests pass**, including their frozen-source oracles
(Node 24.19 test tooling; v1.2.0/v1.3.0 Git objects). The kernel suite's initial
parallel run hit `selection_is_atomic_ephemeral_deduplicated_and_published_before_event`
in `tests/slice3.rs`; the test passed in isolation and the complete kernel suite
passed with `--test-threads=1`. The earlier event-order failure remains a kernel
race observation, not a harness fix. The dependency also emits its pre-existing
Linux unused `path` warning in `kernel/src/locks.rs:175`; this lane leaves it
unchanged.

## Legacy e2e replacement map

The generated [e2e-map.csv](../../apps/native/crates/acceptance/data/e2e-map.csv)
has **396 rows, including every one of the 370 requested specs**. Its
`acceptance_ids` name the mapped workflow contracts plus original A/B cases
whose immutable source names that exact spec. 162 union specs have such direct
A/B links; others retain explicit workflow contract targets. The separate
`supporting_probe_ids` column lists relevant executed slices, without claiming
they replace every assertion. The runner JSON retains the complete input triage
revision, roster membership, classification, disposition, skip/conditional
indicator and evidence. A regression test checks the committed CSV against the
runner's generated map.

All `replacement_status` values remain `blocked-by`: no e2e spec is retired by
this change. Product/mixed specs retain their full user-visible obligations;
`native-engine.spec.ts` and `scene3d-short-attributes.spec.ts` remain conditional
plumbing-only retirement candidates, with safety/resource/standards gates still
open. W072 records the owner's removal decision; W034 retains the documented
native scope boundary. Neither is silently counted as passed.

## Decisions and selecting rules

| Decision | Rule that selected it |
|---|---|
| Compile authoritative inventories and preserve all IDs/provenance | N4/N7: no silently lost requirement; headless inspection by stable ID |
| Keep original acceptance, supporting probes and full-group gates separate | N1/N3: a plumbing/control test cannot certify a pro editing workflow |
| Exercise the production pure resolver without linking macOS UI | N7/N8: real routing logic on Linux, no second implementation or Node runtime |
| Keep actual command effects, native text/menu/IME and pixels blocked | N3/N6: exact behavior needs the real adapter; synthetic output is not proof |
| Use kernel/MCP ownership, transactions, revisions and session saving | N3/N7/N8: one writer, guarded history, worker-only waits |
| Fail fractional-rate rounding instead of changing the expected value | N1/N2/N3: exact authored timing and stronger native behavior |
| Copy corpus inputs, compare retained bytes/state, never edit a source | N4: archive/retain user data; disposable synthetic scratch is the only cleanup |
| Preserve missing/null no-workspace semantics while requiring populated values exact | N3: absence of authored workspace is not browser serialization intent |
| Mark doubles explicitly and use V8/remote-DOM architecture in blockers | N4/N6/N8 and P0 decision 5: all v1–3 extensions remain in scope |
| Keep every legacy spec until its replacement gates pass | Prune rule/N4: this lane deletes no existing consumer or reference |
| Leave the shared lock and fractional-rate fix to their owning lanes | Explicit lane ownership; exact integration changes are recorded above |

## macOS verification and remaining work

Replace the engine/surface doubles with the real in-process engine and semantic
shell hosts. Verify all nine variants per key, AppKit menu deduplication, text
ownership/IME, native accessibility, focus restoration, panels/popouts/windows,
clipboard, dialogs and save/relaunch. Exercise the v1–3 V8 logic/PM/sync-service/
recorder-canvas realm with WebKit remote DOM, plus future v4 panels; none of
those runtime contracts is certified here.

Run decoded frame/audio, standards/f64/colour/text/shader oracles, nested/time-
remapped media, offline/proxy/relink/migration closure, cancellation/export jobs,
physical display/audio timing and controlled performance gates on macOS. Finish
pending MCP/domain/media/agent/Store adapters and the five pro cutover additions.
Implement original blocked A/B contracts as whole cases, then promote W-group
contracts only when all their preconditions, interactions, persistence, cancel,
Undo/Redo and required oracles execute. The table below identifies each current
blocker; the JSON report identifies every individual variant.

Acceptance format v1; platform `linux`; scope `full-inventory`.

Cases: **1399 pass / 1 fail / 3175 blocked-by**.

| Workflow | Verdict | Pass | Fail | Blocked | Reason |
|---|---|---:|---:|---:|---|
| W001 — Launch, restore and activate the last project | blocked-by | 0 | 0 | 14 | 0 passed, 0 failed, 14 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W002 — Move, reorder and focus project tabs across windows | blocked-by | 0 | 0 | 20 | 0 passed, 0 failed, 20 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W003 — Close a project or quit without losing work | blocked-by | 0 | 0 | 9 | 0 passed, 0 failed, 9 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W004 — Recover edits, undo and external connections after a crash | blocked-by | 1 | 0 | 6 | 1 passed, 0 failed, 6 blocked; Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W005 — Inspect and apply app/runtime updates | blocked-by | 0 | 0 | 4 | 0 passed, 0 failed, 4 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W006 — Create, open, rename, duplicate and trash projects | blocked-by | 57 | 0 | 82 | 57 passed, 0 failed, 82 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W007 — Save and reopen a portable project with media and history | blocked-by | 32 | 0 | 64 | 32 passed, 0 failed, 64 blocked; Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W008 — Open large projects while retaining input and tab intent | blocked-by | 0 | 0 | 9 | 0 passed, 0 failed, 9 blocked; Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W009 — Create and configure compositions and delivery defaults | blocked-by | 29 | 0 | 44 | 29 passed, 0 failed, 44 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W010 — Resolve every registered shortcut and preserve text ownership | blocked-by | 7 | 0 | 25 | 7 passed, 0 failed, 25 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W011 — Reach editor commands through native and contextual menus | blocked-by | 0 | 0 | 77 | 0 passed, 0 failed, 77 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W012 — Find and change app settings | blocked-by | 14 | 0 | 34 | 14 passed, 0 failed, 34 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W013 — Complete first-run setup and inspect About | blocked-by | 0 | 0 | 6 | 0 passed, 0 failed, 6 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W014 — Arrange, resize, hide and restore panels/workspaces | blocked-by | 0 | 0 | 14 | 0 passed, 0 failed, 14 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W015 — Edit in a popped-out panel and return it | blocked-by | 0 | 0 | 5 | 0 passed, 0 failed, 5 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W016 — Browse workspaces and live panel previews | blocked-by | 0 | 0 | 7 | 0 passed, 0 failed, 7 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W017 — Use command search, modal dialogs, menus and notices | blocked-by | 14 | 0 | 41 | 14 passed, 0 failed, 41 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W018 — Select and organize project media | blocked-by | 0 | 0 | 17 | 0 passed, 0 failed, 17 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W019 — Import durable media and cancel safely | blocked-by | 28 | 0 | 61 | 28 passed, 0 failed, 61 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W020 — Import, replace and rate-convert an image sequence | blocked-by | 0 | 0 | 9 | 0 passed, 0 failed, 9 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W021 — Preview source media without altering the timeline | blocked-by | 0 | 0 | 8 | 0 passed, 0 failed, 8 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W022 — Recover offline sources or replace media in place | blocked-by | 0 | 0 | 13 | 0 passed, 0 failed, 13 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W023 — Defer and download cloud media | blocked-by | 0 | 0 | 4 | 0 passed, 0 failed, 4 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W024 — Connect subscription, API and local agent providers | blocked-by | 0 | 0 | 32 | 0 passed, 0 failed, 32 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W025 — Open, float, pin and resize the agent conversation | blocked-by | 14 | 0 | 34 | 14 passed, 0 failed, 34 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W026 — Write, attach and send an agent prompt | blocked-by | 0 | 0 | 12 | 0 passed, 0 failed, 12 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W027 — Run and revisit ordinary/background agent threads | blocked-by | 0 | 0 | 24 | 0 passed, 0 failed, 24 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W028 — Read streaming replies, reasoning, approvals and results | blocked-by | 0 | 0 | 7 | 0 passed, 0 failed, 7 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W029 — Publish and reopen interactive visual replies | blocked-by | 0 | 0 | 11 | 0 passed, 0 failed, 11 blocked; Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W030 — Let an agent inspect, edit, verify and undo project work | blocked-by | 4 | 0 | 15 | 4 passed, 0 failed, 15 blocked; Implemented MCP cases run; remaining tool bodies, complete external entry/agent/render integration are pending |
| W031 — Inspect and manage footage through agent tools | blocked-by | 0 | 0 | 17 | 0 passed, 0 failed, 17 blocked; Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W032 — Inspect and operate live panels or transfer a workspace | blocked-by | 1 | 0 | 15 | 1 passed, 0 failed, 15 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W033 — Use an external/headless tool session | blocked-by | 8 | 0 | 15 | 8 passed, 0 failed, 15 blocked; Implemented MCP cases run; remaining tool bodies, complete external entry/agent/render integration are pending |
| W034 — Use the existing browser-host/CLI workflows | blocked-by | 0 | 0 | 8 | 0 passed, 0 failed, 8 blocked; Existing browser/serve workflows have an explicit out-of-scope native boundary in W1; retain evidence |
| W035 — Manage account identity and remembered Store installs | blocked-by | 0 | 0 | 5 | 0 passed, 0 failed, 5 blocked; Store/account lifecycle adapters and explicit-action synthetic service fixtures are pending |
| W036 — Browse, install, update and publish extensions | blocked-by | 0 | 0 | 18 | 0 passed, 0 failed, 18 blocked; Store/account lifecycle adapters and explicit-action synthetic service fixtures are pending |
| W037 — Install, fork, rebase, reload and disable extensions | blocked-by | 0 | 0 | 30 | 0 passed, 0 failed, 30 blocked; Production V8 v1–3 logic/PM/services/recorder canvas and WebKit remote DOM are pending; no extension is excluded |
| W038 — Use Store extensions under permission and crash isolation | blocked-by | 0 | 0 | 13 | 0 passed, 0 failed, 13 blocked; Production V8 v1–3 logic/PM/services/recorder canvas and WebKit remote DOM are pending; no extension is excluded |
| W039 — Configure extension variables and durable storage | blocked-by | 0 | 0 | 3 | 0 passed, 0 failed, 3 blocked; Production V8 v1–3 logic/PM/services/recorder canvas and WebKit remote DOM are pending; no extension is excluded |
| W040 — Run legacy extension APIs and contributions unchanged | blocked-by | 0 | 0 | 51 | 0 passed, 0 failed, 51 blocked; Production V8 v1–3 logic/PM/services/recorder canvas and WebKit remote DOM are pending; no extension is excluded |
| W041 — Download and manage local speech models | blocked-by | 0 | 0 | 16 | 0 passed, 0 failed, 16 blocked; Speech-model download/resume/transcription adapters are pending |
| W042 — Apply appearance and themes without losing customization | blocked-by | 0 | 0 | 17 | 0 passed, 0 failed, 17 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W043 — Navigate with keyboard, text services and accessibility preferences | blocked-by | 0 | 0 | 7 | 0 passed, 0 failed, 7 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W044 — Receive completion notifications, warnings and alignment haptics | blocked-by | 0 | 0 | 5 | 0 passed, 0 failed, 5 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W045 — Navigate zoom, pan, fit and recover the composition view | blocked-by | 126 | 0 | 194 | 126 passed, 0 failed, 194 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W046 — Select and move canvas layers and groups | blocked-by | 204 | 0 | 287 | 204 passed, 0 failed, 287 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W047 — Resize, rotate, align and change anchors | blocked-by | 42 | 0 | 73 | 42 passed, 0 failed, 73 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W048 — Draw and edit vector shapes, paths and masks | blocked-by | 71 | 0 | 114 | 71 passed, 0 failed, 114 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W049 — Create and edit point and paragraph text on canvas | blocked-by | 36 | 0 | 76 | 36 passed, 0 failed, 76 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W050 — Build and navigate editable 2.5D/3D scenes | blocked-by | 0 | 0 | 31 | 0 passed, 0 failed, 31 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W051 — Play, pause, step and navigate visible events | blocked-by | 133 | 0 | 187 | 133 passed, 0 failed, 187 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W052 — Set preview/export in and out points | blocked-by | 57 | 0 | 82 | 57 passed, 0 failed, 82 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W053 — Add, persist, undo and remove composition markers | blocked-by | 1 | 0 | 8 | 1 passed, 0 failed, 8 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W054 — Snap playhead, clips and selected keys without changing spacing | blocked-by | 0 | 0 | 17 | 0 passed, 0 failed, 17 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W055 — Arrange layers/tracks, disclosures and viewport | blocked-by | 1 | 0 | 60 | 1 passed, 0 failed, 60 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W056 — Move, split and trim clips with source continuity | blocked-by | 71 | 0 | 104 | 71 passed, 0 failed, 104 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W057 — Slip a source range through the shipped editing API | blocked-by | 1 | 0 | 2 | 1 passed, 0 failed, 2 blocked; Typed timing probe is supporting evidence; edit_video and decoded start/end frame oracles remain pending |
| W058 — Retime video while preserving its retained source interval | blocked-by | 0 | 0 | 3 | 0 passed, 0 failed, 3 blocked; Typed timing probe is supporting evidence; edit_video and decoded start/end frame oracles remain pending |
| W059 — Play embedded A/V, separate audio and mix it | blocked-by | 0 | 0 | 7 | 0 passed, 0 failed, 7 blocked; Real audio routing, levels and speaker synchronization need the macOS engine |
| W060 — Create, reveal, edit and move keyframes | blocked-by | 183 | 0 | 277 | 183 passed, 0 failed, 277 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W061 — Edit value/speed curves and Bézier handles | blocked-by | 36 | 0 | 72 | 36 passed, 0 failed, 72 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W062 — Group, parent, reorder and precompose without changing authored output | blocked-by | 99 | 0 | 159 | 99 passed, 0 failed, 159 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W063 — Edit selected content, transforms and composition properties | blocked-by | 1 | 0 | 59 | 1 passed, 0 failed, 59 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W064 — Browse, apply, animate and reorder effects or mattes | blocked-by | 0 | 0 | 7 | 0 passed, 0 failed, 7 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W065 — Edit and organize easing/vector preset libraries | blocked-by | 0 | 0 | 11 | 0 passed, 0 failed, 11 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W066 — Change typography, variable axes and text animators | blocked-by | 0 | 0 | 19 | 0 passed, 0 failed, 19 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W067 — Generate, edit, style and exchange captions | blocked-by | 0 | 0 | 79 | 0 passed, 0 failed, 79 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W068 — Mix audible nested layers and inspect meters | blocked-by | 0 | 0 | 8 | 0 passed, 0 failed, 8 blocked; Real audio routing, levels and speaker synchronization need the macOS engine |
| W069 — Configure, start and cancel rendered delivery | blocked-by | 28 | 0 | 56 | 28 passed, 0 failed, 56 blocked; MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W070 — Render queued composition snapshots and retry failures | blocked-by | 0 | 0 | 6 | 0 passed, 0 failed, 6 blocked; MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W071 — Convert source/composition/output frame rates | fail | 0 | 1 | 1 | 0 passed, 1 failed, 1 blocked; MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W072 — Export the shipped standalone web animation | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; Owner removed standalone web-animation export from cutover requirements (2026-10-29); retain traceability, never claim pass |
| W073 — Use numeric, choice, toggle and point controls with keyboard | blocked-by | 0 | 0 | 58 | 0 passed, 0 failed, 58 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W074 — Pick, sample and edit solid/gradient colors | blocked-by | 0 | 0 | 38 | 0 passed, 0 failed, 38 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W075 — Undo/redo, clipboard and live edit transactions across surfaces | blocked-by | 99 | 0 | 142 | 99 passed, 0 failed, 142 blocked; needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W076 — Render, capture and recover authored content correctly | blocked-by | 1 | 0 | 1 | 1 passed, 0 failed, 1 blocked; needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W077 — Keep launch, editing and long playback responsive | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W078 — Write and compile a custom shader | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W079 — Cutover addition: state-dependent J/K/L shuttle | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; needs macOS: J/K/L engine transport plus shipped command fallthrough/decline integration |
| W080 — Cutover addition: ripple delete and ripple trim | blocked-by | 0 | 0 | 2 | 0 passed, 0 failed, 2 blocked; Required cutover addition: ripple/roll/slide/three-point/persistent A/V domain and UI integration is pending |
| W081 — Cutover additions: roll and slide edits | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; Required cutover addition: ripple/roll/slide/three-point/persistent A/V domain and UI integration is pending |
| W082 — Cutover addition: source-monitor three-point editing | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; Required cutover addition: ripple/roll/slide/three-point/persistent A/V domain and UI integration is pending |
| W083 — Cutover addition: persistent linked A/V edits | blocked-by | 0 | 0 | 1 | 0 passed, 0 failed, 1 blocked; Required cutover addition: ripple/roll/slide/three-point/persistent A/V domain and UI integration is pending |
