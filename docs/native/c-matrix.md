# C-MATRIX: extension contract inventory

This is the P0 **source inventory** for apiVersion 1–3 compatibility. It identifies the synchronous boundaries that the native kernel, editor services, compatibility page, Store runtime and recorder canvas must preserve. It does **not** claim that frozen-reference traces or native compatibility tests have passed.

Reference: shipped tag **`v1.2.0`**, commit **`07ed5fa17ee640659cb075f9cad1af2095011d60`**. All source locations below and in [c-matrix.csv](c-matrix.csv) refer to that tag, not the working branch. Resolve a location with `git show v1.2.0:<path>`. No build was run.

Every v1–3 extension remains in scope, including zero-count members, arbitrary trusted DOM, legacy registry users, overrides, forks and themes. Counts select fixtures; they never justify removing functionality. Preserve authored geometry, timing, colour, drawing and API intent. Browser-specific rendering differences require a standards/intent decision; they are not automatically compatibility requirements. Keep replaced code until its last consumer and compatibility gate are gone.

## Coverage and method

| CSV category | Rows | Boundary |
| --- | ---: | --- |
| `api` | 377 | All reachable typed namespaces/members in `PowermoveAPI`, including nested capabilities and returned gesture/navigation/gizmo/modal/drag handles |
| `powermove` | 43 | Runtime value exports in `shared/extension-runtime.ts`, cross-checked against `kernel/editor-helpers.ts`; type-only exports excluded |
| `pm` | 902 | Legacy registry roots, statically declared subsystem members, optional production references and one dynamic-key contract row |
| `service` | 156 | Registered provider roots, typed members and wider runtime fields, including import defaults and source preview |
| `store_global` | 226 | ECMAScript/browser globals, required DOM access points, denied/conditional capabilities and the injected runtime table |
| `canvas2d` | 83 | Context methods/state, gradient/pattern operations and canvas/bitmap creation/readback/promotion probes |
| `command` | 156 | Supplemental concrete/default and loop-expanded command cases for key-consumption classification |
| **Total** | **1,943** | One row per canonical category/member pair |

The CSV records call mode, identity, read-after-write, callbacks/reentrancy/events, decline behavior, cluster, realm, required trace, proposed backing, admission/return rules, source evidence and four usage counts. Namespace rows describe the namespace value; they are not additional callable operations. `()` in a path denotes a returned handle, not an extra property on the factory function. Model records and constant dictionaries are represented as values, rather than enumerating project data or icon/channel table entries.

Method:

1. Read the execution baseline in full. Inspect the tag's API types, host/install adapters, compiler export table, loader, registries, legacy bootstrap/assignments, built-ins, sandbox host/view/boot/shim, security policy and shipped extension guide.
2. Extract public declarations and assignments with a TypeScript syntax parser. Follow local namespace aliases and named service registrations. Reconcile inherited public service interfaces with their wider runtime implementations. Exclude tests, synthetic source fixtures and duplicate numbered source copies from the production surface scan. The permissive `PMRegistry extends Record<string, any>` makes static declarations insufficient by themselves: optional reference-only rows are labelled, and `PM.[key]` preserves dynamic extension-authored fields without listing private keys.
3. Count static source references once per extension installation, built-in or repository fixture. Recognize normal API aliases, named `powermove` imports, service-get aliases, literal property access and Canvas2D receivers inferred from type annotations or `getContext('2d')`. Comments and type-only references do not count. A reference can be a capability check or a definition/registration; these are **reference counts, not observed executions**.
4. Compare source behavior with the proposed native boundary. Put every unresolved identity, callback, error, timing and environment observation into an explicit trace scenario. No private source, manifest names, IDs, code, source paths or project identifiers are copied into these artifacts.

### Count interpretation and limits

The read-only local scan found **44 immediate extension directories with matching manifests**, comprising **24 apiVersion 1 and 20 apiVersion 3** installations and **91 source files** after exclusions. There was also **one nested manifest**, which is not a separately discovered installation; two immediate directories had no manifest. These measurements describe the available folder snapshot, not the enabled set or an earlier profile. They do not assert which installs came from the Store. The discovery rules are in `main/extensions/discovery.ts:35–95`.

- `corpus_usage_count`: distinct local installations with a recognized static reference. Only aggregate counts leave the scan.
- `builtin_usage_count`: distinct built-in directories with a reference, out of **10** in the tag's built-in table.
- `store_fixture_usage_count`: distinct manifest-bearing repository extension fixtures with a reference, out of **10**. This is a regression-fixture count, **not Store adoption**.
- `store_usage_count`: distinct published extensions with a recognized static reference in their latest public release, out of **4** in the **2026-11-01** Store snapshot below. This population is independent of the local corpus and repository fixtures.

A zero means no recognized reference in that scanned set. It does not prove absence: computed keys, callback indirection, imported helper wrappers, generated source and inferred `any` receivers can evade this scan. The count method does not execute extensions or include project-scoped folders, enabled-state metadata or downloaded bundles outside the specified installation folder. Canvas properties on a receiver whose type cannot be established remain a trace concern. The standard Canvas candidate rows deliberately cover more than the positive static-use subset.

The browser document exposes an **open web platform**, not a source-defined global allowlist. The global rows include required members, used capabilities and conservative presence probes. `evidence_status` and `admission` distinguish known runtime code, standard candidates, feature-dependent constructors and denied accessors. Source alone cannot enumerate every browser prototype or prove availability of secure-context/worker-only members. A frozen runtime/view descriptor sweep must close that gap before choosing a DOM-free Store helper. Do not interpret all 226 candidates as guaranteed constructible globals.

## Store snapshot (2026-11-01)

The authorized read-only snapshot covered **4 published extensions**, **4 latest releases**, **11 release files** and **5 TypeScript source files**. The public catalog had **1 page**, **0 further pages** and **0 duplicate listings**. A second catalog read confirmed **0 changes** to the listing/release set during collection. No extension names, identifiers, publishers, source excerpts or per-extension provenance are retained here.

### Acquisition and static-count method

The desktop Store client and MCP `store_search`, `store_extension` and `store_source` gateway share the public Store read API. `store_library` describes local installations and is not a published-catalog denominator. The MCP gateway required an editor session, so collection used the desktop client's public HTTPS GET routes directly at `https://cloud.trypowermove.com`: `/v1/store/extensions?sort=name&limit=50`, then `/v1/store/releases/{releaseId}`, `/tree` and `/tar`. Requests used **no authentication, credentials, cookies or account state**. There was a **1-second minimum pause** between requests, **14 GET requests** in total and **0 rate-limit responses**; retries would honor `Retry-After`. No install, rating, publishing or other Store mutation was performed.

All releases were downloaded under `/tmp/pm-store-snapshot`, outside Git. All **4 archive SHA-256 hashes**, **11 file sizes** and **11 Git blob SHA-1 hashes** matched the published release/tree metadata; all **4 file inventories** were complete. Archive paths were checked before extraction. The temporary snapshot was deleted after aggregate validation.

A TypeScript syntax-tree scan of all **5 source files**, with **0 parse errors**, counted executable references against every existing canonical C-MATRIX row. Comments, type declarations/queries and type-only imports were excluded; GLSL strings were inspected as effect declarations rather than executed JavaScript or browser-global references. Runtime template interpolations remain source expressions. Inspection of all runtime call sites and imports cross-checked the result: **5 type-only imports** and **1 local value import**, whose target was included; **0 external runtime imports**. No extension code was executed and no build was run.

The CSV schema is unchanged: its sole Store column is the number of distinct extensions using the member. The following table supplies the total source occurrence count for every nonzero row. **Every row omitted from this table has 0 Store extensions and 0 occurrences**, including all powermove helpers, PM members, services, Store globals, Canvas2D calls and supplemental commands. Namespace occurrences count the namespace expression within each member access; namespace and leaf counts overlap and must not be summed as independent operations. These are authored references, including registrations and checks, not runtime invocation counts.

| C-MATRIX member | Store extensions | Total occurrences |
| --- | ---: | ---: |
| `api.effects` | 4 | 5 |
| `api.effects.register` | 4 | 5 |
| `api.events` | 1 | 1 |
| `api.events.on` | 1 | 1 |
| `api.log` | 1 | 1 |
| `api.onDispose` | 1 | 1 |
| `api.project` | 1 | 1 |
| `api.project.get` | 1 | 1 |

Thus **8 of 1,943 rows** have positive Store counts; the remaining **1,935 rows** have measured static zeros. The most-used callable member is `api.effects.register`: **4 extensions**, **5 occurrences**.

### Versions and kinds

| apiVersion | Extensions |
| --- | ---: |
| 1 | 1 |
| 2 | 0 |
| 3 | 3 |
| Other | 0 |

Kinds are distinct-extension counts from published manifest contributions, reconciled with catalog categories; kinds can overlap. Script means command/tool contributions or catalog category, not merely having a JavaScript entry module.

| Kind | Extensions |
| --- | ---: |
| Effect | 4 |
| Transition | 0 |
| Panel | 0 |
| Layer | 0 |
| Theme | 0 |
| Script/command/tool | 0 |

### Compatibility risk classes

Counts are distinct extensions and classes can overlap. DOM means browser-document/element construction or access; canvas means authored canvas/context creation or drawing; direct WebGL means an authored context/presenter call. Declarative GLSL effects are reported separately because they use the host shader pipeline and may need its WebGL fallback without creating an extension-owned context. Raster overrides mean replacement/registration of the raster provider, not ordinary effect registration. Synchronous pixel APIs include raster/capture/snapshot and canvas/WebGL readback/promotion; live services mean synchronous service get/register or legacy live-provider access. Direct project writes include assignments, updates and mutations through `PM.proj` or a retained alias.

| Risk class | Extensions |
| --- | ---: |
| DOM | 0 |
| Canvas / Canvas2D | 0 |
| Direct WebGL context or presenter | 0 |
| Declarative GLSL effects (host shader route) | 4 |
| Raster overrides | 0 |
| Synchronous pixel APIs | 0 |
| Live services | 0 |
| Direct `PM.proj` writes | 0 |

### Coverage limits

This is the complete anonymously browsable catalog at collection time, using **one latest public release per extension**. It excludes historical releases, private/unlisted/hidden/removed/tombstoned listings, yanked source unavailable to anonymous reads, unpublished work, installed-only or project-scoped extensions, and later publications. Collection is not a transactional server snapshot; the unchanged final catalog check bounds that limitation. The denominator counts extensions, not individual contributed effects or installations.

Static counts do not prove execution frequency, permission admission, native shader compilation, rendering parity, runtime-global availability, service identity or compatibility conformance. Computed keys, generated/evaluated code, indirect aliases and remotely loaded dependencies can evade static analysis; none of the reviewed runtime imports required external source retrieval. GLSL identifiers and comments are not JavaScript global usage. No descriptor sweep, frozen-reference trace, runtime performance check or build was performed. Zero Store counts retain every compatibility requirement and fixture, including arbitrary trusted DOM, Canvas2D/pixel behavior and service/presenter coverage. This small effects-only catalog does not replace the broader local corpus or establish that every Store runtime can be DOM-free.

## Derived synchronous clusters

Use the **connected closure of synchronous object/callback dependencies**, not merely strongly connected cycles. A one-way synchronous call still cannot become a blocking call to another isolate thread. `K` is the native kernel/data boundary; `E` is the editor cluster; `R` is one Store extension's runtime/view domain. These labels are proposed backing boundaries, not evidence that the tag already isolates its built-ins: trusted code currently shares one renderer realm.

| Caller → provider/callback | Synchronous evidence in the tag | Consequence |
| --- | --- | --- |
| Inspector → timeline | `extensions/inspector/context.ts:171–174`; `ChannelRow.svelte` reads `graph`, writes graph state, calls `reveal` and optional `focusGraph`; `retiming.ts:49` | Timeline and inspector belong together unless the whole service becomes a synchronous kernel facade |
| Inspector → viewer/tool | `StructuredSection.svelte:41–44` writes viewer `activePath` and calls tool `setTool`; `TextAnimators.svelte:94` reads viewer `textSelection` | Scalar-only snapshots lose live writes and identity; viewer chrome and toolbar join E |
| Viewer → inspector/tool | `extensions/viewer/viewer.ts:720–721`; `path-editing.ts:42,74`; `text-editor.ts:173–217` | Mutual editor dependency; refresh/tool callbacks must stay callable inline |
| Timeline → captions | `extensions/timeline/caption-track.ts:43`; `timeline.ts:1214`; `captions/install.ts:144–145,357–368` | Selection plus reverse-ordered edit-request callbacks. Keep callback-bearing caption UI in E until a full facade is proven |
| Timeline tabs → compositions | `extensions/timeline/comp-tabs.ts:29`; `kernel/install.ts:956–963` | Data/open-tab state can live in K, but returned live composition objects and synchronous change callbacks need facade traces |
| Inspector → contributed inspector section | `layers-3d/index.ts:40–42`; inspector contributed-section builders call extension `when/build` and cleanup | The shipped 3D section is a synchronous UI callback edge, even without `services.get`. Include its callback edition in E, or redesign v4 dispatch without blocking another isolate |
| Kernel/legacy edit, model, history, app → editor providers | `legacy/core/services.ts:11–28`; `editing.ts:462,692,1133`; `history.ts:287`; `model.ts:15`; `app.ts:386,1181–1182` | Moving these modules into Rust must remove JS service calls from the actor path or use lease-authorized continuations; it cannot synchronously enter another isolate thread |
| Renderer → registered raster → saved base renderer | `kernel/install.ts:789–810` | Raster provider is a live callable service. Nested capture/override requires engine/page continuations in the same owning realm |
| Media/settings/workspace → viewer/timeline | `panels/AssetsPanel.svelte` uses `viewer.preview`; `settings/install.ts:31`; `legacy/core/workspace.ts:544–548` | These are extra consumers. A typed facade must cover the whole operation, including returns/events; otherwise their compat edition participates in E |

**Decision: E starts with timeline, inspector, viewer chrome and toolbar, including shader hooks, source preview and synchronous caption/3D-section callbacks.** Compositions, selection, import defaults, transport and scalar UI state are candidates for K-backed facades. DOM canvases/elements, `inspector.syncs`, closures and callback claim/decline protocols stay local to the UI edition until proven representable.

**Agent panel:** the tag does not list it as a separate extension in `kernel/builtins.ts`; it is installed through legacy/Svelte panel and agent bridges. There is no named `agent` provider among the shipped `services.register` sites. A separate agent isolate is a **native port proposal**, not a cluster derived from an existing isolated agent extension. Agent edits/commands must use K facades; any retained synchronous editor/DOM callback pulls its compatibility implementation into the shared page/E closure. Async agent-client protocol traffic alone does not require co-location.

**Remaining built-ins:** theme, effects and transitions are registration/data-oriented candidates for a rest cluster. Keymap dispatch goes through the active command resolver. Mods uses extension management and host UI. The 3D inspector callback prevents treating all remaining built-ins as independent by default. Additional dynamic command, menu, inspector and service registrations can extend these closures; fixture the edge before splitting it.

### Cluster rule and chosen backing policy

- All trusted v1–3 consumers/providers stay in one compatibility page per project window, with one live document and PM registry. Store sandbox runtimes do not get the trusted services API.
- A v1–3 replacement of any E provider switches the **entire dependent synchronous closure** to compat editions. For example, a timeline fork brings inspector, viewer chrome and toolbar with it; optional caption/3D callback consumers follow their actual edges. Do not leave a v4 consumer trying to synchronously call a page provider from another isolate.
- Prefer a K-backed `uiState` facade for operations fully representable as data/state. Co-locate live JS object services that cannot be fully represented. Async structured-clone services can be a new v4 contract; they cannot silently replace the old live-object contract.
- `services.get` returns the latest registered implementation **itself**. Repeated reads must be `===` until replacement/disposal. Held references still point to the old object after a new registration. Top disposal reveals the previous provider; buried disposal leaves the current top untouched. There is no service-change notification in `services.ts`.
- Cluster reload must retain service identity and callable history generations, survive failed activation and meet the reload gate. Atomic cluster reload versus staged replacement remains an S1 measurement. No synchronous isolate-thread RPC.

## Write API admission and return rules

These are **shipped source rules**, with native changes called out separately. All are synchronous to trusted callers unless a user callback itself returns a Promise. Sandbox `project.apply` and related operations are permission-checked async RPC, not the same live-object surface. Sandbox `history`, `edit`, `workspace` and `services` are denied.

| Write API | Admission and nesting | Return/error/publication |
| --- | --- | --- |
| `history.do(label, fn)` | Calls `begin`, which nests. No live-source-edit refusal. Executes `fn` inline. | Returns the exact callback value. Throw rolls back the **whole pending history** and rethrows; `finally` commits if still pending. A returned Promise is not awaited: commit occurs before its continuation. |
| `history.begin` | Increments depth. Outer begin captures serialized project; nested begin reuses existing pending entry, including a scoped edit entry. No idle auto-commit. | Outer returns before-snapshot string; nested returns existing `before` or `null` for a scoped entry with no snapshot. |
| `history.commit` | No depth/pending, remaining nested depth, unchanged document or empty patches → no new entry. | `false` in those cases; `true` for an outer changed commit. Push/history publication, project patch publication, touch and autosave are synchronous/delegated boundaries to trace in order. |
| `history.cancel` | Clears pending and resets depth. | `undefined`. **Keeps document edits; drops the pending entry.** No rollback. |
| `history.external` | Invalid undo/redo callbacks → no entry. Otherwise independently pushes a closure entry, even while document history is pending; it does not join `depth`. | Entry id or `null`. Retains undo/redo and optional cleanup, discards redo/trims budgets through push. The implementation does not forward `options.group` despite that field in the type. Closures can reenter synchronously. |
| `history.selection` | Refuses while history pending, on identical selections, or with an existing redo tail. | Entry id or `null`. Restore filters deleted layer IDs and notifies selection; navigation must not truncate redo. |
| `history.undo/redo` | Refuse only when no corresponding stack entry; no implicit close of an open begin. | Boolean; external closure runs inline. Unguarded closure errors can escape after stack cursor movement. Trace failure/reentry and continuation ordering. |
| `edit.apply` / `project.apply` | Expand commands; reject empty input or stale `baseRevision`. During live edit, use per-batch checkpoint; otherwise start scoped history, nesting with existing history. `project.apply` forces extension origin in host adapter. | `EditResult` with results/revision, or `{ok:false,message}` on caught failure. Invalid live batch restores its checkpoint; non-live failure rolls back history/selection. Shader hooks and inspector publication can synchronously reenter. |
| `edit.mutate` | **Refuses before action** if live source edit exists, or action is not callable. Raw structural action must change only layer tree/compositions, excluding allowed revision/provenance bookkeeping. History begin can nest. | Shipped implementation returns **`EditResult`**, with callback result in `data.result`; it does not return raw `T` despite the wider type. No-op is `ok:true`. Action/non-tree errors are caught, rolled back and returned as `ok:false`. Async action is not awaited. |
| `edit.begin` | Throws if source edit already live. Otherwise nested `beginScoped`, tracks revision/edits, retains initial selection and live label/origin. | `undefined`; no idle auto-commit. Raw history nesting remains possible. |
| `edit.dispatch` | Outside live → one-shot `apply`; inside live → `apply` with live label/origin. | `EditResult`; uses the same validation/checkpoint path. |
| `edit.commit` | No live → failure result. Clears live before finishing. No commands or no changes → cancels history. Otherwise records revision/provenance and commits. | `EditResult`; no-op is successful. Unexpected delegated exceptions are not wrapped in a new blanket catch. |
| `edit.cancel` | No live → `false`. Empty unmodified live gesture cancels pending history **without replacing project**, preserving mounted binding references. Changed live gesture rolls back scoped history/selection. | `true` when live was cancelled. In the tag, rollback can undo other writes folded into that scope; native selective cancellation is an explicit improvement below. |
| `EditGestureAPI.begin/write/commit/cancel` | `command`: optional `prepare`, then edit begin/dispatch/commit/cancel. `set`: history begin/raw setter/commit/cancel. `local`: setter only. No independent lock. | These four methods return `undefined`, discarding delegated `EditResult`/booleans. A failed dispatch is not automatically thrown. `set` cancellation **keeps** writes; command cancellation reverts. Setter/build/prepare exceptions are not swallowed here. |
| `EditGestureAPI.once` | Command mode optionally prepares and calls one-shot `edit.apply`; set mode calls `history.do`; local mode calls setter directly. | Command → `EditResult`; set → `history.do` callback result; local → `undefined`. |
| `workspace.mutate` | Editing mode mutates draft. Otherwise clone built-in unless `inPlace`; invoke callback, normalize, save and activate. No source-edit admission or document-history transaction. | Returns resulting current workspace. Callback errors are not caught/rolled back, so partial writes can remain. Non-built-in `current()` returns JSON copy; appearance ownership is recorded in `finally` and unwinds on extension disposal. |
| `PM.proj` raw writes, live model helpers and `PM.touch` | Tag writes are ordinary same-realm JS mutations; no general kernel admission gate is present. `touch` clears evaluator/index caches. Caller-chosen history/edit/notification wrappers govern persistence and publication. | Assignment returns assigned value; `touch` returns `undefined`. Following same-stack reads see raw mutation. A raw write alone does not promise undo, revision bump, save or a project-change event. |

Primary evidence: `legacy/core/history.ts:142–300`; `legacy/core/editing.ts:1280–1420`; `controls/gesture.ts:26–70`; `legacy/core/workspace.ts:560–589`; `kernel/workspace-appearance.ts:187–217`; `kernel/install.ts:175–320`; `legacy/core/anim.ts:19–28`.

**Native disposition:** short kernel transactions validate and acknowledge in each API's own return/error form; changes publish after kernel commit. Open gestures remain joinable: other admitted operations apply immediately and fold, rather than waiting for the gesture. Preserve `edit.mutate`'s live-source-edit refusal. Direct page writes use the proposed per-path validated flush barrier, resync/error on rejection, and retained-reference fixtures; this barrier is not an existing tag implementation.

A native `edit.cancel` reverts only owner operations. Acknowledged foreign writes become separate history, and conflicting inverses produce commit-with-notice instead of deleting acknowledged work. Crash/hang/disposal seals acknowledged operations. These are deliberate native guarantees, **not the tag's rollback trace**. Fixtures must record both the reference and the approved difference. Lost acknowledgements require transaction-id lookup/deduplication. External undo and rendering can require inline page continuations under the lease.

## DOM key order and decline rules

Source: `kernel/registries.ts:285–307,422–513,558–564`; `keychord.ts`; `legacy/ui/shortcuts.ts:43–60,98–106,480–489`; sandbox view forwarding in `sandbox/boot.ts:156–175,214–228`.

1. Normal DOM capture, target and bubble phases run in the focused document. Target/ancestor panel handlers precede the kernel **window bubble** listener. Listeners on the same target run in registration order; the kernel installs before built-in runtime window listeners. Do not claim that every panel window handler precedes the keymap.
2. The keymap converts the event to a normalized chord. Bare modifiers/dead/unidentified keys do not form chords. It ignores host-document echoes targeting an `ext-panel-frame`.
3. Enter on a focused `button`, linked anchor, `select`, button role or radio role returns before editor shortcuts. Highlighted document text owns Cmd/Ctrl+C. Field detection traverses editable ancestors and `composedPath`; inputs/textareas/selects/contenteditable stay in a field context. A selectable read-only text root handles Cmd/Ctrl+A by selecting its text, then returns without applying editor shortcuts.
4. Candidates sort by **ascending numeric priority**, then exact chord before loose match, then greater modifier specificity when loose matching is involved, then registration sequence. Filter field bindings without `inFields`; filter repeats without `repeat`.
5. Ordinary non-repeat, non-`inFields` candidates call `preventDefault()` **before command dispatch**. Literal `false` tries the next candidate, but that prior cancellation remains set. Field-aware and repeat candidates cancel only after a result other than literal `false`. A thrown dispatch logs and ends that handler.
6. The tag's handler checks **literal synchronous `false`**, not an awaited result. `undefined`, `null`, `0` and a Promise object all count as handled. A later Promise resolution to false does not restart DOM dispatch. A missing command returns false.
7. The listener does **not** check `event.defaultPrevented` before dispatch. `preventDefault` in a panel is not equivalent to stopping propagation. A target/ancestor `stopPropagation` can keep the event from reaching window; same-window listener order and `stopImmediatePropagation` need their own fixtures. The kernel does not call `stopPropagation` on handled keys.
8. Browser text/menu defaults occur after DOM dispatch unless cancelled. Key-up remains a separate DOM event for the existing listeners; it is not run through this keydown command loop.

Most legacy `def()` wrappers replace a returned literal false with undefined. Only `preserveFalse` registrations pass it through; `blurField` is directly registered, blurs a field and deliberately returns false. Therefore a helper returning false does **not** establish that its keybinding declines. The CSV includes the concrete preserve-false/default-wrapper cases. Unknown, removed, overridden and extension commands remain conservative may-decline cases. A legacy wrapper around an unproven Promise cannot establish the new resolver's never-decline class either.

The native resolver must hold may-decline decisions in order without blocking the main thread; only proven active never-decline registrations can be consumed eagerly. Override/removal invalidates the proof. Preserve DOM-first dispatch in compat focus. Trace the proposed async-decline replay against the tag's immediate-false and cancellation timing before classifying any difference as an intentional change. Escape/Cmd-period/Cmd-Q bypass and watchdog policy belong to the native resolver; they do not replace the compat DOM's authored handlers.

Store views are a separate document: local target/bubble handlers precede their window forwarding listener. `keyToForward` forwards matching keys plus Escape/Tab, pre-cancelling ordinary non-field matches. Host trusted-input dispatch skips the originating extension, applies field/repeat filters and handles only keyDown. Do not route the same physical press through both DOM forwarding and a host echo. Focus/pointer/user-activation gates also control permitted UI operations such as clipboard writes.

## Compatibility-page and Store risks

| Boundary | Required behavior and trace |
| --- | --- |
| Synchronous pixels | `render.renderFrameTo` returns a real canvas; `render.snapshot` returns JPEG data URL; `render.raster` can return a cached surface. Preserve nested render/override/upload callbacks, held canvas/context references and read-after-write. `project.snapshot` is a Promise wrapper around the synchronous capture, not proof of source-decoding readiness. |
| Presenter/WebGL | `render.gl.init/resize/context/previewViewport`, viewer canvas binding and `frame:rendered` remain usable via page WebGL/readback. Fixture custom canvas bind → resize → pixels → frame event even if measured use is zero. `frame.viewport` must be the exact preview viewport object for that presentation. |
| Live services | `get` returns live mutable provider objects, not serialized snapshots. DOM elements, `inspector.syncs`, graph/path/text selection, source preview and retained callback identity require page editions or complete facades. Registration stack and held-old-object behavior are part of the contract. |
| Direct writes | Live project, layer, property and keyframe references plus public PM assignment/replacement can bypass typed edits. Trace explicit cache touches, no-history mutations, observers, flush timing, validation rejection/resync and races with acknowledged native writes. |
| Recorder canvas | All creation paths: `document.createElement`, parser/innerHTML, cloning, popouts and supplied surfaces. Same element/context through promotion. `drawImage(A)` into B, repaint A, then rasterize B must use the recorded source version. Include OffscreenCanvas/bitmap lifetime, gradients, clipping, text metrics and sync pixel reads. |
| Store runtime DOM | Hidden runtime is still a full document with `allow-scripts allow-forms`, not a worker. `Image`, `createImageBitmap`, `OffscreenCanvas`, `DOMParser`, base64 and EventTarget need explicit runtime fixtures. Existing DOM use triggers a shim or per-extension runtime WebView fallback; lack of catalog evidence is not grounds to deny it. |
| Store runtime/view split | Boot replays activation for a view with quiet/role-specific registration behavior. Project snapshots are deep-frozen and generation-cached; writes/storage/asset queries can be async even for old apiVersions. Events arrive batched, with time/selection latest-value coalescing. Preserve role-specific copy/menu/permission behavior. |
| Font/DOM assets | `media.assets.get().el` is a real media element in trusted code. Fonts loaded with FontFace/document.fonts must reach native text measurement/rasterization. Retain loaded-font, media currentTime, decode, URL and disposal behavior. |
| Security shape | Opaque origin denies origin storage and same-origin host access. CSP allows local data/blob uses and permission-gated remote network, blob workers, no child frames, no navigation/form escape. Full-access Store code takes the trusted path; it must not be counted as sandbox API permission. |

Pure helpers/ECMAScript primitives remain JS values in the owning realm. In the CSV, `kernel` backing for a runtime intrinsic means the native runtime host supplies it; it does not mean every Math/Array/Promise operation crosses the kernel actor. `engine` backing for canvas still requires a real page-owned element where the API exposes one. `compat-page-only` for Store DOM includes the per-extension runtime/view WebView fallback, not sharing the trusted app page.

### Trace inventory and completion gate

All rows intentionally say `trace_needed=yes`. Repeated profile labels organize fixtures, not exemptions: **B** basic calls/errors/identity; **R** registration/override/disposal; **W** transactions/cancel/history; **KEY** focus/DOM/decline/repeat; **S** live services/cluster switching; **EV** event ordering; **PIX** render continuations/presenter; **H** helpers/aliases; **G** runtime global descriptors/permissions; **C** recorder/pixel/text semantics; **DIR/LEG** raw writes and legacy registry behavior.

The first reference fixtures should cover write admission/return forms and DOM keys, followed by service identity/cluster switching and nested pixels. For each fixture record call and return shape, object identity/aliases, errors, callbacks and event order, document/selection/undo state, and role/permission context. No private source or identifiers enter fixtures or traces committed publicly; use public/synthetic reproductions.

Source validation performed for this inventory: 43 compiler helper names reconcile with editor-helper exports; typed API and provider-member extraction reconcile with the tag; canonical category/member pairs are unique; counts stay within scan populations; required write/presenter/runtime-global rows are present; all rows have source, realm, trace and backing fields. Frozen Electron execution, native performance budgets, S1/S5 race testing and visual/spec comparison remain gates for the implementation lanes.

## Open questions for Jude

1. **Reconcile the reference build before traces freeze.** This tag's `RenderAPI`/`makeRender` have no `renderFrameToAsync`, `snapshotAsync` or `rasterAsync` siblings. `PM.Export.snapshotAsync` exists at `legacy/core/exporter.ts:1130`. The tag also binds J/K to visible event navigation and L to audio reveal, and no general direct-write admission barrier was found. If another shipped build is intended, pin it explicitly; until then this matrix uses the requested tag and treats the extra siblings/barrier/shuttle description as unverified. Trusted `api.apiVersion` is also hard-coded to 1 in `host.ts:530`, while sandbox reports `init.apiVersion`.
2. **Store source snapshot: answered (2026-11-01).** Anonymous public reads covered all **4** current catalog extensions; Store counts and occurrence totals are recorded above. Runtime-global availability and compatibility traces remain open; static zero counts do not establish a DOM-free runtime.
3. **Confirm classification of source/reference differences.** Native async-decline replay and short-transaction validation interact with the tag's literal-false/pre-cancel policy, partial workspace mutations, raw direct writes and Promise-returning mutation callbacks. Record which are authored contract and which are intentional fixes. Selective cancel/acknowledged-write preservation is already the native rule; do not silently relabel tag rollback as selective cancel.
4. **Resolve facade completeness with S1/S5 evidence.** Can captions edit-request claims, contributed inspector sections, source preview, external-history closures and wider runtime service fields be represented without cross-isolate synchronous calls? Use E/compat closure until yes. Runtime globals needing full DOM take the per-extension WebView fallback; no affected extension is excluded.

These questions do not authorize dropping a member or changing the reference. Current decisions are to preserve the live trusted realm, keep the wider synchronous closure together, use data facades only where complete, and carry the remaining differences as explicit fixtures/decisions.
