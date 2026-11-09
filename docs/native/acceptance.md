# W1 native acceptance harness

`apps/native/crates/acceptance` (`pm-acceptance`) runs headlessly against production
kernel/MCP bodies and an optional native worker. Wave 5 starts at native-engine
`ff42d751f` on 2026-11-01. Linux evidence is **not a native parity or cutover pass**.
Final revalidation is on upstream base `c8f3f5c6e`; its intervening macOS pacing
and v4 evaluation changes add no portable backing for the remaining W1 cases.
Only the acceptance crate and the three lane-owned status documents change.
The shared native lockfile, engine, shell, JS production sources, vendor and spikes
are untouched. No visual or UX changes, placeholders or design choices were made.

## Before and after

Measured **cases**, distinct from Rust tests; both runs use the same base APIs.
Configured runs supply the separately locked native worker and a current-source
compat/remote-DOM bundle. Before is the unchanged base, including its runner bug.
The base worker's lockfile first required a refresh for merged runtime dependency
drift; no base source or assertion was changed for that measurement.

| Evidence class | Before configured pass/fail/blocked | After configured pass/fail/blocked |
|---|---:|---:|
| Original A/B variants | 11 / 0 / 1,333 | 23 / 0 / 1,321 |
| Complete W-group contracts | 0 / 0 / 83 | 0 / 0 / 83 |
| Key integration variants (195 × 9) | 0 / 0 / 1,755 | 0 / 0 / 1,755 |
| Actual shell resolver probes (195 × 7) | 1,365 / 0 / 0 | 1,365 / 0 / 0 |
| Kernel/MCP/pro supporting probes | 30 / 0 / 0 | 31 / 0 / 0 |
| Optional native worker probes | 3 / 1 / 0 | 15 / 0 / 0 |
| Engine/surface adapter doubles | 2 / 0 / 0 | 2 / 0 / 0 |
| Public container copies | 3 / 0 / 0 | 3 / 0 / 0 |
| **Total** | **1,414 / 1 / 3,171** | **1,439 / 0 / 3,159** |

The default portable run moves from **1,411 / 0 / 3,175** (4,586 cases) to
**1,421 / 0 / 3,177** (4,598 cases). Its blocked count includes 18 explicitly
unconfigured worker cases (including three originals). The configured after run has 12 additional supporting
cases, twelve original blockers closed and the existing Stop fixture repaired.
All 83 full workflows remain blocked; a passing subset does not certify a workflow.
Default/configured exit is 0; `--strict` exits 2 for the remaining blockers.

The kernel's merged fractional-rate correction already passes **29.97 exactly**
on this base; the assertion remains unchanged. The configured baseline failure
was **W038.v8-stop**: the old fixture requested a Store isolate through
`RuntimePool::realm(Store(..))`, now deliberately refused by the runtime.
The independent non-Store isolate is now `V4Extension`; an infinite compat loop
can be stopped while that isolate keeps evaluating. Store confinement remains a
separate macOS gate; the fixture does not bypass Store admission.

All **396 legacy specs**, including the frozen **370-spec roster**, retain
`blocked-by` replacement status. There are **0 untriaged** rows and **0 retired**
specs. 389 retain a macOS integration gate; seven have named portable dependencies.
The authoritative triage CSV and generated replacement map retain all provenance.

`remaining-blockers.csv`
reports **every one of the 3,159 remaining case IDs**, with workflow, evidence kind,
macOS flag, backing and precise unverified assertion. Original cases now include
the exact W1 contract; MCP cases name the actual tool/backing and distinguish a
merged adapter from an absent body. Keyboard rows retain their exact chord,
arguments, routing flags and required real-shell context. See W1's
[native status columns](w1-workflows.md#native-execution-status) for all 83 groups.

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

Wave 5 closes twelve more original cases against merged production bodies:

| IDs | Executed contract |
|---|---|
| A504 / W033 | All eight strip operations; exact resulting timing/state/order, atomic malformed-tail refusal and persisted Undo/Redo |
| A508 / W050 | 3D digest compared with independent tagged TypeScript fixtures; read-only document/revision |
| A513 / W031 | Available/loading/failed/cloud/missing states, durable metadata, nested and structured references, inherited locks and pagination; absent registry never fabricates availability |
| A516 / W031 | Folder create/rename/move/delete, asset rename/move/delete, referenced deletion refusal, opaque metadata retention and persisted Undo/Redo; metadata edits never delete source bytes |
| A529 / W033 | Project/layer/property simplification scopes, exact retained keys, before/after counts, protected locked tracks and conservative expression refusal; persisted Undo/Redo |
| A530 / W033 | All nine `edit_video` operations, exact source timing/key rebasing, handles/locks, unchanged assets and persisted Undo/Redo |
| B258 / W057 | The original source-slip unit contract, including unchanged strip/animation, exhausted handles and persisted Undo/Redo; no dedicated mouse slip tool is claimed |
| A539 / W031 | Kernel-evaluated empty ranges, opacity, composition/work bounds, missing/failed/offline/cloud media, source overrun and audio clipping using explicitly supplied synthetic measured peaks |
| A659 / W040 | Actual CommandsAPI register/run/has/list in pinned V8 for v1–3, exact argument/result identity, literal false, exception attribution and owned override/disposal |
| A689 / W040 | Actual ServicesAPI in pinned V8 for v1–3, held-old live object/closure identity, buried and top disposal, predecessor restoration and invalid-name refusal |
| A691 / W040 | Actual EventsAPI in pinned V8 for v1–3, exact payload identity, synchronous nested ordering, exception attribution with following subscriber delivery and owned subscription disposal |
| A550 / W067 | Actual SRT/WebVTT text in composition time: clipping, overlap normalization, Unicode/escaping, invalid-ID refusal, no edits and saved authored cues |

`W057.mcp-refusals` separately covers atomic invalid targets, source handles and
locks. `Fixture::verify_saved` now removes host revision metadata symmetrically;
restored documents no longer produce a false comparison failure. All authored
fields and media/history checks remain intact. The earlier eleven original passes
remain, including these four Wave 2 additions:

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
catalog backing. The current pending set has 41 members; no particular tool or
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

## Actual native backing and compat boundary

`runtime/` is a separately locked native backing executable, depending on the
production runtime, services, promedia, remote-DOM, surface, Store-host and
recorder/raster crates. Its independent manifest keeps optional backing build/download
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
Node dependency is committed. Runtime kernel/storage hooks and the JS recorder host
are explicit synthetic doubles. The service probe has no project edits; the
JS-recorder probe produces no pixels and refuses readback. The separate native
raster probe executes the merged CPU backend and reads exact pixels. Full
PM-to-kernel/canvas bindings, WebKit page rendering and extension corpus
conformance remain open.
Successful owner disposal is awaited on a worker with bounded deadlines.

Wave 5 adds eleven current-source/native production probes through that worker:

| ID | Evidence and remaining limit |
|---|---|
| W040.compat-remote-dom | Actual pinned V8 runs the current remote-DOM shim; stable window/document/input identities, retained refused batch, input state before callback and ordered Unicode text publication. Actual batches are admitted through `pm-remote-dom::Router` with the synthetic event/causal sequence. No WebKit paint, IME or layout oracle. |
| W001.service-entry | Merged service queue deduplicates/retries cold project opens without loss. EntryHook is an explicit double; LaunchServices/app delivery remains open. |
| W012.service-storage | Production service lock, unknown settings, archive tombstones and backups survive reopening. Kernel/app settings adapter remains open. |
| W019.background-tasks | Production promedia pause/cancel settlement fences late worker completion and old tickets. Real codecs and app Background Tasks wiring remain open. |
| W022.proxy-identity | Proxy-only catalog retains original identity, exact 30000/1001 rate and export warning; wrong-hash relink refuses before matching original restoration. Artifact metadata is synthetic; actual probe/decoder verification remains open. |
| W022.media-archive | Production storage refuses referenced purge and verifies retained archive bytes for originals/proxies/optimized media. Host closure inventory, power loss and APFS remain open. |
| W077.render-cache | Production disk cache persists Keep Rendered, refuses kept eviction/different bytes and immediately misses a changed graph. Host must supply a complete render identity and real rendered frames. |
| W040.native-raster | Actual recorder command stream executes in the production CPU rasterizer; exact RGBA pixels, signed regional readback and stale replay refusal. JS/native canvas binding and full Canvas corpus remain open. |
| W040.remote-router | Production router keeps batch/event identities and state during backpressure, permits exact retry and refuses replay. WebKit transport remains open. |
| W032.native-surface | Production six-op surface rejects an invalid tail atomically; removed controls retry cancellation against the original binding. BindingSink is a queue double; GPUI renderer/kernel adapter remains open. |
| W038.store-view | Production Store host preserves profile/extension data identity across view restart, orders seal/stop/restart and rejects old generations. Platform process death and signed confinement remain open. |

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

Install the pinned remote-DOM build dependencies first (`bun install --frozen-lockfile`
in `apps/native/js/remote-dom`), or provide those packages through `NODE_PATH`.
Bun is a build tool only. This runs both locked acceptance suites, formatting and strict crate-local
Clippy, builds the current compat bundle under `work/native-acceptance`, then
writes `report.json` and `remaining-blockers.csv` there. Its final exit is **0**
with the recorded blockers, or **2** when passed `--strict`. It accepts CLI
filters such as `--case W037.compat-services` and `--workflow W033 --strict`.

Default harness and optional backing can also be run separately:

```sh
cd apps/native
cargo test --locked -p pm-acceptance
cargo run --locked -p pm-acceptance -- --workflow W033 --format markdown
cargo run --locked -p pm-acceptance -- --format e2e-csv
cargo run --locked -p pm-acceptance -- --format catalog
cargo test --locked -p pm-acceptance-runtime
```

JSON format `powermove.acceptance`, version 1, includes case kinds/status/reasons,
blockers, macOS flags, workflow aggregates and all legacy mappings. A filtered
workflow explicitly says it is not a full-workflow verdict. Exit codes: 0 means
no failures with blockers permitted; 1 means an assertion failed; `--strict`
returns 2 for blockers if there are no failures; 3 means invalid configuration.

Linux verification used Rust **1.99.1** and Bun **1.3.14**. **75 pm-acceptance
tests pass** (5 catalog, 8 runner/report, 4 runtime-adapter, 58 scenarios/fixtures),
plus **15 native-backing tests**; no ignored tests. The configured run executes
all five actual V8/compat supporting probes, three original v1–3 API cases in
V8 and ten portable production-crate probes.
Formatting and strict `--all-targets --no-deps` Clippy pass for both owned crates.
Rust 1.99 emits three existing dependency deprecations: `fetch_update` in
`pm-runtime-v8` (two) and `pm-promedia` (one). No owned-crate warning is introduced.
No Node or Electron was used in this lane's execution. The already committed
3D fixture expectations come from the owner's tagged-source oracle.

`--format blockers-csv` emits the exact open-case report. To refresh lane-owned
status artifacts after a full public-synthetic configured run:

```sh
python3 apps/native/crates/acceptance/scripts/refresh-reports.py work/native-acceptance/report.json
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance -- --format e2e-csv > apps/native/crates/acceptance/data/e2e-map.csv
```

The refresh script refuses filtered reports and user-supplied corpus cases.
It edits only e2e status columns, the W1 status table and the blocker snapshot;
it retains every original contract and provenance field. Regression tests reject
report/map/triage/W1 drift. Mock backing tests validate report metadata only;
the configured verification command executes every actual worker assertion.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Keep all legacy specs and exact source/ID links; explicit triage cannot silently promote or drop a spec | N4/N7 and prune rule: no lost requirement or removed consumer |
| Keep original cases, supporting probes, full workflow gates, native runtime probes and doubles distinguishable | N1/N3: a partial domain or recorder test cannot establish UI/pixel acceptance |
| Derive pending tools from `catalog::IMPLEMENTED`, validating inputs first | N3/N7: automation checks real capabilities as implementations land |
| Use real actor commands, exact values, atomic refusal and persisted Undo/Redo | N1/N2/N3/N4: stronger pro editing without altered authored intent or lost recovery |
| Separately locked optional V8 backing, current-source test bundle and explicit missing-capability blockers | N7/N8 and ownership: headless/drivable, no Node in native execution, no shared-lock changes |
| Preserve v1–3 live service identity and recorder pins in their V8 realm; leave WebKit to remote DOM | N4/N6/N8 and P0 decision 5; no compatibility exclusion or browser-quirk adoption |
| Retain W071's exact 29.97 assertion after the merged fix; repair the Stop fixture using a separate v4 isolate | N1/N3/N8: exact authored timing and current runtime placement, without weakening Store isolation |
| Execute native CPU raster/router/surface/services on Linux, retain macOS input/page/codec/presentation gates | N1/N3/N6/N7: test available production backings without overstating their integration |
| Keep A511/A512 blocked for frozen-source reconciliation; retain their requirements unchanged | N3/N4: do not silently rewrite an inventory claim or fabricate a missing tool |
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

Precise portable gaps outside ownership:

- **A506:** kernel container-copy probes preserve embedded bytes, but `Automation`
  has no public file-open/adopt ingress or durable-media byte admission for an MCP
  packing fixture. The MCP owner must expose the real ingress; then execute Save
  against resident media and verify every packed byte/history record. Metadata-only
  imports are insufficient. No fake packed bytes or source registry was introduced.
- **A511/A512:** W1 names a memory index and `project_memory`, while tag
  `07ed5fa17`'s `main/agent-tools/spec.ts` defines ordinary composition inspection
  and its frozen 67-tool union has no `project_memory`. Native inspection also
  has no memory index. `project-memory-contract` records the discrepancy; the
  inventory/contract owner must reconcile it. These original requirements remain
  unchanged and blocked, not silently reclassified as passes.
- **W041/W067:** no native transcription/model-download/inference service crate
  is merged on this base. Kernel caption Add/Edit commands explicitly return
  `NotImplemented`; MCP `transcribe_media`/`generate_captions` remain unavailable.
  Real caption text export is now covered independently by A550.
- **W035/W036:** Store-host policy/identity/install models exist; the remote Store
  catalog, release tree, account/publish APIs and app/kernel/runtime wiring remain
  missing. A local extension registry is not the Store catalog.
- **Engine/media/export:** six MCP host-backed adapters exist, but the Linux
  acceptance worker has no real decoder, renderer, audio analyzer or ExportChild
  codec worker. The full source pixels/labelled sheets/loudness/encoded output
  cases remain blocked; test doubles are supporting evidence only.

No shared lockfile change or unowned code edit was made. Full media/migration
closure, agent/tool adapters and the macOS integration above remain open.

## Current configured workflow report

All 83 workflow verdicts and pass/fail/blocked status columns are maintained in
[W1](w1-workflows.md#native-execution-status). The complete per-case open work is
`remaining-blockers.csv`.

## Parity follow-up — 2026-11-04

This dated section supersedes the Wave 5 counts above. Branch
`feat/na-cloud-accept2`, integration base `ed5482622f41c533199a724dcb8c8738dd4dcc93`.
Only the acceptance subtree and the two lane-owned Markdown documents change.
The shared triage, original workflows/keymap, 396 specs and frozen 370 roster
remain intact. There are no visual changes or new `DESIGN-TODO(opus)` notes.

The configured Linux run executes **4,603 assertions: 1,466 pass / 1 fail /
3,136 blocked**. It closes **23 original blockers** against existing production
backing. The default run is **1,423 / 1 / 3,179**; its additional 43 blockers
explicitly require the optional native worker/bundle. All **83 full workflow
contracts remain blocked**. W061's aggregate is fail because its supporting
curve oracle fails; the other 82 aggregates are blocked. All 396 complete spec
replacements remain blocked. A component/unit pass never certifies an app flow.

| Newly executed original IDs | Real backing and scope |
|---|---|
| A510 | `McpSession` → `pm-mcp-backends::BuiltinSources`: isolated source staging, rewritten user-fork manifest, exact pristine merge base, inert kernel registrations, no overwrite, and retained commit/cancel archives. Inputs are synthetic shipped-resource trees; no installed extension is discovered. |
| A532 | Actual MCP effect registration validator: valid definition, exact diagnostics/precedence, duplicate keys, UTF-16 size boundary, unchanged document/revision. No shader compilation/pixels claimed. |
| A660–A663, A666, A668–A669, A678 | Current-source v1–3 registry APIs in actual native V8: keybindings, effects/transitions/layers, theme data/events, synchronous/asynchronous menus, status callbacks and Inspector sections. Exact identity, override/restoration, refused definitions and owned disposal execute. |
| A690 | Actual `pm-compat-runner` → native V8 StorageAPI get/set/delete, exact nested values, owner isolation and retained state across v1–3 activation/reload. Profile/disk restart remains a host gate. |
| A693, A695–A696, A698 | Activation-frozen variables; all five menu contribution locations; conditional sections at all three placements and retained mount cleanup; both status sides/title/computed callbacks. These certify API data/lifecycle, without painting or native input. |
| B568–B570, B610–B614 | Actual current compat control binding/gesture code in native V8, with the explicit recording API sinks used by the tagged unit contracts. Exact live time, command mapping, work-area clamps, begin/write/commit/cancel, one-shot/default origin, local and legacy-history modes. These are unit dispatch contracts, not kernel transactions or native drags. |

The [closure ledger](../../apps/native/crates/acceptance/data/closures-2026-11-04.csv)
retains every resolved ID and assertion. Original pass count rises from 23 to 46.
Two supporting curve checks and three connected compat probes bring the inventory
from 4,598 to 4,603; 1,365 resolver probes, 18 worker probes, two declared adapter doubles
and three public container copies retain their limits.

Three new supporting probes use the existing real `pm-compat-runner` binding:
`W040.compat-kernel-history` checks native authored writes, callback return/throw
identity, rollback, history.cancel retention, external closures and Undo/Redo;
`W040.compat-kernel-edits` checks typed apply/begin/dispatch/commit/cancel,
atomic malformed-tail refusal and Undo/Redo; `W040.compat-native-pixels` checks
real JS recorder → native CPU fill/drawImage/readback after source resize/disposal.
No recorder or kernel test double supplies those results. Their untested complete
API members and Canvas/engine/WebKit requirements stay blocked. The optional
worker test suite now bundles current production sources and executes every
bundle-backed case, as well as the parent configured report.

### Measured defect remains a failure

`W061.frozen-set-easing` now executes the existing frozen e2e Glide oracle in the
main acceptance report too. Sequential `set_property` at 0/2/3 seconds, with
10/110/−40 values and Glide `[0.16, 0.84, 0.24, 1]`, evaluates at 1 second to
**90.42094586353606**, expected **104.94929544957891**. The paired
`W061.frozen-replace-easing` passes. Both runners share the same independent f64
curve inversion; no tolerance, key, handle or expected value changed. The owning
kernel/MCP lane must preserve both easing handles in sequential key creation.
The known-defect Rust guard is harness verification, never a product pass.

The complete configured run exits **1**, with or without `--strict`, because a
measured failure takes precedence over blockers. A strict W040 run exits **2**
for its remaining blockers; passing filtered A532 exits **0**. Blockers remain
in JSON/CSV when a failure determines the process exit.

### Exact remaining backing and retained provenance

`remaining-blockers.csv`
contains all 3,136 outstanding assertion IDs and exact unverified contracts.
`blocker-groups.csv`
and JSON `blocker_groups` partition them once into **151 backing IDs**. The
[83-workflow backing inventory](../../apps/native/crates/acceptance/data/backings.json)
names each remaining coordinator/adapter/oracle. MCP groups retain exact tool
names and `catalog::backing`; compat groups name detached API backends instead
of grouping all API members as an unspecified runtime. The 1,755 physical-key
variants still need the same real shell dispatcher. No production backing
availability is inferred from a flag or an adjacent passing component.

Dependencies other lanes can provide:

- **Engine/media/export:** `pm-engine-service` links Metal/AppKit and cannot be
  used as the real Headless adapter on Linux. Supply actual renderer/resource,
  original-byte/media-probe/decoder/waveform and ExportChild bindings, then execute
  the retained frame/timestamp/audio oracles. Existing metadata/cache/archive
  probes do not establish decoded media or rendered delivery.
- **Kernel/MCP:** repair the measured Glide defect; file-open/adopt and resident
  media ingress are still needed for A506 packed-media Save; unfinished tool
  bodies remain grouped by exact tool name. Metadata alone cannot seed bytes.
- **Compat:** native V8/kernel history closures and typed edit transactions now
  run, as does JS recorder → native CPU fill/drawImage/readback with immutable
  source pins. Complete `history.selection` dispatch and `edit.mutateStart` /
  `edit.mutateFinish` / `edit.mutateAbort` lease adapters are still missing.
  Animation/model/space3d queries, profile/disk storage lifecycle, full Canvas
  text/paint/encoding, engine resources and native host actions remain.
  Execute WebKit/remote-DOM and all unchanged extension scenarios;
  registry and control dispatch unit checks do not replace them.
- **Application:** native windows/close/open coordination, panels, key dispatch,
  menus, clipboard, IME/AX and editing controls; agent provider/thread/approval,
  Store transport/install/publish and speech-model/inference services remain
  separately named. Physical performance and signed helper/updater gates need Mac.

The requested `bun run --cwd apps/desktop test:e2e:ledger` was attempted on this
container and exits 1: its available `playwright` command reports `unknown command 'test'`. The configured Playwright test runner is not usable here. No Electron ledger
or GUI case is reported as executed or passing. The frozen source/declaration
ledger and retained Rust equivalents run independently above.

The standalone frozen e2e runner initially failed five inventory tests because
its required declaration annotations were absent from the merged shared triage.
The repair stores those exact links in the owned
[e2e-declarations.csv](../../apps/native/crates/acceptance/data/e2e-declarations.csv),
checks all **614** declaration IDs/file/line/hashes/titles/workflows/native-check
links against the frozen ledger, and leaves shared triage untouched. No declaration
or assertion was dropped. A fresh read-only TypeScript 5.9.3 extraction against
the actual tag also matches all 290 source files, 289 specs, 614 declarations
and 80 parameterized declarations. Its actual report remains **29 passed / 1 failed checks**,
with **3 complete headless declarations passed / 1 failed / 610 blocked**.

The refresh script now writes only owned CSV snapshots and appends the dated
[W1 status](w1-workflows.md#native-execution-status--2026-11-04). Identical replay
is a no-op; it refuses rewriting differing dated shared evidence. Historical
triage links remain a subset of the current e2e map; every original provenance
field remains unchanged. Filtered/external-corpus inputs, changed rosters,
duplicate IDs and incomplete blocker partitions are refused.

### Project memory source reconciliation

Read-only checks of frozen commit `07ed5fa17ee640659cb075f9cad1af2095011d60`
find **no `project_memory` reference** under either `apps/desktop/src/main/agent-tools`
or `apps/desktop/src/renderer/src`. Its 67-tool union has no such tool and
`get_project_state` has no memory-index description. A511/A512 and the older
appendix sentence claiming that this already ships misattribute later behavior
to the tag. This is a source-inventory error, not evidence of a lost shipped tool.

N3/N4 select retaining both original IDs/wording as explicit additional native
requirements, with blocker **`memory-post-tag-requirement`**: implement kernel
memory storage/index and admit the tool through an explicit versioned API change.
Do not insert a fictitious 68th member into the frozen union or silently drop
memory. Regression checks keep the frozen union and both pending requirements.

### Decisions and verification

| Decision | Selecting rule |
|---|---|
| Execute existing native V8/registry/file/kernel backing by exact assertion ID; distinguish original unit contracts from application gates | N1/N3/N6/N7: authored behavior and evidence scope stay exact |
| Share the unchanged independent Glide oracle and expose its failure in both reports | N3: correctness over imitation; never turn a defect into a pass or blocker |
| Keep frozen rosters/claims and append W1 evidence; move generated declaration links into owned output | N4/N7 and ownership/prune: retain all consumers and provenance without editing shared triage |
| Keep runtime dependencies separately locked; refresh only graph edges/new package dependencies, including the required smallvec pin | N7/N8 and lockfile ownership: no Node/Chromium in native execution or bulk upgrades |
| Canonicalize disposable temp roots; run actors, child waits and V8 on workers | N4/N8 and macOS-safe test rule: accept system temp aliases and keep main-thread waits absent |

Final Linux checks: **79 acceptance + 17 optional-worker + 36 retained-e2e Rust
tests, plus 4 Python report tests; all pass, none ignored**. Formatting and
crate-local Clippy `-D warnings` pass for all three manifests. The companion's
current-source bundle test is serialized with its service-storage drop/reopen
fixture: a bundler child can briefly inherit an advisory lock during fork/exec.
The exact refusal/reopen assertions remain unchanged, with no retry/tolerance. The configured
acceptance run and frozen e2e report deliberately exit 1 for the measured product
defect above; strict W040 exits 2. Every Cargo build/test/run/Clippy uses `--locked`.

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
apps/native/crates/acceptance/scripts/verify.sh --strict
cargo run --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance --bin pm-e2e-triage
python3 apps/native/crates/acceptance/scripts/refresh-reports.py work/native-acceptance/report.json
```

The first two commands deliberately return 1 while their real reports retain the
failure and blockers. The strict filtered W040 command uses the same native
worker and current bundle as `verify.sh`, with `--workflow W040 --strict`.
Reverification on macOS still needs the S1 archive plus matching bindings and
`--features macos-sandbox`, actual GPUI/AppKit/IME/AX/WebKit execution, Metal
frames, codecs/source identity, audio devices, signed confinement/migration and
physical display/performance gates. Linux evidence certifies none of those.

### Final integration replay — 2026-11-04

Merged `origin/feat/native-engine` at
`ce36cb6fcc26342cbc51ca577eb8cc1ad596de85` without conflicts. Its new Opus surface
specifications do not add portable production backing. Replayed the locked
verification on the merged branch: the same 79/17/36 Rust tests and four Python
tests pass; formatting/strict Clippy pass; configured acceptance retains
1,466 pass / 1 measured Glide failure / 3,136 blockers. No new macOS claim.

## Main reference change — 2026-11-04

The owner's new behavior target is main commit
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. This section supersedes the earlier
v1.2.0 parity target and project-memory/version-approval interpretation. Both
`pm-acceptance` and `pm-e2e-triage` now default to this exact main reference.
`--reference v1.2.0` explicitly selects the retained legacy evidence. Projects
and v1–3 extensions from v1.2.0 remain mandatory compatibility inputs; their
passing assertions are never added to current-main parity counts.

Source discovery executes only pure public declarations from the pinned Git
object, using development-only TypeScript 5.9.3/Bun. It freezes the complete
**68-tool** union, descriptions/input schemas, allowlists/instructions, command
fields/easing, **198 binding definitions** and the hashes of their 13 source
files. Main e2e extraction retains **315 source files / 314 specs / 719 test
declarations / 81 parameterized declarations**, including original parameters,
guards, titles and hashes. The regex-only fixture is still distinguished from
specs. Neither extractor launches Electron or touches installed profiles.

The active report has **4,344 cases: 1,651 pass / 32 fail / 2,661 blocked**,
partitioning blockers into **229 exact backing groups**. These are assertion
counts, not distinct product-defect counts. Passes distinguish schema identity,
actual shell resolver behavior and real kernel/MCP supporting assertions from
complete tool/application acceptance. All **83 full workflow contracts remain
blocked**. Of the 719 current main declarations, the unchanged strict Glide
case fails and the other **718 remain blocked**. All complete spec replacements
remain blocked. Strict mode exits **1** for measured failures, preserving all
blockers; a filtered unresolved A-row exits **2**, and a filtered passing schema
assertion exits **0**.

| Failing assertions | Measured result / next backing |
|---|---|
| 24 binding definition/routing variants | Native shell lacks bare backtick → maximizePanel; viewer Tab → composition.flowchart (`inFields`); timeline M → toggleLayerStrips at priority 90. These are three missing definitions across eight checks each. Native/AppKit command delivery remains a separate blocked gate for all 198 × 9 variants. |
| 3 tool catalog checks + union identity | Native still exposes 67 tools. Main adds project_memory; get_project_state now promises its memory index; export_video adds jobId continuation and its running/progress result contract. Exact JSON paths are reported; the catalog is not silently relabeled current. |
| 2 actual memory calls | Main-valid project_memory list returns Unknown tool; get_project_state lacks the empty memory index required by pinned main. Source proves these are now required A07/P02 behavior. Full mutation/non-Undo/autosave semantics remain delta-data/kernel/MCP work outside acceptance ownership. |
| 2 Glide assertions | Current main's exact unchanged declaration hash and operation parameters reproduce the existing sequential set_property defect: 90.42094586353606 versus 104.94929544957891 at 1 second. The paired replace_keyframes oracle passes. The probe and declaration each retain the failure; no tolerance changed. |

`remaining-blockers.csv`,
[failures.csv](../../apps/native/crates/acceptance/data/failures.csv) and
`blocker-groups.csv`
now describe active main. The e2e union map retains **all 396 legacy rows and the
frozen 370 roster**, adding the main-only scrub-framebuffer-reuse spec for **397
rows**. Existing acceptance links survive in `legacy_acceptance_ids`; every main
declaration has its own reference-qualified ID. The old snapshots are archived
as `legacy-v1.2.0-{remaining-blockers,e2e-map,blocker-groups}.csv`, and both frozen
source/declaration ledgers remain. No legacy test, file, extension or consumer was
deleted. Shared e2e-triage.csv remains read-only.

### Delta scope and owner boundaries

[main-deltas.json](../../apps/native/crates/acceptance/data/main-deltas.json)
retains all **57** audited rows and their routes. A01–A10 have individual current
acceptance gates and immutable source hashes: provider discovery; client setup;
Claude Auto classification; rich markdown/local media grants; visual replies;
headless tools; project memory; extension rebase/appearance; publication and
panel-generation fences; provider review/consent/error identity. These gates
remain blocked until their complete production backing and assertions execute.
Existing kernel/MCP primitives run as narrower supporting evidence where main's
public contract is unchanged. Old 3D/effect fixtures are not promoted into new
main 3D/versioned-effect acceptance.

P rows route to **delta-data**, E rows to **delta-fx**, U rows to the **later UI
lane**; this lane adds requirements/evidence, not their production implementation.
The owner reference change itself reconciles A511/A512/A07: no extra permission
request is required to target the new catalog. The native catalog/body/index
still need their owning implementation. Missing bindings require the shell/UI
owner. No writes to real agent-client settings occur; Claude desktop setup still
requires the owner's explicit consent. No reverse-migration or archive-Restore UI
requirement is introduced. W072's current gate preserves archived web-export
preferences and verifies that the retired action stays absent; it does not
request reinstating standalone web delivery.

The configured legacy compatibility census stays **1,466 pass / 1 fail / 3,136
blocked**, separately nested under `compatibility` with the tagged reference.
Its original 23 closures and all file/extension checks remain available by ID.
The default unconfigured legacy subreport remains 1,423 / 1 / 3,179. These are
historical/compatibility requirements, not the new main baseline.

### Reproduction and decisions

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
apps/native/crates/acceptance/scripts/verify.sh --strict
python3 apps/native/crates/acceptance/scripts/refresh-main-reports.py work/native-acceptance/report.json
# Read-only oracle checks, with TypeScript 5.9.3 available through NODE_PATH:
bun apps/native/crates/acceptance/scripts/main-catalog.cjs
python3 apps/native/crates/acceptance/scripts/main-deltas.py
bun apps/native/crates/acceptance/src/e2e/extract.cjs
bun apps/native/crates/acceptance/src/e2e/extract.cjs --reference=v1.2.0
```

The first command deliberately exits 1 for actual main failures, after running
both configured current and retained compatibility assertions. Both CLIs accept
`--reference main|v1.2.0`; legacy IDs keep selecting their original compatibility scope automatically when
no explicit reference is supplied, preserving existing ID-based consumers. Their
results identify the tagged reference/purpose, never a current-main pass. The standalone e2e CLI's
`--check` uses current `MAIN.*` IDs by default. Old reference-specific extraction
is explicit; the default extractor now checks main. Pure Rust suites require no
TypeScript installation; the optional V8 companion still needs its documented
Bun/remote-DOM build prerequisites and the matching S1 artifact on macOS.

| Decision | Selecting rule |
|---|---|
| Default every behavior-facing CLI and extractor to pinned main; label v1.2.0 explicitly as compatibility | Owner decision, N3/N4/N6: current intent plus every old input retained |
| Keep every declaration, parameter/guard, frozen roster and resolved legacy assertion; archive old snapshots before changing active filenames | N4/N7 and prune: no lost requirement, no removed consumer |
| Fail exact schemas, valid memory calls, missing native binding definitions and the unchanged authored curve | N1/N3/N7: execute real backing; no optimistic flags or reclassification of failure as a blocker |
| Separate schema/routing/supporting evidence, complete bodies/workflows and legacy results | N3: no old or partial pass certifies current main behavior |
| Route data/effect/UI production work to its owners and make no visual/UX changes | Ownership and Opus design rule; no new DESIGN-TODO(opus) |

Linux verification: **85 acceptance + 17 optional-worker + 37 retained-e2e Rust
tests and 8 Python report tests pass**, with no ignored tests. Formatting and
crate-local Clippy `-D warnings` pass across the three manifests. Main and legacy
oracle regeneration checks match their pinned sources. No native workspace or
engine lockfile change is needed: no new package dependency was introduced.
Mac verification remains required for actual menus/maximize/flowchart/input,
AppKit/IME/AX/WebKit panels, complete current extension scenarios, new 3D/effect
pixels, media/export/audio, client setup consent, signed boundaries and physical
performance. Linux proves none of those. The previously recorded unusable Linux
Playwright CLI still prevents an Electron ledger execution; no GUI pass is claimed.


Current main reports name the mandatory old-file/extension checks in
`compatibility_required_ids`. Those checks affect strict/failure exits alongside
current main. Retained old window/layout/visual gates remain historical evidence;
they do not reinstate v1.2.0 as the UI target. All old IDs, cases and provenance
remain inspectable. This distinction follows the owner's explicit current-main
reference plus the required old-project/extension compatibility boundary.

### Main-reference integration replay — 2026-11-04

Merged integration base `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9` without
conflicts, including the owner's newer Opus specifications. After the final
fetch/merge, all **85 + 17 + 37 Rust tests and 8 Python tests** pass; formatting
and strict crate-local Clippy pass. Configured current-main results reproduce
**1,651 / 32 / 2,661**, separately retaining legacy **1,466 / 1 / 3,136**.
Strict exit 1 deliberately preserves the measured failures. All 32 selected
legacy file/extension input probes pass in the configured run; that subset does
not certify the still-blocked complete application/compatibility workflows.
Source oracle/delta-route checks reproduce both pinned ledgers. No Mac evidence
or visual acceptance is claimed. Source SHA-256 metadata is structured to avoid
API-key-pattern false positives without changing digests or scanner rules.

### Integrated Mac repairs — 2026-11-05

The unchanged sequential Glide oracle now passes, as do the native Backtick and
M binding metadata/resolver checks. The measured main result is 1,672 passed,
10 failed and 2,662 blocked. The ten failures are the composition-flowchart
binding's eight checks and the two Studio catalog identity checks. Legacy has
no remaining authored-curve failure. Complete-controller/window evidence stays
blocked. See [int2-fixes.md](int2-fixes.md) for source proof, required-feature
counter changes and final suite results; the older dated tables above retain
their original context.

## Blocker census and connected portable probes — 2026-11-05

Lane: `feat/na-cloud-accept3`, starting integration base
`5bcd425ddc158cbd891ee4b515cf9993fad29a66`. Current behavior stays pinned to
main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`; the original v1.2.0 file and
extension requirements remain separate. No shared W1/table/CSV was rewritten.

The immutable [starting census](../../apps/native/crates/acceptance/data/blocked-baseline-2026-11-05.csv)
retains **all 2,662 main + 3,136 legacy blocked rows**, with original IDs
and backings. The reviewed [reason registry](../../apps/native/crates/acceptance/data/blocker-reasons-2026-11-05.json)
assigns exactly one primary reason to each backing; the anonymized
row-by-row table (`census.py --csv`) contains all **5,798** rows, reference, ID, original backing, single reason,
dependency, current verdict and evidence. Nothing is retired or removed from
the denominator when it passes or fails. The table below groups those exact rows
by backing, without names, private extensions, profiles or user projects.

A primary reason is the next unsatisfied dependency, not a claim that every later
prerequisite is ready. `macos-screen` requires actual native input/window/focus,
IME/AX or panel presentation; `macos-platform` requires GPU/device/signed OS or
paired physical evidence. `unfinished-lane` identifies an incomplete production
adapter/controller/service and retains its exact source requirement. A registry,
trait, model or stub cannot close it. `retained-owner-scope` keeps superseded
historical/browser-host/removal requirements visible under recorded owner scope.
`portable-probe` is a reviewed runnable candidate: it must execute and report its
actual result. No row is currently **credential-only**: the auth-dependent full
provider/Store rows still lack complete application/service integration, which
is their earlier primary blocker. No credential was requested or read.

| Original primary reason | Main rows | Legacy rows | Total |
|---|---:|---:|---:|
| macos-screen | 2,116 | 2,522 | 4,638 |
| macos-platform | 48 | 24 | 72 |
| unfinished-lane | 478 | 578 | 1,056 |
| retained-owner-scope | 1 | 11 | 12 |
| portable-probe | 19 | 1 | 20 |
| **All starting blockers** | **2,662** | **3,136** | **5,798** |

### Real execution and measured defects

Eighteen main tool-body rows now pass through production `McpSession`/`Automation`
and the real native kernel or file-backed source service: project list/create/
open/manage, start/finish/rollback sessions, timeline/selection, command/strip/
video edits, caption sidecars, media management, saved panel-layout
inspection, source-backed 3D scene inspection, effect validation
and built-in source forking. Exact current catalog identity, actual session/
planning/app/closed-connection refusal precedence, stable source identity,
revision refresh, atomic refusal and existing authored-state/history assertions
are exercised. The fork body uses the separately locked worker's existing real
`BuiltinSources` adapter; without that worker it stays blocked. No fake renderer,
network transport or success-shaped service was added. Full app workflows and
all 719 main e2e declarations remain blocked; these tool-body passes do not
certify their windows, gestures or pixels.

The two other runnable starting rows (`MAIN.MCP.save_project.behavior`, legacy
`A506`) are now **failures**, exposing one product defect: opening a packed file,
editing it and calling `save_project` saves the working file in place. Pinned
main promises a separate uniquely named file in the host export folder. The
probe first verifies exact referenced **and unreachable** embedded bytes,
media IDs/types/revisions, authored state and reopened Undo/Redo; it then fails
the unchanged export-destination/uniqueness contract. This needs the MCP/kernel
owner's implementation outside this lane, not a changed acceptance expectation.

A further actual file-ingress probe exposes a second product defect in both
reports: the kernel accepts a valid PMV4 with an authored lone UTF-16 code unit,
but `Automation::open_file` panics in `pm-mcp::json_value` converting the model
through serde JSON. The unchanged synthetic source bytes survive the refusal.
`MAIN.file.utf16-ingress` and `W007.mcp-utf16-ingress` remain **fail**, never a skip
or blocker. The owned fixtures are public/synthetic and vendored; no external
corpus or codec build is needed. Fixture generation is deterministic on Python
3.9+. No tolerance, expected pixel, authored handle or capability was relaxed.

Configured current results: **1,700 pass / 2 fail / 2,643 blocked** (4,345 rows).
Configured retained compatibility: **1,467 pass / 2 fail / 3,135 blocked**
(4,604 rows). There are two product defects, represented separately in the two
reference scopes. Relative to the requested starting population, **18 real main
passes + 2 explicit failed assertions** replace 20 blockers. The additional
UTF-16 case adds one failure per reference, not another closed starting row.
No workflow, screen, signed confinement or physical gate is promoted.

One command executes the locked prerequisites/probes and prints all
pass/fail/blocked counts per reason:

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
apps/native/crates/acceptance/scripts/verify.sh --strict
```

Its expected acceptance exit is **1** for the two real defects. The suites test
that failures stay failures; a green harness suite does not mean green product
acceptance. To inspect an existing configured report without rebuilding:

```sh
python3 apps/native/crates/acceptance/scripts/census.py work/native-acceptance/report.json --strict
```

The reporter rejects missing/duplicate baseline IDs, filtered or mixed-reference
reports, unknown verdicts, inconsistent status/backing and absent evidence. It
prints extra unconfigured-worker rows explicitly and preserves fail over blocked
in strict exits. The reason table is immutable as a starting census; current
verdicts are reconciled by ID from each fresh real execution. Active main CSVs
are refreshed only inside the owned acceptance subtree; the original census
and legacy source/declaration ledgers remain available.

### Decisions and remaining dependencies

| Decision | Selecting rule |
|---|---|
| Keep every starting ID, exact original requirement and separate reference; assign one primary blocker and preserve subsequent prerequisites | N3/N4/N6/N7: authored intent, old inputs and inspectable evidence cannot disappear |
| Execute only connected MCP/kernel/source-service bodies; leave unavailable full controllers, providers, byte admission, runtime query arms and engine bindings blocked | N1/N3/N7/N8: real services, no stub passes or main-thread JS waits |
| Preserve unique-export and UTF-16 ingress as measured failures; request the production fixes from their owners | N1/N3/N4 and lane ownership: correct packing and no lost authored data take priority over a green count |
| Keep all full native workflows/e2e declarations and physical/signed gates open | N1/N2/N3: Linux tool success cannot prove Mac application behavior |
| Add no visual/UX implementation, tokens, wording or interaction | Opus ownership: no new `DESIGN-TODO(opus)` |

Outside this lane: fix unique packed export plus lossless MCP model/UTF-16
transport; connect the remaining real media import/replace/registry/transcription,
Store HTTP/install/publish, provider/thread/agent coordinator, extension
promotion and compat query/selection/mutation/UI-state/workspace adapters.
For example, compat `history.selection` and structural-mutation arms are absent
from the real `KernelAdapter`; factory `KF` needs the unattached ease query;
`UtilAPI` still needs round/timecode/color adapters. Passing adjacent facade tests
cannot promote these compound member contracts. Main project-memory writes
still return unavailable for non-Undo durable admission; full get_project_state
still needs live media/font-layout resources, not its current null/unavailable
projections. No implementation outside the two owned paths was changed.

Mac verification must exercise actual keyboard/menu/Tab-flowchart delivery,
GPUI/AppKit/IME/AX/WebKit controls and lifecycle, engine frames/source identity,
codec/export/audio and real presentation, signed Store/Keychain/updater boundaries,
and both paired physical-performance rigs. Opus must review actual visual
output. Linux evidence certifies none of these. No Mac-only prerequisite was
silently skipped to manufacture a passing row.

### Complete starting-row classification by backing

Prefixes remain intact to distinguish original source/owner routing. Each table
entry links through the row table above to every individual ID and its verdict.
The two count columns partition exactly the requested 2,662/3,136 rows.

| Exact original backing | Main | Legacy | Single primary reason | Dependency |
|---|---:|---:|---|---|
| `agent-approval-question-stream-host` | 0 | 7 | unfinished-lane | agent host |
| `agent-project-tool-integration` | 0 | 13 | unfinished-lane | agent host |
| `agent-provider-auth-stream-host` | 0 | 32 | unfinished-lane | agent host |
| `agent-run-thread-coordinator` | 0 | 16 | unfinished-lane | agent host |
| `agent-visual-reply-publication` | 0 | 10 | unfinished-lane | agent host |
| `cloud-media-download-coordinator` | 0 | 4 | unfinished-lane | media/engine |
| `compat-live-kernel-render-page-binding` | 0 | 7 | unfinished-lane | compat runtime |
| `compat:animation-evaluator` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:asset-byte-import` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:complete-canvas-engine-resources` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:ease-query-backend` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:extension-lifecycle` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:group-query-edit` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:history-selection-adapter` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:legacy-util-backend` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:media-registry` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:model-constructors` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:native-drag-drop` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:native-host-actions` | 0 | 3 | unfinished-lane | compat runtime |
| `compat:palette-host` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:panel-render-mount` | 0 | 3 | unfinished-lane | compat runtime |
| `compat:project-kernel` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:remote-dom-controls` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:scene3d-backend` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:selection-kernel` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:space3d-backend` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:store-namespaced-events` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:structural-mutation-lease-adapter` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:transport-engine` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:ui-state-kernel` | 0 | 1 | unfinished-lane | compat runtime |
| `compat:workspace-kernel` | 0 | 1 | unfinished-lane | compat runtime |
| `decoded-source-retiming-integration` | 0 | 3 | unfinished-lane | media/engine |
| `decoded-source-slip-integration` | 0 | 1 | unfinished-lane | media/engine |
| `delta-data:archived-web-export-preferences` | 1 | 0 | unfinished-lane | media/engine |
| `delta-data:durable-media-import-coordinator` | 21 | 0 | unfinished-lane | media/engine |
| `delta-data:durable-media-relink-replacement` | 8 | 0 | unfinished-lane | media/engine |
| `delta-data:image-sequence-import-coordinator` | 3 | 0 | unfinished-lane | application/services |
| `delta-data:native-async-project-open` | 11 | 0 | unfinished-lane | application/services |
| `delta-data:native-portable-document-flow` | 26 | 0 | unfinished-lane | application/services |
| `delta-data:project-memory-index-and-tool` | 1 | 0 | unfinished-lane | kernel/MCP |
| `delta-fx:metal-authored-frame-oracles` | 3 | 0 | macos-platform | macOS engine/platform |
| `delta-fx:native-audio-mixer-device-meters` | 3 | 0 | unfinished-lane | media/engine |
| `delta-fx:native-effects-matte-controls` | 82 | 0 | unfinished-lane | media/engine |
| `delta-fx:native-embedded-audio-routing` | 3 | 0 | unfinished-lane | media/engine |
| `delta-fx:native-export-child-codec-integration` | 48 | 0 | unfinished-lane | media/engine |
| `delta-fx:native-scene3d-editor` | 12 | 0 | unfinished-lane | media/engine |
| `delta-fx:native-shader-compiler-editor` | 1 | 0 | unfinished-lane | media/engine |
| `durable-media-import-coordinator` | 0 | 25 | unfinished-lane | media/engine |
| `durable-media-relink-replacement` | 0 | 13 | unfinished-lane | media/engine |
| `encoded-fractional-rate-oracle` | 0 | 1 | unfinished-lane | application/services |
| `engine-media-registry-integration` | 0 | 7 | unfinished-lane | media/engine |
| `extension-settings-storage-host` | 0 | 3 | unfinished-lane | application/services |
| `extension-source-promotion-activation` | 0 | 23 | unfinished-lane | media/engine |
| `external-mcp-application-integration` | 0 | 7 | unfinished-lane | agent host |
| `image-sequence-import-coordinator` | 0 | 9 | unfinished-lane | application/services |
| `main:agent-approval-question-stream-host` | 1 | 0 | unfinished-lane | agent host |
| `main:agent-client-config-registration` | 1 | 0 | unfinished-lane | agent host |
| `main:agent-project-tool-integration` | 1 | 0 | unfinished-lane | agent host |
| `main:agent-provider-auth-stream-host` | 1 | 0 | unfinished-lane | agent host |
| `main:agent-run-thread-coordinator` | 1 | 0 | unfinished-lane | agent host |
| `main:agent-visual-reply-publication` | 1 | 0 | unfinished-lane | agent host |
| `main:claude-auto-permission-classifier` | 1 | 0 | unfinished-lane | agent host |
| `main:cloud-media-download-coordinator` | 1 | 0 | unfinished-lane | media/engine |
| `main:compat-live-kernel-render-page-binding` | 1 | 0 | unfinished-lane | compat runtime |
| `main:decoded-source-retiming-integration` | 1 | 0 | unfinished-lane | media/engine |
| `main:decoded-source-slip-integration` | 1 | 0 | unfinished-lane | media/engine |
| `main:durable-media-import-coordinator` | 1 | 0 | unfinished-lane | media/engine |
| `main:durable-media-relink-replacement` | 1 | 0 | unfinished-lane | media/engine |
| `main:encoded-fractional-rate-oracle` | 1 | 0 | unfinished-lane | application/services |
| `main:engine-media-registry-integration` | 1 | 0 | unfinished-lane | media/engine |
| `main:extension-rebase-appearance` | 1 | 0 | unfinished-lane | application/services |
| `main:extension-settings-storage-host` | 1 | 0 | unfinished-lane | application/services |
| `main:extension-source-promotion-activation` | 1 | 0 | unfinished-lane | media/engine |
| `main:external-mcp-application-integration` | 1 | 0 | unfinished-lane | agent host |
| `main:image-sequence-import-coordinator` | 1 | 0 | unfinished-lane | application/services |
| `main:main-mcp-host-integration` | 1 | 0 | unfinished-lane | agent host |
| `main:markdown-local-media-grants` | 1 | 0 | unfinished-lane | media/engine |
| `main:mcp:apply_commands` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:capture_panel` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:check_project` | 1 | 0 | unfinished-lane | media/engine |
| `main:mcp:computer_use_panel` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:create_project` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:delegate_task` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:edit_3d` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:edit_layer_strips` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:edit_video` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:export_captions` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:export_video` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:finish_session` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:fork_builtin_extension` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:generate_captions` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:get_3d_scene` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:get_panel_layout` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:get_panel_state` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:get_project_state` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:get_timeline` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:get_workspace_state` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:import_media` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:import_media_to_timeline` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:inspect_creative_extension` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:inspect_creative_workspace` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:interact_panel` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:list_media` | 1 | 0 | unfinished-lane | media/engine |
| `main:mcp:list_projects` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:manage_media` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:manage_project` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:media_contact_sheet` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:media_waveform` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:open_panel` | 1 | 0 | macos-screen | macOS application |
| `main:mcp:open_project` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:orchestrator_capabilities` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:probe_media` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:publish_extensions` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:publish_visual_reply` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:render_frames` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:replace_media` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:rollback_changes` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:sample_media_frames` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:save_project` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:select_layers` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:set_panel_layout` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:stage_fork_rebase` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:start_session` | 1 | 0 | portable-probe | acceptance |
| `main:mcp:store_extension` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_install` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_library` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_publish` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_publish_prepare` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_search` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_source` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_uninstall` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:store_update` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:task_cancel` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:task_send` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:task_status` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_cancel` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_launch` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_list` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_read` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_send` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_unwatch` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:thread_watch` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:transcribe_media` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:mcp:validate_effect` | 1 | 0 | portable-probe | acceptance |
| `main:metal-authored-frame-oracles` | 1 | 0 | macos-platform | macOS engine/platform |
| `main:native-agent-composer-attachments` | 1 | 0 | macos-screen | macOS application |
| `main:native-agent-conversation-surface` | 1 | 0 | macos-screen | macOS application |
| `main:native-async-project-open` | 1 | 0 | unfinished-lane | application/services |
| `main:native-audio-mixer-device-meters` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-caption-editor-and-inference` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-clip-trim-split-controls` | 1 | 0 | unfinished-lane | application/services |
| `main:native-close-quit-coordinator` | 1 | 0 | macos-screen | macOS application |
| `main:native-colour-control-sampler` | 1 | 0 | macos-screen | macOS application |
| `main:native-composition-settings-controls` | 1 | 0 | macos-screen | macOS application |
| `main:native-control-input-bindings` | 1 | 0 | macos-screen | macOS application |
| `main:native-crash-recovery-coordinator` | 1 | 0 | macos-screen | macOS application |
| `main:native-cross-surface-history-clipboard` | 1 | 0 | macos-screen | macOS application |
| `main:native-easing-preset-library` | 1 | 0 | unfinished-lane | application/services |
| `main:native-effects-matte-controls` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-embedded-audio-routing` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-export-child-codec-integration` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-export-queue-snapshots` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-first-run-about` | 1 | 0 | macos-screen | macOS application |
| `main:native-font-text-animation-controls` | 1 | 0 | unfinished-lane | application/services |
| `main:native-graph-editor-and-curve-oracle` | 1 | 0 | macos-screen | macOS application |
| `main:native-group-parent-precompose-pixels` | 1 | 0 | unfinished-lane | application/services |
| `main:native-input-ime-accessibility` | 1 | 0 | macos-screen | macOS application |
| `main:native-inspector-property-bindings` | 1 | 0 | macos-screen | macOS application |
| `main:native-key-command-dispatch` | 1 | 0 | macos-screen | macOS application |
| `main:native-keyboard-dispatch` | 1782 | 0 | macos-screen | macOS application |
| `main:native-keyframe-editor` | 1 | 0 | macos-screen | macOS application |
| `main:native-launch-open-routing` | 1 | 0 | macos-screen | macOS application |
| `main:native-library-live-previews` | 1 | 0 | unfinished-lane | application/services |
| `main:native-linked-av-editor-integration` | 1 | 0 | unfinished-lane | application/services |
| `main:native-marker-controls` | 1 | 0 | macos-screen | macOS application |
| `main:native-media-browser-controller` | 1 | 0 | macos-screen | macOS application |
| `main:native-menu-command-dispatch` | 1 | 0 | macos-screen | macOS application |
| `main:native-notification-haptic-adapters` | 1 | 0 | macos-platform | macOS engine/platform |
| `main:native-palette-dialog-notices` | 1 | 0 | macos-screen | macOS application |
| `main:native-panel-popout-lifecycle` | 1 | 0 | macos-screen | macOS application |
| `main:native-panel-semantic-surface-host` | 1 | 0 | macos-screen | macOS application |
| `main:native-path-mask-editor` | 1 | 0 | macos-screen | macOS application |
| `main:native-playback-engine-transport` | 1 | 0 | unfinished-lane | application/services |
| `main:native-portable-document-flow` | 1 | 0 | unfinished-lane | application/services |
| `main:native-project-library-controls` | 1 | 0 | macos-screen | macOS application |
| `main:native-ripple-gesture-integration` | 1 | 0 | unfinished-lane | application/services |
| `main:native-roll-slide-gesture-integration` | 1 | 0 | unfinished-lane | application/services |
| `main:native-scene3d-editor` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-settings-service-bindings` | 1 | 0 | macos-screen | macOS application |
| `main:native-shader-compiler-editor` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-shuttle-command-engine-routing` | 1 | 0 | unfinished-lane | application/services |
| `main:native-signed-updater` | 1 | 0 | macos-platform | macOS engine/platform |
| `main:native-source-monitor-three-point` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-source-preview-controller` | 1 | 0 | unfinished-lane | media/engine |
| `main:native-text-editor-layout` | 1 | 0 | macos-screen | macOS application |
| `main:native-theme-host-bindings` | 1 | 0 | macos-screen | macOS application |
| `main:native-timeline-layer-controls` | 1 | 0 | macos-screen | macOS application |
| `main:native-timeline-snapping` | 1 | 0 | macos-screen | macOS application |
| `main:native-viewer-navigation` | 1 | 0 | macos-screen | macOS application |
| `main:native-viewer-selection-movement` | 1 | 0 | macos-screen | macOS application |
| `main:native-viewer-transform-gestures` | 1 | 0 | macos-screen | macOS application |
| `main:native-window-tab-routing` | 1 | 0 | macos-screen | macOS application |
| `main:native-work-area-controls` | 1 | 0 | macos-screen | macOS application |
| `main:native-workspace-layout-controller` | 1 | 0 | macos-screen | macOS application |
| `main:owner-browser-host-scope` | 1 | 0 | retained-owner-scope | owner scope |
| `main:physical-paired-performance-oracles` | 1 | 0 | macos-platform | macOS engine/platform |
| `main:project-memory-index-and-tool` | 1 | 0 | unfinished-lane | kernel/MCP |
| `main:provider-presets-model-discovery` | 1 | 0 | unfinished-lane | agent host |
| `main:provider-review-consent-error-identity` | 1 | 0 | unfinished-lane | agent host |
| `main:publication-panel-generation-fences` | 1 | 0 | unfinished-lane | application/services |
| `main:signed-store-helper-admission` | 1 | 0 | macos-platform | macOS engine/platform |
| `main:store-account-library-service` | 1 | 0 | unfinished-lane | application/services |
| `main:store-remote-install-publish-service` | 1 | 0 | unfinished-lane | application/services |
| `main:transcription-model-inference-service` | 1 | 0 | unfinished-lane | media/engine |
| `main:visual-reply-thread-persistence` | 1 | 0 | unfinished-lane | agent host |
| `mcp:capture_panel` | 0 | 1 | macos-screen | macOS application |
| `mcp:computer_use_panel` | 0 | 1 | macos-screen | macOS application |
| `mcp:edit_3d` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:export_video` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:generate_captions` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:get_panel_state` | 0 | 1 | macos-screen | macOS application |
| `mcp:get_workspace_state` | 0 | 1 | macos-screen | macOS application |
| `mcp:import_media` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:import_media_to_timeline` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:inspect_creative_extension` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:inspect_creative_workspace` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:interact_panel` | 0 | 1 | macos-screen | macOS application |
| `mcp:media_contact_sheet` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:media_waveform` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:open_panel` | 0 | 1 | macos-screen | macOS application |
| `mcp:probe_media` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:publish_extensions` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:publish_visual_reply` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:render_frames` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:replace_media` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:sample_media_frames` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:save_project` | 0 | 1 | portable-probe | acceptance |
| `mcp:set_panel_layout` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:stage_fork_rebase` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_extension` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_install` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_library` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_publish` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_publish_prepare` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_search` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_source` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_uninstall` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:store_update` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:task_send` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_cancel` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_launch` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_list` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_read` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_send` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_unwatch` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:thread_watch` | 0 | 1 | unfinished-lane | kernel/MCP |
| `mcp:transcribe_media` | 0 | 1 | unfinished-lane | kernel/MCP |
| `memory-post-tag-requirement` | 0 | 2 | retained-owner-scope | owner scope |
| `metal-authored-frame-oracles` | 0 | 1 | macos-platform | macOS engine/platform |
| `native-agent-composer-attachments` | 0 | 12 | macos-screen | macOS application |
| `native-agent-conversation-surface` | 0 | 16 | macos-screen | macOS application |
| `native-app:agent-approval-question-stream-host` | 2 | 0 | unfinished-lane | agent host |
| `native-app:agent-project-tool-integration` | 14 | 0 | unfinished-lane | agent host |
| `native-app:agent-provider-auth-stream-host` | 9 | 0 | unfinished-lane | agent host |
| `native-app:agent-run-thread-coordinator` | 12 | 0 | unfinished-lane | agent host |
| `native-app:agent-visual-reply-publication` | 7 | 0 | unfinished-lane | agent host |
| `native-app:cloud-media-download-coordinator` | 2 | 0 | unfinished-lane | media/engine |
| `native-app:compat-live-kernel-render-page-binding` | 3 | 0 | unfinished-lane | compat runtime |
| `native-app:decoded-source-retiming-integration` | 1 | 0 | unfinished-lane | media/engine |
| `native-app:engine-media-registry-integration` | 5 | 0 | unfinished-lane | media/engine |
| `native-app:extension-source-promotion-activation` | 19 | 0 | unfinished-lane | media/engine |
| `native-app:external-mcp-application-integration` | 2 | 0 | unfinished-lane | agent host |
| `native-app:native-agent-composer-attachments` | 9 | 0 | macos-screen | macOS application |
| `native-app:native-agent-conversation-surface` | 21 | 0 | macos-screen | macOS application |
| `native-app:native-caption-editor-and-inference` | 10 | 0 | unfinished-lane | media/engine |
| `native-app:native-close-quit-coordinator` | 10 | 0 | macos-screen | macOS application |
| `native-app:native-colour-control-sampler` | 7 | 0 | macos-screen | macOS application |
| `native-app:native-composition-settings-controls` | 5 | 0 | macos-screen | macOS application |
| `native-app:native-control-input-bindings` | 4 | 0 | macos-screen | macOS application |
| `native-app:native-crash-recovery-coordinator` | 3 | 0 | macos-screen | macOS application |
| `native-app:native-cross-surface-history-clipboard` | 5 | 0 | macos-screen | macOS application |
| `native-app:native-easing-preset-library` | 3 | 0 | unfinished-lane | application/services |
| `native-app:native-first-run-about` | 1 | 0 | macos-screen | macOS application |
| `native-app:native-font-text-animation-controls` | 10 | 0 | unfinished-lane | application/services |
| `native-app:native-graph-editor-and-curve-oracle` | 10 | 0 | macos-screen | macOS application |
| `native-app:native-group-parent-precompose-pixels` | 23 | 0 | unfinished-lane | application/services |
| `native-app:native-input-ime-accessibility` | 11 | 0 | macos-screen | macOS application |
| `native-app:native-inspector-property-bindings` | 13 | 0 | macos-screen | macOS application |
| `native-app:native-key-command-dispatch` | 5 | 0 | macos-screen | macOS application |
| `native-app:native-keyframe-editor` | 30 | 0 | macos-screen | macOS application |
| `native-app:native-launch-open-routing` | 7 | 0 | macos-screen | macOS application |
| `native-app:native-library-live-previews` | 8 | 0 | unfinished-lane | application/services |
| `native-app:native-marker-controls` | 1 | 0 | macos-screen | macOS application |
| `native-app:native-media-browser-controller` | 6 | 0 | macos-screen | macOS application |
| `native-app:native-notification-haptic-adapters` | 1 | 0 | macos-platform | macOS engine/platform |
| `native-app:native-palette-dialog-notices` | 8 | 0 | macos-screen | macOS application |
| `native-app:native-panel-popout-lifecycle` | 2 | 0 | macos-screen | macOS application |
| `native-app:native-panel-semantic-surface-host` | 9 | 0 | macos-screen | macOS application |
| `native-app:native-path-mask-editor` | 15 | 0 | macos-screen | macOS application |
| `native-app:native-playback-engine-transport` | 25 | 0 | unfinished-lane | application/services |
| `native-app:native-project-library-controls` | 3 | 0 | macos-screen | macOS application |
| `native-app:native-settings-service-bindings` | 6 | 0 | macos-screen | macOS application |
| `native-app:native-source-preview-controller` | 2 | 0 | unfinished-lane | media/engine |
| `native-app:native-text-editor-layout` | 11 | 0 | macos-screen | macOS application |
| `native-app:native-theme-host-bindings` | 3 | 0 | macos-screen | macOS application |
| `native-app:native-timeline-layer-controls` | 37 | 0 | macos-screen | macOS application |
| `native-app:native-timeline-snapping` | 4 | 0 | macos-screen | macOS application |
| `native-app:native-viewer-navigation` | 19 | 0 | macos-screen | macOS application |
| `native-app:native-viewer-selection-movement` | 4 | 0 | macos-screen | macOS application |
| `native-app:native-viewer-transform-gestures` | 7 | 0 | macos-screen | macOS application |
| `native-app:native-window-tab-routing` | 11 | 0 | macos-screen | macOS application |
| `native-app:native-workspace-layout-controller` | 7 | 0 | macos-screen | macOS application |
| `native-app:physical-paired-performance-oracles` | 29 | 0 | macos-platform | macOS engine/platform |
| `native-app:signed-store-helper-admission` | 10 | 0 | macos-platform | macOS engine/platform |
| `native-app:store-remote-install-publish-service` | 2 | 0 | unfinished-lane | application/services |
| `native-app:transcription-model-inference-service` | 5 | 0 | unfinished-lane | media/engine |
| `native-async-project-open` | 0 | 9 | unfinished-lane | application/services |
| `native-audio-mixer-device-meters` | 0 | 8 | unfinished-lane | media/engine |
| `native-caption-editor-and-inference` | 0 | 76 | unfinished-lane | media/engine |
| `native-clip-trim-split-controls` | 0 | 14 | unfinished-lane | application/services |
| `native-close-quit-coordinator` | 0 | 9 | macos-screen | macOS application |
| `native-colour-control-sampler` | 0 | 38 | macos-screen | macOS application |
| `native-composition-settings-controls` | 0 | 8 | macos-screen | macOS application |
| `native-control-input-bindings` | 0 | 50 | macos-screen | macOS application |
| `native-crash-recovery-coordinator` | 0 | 6 | macos-screen | macOS application |
| `native-cross-surface-history-clipboard` | 0 | 16 | macos-screen | macOS application |
| `native-easing-preset-library` | 0 | 11 | unfinished-lane | application/services |
| `native-effects-matte-controls` | 0 | 7 | unfinished-lane | media/engine |
| `native-embedded-audio-routing` | 0 | 7 | unfinished-lane | media/engine |
| `native-export-child-codec-integration` | 0 | 19 | unfinished-lane | media/engine |
| `native-export-queue-snapshots` | 0 | 6 | unfinished-lane | media/engine |
| `native-first-run-about` | 0 | 6 | macos-screen | macOS application |
| `native-font-text-animation-controls` | 0 | 19 | unfinished-lane | application/services |
| `native-graph-editor-and-curve-oracle` | 0 | 27 | macos-screen | macOS application |
| `native-group-parent-precompose-pixels` | 0 | 33 | unfinished-lane | application/services |
| `native-input-ime-accessibility` | 0 | 7 | macos-screen | macOS application |
| `native-inspector-property-bindings` | 0 | 59 | macos-screen | macOS application |
| `native-key-command-dispatch` | 0 | 16 | macos-screen | macOS application |
| `native-keyframe-editor` | 0 | 43 | macos-screen | macOS application |
| `native-launch-open-routing` | 0 | 14 | macos-screen | macOS application |
| `native-library-live-previews` | 0 | 7 | unfinished-lane | application/services |
| `native-linked-av-editor-integration` | 0 | 1 | unfinished-lane | application/services |
| `native-marker-controls` | 0 | 8 | macos-screen | macOS application |
| `native-media-browser-controller` | 0 | 17 | macos-screen | macOS application |
| `native-menu-command-dispatch` | 0 | 77 | macos-screen | macOS application |
| `native-notification-haptic-adapters` | 0 | 5 | macos-platform | macOS engine/platform |
| `native-palette-dialog-notices` | 0 | 23 | macos-screen | macOS application |
| `native-panel-popout-lifecycle` | 0 | 5 | macos-screen | macOS application |
| `native-panel-semantic-surface-host` | 0 | 8 | macos-screen | macOS application |
| `native-path-mask-editor` | 0 | 24 | macos-screen | macOS application |
| `native-playback-engine-transport` | 0 | 16 | unfinished-lane | application/services |
| `native-portable-document-flow` | 0 | 28 | unfinished-lane | application/services |
| `native-project-library-controls` | 0 | 10 | macos-screen | macOS application |
| `native-ripple-gesture-integration` | 0 | 2 | unfinished-lane | application/services |
| `native-roll-slide-gesture-integration` | 0 | 1 | unfinished-lane | application/services |
| `native-scene3d-editor` | 0 | 29 | unfinished-lane | media/engine |
| `native-settings-service-bindings` | 0 | 16 | macos-screen | macOS application |
| `native-shader-compiler-editor` | 0 | 1 | unfinished-lane | media/engine |
| `native-shuttle-command-engine-routing` | 0 | 1 | unfinished-lane | application/services |
| `native-signed-updater` | 0 | 4 | macos-platform | macOS engine/platform |
| `native-source-monitor-three-point` | 0 | 1 | unfinished-lane | media/engine |
| `native-source-preview-controller` | 0 | 8 | unfinished-lane | media/engine |
| `native-text-editor-layout` | 0 | 31 | macos-screen | macOS application |
| `native-theme-host-bindings` | 0 | 17 | macos-screen | macOS application |
| `native-timeline-layer-controls` | 0 | 60 | macos-screen | macOS application |
| `native-timeline-snapping` | 0 | 17 | macos-screen | macOS application |
| `native-viewer-navigation` | 0 | 32 | macos-screen | macOS application |
| `native-viewer-selection-movement` | 0 | 26 | macos-screen | macOS application |
| `native-viewer-transform-gestures` | 0 | 19 | macos-screen | macOS application |
| `native-window-tab-routing` | 0 | 20 | macos-screen | macOS application |
| `native-work-area-controls` | 0 | 10 | macos-screen | macOS application |
| `native-workspace-layout-controller` | 0 | 14 | macos-screen | macOS application |
| `owner-browser-host-scope` | 0 | 8 | retained-owner-scope | owner scope |
| `owner-web-animation-export-scope` | 0 | 1 | retained-owner-scope | owner scope |
| `physical-paired-performance-oracles` | 0 | 1 | macos-platform | macOS engine/platform |
| `real-keyboard-dispatch` | 0 | 1755 | macos-screen | macOS application |
| `signed-store-helper-admission` | 0 | 13 | macos-platform | macOS engine/platform |
| `store-account-library-service` | 0 | 5 | unfinished-lane | application/services |
| `store-remote-install-publish-service` | 0 | 9 | unfinished-lane | application/services |
| `transcription-model-inference-service` | 0 | 16 | unfinished-lane | media/engine |

The fresh [legacy workflow aggregate table](../../apps/native/crates/acceptance/data/legacy-workflows-2026-11-05.md)
retains the actual configured per-workflow counts. W007/W031 aggregates now fail
because of the measured ingress/export defects; the other 81 aggregates remain
blocked. All 83 complete-workflow contract rows remain blocked and no workflow
passes. Earlier dated shared W1 tables retain their original evidence instead
of being rewritten to match this new execution.

### Final merge replay — 2026-11-05

Fetched and merged `origin/feat/native-engine` at
`5bcd425ddc158cbd891ee4b515cf9993fad29a66` (already contained; no conflict).
The final locked replay has **106 acceptance + 17 optional-worker + 37 retained-e2e
Rust tests = 160 passed, 0 failed, 0 ignored**, plus **15 Python tests passed**.
All three crate-local formatting and Clippy `-D warnings` checks pass. No package
or workspace member was added, so neither native/engine nor nested lockfiles
needed any dependency update; every Cargo command uses `--locked`.

Exact suite commands, also executed by `verify.sh --strict`:

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-acceptance -p pm-acceptance-runtime
python3 -m unittest discover -s apps/native/crates/acceptance/scripts -p 'test_*.py'
```

The final configured acceptance remains **1,700 / 2 / 2,643 main** and
**1,467 / 2 / 3,135 legacy**; `verify.sh --strict` exits **1**, preserving both
measured product defects. All 5,798 checked-in census verdicts byte-match a fresh
reconciliation. Both new file probes are explicit required compatibility-input
gates in main, alongside their current-main verdicts. Earlier passing file/extension
checks remain and no historical UI gate reinstates the old UI reference.

Only the owned acceptance subtree and this append-only document differ from the
integration base. Secret scanning with the repository's staged gitleaks policy
passes. macOS/application/Opus qualification and the owning production fixes
listed above remain open; this lane is mergeable, not full product acceptance.
