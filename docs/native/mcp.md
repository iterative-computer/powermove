# MCP and automation core (Phase 1)

Crate: `apps/native/crates/mcp` (`pm-mcp`); binary: `powermove-mcp`.
Status: **67 tools / 67 schema-identical / 20 standalone bodies + 6 host-backed adapters / 41 pending**.
`catalog::IMPLEMENTED` contains 26 bodies/adapters; `HOST_BACKED` identifies the
six that still require an injected engine/export worker. The standalone binary
does not fabricate a renderer, decoder, export child or loaded media registry.
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
| `finish_session` | Yes | Implemented | Actor-bound kernel mark/squash/restore + journal publication |
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
| `export_video` | Yes | Adapter | ExportChild video worker; frozen options, immutable snapshot and verified output path |
| `get_3d_scene` | Yes | Implemented | Kernel allProps/evaluation/compositionScene queries; frozen v1.2.0 result projection |
| `edit_3d` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `fork_builtin_extension` | Yes | Pending | Extension runtime/compiler, API pack, isolated sandbox and promotion |
| `get_project_state` | Yes | Implemented | Kernel snapshot, selection, query evaluation; paged digest |
| `list_media` | Yes | Implemented | Kernel asset/folder/reference digest; optional MediaRegistry reports real runtime availability |
| `replace_media` | Yes | Pending | Durable media import/version/Undo service |
| `import_media` | Yes | Pending | Durable media import/version/Undo service |
| `manage_media` | Yes | Implemented | Guarded kernel metadata edits and native Undo; never deletes stored/source bytes |
| `select_layers` | Yes | Implemented | Kernel SelectionCall; ephemeral selection publication |
| `inspect_creative_workspace` | Yes | Pending | Creative-app inventory and bounded source reader |
| `set_panel_layout` | Yes | Pending | Workspace editing state and external workspace Undo/Redo callback host; settings alone are insufficient |
| `get_panel_layout` | Yes | Implemented | Kernel project/named workspace snapshot + winning panel registrations; tagged digest |
| `inspect_creative_extension` | Yes | Pending | Creative-app inventory and bounded source reader |
| `open_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_panel_state` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `interact_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `capture_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `computer_use_panel` | Yes | Pending | Shell/surface host, semantic controls, image/input or thread attachment |
| `get_workspace_state` | Yes | Pending | Kernel identity/selection/effects exist; viewport, renderer errors, projects-screen state and extension health require shell/runtime |
| `render_frames` | Yes | Adapter | EngineHook snapshot capture; encoded images and revision/document fence; host absent by default |
| `apply_commands` | Yes | Implemented | Frozen JSON adapter → typed kernel commands → guarded Transact |
| `edit_video` | Yes | Implemented | All nine frozen operations; source bounds, static-timing guards, exact key rebasing, locks and rollback |
| `rollback_changes` | Yes | Implemented | Document/revision guard + validated kernel Restore + journal publication |
| `validate_effect` | Yes | Pending | Extension registration validator |
| `stage_fork_rebase` | Yes | Pending | Extension runtime/compiler, API pack, isolated sandbox and promotion |
| `probe_media` | Yes | Adapter | EngineHook original resolver and native probe; frozen Rust result adapter |
| `sample_media_frames` | Yes | Adapter | EngineHook decoding/scan; kernel Legacy sourceTime samples; frozen time/quality/result adapter |
| `media_contact_sheet` | Yes | Adapter | EngineHook labelled sheet assembler; frozen source/composition time and grid planner |
| `media_waveform` | Yes | Adapter | EngineHook measured audio/image worker; frozen windows, silence/loudness and time mapping |
| `transcribe_media` | Yes | Pending | Engine-service adapter, media resolution and encoded results/jobs |
| `check_project` | Yes | Implemented | Kernel active/worldOpacity/sourceTime queries; optional media status and measured peaks |
| `store_search` | Yes | Pending | Remote Store catalog/query service; local registry is not the catalog |
| `store_extension` | Yes | Pending | Remote release/detail service; installed/version/source metadata lacks listing and releases |
| `store_source` | Yes | Pending | Remote release tree/file service; kernel overlay inventory is not release-ID addressed |
| `store_library` | Yes | Pending | Full extension manifests/health/trust, provenance, update checks and local-tree hashes; kernel inventory alone is insufficient |
| `store_install` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_update` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_uninstall` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_publish_prepare` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `store_publish` | Yes | Pending | Store service, permissions, verified install/publish lifecycle |
| `generate_captions` | Yes | Pending | Editor domain adapter; captions/3D/video/lint runtime or engine support |
| `export_captions` | Yes | Implemented | ExportChild caption method; default pure Rust SRT/WebVTT exporter |

## Wave 2 kernel/engine audit

Wave 2 adds twelve bodies/adapters: `list_media`, `manage_media`, `edit_video`,
`get_3d_scene`, `check_project`, `export_captions`, and the six `HOST_BACKED`
tools above. Project/library/timeline reads, selection and existing edits continue
using their existing kernel authority. All 67 public names, descriptions and
input schemas remain byte-identical to the tagged catalog.

`list_media` pages metadata, folders, recursive structured references and inherited
parent/group locks. Persisted metadata alone never becomes “available”: only a
host MediaRegistry can report a decoded runtime. `get_project_state` uses that
same optional registry for its media digest. `manage_media` implements every
frozen asset/folder operation, validates the whole operation first and publishes
one guarded id-keyed kernel transaction. Referenced deletion checks locks and
unsupported structured references before removing layers. It never removes a
source file, immutable media object or old byte version. Native Undo retains the
metadata/layer tree. Like the tag, any media action prevents run snapshot rollback
and run squash; ordinary composition rollback cannot restore media bytes.

`edit_video` supports insert, split, trim, move, speed, remove, separate_audio,
audio and slip. Scratch planning preserves complete layer metadata, masks,
effects, transitions and keyframe timing. Source-handle limits, embedded-audio
retiming refusal, animated/remapped timing refusal, inherited group locks and
protected hand edits are checked before publication. Errors apply no partial
work. The existing document identity, revision, rollback and history-group guards
also cover these edits.

`get_3d_scene` evaluates the kernel's compositionScene/allProps/evP queries. The
engine contains later scene fields (object slots and light width/height); the
frozen response projection omits those fields without changing project data.
`check_project` evaluates real active/worldOpacity queries over the tagged bounded
frame sampling schedule, with media/source bounds and measured peak checks when
a MediaRegistry is injected. It does not guess drawing coverage from geometry.

Import/replacement remain pending: kernel AddAsset records metadata but does not
verify persisted bytes, fingerprint a file, retain runtime versions or coordinate
media Undo. SetAssetFolder, full transform_layers, caption edit and 3D command
variants also remain unimplemented in the kernel. Metadata-only “import” and
partial transform success would violate N3/N4/N6. No new public tool or schema is
introduced to bypass these gaps.

`get_workspace_state` still needs the actual viewport, renderer diagnostics,
projects-screen state and complete extension-health records. `set_panel_layout`
needs workspace editing state and external Undo/Redo continuations. Panel and
computer-use tools retain precise structured unavailable errors until their
macOS hosts exist. Store/creative/agent/extension tools still need their respective
services; generic registry inventory is not a substitute for those contracts.

Acceptance integration: **do not use `edit_video` as W033.unavailable**.
`render_frames` has an adapter but remains unavailable without EngineHook;
`get_workspace_state` remains pending in every standalone configuration. Other
stable pending choices are `import_media`, `replace_media`, `edit_3d`,
`generate_captions`, `capture_panel`, `computer_use_panel` and the Store/agent
families. This lane does not edit pm-acceptance.

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
5. `finish_session(commit:true)` groups an uninterrupted run using actor-bound
   `HistoryCall::Squash`. Interleaved/folded runs keep normal history entries.
   Kernel::handle owns the journal/publication boundary before acknowledgement;
   MCP does not keep a second history store.
6. `rollback_changes`, a false finish and disconnect attempt guarded snapshot
   restore. The document identity and revision must still match, no foreign
   edits may have been observed, and no kernel entry may be open. Restore uses
   validated `Call::Restore`, adds an undoable restore entry and publishes
   through the kernel's journal boundary. Refusal preserves all work. Disconnect logs a
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
capture, but no frozen panel tool claims success from this generic hook alone.
`EngineHook` now supplies worker primitives: frozen capture, original media
resolution/probe, source decoding/scan, labelled sheet assembly and measured
waveform/loudness. Rust owns frozen result shapes, dimensions, sample/grid
planning, time-base conversion, partial-sheet omitted-time reports and validated MCP image blocks. Layer source-time samples
come from kernel queries with `retiming: "legacy"`, including animated speed and
remapping, at the tag's precision/density/bounds. Snapshot requests carry project
ID, document-open identity and revision; stale results are rejected. Source tools
and render/export reads do not refresh the agent's edit baseline.

`ExportChild` receives an immutable project and frozen VideoOptions plus the
explicit export directory. The host owns original/proxy provenance, fonts,
extension definitions, cancellation, atomic no-clobber output publication and
child draining/reaping. The adapter refuses absent/empty/outside-destination
outputs. The default video method is explicitly unavailable; its caption method
uses Rust normalization, visible-layer clipping and SRT/WebVTT escaping. Caption
text requires no subprocess or JS. These are worker interfaces, not permission
to block the UI thread or create another kernel writer.

## Decisions

| Decision | Reason |
|---|---|
| Keep frozen response adapters separate from optional native worker primitives | N6 freezes the public contract; N7/N8 permit headless tests without Node or UI-thread waits. |
| Plan full video/media edits in scratch state, then publish one guarded transaction | N3 exact authored timing; N4 no partial edits; kernel single-writer architecture. |
| Preserve all bytes/versions on metadata deletion; block run restore/squash after media actions | N4 archive, never delete; N6 native media Undo contract. |
| Refuse import/replacement and full transform/caption/3D edits until complete backing exists | N1/N3/N4/N6 forbid success with missing durable bytes, validation or authored intent. |
| Evaluate source timing and structural checks through kernel queries | N3 one evaluated truth; N7 deterministic headless inspection. |
| Keep later engine scene fields out of the frozen digest, retaining document data | N6 response compatibility and N4 data preservation. |
| Export captions as pure native text through ExportChild's default method | N3 exact visible cue text; N8 no JS/process dependency for a pure operation. |
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
* **Present since kernel slice 3:** actor-bound selection, history mark/guarded
  squash and validated snapshot restore with document identity, expected revision
  and open-entry fencing. Standalone MCP uses those messages and the kernel's
  normal journal/publication boundary. Full McpSession/transport hosting in the
  application remains an integration task.
* An unopened-library-slot rename operation and application binding of the
  current generation-bound session. Preserve recovery/copy decisions in UI.
* Complete caption/transform/3D/video command implementations and legacy
  result adapters; full provenance/undo support for retained media versions.

### Engine and media

* Bind the existing pm-engine-service Headless renderer and export-child jobs to
  EngineHook/ExportChild in the app, with original media resolution, project fonts,
  active extension definitions, encoded PNG/JPEG and child cancellation/reaping.
  The Linux standalone binary has no real engine service. Decoder/probe, measured
  loudness and sheet/scan primitives need the corresponding native media workers.
* Durable import/replace needs retained versions, verified fingerprints/storage,
  availability and native Undo; metadata AddAsset is insufficient. Source bytes
  and immutable objects must remain archived, never collected by MCP metadata edits.
* The actual extension registration validator and transcription/caption job host
  remain required; generic JSON registration is not a validator.

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

Wave 2 verification: **49 MCP tests pass** (5 unit + 24 existing integration +
20 new integration tests); **267 kernel tests pass, 1 optional test ignored**.
The required combined default test run passed; one earlier kernel selection-event
ordering failure passed its exact retry and the full rerun without kernel edits.
Package formatting and strict all-target Clippy pass with no pm-mcp warnings;
Linux still reports the existing unused `path` warning in kernel locks.rs.
Verification runs on Linux with pinned Rust 1.96.1 and Node 24.19.0.
Node is test-only. All runtime additions are Rust; no Node/Chromium dependency or
new Cargo dependency/lockfile change is introduced. See `tests/wave2.rs` for the
new source-paired cases and worker doubles, alongside the existing conformance,
transport, history, durability and library tests.

`domain-pairs.mjs` executes actual tagged media-management, video-editing, caption
normalization/export, render-time selection and structural-check bodies against
explicit synthetic services. `engine-pairs.mjs` executes tagged media orchestration
with synthetic decoder/process seams; native Rust plans/results are compared to
its outputs. Those image bytes prove transport integrity, not Metal/FFmpeg pixel
correctness. The checked-in PNG/JPEG fixture contains only synthetic flat colour.
`scene-v1.2.0.json` and `export-v1.2.0.json` are generated by tagged code and the
tag's Zod 4.6.5, never by native output. The 3D fixtures canonicalize object-key
ordering before digesting, matching pm-model's documented unordered-object
semantics; arrays retain their order. No live app/profile/project was captured.

Regenerate those two fixtures with `scripts/scene-fixtures.mjs` and a disposable
npm prefix containing Zod 4.6.5; set `PM_ORACLE_MODULE_ROOT` to that prefix.
Ordinary Rust tests consume the committed fixtures and require only Node with
`stripTypeScriptTypes`, not npm or Zod. The frozen 67-tool catalog and stored
installed transport replay remain unchanged.

Required commands, from `apps/native`:

```sh
cargo test --locked -p pm-mcp -p pm-kernel
cargo fmt -p pm-mcp -- --check
cargo clippy --locked -p pm-mcp --all-targets --no-deps -- -D warnings
```

macOS verification still required: real Headless pixels/fonts/extension producers,
source media decode/rotation/retiming/scan, labelled sheets, measured waveforms,
original-only child exports, no-clobber/cancel/reap behavior and full app-actor
hosting. Linux doubles do not certify those integrations or physical UI behavior.
