# v4 native extension surfaces (P3 packages 13/14)

Status: merged UI-kit/API integration and concrete worker data service tested on
Linux; **application KernelData binding, native rendering and cutover gates open**.
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

`createRoot(transport, onError?, options?)` exposes `render`, `flush`, `dispatch`,
`dispatchNative`, `drive`, `observeInput`, `capturePointer`, `releasePointer`,
`setStyleRules`, `inspect`, `dispose` and `recover`. Options carry the surface
generation and an independent asynchronous native/worker retirement callback. Hook updates schedule worker microtasks
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

A rejection/transport error fences that JS root and preserves its accepted snapshot.
`dispose()` is idempotent and releases JS handlers, hook effects and local queues
even while fenced. Healthy disposal sends the normal remove batch; fenced disposal
sends **no additional surface mutation**. In both cases `options.retire(generation)`
is the independent native/worker cleanup route. The application must implement it
to release resources, seal the kernel writer and retire subscriptions. Starting
disposal refuses new work while already submitted acknowledgements settle.

`await root.recover(savedState, adapter)` first validates/copies finite JSON state,
then disposes and calls `adapter.reset({generation,nextGeneration,state})`. Only an
accepted reset naming the next generation and revision zero creates a new root.
The return value contains `{root,state}`; the owner remounts from that state.
A rejected reset preserves the checkpoint for explicit recovery. Never blindly
retry an ambiguous lost acknowledgement as another edit: resolve its kernel txid
or retire/reset its generation. This adapter contract is tested on Linux; the
application's realm restart, durable checkpoint and native reset wiring are outside
this lane. JS checkpoints alone are not a crash-persistent data store.

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

## Inspection and drive (ranks 1, 2 and 4)

`inspect()` returns `{revision,generation,nodes,acceptedNodes,inputs,fenced,disposed}`.
`acceptedNodes` contains exactly the last acknowledged six-op tree, including root
`props.styleRules`. `nodes` is the same tree with each observed input's current
string projected into `props.value`. `inputs[id]` exposes
`{value,selection,composing,composition,sequence}`. All inspector data is copied;
mutating a returned snapshot cannot alter the root. Native observations do not
consume a commit revision. Thus `nodes` is current inspection; use `acceptedNodes`
when comparing raw accepted commit props to Rust. The sequence orders observations
within a generation; it is separate from commit revision.

`observeInput(id, observation, revision?)` validates the complete observation
before changing anything. Selection and optional composition are ordered,
end-exclusive **UTF-16 code-unit offsets**, bounded by the string length. A null
composition means no marked range; a nonnull range requires `composing=true`.
An echo sends no redundant value operation and preserves native caret/IME state.
An external different value cancels composition and clamps selection in Rust;
JS clears the superseded observation until a new native publication arrives.
Node removal retires the observation. Values and offsets are never converted to
UTF-8 bytes or Unicode scalar indices.

```tsx
const fieldId = root.inspect().nodes.find(n => n.props.id === "caption")!.id;
await root.drive(fieldId, "input", {
  value: "A日本🙂B", selection: [3, 5], composing: true, composition: [1, 3]
}, root.inspect().revision, root.inspect().generation);
const observed = root.inspect().inputs[fieldId];
```

`drive(id,type,detail,revision?,generation?)` plans capture/bubble and native default
declarations from accepted listeners, applies the input observation before invoking
handlers, then flushes hooks. Pointer capture routes by pointer ID until release,
up/cancel or target removal. `dispatchNative({generation,plan,type,detail})` validates
the generation before dispatching a native envelope. `dispatch(plan,type,detail)`
remains the host's lower-level delivery entry. Native/AppKit planning is Rust
`Interaction::drive`; the JS planner is for headless hosts. Neither authorizes a
UI-thread wait on JS.

`EventDetail`/Rust `events::EventDetail` carry key/code/repeat, Shift/Option/Control/
Command modifiers, logical-point x/y and wheel deltas, pointer ID, button/buttons/
click count, value, UTF-16 selection/composition, virtual-list range and structured
`drag:{types,data,effect}` (`none|copy|move|link`). Native events have no browser
DataTransfer object. Event declarations include keyboard, pointer, composition,
selection, wheel, drag/drop and range-change handlers. Rust omits absent optional
fields when serializing. Runtime checks refuse malformed finite data before input
or callbacks change. Generation/revision checks reject stale delivery.

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

## Version 1 host service (rank 3)

The public package exports `HostClient`, `HOST_API_VERSION`, `HostRequest`,
`HostAck` and `HostPublication`. Rust exports matching serde envelopes and a
bounded, nonblocking `host_api::HostRouter<KernelQueue>` plus the concrete
`host_data::HostDataService<KernelData>`. No document/settings mirror is owned
by the client, router or data service. The application's `KernelQueue` adapter
must route requests to the **single pm-kernel writer**, and return its actual
acknowledgements. There is no success-shaped default backend.

Each request names `{version:1,realm,generation,project,requestId,expectedRevision,
operation}`. IDs are safe integers. Acks echo the identity/request ID and carry
`{accepted,revision,value?,error?}`. Settings additionally carry
`expectedStateRevision`/`stateRevision`: profile state and project document revisions
are independent. The adapter must never compare a profile revision with a document
revision. Failed queues are not accepted; ambiguous transport/ack failures fence
further edits. Native routing retains up to 4,096 submitted IDs and completed acks,
with at most 256 outstanding requests. Exhaustion requires a generation reset;
old edits are never silently replayed. Polling uses `try_recv`, never a wait.

| Client operation | Native/kernel adapter obligation |
|---|---|
| `registerBinding(handle,channels)` / `disposeBinding(handle)` | Register generation-owned channel handles; cancel original gestures before retiring handles. Only use a handle after its registration ack. |
| `query('project'|'selection'|'time'|'evaluated',channels?,time?)` | Read an immutable publication/QueryMirror at the ack revision; never publish a JS document copy. |
| `subscribe(id,query,callback,channels?)` / returned async unsubscribe | Publish versioned identity, subscription ID, revision, monotonic sequence and value. Client rejects stale, duplicate, foreign-project and retired-generation publications. |
| `edit(label,commands)` | Admit up to 256 typed `{command,args}` entries as **one** undoable actor transaction, with atomic rollback and actual acknowledgement. No per-command optimistic writes. |
| `command(id,args)` | Resolve the authorized typed command and use `KernelHandle::call/call_with`, preserving request-ID → kernel-txid dedupe and revision admission. |
| `getSetting(key)` / `setSetting(key,value)` | Namespace to the extension, preserve arbitrary JSON fields, route durable changes through `KernelHandle::state_transaction`; ack only actual kernel/persistence results. |
| `retire()` and native queue retirement | Refuse new JS requests; independently seal the realm writer and release binding/subscription resources through the actor. |

```ts
const host = new HostClient({realm: "extension", generation: 1, project: projectId},
  request => nativeWorker.enqueueKernel(request), publishedRevision);
await host.registerBinding("position.x", [channelId]);
const stop = await host.subscribe("selection", "selection", value => publish(value));
await host.command(commandId, args); // actual ack, never optimistic local mutation
await stop();
```

`HostDataService` implements the query/subscription lifecycle, rather than leaving
it to a success-shaped transport stub. Its `KernelData::query` reads one immutable
kernel publication (or a worker-owned QueryMirror synced to it), returning
`QuerySnapshot {revision,value}`. `project` exposes the authorized project snapshot;
`selection`, `time` and `evaluated` expose the corresponding authorized projections.
Channels and evaluation time pass through unchanged. The data service never owns
a writable document. `KernelData::enqueue` forwards edits, commands, bindings and
settings to the actor. A failed enqueue produces no ack; a delayed actor ack stays
pending. Only the actual actor result reaches the client.

Subscribe acks contain the initial query value at their revision. The JS client
buffers publications while registration is pending, delivering only after its ack;
a newer publication wins over that ack's older initial value. After each kernel
or transport publication wake, the worker calls `HostRouter::publications()` and
queues its returned envelopes to JS. Time changes can publish at the same document
revision. Sequence numbers start at 1 and increase per subscription; stale or
unchanged snapshots produce no invented changes. A failed query leaves all cursors
unchanged. The worker must retain returned envelopes until its JS delivery queue
accepts them. Active subscriptions are capped at 256; up to 4,096 registered IDs
are retained per generation. An acknowledged subscription ID never recycles in
that generation, including after unsubscribe, so late deliveries cannot address a
new subscription. The client serializes requests against the last actual ack and
copies authored arguments before queueing, preserving concurrent edit order.

The executable Linux `KernelData` double and standalone `host_wire` example prove
public-client/native-service routing, acks, rejection, stale generations, dedupe,
query arguments, subscription races, same-revision time changes and retirement. **A concrete KernelHandle adapter is not
linked here:** the owned surface manifests have no pm-kernel dependency, and the
application runtime is outside this lane. Production queries, binding lifecycle,
settings durability and commands must be verified against that adapter before
freeze. The worker data service is implemented; its application trait implementation and
production edits/settings durability remain integration gates. The `host_wire`
backend is explicitly a synthetic test double, with no persistence or media claim.

## Strict primitive authoring (rank 5)

`PrimitiveProps` and JSX intrinsic declarations have no unknown-property index
signature. `metadata:Record<string,Json>` is the explicit extension-data bag;
`attr`/`state` retain selector semantics. Raw six-op wire callers retain the prior
unknown-property diagnostic/retention behavior for existing consumers (N4/prune),
which does **not** advertise native support. Known native properties are checked
for primitive eligibility and shape by Rust, including atomic range/count checks.

| Primitive | Specific supported data schema |
|---|---|
| `div`, `text`, `button`, `scroll` | Shared semantic/control/style/event props; exact authored child text |
| `input`, `textarea` | Text value; numeric input controls accept document numbers, retaining a separate edit buffer. Bounds/step/speed/precision/unit are typed; step/speed positive, precision a nonnegative safe integer. |
| `img` | `src` or `resource`: opaque string lease; optional `alt` |
| `virtual-list` | Safe-integer `count`, `range:[start,end]` with `0≤start≤end≤count`, positive finite `itemSize`; native publishes `rangechange`, extension republishes only that window |
| `anchored` | `anchor`: accepted node numeric ID **or `{x,y}` logical point**; optional placement string. Point anchors need no ID lookup commit. Element anchors are acquired from `inspect().nodes` after acceptance; removed anchors require re-publication. |
| `path` | Exact authored `d` string or `points:Array<{x,y}>`; finite coordinates, no implicit rounding |
| `canvas`, `frame` | `surface` or `resource` opaque string lease; frame optionally has finite `time` |
| `markdown`, `code` | Exact `source` string or authored child text; code optionally has `language` |
| `Glass`, `material` | Semantic material token/name; Glass optionally carries surface identity; no lane-selected rendering defaults |
| `webview` | `src` or `source` string; creation is still host opt-in, and navigation permissions require separate host authorization |

A resource handle is scoped to its owning realm generation. The host retains it
through native/GPU consumption and releases it on accepted replacement/removal or
independent generation retirement; JS strings do not create resources. A retired
handle must fail visibly, never resolve to another generation. Native resource
registry/renderer wiring remains an application gate. `focusRequest` is an explicit
host-consumed string request; native focus ownership/ack and AX behavior must be
verified on macOS. No schema acceptance proves a renderer implements that behavior.

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

## Compiler aliases and development boot (rank 7)

`compilerAliases` and `compilerOptions` are public immutable exports. The compiler
must apply exact module mappings before resolving imports:

```ts
{
  "powermove/ui": "powermove-v4/ui",
  "powermove/hooks": "powermove-v4/hooks",
  "powermove/jsx-runtime": "powermove-v4/jsx-runtime",
  "powermove/jsx-dev-runtime": "powermove-v4/jsx-dev-runtime"
}
// jsx: 'react-jsx', jsxImportSource: 'powermove-v4'
```

Development host startup must happen in this order (separate module loading is
intentional; a static extension import can run before the startup body):

```ts
await import("powermove-v4/refresh");
const extension = await import(extensionModule);
const root = createRoot(transport);
await root.render(extension.view());
```

The compiler registers stable module/export identities and compatible hook
signatures through the public Refresh functions; incompatible signatures reset
state. Production hosts omit Refresh. Late initialization is unsupported; it
cannot retroactively instrument existing roots. The real compiler/import map and
V8 startup adapter are outside these owned paths; the exports and clean-process
startup test make their required contract concrete.

Inspect generated semantic output instead of guessing component internals:
PointField's axis handles are in `props.attr.xBinding/yBinding` while its top-level
binding remains present. Menu labels are child text, not `props.label`.
Disclosure/Row/Menu generate semantic IDs; use labels/associations or inspect the
accepted numeric ID. No renderer/component implementation was changed here.

## Eval and verification

`eval/tasks.ts` has 40 synthetic task records: ten built-in-style, ten wild, ten
held-out, ten owner-approved anonymized panel specs. The owner records include
inputs, interactions, required host reads/writes, edge cases and acceptance checks.
Their fixtures now report headless `status`, `fixtureStatus` and `serviceStatus`
separately from `productionReady: false`, `fullAcceptance` and `integrationGates`.
A headless pass requires executable public-client/native-data-service checks and
the authored behavior fixture; static absence strings no longer determine that
score. **This is not full owner-panel acceptance:** permission/media/resource and
application wiring gates remain explicit even when the headless score is 40/40. No private owner panel name, ID,
code or source is tracked. The held-out group runs only when explicitly selected;
because its specs are public this is a separation of evaluation sets, not a claim of secret tests.
`eval/scenarios.tsx` provides the original 30 scaffold fixtures and ten authored
owner behavior fixtures. Each owner run exercises the public HostClient through
the real Rust HostRouter/HostDataService and synthetic KernelData backend. The
annotation fixture additionally uses public project/time queries, time subscription,
acknowledged edits and current text through `inspect().nodes`; rejected edits
retain its observable draft. Other domain/media behavior still uses authored test
seams. These are not independent production extension ports. The runner reports failures
and returns nonzero. `--candidate=./module.ts` selects
a module exporting the same `createRoot` adapter API for reconciler comparisons. `--wire` builds the standalone examples and additionally
replays every emitted batch into the real Rust validator and compares final
native types/children; this is a headless protocol subset, not visual acceptance.
The webview fixture opts in only in its test process.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
# Linux harness, same sources/tests:
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface --all-targets -- -D warnings
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --examples
cd js/v4
bun install --frozen-lockfile
bun test # preload builds all native examples; no skipped conformance tests
bun run typecheck
bun run format
bun run eval --include-held-out
bun run eval --include-held-out --wire
bun run bench
cd ../..
cargo run --release --locked --manifest-path crates/surface/standalone/Cargo.toml --example budget
```

The current locked workspace supports `cargo test --locked -p pm-surface`.
The standalone wire harness still validates identical surface sources. No macOS
binary or existing app test is claimed verified by these Linux checks.

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
| Owner specs run with explicit headless/full-acceptance reporting | N1/N3: 40 headless passes do not certify production owner ports; native resource and application gates stay open |
| Queries read immutable publications; edits forward one transaction through KernelData | N3/N7/N8: pm-kernel remains the single writer; no UI-thread wait or local optimistic document |
| Registration ack supplies initial state; subscription IDs never recycle | N3/N7: avoid lost updates and stale delivery into a replacement subscription |
| Neutral semantic controls and token references | Design ownership rule: no lane-selected visual language; DESIGN-TODO(opus) above |

Performance evidence and final verification counts follow in the lane result
update below. Linux timings do not approve the signed V8/GPUI/physical macOS gate.

## Historical package-13 result (Linux, 2026-11-01)

The following numbers predate the pre-freeze fix pass. Current evidence and
freeze recommendation are in [v4-eval.md](v4-eval.md).

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

Remaining integration (outside owned paths): install the published compiler
aliases and Refresh instrumentation; connect the published generation/ack
recovery and kernel host contracts to the runtime and live query/time publications; implement the
GPUI/AppKit `NativeHost` and every primitive/control renderer with pm-design
materials/tokens; apply focus/anchor/virtual-list/resource handling. On macOS verify
accessibility, IME/selection/clipboard and field shortcut ownership; default
cancellation and pointer capture/lock at display scales; layout/grid, anchored
menus and material accessibility fallbacks; no main-thread waits and physical
commit/presentation latency. Supply the ten owner specs before claiming the full
eval. See the DESIGN-TODO(opus) above for the provisional visual surfaces.
