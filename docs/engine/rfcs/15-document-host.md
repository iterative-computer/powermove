# RFC 15: The headless document host interface

Status: the TypeScript worker interface is frozen at version 1
(`apps/desktop/src/engine/document-host/protocol.ts`, docs/engine/document-host.md,
2026-10-18); the Rust types below are its wire description. Open question 1 is
decided: one child process per project. Module: `pm_protocol::document_host`. Gates
Phase 2 (drafted with the rest); freezes when Phase 3c starts, when the
hidden MCP host moves onto it. Owners: Renderer write path (the headless
document host and its per-project instances, section 27), with Export and
agents for the hidden MCP host and tool routing.

## Purpose

`powermove mcp` today runs the production editor in Playwright Chromium on
SwiftShader, one project per host, with no headless document host
(section 22). From Phase 3c on macOS the hidden host runs host services in
the CLI process, **one headless document host worker per open project**
(`pm-wasm`, the kernel, history, the agent harness, and the extension
registry), persisted to the profile store, a `pm-node` engine child
process, and Chromium only on demand for DOM tools. The document model
stays TypeScript (`server/mcp.ts:26-27`). This contract is the interface
between the host and those workers: instances, persistence, the extension
runtime, and agent tool routing.

It also fixes the headless document host's known defects (appendix A
item 16): history is per instance, so concurrent runs on two projects never
break each other's rollback, and captions tools and text-anchor edits work
without the editor.

## Types

| Type | Meaning |
|---|---|
| `InstanceId` | One worker for one open project |
| `OpenProject`, `ProjectOpened` | `Request::OpenProject` (host → worker pool): project id, its profile-store entry as a capability handle, the agent session pinning it. Answered with the instance, its document id, and the revision after load |
| `CloseProject` | `Request::CloseProject`, answered `ack` after a final persist |
| `Persist`, `PersistReason`, `Persisted` | `Request::Persist`: write the instance's document and history to the profile store (autosave, after a tool call, on close, at host shutdown); answered with the revision persisted and its size |
| `RegisterExtension`, `ExtensionSource`, `ExtensionRegistered` | `Request::RegisterExtension`: register or hot-reload an extension bundle (local, agent-published, or Store) in one instance; answered after its definition snapshots are in force, with the definitions generation (contract 10) |
| `ToolCall`, `ToolResult` | `Request::ToolCall`: an agent tool call routed to an instance, with the tool's existing JSON arguments; answered with its existing JSON content, an error flag, and the document revision after the call |
| `ToolRoute`, `route_tool`, `DOM_TOOLS`, `HOST_TOOLS`, `JOB_TOOLS` | Where each tool runs (below) |
| `limits` | The external MCP limits the hidden host keeps (section 20): 10 s auth, 2 MiB lines, 32 queued requests, 64 MiB responses |

## Tool routing

`route_tool(name)` decides where a call runs (tested):

| Route | Tools |
|---|---|
| Chromium on demand | DOM tools: `capture_panel`, `computer_use_panel`, `interact_panel`, `get_panel_state`, `open_panel`, `get_panel_layout`, `set_panel_layout`. Chromium joins the session's pinned project as a tab whose viewer uses CPU frames from `pm-node`; when Chromium is missing the call fails with a named error |
| Job engine | `export_video`, `get_render_status` (contract 12) |
| Host services | `save_project` (the project packer, against the media store), the agent media tools (`probe_media`, `sample_media_frames`, `media_contact_sheet`, `media_waveform`, `transcribe_media`, over our `ffmpeg` executable), the Store tools, creative-workspace inspection, extension forking, and orchestration. The same split as the desktop's `main/agent-tools/bridge.ts` |
| Document host instance | Everything else: document and 3D tools (`apply_commands`, `edit_video`, `edit_3d`, `get_project_state`, `get_3d_scene`), media import and replace (bytes through host services), captions, `render_frames` (frames from the job engine at agent priority), `check_project` |

`ToolCall` carries only document-host tools; the host dispatches the other
routes itself. `ToolCall::validate` rejects a DOM, job, or host-services
tool sent to an instance.

## Invariants

1. **One instance per open project.** Each has its own `pm-wasm` mirror,
   kernel, history, harness, and extension registry. On the hidden host
   each pinned project has its own document host; a desktop window with an
   active agent session asks before switching project (section 20).
2. **Tool schemas are unchanged.** Arguments and results are the tools'
   existing JSON; changes are additive and optional (`export_video`
   `mode`, the queued result). Tool names are snake_case, 1 to 64 bytes;
   call ids are 1 to 128 bytes.
3. **Read-your-writes across the hop.** `ToolResult.revision` is the
   document revision after the call; the next `render_frames` or
   `export_video` from that run carries it as its minimum revision
   (contracts 5, 9, 12).
4. **Extensions register before tools return.** Local and agent-published
   extensions register in the instance's kernel; Store extensions run in
   their contained sandbox. `ExtensionRegistered` is sent only after the
   registration's definition snapshots reached the engine
   (`definitions_generation`), so `publish_extensions` followed by
   `render_frames` renders the new definitions (section 22's
   `publish_extensions` plus `render_frames` case).
5. **Persistence is per instance and explicit.** A worker persists on
   autosave, after a mutating tool call, on close, and at host shutdown;
   `Persisted.revision` is the revision on disk. Closing persists first.
6. **The external MCP limits stay** (section 20): 10 s auth, 2 MiB lines,
   32 queued requests, one call at a time per connection and per stdio
   relay, 64 MiB responses.
7. **Media import goes through the host's media store**, always copied
   unless the call passes `storage: "link"`; files inside an agent
   workspace are always copied (section 21).
8. **Same documents everywhere.** Agent-tool contract tests produce
   byte-identical documents on the desktop and the hidden host (the
   Phase 3c gate).

## Versioning

Version 0 until Phase 3c starts. Adding a route or moving a tool between
routes is a version bump, because the host and the worker must agree on
which side runs it. New tools that run in the document host need no change
here: the default route is the document host.

## What Phase 0 must confirm

None of the eight spikes exercises this contract directly; it is drafted
in Phase 0 so the Renderer write path lane can build the Phase 1
headless-host switch to `pm-wasm` against stable instance and persistence
messages. Before the freeze:

- **S5 (WASM evaluator):** the per-window `pm-wasm` memory budget, which
  bounds how many instances one hidden host can keep open, and whether
  instances run as worker threads or child processes.
- **Phase 1 (headless document host on `pm-wasm`):** that the kernel,
  history, and harness run without happy-dom, and that the appendix A
  item 16 fixes hold (captions tools without `captions/install`,
  `editing.ts:927` guarded, history not cleared on project switch).
- **The tool lists against the code:** `HOST_TOOLS` mirrors
  `AGENT_MEDIA_TOOL_NAMES` (`shared/media-tools.ts`),
  `POWERMOVE_STORE_TOOL_NAMES` (`main/agent-tools/spec.ts`), and
  `ORCHESTRATION_TOOL_NAMES` (`shared/agent-orchestration.ts`), and every
  other host tool is dispatched by name in `main/agent-tools/bridge.ts`.
  `tests/tool_routing.rs` reads those TypeScript files and fails on drift,
  so a tool added to the desktop bridge cannot silently route to a document
  host. The same test checks that every DOM tool and `export_video` exist
  in `main/agent-tools/spec.ts` (`get_render_status` is new in Phase 3b).

## Open questions

1. Whether instances are worker threads in the CLI process (cheaper, shared
   memory) or child processes (crash isolation per project). The plan says
   "worker per open project"; a crash in one project's kernel should not
   take down others, which favors processes. Decided with S5's memory
   numbers.
2. Whether `Persist` should carry history depth limits (today's Undo stack
   size) or leave them to the instance. Left to the instance.
3. Whether the desktop app should also route agent tools through this
   interface (one renderer as one instance) so both hosts share one code
   path. Not required by the plan; it would make the byte-identical gate
   structural rather than tested.
4. Whether `ToolCall` needs a deadline field. Tool timeouts (`LONG_CALLS`,
   `toolTimeoutSec`) are enforced by the harness, and `Message::Cancel`
   cancels a call; a deadline field is redundant today.
5. Linux keeps today's Chromium hidden host until after cutover
   (decision 16); this contract is macOS-first and has no Linux-specific
   parts.
