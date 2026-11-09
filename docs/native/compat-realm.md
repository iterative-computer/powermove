# v1–3 compatibility realm bundle

P2 package 9 lives in `apps/native/js/compat` as `@powermove/compat-realm`.
The production entry is `src/index.ts`; `bun run build` writes the bundles the
app embeds (`compat.js`, `store.js`, `page.js` and the runtime module list) to
ignored `dist/`, as compat-runner's build script does. They have no runtime
package dependencies, Node/Bun imports, Chromium, or required DOM global.
Bun, TypeScript, Happy DOM, jsdom, Prettier and the Svelte compiler are development/test tools only. The shared Svelte runtime is injected by the embedder.

This is an embeddable realm package with Linux test doubles, **not a complete
native extension runtime or a cutover gate**. The Rust V8/kernel adapter, WebKit transport, complete native PM capability backends and native recorder rasterizer are integration work. The merged remote-DOM shim now connects directly to the realm. No desktop runtime, Rust crate, vendor, spike,
profile or existing consumer is replaced or removed.

## Frozen extraction

Reference: tag `v1.2.0`, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`.

| Source at the tag                                                       | Package implementation                                                                                                          |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `renderer/src/kernel/services.ts`                                       | `src/services.ts`: unchanged stack/identity/disposal behavior; local type imports                                               |
| `renderer/src/kernel/registry.ts`                                       | `src/registry.ts`: contribution stacks, batches and ownership                                                                   |
| `renderer/src/kernel/keychord.ts`                                       | `src/keychord.ts`: chord helpers; DOM helpers are not required at module load                                                   |
| `renderer/src/controls/gesture.ts`                                      | `src/gesture.ts`: exact command/set/local dispatcher; local types                                                               |
| `renderer/src/legacy/core/model.ts`                                     | `src/legacy/model.ts`: factories, defaults, clone and serialization; excludes model/index/group/composition install and HMR     |
| `renderer/src/core/color-depth.ts`, `core/export-defaults.ts`           | Pure helpers under `src/legacy`                                                                                                 |
| `renderer/src/kernel/registries.ts`                                     | `src/keymap.ts`: DOM-free adaptation of binding sorting and synchronous bubble resolution                                       |
| `kernel/install.ts`, `legacy/core/history.ts`, `legacy/core/editing.ts` | `src/realm.ts`: new kernel-channel facade preserving documented admission/return forms; history/document authority stays native |

Paths in the table are beneath `apps/desktop/src` at the frozen tag.

`bun run freeze-reference` copies, verbatim, the relative-import closure of the
public `powermove` module (`kernel/editor-helpers.ts`), the legacy runtime and
the shared controls from commit `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`
into `src/frozen/reference`. That module's code is identical at the v1.2.0 tag;
one copy serves extensions, the legacy runtime, the controls and the Store
facades. It is available through the package `./powermove` subpath and
`__powermove_runtime.powermove`. The embedding compiler/runtime supplies its shared
Svelte modules through `runtimeModules`. DOM helper bodies use only bridge globals
when called; they do not require a document at bundle load.

`src/frozen/host` additionally extracts the complete public host/contribution
registry graph and PM-backed facade factories from the pinned commit.
`bun run freeze-host` reproduces these files using TypeScript 5.9.3 and Prettier
3.6.2. The regression checks regeneration against the tag. App imports for
Svelte mounting/subscriptions, haptics and optional profiling are dependency
injections; elapsed-time diagnostics fall back to Date.now when no performance
clock is attached. Runtime code has no Node/Bun imports or eval. The Store
facade extraction additionally includes the pinned `sandbox/shim-api.ts`,
`shared/sandbox-rpc.ts` and legacy easing runtime. Only their relative import
paths are adapted. Its asynchronous API remains separate from `CompatRealm`;
the native confined helper/core integration and its remaining capture/resource
limits are documented in [the P2 evidence](extcheck-p2.md).

The extension-check embedder compiles seven unchanged frozen shared controls
with one physical Svelte runtime. `connectControlState` publishes authoritative
project, selection and transport facts into their frozen read stores and
releases its subscriptions/styles on teardown. Other controls remain explicit
unavailable capabilities.

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

## Remote DOM connection

`src/remote-dom.ts` now describes only the realm-local `connect` interface.
The previously hypothetical string-id/synchronous DOM wire types had no consumers
and were removed. Transport is exclusively `remote-dom/src/protocol.ts` **v1**;
no Rust/page wire operation or version changed. A session's panel id must match
the owning realm id, with the same generation, and connects once.

`RemoteDOM.connect` supplies stable DOM globals and wraps incoming feedback in
`realm.task`, including input events and resize callbacks. The host must still
flush after V8 microtasks/timers. Inspectable lifecycle is created → connected →
retired, or faulted on page failure. Foreground/background/headless are separate
host-reported visibility facts; none pauses V8 work. Fault retirement rejects
pending layout reads, fences all old messages and preserves the owning realm's
document/history logic. Page recreation/remount and preservation of panel-local
state are native integration work, not implemented recovery.

The connection accepts the reserved recorder factory but the shim does not yet
promote DOM canvas elements to native recorder surfaces. Fresh synchronous
layout, dynamic cancellation of a browser default, full DOM/platform semantics
and same-element canvas promotion remain explicit cutover blockers. Cached
geometry is never represented as acknowledged fresh layout.

Svelte component panels use the **shared injected client runtime** for mount,
flushSync/unmount and `svelte/reactivity.createSubscriber`. Without Svelte,
ordinary API reads remain plain; component mounting fails explicitly. Native
project/time/selection/theme publications drive the extracted subscription code.
A compiled original synthetic Svelte 5.56.10 panel is tested through the actual
realm, shim and jsdom page: mount → native publication → edit from input →
reactive render → rebuild → registration disposal → teardown. Component
registration disposal unmounts immediately; it follows the tag's documented
intent rather than retaining the tag's leaked mount until owner disposal.

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

## Package 9 part 2 decisions

| Decision | Selecting rule |
| --- | --- |
| Extract the complete contribution host and PM facade factories, preserving validation, exact argument lists/defaults/results, ownership, stack restoration, callback identity and async-when/menu behavior | N4/N6; reference is v1.2.0, not an inferred API |
| Native/platform functions remain injected capabilities; absent operations throw Unavailable, including fonts and controls; unavailable transform evaluation never manufactures an identity matrix | N1/N3/N4; presence checks are not operational certification |
| Preserve later local intent after rejection; defer overtaking publications until the immutable lost-ack request resolves | N3/N4; do not resurrect an acknowledged value over a newer native edit |
| Keep the submitted structural mirror on a lost finish acknowledgement; abort an unused recovered start lease without running the callback later | N3/N4/N6; callbacks execute at most once, acknowledged work survives |
| Isolate publication observers and retirement cleanup errors; drain all retired handles | N4/N6; an observer cannot mask a host rejection or strand other pins |
| Permit omitted optional workspace record fields assigned by tagged layout helpers; retain strict document validation | N3/N6; matches JSON workspace persistence without weakening document validation |
| Unmount a component when its registration is disposed | N2/N3; host.ts explicitly promises this; the tag's delayed cleanup is a leak, not authored intent |
| Use requested apiVersion, and log disposer errors without reporting retired-owner health failures | N3 for the accidental hardcoded version; N6 for the extracted disposal policy |
| Metadata-only realm/page inspection and commands/panels addressable by id | N7; no vars or document contents in inspection metadata |
| Remove only the replaced unused hypothetical DOM types; retain reference tooling, all existing app/runtime consumers and protocol v1 | Prune rule, N4/N8; lane ownership |

## Verification and coverage

From `apps/native/js/compat`, after shell activation:

```sh
bun install --frozen-lockfile
bun test tests
bun run typecheck
bun run format
bun run build
bun run replay
bun run corpus
```

Linux result: **173 compat tests**, **36 remote-DOM tests**, **8 trace-recorder
tests**, and **22 unchanged Rust remote-DOM router tests** pass. Both JS packages
pass typechecking/browser builds/formatting; the requested workspace `cargo test --locked -p pm-remote-dom` also passes
22 tests, and strict standalone router Clippy/rustfmt pass. No Rust source or
workspace lock changes were needed. The compat production IIFE is approximately **224 kB** uncompressed.
Bun uses JavaScriptCore; the remote-DOM suite also runs a bare Node V8 dependency
probe and the real JS↔Rust JSON router roundtrip. This is not certification of the
pinned sandboxed rusty_v8 embedding. The remote suite's cold Rust build/toolchain setup exceeded its 5-second
per-test limit. Prebuilding the unchanged wire example and selecting the installed
Linux toolchain explicitly makes the boundary test pass. The supplied activation script was absent; Bun 1.4.4, Rust
1.96.1 and gitleaks 8.30.0 were installed in scratch, without tracked tool changes.

The existing replay still matches **78/78 public boundaries from 11 Electron
goldens**. It checks returns/errors, callback order, name/layer ids/revision and
history availability, with test doubles for DOM/menu delivery. These are not
1,093-row byte matches, a new Electron capture, full document/patch comparisons,
actual native kernel leases or visual parity.

`tests/corpus/extensions.ts` contains original synthetic registration/runtime
activations with apiVersion 1, 2 and 3 manifests. The corpus exercises every
contribution family, storage isolation, activation vars, six typed synchronous
provider families, workspace appearance, async command visibility and menus,
import defaults and drag payloads. `facades.test.ts` and `scripts/corpus.ts` run
**95 call/argument/result comparisons** against facade functions loaded directly
from the pinned tag's `kernel/install.ts`. Native operations are independent
synthetic recording providers; 38 native-only assets/scene/extension delegates
also verify exact arguments, callback/result/error and Promise identity; no golden result is used as a host response.
This is source-level replay, not additional frozen Electron evidence.

The following counts are required immediate namespace members (overloads are
collapsed, optional UI.copy excluded), not the 377 nested C-MATRIX rows. All
**36 families** declare their required members under each apiVersion. A declared
native delegate is not proof its platform implementation exists. Source callable
counts can include nested members and wider adapter methods.

| API family | Required members declared | Behavioral evidence / remaining gate |
| --- | ---: | --- |
| panels | 6/6 | Registration/build/unload, compiled Svelte mount/rebuild/dispose, hidden-on-activation policy; native dock host required |
| commands | 4/4 | Register/run/has/list, exact results, guards, async when, id dispatch |
| keybindings | 4/4 | Chord/list/ownership plus existing DOM/keymap/golden tests |
| effects | 3/3 | Tagged validation, get/list/ownership; native shader integration required |
| transitions | 3/3 | Tagged validation, get/list/ownership; native shader integration required |
| layers | 3/3 | Tagged validation, definition identity/ownership; native renderer required |
| scene3d | 24/24 | 24 native delegate identity probes and explicit absence; real scene/navigation/gizmo backend remains |
| assets | 5/5 | 5 native delegate identity probes and explicit absence; picker/durable import/network backend remains |
| theme | 6/6 | Register/activation/scheme/ownership restoration; page/native theme application remains |
| palette | 2/2 | Provider/callback/ownership; native palette host required |
| menus | 3/3 | Sync collect, async ordered gather, ownership; native presentation required |
| status | 2/2 | Live callback/list/ownership; native presentation required |
| project | 14/14 | Existing write/history goldens and Svelte publication/edits; transport host required |
| anim | 19/19 | 18 source callable comparisons; native evaluator required |
| model | 15/15 | 11 source callable comparisons, factories/definitions/retained identity |
| selection | 9/9 | 8 source callable comparisons plus mirror/race tests |
| groups | 7/7 | 7 source callable comparisons; native graph helpers required |
| transport | 11/11 | 8 source callable comparisons, lazy backend accessors; engine/clock required |
| history | 8/8 | Existing goldens, inline continuations/reentry, pin cleanup and lost-ack tests; native selective history required |
| edit | 6/6 | Existing goldens, preview/admission/structural leases and recovery; actor implementation required |
| media | 8/8 | 11 nested source callable comparisons, import-default stacks; real media/fonts required |
| inspector | 2/2 | Section validation/build/cleanup/ownership; real inspector host remains |
| render | 5/5 | 9 nested source callable comparisons, override→nested capture→held base tests, existing recorder fixtures; native pixels/presenter remains |
| uiState | 12/12 | 13 source callable comparisons; native persistence/ownership required |
| ui | 14/14 | Shared Svelte mount, gesture/bindings, explicit native UI/control delegates; actual controls/overlays required |
| dnd | 11/11 | Tagged MIME/parser helpers, live drag identity, malformed decline; native drag/files required |
| workspace | 9/9 | Hidden geometry, membership/moves, partial errors and user-over-owner preservation; native persistence required |
| util | 9/9 | 9 source callable comparisons; pure/backend helpers required |
| ease | 2/2 | 1 source callable comparison; native/legacy preset provider required |
| space3d | 10/10 | Existing frozen transform fixtures and fail-explicit live adapter; full scene integration required |
| services | 2/2 | Six typed provider families' closure/identity/override/disposal; wider runtime provider implementations remain |
| storage | 3/3 | Namespaced read/modify/write/delete, isolation; native private store backend required |
| events | 2/2 | Ownership, error isolation, publication ordering and Svelte subscriptions |
| extensions | 9/9 | 9 native delegate identity probes and explicit absence; loader/compiler/discovery/reload backends remain |
| vars | 3/3 | Frozen per-activation string values; private native values provider required |
| host | 4/4 | Exact PM, shared Svelte mount and injected state/haptics; real native state/controls required |

## Remaining completion gates and macOS handoff

**The whole v1–3 contract is not certified complete.** No existing extension is
excluded and no runtime consumer was pruned. Declared/delegated facade coverage
must not be reported as complete execution of all 377 typed, 902 PM or 156 wider
service rows. Arbitrary raw PM subsystems, evaluator/composition/group/media/UI
backends, all returned handles, remaining runtime globals and Store roles still
need implementations and behavior evidence from their owning lanes.

Outside these owned paths: implement the string adapter methods over the actual
Rust actor, including selective ownership/provenance, validation and structural
leases; txid lookup/deduplication, mailbox ordering, external undo leases and
retirement; V8 microtask/timer/host entry scheduling; private WebKit transport,
page recreation/remount/local-state recovery; native canvas promotion/raster/text/
fonts/pixels/JPEG/WebGL/presenter; scoped storage/assets/extensions/UI/scene
backends and unchanged live inspector/timeline/provider clusters. The fake actor
follows the tag's whole-scope cancellation. Its dirty-write/live-edit test proves
bridge ordering, **not native selective cancel or foreign-writer safety**.

On macOS verify real V8/native actor races (including acknowledged foreign writes
and external undo racing queued page/agent commands), render error/cancel/revision
continuations, fresh layout/default-action semantics, IME/clipboard/focus/text undo,
physical key/menu ordering, accessibility, real styles/fonts/assets/pixels and
colour, page termination/recreation, retained objects during reload, sandbox and
permission roles, 39-extension memory and keystroke→visible latency at 60/120 Hz.
N1/N2 performance and N4/N6 cutover remain open until that evidence exists.
