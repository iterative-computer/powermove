# v1–3 compatibility realm bundle

P2 package 9 lives in `apps/native/js/compat` as `@powermove/compat-realm`.
The production entry is `src/index.ts`; `bun run build` produces the
`PowermoveCompat` IIFE in ignored `dist/compat-realm.js`. It has no runtime
package dependencies, Node/Bun imports, Chromium, or required DOM global.
Bun, TypeScript and Happy DOM are development/test tools only.

This is an embeddable realm package with Linux test doubles, **not a complete
native extension runtime or a cutover gate**. The Rust V8/kernel adapter,
remote-DOM implementation, complete PM capability backends and native recorder
rasterizer are integration work. No desktop runtime, Rust crate, vendor, spike,
profile or existing consumer is replaced or removed.

## Frozen extraction

Reference: tag `v1.2.0`, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`.

| Source at the tag                                                       | Package implementation                                                                                                          |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `renderer/src/kernel/services.ts`                                       | `src/services.ts`: unchanged stack/identity/disposal behavior; local type imports                                               |
| `renderer/src/kernel/editor-helpers.ts` and its 15-module export graph  | `src/frozen`: complete frozen `powermove` runtime exports, with shared caption modules re-exported                              |
| `renderer/src/kernel/registry.ts`                                       | `src/registry.ts`: contribution stacks, batches and ownership                                                                   |
| `renderer/src/kernel/keychord.ts`                                       | `src/keychord.ts`: chord helpers; DOM helpers are not required at module load                                                   |
| `renderer/src/controls/gesture.ts`                                      | `src/gesture.ts`: exact command/set/local dispatcher; local types                                                               |
| `renderer/src/legacy/core/model.ts`                                     | `src/legacy/model.ts`: factories, defaults, clone and serialization; excludes model/index/group/composition install and HMR     |
| `renderer/src/core/color-depth.ts`, `core/export-defaults.ts`           | Pure helpers under `src/legacy`                                                                                                 |
| `renderer/src/captions/model.ts`, `core/types/captions.ts`              | Pure caption helpers under `src/legacy`                                                                                         |
| `renderer/src/kernel/registries.ts`                                     | `src/keymap.ts`: DOM-free adaptation of binding sorting and synchronous bubble resolution                                       |
| `kernel/install.ts`, `legacy/core/history.ts`, `legacy/core/editing.ts` | `src/realm.ts`: new kernel-channel facade preserving documented admission/return forms; history/document authority stays native |

Paths in the table are beneath `apps/desktop/src` at the frozen tag.
The complete frozen `powermove` helper export graph is also extracted under
`src/frozen`; it is available through the package `./powermove` subpath and
`__powermove_runtime.powermove`. The embedding compiler/runtime supplies its shared
Svelte modules through `runtimeModules`. DOM helper bodies use only bridge globals
when called; they do not require a document at bundle load.

Pure model factories keep unknown authored data when cloning; IDs are issued in
the realm/generation namespace. Native evaluation, composition/group operations,
media, UI, storage and the remaining PM namespaces require explicit backends.
There is no success-shaped generic stub for absent backends.

## Embedding and wire contract

```ts
const realm = new PowermoveCompat.CompatRealm({
  id: projectRealmId,
  generation,
  initial: publication,
  host: kernelHost,
  recorder: nativeRecorder,
  dom: remoteDOM,
  capabilities: nativeCapabilities,
});
const extension = realm.extension(extensionId, 3);
realm.task(() => activate(extension.api));
// At host→JS task boundaries and after draining V8 microtasks:
realm.flush();
// Unsolicited native publication on this same realm owner thread:
realm.receive(nextPublication);
```

`KernelHost.call` is a synchronous **worker-thread** host function. Its Rust
implementation may wait on the kernel actor there; neither the UI thread nor
kernel actor enters JS while waiting for the other. `query` is a worker-local
synchronous capability, not an isolate-thread RPC. Every callback/event/microtask
checkpoint must run through the realm task/flush boundary. A returned Promise
is never implicitly awaited by history or mutation APIs.

`Request` carries `{realm,generation,txid,method,args}`. The adapter maps identity
to `Writer` and `TxId`; the host must deduplicate before applying. If transport
throws after applying, the same immutable request remains outstanding and is
retried with its original txid. A second operation cannot bypass that pending
ack. `Reply` carries an exact return value, authoritative `Publication`, optional
error, continuation, and same-realm retired closure handles. Revision is the
kernel's publication revision; `document.revision` remains source provenance.
The adapter must return a publication even on validation rejection. A transport
error is distinct from a rejected transaction.

| Method                                                           | Native adapter responsibility                                                                                                                                                                                                                                      |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `document.directWrite`                                           | `Call::DirectWrite { ops }`; validate atomically, LWW at each patch path, publish or return authoritative resync/error                                                                                                                                             |
| `document.touch`                                                 | Flush barrier plus evaluator/index invalidation; return undefined                                                                                                                                                                                                  |
| `history.begin/commit/cancel/rollback`                           | Matching `HistoryCall`; begin returns serialized before/null, commit boolean, cancel keeps writes, rollback uses native selective policy                                                                                                                           |
| `history.external`                                               | Register `(Writer,handle,bytes,cleanup)` independently of pending history; return native entry ID                                                                                                                                                                  |
| `history.undo/redo/externalDone`                                 | Move cursor, run owning-realm inline continuation, finish token exactly once; callback exception identity stays in JS                                                                                                                                              |
| `history.selection/clear/undoIfTop/export/import`, history facts | Matching history calls/queries; retirement cleanup drains on owning realm                                                                                                                                                                                          |
| `edit.begin/apply/dispatch/commit/cancel`                        | Matching `EditCall`, checkpoint/selection semantics and existing EditResult messages                                                                                                                                                                               |
| `edit.admitMutation`                                             | Authoritative pre-action live-edit refusal; submit-time admission must also check                                                                                                                                                                                  |
| `edit.mutateBegin/Finish/Abort`                                  | Open a native callback lease after admission; allow nested synchronous host calls and fold their writes; validate the final tree, record native provenance/history, or selectively abort owner writes. Return EditResult; JS attaches exact `data.result` identity |
| `selection.set`                                                  | Kernel selection state publication                                                                                                                                                                                                                                 |
| `workspace.prepare/replace/mutate`                               | Native workspace draft/built-in clone/normalization/state persistence; retain partial writes when callback throws                                                                                                                                                  |
| `realm.failed`                                                   | Seal acknowledged work, fence generation, external entries become barriers                                                                                                                                                                                         |

The method names are the **JS adapter protocol**, not a claim that the existing
Rust actor already exposes these string calls. `src/types.ts` is the definitive
TypeScript interface. The Rust adapter is outside this lane's owned paths.

## Mirror and write barrier

`PM.proj` is a plain live JS tree. Writes return their ordinary assigned value;
reads in the same stack see them immediately. A boundary diff captures assignment,
delete, array methods, defineProperty and writes through unwrapped retained
aliases. This avoids a Proxy-only barrier that misses aliases retained before
assignment. No raw write promises an Undo record: the kernel's open entry policy
controls history. Sync calls flush before admission; task/microtask boundaries
flush otherwise. Event publication occurs after the authoritative mirror update.

Disjoint native changes merge into pending local writes; later local leaf values
win on the same logical path. Layer `id` and keyframe `i` paths are translated
across native reorder before flushing. Array insertion/removal/reorder uses a
whole-list patch; concurrent list structure requires kernel validation. If a
retained record was deleted, fail and resync instead of retargeting its former
index. Reconciliation preserves retained roots, layers, properties and keyframes
where representable. An explicit `PM.proj = replacement` retains that replacement
object as the new root. Detached deleted references stay detached.

JSON is validated without invoking getters. Cycles, nonfinite numbers, BigInt,
functions, undefined document fields, sparse/decorated arrays and prototype keys
are rejected instead of being silently serialized away. Rejection resyncs and
throws; scheduled failures go to `reportError`. Nonconfigurable/frozen descriptors
and hostile proxies still require broader compatibility fixtures. Boundary diff
cost is proportional to the live tree and is **not performance certified**.

## Synchronous services, closures and rendering

One realm owns the full synchronous service closure. `get` returns the provider
itself; callbacks, old retained providers and mutable objects keep identity.
Extension disposal removes owned registrations and subscriptions. External undo
closures retain generation pins independently of extension disposal until native
history retirement sends cleanup. A throwing external undo leaves the native
cursor moved and rethrows the original JS value. Engine overrides use the same
`raster` service stack and may delegate to a held base provider inline.

`RecorderHost` in `src/canvas.ts` owns native surfaces, context state, recorded
commands, metrics, pixels and encoding. The JS canvas/context/method identities
remain stable across resets. `drawImage(canvas)` retains an immutable source
version, including across source repaint/disposal, until the destination resets
or retires. Native host code must reference-count these versions and enforce
memory quotas. Paths, transforms, clipping, text, paint handles and pixel methods
are forwarded synchronously. Tests check protocol forwarding/lifetime, **not
pixel or browser-spec equivalence**. Pixel arrays must use V8 sandbox allocations.

`render.*` synchronous and additive async siblings call worker-local host
capabilities. The host may return a `NativeSurface` descriptor; the realm adopts it as one
recorder canvas per native surface ID without reallocating. Render queries carry
realm, generation and publication revision. The host must preserve
nested override/capture semantics; it must never serialize DOM nodes or pixel
payloads over the remote-DOM channel. Snapshot JPEG correctness, full canvas
creation/promotion, alpha/colour/text behavior, WebGL/presenter and OffscreenCanvas
still require native/DOM adapters and spec fixtures.

## Remote DOM interface

`src/remote-dom.ts` defines `RemoteDOMBridge`, stable node IDs, ordered batches,
event paths, generation fences, read/invoke acknowledgement sequences and
cancellable-default tokens. The other lane supplies DOM globals via `connect`.
It receives this realm's task boundary and recorder canvas factory so parser,
clone, popout and imperative canvas creation can preserve element/context identity.
There is deliberately no fake document installed when no bridge is attached.

Synchronous layout/style reads require a flush/acknowledged batch; returning stale
zeroes is not acceptable. WebKit must defer a cancellable default until the V8
result resolves its token; an asynchronous mutation stream cannot retroactively
cancel a default action. The bridge must preserve DOM-first target/bubble order,
listener ordering, focus/selection/IME and clipboard user activation. The interface
expresses those obligations; this lane does not implement WebKit transport or DOM.

## Decisions and selecting rules

| Decision                                                                                                               | Selecting rule                                                             |
| ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| Keep native actor authoritative; JS callbacks/services stay synchronous on the realm worker                            | N6, N8; adopted P0 V8/remote-DOM decision                                  |
| Plain-object boundary diff, including raw retained aliases, with validated native LWW flush                            | N3 authored intent, N4 preservation, N6 live identity                      |
| Rebase retained id/keyframe paths across native reorder; reject deleted targets                                        | N3, N4; never redirect an authored write onto a different layer            |
| Exact callback/Promise/error identity; immediate history commit; cancel keeps writes; partial workspace errors persist | N6 and explicit C-MATRIX source rules                                      |
| Native selective cancel and seal on realm failure, not tag-wide foreign rollback                                       | N2, N4; explicit native disposition in C-MATRIX/kernel                     |
| Single synchronous service realm and history generation pins                                                           | N4, N6; no synchronous isolate-thread RPC                                  |
| Immutable native recorder source versions                                                                              | N3 Canvas authored intent, N8; no Chromium dependency                      |
| Report requested apiVersion rather than tag's hard-coded 1                                                             | N3 and quirk policy: accidental hardcoding is not recorded authored intent |
| Preserve reference/runtime consumers and remove nothing                                                                | Prune rule; N4                                                             |

## Verification and remaining work

Run from the repo with the supplied activation script, then:

```sh
cd apps/native/js/compat
bun install --frozen-lockfile
bun test tests
bun run typecheck
bun run build
bun run replay
```

Linux verification: **131/131 package tests** and **8/8 existing trace-recorder
tests** pass (including the overlay parser check against a disposable extraction
of the frozen tag). TypeScript checking, production bundling, locked installation,
Prettier and Git whitespace checks pass. The production bundle is approximately
129 kB before compression. The tests run in Bun/JavaScriptCore; actual sandboxed
V8 loading remains a native-host verification item.

Replay executes the **original** `tools/trace-recorder/scenarios.ts` against the
bundle's facades and the independent fake kernel. It validates all 11 goldens,
compares root public call/throw/return forms, exact callback assertions, explicit
callback order, project name/layer IDs/names/types/source revision and history
availability. It does **not** copy golden output into fake host replies.
Eight realm scenarios and three input-routing projections match **78/78 public
boundaries**. Keymap tests use development-only Happy DOM; menu commands are
explicitly driven at the realm boundary. These are not 1,093-row byte matches,
full project/provenance/patch comparisons, internal allocation matches, actual
AppKit/Chromium menu delivery, native kernel replay or visual parity.

The legacy runtime has more than these boundaries. This package does not yet
implement the complete 377-member C-MATRIX API, Svelte mounting/reactivity,
activation/reload staging, full legacy composition/group/evaluation surfaces,
workspace/theme ownership stacks, remaining contribution kinds/ownership adapters, host storage,
media, permissions or every global. Native adapters must provide those capabilities
before activating arbitrary existing extensions. Structural mutation uses the callback-lease protocol to preserve reentrant host writes. The Rust adapter must implement these three lease methods using native owner tracking and selective rollback; they are not existing string actor calls.

Root `package.json` currently includes only `apps/*` and `packages/*`; the owning
integration lane must add `apps/native/js/*` to register this nested Bun workspace
package and reconcile the root lockfile. This lane keeps a local locked package
so it is independently buildable without editing unowned paths. Rust adapter
implementation belongs in the V8/kernel runtime lane. No Rust source changed,
so cargo test/fmt/clippy are not applicable to this TypeScript-only package.

On macOS verify sandboxed V8 loading/microtasks, real kernel mappings and selective
cancel/foreign-write races, main-thread nonblocking behavior, external-history
reload pins, native recorder pixels/pm-vector/pm-text/nested render continuations,
remote WebKit DOM identity/defaults/layout/IME/promotion, AppKit menu delivery,
unchanged inspector/timeline forks, 39-extension memory and all applicable S1/S5
latency/background budgets. None of those gates is established by Linux doubles.
