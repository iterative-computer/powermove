# The headless document host (Phase 3c, lane L5)

Design notes for `apps/desktop/src/engine/` (plan sections 6, 21, and 22; contract 15;
amendments 21 and 22). The document stays TypeScript (`server/mcp.ts:26-27`): a
document host runs the editor's own model, history, editing, agent harness,
captions service, and extension registry without a browser, on pm-wasm.

## Shape

```
embedder (serve supervisor: engine/main.ts; hidden MCP host: agents lane)
  │  DocumentHostPoolApi            ← frozen, version 1 (document-host/protocol.ts)
  ▼
DocumentHostPool (document-host/pool.ts)
  │  child-process IPC (ToWorker / FromWorker, structured clone)
  ├── worker: project A   (node engine.mjs --document-worker)
  │     pm-wasm mirror, kernel, history, harness, captions, extension registry
  └── worker: project B   (its own process, its own everything)
```

- **One process per open project.** RFC 15's open question 1 is decided for
  processes: a crash in one project's kernel never takes another down, every module
  global of the editor (`window.PM`, the extension store, the font loader) is
  per project by construction, and CoreText's font registry needs a main thread,
  which a worker thread does not have. A worker boots in about 0.3 s.
- **The supervisor** (`engine.mjs <url> <token>`, forked by `powermove serve`)
  connects to the host like a tab and forwards each tool request to the pool by its
  `projectId`. It no longer has a document of its own, so opening project B never
  clears project A's history (the old `PM.hist.clear()` on every switch is gone).
- **A worker** (`document-host/worker.ts`) boots the chain (`chain.ts`), joins the
  project's session, resumes its persisted undo history, boots the built-in
  extensions against the project, and answers calls.

## The worker interface (frozen, version 1)

`document-host/protocol.ts`. The agents lane builds the hidden MCP host on it.

| Member | Contract 15 | Meaning |
|---|---|---|
| `open(projectId, { pinnedBy })` | `OpenProject` / `ProjectOpened` | Starts (or returns) the project's worker; answers the instance, revision, restored undo entries, and which engine answers (`pm-wasm`, mode, fonts) |
| `call(projectId, request)` | `ToolCall` / `ToolResult` | A document-host tool call in the tools' existing JSON (`AgentToolRequestEvent` → `AgentToolResponseEvent`); opens the instance first; refuses DOM and job tools (`wrong-route`) |
| `persist(projectId, reason)` | `Persist` / `Persisted` | Writes the instance state now |
| `registerExtension(projectId, record)` | `RegisterExtension` / `ExtensionRegistered` | Activates (or hot-reloads) an extension in the instance's kernel; answers after activation, so the next call sees its contributions |
| `stats(projectId)` | (additive) | Revision, undo depth, fallbacks by `<op>:<reason>`, native calls, auditor, dual answers, consistency checks, named failures, extensions |
| `close(projectId)`, `shutdown()` | `CloseProject` | Persist, leave the session, stop the process |

What the embedder provides (`DocumentHostServices`): the profile store
(`read`/`write`/`remove`), the project session (`join`, `patch`, `leave`,
`onPatch` for other members' edits), `mediaPath.lookup` (agent media and captions
find a project's media in the host's store), and `transcription` (status,
transcribe, cancel, progress). The serve supervisor implements them over its link
(`linkServices` in `engine/main.ts`: `store:get-sync`/`store:set`/`store:delete`,
`web:sync-*`, `media-path:lookup`); the hidden MCP host implements them in-process.

Failures are named (`DocumentHostFailure`: `code`, `message`, `retry`):
`pm-wasm-unavailable`, `font-service-unavailable`, `project-missing`,
`requires-new-engine`,
`worker-crashed`, `worker-timeout`, `closed`, `wrong-route`,
`sandbox-unavailable`, `service-unavailable`, `worker-error`. The supervisor turns
them into the tool's error text, with "(retry the call)" when `retry` holds.

## Invariants as implemented

1. **History per instance, persisted.** After every call that changed the document,
   on close, and at shutdown, a worker writes `documentHost.<projectId>`
   (`{ v: 1, projectId, hash, revision, history: PM.hist.export(), savedAt }`) to
   the profile store. On open it imports it (`PM.hist.import`), so a restarted host
   or a crashed worker resumes the same undo stack. The key is the host's own: a
   desktop tab's `projectHistory.<id>` is never touched (each member keeps its own
   undo, as in remote-sync). When another member changed the document since, undo
   applies on top, as it does in a tab.
2. **Read-your-writes across the hop.** Before a call answers, the worker diffs its
   document against what the session last had from it, marks it synced and queues the
   patches, all synchronously (as `remote-sync.ts` does), and awaits the host's
   acknowledgement of that batch. An edit made while an earlier batch or a remote
   patch is in flight is therefore always in a later batch, never marked synced
   unsent. Remote patches apply at once without touching history. When a remote
   patch writes a path one of the worker's unacknowledged batches also writes (the
   host's order, which the worker cannot see, decides the value), and after a batch
   the host refused, the worker reconciles behind its sends: it joins again, takes the
   host's copy, puts its unsent edits on top, and sends them. While a reconciliation
   is pending no batch is cut, so every remote patch is rebased: edits the host has
   not seen (the diff from `synced`, and refused paths kept for a retry) go back on
   top of the patch, because the host applies them after it. A remote patch that
   arrives while the reconciliation's join is in flight is applied to the host's copy
   too when its seq is newer than the copy's. A refused batch whose value a remote
   patch overwrote in flight gets its value back in the document (the host never
   applied it), unless a later local edit wrote the same path. `stats().hash` is the
   instance document's canonical hash, which tests compare with the host's.
3. **Concurrent calls.** Tool calls run concurrently inside a worker (the harness
   keeps one transaction per run and checks base revisions); lifecycle requests run
   in order. Each answer is matched to its call by run and call id.
4. **Extensions register before the call returns.** `registerExtension` adds the
   record to the worker's list and sends the kernel's loader an `extensionsChanged`
   for it, so `planLoad` decides as in the editor (a disabled record stays off, a fork
   replaces its built-in, a missing dependency or blocked state skips it, a Store
   record runs contained); it answers once the loader settled. `definitionsGeneration`
   counts registrations in this instance; pushing definition snapshots to `pm-node`
   for `render_frames` is the agents lane's (contract 10).
5. **Crash and backoff.** A worker that exits rejects its pending calls with
   `worker-crashed` (retry); the next call starts it again after 1 s, doubling to
   30 s, up to five attempts, as the desktop's engine supervisor does. A crash more
   than a minute ago no longer counts, so a retry after the budget ran out can work.
   History persists once a burst of changing calls settles (750 ms), on close and at
   shutdown; the document itself is in the session before each answer.
6. **The production consistency check** runs after every changing call: the mirror's
   hash must equal the document's.
7. **The shared reader guards** (`shared/engine-stamp.ts`, `shared/reader-version.ts`)
   apply as on the desktop: a project stamped `requiresEngine: "native"` is refused
   with `requires-new-engine` when the new engine is off; a project that needs a newer
   reader opens read-only (`InstanceOpened.readOnly`), so its edits stay in the
   instance and never reach the session or the profile store.

## The engine policy

`chain.ts installHeadlessEngineDoc`, with "Use New Engine" read by the embedder
(`useNewEngine` in the profile store, `POWERMOVE_NEW_ENGINE` overriding):

| | New engine on | New engine off |
|---|---|---|
| pm-wasm missing or fails to load | `pm-wasm-unavailable`, retry | today's behavior: the legacy evaluator, logged |
| Font service (pm-bridge `FontRegistry`) missing | `font-service-unavailable`, retry | loaded only with `POWERMOVE_ENGINE_FONTS=1` |
| Evaluator mode (production) | `on` | `off` |
| Evaluator mode (development/test build) | at least `on` | `POWERMOVE_NATIVE_EVALUATOR` (default `on`) |
| `installEngineDoc` `strict` | yes: named failures (below) | no |

A retry is a new worker: the pool drops a worker whose open failed, so the next call
checks again (a rebuilt module is picked up without restarting the host).

## What the chain installs

`chain.ts bootChain`, in the renderer's order (legacy/bootstrap.ts): diag, util, the
kernel, ui-state, memory, fonts, easing, model, selection, anim, history, the
document mirror, library, projects, editing, capabilities, **media, gl/shaders,
gl/transitions, captions**, workspace, the harness; then, once the project is open,
**the extension loader with the built-ins**. The bold ones are new: before, the
headless host had no effect or transition registry (`add_effect` answered "Unknown
effect" for every basic effect) and no captions service (`generate_captions`
dereferenced a missing `PM.Captions`). Built-ins boot against the open project, so
`timeline` and `inspector` activate as on the desktop.

The bridge the editor's modules see carries `agentTools`, `mediaPath`, and
`transcription` (forwarding to host services), and is set on both the DOM's window
and `globalThis` (`kernel/bridge.ts` reads `globalThis`; in Node the DOM's window is
a separate object, which is why the old engine's harness never saw its bridge
outside the happy-dom test environment).

## Captions without a DOM

`generate_captions` runs the editor's own `PM.Captions.generate`: the media path from
`mediaPath.lookup`, the transcript from `transcription.transcribe`, progress events
fanned out by the pool, cues applied inside the run's transaction. Without a
transcription service the host reports `available: false`, and the tool answers that
no model is installed, as the web host does.

## Exact answers and named failures (engine-doc)

Shared with the editor (docs/engine/write-path.md has the details):

| Reason (before) | Now |
|---|---|
| `trap` | The question is asked again on a fresh instance, made synchronously from the kept module and resynced: the answer is native. A second trap is `pm-wasm-trap` (named) |
| `detached-prop`, `detached-parent`, `detached-path` | Gone: the channel, parent layer, or path goes to pm-wasm as JSON and is evaluated ad hoc (`hasKeyAt`, `transformParentMatrix`, `parent3D`, `pathValues`, `groupMatrix`); dual answers equal the legacy ones |
| a non-object where a channel or path belongs | Native: the legacy TypeError and message (`p.kf`, `path.id`, `path.p`, `Object.entries`) thrown before the layer is read, as today |
| `error:*` | Still counted; `pm-wasm-error` named failure with the new engine on |
| `unavailable` | Workers never answer before the module is ready (open fails by name). In the editor, the boot window before `whenReady` remains (below) |
| `fonts` | Synchronous registry answers and whole faces in workers and the editor; missing faces/services are named failures with Retry |
| `ink` | Whole faces supply outlines in both hosts |
| `legacy` (pm-edit `LEGACY_ONLY`, the self-space camera reparent) | Gone: pm-edit answers it natively, reproducing the legacy stale-cache read (docs/engine/pm-edit.md) |
| `context` in `editKernels.align` (layers outside the open composition, empty selection) and its kernel's `host`/`fonts`/`ink` | The caller's own code answers (the strip's `alignCommands`, extension code over `api.anim`/`api.render`, native or failing by name), counted in `stats.callers`; never the legacy evaluator |
| `host`, `3d`, `flush`, `exception`, other missing context | Named strict failure; never a legacy answer |

`gl.bounds` and `gl.pick` have no legacy reference headlessly (no compositor).
The desktop contract records real Canvas measurements and replays them through
the unmodified legacy font-anchor kernels in a test-built worker. Their dual
comparison counts are required to be nonzero; missing measurements fail the run.

The dual, audit, and tracking modes are development-only: a production build takes
them only from its build flag; a profile's stored override can turn the plain mirror
on or off and nothing else (`flags.ts profileOverride`).

## Extensions

The worker's bridge carries the editor's extensions bridge (`document-host/extensions.ts`)
over the embedder's optional `extensions` service (`list`, `bundle(url)`,
`onChanged`, `values`), so the kernel's own loader boots the user's extensions as the
editor does: built-ins first, then user and project extensions, overrides and forks
(`replaces`), `dependsOn`, enabled state and blocked health, through `planLoad`, and
reloads on `extensionsChanged`. Bundles are fetched once per URL into a worker temp
directory (its real path) and imported from there. Health stays in the worker (the
editor is the authority for what Mods shows); `stats().extensions.failed` reports it.
The serve supervisor lists records over its link (`EXT_IPC.list`, `EXT_IPC.changed`)
and fetches bundles from the host's `/ext/` route with the session cookie; vars values
stay on the Mac that runs Powermove, as for a tab. Without the service a worker boots
the built-ins only.

## Contained runtimes (Store extensions and scripts)

A Store extension runs its sandbox runtime role (`renderer/sandbox/boot.ts`
`bootRuntime`) in a process of its own from `sandbox-runtime.mjs` (a second entry of
the engine bundle). The kernel's own sandbox host (`kernel/sandbox-host.ts`) drives it
exactly as it drives a frame, through the `process` option (`loaded`, `start(port,
init)`, `terminate`): the same RPC, schemas, budgets, watchdog (it is isolated, so a
spinning runtime is killed and turned off) and permission checks, over the process's
IPC channel instead of a frame's port. Disposing the runtime stops the process.

The process (`document-host/contained.ts`) is held three ways:

1. **macOS seatbelt** (`sandbox-exec`): deny by default; read only Node, the system
   libraries, the runtime bundle (and the `ws` package happy-dom imports) and the
   extension's bundle, with metadata only on their ancestors; write nothing but
   `/dev/null`; exec nothing but Node; no network unless the manifest has `network`
   (then outbound TCP and DNS only).
2. **Node's permission model** (`--permission`): the same reads, no writes, no child
   processes, worker threads, addons or WASI.
3. **Inside the runtime**: a resolve hook refuses every import the bundle makes (it
   is self-contained, as under the frame's `script-src`), `process.binding`,
   `getBuiltinModule`, `dlopen`, `kill` and friends are removed before it runs, and
   `fetch`/`WebSocket` follow the CSP's `connect-src` (data:, blob:, and with
   `network` https: and wss:), reporting violations as the frame does. The environment
   is scrubbed to `PATH`, `LANG`, `TZ`.

Without seatbelt (not macOS) layers 2 and 3 hold; the worker logs which
(`containment()`). Generated scripts (`PM.Script.preview`/`apply`) run the editor's
own worker source the same way (`PM.Script.isolate`, `--script`), with no network, the
frame's time limit and its error texts. Store records whose runtime bundle is missing
fail activation by name; nothing runs uncontained.

## Media status and file state

`PM.assets` in a worker is a status view over the host's media store
(`document-host/media.ts`): an asset is available when `mediaPath.lookup` finds its
bytes and offline when not; `dur`, `w`, `h`, `hasAudio` come from the asset record,
which the editor's decoder wrote at import. Lookups settle before every call, so
`check_project`, `list_media`, `edit_video` and media sources answer as on the desktop
and never depend on timing. Importing and replacing media still refuse in a worker
(survey C24). `get_workspace_state` reads the project's file state from the profile
store (`projectMeta.<id>`), unsaved unless the document's fingerprint equals the saved
or baseline hash; a worker reports the historical hidden Chromium context's viewport (1440 × 1000, DPR 1).

## Text ink

Workers load whole faces (pm-bridge `FontRegistry.whole`, `pm_text::subset::whole_face`:
a single-face file's bytes, or a collection face's tables as one sfnt) and declare
outlines to pm-wasm, so font-anchor capture, anchor references and text bounds are
native; with the new engine on a registry without `whole` is `font-service-unavailable`.
Anchor bounds keep the legacy key order (x0, y0, x1, y1) whichever side measured them
(`edits.ts legacyKeyOrder`), so answers that serialize the live document are byte-for-byte
the desktop's. Where there is no raster, `PM.textLayout` (the glyph layout
`get_project_state` reports for selected text) is pm-wasm's `glyphLayout`; kernels and
queries with no headless legacy reference (`captureFontAnchor`, `anchorReference`,
`glyphLayout`, `gl.bounds`, `gl.pick`) have no live headless reference.
The Electron contract replays recorded Canvas rasters for the font-anchor kernels
and requires that both ink kernels were actually dual-compared.

## The editor with the new engine on

`legacy/bootstrap.ts` installs the UI without awaiting the mirror or fonts.
Once the mirror is ready, fonts warm in the background, also on later project
opens. Warming visits every composition (including nested `comps`) and every
keyed time in each text layer. Question-time font misses resolve synchronously
through the main-frame-only font channel; whole faces carry outlines.

Every strict native refusal reports a named `EngineFailure` (bus, report, existing
failure toast), then throws. No fallback branch calls the legacy evaluator in
strict production mode. Audit builds intentionally execute references for comparison.
Retry resets a disabled font loader, including its face cache; a missing bridge is
a named font-service failure on the first text question.

## Interfaces to other lanes

- **Fonts.** `main/engine-fonts.ts` owns one synchronous CoreText registry per
  editor sender. The preload exposes `answerSync` and `wholeSync`. Project font
  mutations update that sender's registry before acknowledging, alongside the
  asynchronous interactive-engine service. The shared channel keeps the same
  sender check and byte/query limits. Failed registry creation retries on the next
  request; a failed face load is not marked loaded.
- **3D (host, 3d).** pm-wasm's `RefuseHost` and the `3d` pick fallback are the threed
  lane's (`pm-geom3d` in pm-wasm). Nothing here routes around them.
- **Agents (hidden MCP host).** Builds on `DocumentHostPoolApi`, implements
  `DocumentHostServices` (including `extensions`) in the CLI process, routes DOM, job,
  and host-services tools itself (`DOM_TOOLS`, `JOB_TOOLS` mirror
  `pm_protocol::document_host`), and owns export and save routing (`server/runs.ts`).

## Verification

| Check | Command | Result (2026-10-18, M4 Max) |
|---|---|---|
| Pool against real workers | `vitest run src/engine/document-host/pool.test.ts` | 10 tests: the previous 8, plus the reviewer's sequence step by step (T1's batch in flight, remote patch, T2 edits, T2's batch in flight, T3 edits: every edit reaches the host, hashes equal; fails on the previous worker) and a same-path conflict converging on the host's order |
| Worker without a browser | `vitest run src/engine/document-host/hosts.test.ts` | 7 tests: captions through host services; no-model answer; 2D and text edits (size, weight, tracking, font) in `audit` mode with **0 fallbacks, 0 dual mismatches, 0 auditor failures**; font-anchor bounds recorded natively with 0 fallbacks; user extensions with a fork replacing `effects-basic`, a missing dependency, a disabled record and a live disable; registration and hot reload through the loader; a Store extension contained (its effects work; `node:fs`, `node:child_process`, fetch, native process members refused; clean environment) |
| Contained runtimes | `vitest run src/engine/document-host/contained.test.ts` | 5 tests: scripts' results, time limit and errors as the frame gives them; a contained process cannot read home or `/etc`, write, exec, and gets a clean environment; with seatbelt it cannot connect without `network` and can with it |
| Contract sequence | `vitest run src/engine/document-host/contract.test.ts` | 32 calls (4 expected refusals), two workers byte-identical |
| Serve wiring | `vitest run src/engine/document-host/serve.integration.test.ts` | a tab and a worker share the session both ways; a user extension in the host profile is built by the serve host, listed over the link, fetched from `/ext/` and used by the worker; history reaches the host's profile store |
| Desktop vs worker | `pm-electron-lock playwright test e2e/document-host-contract.spec.ts --workers=1` | 32 calls from the same document, errors included: **byte-identical canonical documents and identical answers, with no pinned difference**; `get_workspace_state`'s document fields equal with the desktop's extension list |
| Exact answers | `vitest run src/renderer/src/engine-doc/strict.test.ts` | 6 tests: the previous 5, plus warmed faces answer the editor's first text question natively (an unwarmed one falls back, measured) |

## Round 4 verification and remaining work

The earlier verification table above is historical. Round 4 was rerun on
integration tip `e9a80939` (protocol `c137021c07039329`):

| Check | Observed result |
|---|---|
| Pool | 15/15, including 32 seeded interleavings, reconciliation-send windows, refused-batch retry and changed-hash history import |
| Strict query/edit guards | 12/12, including nested/replacement scopes, on-demand fonts, keyed nested font warmup, disabled-loader recovery and explicit group lists |
| Desktop / worker, setting off and on | Both pass: 43 answers (6 expected errors) and live document bytes after every call; only generated ids renamed. Each headless run compares 144 dual answers, including font-anchor capture/reference, with 0 mismatches, 0 fallbacks and 0 auditor failures |
| Real app boot | Five alternating pairs, no engine failures. UI p50 off/on 986.35/964.98 ms; observed max 1010.02/1013.63 ms. Editor p50 1396.28/1363.28 ms. These are five observations per mode, not a tail-latency estimate |
| Required broader unit selection | 138 files passed, 1 failed, 2 skipped; 1157 tests passed, 1 failed, 2 skipped. Failure: unchanged `legacy/core/project-file.test.ts:153`, a 5000 ms timeout on the 3 MiB Blob comparison, also reproduced in isolation. No timeout or tolerance was changed |
| Rust | pm-wasm, pm-eval and pm-text: 102 standard tests across 19 successful targets; the custom 5200-case Canvas text corpus also passes, identical breaks and max layout error 0.001047 px |
| Svelte / node TypeScript | Svelte: 0 errors, 1 existing warning. Node tsc: 75 existing errors (missing cloud generated types, resulting API types, and TS6307 file-list issues); no errors in the lane files |

Remaining parity gaps are blockers, not allowed differences. Detailed transcripts,
full workspace-state fields and diagnostics are retained outside Git in the lane
report's evidence paths.

- Reconciliation flushes chain a fresh batch behind a pending reconciliation;
  the reconciliation's own batch joins `unacked`. A conflict during that send
  requests another reconciliation. Refused paths retain their current values for
  the next retry instead of disappearing into the acknowledged baseline. Each
  successful tool answer waits for all of that work. Pool tests cover the two named windows and 32 seeded interleavings.
- Nested compositions use canonical member-path addresses (`@path:`), retaining
  identity even when nested maps reuse ids. A detached current scope during
  replacement is evaluated natively from `compJson`. A project without `compId`
  uses its stored root id. Both query and interned-property paths resolve nested
  addresses. Tests compare nested and replacement queries against the old evaluator.
- Production workers ignore `POWERMOVE_NATIVE_EVALUATOR` entirely; only the setting
  decides. Test bundles explicitly use Vite mode `test`. Deterministic clock/id
  seams and Canvas-reference replay are compiled out of production.
- The contract includes `manage_media` rename/move/delete and every folder action,
  referenced-delete refusal and `removeReferenced`, and `__composition_info`.
  `DOCUMENT_TOOLS` documents their default route. A coverage test audits the public
  spec and harness dispatch names against the sequence and explicit exclusions.
  Document comparison uses the worker's live `JSON.stringify(PM.proj)` bytes,
  exposed only by an explicitly seeded test build, and the desktop's live bytes.
  Only minted ids are renamed; there is no composition projection or key sorting.
- `get_workspace_state` prefers a tab on the run's project in `RunHub`; an unrelated
  tab is never selected. Without a tab, the worker's viewport is today's hidden
  Chromium context, 1440 × 1000 at DPR 1 (`server/mcp.ts`). Exact persisted window
  layout and open-project fields still need the hidden-host embedder's window
  state integration; the document fields remain contract-compared. This is a gap,
  not an accepted difference.
- Persisted history is imported even if the document hash changed, matching the
  desktop's unconditional import. A test changes the stored document between
  worker lifetimes and verifies the retained history and foreign edit.
- Native 3D bounds/picking still require the threed lane's host geometry service.
  Calls reaching those queries (`get_project_state` with selected 3D content and
  geometry edits through `apply_commands`, including alignment/group placement)
  now fail visibly instead of silently invoking three.js. (The cyclic camera/light
  reparent named here in round 4 is ported in round 5.)
- Host media import/replace, render pixel output, and staging/rebase are explicit
  corpus exclusions with reasons; their service-specific gates remain with the
  media, agents and extension lanes. No missing tool is silently excluded.
- Detached `NativeEdits` leave-attributes and anchor-reference helpers
  are named `context` failures when the layer is outside the open document
  (alignment runs the caller's own code since round 5). They
  never execute a legacy fallback in strict mode. Normal document tool targets
  remain live ids; none of the 43 contract calls hits this failure.
- The 750 ms history debounce still means SIGKILL inside that interval can lose
  undo entries; the document itself has reached the session before each answer.
- Store containment limitations from the earlier rounds remain: no OS sandbox
  outside macOS, and no headless XHR/EventSource/Web Workers/Blob IPC parity.

## Round 5 (review fixes) verification and remaining work

Rerun on integration tip `e9a80939` (unchanged since round 4).

- **Read-your-writes during a reconciliation** (blocking finding 1): fixed as in
  invariant 2. The reviewer's reproduction (an edit during the reconciliation's own
  send, then a same-path remote patch) is a pool test; it ends with the edit, as
  outside a reconciliation.
- **Seeded schedules** (finding 2): `sync-schedule.test.ts` replaces the two fixed
  schedules. Each seed drives a real worker step by step and chooses, at every step,
  among tool calls on colliding and disjoint values (composition name, duration, new
  layers, layer names), remote patches on the same values by path, and releasing or
  refusing the oldest held round trip; it also picks whether the host holds each
  send or join before applying (snapshotting) or after (ack or copy in flight).
  Read-your-writes is checked the moment each answer arrives (the host received a
  worker state holding the call's value, or a later local value of the same slot);
  at the end the hashes must match and every refused edit must have reached the
  host unless a later call wrote the same slot. The default 64 seeds: 448 calls,
  320 remote patches, 91 reconciliation joins, 37 refused sends, 9 refused joins,
  23 calls and 19 remote patches during a held join, 22 calls and 16 remote patches
  during a held reconciliation send; 0 failures. 300 more seeds from another base:
  0 failures. The previous worker fails 6 of the first 60 seeds (one answered edit
  never reached the host; refused edits lost to a remote patch).
- **Composition addresses** (finding 3): a top-level precomposition is checked by
  `proj.comps[cur.id] === cur` again, and a nested one comes from a path cache
  verified by walking the path on each call. On the reviewer's 100-layer, 40-keyed
  property shape: 7.96 ms per scope lookup before, 0.15 µs (precomposition) and
  0.39 µs (nested) after (strict test, bound 50 µs).
- **Cyclic camera/light reparent** (finding 4): ported; see docs/engine/pm-edit.md.
  The 36 committed `selfSpace` vectors match, 216 more from a fresh seed match, and
  without the stale read the 36 fail.
- **Align** (finding 5): `editKernels.align` runs the caller's own code for layers
  outside the open composition, empty selections, and the kernel's host/fonts/ink
  refusals; engine faults stay named failures.
- Also: non-object detached inputs throw the legacy TypeError natively; a root
  without `compId` no longer runs a structural-diff repair on every query (its
  layers are at the mirror's top level); explicit layer lists are asked by their
  composition's address (0.014 ms/call) or by their structural members (264 ms ->
  0.4 ms/call on 102 heavy layers); `hostFetch` sends the session cookie only to
  the serve host's origin.

Remaining (measured, not accepted as differences):

- A detached current scope (`PM.curComp()` not in the document) is still asked as
  `@json:` with the whole composition, and pm-wasm resets its evaluator cache for
  every such query: 62 ms per `evP` for a detached 100-layer, 40-keyed-property
  composition. Attached and nested compositions are unaffected. Making it cheap
  needs pm-wasm to keep a detached composition between queries, keyed by a
  change signal the barrier does not have for objects outside the document.
- The editor's boot window: `EngineDoc.enabled` is true before pm-wasm compiles,
  so a kernel query in that window is a named `unavailable` failure. Round 4 boots
  showed none. Gating the kernel boot on `whenReady` would delay extension UI,
  which decision F rules out; compiling synchronously is not done.
- Native 3D bounds/picking (issue D1, threed lane) still fail by name with the
  setting on.

## Out-of-ownership edits (all rounds)

Owned lane paths are `src/engine`, `renderer/src/engine-doc`, `editing.ts`, shared
stamp/reader guards, the worker interface and the document-host/write-path notes.
The following edits outside them are intentional and local:

- `shared/store-keys.ts`, `main/store-engine.ts`, `legacy/core/projects.ts`:
  history key namespace, size limit, and project-destruction cleanup.
- `kernel/install.ts`, `kernel/loader.ts`, `kernel/sandbox-host.ts`: headless
  extension boot, external contained-process sandbox transport, and isolation.
- `legacy/core/scripting.ts`: contained script runtime hook.
- `legacy/assistant/harness.ts`: live tool answer seam and headless viewport guard.
- `legacy/bootstrap.ts`: engine installation and background font warming.
- `engine.vite.config.ts`, `e2e/document-host-contract.spec.ts`,
  `e2e/document-host-boot.spec.ts`: runtime bundle and integration gates.
- `main/engine-fonts.ts` and its tests, `preload/index.ts`,
  `shared/engine-fonts-ipc.ts`: synchronous full-face font channel (fonts/bridge).
- `server/runs.ts` and its tests: project-window preference for workspace state.
- `pm-text/src/subset.rs`, `pm-text/src/registry/coretext.rs`,
  `pm-bridge/src/fonts.rs`: whole face service, including collections.
- `pm-eval/src/bounds.rs`: detached input support from the first round;
  `pm-eval/src/comp.rs`: nested and ad-hoc composition addressing.
- `pm-wasm/src/engine.rs` and `tests/queries.rs`: detached query inputs, nested
  composition scopes and replacement-time native queries; round 5 removes the
  `{legacy}` reply (`engine.rs`, `edit.rs`, `tests/edit.rs`).
- `pm-edit/src/pose.rs`, `pm-edit/src/lib.rs`, `pm-edit/tests/edit_parity.rs`,
  `pm-edit/tests/vectors/generate-edit-vectors.ts` (comments only), and
  `docs/engine/pm-edit.md` (round 5): the self-space reparent port.
- `engine/main.ts` is owned; `serve.integration.test.ts` gains the origin test.
- `docs/engine/rfcs/15-document-host.md`: frozen version-1 interface description;
  `docs/engine/pm-eval.md`: composition-scope API notes.

Idle/close policy belongs to the agents lane: workers remain alive until explicit
close or shutdown; no competing eviction policy was added here.
