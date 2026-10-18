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
   host's copy, puts its unsent edits on top, and sends them. `stats().hash` is the
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
| Evaluator mode | at least `on` | `POWERMOVE_NATIVE_EVALUATOR` (default `on`) |
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
| a non-object where a channel or path belongs | `invalid-input`: the legacy code throws reading it before any evaluation, and that throw is the API's answer |
| `error:*` | Still counted; `pm-wasm-error` named failure with the new engine on |
| `unavailable` | Workers never answer before the module is ready (open fails by name). In the editor, the boot window before `whenReady` remains (below) |
| `fonts` | Synchronous in workers (no fallback). In the editor, the first question per face still waits on main (below); a provider that stopped answering is `font-service-unavailable` |
| `ink`, `host`, `3d` | Interfaces to the fonts and threed lanes (below) |

`gl.bounds` and `gl.pick` have no legacy reference headlessly (no compositor), so the
dual answers skip them in a worker; the editor's dual answers compare them.

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
or baseline hash; a worker reports no viewport (it has no window).

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
`glyphLayout`, `gl.bounds`, `gl.pick`) are not dual-compared in a worker.

## The editor with the new engine on

`legacy/bootstrap.ts` waits (bounded at 15 s) for the mirror before the project opens,
then (bounded at 10 s) for the boot project's faces (`EngineDoc.warmFonts`, which asks
pm-wasm directly and loads what its text layers need), and warms the faces of every
project opened later. A module that does not load is the named `pm-wasm-unavailable`
failure with Retry.

## Interfaces to other lanes

- **Fonts (editor ink and new faces).** The editor's provider is asynchronous (main
  relays to the font service) and has no `whole`, so in the editor an ink question
  still answers `{ink}`, and a face first used after a project opened (a font picked
  in the inspector) is answered by the legacy code while it loads. Closing both needs
  a synchronous channel from the renderer to a registry in main
  (`ipcRenderer.sendSync`, the fonts and bridge lanes' files) and `whole` on it.
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

## What remains

- **The editor's first question about a face picked after open**, and **ink in the
  editor**, wait for the fonts lane's synchronous channel (above). Workers have neither.
- **`context`** (no `compId`, or `PM.curComp()` not registered in `comps`, mid-replacement
  or a nested composition from a composition's own `comps`) still answers with the
  legacy code; making it exact needs pm-wasm to address compositions by path.
- **`get_workspace_state`'s window fields** (`layout`, `viewport`, `openProjects`,
  `projectsScreenOpen`) describe an editor window; a worker has none. Its document
  fields match the desktop's. Whether the route table sends the tool to an open editor
  window is the orchestrator's decision.
- **Media decoding facts** a worker cannot know without engine probes: decode failures
  and cloud placeholders (`media-failed`, `media-in-cloud`), a video's audio track when
  the record does not say, and media import and replace (survey C24).
- **3D** questions (`host`, `3d`) wait for the threed lane.
- **Export and save** from workers (survey C23) and the hidden MCP host's tool
  routing are the agents lane's on this interface.
- Store extensions use Web Workers or `XMLHttpRequest` only in a browser; the
  contained runtime offers neither.
