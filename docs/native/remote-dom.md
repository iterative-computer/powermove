# P2 remote DOM bridge

Status: **standalone protocol, router, shim and page applier implemented and
Linux-tested; full v1–3 compatibility/cutover and macOS latency remain OPEN.**
No production runtime placement, existing panel, migration reader or user data
is changed. The S5c architecture decision in `p0-decisions.md` takes precedence
over older “page-owned logic” descriptions in C-MATRIX/S1/S5.

## Components and ownership

* `apps/native/crates/remote-dom` — `pm-remote-dom`: serde protocol, complete-batch
  admission, generation/revision fencing, bounded nonblocking host queues and
  inspectable routing state. No V8, WebKit, GPUI, engine, Node or platform dependency.
* `apps/native/js/remote-dom/src/shim.ts` — V8-side `RemoteDOM`: a stable document,
  node identities, selectors, templates, forms, event dispatch and asynchronous
  layout cache. Extension closures stay here with PM and synchronous services.
* `src/applier.ts` — `PageApplier`: applies the protocol to a dedicated real
  document and serializes native DOM events/layout back. It evaluates no extension
  source. Script elements, inline event attributes, executable URLs, embedded
  browsing/plugin contexts and `srcdoc` are refused. The host must also enforce
  a private origin, channel authentication, navigation/asset policy and CSP.
* LinkeDOM **0.18.12** supplies the tree/parser. Property-sensitive selectors use
  css-select **5.2.2** / css-what **6.2.2**. Browser bundles contain these libraries;
  they have no Node runtime requirement. The mirror replaces LinkeDOM's simplified
  event dispatch and extends form state, namespaces, clone state and geometry.
  LinkeDOM is **not** a browser-conformance oracle.

Bun, jsdom, TypeScript, Prettier, the Svelte compiler and the Node-based bare-V8
probe are build/test tools only. None is required by the shipped app. A test
builds the shim for a browser target and executes it in a fresh V8 `vm` context
with no `process`, `require` or existing document. This is an execution/dependency
probe using Node's V8, not certification of the pinned rusty_v8 sandbox/runtime.

## Protocol and host handoff

Version 1 is JSON, with no function source, eval, pixel buffers or live service
objects. A session is `{panel, generation}`. Each generation gets its own router,
realm mirror and dedicated page. Never rebind an old node id to a new generation.
Both directions must use ordered queues. Disposing a page fences that generation;
restart with a larger generation and reconstruct from the owning realm/state.
Persistent project/profile state remains the kernel's responsibility.

Reserved ids: document **1**, html **2**, head **3**, body **4**, window **5**;
new nodes begin at **6**. All ids/sequences are exact positive JavaScript integers.
Detached nodes retain identity until generation retirement. `remove` detaches;
it does not destroy held objects. Fragments are local construction containers;
insert their children explicitly, as the shim does, rather than mounting the
fragment itself. The initial document roots must remain mounted.

A batch is `{version, session, seq, caused_by, ops}`. `seq` starts at 1 and is
contiguous; `caused_by` is null or the last delivered event sequence. Operations:

| Operation | Meaning |
| --- | --- |
| create | Element/namespace, text, comment or fragment, initially detached |
| insert / remove | Reparent/reorder using stable ids and a nullable before anchor |
| attribute / text | Exact strings; null removes an attribute; namespace is explicit |
| property | Typed allowlist for form values/checked/selected/disabled/etc.; never arbitrary property assignment |
| listen | Event type, enabled state, and explicit observe/block default-action policy |
| focus / blur | Focus command, with preventScroll for focus |
| selection | Input/textarea selection using UTF-16 offsets/direction |
| range | DOM selection anchor/focus ids and UTF-16/child offsets; null clears |
| scroll | Coordinates and auto/instant/smooth intent |
| measure | Asynchronous request for element rectangles, scroll/client dimensions and computed styles |
| observe_resize | Reference-counted subscription to page resize snapshots |

The page returns `applied`, `event`, `layout` or `fault`. `applied` acknowledges
DOM application, **not paint or scanout**. Events include their visible applied
revision, target id, flags and allowlisted scalar data; value, checked, selected
options, caret and related-target state are installed before realm callbacks.
Input/change, focusin/out and selectionchange have internal subscriptions even
when extension code has no listener. Native trust is carried as event metadata.
No DOM object or arbitrary callback crosses the boundary.

Rust validates against staged state before queue publication. It rejects unknown
ids, duplicate ids, cycles, invalid anchors, malformed/oversized messages,
unsupported versions, stale panels/generations, replayed events, unsafe property
names, executable content and invalid geometry. Failed admission or a full queue
does not advance the router. Its copy-on-write view per in-flight revision lets
an event from the still-visible tree survive a newer **queued** removal. Applied
acks retire older views. An unexpected page apply failure fences the generation
and preserves router inspection; there is no success-shaped partial recovery.

`TrySink<T>` has a no-wait/no-side-effect-on-error contract. `bounded_channel`
provides a std `SyncSender::try_send` implementation; the receiver is drained on
the actual page/realm owner. Full and disconnected queues are distinct failures.
The JS sender retains the exact refused batch and subsequent deltas; call
`flush()` when the native queue becomes writable. Applied feedback also wakes
pending work. Do not busy-poll, drop accepted keystrokes, or substitute a blocking
send/evaluateJavaScript/reply wait on the UI thread.

Default Rust admission limits: 1 MiB serialized messages, 16,384 operations,
100,000 retained nodes and 8 in-flight batches. The shim targets 2,048 operations
and a conservative 128 KiB operation estimate per chunk, preserving ordering
across large mounts. Limits are configurable and must agree at the host boundary.
A single operation is not truncated or split in the middle of an authored string;
a host-limit refusal must be surfaced/retried with an appropriate admission policy.
These limits do not authorize silently dropping panels or user content.

For a dedicated production V8 realm, construct `RemoteDOM` with
`realmGlobal: globalThis`, then call `installGlobals()`. This preserves
`window === globalThis` and the runtime's injected timers/network/PM globals.
That bootstrap mutates the dedicated realm; retire the realm as a unit. The
separate-window default and restore callback are for isolated tests/embedding
without taking over the host's own global object. Supply native frame scheduling
through the explicit options; do not invent display cadence in the shim.

## Event, form and layout semantics

Capture → target → bubble runs in V8 against a snapshotted propagation path.
Listener identity/capture, once, passive, AbortSignal, removal during dispatch,
stopPropagation/stopImmediatePropagation, `this`/currentTarget, on-event properties,
exception isolation and cancelable/defaultPrevented are tested. This deliberately
avoids copying LinkeDOM's bubble-only behavior or a Chromium event quirk (N3).

Form values have property state separate from default attributes. An incoming
native edit is installed before listeners and is not echoed back merely because
it was observed. A queued property/selection/focus update causally older than a
newer browser edit does not overwrite that edit; updates caused by the delivered
event remain eligible. Native select state includes the selected option indices,
so Svelte's `:checked` queries see current property state. Moving a real input
retains its identity. Cloning copies current form values and never listeners.
String-valued successful controls can be read with the realm FormData adapter;
file controls explicitly require host file capabilities.

Two important compatibility boundaries remain explicit:

1. **Dynamic cancellation of a real browser default is unresolved generally.**
   `observe` forwards the event without blocking its default. A later V8
   preventDefault is reported through `lateCancellation`, or recorded in
   inspection diagnostics if no callback is installed. `block` pre-cancels the
   default in the page, unconditionally. It is useful only for controls whose
   authored policy is already “always cancel”. It is not a general substitute for
   conditionally canceled text insertion, checkbox activation, navigation,
   clipboard, focus or IME. There is no generic default-action replay here. N4/N6
   keep these cases as cutover blockers; the host may not silently select a policy
   and declare all old panels compatible.
2. **Layout reads are synchronous, as a browser's forced layout.**
   `getBoundingClientRect`, client/scroll dimensions and getComputedStyle (per
   property, when read) return the page's layout as of the current document: a
   read the page has not measured since the document changed sends the pending
   changes with a measure request and the realm's worker waits (250 ms; 3 s for a
   page still loading) for the page's answer, which the host routes straight to
   the waiting realm (`measureSync`, `dom.measureSync`). The main thread never
   waits. Unchanged documents read the cache without a round trip; an unanswered
   read keeps the cache and reads stop waiting for a second. Resize observers hear
   about a change once, after the task that found it. `layoutStatus(node)` and
   `await measure(node, styles)` remain.

The basic Range/Selection adapter supports text selection, collapse, anchor/focus,
selectAllChildren, range endpoints/cloning and contextual fragments. It is not
all of DOM Range. Scroll writes keep authored coordinates locally until page
snapshots report actual/clamped values. ResizeObserver uses page snapshots;
full box-size/geometry semantics and native callback ordering remain unverified.
LinkeDOM MutationObserver attributes/old values/checkpoint delivery are tested;
its complete childList/characterData/detached-subtree semantics are **not** claimed.

## C-MATRIX and Svelte evidence

The DOM/web rows are in C-MATRIX's `store_global` category (not separate `dom`
or `web` categories). Zero usage counts retain their compatibility obligations.
This lane read the public architecture, kernel/engine-service, inventories,
workflows, keymap, traces, first-launch and MCP documents before implementation.
It inspected the tagged TextField control and inspector/timeline layout/observer
call sites. No private corpus, profile, extension source or identifiers are copied.

| Inventory surface | Current evidence / remaining work |
| --- | --- |
| document/createElement/createElementNS, Node/Element/HTMLElement, templates/fragments, selectors, attributes/classes/styles/text | Mirror → real jsdom document tests, SVG/xlink clone/remove tests and 100 deterministic mixed-mutation batches |
| Event/EventTarget/CustomEvent, mouse/keyboard/input/focus/wheel/pointer scalar data, add/remove/dispatchEvent | Capture/default/listener tests and compiled delegated handlers; real native pointer/key/drag events need macOS |
| document.activeElement, inputs, forms, selection, scroll | Native-state freshness and stale-echo race tests, checkbox/select/multi-select, FormData, text ranges and scroll commands; native IME/undo/clipboard/file controls remain open |
| getBoundingClientRect/getComputedStyle/ResizeObserver | Cached/explicit async snapshot tests; fresh sync layout, real WebKit layout/box sizes, font/assets/display-scale changes remain open |
| MutationObserver | Attribute filter/old-value/checkpoint test and Svelte select runtime; full DOM observer conformance remains open |
| document.fonts, rAF/cancelAnimationFrame, media elements, DOM canvas/WebGL, OffscreenCanvas, DOMParser/XML, Shadow DOM, popouts | Tree/parsing support is not full platform support. Host/native recorder/media/font/frame integration and conformance are still required. rAF requires an injected host scheduler; it is not emulated by a bridge frame delay. |

Three **original synthetic** fixtures are compiled with **Svelte 5.56.10**,
browser/client output bundled, and mounted against the shim with a jsdom page:

* Inspector: reactive text/checkbox/select two-way bindings and style directives.
* Keyed layers: delegated clicks, keyed each reorder/remove, selected classes,
  retained real element identity and unmount.
* Panel: conditional comment/fragment anchors, SVG, window key events and teardown.

The tests execute generated runtime code; they do not replace its DOM calls with
handwritten equivalents. This is representative Svelte coverage, not the entire
Svelte feature space, unchanged built-in service closure, all v1–3 panels, or a
frozen Electron trace replay. No existing consumer was pruned.

## Latency design and Linux diagnostic

A mutation flush runs at the microtask checkpoint, not on a display timer. The
page applies on arrival, not in rAF. Event data and all mutations caused by that
turn travel in bounded batches. There is no hot-path JS evaluation RPC or sync
layout request. The native host must preserve this scheduling on real queues.
Presentation measurement must not insert two extra rAF waits into the bridge.

`bun run bench` uses 5,000 unchanged rows plus an input/output. Each event and
mutation batch is JSON-encoded/decoded across microtasks; output equality is
checked. With 100 warmups and 1,000 measured events on this Linux session, the
optimized diagnostic was p50 **0.085 ms**, p95 **0.232 ms**, max **5.019 ms**.
Earlier local versions scanned unchanged siblings quadratically and had p95
**17.835 / 20.232 ms** (max **108.364 ms** in the second run). Avoiding those scans
and full page-state copies fixes that CPU regression; those old results are not
macOS comparisons. These numbers exclude the Rust queue hop, actual V8 embedding,
WebKit IPC/layout, display cadence and photons. **The ≤21 ms keystroke → visible
macOS target remains unverified**; S5c's 36 ms bridge measurement is not replaced
by this Linux result.

## Verification and integration

Run from the checkout, with the task's activation shell:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cd apps/native/js/remote-dom
bun install --frozen-lockfile
bun test
bun run typecheck
bun run format
bun run build
bun run bench
cd ../..
cargo test --locked --manifest-path crates/remote-dom/standalone/Cargo.toml
cargo clippy --locked --manifest-path crates/remote-dom/standalone/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path crates/remote-dom/standalone/Cargo.toml -- --check
```

Verified with Rust **1.96.1** and Bun **1.4.4**: **22 Rust tests**, **36 Bun tests**
(including the three compiled Svelte fixtures, bare-V8 and real JS↔Rust JSON
boundary probes). TypeScript, browser builds, formatting and strict crate Clippy
pass. Only synthetic DOM/documents and test inputs are used.

The requested shared-workspace `cargo test --locked -p pm-remote-dom` was also
attempted. It stops because the new workspace package needs its Cargo.lock entry.
The parent lock is outside this lane's ownership, so it is unchanged. The owned
standalone manifest/lock compiles the **same sources and tests** without resolving
or changing other lanes. The integrator must add this entry to the shared lock
using that workspace's existing serde/serde_json resolution:

```toml
[[package]]
name = "pm-remote-dom"
version = "0.1.0"
dependencies = ["serde", "serde_json"]
```

Still required outside this lane: wire the runtime's V8 bootstrap/microtask/timer
hooks, the private WebKit transport and queue owners, shell page lifecycle,
native recorder/media/font/asset capabilities, and MCP id-based panel inspection
and interaction. The Rust router, JS `inspect()`, `id(node)`/`node(id)` and page
`node(id)` expose the necessary inspectable boundary; no MCP tool is implemented
or renamed here.

On macOS verify real focused input → V8 → applied output → actual presentation
at 60/120 Hz, foreground/background and under same-realm raster work; physical
keys, IME/composition, text undo, focus/caret, selection, mouse/pointer/wheel,
clipboard/drag/files, AX, styles/fonts/assets and display changes. Exercise page
termination/recreation, queued/stale feedback, reload/history-held objects,
transport identity/origin/CSP, large mounts/backpressure and retained-node memory
(including the 39-extension workload). Resolve the two semantic blockers above
and close full observer/Range/media/canvas/platform coverage with standards/intent
fixtures and frozen-reference traces before claiming N4/N6 cutover.

## Decisions and selecting rules

| Decision | Rule |
| --- | --- |
| Keep logic/closures/services in V8; page contains rendering/serialization only | S5c architecture; N6 identity/sync contract; N8 no Node/Chromium or UI-thread wait |
| Stable ids, generation fencing, retained detached objects and revision-aware events | N4/N6 preserve held identity and queued intent; N7 headless inspection/driving |
| Microtask batching, immediate apply, incremental sibling links and bounded queue retries | N1/N2 professional responsiveness; N8 no main-thread wait; latency target requires avoiding an extra display interval |
| Preserve newer native form edits over causally older queued property writes | N3 exact authored/user intent; N4/N6 no lost keystrokes or stale value echo |
| Standards-oriented events/namespaces/form state rather than browser-specific quirks | N3 and quirk policy; library shortcuts are not authored intent |
| Explicit cached geometry and explicit default-action policies, with unresolved cases blocking cutover | N3/N4/N6: do not claim fresh synchronous layout or conditional default cancellation that this channel cannot provide |
| No existing runtime/consumer removal, no profile/project mutation, no parent lock edit | Prune rule and lane ownership; N4 preserve current users/data until compatibility gates pass |


## Package 9 part 2 realm connection

The shim now directly implements the compat package's local `connect` contract.
Session panel id equals realm id and generation; mismatched or duplicate
connections are refused before globals are exposed. All incoming page feedback
runs through the owning realm's task boundary. The realm host still drains V8
microtasks and flushes after timers/async checkpoints.

`inspect()` exposes created/connected/faulted/retired lifecycle and the host's
foreground/background/headless visibility fact. `setVisibility` is metadata;
hidden/headless pages never pause V8 logic. A page fault retires pending reads
and old messages while the independent realm can continue editing. Recreating a
native page and recovering mounted panel-local state remain host work.

Protocol **v1 is unchanged**; no Rust router/page-applier operation was modified.
The unused hypothetical synchronous compat DOM types were removed after checking
consumers. The reserved canvas factory is not yet wired to DOM canvas promotion.
The two semantic blockers above—fresh synchronous geometry and conditional
browser-default cancellation—remain cutover blockers.

Linux integration in compat's `tests/panels.test.ts` executes an original compiled
Svelte 5.56.10 panel using the shared client/subscriber runtime: mount, native
publication/reactivity, native input→PM/history edit, rebuild, immediate component
registration cleanup and retirement. It also verifies generation mismatch,
visibility transitions, faulted reads and document editing after page failure.
This adds four integration tests, including target-before-kernel key ordering and field exclusion; remote-DOM's existing **36 tests** and the
unchanged Rust router's **22 tests** remain green. It does not establish real
WebKit scheduling, page recovery, IME, AX or latency. See
[compat-realm.md](compat-realm.md) for the per-family coverage and native handoff.

Rules: N4/N6 select stable identities, shared synchronous closures and task-boundary
flushing; N7 selects metadata inspection and generation-fenced ids; N8 selects
visibility-independent V8 work and an unchanged nonblocking protocol. N2/N3 select
unmount on component registration disposal, as explicitly promised by the tag's
host comment, instead of retaining a leaked mount until extension unload. No
macOS UI, vendor/spike code or existing runtime consumer is changed.

The current merged workspace also passes `cargo test --locked -p pm-remote-dom`
(22 tests); the earlier lock-entry limitation above no longer applies to this
base. Standalone strict Clippy/rustfmt still pass, with no Rust/lock edits.
