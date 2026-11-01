# MCP and automation core (Phase 1)

Crate: `apps/native/crates/mcp` (`pm-mcp`); binary: `powermove-mcp`.
Status: **67 tools / 67 schema-identical / 13 implemented / 54 pending**.
This is the standalone automation core, with an asynchronous adapter to the
application's kernel actor. It does not yet provide the complete agent runtime,
extension activation, rendering or surface automation of the windowed app.

## Public contract and evidence

`fixtures/catalog-v1.2.0.json` freezes names, descriptions, input schemas,
inspection/app allowlists, edit-command field lists and easing presets from tag
`v1.2.0`. No tool is renamed, omitted or given a success-shaped placeholder.
Pending calls return an MCP tool error with `code`, `message` and `backing`.
Unknown tools and malformed arguments return `invalid_arguments`.
Schema identity does not mean that every tool body is implemented.

* `scripts/tag-oracle.mjs` executes actual tagged TypeScript to verify the
  catalog. Node is a **test-only** dependency, not part of the native binary.
* Seven stored request/response pairs in `transport-installed.json` came from
  the packaged app's stdio proxy connected to a synthetic, read-only loopback
  host. They cover initialization, duplicate initialization, ping, malformed
  calls, unknown methods and tool listing. The catalog response came from the
  frozen source; **no live project-tool body was captured**. The stored evidence
  says `projectToolsCaptured: false`. These are transport evidence, not 67
  behavioral conformance proofs. Do not run the capture script against real data.
* `source-pairs.mjs` executes tagged project-state/timeline bodies using
  synthetic services; `command-pairs.mjs` executes six tagged command bodies.
  Their expected outputs never come from native code. The latter imports the
  tagged `LAYER_FIELDS` constant as well as the bodies that depend on it.
* Rust tests cover guarded edits, atomic batches, selection, history grouping,
  rollback, interleaving, disconnect, library durability, file locks, session
  generations and the real stdio-to-loopback binary path. Fixtures use temporary
  profiles only. Broader frozen-editor project traces remain an integration gate.

## Tool conformance table

“Implemented” means a concrete standalone P1 body; adapter limitations are
listed below. Every row's input schema, description and name match v1.2.0.

| Name | Schema identical | Body | Backing / remaining adapter |
|---|---|---|---|
| `list_projects` | Yes | Implemented | Kernel shell Recent / native library |
| `create_project` | Yes | Implemented | Kernel shell Open(New), Save; synthetic empty project |
| `open_project` | Yes | Implemented | Kernel shell Open(Library), recovery and locks |
| `start_session` | Yes | Implemented | Connection state, pinned document identity, isolated workspace |
| `publish_extensions` | Yes | Pending | Extension runtime/compiler, API pack, isolated sandbox and promotion |
| `import_media_to_timeline` | Yes | Pending | Durable media import/version/Undo service |
| `finish_session` | Yes | Implemented | DocumentStore mark/squash/restore + kernel journal drain |
| `orchestrator_capabilities` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `delegate_task` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `task_status` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `task_cancel` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `task_send` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_launch` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_list` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_read` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_send` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_watch` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_unwatch` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `thread_cancel` | Yes | Pending | Agent protocols, approvals, child/thread lifecycle |
| `publish_visual_reply` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_timeline` | Yes | Implemented | Kernel snapshot, transport time, query evaluation; paged strips |
| `edit_layer_strips` | Yes | Implemented | Kernel timing/layer commands + atomic id-keyed operations |
| `manage_project` | Yes | Implemented | Kernel shell library operations; active-project rename via CompCommand |
| `save_project` | Yes | Implemented | Generation-bound shell SaveAs / Save; PMV4 writer and locks |
| `export_video` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `get_3d_scene` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `edit_3d` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `fork_builtin_extension` | Yes | Pending | Extension runtime/compiler, API pack, isolated sandbox and promotion |
| `get_project_state` | Yes | Implemented | Kernel snapshot, selection, query evaluation; paged digest |
| `list_media` | Yes | Pending | Durable media import/version/Undo service |
| `replace_media` | Yes | Pending | Durable media import/version/Undo service |
| `import_media` | Yes | Pending | Durable media import/version/Undo service |
| `manage_media` | Yes | Pending | Durable media import/version/Undo service |
| `select_layers` | Yes | Implemented | DocumentStore selection (standalone host) |
| `inspect_creative_workspace` | Yes | Pending | Creative-app inventory and bounded source reader |
| `set_panel_layout` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_panel_layout` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `inspect_creative_extension` | Yes | Pending | Creative-app inventory and bounded source reader |
| `open_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_panel_state` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `interact_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `capture_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `computer_use_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_workspace_state` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `render_frames` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `apply_commands` | Yes | Implemented | Frozen JSON adapter → typed kernel commands → guarded Transact |
| `edit_video` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `rollback_changes` | Yes | Implemented | Revision guard + validated DocumentStore restore + journal drain |
| `validate_effect` | Yes | Pending | Extension registration validator |
| `stage_fork_rebase` | Yes | Pending | Extension runtime/compiler, API pack, isolated sandbox and promotion |
| `probe_media` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `sample_media_frames` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `media_contact_sheet` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `media_waveform` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `transcribe_media` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `check_project` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `store_search` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_extension` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_source` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_library` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_install` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_update` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_uninstall` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_publish_prepare` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_publish` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `generate_captions` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `export_captions` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |

## Implemented adapter limits

`apply_commands` resolves IDs, names, `$selection` and one-based references in a
scratch store, in order, then applies the complete typed batch once. Planning
failure changes nothing. Kernel lock, hand-edit and validation policy remains
in force. `edit_layer_strips` supports explicit-target move, duplicate, sequence,
align, reorder, state, remove and close-gap operations, with fit/group checks
and one atomic batch.
Effect/transition commands require real registered JSON definitions; no fake
registry entry or callable default is invented. The host reads the kernel's
transport playhead instead of keeping a separate time store.

The frozen command vocabulary contains `transform_layers`, `add_captions` and
`edit_captions`, whose complete legacy adapters are pending. Caption commands
in the kernel are currently declarations returning NotImplemented. Transform
nudge/align/distribute exist, but full transform adapter/center-anchor/font
support does not. These command variants return explicit unavailable errors
before applying any batch. The frozen `availableOperations` vocabulary is not
a claim that these variants work. Some engine-dependent layer/property operations
can also be refused by kernel validation. Full legacy edit-command conformance
is still required; the six source-paired bodies are not evidence for all variants.

`manage_project` lists, creates, opens, saves, duplicates, trashes and restores
through the native library/session API. Rename is implemented for the active
project only; unopened-slot rename still needs a kernel library operation.
Trash requires a closed project, as enforced by the kernel. Library operations
are immediate commit boundaries, outside composition snapshot rollback.

`save_project` uses the current generation-bound session. First export uses
shell SaveAs in the explicit host exports directory; later saves use shell Save.
The kernel owns locking, PMV4 media packing, associated-file metadata, library
checkpoint and journal rebasing. Recovery-copy/fallback results are surfaced
as errors with their kernel details, never represented as a successful save to
the original file. The asynchronous actor adapter also exposes Save/SaveAs and
returns the full ShellReply so app callers can present recovery-copy decisions.

## Session model

1. `Automation::new` requires absolute, explicitly supplied native profile and
   export paths. It never discovers the real profile. Kernel AttachLibrary uses
   the rollback-readable V1_2 disk format and its normal locks/copy policy.
2. A connection owns one `McpSession`. Project access binds the project ID,
   **document-open identity** and observed revision; closing/reopening the same
   ID cannot make an old run valid again. App-context sessions manage the
   library; editor access is restricted by the frozen inspection allowlist.
3. Project-access runs get a private working directory containing `extensions`,
   `api`, `artifacts` and `inputs`. The input JSON is a run-start snapshot;
   `api/mcp-tools.json` is the frozen tool catalog. The complete extension API
   pack, user MCP servers, executable scripting and extension baseline are not
   available yet. Staging is inert. No connection credential appears in results.
4. Before the first composition mutation the session captures a snapshot and a
   kernel history mark. Each batch supplies the observed revision. A state or
   timeline read refreshes that observation. If newer work arrived after this
   run edited, the run becomes interleaved; reading never makes foreign work
   eligible for rollback or grouping. No-op batches do not mark the run changed.
5. `finish_session(commit:true)` groups an uninterrupted run using the kernel's
   `squash_since`. Interleaved/folded runs keep normal history entries. History
   taps are drained through Kernel::handle before acknowledgement, using the
   kernel journal rather than a second history store.
6. `rollback_changes`, a false finish and disconnect attempt guarded snapshot
   restore. The document identity and revision must still match, no foreign
   edits may have been observed, and no kernel entry may be open. Restore uses
   validated `DocumentStore::restore_project`, adds an undoable restore entry
   and drains its journal taps. Refusal preserves all work. Disconnect logs a
   refusal; the client is already gone. Undo can recover a successful rollback.
7. Finish removes extension staging; false finish/disconnect removes the run
   workspace. Committed artifacts/API/input files remain. Finish errors also
   end the run, preserving the document and its ordinary Undo history.

Applied acknowledgements are retained when the kernel reports a notice such as
NotDurable. Mutation results include the notice and subscribers/stderr receive
it, so a journal warning cannot hide an applied edit from session rollback.
State is never claimed crash-safe when the kernel reports it is not durable.

## Transport and headless automation

The binary serves newline-delimited JSON-RPC over stdio. It supports initialize,
ping, tools/list and tools/call; input lines are capped at 2 MiB. `--listen`
adds an authenticated IPv4-loopback bridge. Its random credential lives only
in a mode-0600 `mcp/connection.json` under the explicit profile, inside a private
directory; descriptor creation refuses another owner. Shutdown closes/joins
clients, attempts their rollbacks and removes only its own descriptor.
`--powermove-mcp` runs the stdio proxy against that descriptor. It never starts
or attaches to an installed editor implicitly.

Example with a **new disposable profile**, after building with the pinned
workspace toolchain:

```sh
powermove-mcp --profile /absolute/disposable-native-profile --exports /absolute/disposable-exports
```

The standalone `Automation` serializes its sole Kernel owner behind the worker
host mutex. It must never be called from a UI thread that waits on that mutex.
`ActorAutomation` instead reads immutable Published snapshots, awaits guarded
Transact and shell save calls, and subscribes to KernelEvents. It does not open
a second writer. Full McpSession/transport binding to the application's actor
is pending the actor operations below.

`SurfaceHook` specifies panel listing, semantic inspection, stable-ID drive and
capture. `EngineHook` specifies revision-bound frame capture. Both adapters are
explicitly absent by default; no empty screenshot or fabricated panel succeeds.
Their presence does not automatically implement the frozen panel/render tools.
Surface IDs must be stable and shared between windowed and headless activation,
with credentials and other sensitive controls omitted.

## Decisions

| Decision | Reason |
|---|---|
| Keep all 67 frozen public schemas, including pending tools | Existing clients can discover the same contract; unsupported bodies fail explicitly. |
| Reuse kernel transactions, history, persistence and sessions | One document/history authority; no MCP-owned save or undo format. |
| Guard whole-run restores conservatively | A cancelled run must not remove acknowledged foreign work. |
| Bind runs to document identity as well as ID/revision | Reopening the same ID creates a different document lifetime. |
| Keep standalone ownership separate from the app actor adapter | Missing actor hooks must not lead to a second writer in the application. |
| Use session SaveAs/Save instead of direct PMV writes | File locks, destination, metadata, recovery and journal rebasing stay coordinated. |
| Keep applied notices alongside acknowledgements | A durability warning does not mean an edit failed to apply. |
| Provide inert staging and explicit host hooks | No unimplemented scripting, rendering or extension activation is advertised as functional. |

## APIs still needed

### Kernel/application actor

* **Present and used:** DocumentStore selection, history_mark/squash_since,
  restore_project; guarded Transact; slice-2 shell sessions, Save/SaveAs,
  Recent/Duplicate/Trash/Restore; immutable state, UI facts and registries.
* **Missing actor messages:** selection mutation/publication; history mark and
  guarded squash; validated snapshot restore with expected revision and an
  open-entry check. Store-only methods cannot be used through KernelHandle.
  The standalone host's journal drain is an interim integration boundary, not
  an actor API. Once these messages exist, bind McpSession to the app actor.
* An unopened-library-slot rename operation and application binding of the
  current generation-bound session. Preserve recovery/copy decisions in UI.
* Complete caption/transform/3D/video command implementations and legacy
  result adapters; full provenance/undo support for retained media versions.

### Engine and media

* `pm-engine-service` already provides Headless frame rendering and export
  infrastructure. MCP still needs its adapter: revision-bound snapshots, fonts,
  active extension definitions, media resolution, capture dimensions/times,
  encoded PNG/JPEG MCP image blocks, and cancellable export jobs/results.
  The current core has no engine-service dependency and no render tool body.
* Durable media catalog/import/replace with asset identity, retained versions,
  missing/cloud/loading status and native Undo. Source sampling/contact sheets,
  waveform/probe/transcription and caption jobs need their service adapters.
* The actual effect registration validator; generic JSON registry storage alone
  is not the shipped validator.

### Shell, extensions and agent runtime

* Adapt registered panels/workspaces to SurfaceHook and to the frozen result
  shapes: fresh semantic refs, stable element IDs, viewport/layout, bounded
  errors, captures and observed input. Raw UiState is insufficient for these
  tools. Connect capture to real pixels and commands to real controls.
* Shared windowed/headless extension activation, deterministic virtual time,
  V8/compat runtimes, complete API pack, permissions, staging/compiler/scanning
  and atomic promotion/reload. User MCP and scripting need their real sandbox.
* Pinned provider clients, approvals, child cancellation/draining, resume,
  notifications, thread lifecycle and Store/creative inventory adapters. No
  provider login or OAuth state is read or changed by this crate.

## Validation

Verified: **25 passing tests** (1 unit + 24 integration), formatting and strict
Clippy (`--all-targets --no-deps -- -D warnings`). Recovery checks prove that
both grouped history and snapshot rollback reach the kernel journal. A forced
rotation failure verifies that a NotDurable notice keeps applied work tracked.

From `apps/native`, with toolchain `1.96.1` pinned by rust-toolchain.toml:

```sh
source ../../engine/env.sh
export CARGO_TARGET_DIR=~/Developer/pm-target
~/.cargo/bin/cargo test -p pm-mcp
~/.cargo/bin/cargo fmt -p pm-mcp -- --check
```

The conformance suite requires Node with `node:module.stripTypeScriptTypes`
(the reference scripts use Node 26) and the local `v1.2.0` tag. Runtime code
requires neither Node nor Electron. Do not use a real user profile/project for
these checks or commit runtime descriptors, artifacts or captures.
