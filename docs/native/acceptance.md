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

[remaining-blockers.csv](../../apps/native/crates/acceptance/data/remaining-blockers.csv)
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
cargo test --locked --manifest-path crates/acceptance/runtime/Cargo.toml
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
[remaining-blockers.csv](../../apps/native/crates/acceptance/data/remaining-blockers.csv).
