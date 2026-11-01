# v4 API evaluation before freeze — package 14

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
The observed score is below any available-task requirement greater than 28/30;
no full 40-task requirement can be certified without the owner specifications.
The public benchmark's separate provisional p95 ceiling is 4 ms; it is not an
invented substitute for the private API-usability threshold. The orchestrator
must apply the actual threshold before changing this recommendation.

Evaluation base: `09af1206162f5a44401b777bcb9ac2e3bd07421b`. Linux x86_64,
Bun 1.4.4; Rust checks use repository-pinned Rust 1.96.1. Synthetic data only.
The frozen behavior source is commit `07ed5fa17ee640659cb075f9cad1af2095011d60`
(v1.2.0). Owner-source approval in `p0-decisions.md` permits anonymized behavior
specs, but none of the ten reserved owner briefs was provided here.

## Method and limits

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

## Per-task result

Total task code/checks: **630 lines**; **47 public-reference consultations**.

| Task | Verdict | LOC | Lookups | Wall ms | Commits / ops | Friction |
|---|---|---:|---:|---:|---:|---|
| builtin-01 | pass | 25 | 2 | 8.86 | 1 / 20 | A1, A2 |
| builtin-02 | pass | 14 | 2 | 5.84 | 2 / 17 | A1 |
| builtin-03 | pass | 16 | 1 | 2.77 | 1 / 12 | A3 |
| builtin-04 | pass | 16 | 2 | 3.46 | 1 / 11 | A3 |
| builtin-05 | pass | 15 | 2 | 7.43 | 2 / 20 | A2 |
| builtin-06 | fail | 17 | 2 | 14.23 | 1 / 5 | A4 |
| builtin-07 | pass | 26 | 1 | 3.53 | 1 / 24 | A1, A2 |
| builtin-08 | pass | 14 | 2 | 3.56 | 1 / 27 | A1, A3 |
| builtin-09 | pass | 22 | 2 | 4.12 | 1 / 29 | A2 |
| builtin-10 | pass | 18 | 1 | 2.90 | 1 / 11 | A2 |
| wild-01 | pass | 18 | 1 | 2.89 | 1 / 22 | — |
| wild-02 | pass | 35 | 2 | 5.07 | 2 / 16 | A3 |
| wild-03 | pass | 11 | 1 | 2.89 | 1 / 10 | A3 |
| wild-04 | pass | 12 | 2 | 2.46 | 1 / 10 | A3 |
| wild-05 | pass | 24 | 2 | 8.02 | 2 / 115 | A3 |
| wild-06 | pass | 25 | 2 | 6.55 | 2 / 26 | A2, A3 |
| wild-07 | pass | 15 | 2 | 3.05 | 1 / 21 | A3 |
| wild-08 | pass | 31 | 1 | 3.35 | 1 / 21 | A1, A2 |
| wild-09 | pass | 6 | 1 | 4.20 | 1 / 4 | A3 |
| wild-10 | fail | 35 | 2 | 5.69 | 2 / 9 | A7 |
| held-out-01 | pass | 17 | 1 | 5.28 | 2 / 22 | — |
| held-out-02 | pass | 37 | 2 | 5.16 | 2 / 14 | A4 |
| held-out-03 | pass | 25 | 2 | 4.88 | 2 / 10 | A2, A4 |
| held-out-04 | pass | 18 | 1 | 4.67 | 2 / 10 | — |
| held-out-05 | pass | 25 | 1 | 2.48 | 1 / 6 | — |
| held-out-06 | pass | 24 | 1 | 8.15 | 3 / 18 | — |
| held-out-07 | pass | 37 | 2 | 7.83 | 3 / 8 | A5 |
| held-out-08 | pass | 29 | 1 | 3.19 | 1 / 18 | A2 |
| held-out-09 | pass | 10 | 1 | 2.52 | 1 / 4 | — |
| held-out-10 | pass | 13 | 2 | 8.54 | 2 / 5 | A6 |

`owner-01` through `owner-10`: **pending-owner**; no implementation, LOC, lookup
count or score is fabricated. Raw task results and diagnostics are in
[`eval/evidence/results.json`](../../apps/native/js/v4/eval/evidence/results.json).
The ten owner tests assert pending status; they are not ten successful tasks.

## Findings and authoring corrections

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

## Ranked proposals before freeze (no API changes implemented)

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

## Performance

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

## Verification and reproduction

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
# Expected exit 1 until A4/A7 are addressed; do not convert the failures to passes.
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
| Propose API changes only in this document; no freeze-changing runtime edits | Lane ownership and orchestrator freeze authority |
| Use public package exports, declaration signatures, IDs and black-box feedback | N7; third-party evaluation constraint |
| Inject immutable synthetic panel inputs/requests and mark production conversion blocked | N3/N7/N8; pm-kernel remains the sole writer |
| Default cancellation stays declared natively; the test planner never implies synchronous native JS | N8; v4 contract does not alter N6 compatibility |
| Preserve exact Unicode, finite JSON, false/empty/null and numeric values; do not imitate browser coercion | N3 and quirk policy |
| Use neutral semantic placeholders and explicit design TODOs | Design ownership; Opus owns visual/UX decisions |

Before freeze: resolve A4/A7 semantics and re-run; publish/implement the missing
host/event/primitive contracts; supply owner specifications and the exact private
threshold; run real compiler/startup/recovery integration; meet signed V8 and
full native budgets. The ten owner records remain pending-owner throughout.

On macOS verify every primitive/control renderer, virtual-window publication,
resources/anchor placement, native default cancellation, focus/keyboard ownership,
AX associations, IME/UTF-16 composition/caret/clipboard, pointer capture/lock and
drag payloads. Drive the three ports against real kernel queries/commands/bindings
and durable settings, including project switch/disposal/lost ack. Verify native
appearance against pm-design and Opus review, and measure input-to-presentation
at display scales/120 Hz with no main-thread JS waits. No Linux result certifies
those gates or v1–3 compatibility cutover.
