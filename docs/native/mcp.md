# Native MCP

## Current state (2026-11-05)

The app and the `powermove-mcp` binary share one tool path: `McpSession` over
`Automation`. `Automation` either owns a kernel (headless binary) or attaches to
the running app through `pm_mcp::app_host::Host` (`Automation::attached`). The
app's `Connection` only adds its session fence. Error strings therefore match
between the two hosts. `ActorAutomation` remains for acceptance scenarios and has
no production caller.

`app/src/mcp_services.rs` holds the production hosts:

- `export_video` and `render_frames` run the same `--engine-job` child as the
  Export panel. Exports land in `~/Powermove`, or `<profile>/exports` for an
  explicit `--profile`. Effects need `pm-glsl-compiler` beside the executable;
  `package.py --glsl-compiler` puts it there.
- `probe_media`, `sample_media_frames`, `media_contact_sheet` and
  `media_waveform` read originals through `pm-media`. Layer targets refuse until
  evaluated clip timing exists; pass `assetId`. The `informative` frame scan
  refuses. Loudness is peak level only.
- `import_media`, `replace_media` and `import_media_to_timeline` retain originals
  through `Archive::retain_path` and admit them with `Call::AdmitMedia`, so they
  use normal media Undo. Images aren't converted and videos get no playback proxy.
  3D models import but aren't placed; use `edit_3d`. `list_media` reports
  availability from the kernel's media table.
- `thread_list` and `thread_read` read the durable thread archive. `activity` and
  `questions` stay empty, because a live session isn't reachable from an MCP worker.

Of the 68 tools, 36 have bodies in the app. The other 32 refuse with
`not_available_in_this_build` and a reason. `mcp_transport.rs` holds this table to
the running binary in both directions.

<!-- refused-tools -->
| Tools | What is missing |
|---|---|
| `capture_panel`, `computer_use_panel`, `get_panel_state`, `get_workspace_state`, `interact_panel`, `open_panel`, `set_panel_layout` | A bridge from MCP workers to the main-thread shell and v4 surfaces. |
| `generate_captions`, `transcribe_media` | A local transcription runtime. |
| `publish_extensions`, `stage_fork_rebase`, `fork_builtin_extension` | Extension activation; the app has no compat realm. Also a shipped source root for forks. |
| `publish_visual_reply` | A safe viewer. `pm_agents::visual` validates input. |
| `store_search`, `store_extension`, `store_source` | A production `StoreTransport` and origin. `NativeStoreReads` is ready in `pm-mcp-backends`. |
| `store_install`, `store_library`, `store_publish`, `store_publish_prepare`, `store_uninstall`, `store_update` | The signed Store envelope and install host. |
| `orchestrator_capabilities`, `delegate_task`, `task_status`, `task_send`, `task_cancel`, `thread_launch`, `thread_send`, `thread_cancel`, `thread_watch`, `thread_unwatch` | A production `Threads` and orchestration host over `ProviderRuns`. |
<!-- refused-tools -->

The sections below are older dated notes. Where they differ, this section wins.

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
media Undo. Caption edit and 3D command
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

The frozen command vocabulary contains `add_captions` and `edit_captions`,
whose complete legacy adapters are pending. `transform_layers` compiles to
primitive edits in the kernel over the batch's selection and playhead; text
geometry needs the app's fonts, so a headless kernel refuses it. Unsupported
variants return explicit unavailable errors before applying any batch. The frozen `availableOperations` vocabulary is not
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

## 2026-11-04 — MCP admission, error content and memory contract

This dated section supersedes the earlier statement that MCP errors serialize
`{code,message,backing}` into a text block. `McpSession::dispatch` still returns
those structured Rust diagnostics; **MCP text blocks now contain the error's
message verbatim**, as the tagged `mcp-stdio.mjs` does. Existing Rust diagnostic
assertions use `dispatch`; actual `call`/JSON-RPC tests assert the wire content.
JSON-RPC also treats null tool arguments as `{}`, matching the tag's `?? {}`;
arrays/nonobjects still receive the protocol-level invalid-arguments response.

`scripts/error-pairs.mjs` executes the actual tagged external-session class and
bridge admission method. Its committed fixture records **209 refusals**, covering
all 67 tool names across closed/no-session/planning/app access and competing
argument errors, plus session parser cases and unknown tools. The oracle uses
Zod **4.6.5** only when regenerating fixtures; ordinary Rust tests do not require
npm/Zod. Generation refuses unexpected TypeError/ReferenceError failures instead
of recording broken seams as expected tool errors:

```sh
PM_ORACLE_MODULE_ROOT=/absolute/disposable-npm-prefix \
  node apps/native/crates/mcp/scripts/error-pairs.mjs --write
```

External session admission now precedes ordinary body/schema validation. The
publish and direct-timeline-import exceptions retain their tagged gate order;
publish payload parsing runs before the explicit unavailable-promotion error.
The native session/publish parsers reproduce Zod issue ordering, paths and message
format. Zod's string limits count Unicode code points in this pinned version;
the tagged command JSON limit separately counts UTF-16 units. These are distinct
contracts, exercised by Unicode cases.

`apply_commands` cleans **the entire batch** before checking revisions or resolving
command targets. Unsupported types, invalid JSON, malformed keyframe/section
payloads and oversized commands report the tagged first-invalid-index message.
Ignored fields are removed before the cleaned-object size limit; default start
time and hand-edit protection are explicit; section prototype keys are removed.
The typed scratch planner and single guarded kernel transaction remain the write
authority. Source-paired tests include a malformed later command competing with
an earlier missing target and a foreign revision, with no partial publication.
`select_layers` now uses the actual body validation/message and deliberate
ID-deduplication behavior. Extra body fields do not change selection. The frozen
catalog's `uniqueItems` declaration remains unchanged; tagged `Set` normalization
is authored logic, not a Chromium quirk.

### A511/A512: resolved attribution, retained cutover requirements

The frozen reference is `v1.2.0`, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`. Executing its complete external +
agent catalog gives **67 tools and no `project_memory`**. Its `get_project_state`
description and renderer projection contain no project-memory index, even when
the supplied synthetic project has memory fields. `scripts/memory-contract.mjs`
and the Rust regression pin these facts against the actual tag; they do not
infer them from native output.

**Contract reconciliation:** W1 A511/A512's attributed *shipped-tag* provenance is
incorrect. Retain both IDs and their full memory-index/read-write requirements
as **additive native cutover requirements**, separately from frozen 67-tool
conformance. Neither requirement is deleted, marked passed, nor silently
implemented by adding a tool to the frozen catalog. The additive memory schema,
authority/agent-access policy, durable kernel admission and read/write/index
adapter still need an explicit contract and implementation. In particular,
preserving an opaque `memory` field is not a working `project_memory` tool.
The acceptance/W1 owners must append this attribution correction and retain
both cases as blocked on that additive implementation; those files are outside
this lane's ownership. The existing acceptance blocker therefore remains visible.

### Real PMV ingress for A506 consumers

`Automation::open_file(id, absolute_working_path)` exposes the kernel's
`PrepareOpen(File)`/adoption path without changing any frozen tool schema. It
retains kernel file locks, recovery/copy refusals, session generation and the
resident original-media table. It performs no metadata-only media import.
Call this on the MCP worker with an explicitly selected working copy before
starting a session. Recovery/copy outcomes remain errors requiring the host's
existing decision flow; they are never silently accepted as writable originals.

A synthetic real PCM WAV is packed into a PMV, copied to a working file, opened
through this public ingress, edited and saved through MCP, then reopened with the
production reader and again through public ingress. The test verifies every WAV
byte, native persisted Undo history, opaque memory/unknown fields and unchanged
original PMV bytes. This supplies the previously missing MCP-owned ingress for
A506's acceptance fixture. It does not certify every real-project media case,
decoding, migration or audible playback.

Decisions and selecting rules:

- **N6/N7:** error messages cross MCP unchanged; structured diagnostics stay on
  Rust `Result`. Session/access failures win over body validation where the tag
  specifies that order.
- **N3/N4/N6:** clean complete batches before admission, then use one atomic
  kernel transaction; malformed later commands never publish earlier ones.
- **N3/N4/N6:** retain A511/A512 as additive requirements with corrected
  provenance, and preserve opaque memory data without inventing a shipped API.
- **N4/N7:** expose explicit working-file ingress through kernel sessions,
  retaining original bytes and history rather than creating another packer.

## 2026-11-04 — Native creative inspection and remaining backing

**Current status, superseding the historical opening/table: 67 frozen tools /
67 schema-identical / 23 concrete bodies + 10 existing host adapters /
34 pending bodies.** The two new concrete bodies are
`inspect_creative_workspace` and `inspect_creative_extension`. The 34 pending
calls remain explicit failures; the new tests do not convert their absence into
body-conformance passes.

The worker-only `CreativeInspection` trait binds `creative::CreativeReader`, which reads actual saved After Effects preference/layout,
script-panel, CEP and compiled-plugin inventories. It ports the frozen bounded
Adobe property-map parser, normalized bounds/tab groups/visibility/windows,
newest numeric preference version, current/named selection, modified-layout
precedence and parse warnings. It does not guess a missing workspace or execute
Adobe code. Tool IDs are opaque SHA-256-derived identities; responses omit local
inventory roots. Script sources follow only bounded literal local includes;
CEP file lists apply the frozen source allowlist, traversal/private-file rules,
file/depth/count limits and explicit per-file reads. Compiled/protected sources
remain metadata-only. All response copy comes from the tagged implementation.

On macOS the default reader uses the tagged platform locations. Linux's default
returns the tagged unsupported-platform result/error. Portable tests inject
`CreativePaths { home, applications, system_cep }` rooted in canonicalized
synthetic temporary directories. Kernel documents and edit observations are not
refreshed by these main-host read tools. Planning sessions retain the frozen
inspection allowlist (which excludes these tools); project-access and app
sessions can use them.

Inventory enumeration and reads use descriptor-relative `openat`,
`O_NOFOLLOW` and `fstatat`, including every ancestor component, without `/proc`
or GNU paths. Final file reads are capped again after opening and are nonblocking
for special files. Authored source is never executed, copied into an extension,
modified or removed. In addition to the tagged per-tool limits, a single
inventory directory exceeding 65,536 entries returns a visible budget error.
Source-tree ordering uses native ICU root Unicode collation rather than ASCII
sorting or an inherited process locale; cross-locale Electron ordering is not
claimed. Tests compare the source listings directly to the tagged reader under
the recorded oracle locale, including uppercase names and Unicode content.

`libc` supplies portable Darwin/Linux descriptor APIs; `icu_collator` supplies
native Unicode collation. The root native lockfile adds only the MCP dependency
edges and four required collation packages, with no version upgrades. The owned
backend lockfile is also reconciled with the real dependency graph (including
its previously absent pm-promedia→pm-protocol edges); no dependency was bulk-upgraded.
No new crate/workspace member, Node runtime, Chromium dependency or visual
surface was introduced. **No DESIGN-TODO(opus) was added:** no design choice or
provisional UI was made.

### Remaining 34 tool bodies — not completed by this lane

| Tools | Required backing still absent on this base |
| --- | --- |
| `orchestrator_capabilities`, `delegate_task`, `task_status`, `task_cancel`, `task_send`, `thread_launch`, `thread_send`, `thread_watch`, `thread_unwatch`, `thread_cancel`, `publish_visual_reply` | Root-owned provider/model/task/thread/watch/visual-attachment coordinator, authority, durable lifecycle, cancellation/draining and delivery. pm-agents provider/conversation primitives alone are insufficient. |
| `publish_extensions`, `stage_fork_rebase` | Complete API pack, provenance/base inventory, compiler/scanner/isolated activation admission and atomic promotion/reload/rebase coordinator. |
| `import_media_to_timeline`, `import_media`, `replace_media` | Durable original-byte/fingerprint/probe admission, retained runtime versions and native media Undo coordinated with the kernel. File-open ingress does not implement media import. |
| `edit_3d` | Complete scene edit/validation admission: `kernel/src/commands/scene3d.rs` still returns `NotImplemented`. |
| `set_panel_layout`, `open_panel`, `get_panel_state`, `interact_panel`, `capture_panel`, `computer_use_panel`, `get_workspace_state` | Workspace editing + external history continuations; registered live native/remote-DOM panel/input/capture/semantic-reference and renderer/workspace diagnostics hosts. Saved layout facts are insufficient. |
| `transcribe_media`, `generate_captions` | Speech model/download/job/cache/original-PCM service and caption command admission; `kernel/src/commands/captions.rs` still returns `NotImplemented`. |
| `store_library`, `store_install`, `store_update`, `store_uninstall`, `store_publish_prepare`, `store_publish` | Durable StoreKernel + permission/trust/provenance/local-tree/activation/publish-confirmation coordinator. `store-host/src/install.rs` supplies an in-memory StoreKernel test implementation and primitives, not this production coordinator. |

`fixtures/unavailable-wave4.json` and its regression exercise all 34 names with
schema-valid requests, require inspectable unavailable errors, and verify no
project or history changes. **These are 34 missing-capability refusal cases,
not 34 implemented tool bodies.** No optimistic flags, placeholder jobs, metadata
imports, fake pixel outputs or success-shaped catalogs replace the missing
services. Existing ten worker adapters also still require real application
injection. Broader per-tool error/body identity beyond the source-paired cases
remains an integration gate.

Decisions and selecting rules:

- **N3/N6/N7:** use the actual bounded native filesystem reader and frozen result
  adapter, opaque IDs and explicit roots, rather than a catalog-only claim.
- **N1/N3/N4:** refuse linked ancestors and unbounded directories even where the
  tag's pathname reader could follow them; such filesystem exposure is not
  authored creative intent. Preserve all source/configuration bytes. Digest truncation does not split a Unicode scalar: a
  boundary-crossing emoji is omitted from the bounded label rather than producing
  the tag’s lone surrogate; the original XML/source remains byte-identical. This
  JS string-slicing artifact is not authored naming intent (N3 quirk policy).
- **N7/N8:** use deterministic native Unicode collation and worker-only I/O;
  Node executes tagged code only in tests, never in the application.
- **N1/N3/N4/N6:** keep each absent coordinator/body visible. The overall request
  to complete every remaining MCP tool is **BLOCKED**, not merge-ready completion.

## 2026-11-05 — Owner-pinned main contract, registration and recovery

This section supersedes the two preceding v1.2.0 checkpoint sections wherever
scope or current status differs. The behavior reference is now exactly
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, following the owner's reference change
and P0 decision. It is **68 tools**, including `project_memory`; the v1.2.0
67-tool fixture/oracle remains executable backward-compatibility evidence.
The earlier W1 A511/A512 discrepancy is resolved by provenance: the tag has no
memory tool, pinned main does, and the latter is now required. It is no longer
an optional future API. Existing rows above are historical, not current claims.

Current body status: **23 concrete bodies, 10 host adapters, one partially
implemented memory tool, 34 remaining unavailable bodies**. The earlier table
of 34 missing coordinators still applies. `project_memory` adds a 35th
schema-valid missing-capability refusal case for its valid write operation;
this case is not counted as an implemented write. There is no full lane
completion or Mac/application conformance claim.

### Main behavior and native backing

- The main catalog and 213 admission/parser error cases are independently
  generated from immutable main sources. Existing command, layout, media,
  creative, Store and effect comparisons now execute that same reference.
  Historical tag fixtures remain separately available. Tool errors retain the
  source message and validation precedence; Rust errors retain diagnostics.
- `memory.rs` ports the actual main sanitizer, six kinds, UTF-16 clipping,
  single-goal replacement, 100-entry eviction, update/remove validation, sorted
  list and compact index. Main's `get_project_state` now includes that index.
  Disabled memory is retained in the project but hidden from agent indexes,
  lists and staged input snapshots, including nested compositions. The pure
  mutation planner returns displaced entries for archival; it does not publish
  them. Tests execute main memory functions with synthetic entries.
- **Memory add/update/remove remain blocked.** Kernel document publication
  always increments edit revision; there is no actor admission that durably
  autosaves metadata without composition Undo/revision. Using a raw edit or
  MCP side store would violate authored behavior and the single writer rule.
  Required outside-owned change: a kernel actor memory-metadata admission,
  durable archival of displaced notes, autosave/event delivery, and overlay of
  current metadata during all composition rollback paths. MCP's rollback
  overlay preserves current `memory` and `memoryEnabled`, including explicit
  true versus absence, but cannot supply the missing durable write API.
- `registration.rs` exposes read-only status/review and explicit application of
  a prepared change for all nine main clients: Claude Code, Codex, Claude
  Desktop, Gemini CLI, Cursor, VS Code, Windsurf, LM Studio and OpenCode. Seven
  JSON variants and Codex TOML have native writers; Claude Code uses an injected
  CLI trait with source-paired argument/result checks. Missing CLI backing is
  a visible error. Native `powermove-native` bundle paths are recognized alongside
  legacy `Powermove` paths. No Node launcher or automatic startup setup is added.
- **Claude Desktop connect requires `RegistrationConsent::ask` approval.** An
  absent or denying consent host returns before settings, archives, directories
  or receipts are written. Inspection and constructors cannot register a client.
  Review is bound to its issuer and exact prior bytes; changes during approval
  are refused. A private receipt preserves intentional disconnect. All tests
  operate on canonicalized synthetic roots; no person's settings were changed.
  Unrelated settings/comments and hand-written entries are retained/refused as
  in main. Every displaced settings file is retained in a private archive.
  Descriptor-relative no-follow access and atomic exchange/no-replace protect
  publication; a late conflict preserves both versions and reports an explicit
  `configuration_conflict`, including uncertain publication outcome.
- `ExportJobs` supplies resumable `export_video` waiting with main's optional
  jobId-only resume, running response/progress, duplicate-export refusal and
  single consumption. Immutable kernel snapshots and an injected `ExportChild`
  back each job; waits and progress callbacks release the shared host mutex.
  Workers survive the requesting session ending, and document identities fence
  later polling. Native progress is omitted when the host supplies none.
  A deadline does not claim cancellation: still-running workers remain tracked
  and `drain` visibly refuses while they remain. Tests use explicitly labeled
  synthetic worker bytes, not fake video evidence. Production encode, progress,
  cancellation/reaping and stage-timing reports still require the real host.
- Replacing a document retires its portable MCP run while retaining/archiving
  its staged sources; the same connection can start a new run. No old snapshot
  is restored into the replacement. The stdio bridge rereads the descriptor and
  credentials on the next explicit request after a broken connection, creates
  a new server session, and never replays the failed mutation. Connection
  teardown runs on error and preserves the original failure over cleanup errors.
- `validate_effect` now follows main's typed inputs, generated-uniform
  collisions, contextual defaults, sampling, temporal/source/grid/path/LUT
  declarations, revision history and action validation, with direct source
  comparisons. It remains validation, not compilation or activation proof.

### A01–A10 ownership and remaining acceptance

All ten rows are in scope; none is dropped because its application owner is
outside this lane. The following is a dependency ledger, not completion by
catalog/schema correspondence.

| Main delta | MCP result and remaining dependency |
| --- | --- |
| A01 | Provider presets/discovery/address-scoped credentials and live error/context behavior still require agents/services/application owners; no MCP capability flag claims them. |
| A02 | Native review/consent/configuration/receipt/archive APIs and paired tests added. App Settings binding, actual CLI execution, installed signed executable, restart behavior and user-visible consent remain. |
| A03 | Claude Auto classifier/authority requires the real provider runner owner; not synthesized in MCP. |
| A04 | Markdown, safe local/remote media grants, work grouping and duration/steering require agent-panel/media-host integration. |
| A05 | `publish_visual_reply` remains unavailable without durable thread/artifact coordinator and inert viewer. Existing artifacts must be retained. |
| A06 | Working-file ingress, body/error/source comparisons and existing adapters advanced; the earlier 34-body dependency table remains open. |
| A07 | Current catalog, reads, disabled visibility, index, pure mutation/archival plan and rollback overlay implemented. Durable non-Undo metadata admission and actor binding remain blocked. |
| A08 | Rebase/publication/theme/workspace lifecycle requires extension/app coordinator and native DOM integration; saved facts do not substitute for it. |
| A09 | Portable document retirement, reconnect without replay, original-error teardown and export identity fencing tested. App publication/authorization queue and panel-generation capture/input fencing remain. |
| A10 | Original-error teardown tested. Real provider frame review, capture error/edit preservation, supervised command approval and complete event ordering still need provider/application integration. |

**Application integration boundary:** the GPUI app owns a separate
`apps/native/crates/app/src/mcp.rs` dispatcher on the real kernel actor. It does
not instantiate the portable `McpSession`. Shared catalog/validation/view changes
are reusable, but new portable dispatch bodies, `ExportJobs`, registration and
memory admission must be bound there by its owner. Do not replace the app actor
with a second `Automation` kernel. Both host adapters and that binding require
actual application acceptance; Linux standalone tests alone cannot prove them.

**S070 update consumed from the final native-engine merge.** Opus has now supplied
[the consent and Other Agent Apps spec](design/surfaces/agent-auth.md#s070-subscriptions-and-external-clients).
The earlier design dependency is resolved; there is no remaining missing-design
placeholder in this lane. The app owner must still bind its specified alert and
Settings button to `RegistrationConsent`, persist `externalClients.claudeDesktop`
(`added`/`declined`/`skipped`) before any client-setting write, and implement the
specified installed/legacy/customized/intentional-removal skip rules. The native
registration API is explicit-action-only and does not implement that first-window
presentation or per-profile prompt scheduler. No provisional visual surface or
new copy/design choice was created in this lane.

Decisions and selecting rules:

- **N3/N6:** use owner-pinned main for current behavior and retain tag evidence
  for backwards compatibility; do not rewrite history to invent a tagged tool.
- **N3/N4/N7:** expose memory reads/plans but refuse writes until durable
  non-Undo single-writer admission exists; archive displaced notes/settings.
- **Owner consent decision, N3/N4:** never add Claude Desktop settings without
  explicit approval, preserve intentional removal and custom configuration.
- **N1/N3/N4:** protect native settings publication against links/races and keep
  uncertain outcomes visible; preserve every displaced version, never delete it.
- **N3/N7/N8:** native worker jobs/reconnect use immutable identities, no
  automatic mutation replay, no host-lock waits, no Node/Chromium runtime.
- **N3/N4:** hide disabled notes from staged agent snapshots and preserve current
  metadata through composition rollback; a raw snapshot leak/metadata rollback
  is not recorded authored intent and is not copied as an Electron quirk.
- **N1/N3/N6:** all absent bodies/providers/coordinators stay blockers, even with
  passing catalog, planner or source-comparison tests. Prune no old consumers.

Mac acceptance still required: APFS atomic exchange/exclusion and archive
permissions; real profile/system-root links and default Adobe roots; signed
bundle launch and all external clients (including CLI and declined/approved
Claude consent); app actor binding, real export/video/progress/cancel/reap and
provider/panel/extension flows. None was exercised in this Linux container.

### Final merged Linux validation — 2026-11-05

Integration base merged without conflicts:
`d7a0b01d430d7ec9977f52d784f047c027801b14`. Both native lockfiles are committed;
all final Cargo builds/tests use `--locked`. No Mac target was built or run.
Commands below run from the repository root after sourcing the activation script.

| Command | Final result |
| --- | --- |
| `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-mcp` | 126 passed, zero failed/ignored, normal parallel harness. |
| `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-mcp -p pm-kernel -- --test-threads=1` | 399 passed (126 MCP + 273 kernel), zero failed, one pre-existing optional real-profile smoke test ignored. |
| `cargo test --locked --manifest-path apps/native/crates/mcp/backends/Cargo.toml` | 10 passed, zero failed/ignored. |
| `cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-mcp --all-targets -- -D warnings` | Clean. |
| `cargo clippy --locked --manifest-path apps/native/crates/mcp/backends/Cargo.toml --all-targets -- -D warnings` | Clean. |
| `cargo fmt --manifest-path apps/native/Cargo.toml -p pm-mcp -- --check` | Clean. |
| `cargo fmt --manifest-path apps/native/crates/mcp/backends/Cargo.toml -- --check` | Clean. |
| `node apps/native/crates/mcp/scripts/main-oracle.mjs` | 68 current tool names/descriptions/schemas identical. |
| `node apps/native/crates/mcp/scripts/tag-oracle.mjs` | 67 historical tool names/descriptions/schemas identical. |
| `PM_ORACLE_MODULE_ROOT=work/oracle node apps/native/crates/mcp/scripts/main-error-pairs.mjs` | 213 current source gate/parser refusals unchanged. |
| `PM_ORACLE_MODULE_ROOT=work/oracle node apps/native/crates/mcp/scripts/error-pairs.mjs` | 209 historical source gate/parser refusals unchanged. |

The last two optional fixture-regeneration checks require a disposable npm
prefix with the source's Zod 4.6.5; committed fixtures suffice for normal Rust
regressions. Node 24's TypeScript stripping runs source oracles in tests only.
No test invokes a real external-client registration or live provider.

Earlier validation exposed an untouched kernel event-order race in the default
parallel combined run:
`kernel/tests/slice2.rs::workspace_mutations_keep_partial_writes_without_document_undo_and_emit_events`
received a non-`Published` event at its first-event assertion. Its exact retry
passed; the final full combined run uses a serial test harness. The kernel test
and its assertion were not changed, skipped or widened. This remains a kernel
owner follow-up, not evidence that the race has been fixed. The sole existing
ignored test is `smoke_real_store_copy_counts_only`, which needs an explicitly
provided copy of a real profile; none was supplied. No new test was ignored.
Registration's new late-publication race regression also initially exposed a
misclassified project-revision error; it was fixed to `configuration_conflict`,
with the byte-preservation assertion retained. Final export tests additionally
prove worker completion/drain while a host progress callback is blocked.

**Finish status: BLOCKED.** Passing tests cover the implemented behaviors and
visible refusals, not the 34 missing tool bodies, memory publication, A01–A10
application/provider dependencies, S070 answer persistence/presentation or Mac
acceptance. The branch finish signal must therefore be `BLOCKED`, never a false
`MERGE-READY` completion signal.

## 2026-11-05 — actual native Claude Code registration worker

`registration_cli::NativeClaudeCli` implements the existing `ClaudeCli` hook with
an actual subprocess. The Settings worker supplies the discovered native binary,
working directory and explicit user configuration environment, then assigns it
to `Registration::claude_cli`. It runs `claude mcp` through literal argv, with no
shell, no Node launcher and no auth copying. Stdout/stderr drain without blocking
each other, each is capped at 1 MiB, and the deadline cannot exceed main's 60 s.
Timeout/overflow terminates the owned process group and reaps the direct child;
cleanup cannot replace the original diagnostic. The final diagnostic line uses
main's trim policy and a 240-UTF-16-unit bound without splitting a Unicode scalar
(N3/quirk policy: do not reproduce an unpaired surrogate).

`Registration::apply` now reads the actual Claude Code config after the CLI
returns success. Connect requires the owned entry with this exact native
executable; disconnect requires its absence. An exit-zero CLI that published
nothing is an error, never `changed: true` (N2/N3/N7). Original config bytes are
archived before remove/add; a failed add leaves the displaced connection's
archive intact (N4). Existing caller review, stale-file checks and handwritten
entry preservation remain. Claude Desktop still requires explicit
`RegistrationConsent::ask`; this adapter performs no desktop auto-registration.

Four new tests use an executable synthetic subprocess: real connect/readback/
disconnect with literal space/semicolon paths and retained foreign config;
no-write success refusal; simultaneous full pipes with original-error retention;
and bounded stalled/flooding peers. The existing 11 registration tests still
compare the nine-client source contract and consent/config/archive behavior.
These do not establish installed Claude Code or Settings Accounts UI behavior.

The Settings host must bind the native adapter and existing review/apply APIs to
its command registry and consent UI. That registry/native shell is outside this
lane's owned files. Actual installed-client restart, both signed architectures,
configuration environment selection and descendant drainage remain Mac checks.
No A02 or other main-delta row is marked fully closed by this component alone.
