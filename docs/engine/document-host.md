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
   document against what the session last had and sends the patches
   (`session.patch`), awaiting the host's sequence number. Remote patches from other
   members arrive through `onPatch`; the worker sends its own unsent edits first,
   then applies them without touching its history (as `remote-sync.ts` does).
3. **Concurrent calls.** Tool calls run concurrently inside a worker (the harness
   keeps one transaction per run and checks base revisions); lifecycle requests run
   in order. Each answer is matched to its call by run and call id.
4. **Extensions register before the call returns.** `registerExtension` activates
   through the kernel's own loader and records the extension in the kernel's store.
   `definitionsGeneration` counts registrations in this instance; pushing definition
   snapshots to `pm-node` for `render_frames` is the agents lane's (contract 10).
5. **Crash and backoff.** A worker that exits rejects its pending calls with
   `worker-crashed` (retry); the next call starts it again after 1 s, doubling to
   30 s, up to five attempts, as the desktop's engine supervisor does.
6. **The production consistency check** runs after every changing call: the mirror's
   hash must equal the document's.

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

## Interfaces to other lanes

- **Fonts (ink).** `FontProvider.whole?(blob, index)` (engine-doc/fonts.ts): when the
  fonts lane's service provides whole faces, the loader loads every face whole and
  declares outlines to pm-wasm (`pm_fonts_outlines(1)`), and ink questions (text
  bounds, picks over text, raster placement, font-anchor capture) answer natively.
  pm-bridge's `FontRegistry` has no such method yet.
- **3D (host, 3d).** pm-wasm's `RefuseHost` and the `3d` pick fallback are the threed
  lane's (`pm-geom3d` in pm-wasm). Nothing here routes around them.
- **Store extensions.** The kernel's sandbox runtime needs a browser frame; a worker
  refuses a Store extension with `sandbox-unavailable` rather than running it
  uncontained.
- **Agents (hidden MCP host).** Builds on `DocumentHostPoolApi`, implements
  `DocumentHostServices` in the CLI process, routes DOM, job, and host-services tools
  itself (`DOM_TOOLS`, `JOB_TOOLS` mirror `pm_protocol::document_host`), and owns
  export and save routing (`server/runs.ts`).

## Verification

| Check | Command | Result (2026-10-18, M4 Max) |
|---|---|---|
| Pool against real workers | `vitest run src/engine/document-host/pool.test.ts` | 7 tests: two projects in two processes with interleaved calls, answers attributed by run and call; rollback on one never touches the other; restart (new pool, same store) resumes 3 and 1 undo entries; SIGKILL'd worker restarts and resumes; remote session edits applied without undo entries; missing pm-wasm and missing font service fail by name with retry and recover when the module appears; flag off keeps today's evaluator; export_video refused as `wrong-route` |
| Worker without a browser | `vitest run src/engine/document-host/hosts.test.ts` | 5 tests: `generate_captions` through host services; no-model answer without a service; 2D and text edits in `audit` mode with **0 fallbacks, 0 dual mismatches, 0 auditor failures**; the measured ink fallback (below); local extension registration and Store refusal |
| Contract sequence | `vitest run src/engine/document-host/contract.test.ts` | 15 calls answered, two workers byte-identical |
| Desktop vs worker | `playwright test e2e/document-host-contract.spec.ts --workers=1` | see the lane report |
| Exact answers | `vitest run src/renderer/src/engine-doc/strict.test.ts` | detached inputs native with 0 dual mismatches; trap answered by a fresh instance; double trap named; unloadable module named and recovered by `retry()`; development modes kept out of production overrides |

## What remains

- **Font-anchor capture headlessly needs glyph ink.** A size, font, weight,
  tracking, or box edit on a text layer records its ink bounds
  (`font-anchor.ts captureFontAnchor`). With subsets the kernel answers `{ink}`, the
  legacy edit runs, and the headless legacy raster has no pixels, so the worker
  records no `fontAnchorBounds` where the desktop records them: a document
  difference for those edits until `FontProvider.whole` exists
  (`hosts.test.ts` pins it as `edit.captureFontAnchor:ink: 1`).
- **The editor's boot window and first font question** still answer with the legacy
  code (`unavailable` before `whenReady`, `fonts` while main serves a face for the
  first time). Closing them needs the renderer's bootstrap to await the mirror and the
  project's fonts before the editor opens a project (legacy/bootstrap.ts, not this
  lane's file).
- **`context`** (no `compId`, or `PM.curComp()` not registered in `comps`) still
  answers with the legacy code; it occurs only mid-replacement.
- **Store extensions in workers** need a contained runtime that is not an iframe.
- **3D** questions (`host`, `3d`) wait for the threed lane.
- **Export and save** from workers (survey C23) and the hidden MCP host's tool
  routing are the agents lane's on this interface.
