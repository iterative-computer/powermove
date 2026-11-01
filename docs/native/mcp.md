# MCP and automation core (Phase 1)

Crate: `apps/native/crates/mcp` (`pm-mcp`); binary: `powermove-mcp`.
Status: **67 tools / 67 schema-identical / 21 standalone bodies + 10 host-backed adapters / 36 pending**.
`catalog::IMPLEMENTED` contains 31 bodies/adapters; `HOST_BACKED` identifies the
ten that require an injected engine/export worker, native Store transport or
explicit shipped extension source root. The standalone binary
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

“Implemented” means a concrete portable body; “Adapter” requires the named
application capability. Adapter limitations and native binding instructions are
listed below. The standalone binary does not inject these capabilities. Every
row's input schema, description and name match v1.2.0.

| Name | Schema identical | Body | Backing / remaining adapter |
|---|---|---|---|
| `list_projects` | Yes | Implemented | Kernel shell Recent / native library |
| `create_project` | Yes | Implemented | Kernel shell Open(New), Save; synthetic empty project |
| `open_project` | Yes | Implemented | Kernel shell Open(Library), recovery and locks |
| `start_session` | Yes | Implemented | Connection state, pinned document identity, isolated workspace |
| `publish_extensions` | Yes | Pending | V8/Svelte compiler exists; missing complete API pack, source scanner, isolated activation validation and atomic source promotion/reload coordinator |
| `import_media_to_timeline` | Yes | Pending | ImportQueue/MediaCatalog exist; missing durable original-byte admission, retained versions, media Undo and coordinated timeline placement |
| `finish_session` | Yes | Implemented | Actor-bound kernel mark/squash/restore + journal publication |
| `orchestrator_capabilities` | Yes | Pending | pm-agents provider protocols exist; missing root-run/provider/model catalog and application orchestration manager |
| `delegate_task` | Yes | Pending | Missing root-owned task graph, inherited authority/approvals, provider selection, durable child scope and delivery coordinator |
| `task_status` | Yes | Pending | Missing root-owned task records, bounded wait and terminal-result acknowledgement coordinator |
| `task_cancel` | Yes | Pending | Missing root/task ownership, descendant cancellation and provider child draining coordinator |
| `task_send` | Yes | Pending | Missing task ownership, live steering and interrupt/resume coordinator preserving task/workspace identity |
| `thread_launch` | Yes | Pending | Missing ordinary conversation creation/run host and inherited project/planning authority |
| `thread_list` | Yes | Pending | pm-agents conversation persistence exists; missing authorized project-scoped ordinary-thread inventory |
| `thread_read` | Yes | Pending | Missing authorized project-scoped ordinary-thread read/context adapter |
| `thread_send` | Yes | Pending | Missing ordinary-thread steering/continuation and model/effort selection coordinator |
| `thread_watch` | Yes | Pending | Missing run-specific watch ownership and automatic result delivery/resume coordinator |
| `thread_unwatch` | Yes | Pending | Missing root-owned watch lifetime/release coordinator |
| `thread_cancel` | Yes | Pending | Missing authorized ordinary-thread current-run cancellation/draining host |
| `publish_visual_reply` | Yes | Pending | Missing ordinary-thread artifact attachment/visual-reply publication host; an artifact file alone is insufficient |
| `get_timeline` | Yes | Implemented | Kernel snapshot, transport time, query evaluation; paged strips |
| `edit_layer_strips` | Yes | Implemented | Kernel timing/layer commands + atomic id-keyed operations |
| `manage_project` | Yes | Implemented | Kernel shell library operations; active-project rename via CompCommand |
| `save_project` | Yes | Implemented | Generation-bound shell SaveAs / Save; PMV4 writer and locks |
| `export_video` | Yes | Adapter | ExportChild video worker; frozen options, immutable snapshot and verified output path |
| `get_3d_scene` | Yes | Implemented | Kernel allProps/evaluation/compositionScene queries; frozen v1.2.0 result projection |
| `edit_3d` | Yes | Pending | `pm-kernel::Scene3dCommand::plan` still returns NotImplemented; missing complete frozen scene command/validation adapter |
| `fork_builtin_extension` | Yes | Adapter | Native `BuiltinSources`: explicit shipped root, frozen manifest validation, inert run staging and pristine merge base |
| `get_project_state` | Yes | Implemented | Kernel snapshot, selection, query evaluation; paged digest |
| `list_media` | Yes | Implemented | Kernel asset/folder/reference digest; optional MediaRegistry reports real runtime availability |
| `replace_media` | Yes | Pending | MediaCatalog/ImportQueue are primitives; missing verified byte replacement, retained runtime versions and native media Undo admission |
| `import_media` | Yes | Pending | Missing durable original-byte storage/fingerprint/probe admission and coordinated kernel/media Undo; AddAsset alone is metadata |
| `manage_media` | Yes | Implemented | Guarded kernel metadata edits and native Undo; never deletes stored/source bytes |
| `select_layers` | Yes | Implemented | Kernel SelectionCall; ephemeral selection publication |
| `inspect_creative_workspace` | Yes | Pending | Missing native After Effects preference/layout and script/CEP/plugin inventory reader; no equivalent in pm-services |
| `set_panel_layout` | Yes | Pending | Kernel workspace facts exist; missing workspace edit service and external workspace Undo/Redo continuations |
| `get_panel_layout` | Yes | Implemented | Kernel project/named workspace snapshot + winning panel registrations; tagged digest |
| `inspect_creative_extension` | Yes | Pending | Missing opaque creative-tool inventory and bounded no-symlink source/include reader |
| `open_panel` | Yes | Pending | Missing registered macOS/remote-DOM shell panel-open/focus host |
| `get_panel_state` | Yes | Pending | Missing fresh semantic panel/control snapshot and registration/document lifetime refs |
| `interact_panel` | Yes | Pending | Missing live control refs, observed input dispatch and post-action semantic snapshot host |
| `capture_panel` | Yes | Pending | Missing registered visible-panel target validation and real encoded pixels from macOS |
| `computer_use_panel` | Yes | Pending | Missing observed panel pixel bounds, macOS input/capture and lifetime validation |
| `get_workspace_state` | Yes | Pending | Kernel identity/selection exist; missing viewport, projects-screen state, renderer diagnostics and full extension health |
| `render_frames` | Yes | Adapter | EngineHook snapshot capture; encoded images and revision/document fence; host absent by default |
| `apply_commands` | Yes | Implemented | Frozen JSON adapter → typed kernel commands → guarded Transact |
| `edit_video` | Yes | Implemented | All nine frozen operations; source bounds, static-timing guards, exact key rebasing, locks and rollback |
| `rollback_changes` | Yes | Implemented | Document/revision guard + validated kernel Restore + journal publication |
| `validate_effect` | Yes | Implemented | Pure Rust frozen `kernel/glsl.ts` registration validator port; does not register or compile shaders |
| `stage_fork_rebase` | Yes | Pending | Fork copying works; missing installed-fork provenance/base inventory and three-way staged rebase/compile validation coordinator |
| `probe_media` | Yes | Adapter | EngineHook original resolver and native probe; frozen Rust result adapter |
| `sample_media_frames` | Yes | Adapter | EngineHook decoding/scan; kernel Legacy sourceTime samples; frozen time/quality/result adapter |
| `media_contact_sheet` | Yes | Adapter | EngineHook labelled sheet assembler; frozen source/composition time and grid planner |
| `media_waveform` | Yes | Adapter | EngineHook measured audio/image worker; frozen windows, silence/loudness and time mapping |
| `transcribe_media` | Yes | Pending | No native speech model/session/job/cache service or original PCM staging binding; promedia Decoder is an injected trait |
| `check_project` | Yes | Implemented | Kernel active/worldOpacity/sourceTime queries; optional media status and measured peaks |
| `store_search` | Yes | Adapter | Rust listing projection → `NativeStoreReads` → actual `pm-services::store::StoreTransport` GET `/v1/store/extensions` |
| `store_extension` | Yes | Adapter | Rust detail/release-history projection → native Store transport GET `/v1/store/x/{handle}/{slug}` |
| `store_source` | Yes | Adapter | Rust release-ID tree/file adapter → native Store transport GET release/tree and coordinate `/r/{version}/file/{path}` |
| `store_library` | Yes | Pending | Missing full installed manifest/health/trust/provenance snapshot, local-tree hashing and update-check binding |
| `store_install` | Yes | Pending | Store verification/install primitives exist; missing durable single-writer StoreKernel and permission/trust/runtime activation binding |
| `store_update` | Yes | Pending | Missing durable StoreKernel, installed provenance/local-overlay checks and permission-aware verified activation binding |
| `store_uninstall` | Yes | Pending | Archive-preserving store-host uninstall exists; missing durable StoreKernel and extension registry/runtime refresh binding |
| `store_publish_prepare` | Yes | Pending | Missing publisher identity/provenance, local source-tree collection, permission/secret scanner and publish-plan coordinator |
| `store_publish` | Yes | Pending | Missing authenticated publisher, prepared-plan revalidation and native publish confirmation/receipt host |
| `generate_captions` | Yes | Pending | pm-captions is pure cue/text data; missing speech/model job host and kernel caption command admission (still NotImplemented) |
| `export_captions` | Yes | Implemented | ExportChild caption method; default pure Rust SRT/WebVTT exporter |

## Wave 2 kernel/engine audit

Wave 2 adds twelve bodies/adapters: `list_media`, `manage_media`, `edit_video`,
`get_3d_scene`, `check_project`, `export_captions`, and the six original engine/export
adapters in the table above. Project/library/timeline reads, selection and existing edits continue
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
macOS hosts exist. Store writes, creative inventory, agents and extension activation still need
their respective host coordinators. Store reads and inert built-in fork staging
now have concrete adapters; generic registry inventory remains insufficient for
the other contracts.

Acceptance integration: **do not use `edit_video` as W033.unavailable**.
`render_frames` has an adapter but remains unavailable without EngineHook;
`get_workspace_state` remains pending in every standalone configuration. Other
stable pending choices are `import_media`, `replace_media`, `edit_3d`,
`generate_captions`, `capture_panel`, `computer_use_panel` and the Store/agent
families. This lane does not edit pm-acceptance.

## Wave 3 backing audit and native bindings

This wave adds **five tools: one portable body and four host adapters**. It does
**not** complete every tool body. All 67 schemas remain frozen; 36 bodies remain
pending with exact missing backing in the table above. Existing service crates
provide primitives, not every end-to-end MCP guarantee:

* `kernel/src/commands/captions.rs` and `scene3d.rs` still return
  `CommandError::NotImplemented`. `pm-captions` supplies normalization, editing
  and subtitle codecs, not a speech model/service or kernel command admission.
* `services/src/media.rs::ImportQueue` requires a host `MediaHook::probe/admit`.
  `promedia/src/media.rs::MediaCatalog` retains original/proxy relationships;
  `promedia/src/codec.rs::{Decoder,Encoder}` are injected traits. These do not
  yet copy/admit durable imported bytes and media Undo together.
* `store-host/src/install.rs::StoreKernel` requires durable single-writer
  `load/compare_exchange`; the implementation in that module is `MemoryKernel`.
  Verification/archive/install primitives cannot substitute for the missing
  kernel/permission/activation/provenance/publish coordinator.
* `runtime-v8::{Realm,SvelteCompiler}` provide owner-worker execution and
  compilation. Compat extraction supplies the realm facade, not complete native
  panel hosts, API/source staging, scanner or atomic promotion/reload admission.
* `engine-service/src/resources.rs::ResourceSnapshot::validate` explicitly
  rejects nonempty extension/font resources. Headless rendering/export exists,
  but that does not certify encoded MCP captures with arbitrary active extension
  producers/project fonts. Existing six EngineHook/ExportChild adapters remain
  explicit application integration requirements.
* `pm-agents` implements provider protocols, runtime/approval and conversation
  primitives. The frozen root-task graph, ordinary-thread coordinator, watches,
  provider catalog and automatic delivery are not an MCP-hosted service yet.

### Concrete host bindings within the MCP-owned path

`apps/native/crates/mcp/backends` is package **pm-mcp-backends**, with an owned
standalone workspace/lockfile. It depends on the actual `pm-services` and
`pm-promedia` crates; it is not another kernel writer. Keeping its dependency
integration here avoids modifying the integration lane's shared native
Cargo.toml/Cargo.lock. App hosts must add this package to their dependencies and
update that shared lockfile when integrating it. The standalone `powermove-mcp`
binary does not discover an HTTP client, app resource root or loaded catalog.

On the dedicated MCP worker:

```rust,ignore
pm_mcp_backends::bind_store(&mut host, store_origin, native_store_transport)?;
host.extension_sources = Some(std::sync::Arc::new(
    pm_mcp_backends::extensions::BuiltinSources::new(shipped_resources_root)?
));
// Share the actual catalog; retire/rebind it on project close/reopen.
let registry = std::sync::Arc::new(pm_mcp_backends::NativeMediaRegistry::default());
registry.bind(project_id, shared_native_media_catalog)?;
host.media_registry = Some(registry);
```

`NativeStoreReads` validates origins using the actual native StoreClient,
forwards bounded reads to its StoreTransport and strips diagnostic error chains.
Rust maps catalog listings, full detail/release histories and release-ID-addressed
source trees/files into exactly the tagged result fields. Source file reads
resolve release coordinates first; local overlays are never used as release
source. URL path segments/query values are encoded, traversal refused, redirects
and streaming caps stay with the native transport, and responses are capped again
at admission. Read-only calls do not refresh an agent's composition edit baseline.
Both app and inspection sessions work under the frozen allowlists.

`NativeMediaRegistry` reads real catalog availability and exact rational source
duration for list/state/check/video planning. Registering metadata alone does not
make an asset online. It does not invent measured peaks, loading/cloud states,
decoders or a working original where only a proxy exists. Original-only export
and media-source resolution still belong to their existing engine host adapters.

`BuiltinSources` copies the explicitly supplied **immutable shipped source
root** into the current run, checks the frozen v1–3 manifest contract, preserves
unknown manifest fields, rewrites fork lineage/replaces and retains the pristine
`.forked-from` tree. It does not activate or compile anything. A destination is
reserved exclusively; the rewritten manifest is written last after source/base
copies. Failed/partial staging is retained without a published manifest; the
normal session archive preserves it on finish/disconnect. This deliberately
replaces the tag's failure-time temporary-directory deletion (N4). Copies enforce
the existing 400-file/8-MiB source budget and a bounded traversal depth. Shipped
resources must remain immutable for the operation; staging must be worker-owned.

`validate_effect` ports the frozen `kernel/glsl.ts` admission rules and ordered
errors into Rust. The source-paired tests cover missing/duplicate params, flags,
integer pass bounds and UTF-16 shader-length limits. These are authored ECMAScript contract semantics, not Chromium rendering
quirks. End-anchor tests confirm both the tagged and native validators refuse
trailing line terminators in identifiers. Validation has no registration, shader
compilation or composition mutation side effect. Kernel/runtime registration can
reuse this admission function when its host is integrated.

### Wave 3 decisions

| Decision | Picking rule |
|---|---|
| Keep five new bodies/adapters behind the existing frozen tool names and allowlists | N6 contract unchanged; N7 app and editor clients remain drivable. |
| Use actual services transport/catalog bindings; leave absent hosts explicit | N1/N3 forbid success-shaped metadata or fake runtime availability; N8 keeps I/O off main. |
| Project-bind Store reads and forks without refreshing edit observations | N3/N4 preserve revision/document fences and foreign work. |
| Copy forks into inert run staging with exclusive reservation, manifest-last publication and retained partials | N4 archive, never delete; N6 retains source/lineage/result behavior without requiring JS activation. |
| Preserve authored ECMAScript validation/string-length semantics in a native port | N6 and N3; no browser-specific rendering behavior is copied. |
| Keep the new concrete bindings in an owned nested workspace until app integration | Ownership boundary and N8; no shared lockfile mutation or Node/Chromium runtime dependency. |
| Keep the remaining 36 tools unavailable with exact missing backing | N1/N3/N4/N6: primitive presence does not prove durable admission, complete legacy behavior or actual pixels. |

No visual/UX change or new visual placeholder is introduced. There are no
`DESIGN-TODO(opus)` additions in this lane. Existing visual output remains
provisional for Opus review.

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
7. Finish archives extension staging under `mcp-archives/<runId>/extensions`;
   false finish/disconnect archives the whole run under
   `mcp-archives/<runId>/workspace`. Committed artifacts/API/input files stay
   in the original run directory. No authored workspace file is deleted.
   Finish errors also end the run, preserving the document and ordinary Undo
   history; failed archival leaves the source files in place.

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
| Archive retired run/staging files instead of deleting them | N4 protects authored artifacts even on cancel, disconnect and failed finish; N6 keeps response schemas unchanged. |
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
* The native effect admission port is now available and source-paired; the
  transcription/caption job host remains required. Generic JSON registration
  is not a validator or a speech service.

### Shell, extensions and agent runtime

* Adapt registered panels/workspaces to SurfaceHook and to the frozen result
  shapes: fresh semantic refs, stable element IDs, viewport/layout, bounded
  errors, captures and observed input. Raw UiState is insufficient for these
  tools. Connect capture to real pixels and commands to real controls.
* Shared windowed/headless extension activation, deterministic virtual time,
  V8/compat runtimes, complete API pack, permissions, staging/compiler/scanning
  and atomic promotion/reload. User MCP and scripting need their real sandbox.
* Bind pinned provider clients and approvals to root/child orchestration,
  cancellation/draining, resume, notifications and thread lifecycle. Store writes
  and creative inventory still require their application adapters. No
  provider login or OAuth state is read or changed by this crate.

## Wave 3 validation

**62 MCP tests pass** (5 unit, 24 existing conformance, 22 wave-2 and 11 wave-3).
**273 kernel tests pass, 1 optional test ignored**. **10 native backend tests
pass** (5 source/manifest/fork/archive tests and 5 actual service transport/catalog
binding tests). Package formatting and strict all-target Clippy pass for pm-mcp
and pm-mcp-backends. Both required Git tags must be fetched (`v1.2.0`, `v1.3.0`)
for the existing kernel/MCP source-oracle tests; a first combined run stopped
because this fresh checkout lacked v1.3.0, then the full run passed after fetching
it. No kernel source change was required.

`service-pairs.mjs` executes actual tagged effect-validator and Store gateway
bodies. `fork-pairs.mjs` executes actual tagged manifest/fork bodies against
isolated synthetic resources. Expected results never come from native output.
Tests cover app/editor access, an unchanged foreign-edit baseline, explicit
missing hosts, Store errors and byte caps, encoded source routes, manifest
v1–3 admission, pristine bytes, no overwrite, inert publication and archival on
both commit and cancel. All fixtures are synthetic; no live Store request,
provider login, installed app/profile scan or user project was used.

```sh
# From apps/native
cargo test --locked -p pm-mcp -p pm-kernel
cargo fmt -p pm-mcp -- --check
cargo clippy --locked -p pm-mcp --all-targets --no-deps -- -D warnings
cargo test --locked --manifest-path crates/mcp/backends/Cargo.toml
cargo fmt --manifest-path crates/mcp/backends/Cargo.toml -- --check
cargo clippy --locked --manifest-path crates/mcp/backends/Cargo.toml --all-targets --no-deps -- -D warnings
```

macOS still needs application dependency/lockfile integration, native transport
and shipped-resource injection, catalog retirement on close/reopen, worker-only
invocation, bundle/symlink/permission behavior, and real Headless/media/export and
remote-DOM surface verification listed below. Linux tests do not certify those
integrations. No AppKit/Metal/WebKit code was introduced or exercised here.

## Wave 2 validation (historical)

Wave 2 verification: **51 MCP tests pass** (5 unit + 24 existing integration +
22 new integration tests); **267 kernel tests pass, 1 optional test ignored**.
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
