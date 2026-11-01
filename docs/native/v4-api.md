# v4 native extension surfaces (P3 packages 13/14)

Status: headless API and eval scaffold; **native rendering/cutover gates open**.
Owned implementation: `apps/native/js/v4` (`powermove-v4`) and
`apps/native/crates/surface` (`pm-surface`). No Electron, compatibility realm,
project/profile data, built-in panel, vendor, or application source is replaced.
The frozen control source is `v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`).

## Protocol and ownership

JSX uses pinned Preact 11.1.1 with a virtual element adapter. Nodes obtain their
own virtual owner document; **no global DOM, Node or Chromium is needed**. This
adapter has no browser layout/style query API. The browser-target bundle runs in
a test context without `process`, `Buffer`, `window` or `document`. `@prefresh/core`
1.5.9 is a separate development entry. Compatible hook state survives Refresh;
compiler-supplied changed hook signatures or explicit reset discard incompatible
state. The compiler must register stable module/export identities and signatures.

`createRoot(transport, onError?)` exposes `render`, `flush`, `dispatch`,
`setStyleRules`, `inspect`, and `dispose`. Hook updates schedule worker microtasks
and commits even without another UI event. Explicit flush is available to headless
hosts. Roots share Preact's update queue but own documents, IDs, commit queues and
accepted snapshots. The optional error callback and fenced inspector report
asynchronous failures. Hosts must dispose roots when retiring a panel/realm.

The transport is asynchronous, owned by the JS worker. It sends exactly one
complete batch at a time per root, then awaits `{accepted, revision, error?}`.
The UI thread never waits on JS. A native worker validates/stages; the shell
receives an immutable accepted `Arc<Surface>` through `NativeHost::enqueue`.
`InspectHost` is the Linux/headless publication double. A renderer implementation
must enqueue, not call JS synchronously. Kernel mutations stay with pm-kernel:
`BindingSink::enqueue` is the nonblocking adapter seam, not a second writer.

```json
{"baseRevision":0,"ops":[{"op":"create","id":1,"type":"button"},{"op":"setProps","id":1,"patch":{"label":"Run"}},{"op":"insertBefore","parent":0,"id":1,"before":null}]}
```

| Operation | Meaning |
|---|---|
| `create(id,type)` | Fresh monotonically increasing safe-integer ID; declared primitive only |
| `insertBefore(parent,id,before)` | Insert/reparent/move; null anchor appends; an existing sibling is required |
| `remove(id)` | Explicit subtree destruction; root 0 cannot be removed |
| `setProps(id,patch)` | Structured finite JSON data; null removes a property |
| `setText(id,text)` | Exact authored text; cannot implicitly destroy child nodes |
| `setListeners(id,flags)` | Replace the complete event-declaration map; callbacks stay in JS |

Root 0 exists before the first commit. IDs are local to a surface; routes must
also carry the owning panel/realm generation. Acknowledged IDs never recycle, so
late events cannot target a different element. Rejected IDs are not consumed.
Temporary detached nodes are allowed inside a batch; the final tree must be rooted,
acyclic, uniquely parented and fully attached. Leaves cannot contain children.
Known malformed prop types, stale revisions, unknown primitives, missing targets,
bad anchors, unsafe/reused IDs, cycles, orphan nodes and resource-limit violations
reject the **entire** commit. Nodes, input state, styles, IDs and revision remain
unchanged. Unknown properties are retained and returned as `unknownProp`
diagnostics, never fatal merely because their name is unknown.

Defaults: 100,001 retained nodes including root; 600,000 operations; 256 levels;
16 MiB encoded commit. Limits are configurable by the host. Encoded byte length is
checked before wire decoding and again for directly constructed commits. JSON
wire decoding rejects unknown operation/envelope fields. An accepted batch advances
one revision, including an explicit empty batch; JS avoids sending empty batches.

A rejection/transport error fences that JS root and preserves its last accepted
inspector snapshot. Further speculative work cannot diverge silently. The runtime
owner must recover/remount that realm against an explicitly reset surface and
restore extension state; automatic realm recovery is **not implemented here**.
Do not retry an ambiguous lost acknowledgement as a new revision. Generation/ack
recovery belongs in the runtime transport adapter.

## Primitives and style

All primitives have native tree representations: `div`, `text`, `img`, `input`,
`textarea`, `button`, `scroll`, `virtual-list`, `anchored`, `path`, `canvas`,
`frame`, `markdown`, `code`, `Glass`, `material`, `webview`. Resource IDs/data stay
structured; `canvas`/`frame` are handles for native renderers, not DOM canvases.
`webview` creation requires explicit host opt-in (default false). This opt-in is
not a network permission grant or replacement for Store permission checks.
Virtual-list range/count/item-size data is bounded by the renderer's visible
window; no million-row JS subtree is generated by the scaffold fixture.

Styles support flex/grid data and semantic token references, e.g.
`{display:'grid', gap:{token:'metrics.gap'}}`. `pm-design` owns token resolution,
material eligibility, accessibility fallbacks and all visual defaults. The tree
validator does not invent a CSS renderer or browser quirks. `StyleSheet` supports
id, class, role, attribute equality, state equality and conjunction selectors.
ID specificity precedes other terms; ties use source order; inline style wins.
`root.setStyleRules(rules)` writes root `styleRules` through `setProps`, so a rule
replacement and other tree changes share the same atomic boundary. Invalid rules
keep the previous tree/styles. Arbitrary CSS syntax, pseudos and descendant
selectors are not claimed. The renderer still must implement layout/painting.

## Interaction and bindings

The native `Interaction` planner emits capture (root → target), then bubble
(target → root) deliveries. `setListeners` maps event names to
`{capture,bubble,preventDefault,keys}`. An empty key list applies to all keys;
otherwise cancellation applies only to the declared exact key names. Native
cancellation happens **before enqueueing**, independently of JS latency. JS
`stopPropagation` and `stopImmediatePropagation` control subsequent JS deliveries.
Calling `preventDefault()` after dispatch cannot retroactively cancel native
behavior; it is a no-op if already declared/cancelled, otherwise a reported error.
This is a new v4 contract; v1–3 DOM event semantics are unchanged.

Pointer capture routes by pointer ID until release/cancel, and is retired when its
element disappears. `Drag` uses logical points and starts at exactly **3 pt**.
Focus scopes trap traversal, reject out-of-scope focus and retain a restore target;
tab skips disabled/negative-tab-index controls. Host adapters must call interaction
and binding reconciliation only after accepted commits. AppKit IME, keyboard
ownership, focus-request application, accessibility, hit testing, pointer lock/
infinite scrubbing, haptics and physical coordinate conversion remain integration
checks on macOS.

For controlled text inputs, native observations carry value, UTF-16 selection and
composition state. An incoming JS value is assigned only when different from the
last native observation. Echoes preserve caret, composition and native write count.
A changed external value clamps the selection to valid UTF-16 length. Numeric
native controls accept a document number and keep their editing buffer separate.
JS sees native input observations before invoking its handler.

Rust `Bindings` owns begin/write/commit/cancel and cancellation on destroy,
disable or rebinding. The original binding handle owns cancellation. Queue failures
remain retryable; a failed begin does not open a gesture. `Controls` implements
live text edit/blur/Enter/Escape, finite numeric typing/arithmetic, clamped arrows,
typed select values/declined UI-only choices, and mixed toggle keyboard semantics.
The scrub calculation preserves right/up increase, Shift ×10, Option ×0.1 and
bounds. Text commit and scrub clamp paths are deliberately separate (W073 B567).
No JS callback is required for these Rust binding operations.

## `powermove/ui`

The package exports `powermove-v4/ui`. The extension compiler must alias
`powermove/ui` to that export (the package tsconfig demonstrates the mapping);
this lane cannot change the compiler outside its owned paths. JSX uses
`jsxImportSource: 'powermove-v4'`, or the corresponding compiler alias. Hooks are
exported at `powermove-v4/hooks`.

All tagged component names are present: BackTitle, ColorField, Disclosure,
FillField, FontField, NumField, PointField, RampField, Row, Section, Segmented,
SelectField, SliderField, TextField, ToggleField; plus FieldMenu, Menu,
ToolbarItem and Glass. V4 accepts data values and opaque Rust binding handles
instead of legacy live `api/get/edit` closures. Row label association uses context;
Disclosure, Segmented and Menu have tested state/navigation behavior. Native
field descriptors preserve typed values/options, precision, unit, bounds, mixed
state and original binding identity. Pure tagged numeric arithmetic and colour
math (sRGB/P3/HSV/HSL/OKLab/OKLCH, alpha and authored hex storage) are retained and
covered by tagged-derived tests. Colour-space logic does not choose UI colours.

These are semantic component ports and headless control state machines, **not a
claim that every original picker UI/control interaction is rendered**. Native
colour/fill/font/ramp/point surfaces, trackpad gesture dispatch, infinite scrubs,
linked-axis command integration, live time/query subscriptions, field-menu focus/
anchor/motion and kernel edit/invalidation adapters still need host wiring.

**DESIGN-TODO(opus):** approve v4 control/picker renderers, menu and anchored
surfaces, composition layout, and motion using `pm-design` and
`docs/native/design/`. Current components are neutral semantic placeholders with
no new visual defaults. All future visual output remains provisional until review.

## Eval and verification

`eval/tasks.ts` has 40 synthetic task records: ten built-in-style, ten wild, ten
held-out, ten owner-approved anonymized panel specs. The owner records include
inputs, interactions, required host reads/writes, edge cases and acceptance checks.
Their synthetic fixtures report `fixtureStatus` separately from full API
acceptance: unavailable host capabilities yield `status: fail`,
`failureKind: api-gap` and a `blockedBy` list. No private owner panel name, ID,
code or source is tracked. The held-out group runs only when explicitly selected;
because its specs are public this is a separation of evaluation sets, not a claim of secret tests.
`eval/scenarios.tsx` provides the original 30 scaffold fixtures and ten authored
owner behavior fixtures. Owner service responses and writes are test seams, not
working host services or independent extension ports. The runner reports failures
and returns nonzero. `--candidate=./module.ts` selects
a module exporting the same `createRoot` adapter API for reconciler comparisons. `--wire` additionally
replays every emitted batch into the real Rust validator and compares final
native types/children; this is a headless protocol subset, not visual acceptance.
The webview fixture opts in only in its test process.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
# Shared workspace lock integration is outside this lane. Linux harness, same sources/tests:
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface --all-targets -- -D warnings
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --example wire
cd js/v4
bun install --frozen-lockfile
PM_SURFACE_WIRE=../../crates/surface/standalone/target/debug/examples/wire bun test
bun run typecheck
bun run format
bun run eval --include-held-out --wire=../../crates/surface/standalone/target/debug/examples/wire
bun run bench
cd ../..
cargo run --release --locked --manifest-path crates/surface/standalone/Cargo.toml --example budget
```

The requested workspace `cargo test --locked -p pm-surface` stops because adding
the member requires shared Cargo.lock reconciliation. This lane leaves that
unowned lock untouched and tests the same library, integration tests and examples
through the checked-in standalone lock. No macOS binaries or existing app tests
are claimed verified by these Linux checks.

## Rule-selected decisions

| Decision | Selecting rule / evidence |
|---|---|
| Real pinned Preact first; separate development Refresh entry | Smallest passing candidate rule; S1's 32,129 vs 705,706-byte diagnostic favors Preact; production native/macOS gate still open |
| Atomic staged tree + stylesheet validation; reject without partial publication | N1/N3: authored intent/correctness; valid leading ops cannot leak through an invalid tail |
| Keep all v1–3 paths and shipped components intact | N4/N6 and prune rule: this opt-in v4 API has not retired any old consumer |
| Structured data instead of HTML/CSS serialization | N3 and quirk policy: false, empty strings, typed values and exact authored text survive without DOM property coercion |
| Native declared defaults, focus/capture and Rust binding seam | N7/N8: inspect/drive by stable ID; no main-thread JS waits or second document writer |
| Monotonic element IDs + per-root revisions + explicit fencing | N3/N7: stale events or ambiguous acknowledgements cannot silently address new state |
| Host opt-in for webview; no browser dependency for other primitives | N8; native operation and bundled JS pass document-free tests |
| Opaque missing owner specs remain pending | N1/N3: 30 available scenarios cannot be called 40 passes |
| Neutral semantic controls and token references | Design ownership rule: no lane-selected visual language; DESIGN-TODO(opus) above |

Performance evidence and final verification counts follow in the lane result
update below. Linux timings do not approve the signed V8/GPUI/physical macOS gate.

## Verified lane result (Linux, 2026-11-01)

- Rust 1.96.1: **35 tests passed** using the locked standalone manifest; clippy
  with `--all-targets -- -D warnings` and rustfmt check passed.
- Bun 1.4.4: **55 tests passed, zero skipped** with `PM_SURFACE_WIRE` set. This
  includes all 30 available eval cases replayed into the Rust validator, a
  rejection-tail/controlled-prop regression, compatible/incompatible Refresh,
  asynchronous hook publication and a document-free browser bundle smoke test.
- TypeScript typecheck, Prettier check, browser-target builds and frozen Bun
  install passed. CLI eval using `--candidate=./src/index.ts` and `--wire`
  reports **30 passed / 0 failed / 10 pending-owner**. These fixture checks do
  not establish full built-in conversion, visual parity or 40 owner passes.

CPU diagnostics (30 warmups, 100 samples; synthetic data only):

| Path | Native nodes | p50 / p95 ms | Scope/verdict |
|---|---:|---:|---|
| JS Preact, production runtime bundle 19,497 bytes minified | 1,000 | 1.737 / 3.145 | Pass provisional separate 4-ms diagnostic |
| JS Preact, same adapter, S1-size workload | 2,002 | 4.239 / 5.654 | **Fails** that provisional ceiling |
| Rust release atomic commit | 1,000 | 1.633 / 2.037 | JSON decode/staging/validation/publication only |

The JS runtime size excludes UI, JSX-runtime and development Refresh entry
bundles. Timing includes JS op generation and accepted inspector snapshots with
an immediate synthetic acknowledgement. Rust timing is a separate run/workload;
**do not add these percentiles or call either an end-to-end budget pass**. Earlier
JS 1,000-node p95 readings were 3.308 and 2.861 ms; the S1-size diagnostic remains
over budget. Preact is the smallest provisional candidate passing the available
1,000-node subset and Refresh here. N1/N2 keep final reconciler selection open
until signed V8, real transport, native validation/layout/paint and stable macOS
measurements pass the intended complete eval.

Remaining integration (outside owned paths): reconcile native Cargo.lock; add
compiler aliases and Refresh instrumentation; connect runtime generation/ack
recovery, kernel binding queues and live query/time publications; implement the
GPUI/AppKit `NativeHost` and every primitive/control renderer with pm-design
materials/tokens; apply focus/anchor/virtual-list/resource handling. On macOS verify
accessibility, IME/selection/clipboard and field shortcut ownership; default
cancellation and pointer capture/lock at display scales; layout/grid, anchored
menus and material accessibility fallbacks; no main-thread waits and physical
commit/presentation latency. Supply the ten owner specs before claiming the full
eval. See the DESIGN-TODO(opus) above for the provisional visual surfaces.
