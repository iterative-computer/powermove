# W1 native acceptance harness

`apps/native/crates/acceptance` (`pm-acceptance`) runs headlessly against the
production kernel and MCP traits. Wave 2 package 20 starts at native-engine
`36e677963` on 2026-11-01. Linux coverage is **not a native parity or cutover pass**.
No shell, engine, vendor, spike or shared workspace lockfile is changed here.

## Before and after

These are measured acceptance **cases**, separate from Rust test counts. The
before run uses the unchanged base; the configured after run supplies the native
V8 backing and the bundle built from the current compat sources.

| Evidence class | Before pass/fail/blocked | After pass/fail/blocked |
|---|---:|---:|
| Original A/B variants | 7 / 0 / 1,337 | 11 / 0 / 1,333 |
| Complete W-group contracts | 0 / 0 / 83 | 0 / 0 / 83 |
| Key integration variants (195 × 9) | 0 / 0 / 1,755 | 0 / 0 / 1,755 |
| Actual shell resolver probes (195 × 7) | 1,365 / 0 / 0 | 1,365 / 0 / 0 |
| Kernel/MCP/pro supporting probes | 22 / 1 / 0 | 29 / 1 / 0 |
| Actual V8 and compat-source probes | 0 / 0 / 0 | 4 / 0 / 0 |
| Engine/surface adapter doubles | 2 / 0 / 0 | 2 / 0 / 0 |
| Public container copies | 3 / 0 / 0 | 3 / 0 / 0 |
| **Total** | **1,399 / 1 / 3,175** | **1,414 / 1 / 3,171** |

The configured run has **4,586 cases**. Without `--runtime-runner` and
`--compat-bundle`, the four runtime cases are explicit portable blockers:
**1,410 pass / 1 fail / 3,175 blocked**. Both modes exit **1** for the measured
fractional-rate failure. Zero full workflows pass: W071 fails and 82 are blocked.

All **396 legacy specs**, including the frozen **370-spec roster**, retain
`blocked-by` replacement status before and after. There are **0 untriaged**
rows after this pass; none of the specs is retired. The pre-existing replacement
map already linked all 396; this pass adds those explicit acceptance/probe IDs,
status, `needs_macos` and dependency columns to the authoritative triage CSV,
and regression-checks them against the runner. 389 specs currently have a macOS
integration gate; the other seven retain named non-macOS dependencies.

**Measured failure — W071.fractional-rate:** MCP `apply_commands` with
`set_composition {patch:{fps:29.97}}` reads back `fps=30` because
`pm-kernel/src/commands/comps.rs`, `CompCommand::Set`, rounds it. The acceptance
assertion remains exactly 29.97 (N1/N3). The owning kernel lane must fix this;
then remove the old measured-failure expectation in `tests/scenarios.rs` and
update report counts while preserving the exact authored-value assertion.

## Inventory and replacement limits

The runner compiles [W1 workflows](w1-workflows.md), [keymap](w1-keymap.md) and
[e2e triage](e2e-triage.csv). It preserves 83 workflow contracts, all 1,344
rebased A/B variants with source provenance, all 195 bindings with arguments,
priority, field/repeat/loose flags, and all 396 triage rows. The 48 wholly
post-tag appendix claims are not promoted into shipped requirements.

Every original variant has a result. Implemented originals execute their MCP
bodies through `McpSession` and the host's single kernel. Unsupported variants
and each full `Wnnn.contract` keep named dependencies and reasons. A subset pass
never makes a complete workflow pass. Assertion failures, backing-process errors
and panics are **fail**, never blockers. Unknown/empty/mismatched filters are
configuration errors. No acceptance test is ignored or skipped.

The generated [e2e-map.csv](../../apps/native/crates/acceptance/data/e2e-map.csv)
retains every spec. `acceptance_ids` contains workflow contracts plus original
A/B cases whose source names that exact spec path. 162 specs have direct A/B
links. Path-boundary matching prevents `timeline.spec.ts` from acquiring
`audio-timeline.spec.ts` cases. `supporting_probe_ids` records narrower evidence
without certifying the spec's remaining interaction or pixel assertions. JSON
also preserves roster membership, evidence revision, classification, disposition,
conditional/skip indicator and coverage limits. Triage/map drift fails tests.

Product and mixed specs keep their full obligations. The two plumbing-only
retirement candidates (`native-engine.spec.ts`, `scene3d-short-attributes.spec.ts`)
still need safety/resource/standards gates. W072 retains the owner's removal
of standalone web-animation export; W034 retains its explicit native scope
boundary. Neither becomes a pass. Existing tests and consumers are retained.

## New portable automation

Four more original MCP cases execute:

| ID | Executed contract |
|---|---|
| A494 / W033 | Exposed frozen tool union, unique names, implemented members and allowlist references |
| A503 / W033 | Timeline stack order, paging, selection, frame bounds, source ranges/rate, flags and parent identity without edits |
| A505 / W033 | Library list/create/open/rename/duplicate/save/trash/restore, retaining the original and copied document |
| A520 / W032 | Saved dock/panel digest in project and app sessions, absent providers, authored titles, size defaults and malformed-input refusal |

The existing seven originals remain: A495–A498, A501, A517 and A531.
`W033.unavailable` now derives its pending set from **all catalog tools minus
`pm_mcp::catalog::IMPLEMENTED`**. Every pending tool receives schema-valid
synthetic arguments and must return `not_available_in_this_build` with the exact
catalog backing. The current pending set has 53 members; no particular tool or
fixed count must remain pending. Newly unfamiliar schemas fail with an explicit
request for a synthetic input, rather than silently reducing coverage.

Seven new pro supporting probes use `KernelActor`, `ActorAutomation`, typed
transactions and stable IDs. They check exact authored timing/source/key data,
atomic locks/handle refusals, and untouched tracks:

| IDs | Evidence |
|---|---|
| W079.shuttle | State-dependent J/K/L, saturation, direction changes, paused decline, explicit reverse; actor transport publication without document revision/dirty/history changes |
| W080.ripple-delete / ripple-trim | Explicit-track gap closure and exact positive/negative trim deltas; downstream lock refusal |
| W081.roll / slide | Outer-boundary/source continuity and rebased keys; middle-source/animation retention; handles/locks |
| W082.source | Three-point video insert and overwrite across a clip, retained fragments/source offsets, untargeted tracks and missing-source refusal |
| W083.links | Persistent A/V link, linked move/trim/split/delete, explicit slip bypass, source-edit cancellation and unlink |

Each accepted document operation runs Undo/Redo, saves to a fresh synthetic PMV,
reopens through the production reader, then runs persisted Undo/Redo. These are
domain probes; source-monitor UI, audio-only/both-stream UI, native drags and
decoded frame/audio assertions remain integration gates.

Existing probes cover library/journal recovery, PMV4/history, atomic batches,
stale revisions/interleaved writers, locks/disconnect, composition settings,
selection, work bounds/markers, strips, split/slip, keyframes/easing, groups,
Unicode text, masks and exact f64 properties. W057 retains its exact
from=2, duration=3, trim=1, rate=2, source=12 fixture: allow 5+3×2, refuse 8+3×2,
retain animation, Undo and Redo.

## Actual V8 backing and compat boundary

`runtime/` is a separately locked native backing executable, depending on the
production `pm-runtime-v8`. Its independent manifest keeps V8 build/download
requirements out of the default kernel-only harness and avoids editing the
shared workspace lockfile. It uses upstream Linux V8 152.2.0; it does not build
V8 from source or run Node/Chromium. macOS requires `--features macos-sandbox`
and the S1 archive **with matching bindings** described in [runtime-v8.md](runtime-v8.md).

`RuntimeHook` keeps backing capability explicit. `--runtime-runner` selects the
native executable, and `--compat-bundle` selects the generated current-source
bundle. Missing configuration is blocked with `needs_macos=false`; spawn errors,
nonzero exits, timeouts, malformed/oversized responses and assertion failures
are failures. The parent enforces a 20-second wall limit and kills/reaps its own
child on timeout. Filters execute only their selected backing case.

| ID | Executed evidence and limits |
|---|---|
| W037.v8-reload | Real V8 failed activation preserves the active generation, live provider and storage/hot data; failed staged ops never publish; process/document globals are absent |
| W038.v8-stop | Explicit Stop terminates an infinite loop while a second real isolate keeps evaluating; this does **not** certify Store helper-process confinement |
| W037.compat-services | Current `CompatRealm` in actual V8: v1–3 declared versions, live synchronous service object/closure identity, provider override/disposal and restoration |
| W040.compat-recorder | Current `RecorderCanvas` in actual V8: canvas/context identity, immutable drawImage source-version pins across resize/disposal, release of retained commands |

The compat entry imports `apps/native/js/compat` source directly. Bun is only a
test bundler; no generated bundle, second facade implementation or runtime
Node dependency is committed. Runtime kernel/storage hooks and recorder host
are explicit synthetic doubles. The service probe has no project edits; the
recorder probe produces no pixels and refuses readback. Full PM-to-kernel
channels, native raster, remote DOM and extension corpus conformance remain open.
Successful owner disposal is awaited on a worker with bounded deadlines.

## Input, engine and platform boundaries

The macOS-only shell cannot link on Linux. The harness compiles its existing
pure `keyboard.rs` source in place and compares all K rows against its shipped
table under seven routing contexts: normal, held, extra modifier, input,
contenteditable, prior decline and key-up. These 1,365 passes prove resolver
admission/command/args, not command effects or AppKit focus behavior. Each binding
also retains nine blocked integration variants: normal, held, extra modifier,
input, contenteditable, read-only selection, focused button, prior extension
decline and menu single execution. All 1,755 have `needs_macos=true`.

Engine/surface doubles implement the actual MCP `EngineHook`/`SurfaceHook` traits.
Their two passing cases remain `test-double-probe` with `needs_macos=true`.
They check revision-bound requests and stable-ID inspection/drive, including
stale/unknown-ID refusal. No pixels or real input are fabricated.

The workflow blocker text now acknowledges existing V8/pro backing and names
the remaining native raster/remote-DOM, gestures, source-monitor, decoded A/V
and engine integration gates. W080–W083 full contracts have `needs_macos=true`.

## Synthetic fixtures and public-data safety

Every scenario owns disposable profile, journal, export and run directories.
Blocking actor waits, child waits and Drop run on workers, never macOS main.
All document changes pass through kernel transactions; no second writer exists.
No installed profile is discovered and no Electron process is launched.

Three embedded public engine container fixtures cover JSON, PMV3 and PMV4.
Inputs are copied before opening and saved to a second scratch destination.
Checks retain exact values (including unknown fields/lone surrogates), envelope,
workspace, history/cursor or retained opaque history, media IDs/revisions/types
and every embedded byte. Missing workspace and null both mean no authored
workspace; populated workspaces must stay exact. Bounded streaming compares
media, including unreachable records; omitted media fails.

`--corpus /absolute/public-synthetic-corpus` adds sorted immediate `.pmv`/`.json`
inputs, without recursion/discovery. `create_new` copies prevent overwriting;
file symlinks are refused. Sources are never saved/locked/renamed/deleted.
Reports use ordinal IDs and generic faults, not source paths or contents.
Cleanup removes only runner-owned scratch. User data must never enter this
public regression workflow. Migration closure, real decode and unsupported
project-format proofs remain separate gates.

## Running and verification

From the repository root, with Rust, Bun and the documented toolchain available:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
apps/native/crates/acceptance/scripts/verify.sh
```

This runs both locked acceptance suites, formatting and strict crate-local
Clippy, builds the current compat bundle under `work/native-acceptance`, then
writes `report.json` there. Its final exit is **1** for W071's known failure;
it does not hide that failure behind passing harness tests. It accepts CLI
filters such as `--case W037.compat-services` and `--workflow W033 --strict`.

Default harness and optional backing can also be run separately:

```sh
cd apps/native
cargo test --locked -p pm-acceptance
cargo run --locked -p pm-acceptance -- --workflow W033 --format markdown
cargo run --locked -p pm-acceptance -- --format e2e-csv
cargo run --locked -p pm-acceptance -- --format catalog
cargo test --locked --manifest-path crates/acceptance/runtime/Cargo.toml
```

JSON format `powermove.acceptance`, version 1, includes case kinds/status/reasons,
blockers, macOS flags, workflow aggregates and all legacy mappings. A filtered
workflow explicitly says it is not a full-workflow verdict. Exit codes: 0 means
no failures with blockers permitted; 1 means an assertion failed; `--strict`
returns 2 for blockers if there are no failures; 3 means invalid configuration.

Linux verification used Rust **1.99.1**, Bun **1.3.14**, and Node **24.19.0**
only for existing frozen-source regression oracles. **63 pm-acceptance tests
pass** (5 catalog, 6 runner, 4 runtime-adapter, 48 scenarios/fixtures), plus **4
native-backing tests**. Formatting and strict `--all-targets --no-deps` Clippy
pass for both owned crates. Kernel/MCP regressions pass serially: **267 kernel
and 29 MCP tests**, plus **45 production V8 tests**, with one pre-existing
ignored kernel profile test. Existing
out-of-lane warnings remain: Linux `kernel/locks.rs:175` unused `path`, and
Rust 1.99's `runtime-v8/runtime.rs:304` deprecated `fetch_update`. No owned crate
warning is introduced. The activation/tooling and parity tags were initially
absent; tools were installed under untracked scratch and public tag objects
fetched for the existing oracles.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Keep all legacy specs and exact source/ID links; explicit triage cannot silently promote or drop a spec | N4/N7 and prune rule: no lost requirement or removed consumer |
| Keep original cases, supporting probes, full workflow gates, native runtime probes and doubles distinguishable | N1/N3: a partial domain or recorder test cannot establish UI/pixel acceptance |
| Derive pending tools from `catalog::IMPLEMENTED`, validating inputs first | N3/N7: automation checks real capabilities as implementations land |
| Use real actor commands, exact values, atomic refusal and persisted Undo/Redo | N1/N2/N3/N4: stronger pro editing without altered authored intent or lost recovery |
| Separately locked optional V8 backing, current-source test bundle and explicit missing-capability blockers | N7/N8 and ownership: headless/drivable, no Node in native execution, no shared-lock changes |
| Preserve v1–3 live service identity and recorder pins in their V8 realm; leave WebKit to remote DOM | N4/N6/N8 and P0 decision 5; no compatibility exclusion or browser-quirk adoption |
| Keep W071's exact 29.97 assertion failing until the kernel fix | N1/N3: correctness over imitation |
| Require macOS for real input, native raster/remote DOM, decoded A/V and complete pro workflows | N1/N3/N6: Linux doubles cannot clear those gates |
| Copy public inputs and clean only owned synthetic scratch | N4: source data remains retained; no archive or user file is deleted |

## macOS verification and remaining work

Verify GPUI/AppKit hit testing, focus/IME/AX, menus without duplicate execution,
all nine key variants, clipboard/dialogs, save/relaunch, panels/popouts/windows,
WebKit remote DOM and the complete v1–3 corpus with live PM/kernel services.
Replace engine/surface doubles and recorder hosts with the real in-process
engine/native raster. Check decoded source-frame identity, A/V sync, reverse
rates/play-around, exact fractional delivery timing, color/text/shaders,
linked bypass/track targets, native gesture cancellation and persisted history.
Physical display/audio/performance gates and signed sandbox/helper-process
confinement remain macOS work. No macOS execution was performed here.

Pending MCP/domain/media/agent/Store adapters and full migration/media closure
remain outside this lane. Kernel must preserve fractional fps in
`CompCommand::Set`. No shared lockfile change is required. W1 source contracts
are unchanged; measured statuses live in this document and the triage CSV.

## Current configured workflow report

| Workflow | Verdict | Pass | Fail | Blocked | Remaining gate |
|---|---|---:|---:|---:|---|
| W001 — Launch, restore and activate the last project | blocked-by | 0 | 0 | 14 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W002 — Move, reorder and focus project tabs across windows | blocked-by | 0 | 0 | 20 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W003 — Close a project or quit without losing work | blocked-by | 0 | 0 | 9 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W004 — Recover edits, undo and external connections after a crash | blocked-by | 1 | 0 | 6 | Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W005 — Inspect and apply app/runtime updates | blocked-by | 0 | 0 | 4 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W006 — Create, open, rename, duplicate and trash projects | blocked-by | 57 | 0 | 82 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W007 — Save and reopen a portable project with media and history | blocked-by | 32 | 0 | 64 | Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W008 — Open large projects while retaining input and tab intent | blocked-by | 0 | 0 | 9 | Kernel slices run; complete corpus/media closure, recovery and shell lifecycle acceptance remains |
| W009 — Create and configure compositions and delivery defaults | blocked-by | 29 | 0 | 44 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W010 — Resolve every registered shortcut and preserve text ownership | blocked-by | 7 | 0 | 25 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W011 — Reach editor commands through native and contextual menus | blocked-by | 0 | 0 | 77 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W012 — Find and change app settings | blocked-by | 14 | 0 | 34 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W013 — Complete first-run setup and inspect About | blocked-by | 0 | 0 | 6 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W014 — Arrange, resize, hide and restore panels/workspaces | blocked-by | 0 | 0 | 14 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W015 — Edit in a popped-out panel and return it | blocked-by | 0 | 0 | 5 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W016 — Browse workspaces and live panel previews | blocked-by | 0 | 0 | 7 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W017 — Use command search, modal dialogs, menus and notices | blocked-by | 14 | 0 | 41 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W018 — Select and organize project media | blocked-by | 0 | 0 | 17 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W019 — Import durable media and cancel safely | blocked-by | 28 | 0 | 61 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W020 — Import, replace and rate-convert an image sequence | blocked-by | 0 | 0 | 9 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W021 — Preview source media without altering the timeline | blocked-by | 0 | 0 | 8 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W022 — Recover offline sources or replace media in place | blocked-by | 0 | 0 | 13 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W023 — Defer and download cloud media | blocked-by | 0 | 0 | 4 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W024 — Connect subscription, API and local agent providers | blocked-by | 0 | 0 | 32 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W025 — Open, float, pin and resize the agent conversation | blocked-by | 14 | 0 | 34 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W026 — Write, attach and send an agent prompt | blocked-by | 0 | 0 | 12 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W027 — Run and revisit ordinary/background agent threads | blocked-by | 0 | 0 | 24 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W028 — Read streaming replies, reasoning, approvals and results | blocked-by | 0 | 0 | 7 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W029 — Publish and reopen interactive visual replies | blocked-by | 0 | 0 | 11 | Provider/thread/approval/composer integration is pending; use synthetic providers only |
| W030 — Let an agent inspect, edit, verify and undo project work | blocked-by | 4 | 0 | 15 | Implemented MCP cases run; remaining tool bodies, complete external entry/agent/render integration are pending |
| W031 — Inspect and manage footage through agent tools | blocked-by | 0 | 0 | 17 | Durable import/relink/source-preview MCP adapter and decoded-media oracles are pending |
| W032 — Inspect and operate live panels or transfer a workspace | blocked-by | 2 | 0 | 14 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W033 — Use an external/headless tool session | blocked-by | 11 | 0 | 12 | Implemented MCP cases run; remaining tool bodies, complete external entry/agent/render integration are pending |
| W034 — Use the existing browser-host/CLI workflows | blocked-by | 0 | 0 | 8 | Existing browser/serve workflows have an explicit out-of-scope native boundary in W1; retain evidence |
| W035 — Manage account identity and remembered Store installs | blocked-by | 0 | 0 | 5 | Store/account lifecycle adapters and explicit-action synthetic service fixtures are pending |
| W036 — Browse, install, update and publish extensions | blocked-by | 0 | 0 | 18 | Store/account lifecycle adapters and explicit-action synthetic service fixtures are pending |
| W037 — Install, fork, rebase, reload and disable extensions | blocked-by | 2 | 0 | 30 | V8 lifecycle and compat service/recorder probes run with the optional native backing; full corpus, host services, WebKit remote DOM and native raster pixels still need macOS; no extension is excluded |
| W038 — Use Store extensions under permission and crash isolation | blocked-by | 1 | 0 | 13 | V8 lifecycle and compat service/recorder probes run with the optional native backing; full corpus, host services, WebKit remote DOM and native raster pixels still need macOS; no extension is excluded |
| W039 — Configure extension variables and durable storage | blocked-by | 0 | 0 | 3 | V8 lifecycle and compat service/recorder probes run with the optional native backing; full corpus, host services, WebKit remote DOM and native raster pixels still need macOS; no extension is excluded |
| W040 — Run legacy extension APIs and contributions unchanged | blocked-by | 1 | 0 | 51 | V8 lifecycle and compat service/recorder probes run with the optional native backing; full corpus, host services, WebKit remote DOM and native raster pixels still need macOS; no extension is excluded |
| W041 — Download and manage local speech models | blocked-by | 0 | 0 | 16 | Speech-model download/resume/transcription adapters are pending |
| W042 — Apply appearance and themes without losing customization | blocked-by | 0 | 0 | 17 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W043 — Navigate with keyboard, text services and accessibility preferences | blocked-by | 0 | 0 | 7 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W044 — Receive completion notifications, warnings and alignment haptics | blocked-by | 0 | 0 | 5 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W045 — Navigate zoom, pan, fit and recover the composition view | blocked-by | 126 | 0 | 194 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W046 — Select and move canvas layers and groups | blocked-by | 204 | 0 | 287 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W047 — Resize, rotate, align and change anchors | blocked-by | 42 | 0 | 73 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W048 — Draw and edit vector shapes, paths and masks | blocked-by | 71 | 0 | 114 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W049 — Create and edit point and paragraph text on canvas | blocked-by | 36 | 0 | 76 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W050 — Build and navigate editable 2.5D/3D scenes | blocked-by | 0 | 0 | 31 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W051 — Play, pause, step and navigate visible events | blocked-by | 133 | 0 | 187 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W052 — Set preview/export in and out points | blocked-by | 57 | 0 | 82 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W053 — Add, persist, undo and remove composition markers | blocked-by | 1 | 0 | 8 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W054 — Snap playhead, clips and selected keys without changing spacing | blocked-by | 0 | 0 | 17 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W055 — Arrange layers/tracks, disclosures and viewport | blocked-by | 1 | 0 | 60 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W056 — Move, split and trim clips with source continuity | blocked-by | 71 | 0 | 104 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W057 — Slip a source range through the shipped editing API | blocked-by | 1 | 0 | 2 | Typed timing probe is supporting evidence; edit_video and decoded start/end frame oracles remain pending |
| W058 — Retime video while preserving its retained source interval | blocked-by | 0 | 0 | 3 | Typed timing probe is supporting evidence; edit_video and decoded start/end frame oracles remain pending |
| W059 — Play embedded A/V, separate audio and mix it | blocked-by | 0 | 0 | 7 | Real audio routing, levels and speaker synchronization need the macOS engine |
| W060 — Create, reveal, edit and move keyframes | blocked-by | 183 | 0 | 277 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W061 — Edit value/speed curves and Bézier handles | blocked-by | 36 | 0 | 72 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W062 — Group, parent, reorder and precompose without changing authored output | blocked-by | 99 | 0 | 159 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W063 — Edit selected content, transforms and composition properties | blocked-by | 1 | 0 | 59 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W064 — Browse, apply, animate and reorder effects or mattes | blocked-by | 0 | 0 | 7 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W065 — Edit and organize easing/vector preset libraries | blocked-by | 0 | 0 | 11 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W066 — Change typography, variable axes and text animators | blocked-by | 0 | 0 | 19 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W067 — Generate, edit, style and exchange captions | blocked-by | 0 | 0 | 79 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W068 — Mix audible nested layers and inspect meters | blocked-by | 0 | 0 | 8 | Real audio routing, levels and speaker synchronization need the macOS engine |
| W069 — Configure, start and cancel rendered delivery | blocked-by | 28 | 0 | 56 | MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W070 — Render queued composition snapshots and retry failures | blocked-by | 0 | 0 | 6 | MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W071 — Convert source/composition/output frame rates | fail | 0 | 1 | 1 | MCP export jobs, codec/media adapters, cancellation and frame-rate output verification are pending |
| W072 — Export the shipped standalone web animation | blocked-by | 0 | 0 | 1 | Owner removed standalone web-animation export from cutover requirements (2026-10-29); retain traceability, never claim pass |
| W073 — Use numeric, choice, toggle and point controls with keyboard | blocked-by | 0 | 0 | 58 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W074 — Pick, sample and edit solid/gradient colors | blocked-by | 0 | 0 | 38 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W075 — Undo/redo, clipboard and live edit transactions across surfaces | blocked-by | 99 | 0 | 142 | needs macOS: real GPUI/AppKit input, semantic shell/extension activation, menus, focus and persistence integration |
| W076 — Render, capture and recover authored content correctly | blocked-by | 1 | 0 | 1 | needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W077 — Keep launch, editing and long playback responsive | blocked-by | 0 | 0 | 1 | needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W078 — Write and compile a custom shader | blocked-by | 0 | 0 | 1 | needs macOS: real Metal pixels, shader/text/colour standards oracles and physical performance evidence |
| W079 — Cutover addition: state-dependent J/K/L shuttle | blocked-by | 1 | 0 | 1 | needs macOS: J/K/L engine transport plus shipped command fallthrough/decline integration |
| W080 — Cutover addition: ripple delete and ripple trim | blocked-by | 2 | 0 | 2 | Actor-backed pro domain edits, saved history and refusal probes run; source-monitor UI, gestures, decoded A/V and engine integration still need macOS |
| W081 — Cutover additions: roll and slide edits | blocked-by | 2 | 0 | 1 | Actor-backed pro domain edits, saved history and refusal probes run; source-monitor UI, gestures, decoded A/V and engine integration still need macOS |
| W082 — Cutover addition: source-monitor three-point editing | blocked-by | 1 | 0 | 1 | Actor-backed pro domain edits, saved history and refusal probes run; source-monitor UI, gestures, decoded A/V and engine integration still need macOS |
| W083 — Cutover addition: persistent linked A/V edits | blocked-by | 1 | 0 | 1 | Actor-backed pro domain edits, saved history and refusal probes run; source-monitor UI, gestures, decoded A/V and engine integration still need macOS |
