# v4 integration evaluation before freeze — 2026-11-01

**Final recommendation: HOLD the production API/cutover freeze.** The integrated
Linux headless suite is green: **172 Bun tests**, **95 distinct pm-surface tests**,
and **40/40 headless eval tasks (100%)**, including all ten supplied owner
behavior specs, both with and without `--wire`. There are no failed, skipped or
pending tests/tasks. The scaffold-with-wire comparison also passes 40/40.
**Ten production owner-panel acceptances remain blocked on integration.** No
Linux score certifies their media services, actual kernel binding or macOS UI.

This section supersedes every status/count/reproduction instruction in the
archived reports below. Base: `fd9c3109d` on `feat/na-v4-integrate`; Linux x86_64,
repository-pinned Rust 1.96.1, Bun 1.4.4, synthetic fixtures only. This is an
integration rerun of the existing corpus, not a new blinded usability study.
No private owner names, source, identifiers or user projects were used.

## Final evidence and score boundaries

| Check | Result | Scope |
|---|---|---|
| `cargo test --locked -p pm-surface` | 95 passed / 0 failed / 0 ignored | 10 module + 11 host-data + 35 surface + 39 kit tests |
| Same tests via standalone manifest | 95 passed | Same tests, not 95 additional tests |
| Workspace and standalone all-target Clippy, `-D warnings` | pass | No new warnings |
| Workspace and standalone rustfmt | pass | Owned surface source/examples/tests |
| Plain `bun test` | 172 passed / 0 failed / 0 skipped | Preload builds/configures native conformance examples |
| Base and eval/examples TypeScript, Prettier, browser build | pass | Includes kit declarations and document-free bundle tests |
| Default `eval --include-held-out` | 40 passed / 0 failed / 0 pending | Public briefs plus authored owner fixtures and native host-service checks |
| `eval --include-held-out --wire` | 40 passed / 0 failed / 0 pending | Additionally validates surface batches in standalone Rust |
| `eval --include-held-out --scaffold --wire` | 40 passed / 0 failed / 0 pending | Original scaffold plus the same authored owner fixtures |

The owner score now has two explicit dimensions. `status`, `fixtureStatus` and
`serviceStatus` report the **executable headless contract**. `productionReady:
false`, `fullAcceptance: blocked-on-production-integration`, `integrationGates`
and top-level `ownerProductionBlocked: 10` retain the production gates. This
replaces unconditional failures based on stale “No public … service” strings;
it does not declare the remaining media/capture/resource contracts implemented.
The prior full-acceptance failure reports are preserved below for provenance.
The private plan's LOC/lookup/success threshold is still unavailable and is not
certified by the numerical headless task score.

Each owner run executes public HostClient queries/subscriptions/atomic edits and
settings through the real Rust HostRouter/HostDataService with an explicit
KernelData double. The annotation fixture additionally drives project/time reads,
time subscription, acknowledged annotation edit/rejection, unsubscribe and
current text through `inspect().nodes`; it no longer checks a private test
observation map for the retained draft. Domain/media algorithms in other owner
fixtures still use authored synthetic service seams. Their passing behavior is
not evidence of actual capture, decoding, network access, asset import, frame
readback or resource ownership. The data smoke checks and fixtures do not port
the owner's original extension code.

Tracked rerun outputs (historical evidence files are unchanged):

| Evidence | SHA-256 |
|---|---|
| `eval/evidence/integration.json` | `53823108899991ad2676045fa81faa6cc9757e2c07cdc677f0eb451322fd43ee` |
| `eval/evidence/integration-wire.json` | `10320a733e7a33a2ca49f6fe15c9f8ad978157fefaed31638996c4a01dba5252` |
| `eval/evidence/integration-scaffold-wire.json` | `cac068eef98822b6e4adb81437596d0a3fa21f633c20de417aa08b7a9ea96e9e` |

These outputs contain process/replay timings, not settled performance samples.
No performance benchmark was rerun or promoted to an N2 result in this pass.

## Changes and rule-selected decisions

| Decision/change | Selecting rule |
|---|---|
| Anchor kit fixtures use `anchored`; retain strict primitive validation and every Opus recipe/interaction decision | N1/N3; design ownership and `design/reviews/na-cloud-uikit.md` |
| Explicit kit AX/semantic props; component callbacks/options override primitive signatures, preserving other primitive callbacks | N3/N7; typed contracts must match the actual wire data |
| Plain Bun conformance builds the real validators; stale expected-failure assertions now require passes | N1/N3; infrastructure or old assertions cannot hide a regression |
| Concrete worker HostDataService queries immutable publications; atomic edits forward unchanged through KernelData to the single actor writer | N3/N7/N8; no JS document mirror or UI-thread JS waits |
| Subscribe ack includes initial snapshot; buffered publication wins over older ack; same-revision time updates use monotonic sequences; acknowledged IDs never recycle | N3/N7; prevent lost updates and stale delivery to another subscription |
| Requests copy authored data and serialize against actual acks; delayed/rejected edits never become optimistic success; failed queues/retirement remain retryable | N3/N4/N7/N8 |
| Preserve raw accepted tree versus observed current text; echo emits no redundant value write and rejected annotation edits retain the public draft | N3/N7; exact authored Unicode and native caret/composition semantics |
| Report headless passes separately from production acceptance, preserving the old reports | N1/N2/N3; no fabricated resource, durability, visual or performance proof |
| Retain all v1–3/app/built-in consumers and historical evidence; no cutover or pruning | N4/N6 and prune rule |

New **DESIGN-TODO(opus): none**. Existing panel-composition/control-renderer TODOs
and Opus F1–F7 follow-ups remain. All visual output is provisional pending the
actual native rendering review.

## Freeze blockers and macOS verification

Before production freeze, bind KernelData to KernelHandle/QueryMirror and
state_transaction in the application runtime. Verify authorization, writer
registration/sealing, request-ID -> kernel-txid dedupe, grouped undo/rollback,
selection/time publication and render invalidation, settings namespace/durability,
project switching, realm retirement and lost-ack recovery against the actual actor.
The owned manifests intentionally link no kernel implementation; no application
source outside this lane was edited.

Supply/verify permission, media decoding/analysis/capture, import/export, catalog,
frame/readback, preview/resource lifecycle and visibility/frame-scheduler adapters
for the owner workflows. Verify actual compiler aliases/Refresh boot and realm
checkpoint/reset. The legacy semantic UI export remains separate from the reviewed
kit; real compiler/export cutover remains integration work.

On macOS verify GPUI/AppKit controls, system pop-ups/sheets/popover placement,
Esc/outside dismissal and focus restoration; AX/VoiceOver and keyboard ownership;
UTF-16 IME/caret/composition/clipboard; pointer capture/lock/infinite scrub and
coordinate conversion; resource retirement and actual renderer behavior. Obtain
Opus light/dark/accessibility rendering review and signed V8/physical presentation
performance evidence, including proof that the main thread never waits on JS.
No new design, compatibility-cutover or N2 performance approval is claimed.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
cargo clippy --locked -p pm-surface --all-targets -- -D warnings
cargo fmt -p pm-surface --check
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --examples
cd js/v4
bun install --frozen-lockfile
bun test
bun run typecheck
bunx tsc -p eval/tsconfig.json
bun run format
bun run build
bun run eval --include-held-out
bun run eval --include-held-out --wire
bun run eval --include-held-out --scaffold --wire
```

## Archived pre-integration reports (superseded)

Everything below is retained historical evidence, including references to then
current failures, missing owner specs, ownership restrictions and old test counts.
Use the final report above for the integrated result and freeze recommendation.

### Pre-integration API fix pass

Recommendation: **do not freeze yet**. The rerun of the original independent
public-API extensions now passes **30/30 available briefs (100%)**, up from 28/30.
The original scaffold also passes **30/30**. All **10 owner-panel briefs remain
pending-owner**, so positive task evidence is **30/40 (75%)**, not 40 passes.
Ranks 1–7 now have lane-owned implementations/contracts and documentation;
rank 3's concrete kernel adapter and production compiler/native lifecycle wiring
remain external integration work. No v1–3, built-in panel or renderer was replaced.

Runtime implementation tested: `640a2ad81` (parent/base
`ff42d751fbf6a9266cd2ba1a8e25345b60baa5d7`). Linux x86_64, Bun 1.4.4,
repository-pinned Rust 1.96.1. Synthetic data only. The runtime fix author read the
implementation while fixing it; this is a **rerun of the existing independently
authored public-API corpus**, not a new blinded authoring study or another set of
30 developers. The original public-only attempt's method and findings are retained
below as historical provenance. Its unowned source/evidence files were not rewritten.

## Current verification

- **45 Rust tests passed** (10 new module tests plus 35 existing integration tests),
  using `cd apps/native && cargo test --locked -p pm-surface`. Workspace all-target
  clippy with `-D warnings` and rustfmt passed. Unlike the old evaluation base,
  this base's shared lock works without modification. The standalone harness builds
  and exercises the same sources for wire replay.
- **15 new Bun regression tests passed**. Full suite: **122 pass / 2 fail / 0 skip**
  across 124 tests with the real Rust wire configured. The two failures are the
  existing unowned assertions in `eval/extensions.test.ts` that require
  `builtin-06` and `wild-10` to fail; both task executions now correctly pass.
  Ownership permission was requested for those two assertion updates; absent that
  permission they remain unchanged. This is explicitly **not a green full suite**.
- Base and eval/examples TypeScript checks, Prettier, browser builds and the
  document-free runtime test pass. New declaration regressions reject properties
  on the wrong primitive, arbitrary native capability names and malformed ranges.
- `bun run eval --include-held-out --wire=…`: **30 pass / 0 fail / 10 pending**.
  Separate `--scaffold` run: **30 pass / 0 fail / 10 pending**. Owner placeholders
  are never counted as successful tasks.
- Gitleaks scans before each commit passed; no project/profile/owner data was used.

The input wire example still accepts commits rather than AppKit observations. The
independent corpus retains its explicit input overlay, now matching the documented
current projection in `inspect().nodes`. New JS drive tests and Rust native-drive
unit tests separately cover UTF-16 ranges, exact echo/caret/composition state,
stale generations/revisions, capture, drag payloads and malformed input rollback.
This is not an end-to-end AppKit/V8 input proof.

## Implemented ranks and remaining integration

| Rank | Lane-owned change and evidence | External gate | Selecting rule |
|---|---|---|---|
| 1 | `acceptedNodes` versus current `nodes`/typed `inputs`; observe/drive by ID; UTF-16 composition ranges; exact echo and stale-drive regressions | AppKit delegate → Rust → V8 event delivery and real IME/clipboard | N1/N3/N7 |
| 2 | Root stylesheet participates in accepted snapshot replay at the acknowledged revision; replacement/rejection isolation tests; wild-10 passes | Native stylesheet layout/paint | N3/N7 |
| 3 | Version-1 HostClient + Rust HostRouter/KernelQueue envelopes; binding/query/subscription/command/settings calls; identity/revision/ack checks, bounded queues, duplicate refusal, lost-ack fencing and independent profile revisions | Implement actual KernelHandle/QueryMirror/state_transaction adapter and binding/permission mapping in runtime; verify durability and production edits | N1/N4/N7/N8 |
| 4 | Typed modifier/key/pointer/selection/composition/drag/range events; native envelope generation checks and headless accepted-listener planner; capture/default declarations stay native | Physical hit testing, drag/drop and UI-thread enqueue integration | N3/N6/N7/N8 |
| 5 | Strict JSX per-primitive declarations, explicit metadata, Rust supported-field shape/eligibility validation, bounded virtual ranges, point anchors without lookup commits, documented focus/resource lifetimes | Renderer/resource registry behavior and host focus/window publication | N1/N3/N7; N4/prune preserves old raw-wire unknown diagnostics |
| 6 | Idempotent fenced disposal runs local effect cleanup without another tree mutation; independent retire callback; checkpoint-copy + next-generation/reset ack recovery; in-flight retirement tests | Durable realm checkpoint, writer sealing, renderer retirement and actual reset transport | N3/N4/N8 |
| 7 | Public compiler alias/options exports; concrete Refresh-before-module/root boot example; generated axis binding/menu-label documentation; clean-process existing Refresh test | Install mappings/instrumentation in the real compiler/V8 host | N1/N7/N8 |

No visual design was selected. Existing semantic placeholder controls and the
**DESIGN-TODO(opus)** for picker/control rendering, panel composition, copy,
focus/navigation, materials and motion remain below; all visual output is provisional.

<!-- earlier section kept from the owner-spec lane -->
## Owner specification follow-up

The ten reserved owner tasks now have owner-approved, anonymized behavior briefs
and runnable synthetic fixtures. The current **JS-only** run on macOS arm64 with
Bun 1.4.4 reports **27 pass / 13 fail / 0 pending**, exits 1, and includes held-out
tasks. No Rust build or wire validator was used for this follow-up. This is a
separate evidence scope from the historical wire-backed results below.

All **10/10 owner fixtures pass their synthetic behavior checks**. Full owner
acceptance is **0/10 pass, 10/10 fail (`failureKind: api-gap`)**. The runner reports
`fixtureStatus` separately and includes each task's `blockedBy` list; passing
locally authored decision logic never certifies unavailable host services. No API
implementation was changed to obtain these results.

| Task | Generic behavior title | Synthetic fixture | Full API acceptance / main gap |
|---|---|---|---|
| owner-01 | Curve segment editor | pass | fail: project edits, preferences, pointer/clipboard payload |
| owner-02 | Video cutout with brush guidance | pass | fail: decoding/analysis lifecycle, mask insertion, brush coordinates |
| owner-03 | Audio take recovery workflow | pass | fail: capture, transport, asset import and acknowledged insertion |
| owner-04 | Paged media search and import | pass | fail: catalog access, previews, file picker and import |
| owner-05 | Verified transparent asset picker | pass | fail: alpha inspection, catalog/attribution and import |
| owner-06 | Read-only frame comparison | pass | fail: evaluated geometry, frame readback and preview lifecycle |
| owner-07 | Position-preserving pivot grid | pass | fail: evaluated geometry/transforms, project edits and preferences |
| owner-08 | Selective effect batch action | pass | fail: project/selection services and command/control modifiers |
| owner-09 | Keyboard arcade with saved score | pass | fail: durable settings, visibility/frame lifecycle and key-repeat field |
| owner-10 | Playhead annotation form | pass | fail: time/document subscriptions, edits and native-input inspection |

Selection favored distinct panel workflows by interaction, state and data-flow
complexity: complex editors and asynchronous media workflows through batch tools
and a small time-based form. Redundant interaction patterns and non-panel effects
were excluded. Source references were inspected read-only. No source mapping,
name, identifier, path, literal UI copy, code, artwork or user project data is
included; every fixture label, numeric example and resource handle is synthetic.

Each registry record describes what is shown, inputs, interactions, required
reads/writes, edge cases, acceptance checks and current API gaps. The owner
fixtures live in `eval/scenarios.tsx` and use the public snapshot/event API via
`Evaluation`; command logs, service responses, notification delivery, frame ticks
and resource disposal are explicitly test seams. Some notifications are modeled
by republishing a view. They do not perform real host reads/writes, networking,
permission requests, capture, analysis or file import. Unknown properties and
resource descriptors prove no native renderer behavior. Native focus, rendering,
performance and all descriptive edge cases beyond the enumerated checks remain
unverified. For annotation input, the host observation overlay retains the draft;
this does not fix the public accepted-snapshot limitation. With wire validation,
that existing mismatch can fail the fixture before the API-gap classification.

The other three JS-only failures are `builtin-06` (current controlled text is not
observable in the accepted snapshot), `wild-09` (default-deny validation requires
the wire binary), and `held-out-10` (atomic native rejection requires that binary).
The last two are missing validation infrastructure in this run, not new API
regressions. Without wire, `wild-10` cannot detect the previously reported native
stylesheet mismatch; its headless pass does not resolve that finding.

Follow-up checks: **74 Bun tests passed, 45 wire-dependent tests skipped, zero
failed**; base and eval/examples TypeScript checks, formatting and diff whitespace
checks passed. The locked install required Bun 1.4.4; no lockfile was changed.

Reproduction (use Bun 1.4.4; an older Bun may not understand this lockfile):

```sh
cd apps/native/js/v4
bun install --frozen-lockfile
bun run eval --include-held-out
bun run eval --group=owner
bun test
bun run typecheck
bun x tsc --noEmit -p eval/tsconfig.json
bun run format
```

The CLI default and `--scaffold`/`--candidate` paths all route owner records to the
same fixture and full-acceptance reporting. This is an authored behavior evaluation,
not a blinded independent owner-panel port. No LOC or lookup metric is invented
for these fixtures. Public API basis: `v4-api.md` sections Protocol and ownership,
Interaction and bindings, and powermove/ui, plus the exported `SurfaceEvent`
declaration. Those document the missing host services and typed-event limits.
The recommendation remains **not ready**; the private plan threshold and native
acceptance gates remain unverified.

## Historical package 14 wire-backed evaluation

Recommendation: **not ready**. The independent public-API attempt passes **28/30
available briefs (93.33%)**, fails two inspection/drive checks, and leaves **10/10
owner-panel specifications pending-owner**. Only 28/40 records currently have
positive independent evidence (70%); missing specifications are not failures or
passes. The original scaffold still passes 30/30, which does not resolve the
independent findings. No API implementation, v1–3 path, built-in panel or app was
changed by this lane.

## Threshold and evidence scope

The private plan v6 and its numeric success/LOC/lookup threshold were not supplied.
Neither `v4-api.md`, `p0-decisions.md` nor the public task registry states that
threshold. **Score against the plan threshold: unverified, not certified.**
The observed available-task score is 30/30, but the private threshold and its
LOC/lookup conditions remain unverified; no full 40-task requirement can be
certified without the owner specifications.
The public benchmark's separate provisional p95 ceiling is 4 ms; it is not an
invented substitute for the private API-usability threshold. The orchestrator
must apply the actual threshold before changing this recommendation.

Historical independent-attempt base: `09af1206162f5a44401b777bcb9ac2e3bd07421b`. Linux x86_64,
Bun 1.4.4; Rust checks use repository-pinned Rust 1.96.1. Synthetic data only.
The frozen behavior source is commit `07ed5fa17ee640659cb075f9cad1af2095011d60`
(v1.2.0). Owner-source approval in `p0-decisions.md` permits anonymized behavior
specs, but none of the ten reserved owner briefs was provided here.

## Original independent attempt method and limits (historical)

Each task has its own extension JSX and behavior checks in
[`examples/tasks`](../../apps/native/js/v4/examples/tasks). This is one coding-agent
attempt with cached documentation, not thirty independent blinded developers.
Built-in and wild tasks were implemented first; the public held-out briefs were
then implemented without modifying the API. Their public availability is not
secret-test isolation. Boundary disclosure: an initial read of the exported protocol source also exposed
its finite-JSON helper body; it was not used to implement solutions. No
reconciler/host/control/style/Refresh implementation source or preexisting
scenario solution was opened. Error stacks exposed small runtime snippets.
A strictly blinded repeat should use a declaration-only SDK checkout. Public
package exports and TypeScript declaration-only compiler output supplied
signatures; public
`inspect()` output and Rust wire diagnostics supplied black-box feedback.
Tagged built-in source was consulted only for the three panel behavior ports.

The CLI defaults to these independent extensions. `--scaffold` and `--candidate`
retain the prior reconciler fixtures for existing consumers. All six operations
are validated by the existing Rust JSON-lines example; the harness compares node
counts, IDs, types, children, parents, props, authored text, listener maps and
revisions. It additionally tests an invalid final operation after valid mutations
and webview creation with opt-in disabled. Rejections preserve the entire native
tree and revision. No JS callback, private DOM adapter or `root.host` is used to
inspect a solution.

Event plans are a synthetic native planner over accepted public snapshots.
Input observations are an explicitly separate test overlay because the wire
example accepts commits, not AppKit input observations. The overlay consumes no
revision and is superseded by a later explicit value prop. This validates commit
encoding, not Rust native input reconciliation, IME or focus behavior. The two
failed cases remain failures of this public inspection/drive evaluation; the
caption finding is **not evidence of lost real AppKit input**.

LOC is nonblank/non-`//` source lines after Prettier, including imports and local
behavior assertions. It is an upper bound on solution code, not a pure UI-only
LOC metric. Shared harness/runner and three panel sources are additional files,
not silently included in the task counts. Lookups count distinct public
section/type consultations needed per task, including cached references; they
do not count shell reads or re-reading a cached document as fresh lookups.
`extensions.ts` records the references. Black-box correction attempts are listed
below. Wall ms includes module loading, assertions and process startup/replaying
all previous commits into Rust; it is **not** a performance-budget sample.

## Current per-task rerun result

Total task code/checks: **630 lines**; **47 public-reference consultations**.

| Task | Verdict | LOC | Lookups | Wall ms | Commits / ops | Friction |
|---|---|---:|---:|---:|---:|---|
| builtin-01 | pass | 25 | 2 | 6.61 | 1 / 20 | A1, A2 (historical) |
| builtin-02 | pass | 14 | 2 | 5.90 | 2 / 17 | A1 (historical) |
| builtin-03 | pass | 16 | 1 | 2.76 | 1 / 12 | A3 (historical) |
| builtin-04 | pass | 16 | 2 | 3.30 | 1 / 11 | A3 (historical) |
| builtin-05 | pass | 15 | 2 | 8.01 | 2 / 20 | A2 (historical) |
| builtin-06 | pass | 17 | 2 | 3.24 | 1 / 5 | A4 (historical) |
| builtin-07 | pass | 26 | 1 | 4.21 | 1 / 24 | A1, A2 (historical) |
| builtin-08 | pass | 14 | 2 | 4.14 | 1 / 27 | A1, A3 (historical) |
| builtin-09 | pass | 22 | 2 | 5.38 | 1 / 29 | A2 (historical) |
| builtin-10 | pass | 18 | 1 | 2.92 | 1 / 11 | A2 (historical) |
| wild-01 | pass | 18 | 1 | 3.35 | 1 / 22 | — (historical) |
| wild-02 | pass | 35 | 2 | 4.94 | 2 / 16 | A3 (historical) |
| wild-03 | pass | 11 | 1 | 2.32 | 1 / 10 | A3 (historical) |
| wild-04 | pass | 12 | 2 | 2.49 | 1 / 10 | A3 (historical) |
| wild-05 | pass | 24 | 2 | 8.18 | 2 / 115 | A3 (historical) |
| wild-06 | pass | 25 | 2 | 5.56 | 2 / 26 | A2, A3 (historical) |
| wild-07 | pass | 15 | 2 | 2.87 | 1 / 21 | A3 (historical) |
| wild-08 | pass | 31 | 1 | 3.33 | 1 / 21 | A1, A2 (historical) |
| wild-09 | pass | 6 | 1 | 3.54 | 1 / 4 | A3 (historical) |
| wild-10 | pass | 35 | 2 | 4.45 | 2 / 9 | A7 (historical) |
| held-out-01 | pass | 17 | 1 | 5.38 | 2 / 22 | — (historical) |
| held-out-02 | pass | 37 | 2 | 4.65 | 2 / 14 | A4 (historical) |
| held-out-03 | pass | 25 | 2 | 4.96 | 2 / 10 | A2, A4 (historical) |
| held-out-04 | pass | 18 | 1 | 4.62 | 2 / 10 | — (historical) |
| held-out-05 | pass | 25 | 1 | 2.16 | 1 / 6 | — (historical) |
| held-out-06 | pass | 24 | 1 | 7.72 | 3 / 18 | — (historical) |
| held-out-07 | pass | 37 | 2 | 6.89 | 3 / 8 | A5 (historical) |
| held-out-08 | pass | 29 | 1 | 2.88 | 1 / 18 | A2 (historical) |
| held-out-09 | pass | 10 | 1 | 2.73 | 1 / 4 | — (historical) |
| held-out-10 | pass | 13 | 2 | 7.65 | 2 / 5 | A6 (historical) |

`owner-01` through `owner-10`: **pending-owner**; no implementation, LOC, lookup
count or score is fabricated. The retained **pre-fix** raw task results and diagnostics are in
[`eval/evidence/results.json`](../../apps/native/js/v4/eval/evidence/results.json).
The ten owner tests assert pending status; they are not ten successful tasks.

## Original findings and authoring corrections (historical)

These describe the pre-fix API, not the current contract. The rank/status table
above records their resolution and remaining application integration.

**A1 — Missing host lifecycle/data contract.** Binding handles must already exist,
but the public surface API offers no registration, disposal, channel query,
selection/time subscription, command submission or durable extension-settings
entry point. A literal synthetic handle is sufficient to test a descriptor;
it cannot perform a production edit. The panel fixtures take immutable inputs
and record requests at explicit test seams. They never invent a working host
service or write a JS document mirror. Affects builtin-01/02/07/08, wild-08 and
all three panels. Timeline Cmd/Ctrl-toggle selection is an additional A4 gap.

**A2 — Component output and compiler alias documentation.** `powermove/ui` needs
an extension compiler alias; package exports work directly in these examples.
Control binding/picker props are discoverable only in public declarations.
`PointField` keeps its top-level binding but puts axis bindings under `attr` in
its emitted snapshot; a first assertion expecting the axis handles at top level
failed and was corrected after inspecting the public snapshot. Menu items expose
their labels as child text, not `props.label`; two initial tests used the wrong
lookup and now inspect the button's child text. These were authoring/driver
corrections, not runtime defects. Menus/Disclosure/Row generate semantic IDs, so
tests must inspect labels and associations rather than predict internal ID text.
Affects builtin-01/05/07/09/10, wild-06/08, held-out-03/08 and inspector/effects.

**A3 — Primitive schemas are underspecified.** The JSX index signature accepts
almost any property. `anchor={{x,y}}` typechecked but Rust rejected it as
`invalid anchor`. The graph example now publishes its path, obtains the accepted
numeric ID via `inspect()`, then publishes `anchor=<that ID>` and preserves the
authored point as metadata. This extra commit is a workaround; it does not prove
point-relative placement. Public docs do not specify path geometry, resource
handle lifecycle, markdown/code source fields, progress semantics, virtual-list
range publication, native focus requests or webview navigation permissions in
sufficient detail to implement the host. For these tasks, accepted structured
metadata is all that is proven. Virtual-list count/window remains bounded and
webview default-deny is actually checked. Affects builtin-03/04/08, wild-02/03/
04/05/06/07/09, timeline and effects. No unknown-property diagnostic is treated
as proof that a native renderer implements that property.

**A4 — Public input inspection and event contract.** builtin-06 starts with
`A🙂B`, dispatches native input `A日本🙂B`, and echoes that value through hooks.
No new value op is needed because the renderer observed native input first;
`inspect()` still exposes the last accepted pre-input value. The evaluation
cannot observe the current controlled field through the advertised public
snapshot, so the task fails. The separate native-observation overlay makes this
limitation explicit; it does not assert a corrupted native text buffer. Also,
`SurfaceEvent` omits metaKey/ctrlKey, pointer coordinates, mouse button/click
count, composition/selection and drag payload. The timeline's authored Cmd/Ctrl
selection and effects drag-to-apply cannot be ported through this event type.
Stale revision dispatch throws; the held-out test now expects that refusal and
checks that the retired callback was not called (a driver correction).
Affects builtin-06 (failed), held-out-02/03, timeline and effects. Rust focus/IME/
pointer state machines already tested elsewhere are not a public event API.

**A5 — Refresh initialization order is not stated.** Loading the public dev entry
after creating roots produced `Before` instead of `After` in a clean-process
probe. Explicitly initialize `powermove-v4/refresh` before any evaluation root;
this host setup makes held-out-07 pass without editing the runtime. Registration
identities/signatures are still compiler obligations. Document and test that
boot order; do not accidentally rely on importing the old scenario scaffold.

**A6 — Rejection and recovery ownership.** held-out-10 tests a malformed batch
against a separate Rust replay, proving rollback and subsequent valid edits.
It does not pretend the JS reconciler generated an invalid commit. The public
root contract fences after rejection and requires the owner to reset/remount;
no reset/generation/ack-recovery adapter is available here. In the initial
rejected-anchor attempt, calling public `dispose()` on the fenced root also
threw. The harness retires its test reference instead of claiming successful
unmount. Production worker/realm teardown must release resources independently
of sending another rejected tree mutation. No existing v1–3 behavior is removed.

**A7 — Stylesheet snapshot mismatch.** wild-10 publishes a valid tagged-union
selector conjunction through `setStyleRules`. Rust accepts root `styleRules`,
while JS `inspect()` reports root props `{}` at the same revision. Complete
snapshot comparison fails. An earlier untagged selector object was an authoring
mistake caught by TypeScript and Rust; it was corrected from public `Selector`
declarations and is not counted as a runtime defect. The remaining discrepancy
is reproducible with a single valid id selector too.

## Original ranked proposals (implemented scope recorded above)

| Rank | Recommended API/documentation change | Failed tasks fixed / other affected evidence | Rule |
|---|---|---|---|
| 1 | Specify current-native-input vs accepted-commit snapshot semantics; expose a typed current input observation/inspection and a headless drive route with UTF-16 selection/composition. Keep echo/caret semantics exact. | builtin-06; A4; timeline/inspector/effects input integration | N1/N3/N7 |
| 2 | Include accepted root stylesheet state in the public snapshot, or expose a documented complete accepted-surface inspector including rules. Ensure atomic revision agreement. | wild-10; A7 | N3/N7 |
| 3 | Publish a versioned host API for binding registration/disposal, selection/time/evaluated queries, commands and durable settings. Route all mutations asynchronously to pm-kernel with revision/generation-aware acknowledgements. | No additional available brief currently fails on this because descriptors accept synthetic handles; fixes production blockers in all 3 panel ports and A1 tasks | N1/N4/N7/N8 |
| 4 | Complete typed surface events and native drive declarations for meta/control modifiers, pointer coordinates/buttons/count, selection/composition and drag data/capture. Keep native default declarations and no UI-thread JS waits. | No additional available brief fails today; closes A4 partial timeline selection and effects drag behavior | N3/N6/N7/N8 |
| 5 | Export strict per-primitive prop/resource schemas and examples; allow extension metadata through an explicit bag, separate from supported native behavior. Document anchored ID acquisition, virtual-window updates and focus/resource lifetimes. | No remaining available brief failure; eliminates wild-02 rejected first attempt/two-commit workaround and A3 uncertainty | N1/N3/N7 |
| 6 | Specify fenced-root disposal and generation/ack recovery: retirement cannot depend on accepting another JS tree mutation; preserve extension state before remount. | held-out-10 recovery scope; rejected-anchor probe A6 | N3/N4/N8 |
| 7 | Ship compiler aliases/Refresh startup examples and document generated semantic output (including axis binding placement and menu labels). | held-out-07 late-init probe; A2/A5; remaining briefs pass after documented workarounds | N1/N7/N8 |

Ranks 1–2 address actual failed task checks. Ranks 3–7 address missing production
capabilities and measurable authoring burden, not invented extra failed tasks.
Optimizing the larger reconciliation workload is also required by the performance
gate; switching reconcilers is not justified by this one-candidate evaluation.

## Three representative panel ports

These are executable behavior/data-flow subsets, **not full built-in conversions**:

| Panel | Ported and checked | Still blocked through public v4 |
|---|---|---|
| Timeline row list | 10,000 synthetic rows, bounded 32-row window; stable keyed reorder, selected/locked metadata; ancestor collapse, stable range anchor/shrink, pure toggle selection; collapse/selection requests | Selection/project subscriptions, key/cue selection clearing, UI-state persistence, real native window publication, Cmd/Ctrl fields, drag/reorder commands and hit testing |
| Inspector property group | Mixed numeric values, authored precision/unit/bounds, label association and binding identity; evaluated-value/time republish, lock admission, key requests at current time, generic-vs-transform channel selection | Binding registration/edit transactions, live evaluated queries, multi-edit/keyframe commands, expression/picker surfaces and timeline service integration |
| Effects browser | Tag search by label/id, category fallback after unload, Recent filtering/dedup/max eight, no-selection request and add-by-ID request, search reset on category pick | Registry/selection subscriptions, actual add-effect ack/history/invalidation, durable recents, drag payload, click-count suppression and native focus/navigation |

Behavior provenance: tagged `extensions/timeline/timeline.ts`
(`toggleTimelineDisclosure`, `selectLayerForPointer`),
`extensions/inspector/ChannelRow.svelte`, `panels/FxBrowserPanel.svelte` and
`panels/fx-browser.svelte.ts` under `apps/desktop/src` (panel files under
`renderer/src`). No reference UI styles were copied. Eight panel tests exercise
these subsets through the real Rust wire validator and pure behavior models.
The models' callbacks are labeled test seams; they are not API-change proposals
silently implemented in examples.

**DESIGN-TODO(opus):** review timeline row, inspector property group and effects
browser composition, copy, focus/navigation, control/picker surfaces and eventual
native rendering. Current trees are neutral placeholders, with no selected
colours, spacing, typography or materials. `itemSize=1` is a test coordinate
unit, not a row-height design. All visual output remains provisional.

## Current CPU diagnostics and evidence identity

Same diagnostic scope: 30 warmups, 100 measured samples per workload; immediate
synthetic async JS ack, no kernel/IPC/layout/paint/presentation. These are separate
CPU distributions, never additive percentiles or physical acceptance. The larger
JS and effects workloads still exceed the public provisional 4-ms ceiling.

| Diagnostic | Native nodes | p50 ms | p95 ms |
|---|---:|---:|---:|
| JS (499 widgets) | 1000 | 1.533 | 3.014 |
| JS (1000 widgets) | 2002 | 3.704 | 5.484 |
| timeline-10000-window32 | 65 | 2.007 | 3.697 |
| inspector-50-properties | 253 | 0.873 | 2.168 |
| effects-1000-items | 1007 | 4.133 | 5.134 |
| Rust release validation/publication | 1000 | 1.327 | 1.418 |

Minified runtime-only bundle: **28,829 bytes**, excluding UI, JSX runtime and dev Refresh.
The 1,000-node pass alone does not demonstrate N2 improvement or certify the
complete performance gate. Existing tracked evidence files remain historical
because this lane owns only `src/**` and the two API/eval documents.

Reproduction output SHA-256 (stdout JSON, excluding Bun's stderr command echo):

| Run | SHA-256 |
|---|---|
| eval.json | `ead47a732e5a0b7845ff419a9562625b7f274fb7fb2b67f67ed719243fd6570c` |
| scaffold.json | `870371c3dee4829b6f529b807a85e58c311416a9c68f04711b6550da8df8c5c8` |
| bench.json | `2069fccf7a78d343ef20e0af8c5588b895539d5b05b9eafc24b1990a9e424935` |
| panel-bench.json | `2607d3f54ddddbf69fee8f18e6e91f34586235e038858e5e53721fc26ef4eca5` |
| rust-budget.json | `a36cbb154387b9b02f00bbc64471d23a6f5a06b8a8f1b7a8d4a130eec4cbf03b` |

## Historical pre-fix performance

One sequential Linux run, 30 warmups and 100 measured samples per workload.
JS numbers include reconciliation/op generation/accepted inspection with a
synthetic immediate async ack. Rust measures decode/stage/validation/publication
in a separate workload. They exclude IPC/kernel/layout/paint/presentation and
must not be added as percentiles or called an end-to-end pass.

| Diagnostic | Native nodes | p50 ms | p95 ms | Verdict |
|---|---:|---:|---:|---|
| JS benchmark (499 widgets) | 1000 | 1.552 | 3.577 | Pass public provisional 4-ms diagnostic |
| JS benchmark (1000 widgets) | 2002 | 3.740 | 5.977 | Fail public provisional 4-ms diagnostic |
| Rust release budget example | 1000 | 1.548 | 1.749 | Separate CPU diagnostic only |
| timeline-10000-window32 | 65 | 2.238 | 3.427 | Behavior subset only |
| inspector-50-properties | 253 | 0.790 | 2.000 | Behavior subset only |
| effects-1000-items | 1007 | 4.407 | 5.803 | Behavior subset only |

Runtime-only minified bundle: 19,497 bytes (excludes UI, JSX runtime and dev
Refresh). Effects has roughly 1,000 retained nodes because labels are semantic
props; its pure category/group filtering still visits the whole collection.
The bounded timeline proves no 10,000-row subtree, not sublinear query cost.
Raw measurements: `eval/evidence/bench.json`, `panel-bench.json`,
`rust-budget.json`. Larger JS and effects workloads exceed the provisional
ceiling on this host. Signed sandboxed V8 and physical macOS presentation remain
unmeasured; this is no basis for claiming N2 improvement over shipped Electron.

## Historical verification and current reproduction

- Locked native workspace: **35 Rust tests passed**, no ignored tests. The same
  35 tests also pass through the standalone lock; this is not 70 distinct tests.
- `cargo clippy --locked -p pm-surface --all-targets -- -D warnings` and rustfmt
  checks passed. No Rust source was changed.
- **109 Bun tests passed, zero skipped with the wire configured**: 55 existing
  tests plus 54 evaluation/port tests. Two regression tests assert known eval
  failures, and ten assert pending-owner. A green test suite is not 40 task passes.
- Base TypeScript typecheck, separate eval/examples typecheck, original Prettier
  check and an explicit examples check passed. No API/source formatting changed. All three panel entry points also build for
  the browser target without Node dependencies.
- Independent CLI exits **1**, deliberately reporting 28 pass / 2 fail / 10
  pending. Original scaffold CLI exits 0, 30 pass / 0 fail / 10 pending.
- Without `PM_SURFACE_WIRE`, the new native integration tests skip explicitly;
  the existing non-wire suite remains green. Official evidence always uses wire.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
cargo clippy --locked -p pm-surface --all-targets -- -D warnings
cargo fmt -p pm-surface --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --example wire
cd js/v4
bun install --frozen-lockfile
PM_SURFACE_WIRE=../../crates/surface/standalone/target/debug/examples/wire bun test
bun run typecheck
bunx tsc -p eval/tsconfig.json
bun run format
bunx prettier --check examples
# Current expected CLI result: 30 passes, 0 failures, 10 pending-owner.
bun run eval --include-held-out --wire=../../crates/surface/standalone/target/debug/examples/wire
bun run eval --include-held-out --scaffold --wire=../../crates/surface/standalone/target/debug/examples/wire
bun run bench
bun eval/panel-bench.tsx
cd ../..
cargo run --release --locked --manifest-path crates/surface/standalone/Cargo.toml --example budget
```

## Rule-selected decisions and remaining gate

| Decision | Selecting rule |
|---|---|
| Preserve independent failures and distinguish metadata acceptance from renderer behavior; never inflate 30 briefs into 40 passes | N1/N3 |
| Keep all old scaffold consumers and v1–3/built-in code; remove nothing this lane does not retire | N4/N6 and prune rule |
| Original evaluation proposed changes only; this fix lane implements the owned API contracts and retains freeze authority with the orchestrator | Lane ownership and orchestrator freeze authority |
| Use public package exports, declaration signatures, IDs and black-box feedback | N7; third-party evaluation constraint |
| Inject immutable synthetic panel inputs/requests and mark production conversion blocked | N3/N7/N8; pm-kernel remains the sole writer |
| Default cancellation stays declared natively; the test planner never implies synchronous native JS | N8; v4 contract does not alter N6 compatibility |
| Preserve exact Unicode, finite JSON, false/empty/null and numeric values; do not imitate browser coercion | N3 and quirk policy |
| Use neutral semantic placeholders and explicit design TODOs | Design ownership; Opus owns visual/UX decisions |

Before freeze: wire and verify the concrete kernel/permission/resource adapters;
update the two stale test assertions with ownership approval; supply owner
specifications and the exact private threshold; run real compiler/startup/recovery
integration; meet signed V8 and full native budgets. A4/A7 available-task failures
are resolved. The ten owner records remain pending-owner throughout.

On macOS verify every primitive/control renderer, virtual-window publication,
resources/anchor placement, native default cancellation, focus/keyboard ownership,
AX associations, IME/UTF-16 composition/caret/clipboard, pointer capture/lock and
drag payloads. Drive the three ports against real kernel queries/commands/bindings
and durable settings, including project switch/disposal/lost ack. Verify native
appearance against pm-design and Opus review, and measure input-to-presentation
at display scales/120 Hz with no main-thread JS waits. No Linux result certifies
those gates or v1–3 compatibility cutover.


## Merge-ready follow-up — 2026-11-01

Merged `origin/feat/native-engine` at
`4204a153dbf8af916ef2b92d7ca01858e363516a` in merge commit `9f1d789d8`.
Merged/check-confirmed `origin/feat/na-v4-integrate` at
`483ff243da57b99f61981eab8f57de4b401be771` (already an ancestor).
The sole merge conflict was `apps/native/Cargo.lock`: retain the upstream
superset, adding four panel-model package entries while preserving all surface
and design dependencies. No dependency versions were upgraded. `engine/Cargo.lock`
and the standalone lock require no changes. No new crate or workspace member was
introduced. Unowned files imported by the upstream merge were not edited locally.

File-URL conversion now decodes native test/build paths and safely constructs the
candidate-module base URL. A regression runs candidate evaluation from a directory
containing spaces and `#`, canonicalizing its own synthetic temporary directory;
it assumes neither the spelling nor ownership of macOS system temp symlinks.
The owned tests use no `/proc`, ffmpeg/ffprobe, GNU-only command-line flags or
unavailable spike fixtures. Test manifests and fixture sources live under the owned
surface/JS paths; pm-design remains an existing declared workspace dependency.
N3/N7 select exact path/identity handling; the merge-ready portability rules select
system-neutral temporary-directory handling. No visual or UX behavior changed.

Post-merge verification (same Linux/Bun/Rust versions as above):

```sh
# apps/native
cargo test --locked -p pm-surface
# 95 passed, 0 failed, 0 ignored
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml
# Same 95 tests passed, not additional distinct tests
cargo clippy --locked -p pm-surface --all-targets -- -D warnings
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml --all-targets -- -D warnings
cargo fmt -p pm-surface --check
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --examples
# All pass

# apps/native/js/v4
bun test
# 172 passed, 0 failed, 0 skipped
bun run typecheck
bunx tsc -p eval/tsconfig.json
bun run format
bun run build
# All pass
bun run eval --include-held-out
bun run eval --include-held-out --wire
bun run eval --include-held-out --scaffold --wire
# Each: 40 passed, 0 failed, 0 pending; 10 ownerProductionBlocked
```

The tracked integration JSON/hash records above remain the prior successful
rerun's evidence; the post-merge rerun preserves their score and production gates.
The extra portability regression raises the Bun count from 171 to 172.
**Production freeze remains HOLD.** The KernelData application binding,
permission/media/resource adapters and macOS checks listed above remain open:
IME/caret/clipboard, native focus and AX/VoiceOver, rendering/Opus review, input
routing and physical presentation performance. No new DESIGN-TODO(opus).
