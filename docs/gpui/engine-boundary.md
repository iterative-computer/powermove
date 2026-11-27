# Native Mac engine boundary

Research snapshot: 2026-10-31, `feat/gpui-mac` at `e15c87539f`.
Legacy source: committed `main` at `d9a98ea5a7`, read through Git objects.
This is a source audit, not a build, performance measurement, or parity certification.
No Cargo builds or application runs were performed for this document.
Paths beginning `engine/` describe this branch; `desktop:` means
`main:apps/desktop/`, not the desktop directory on the current branch.
New crate and module names below are proposals, explicitly distinguished from existing APIs.

## 1. Decisions that define the boundary

1. Link the Rust renderer, model, evaluator, media, and audio into the native app.
   AppKit and GPUI share the macOS main thread; neither owns rendering-engine work.
2. One Rust document owner accepts every editor, extension, agent, and MCP mutation.
   It owns command validation, transactions, undo, selection, and dirty state.
   Engine document snapshots are shared read versions, never competing authorities.
3. Retain a synchronous Rust query evaluator beside that owner for viewer tools.
   Do not replace the old synchronous WASM calls with round trips to a worker process.
4. Use an IOSurface-backed RGB frame path, with explicit GPU completion and reuse.
   GPUI 0.3.8 needs an RGB surface integration: its shipped video surface is NV12-only.
5. Put exports and large agent renders in the same binary's `--engine-job` child.
   Reuse the existing job protocol, capability handles, budgets, and cancellation.
6. Initially retain provider integrations in an optional Node/TypeScript service child.
   No Electron anywhere in this architecture; no Node runtime in the UI process.
   Node in this service is an explicit migration choice, not an engine dependency.
7. Replace the legacy extension runtime with native v4: JavaScriptCore scripts,
   declarative native panels, and the existing GLSL definition contract.
   Svelte panel compatibility is not a requirement; agent-driven migration is.
8. Treat browser-dependent shader/raster fallbacks as unresolved parity work.
   A TypeScript service without a browser cannot supply DOM, Canvas, or WebGL pixels.

These refine [architecture.md](architecture.md), especially its process and extension sections.
The app must preserve behavior in the two GPUI parity inventories; the older engine
plan's Electron transport choices and retired features are not new product requirements.

## 2. What can be linked today

“Linkable” below means the source exposes an ordinary Rust library with no required
Node/WASM ABI. It does not assert that a new GPUI workspace has been compiled.
Native dependencies, framework linkage, and packaging still need verification later.

| Existing crates | Reuse in a native host | Important boundary |
|---|---|---|
| `pm-model`, `pm-time`, `pm-eval`, `pm-expr`, `pm-edit` | Direct Rust document, time, queries, expressions, geometry edit kernels | Not a complete application document store, command router, or history service |
| `pm-render`, `pm-gpu`, `pm-color`, `pm-glsl` | Direct wgpu/Metal composition renderer and display transform | Need assets, definitions, fonts, scheduling, and presentation integration |
| `pm-text`, `pm-vector`, `pm-geom3d`, `pm-studio` | Text/vector/3D support, largely consumed through `pm-render` | CoreText service needs main-thread setup; Studio has its own device requirements |
| `pm-media`, `pm-audio`, `pm-export`, `pm-stretch` | Decode/probe, audio playback/mix, delivery | FFmpeg/native libraries, CoreAudio, VideoToolbox, and codec packaging remain necessary |
| `pm-engine`, `pm-protocol`, `pm-host` | Sessions, request state machine, scheduling, host capabilities and budgets | Host must drive actions and provide actual services; no app UI lives here |
| `pm-node` with `node` disabled | Existing native kernels, presenter, fonts, job workers | Plain rlib despite its name; several facilities still assume bridge/host protocol plumbing |
| `pm-wasm` on native target | Existing synchronous document/query/edit facade | Plain rlib despite its name; only `abi` is wasm-target gated |
| `pm-bridge` | Native Mach/shared-memory cross-process frame bridge | Optional for native in-process viewer; not the GPUI adapter |

Evidence: each crate's `Cargo.toml` and `src/lib.rs`; especially
`engine/crates/pm-node/Cargo.toml` (optional `node` feature),
`engine/crates/pm-wasm/Cargo.toml` (plain rlib), and
`engine/crates/pm-wasm/src/lib.rs:33` (`abi` target guard).
Do not link `pm-node-addon`, `pm-bridge-addon`, or `pm-wasm-module` into the app.
Those cdylib shells are integration products for Node or WASM, not required engines.
`pm-wasm`'s native expression path must keep the existing compatibility semantics;
its WASM-only `js-pow` dependency is not permission to run all expressions in JSC.

### 2.1 Sessions and host services: real calls, not an imagined SDK

The existing low-level session API is protocol driven:

- Construct `pm_engine::Engine::new(EngineConfig::new(EngineProcess::Interactive))`.
- Call `Engine::attach(port, PeerKind::Host/Renderer/PeerEngine)` and perform the
  protocol hello exchange; “Renderer” is a peer role and need not mean a web page.
- Encode `pm_protocol::envelope::Envelope` messages and call
  `Engine::receive(port, bytes, HostMicros)`; process returned `Action`s.
- Send `Request::OpenSession(OpenSession { document, role, process, color })`;
  retain `Response::SessionOpened`'s session, then resync its document.
- Drive `Engine::tick(now)` and `Engine::pump()`; close with
  `Request::CloseSession`, then `Engine::detach(port)` on connection teardown.
- Install work providers with `Engine::set_presenter(Box<dyn FramePresenter>)`
  and `Engine::set_job_service(Box<dyn JobService>)`.

There is no public `Engine::open_pmv()` or `Engine::pick()` convenience method.
Sources: `pm-engine/src/engine.rs:438`, `:632`, `:718`, `:846`, `:2742`;
`pm-protocol/src/envelope.rs:139` and `:178`; `pm-engine/src/present.rs:103`.
Use the existing codec and envelope validation, not a second homemade message format.

`pm_node::EngineHost::{new,with_sink,attach,receive,tick,pump,engine_mut}` is a
working Rust wrapper (`pm-node/src/lib.rs:80`). `with_sink` installs asynchronous
native work and font wakeups. Its interactive presenter starts only when
`PresenterConfig::from_env` finds the current frame-bridge configuration and a sink.
Simply calling `EngineHost::new` will not make GPUI receive rendered frames.
It also applies process QoS; extracting this wrapper for a UI process needs care.

`pm_host::CliHost::{new,set_asset_resolver,serve,mint,choose_destination,end_launch}`
and `Bound: HostServices` provide useful host implementation patterns.
They enforce path capabilities, output placement, lease/hardware-session arbitration,
and clean up abandoned outputs (`pm-host/src/lib.rs:69`, `:248`; `outputs.rs`).
`CliHost` is incomplete for an editor: `ProjectStore` and `ExtensionBundle` handles
are unsupported there, and its Blender handlers are stubs.
Implement native editor services, not a blind rename of `CliHost` to “MacHost”.

### 2.2 Opening .pmv and maintaining the document

Existing Rust read path:

1. `pm_model::container::read_container(&file, ReadOptions { verify_media })`.
   `RandomAccess` supports disk access; `read_container_bytes` is the memory version.
2. Inspect `Container { format, document, media }`: JSON, PMV3, and PMV4 are supported.
   PMV4 index/document hashes are checked; media hashes can be checked on demand with
   `Container::verify_media`. Media records are absolute byte ranges, not decoded assets.
3. Keep the saved envelope, commonly `{ proj, history, ws, v }`, separately from the
   engine project. Use `canonical::project_of/into_project` and `canonical_project`.
4. Build `Document::new(Some(canonical_project), revision)` or adopt a prepared tree.
   Preserve unknown fields and media records so opening and saving does not erase data.
5. Bind embedded media to usable files/ranges, populate asset resolution, install
   project fonts and definition snapshots, then send the initial render snapshot.

Sources: `pm-model/src/container.rs:1`, `:127`, `:240`, `:478`;
`pm-model/src/canonical.rs:87`; `pm-model/src/doc.rs:598`.
The reader does not reproduce all UI import defaults, recovery, relinking, or factories.
`ingest_project` normalizes engine time data; do not assume it is the whole TS loader.

For writes, `Document::{apply,apply_at,apply_ops,apply_ops_at}` accepts `Patch`/`Op`.
`Op` includes leaf writes plus id-keyed array insert/delete/order operations.
`pm_model::ops::{diff_ops,diff_node_ops}` and `binary::{encode,decode}` already exist.
`canonicalV1` bodies carry these operations with base/revision checks.
`pm_engine::document::DocumentUpdate::{Replace,Ops,Patches}` and `apply_to` already
describe shared-tree replacement and ordered incremental updates for engine readers.
These operations are not full editor commands and do not automatically create undo.
Patch failures can leave earlier writes applied: stage mutations on a scratch clone,
validate the complete transaction, and publish only after success.

For the synchronous query mirror, reuse `pm_wasm::Engine` directly in Rust:
`new`, `resync(json, revision)`, `adopt_document(document, ConsistencyHash)`,
`apply(body, base, revision)`, `document`, `revision`, and `to_json`.
For loading/hashing off-thread, `pm_wasm::kernel::PreparedDocument::{parse,
parse_tree,from_document}` and `Kernel::{adopt,document,dispatch,dispatch_bytes}` exist.
`pm_node::kernel::NativeKernels::{new,call,call_bytes}` adds background loading,
refresh, checkpoint persistence, and owner management; `EngineHost::document_kernel*`
exposes it without requiring napi (`pm-node/src/kernel.rs:700`, `:847`).
Keep only one mutable authority: query/render copies must follow committed revisions.

### 2.3 Synchronous evaluation, pick, bounds, and text

The quickest host facade can wrap existing `pm_wasm::Engine` methods:

| Need | Existing concrete entry point |
|---|---|
| Property evaluation | `intern(&pm_model::Value) -> Option<PropId>`, `ev_id(id, seconds)`, `ev_batch(ids, seconds, out, kinds)`, `release(id)` |
| General queries | `query(json_bytes) -> Reply { text, special }` |
| Hit tests | Query `op: "pick"`, `comp`, `t`, `x`, `y`, optional `includeLocked`, `localView` |
| Geometry | Query `op: "bounds"` or `"groupBounds"`, `comp`, `layer` index, `t`; transform/world queries also exist |
| Text editor geometry | Query `op: "textLayout"` or `"textRaster"`, composition, layer, time |
| Model geometry | `set_model(id, kind, bytes)`, `forget_model`, `clear_models`; `geometry::Models` caches imported assets |
| Fonts for queries | `add_font_answers`, `add_font`, `set_font_seeds`, `set_font_outlines`, `font_report`, `clear_fonts` |
| Extension property metadata | `set_definition(&Value)` |

Implementation references: `pm-wasm/src/engine.rs:432`, `:554`, `:588`, `:647`,
`:738`, `:1546`; `pm-wasm/src/geometry.rs:60` and `reply.rs`.
The JSON facade is callable natively today; it is not a WASM-only export.
Add typed wrappers for common query results later to avoid serialization during drags.
Lower-level equivalents are `pm_eval::{Evaluator,Comp,Scope,PropAddress}`:
`PropAddress::channel`, `Scope::open`, `Scope::{channel,num,bounds,group_bounds,pick}`.
Use `pm-wasm` initially because it already supplies expressions, text, and geometry hooks.

Queries can return `fonts`, `ink`, `models`, `host`, `unsupported`, or `throw`,
not just `ok`. Handle these states explicitly; do not interpret them as “no layer”.
Load font outlines, model bytes, and editor-camera inputs before pointer interaction.
No native host may fall back to three.js or browser text measurement implicitly.
`pm-edit` already ports pose/reparent, grouping, anchor, align/distribute, split-text,
and comp-bounds kernels. `Edit::new`, its writes, and `Edit::finish() -> Vec<Op>`
are reusable inside the application's command transaction (`pm-edit/src/lib.rs`).

### 2.4 Rendering and GPU output

- `pm_gpu::Gpu::new(label)` returns a Metal-backed `device` and `queue`.
- `pm_render::Renderer::new(device, queue)` or `Renderer::headless()` creates a renderer.
- Configure `set_definitions`/`apply_definitions`, `set_mesh_definitions`,
  `set_text_fonts`, `set_program_cache_dir`, and media interpretation as needed.
- `Renderer::render(&Document, composition, Ticks, &RenderRequest, &dyn AssetResolver)`
  returns `Frame`; its `Target` is a wgpu texture, with optional depth/id and failures.
- `Renderer::render_into(..., &Target, Option<&Target>) -> Rendered` draws into a
  caller-owned working target. `Target::wrap(texture)` adapts existing textures.
- `RenderRequest::{at_scale,sized,from_frame_request}` sets size, ROI, precision,
  and preview/final behavior; do not substitute display time floats for engine ticks.
- `Renderer::recycle(frame)` retains allocation reuse. Readback APIs are for captures,
  exports, and diagnostics, not the interactive presentation path.

Sources: `pm-gpu/src/device.rs:24`; `pm-render/src/renderer.rs:404`, `:1295`, `:1364`;
`pm-render/src/request.rs`; `pm-gpu/src/target.rs:68`.
`pm_render::{MapResolver,SourcePaths,AssetResolver}` resolve asset IDs to files;
the GPUI media catalog must supply a real resolver rather than put file paths in IDs.

The engine already exports shared surfaces:
`pm_gpu::bridge_out::surface::ExportedSurface::create(device, &SurfaceRequest)`
returns `{ iosurface, texture, view, size, format }`.
Formats are `Bgra8Unorm` and `Rgba16Float`; `SharedIoSurface` is retained and Send/Sync.
`WorkingTexture::new` plus `DisplayTransform::{new,encode}` performs working-color
to display conversion into that surface (`bridge_out/display.rs:33`, `:100`, `:217`).
Use this transform once, not a raw reinterpretation of working pixels as display RGB.
`SurfacePool` supplies allocation/generation patterns; GPU lifetime is separate from
the lifetime of the retained IOSurface object.

### 2.5 Playback and audio

`pm_audio::playback::Playback::start(MixFormat, Arc<SourceCache>, PlaybackConfig)`
opens default CoreAudio output through cpal and starts the native mixer workers.
`set_project(Arc<Document>, comp_id, &dyn pm_audio::AssetResolver)` builds a mix graph
and reports unavailable sources. Rendering and audio resolvers are different traits;
one catalog should implement adapters for both.
`set_evaluator`/`set_standard_evaluator` configure expression-aware evaluation.
`enqueue_command(TransportCommand)` returns a `CommandReceipt`; observe `try_result`
away from the UI hot path. `command` waits, so do not call it from a GPUI event.
`clock_handle().latest()`, `clock()`, `meters()`, `take_errors()`, `stats()`, and
`shutdown()` expose runtime state (`pm-audio/src/playback.rs:821`, `:1068`, `:1094`).

`pm_protocol::clock::TransportCommand` includes Play, Pause, Seek, Step, loop controls,
PlayAround, and scrub controls. Reuse its rational playback rates and tick ranges.
`pm_audio::transport::Transport`, `ClockCore`, `ClockSlot`, and `ClockSample` provide
the lower layers; `pm_node::present::{audio,clock,render}` already combines them
with silent host-clock runs, warm-up, device latency, epochs, and viewer scheduling.
Extract/reuse that policy rather than ticking the playhead independently in GPUI.
`Playback::start_headless` returns the callback-facing `DeviceSide` for an owning host;
it is not an automatic real audio device and is not needed for normal Mac playback.

### 2.6 Export and media jobs

Native export calls already exist in `pm-export/src/render.rs`:
`ExportJob`, `ExportPlan`, `MediaPaths`, `plan_export`/`plan_export_for`,
`run_export`/`run_export_into`, `Progress`, `render_still`, and `render_frame`.
The caller supplies renderer, assets, frozen document, settings, cancellation,
progress/preview plumbing, audio sources, and GPU scheduling through `GpuGate`.
Low-level `pm_export::export(ExportSpec)` returns `Export = Pipeline<OutputSink>`;
`push_frame`, `push_encoder_input`, `push_audio`, `finish_with_cancel` manage delivery.
This is export encoding, not the editor's persistent .pmv writer.

Existing job facade: `pm_node::jobs::JobEngineService::{start,start_with_fonts,
start_probes_only}` implements the engine job service. Attach it with
`Engine::set_job_service`, then use `Request::{SubmitJob,CancelJobs,AgentFrames}`
and `Event::JobStatus` through `pm-protocol` (`jobs/mod.rs:458`; `envelope.rs:193`).
Reuse it in a plain Rust child with real `HostServices`; do not require a utilityProcess.

| Media task | Existing Rust entry point | Missing application work |
|---|---|---|
| Probe | `pm_media::probe::probe(&Path) -> Probed`, `probe_timing`, `MediaProbe::to_json` | Catalog insertion, identity, ownership, relink/import UX |
| Poster/thumbnail frames | `PreviewReader::{open,open_with_service,open_sequence,poster,frame_at,filmstrip}` | Resize/encode/cache/publish thumbnails and cancel stale requests |
| Video decode | `decode::session::Session::{new,presented,sample,sample_with}`; shared `DecodeService` | Per-instance scheduling, budgets, lifetime; prefer renderer's existing integration |
| Waveforms | `pm_audio::waveform::legacy_peaks`, `Waveform::compute`, `level_for` | Cache, visible-range requests, timeline presentation |
| Model import | `pm_geom3d::{model_import,gltf,obj}` and `pm_wasm::Engine::set_model` | Choose/normalize import route and store dependent resources |

Sources: `pm-media/src/probe/mod.rs:292`, `preview.rs:75`, `decode/session.rs:97`;
`pm-audio/src/waveform.rs:24`, `:118`; `pm-geom3d/src/lib.rs`.
None of these is a complete “import into project” operation.
URL ownership, iCloud materialization, file access, deduplication, image sequences,
clipboard, captions, missing-media diagnosis, and project-relative paths remain host work.

## 3. Native rendering: implemented does not mean cutover certified

Appendix B (`docs/rust-engine.md:2251`) is the required feature inventory, not a
statement that every line has shipped or passed a native-only gate.
The gate reports are dated measurements on specific commits, often inside Electron.
`docs/engine/e2e-ledger.md` is a legacy app baseline with quarantines and environment
skips; its 501-pass amendment run does not certify GPUI or even native-only rendering.
Phase 1/2 results also include write tracking, font parity, and app integration failures.

Historical evidence that must not be silently promoted to current certification:

| Evidence | What it actually establishes |
|---|---|
| `phase3a-gate.md:35` | Gate failed: 92/218 projects routed, 14 compared-frame failures, text/Blender/performance gaps; several impressive UI timings measured legacy rendering |
| `phase3a-gate.md:59` | Native 3D pick kernel met its measurement; the app still picked through three.js at that measurement |
| `phase3b-gate.md:45` | Decode/delivery conformance strong, but overall gate failed on export speed, intermittent J/K/L sync, and app defects |
| `gate-fixes.md` | Documents specific fixes and reruns, including T3/T5/T6/D1/D2/R1 and later corrections; not a blanket replacement gate |
| `quality-ledger.md`, `text-render.md`, `vector.md`, `scene3d.md`, `shaders.md` | Later class-specific implementations, deviations, and measurements; consult beside executable capability checks |

Current code is substantially beyond the Phase 3a snapshot. In particular,
`pm-render/src/capability.rs:263-465` now accepts path masks, 3D masks, level-2
content, motion blur/2.5D for supported visual types, and implemented native stamps.
Do not repeat the old gate's 92 routed projects as a current count.
Likewise, old Blender rows are historical: `pm-node/src/present/document.rs:569`
explicitly records Blender retirement and inert `render3d`; Studio is now integrated
through `pm-render/src/scene3d/studio.rs` and `pm-studio`.
Reconcile Appendix B's Blender wording with the committed product baseline before
creating work to restore an obsolete renderer.

### 3.1 Native implementations present in source

- Composition backgrounds, parenting/transforms/opacity, solo/visibility, group
  boundaries, nested precomps, retiming, blend modes, mattes, masks, and adjustments.
- Parametric/editable vector shapes, paths, SVG support, coverage/paint, and filters
  in the implemented SVG classes (`pm-render/src/vector`, `pm-vector/src`).
- Text/captions, animators, output-resolution layout, glyph outlines/bitmap paint,
  and caret geometry when fonts are configured (`pm-render/src/text`, `pm-text`).
- Built-in and supplied GLSL effects/transitions, shader and fragment/mesh layers,
  native translation, texture inputs, and per-layer failure reporting.
- 2.5D planes and shutter sampling/motion blur for the checked types
  (`pm-render/src/{space25d,motion}`); inspect those lists when adding a new type.
- Native three.js-compatible scene rendering, imported models, animation/geometry,
  lights/shadows/cameras and editor looks; Studio tracing through its integrated pass.
- Native progressive/interlaced media handling, interpreted color/timing, sequences,
  animated-image paths, tested still formats, preview and export output conversion.
- Native audio transport, mix, meters, waveform computation, and offline export.

These are implementation claims, not a promise of arbitrary-project visual equality.
`Renderer::unsupported_with_media` considers actual definitions, fonts, and media;
use it at load and relevant revision changes. Also handle runtime `RenderError`,
`LayerFailure`, shader watchdog reports, and missing-asset diagnostics.
An empty static unsupported list cannot prove every runtime resource is available.

### 3.2 Remaining TypeScript/browser-dependent paths

| Path still present | Concrete evidence | Native rewrite consequence |
|---|---|---|
| GLSL translation/device failures | `pm-render/src/capability.rs:67`, `renderer.rs:552-624`; `docs/engine/shaders.md:119-166` | Existing fallback uses a WebGL2 runner in a window and job-to-main relay; pure Node/JSC cannot execute it |
| v1–3 raster service wrappers | `pm-render/src/compat.rs:1`, `CompatRasters`, `CompatPending` | Text/captions/shapes may ask the legacy renderer for wrapper-produced pixels; replace with v4 native contracts |
| Unsupported media/SVG classes | `pm-render/src/capability.rs`, `media.rs`, `vector/svg.rs`; `docs/engine/pm-render.md` image classes | Old app can route whole compositions to TS/WebGL; native app needs native coverage or a named unsupported state until fixed |
| Font/cache-seed compatibility | `pm-wasm/src/engine.rs:647`, `pm-node/src/fonts`, `text-render.md`, `quality-ledger.md` | Reuse CoreText answers/full fonts and seed semantics; GPUI UI text is not composition text |
| Legacy host render/kernel APIs | `desktop:src/renderer/src/kernel/{api,install,host}.ts` | Rust compute exists for many calls, but UI, events, registry lifecycle, and capture routing still need a host |

The shader contract staying unchanged includes its visible output, not a guarantee
that every legal WebGL program already translates. First inventory fallback-only
programs; close native compiler gaps or implement a native offscreen GL/ANGLE backend.
That backend would be new work with its own parity and packaging proof.
Do not silently bundle Chromium as an architectural dependency to hide these gaps.
The extension compatibility exception permits replacing Svelte panels, not losing
built-in effects, valid user shaders, or ordinary project media from the parity target.

## 4. TypeScript application logic that is not already a Rust application

Counts below are approximate physical source lines on `main`, including comments
and blanks, excluding `*.test.*`, `*.spec.*`, fixtures, and declaration files.
Tracked duplicate-named sources are included where present. These are scoped slices,
not disjoint totals or estimates of effort; overlapping helpers must not be added twice.
Paths in this section use the `desktop:` prefix defined above.
Some compute kernels already exist in Rust; “port” means the remaining app behavior.

| Area / measured source slice | Approx. lines | Dependencies and recommendation |
|---|---:|---|
| Document: `state/document.svelte.ts` (56), `legacy/core/model.ts` (290); runtime `ports.ts` + `invalidate.ts` (110), `project-index.ts` (203) | 346 + 313 | Svelte raw state/counters, PM registry, factories/defaults, mutation scopes; **(a) port** to Rust core and GPUI notifications; reuse `pm-model`, not a deep reactive JSON wrapper |
| Editing: `legacy/core/editing.ts` (1,493), `compositions.ts` (720), `layer-groups.ts`, `parenting.ts`, `core/validate/commands.ts` (1,269), `core/types/commands.ts`, `core/edit-log.ts`, `captions/commands.ts` | 4,606 | PM helpers, validation, scene/caption schemas, locks, channel/keyframe semantics; **(a) port** command dispatcher and actions; call existing `pm-edit` geometry kernels |
| Undo/history: `legacy/core/history.ts` (501), `shared/history-memory.ts` (120), `shared/patch.ts` (89) | 710 | Transaction scopes, snapshots/patches, shared records, selection, byte budgets and saved dictionary; **(a) port**, including grouped drags, rollback, redo pruning, takes and persistence |
| Selection: `legacy/core/selection.ts` (66), `state/selection.svelte.ts` (10) | 76 | PM registry, temporal keys, property index; **(a) port** to stable layer/key/property IDs; also capture `model.ts::selectLayers` and panel interaction policy |
| Kernel/extensions: `renderer/src/kernel/`, runtime extension load, legacy registry, `main/extensions/`, shared extension contracts | 10,071 | Svelte mount/signals, DOM/iframe sandbox, registries/LIFO, esbuild, filesystem watcher, forks/rebase; **(a) port** registry/lifecycle; **(c) replace** legacy UI/runtime with JSC + native v4 panels |
| Codex provider: `main/codex/` | 5,273 | Node child_process/fs/readline, app-server protocol, account flow, staging/publication/repair/isolation; **(b) retain** provider logic in service; replace Electron IPC edges |
| Claude provider: `main/claude/` | 1,448 | Node process spawning, CLI streams, SDK types, shared Codex workspace/result logic; **(b) retain** in same service with an explicit provider adapter |
| Agent tools: `main/agent-tools/` | 3,562 | Socket/crypto/fs, Electron WebContents/IPC bridge, media FFmpeg, specs, MCP, orchestration; **(b) retain** transport/tool schemas where practical, **(a) port** document/tool execution to Rust authority |
| Renderer agent harness: `legacy/assistant/`; plus `main/agent-orchestrator.ts` (647) | 5,897 + 647 | PM/DOM/UI, command repair/validation, checkpoint/rollback, project tools/media handoff; **(a) port** live mutation/checkpoint semantics, **(b) retain** orchestration/provider policy after replacing ports |
| External MCP subset: `main/agent-tools/{external-*,mcp-*}`, `server/mcp.ts` | 704 | Node net/fs, private discovery descriptor, session identity; headless mode uses Playwright + production renderer; **(b) retain** stdio adapter initially, **(a) port** native session executor/server, **(c) replace** hidden-browser document host |
| Cloud/Store: `main/cloud/`, renderer cloud/store, `legacy/core/cloud-media.ts` | 7,994 | Registry wire/zod/fetch, auth, safeStorage, publish/install/trust/provenance, Svelte screens; **(b) retain** wire/service workflow first, **(a) port** native UI and Keychain integration |
| Transcription: `main/transcription/`, renderer transcription, shared transcription contracts | 3,528 | transcribe-cpp/dylib, FFmpeg, model download/checksum/catalog/cache, worker lifecycle; **(b) retain** service orchestration first, **(a) port** UI and eventually Rust worker control, reusing native inference library |
| Recovery slice: `projects.ts`, autosave queue, memory/close helpers, `main/renderer-recovery.ts`, durable-file, backup-retention | 919 | PM store/bus, journal/checkpoints, file fsync, retention, renderer liveness; **(a) port** to Rust journal/recovery and native window lifetime; memory.ts is cache policy, not the autosave implementation |
| Recovery orchestration/storage: `legacy/app.ts` (2,055), `main/storage.ts` (833), storage codec/upgrade (172) | 3,060 | Shared app file also contains import/save/UI logic; Node fs, data migrations and storage batching; **(a) port** applicable behavior by module, not this monolith verbatim |
| Project I/O: `main/project-files.ts` (343), `project-save.ts` (157), hash/validator; shared project-container/incremental; `legacy/core/project-file.ts` (299) | 1,144 | Chunked media, verified range reuse, durable publication, backups, stale save checks; **(a) port** writer/storage policy; reuse existing Rust reader and native canonical streaming |
| Media/library slice: legacy media/library/asset-folders/media-work, host/library-media, main library-media/media-proxy/remote-media, shared library-media | 2,651 | Catalog ownership, local/linked/remote media, proxy queue/cache, filesystem, PM document/history; **(a) port** authoritative catalog and native import, reuse `pm-media`; selected network helpers may temporarily use **(b)** |

The smaller counts are deliberately not claims of small scope. Selection gestures
also live in viewer/timeline panels; imports also live in `legacy/app.ts`, image/folder
import, paste, SVG/Figma/caption modules; UI action registration lives in kernel/install.
Audit those callers against the GPUI inventories when implementing a lane.
Rust query/kernel support does not make their application-level orchestration disappear.

### 4.1 The TypeScript service contract

Proposed service: `apps/mac/services/agent-service/`, launched with a bundled Node
runtime only when needed. It reuses provider/cloud/transcription orchestration after
removing Electron imports; existing files are not all runnable unchanged under Node.
Replace `ipcMain`, `webContents.send`, dialogs, shell open/reveal, `safeStorage`, and
`utilityProcess` with explicit host requests. Keep native UI and Keychain in Rust/AppKit.
Transcription's worker/process glue must use ordinary child-process transport.

Use versioned local RPC over inherited pipes or a private Unix socket, with request ID,
document/session identity, expected revision, progress, cancellation, and bounded payloads.
Reserve `pm-protocol` for engine semantics; add application RPC schemas for provider,
cloud, dialog, extension migration, and document-command operations.
Large captures/media travel by approved file/range handles, not unbounded JSON arrays.
All writes are `execute_commands` against the Rust owner, returning committed revision
and undo transaction ID. No service may retain and later overwrite a stale whole project.
Stream provider events to `panel-agent`; route questions/approvals through native UI.
Preserve existing consent, isolation, checkpoint conflict and publication behavior.

MCP's connection descriptor/token remains private local state, with lifecycle cleanup;
see `desktop:src/main/agent-tools/external-server.ts:8` and `external-session.ts`.
Use the same Rust command/query service for native UI, MCP, and agent tools.
Headless mode becomes `--headless-document` with a Rust core/job host, not a GPUI
window or Playwright page (`desktop:src/server/mcp.ts:26` is the old browser dependency).

### 4.2 Native v4 extensions

Proposed `apps/mac/crates/extensions` owns manifest validation, registration, disposal,
permissions, watching, deterministic precedence, and per-document extension state.
Its separate JSC host process runs JavaScript; TypeScript must be compiled beforehand
or by the service/build tooling. JavaScriptCore does not parse TypeScript directly.
Panels declare `kit` components/actions/state over RPC and receive native event IDs.
Never expose GPUI objects, Rust pointers, or arbitrary AppKit selectors to scripts.
Effects/transitions/shader layers keep existing GLSL headers, uniforms, texture input,
definition snapshots, missing-definition behavior, and hot-reload invalidation.
Svelte/DOM/iframe mounting and synchronous canvas-returning v1–3 APIs are replaced.
Scripts can await render/query requests; synchronous viewer picks remain in Rust.
Provide the documented `migrate_extension` tool/skill path and validate migrated
manifests, actions, panels, and shader output before marking an extension migrated.
Old scripts depending on Node APIs or raster wrappers need substantive rewrites.

## 5. Process, thread, and frame model

Recommended process allocation:

- **UI Rust process:** main-thread GPUI/AppKit, authoritative core, local query cache;
  engine render/scheduler workers; native audio control/mix workers; bounded media pool.
- **Rust job child:** exports, agent frames/contact sheets, proxies; immutable snapshots,
  separate GPU/cache budgets, lower priority, cancellation, verified output publication.
- **JSC extension child:** script evaluation and declarative panel messages; restartable.
- **Optional Node service child:** provider/cloud/transcription orchestration; launches
  the already external `codex app-server`, `claude`, and inference workers as needed.

Render threads own `Renderer`, mutable GPU caches, and submitted work. Decode/mix/file
work never waits for GPUI; main-thread font broker asks are serviced asynchronously.
Construct/pump `FontService` on the main thread (`pm-node/src/fonts/mod.rs:295`),
and use `FontBroker`/`RenderFonts` or `ServedFonts` for worker consumers.
Avoid a deadlock where main waits on rendering while rendering requests main's fonts.
Move large parsing, hashing, serialization, thumbnail work, and retired tree destruction
off-main. The UI may make small synchronous queries against already prepared state.
Apply foreground QoS to interactive workers, not the old utility-process-wide setup
indiscriminately to the whole app. Preserve job GPU slicing while playback is active.

### 5.1 IOSurface to a GPUI element: the required bridge

Verified installed source for the pinned `gpui-pre = 0.3.8`:
`gpui-pre/src/elements/surface.rs` exposes `surface(CVPixelBuffer)` and calls
`Window::paint_surface`; `gpui-pre-apple/src/metal_renderer.rs:1165` draws it.
At line 1200 that renderer asserts `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`
and samples Y and CbCr planes. BGRA/RGBA-half IOSurfaces will not work through it.
These sources are under the local Cargo registry, not files owned by this repository.
The installed source was used because the matching docs.rs pages were inaccessible.

Recommended bridge is a small maintained GPUI platform patch adding RGB surface input,
RGB Metal sampling, color/alpha metadata, and consumer-completion notification.
Keep that patch outside panel code and pin its revision; upstream it where practical.
Do not convert graphics frames to NV12 just to fit the video element: that loses alpha,
chroma detail, and half-float range. Do not use per-frame CPU image readback/upload.
An AppKit Metal subview can be a bounded prototype alternative, but then document its
clipping/z-order/glass/overlay/accessibility behavior; it is not the requested GPUI
element integration and should not quietly become the permanent viewer architecture.

The intended RGB path, in order:

1. Core publishes immutable document revision R and request generation G/time T.
2. Render worker renders working pixels with `Renderer::render_into`; reuse the
   existing viewer placement/ROI logic from `pm-node/src/present/compose.rs`.
3. `DisplayTransform::encode` converts once into a free `ExportedSurface` slot.
4. Submit wgpu work and publish readiness only after producer GPU completion,
   asynchronously; alternatively use a deliberately implemented shared-event fence.
5. Send an in-process frame handle carrying retained surface, R/G/T, size, format,
   placement, color space, and slot/generation. No Mach port is needed in-process.
6. Wrap the IOSurface in a retained CVPixelBuffer with matching RGB format using
   `CVPixelBufferCreateWithIOSurface`, or expose the retained surface to the new source.
   Bridge the objc2/CoreFoundation and GPUI core-video wrappers with audited ownership.
7. The patched GPUI element imports/samples the RGB Metal texture and draws it at
   the viewer's clipped layout rectangle; GPUI draws gizmos above it in the same scene.
8. Release the pool slot only after GPUI's consuming Metal command buffer completes.
   Retaining an IOSurface or returning from `paint` does not establish that completion.

Start with color-correct SDR BGRA. GPUI's current Apple layer uses BGRA8Unorm;
RGBA16F/EDR presentation requires additional layer/display/color work, not just accepting
a half-float source. Test sRGB/P3 conversion, premultiplication, top-down rows, Retina
scaling, resize, clipping and transparency before extending to EDR.
Validate adapter identity when the engine and GPUI choose Metal devices separately.
Use a small bounded ring; drop superseded ready frames, never overwrite an in-flight
slot, and retain the last good frame when paused, decoding, or rebuilding the pool.
On resize/device loss, rotate pool generations and retire old slots after fences.

### 5.2 Synchronous gizmos and answers

Core owns a `pm_wasm::Engine` query instance at the committed document revision.
It receives each accepted canonical patch transaction immediately, before UI events
for that revision. Worker rendering uses shared immutable snapshots of the same tree.
Gizmo pointer handlers convert view coordinates through the viewer placement and call
local pick/bounds/text geometry directly. No Node, JSC, IPC, audio, or GPU wait occurs.
Cache per revision/time/camera/font/model generation, not just layer ID.
Bind picked indices back to stable IDs before issuing commands; reorder invalidates indices.
If displayed pixels lag the current edit, use frame metadata to keep overlays and
hit tests on one declared revision/time, or hide stale overlays until they agree.
Never combine an old frame's coordinates with an unrelated current query snapshot.
Preload models/font outlines; schedule expensive changed-time mesh preparation ahead.
The old 1M-vertex first-pick result can take 15 ms even though cached picks are tiny;
“synchronous” is not permission to block every drag frame on uncached geometry.

## 6. First spine milestone: open, view, play, edit, undo, save

Deliver one native window opening a real existing .pmv with embedded media, showing
its layer list and current frame, scrubbing/playing, changing opacity, undoing/redoing,
then saving and reopening with media intact. Choose a representative fixture within
implemented native classes; report unsupported features for other files without losing
their data. This milestone proves the boundary, not all 1:1 parity items.

New application crates/modules (proposed, not present APIs):

| Owner | New files/modules | Concrete responsibility |
|---|---|---|
| `apps/mac/crates/core` (`pm-mac-core`) | `document.rs`, `commands.rs`, `history.rs`, `selection.rs`, `queries.rs`, `project_io.rs`, `media_catalog.rs` | Single document authority; stable IDs; save envelope; transactions; local queries |
| `apps/mac/crates/engine-host` (`pm-mac-engine-host`) | `session.rs`, `render.rs`, `transport.rs`, `fonts.rs`, `frames.rs` | Plain Rust facade over existing engine calls, worker scheduling, assets/fonts, frame lifecycle |
| `apps/mac/crates/panel-viewer` | `frame_element.rs`, `view.rs` | RGB GPUI element consumer, fit/zoom/placement, time input and overlays |
| `apps/mac/crates/panel-project` | `layers.rs` | Read-only layer list plus opacity control calling core command |
| Small GPUI Apple patch dependency | RGB surface/Metal/completion changes | Makes engine frames displayable without NV12 conversion |
| App shell owner | `apps/mac/src/main.rs`, `workspace.rs`, manifests | Menus, open/save sheets, crate wiring; only this owner edits these shared files |

Minimum implementation sequence:

1. **Read and retain.** Background `read_container` from a file; preserve envelope and
   source media table; canonicalize project and select `open_comp_id`. Build native
   catalog mappings, extracting embedded ranges to bounded managed files as needed.
   Keep originals/ranges for save reuse. Handle unavailable linked files explicitly.
2. **Own and query.** Create Rust document session with monotonic revision and saved
   revision/hash. Adopt it into the local `pm_wasm::Engine`; resolve open `Comp` and
   enumerate `Scope::layers` or `Node` arrays to build the layer list.
   A proposed `NativeSession` facade may use `Engine` protocol sessions internally;
   do not pretend the existing engine has a typed GPUI session constructor.
3. **Prepare rendering.** Start `Gpu`/`Renderer`; install `all_builtin_definitions`
   and project/extension snapshots, assets, and main-thread font service. Run capability
   check. Start with direct Rust calls; no napi addon or WASM module is loaded.
4. **Present.** `RenderRequest` + `Renderer::render_into` + working/display conversion
   + retained RGB surface bridge. Stamp the first frame with revision and time.
5. **Scrub/play.** Convert slider input to rational-rate engine ticks; coalesce seek
   requests. Use native transport/playback clock policy; update render requests and
   UI from that clock. Include a real audible clip in the spine to verify sound and
   pause/seek behavior, as well as a silent composition's host-clock path.
6. **Edit.** Add one typed `SetStaticOpacity { layer_id, value }` command. For an
   unanimated layer, write the channel's static value under canonical
   `comps/<comp-id>/layers/<resolved-index>/p/opacity/v` (percent, 0–100).
   Validate existence, lock state, finite range, and absence of animation/expression
   for this first command; animated/keyframed editing belongs to the next command lane.
7. **Undo.** Capture inverse leaf state, including existence, before applying forward
   `Op::Leaf(Patch)`. Stage atomically; commit one revision and one history record;
   update query/render/audio snapshots. Undo/redo applies inverse/forward operations
   at NEW increasing revisions. Do not rewind revision numbers to undo old content.
8. **Save.** Freeze envelope/history/media at a chosen revision; stream a PMV4 container
   into a sibling temporary file, hash records/index, flush/fsync, verify, then publish
   durably. Reuse verified unchanged media or stream it from the retained source.
   Only clear dirty state for the revision actually written; edits during save survive.

The writer in step 8 is real new work. `pm-model::container` is read-only.
`pm_node::kernel`'s `persist_to_file` already streams/fsyncs canonical JSON with
cancellation, and `NativeKernels` exposes persist receipts, but that is a checkpoint,
not a PMV4 envelope/media writer. Reuse its bounded streaming principles.
Port the container/writer semantics from `desktop:src/shared/project-incremental.ts`,
`main/project-save.ts`, `project-files.ts`, and `durable-file.ts`.
Never save only `pm_wasm::Engine::to_json()` and discard workspace/history/media.
Initially preserve old history/workspace fields even if only new-session undo is active;
reading/replaying the old saved history format remains an explicit later parity item.

Spine acceptance evidence when disk permits later validation:

- Open a real PMV4 plus legacy JSON/PMV3 samples; correct composition/layer IDs/order.
- Embedded media and fonts display; selected-frame comparison uses native pixels.
- Scrub final source frames, play audible and silent clips, pause, seek, loop.
- One opacity edit changes pixels; undo restores them; redo reproduces them.
- Save/reopen retains media bytes, unknown fields, workspace envelope, and edit value.
- Save failure/disk-full leaves the prior file valid; editing during save stays dirty.
- Rapid resize/scrub/reopen never shows a retired surface or crosses document identity.

## 7. Migration lanes and file ownership

The following is an ordering for future agents, not a request to run them now.
Keep the architecture's maximum of three concurrent lanes and shared target directory;
no lane should start builds while disk is nearly full.

**Lane 0, one spine owner:** settle core command/query/frame contracts, project writer,
RGB surface patch, open/edit/undo/save proof, workspace manifests and shell wiring.
Do this before panel agents depend on guessed APIs. Publish Rust interfaces for
`DocumentSnapshot`, command results, `QueryContext`, `FrameHandle`, catalog, and jobs.

**Wave 1, separate files:** (A) `crates/kit` and native controls/accessibility;
(B) `crates/panel-timeline` using core commands; (C) `crates/panel-project` plus catalog
UI. After kit contracts land, run inspector as its own lane. Project catalog changes
under core go through a named core owner, not parallel edits by panel authors.

**Wave 2:** (A) viewer tools/gizmos in `panel-viewer`; (B) inspector/effects/easing/graph
in their owned panel crates; (C) `crates/export` plus job-host adapter. The renderer
parity lane owns `engine/crates/pm-render`, GPU, text/vector/media fixes; schedule it
separately from another lane touching those same engine modules.

**Wave 3:** (A) agent-service + `panel-agent`; (B) v4 extensions + migration tooling;
(C) native MCP/headless entry. All consume the frozen core protocol; only the core
owner adds commands. Then cloud/Store and transcription/mixer/captions can be separate
lanes with their service modules and native panels, followed by deeper 3D editing.

**Shell/accessibility and cutover:** shell/settings/workspaces, native menus/keymaps,
file associations, update/signing/packaging, localization, and end-to-end parity audit.
Let one integration owner edit `apps/mac/Cargo.toml`, lockfile, `src/main.rs`,
`workspace.rs`, and shared protocol/schema exports. Panel lanes submit integration
requests instead of touching these files. Keep the engine facade change surface small.
Each lane owns its parity checklist evidence; historical Electron tests are behavioral
references, not automatically passing native tests. No release is implied by this plan.

## 8. Largest risks to resolve first

1. **RGB presentation and reuse:** GPUI's NV12-only surface and absent application-facing
   consumer fence are a real integration task; prove this before polishing panels.
2. **Saving without data loss:** a good reader/checkpoint writer is not the durable,
   incremental, media-preserving file workflow that users already rely on.
3. **Hidden browser dependencies:** fallback GLSL, raster wrappers, font seeds, and
   routed formats cannot be delegated to a headless TypeScript service by assumption.
4. **Ownership and latency:** one mutable document, matching query/frame revisions,
   main-thread fonts, bounded GPU queues, and no blocking audio commands in the UI.
5. **Parity evidence:** current source supersedes some failed gate-era limitations;
   it does not supply a fresh full native-only gate or GPUI performance evidence.
6. **Scope outside rendering:** provider workflows, recovery, media ownership, v4
   migration, and old saved history are substantial work despite a large Rust engine.
