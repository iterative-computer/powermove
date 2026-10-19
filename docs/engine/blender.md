# Blender in the engine: design notes

The Blender lane (plan section 8 "Blender surfaces", section 9 "3D layers"
and "Alpha", section 14.7's Blender surface cache, section 17's Blender
stage, contract 11 (`docs/engine/rfcs/11-blender-jobs.md`), contract 14's
Blender supervision, amendments 11 and 14, spike S7). Branch
`feat/ne-blender`. The plan stays the authority; this file records what was
built, how it behaves, and what was measured.

## The pieces

| Where | What |
|---|---|
| `engine/crates/pm-blender` | Snapshots (`renderSnapshotJsonV1`: canonical bytes, BLAKE3 hash, camera hash), section 25's snapshot comparison, surface decoding, the section 14.7 key, the surface cache (memory and disk, device record), the pinned worker script and its hash, Blender discovery and the supervised worker (CLI host, tests), and the Blender stage (`job::BlenderStage`) with the `BlenderBackend` trait |
| `engine/crates/pm-blender/worker/worker.py` | The pinned worker: today's worker with section 17's output settings, metadata stamps off, `outputs` per request, and the resolved device in the manifest |
| `engine/crates/pm-render/src/blender/` | Snapshots built from `pm-eval` and `pm-geom3d` (`snapshot.rs`), today's `blenderFrameRequests` (`frames.rs`), the plan hook and `BlenderRender` modes (`mod.rs`), the surface upload (`gpu.rs`) |
| `engine/crates/pm-export/src/blender.rs` | The export's Blender stage: per frame, the renderer's snapshots, the stage, and `BlenderRender::Final` |
| `engine/crates/pm-node/src/jobs/blender.rs` | The job engine's backend: snapshots and converted assets into the Blender Surfaces cache area, `blenderRender` over the host port, surfaces from `blenderSurface` events, `cancelJobs` on cancel |
| `apps/desktop/src/main/engine/blender.ts` | Host services' Blender supervision behind Use New Engine: the pinned worker in a least-privileged Blender, events before the answer, cancel |

## Snapshots

`pm_render::blender::snapshot::build` is today's `renderSnapshot`
(`core/scene3d/rendering.ts`) on the engine's evaluation:

- The scene is `compositionScene` (`Scope::composition_scene`); meshes and
  world matrices come from the 3D pass's scene builder, which places
  objects at `world3D` exactly as `compositionRuntime` does, over
  `pm-geom3d` geometry that is bit-identical to three.js's.
- One mesh per geometry group (or one over the whole geometry): vertices
  de-indexed as `getVertexPosition` gives them (f32 widened to f64),
  normals normalized as `Vector3.normalize` (a multiply by the reciprocal
  of the f64 length), UVs as `uv.getX(i) || 0`, `slot?.material ||
  node.material` as evaluated, the mesh's `matrixWorld`, `smooth: true`.
  Source materials (`useSourceMaterials` without a slot) are written as
  `sourceMaterialSnapshot` writes the runtime's material (amendment 14's
  fix: a `MeshBasicMaterial` becomes an emissive surface).
- `castShadow` is JavaScript's `node.castShadow && environment.shadows`
  (an absent value stays absent); lights at their runtime positions and
  targets (points and area lights: `[0, 0, 0]`).
- `blocks` walks `compositionDepthIds` over the composition's non-group
  layers in stack order (group ancestors are looked up in a list without
  groups, so only a layer's own solo counts), in its insertion order;
  `backgroundFor` takes the last of each set in `compositionOrderedLayers`
  (runs of 3D layers sorted by the camera depth of their origins).
- The camera is the composition's (or the editor camera of a preview in
  the editor view), with `orthoHeight` 6 (±3 after `resizeCamera`) for
  orthographic cameras and 45° for their field of view, as today.
- Size: `Math.round(size × previewScale)`, at least 2. Settings:
  `render3DSchema.parse`, the frame's samples, EEVEE for Material Preview.
- Assets: every map, shader source, and shader graph image the scene's
  materials name. PNG, JPEG, and WebP files are passed as they are (sniffed,
  as today's `blob.type`); other images are converted to 8-bit PNG
  (today: Canvas2D, appendix A item 3), other assets by name.

Not built (the frame is routed, as the static check already routes it):
imported glTF models (the glTF lane's loader) and model-group recipe
parts (`pm-geom3d` recipe geometry).

The JSON tree is a `pm_model::Value`; its canonical serialization (sorted
keys, `JSON.stringify` numbers) is the cache file's bytes and what the
snapshot hash covers.

## Frames

The frames an output frame needs are found by planning it
(`Renderer::blender_snapshots`): every compositing pass of a composition
with `render3d.enabled` (every one with 3D objects for previews) gives its
frame at the plan's own composition, time, and size, so the lookup at draw
time always matches. `blender::frames::frame_requests` is the port of
today's `blenderFrameRequests` (motion-blur shutter samples at
`t + ((i + 0.5)/n − 0.5) × shutter / fps`, nested sizes, depth 8, 4,096
frames); it equals today's on every corpus frame list. pm-render still
routes motion blur (`Feature::MotionBlur`); when it draws it, the plan
visits the sample times and the snapshots follow.

## Surfaces

- Pinned output (section 17, amendment 11): 16-bit RGBA PNG,
  `view_transform 'AgX'`, `look 'None'`, display `sRGB`, gamma 1,
  `dither_intensity 0`, image color management following the scene,
  compression 15 (S7's "fast compression"), metadata stamps off.
- Decoding (`pm_blender::surface`): codes as written (no gamma, sRGB, ICC,
  or cHRM handling), `code / 65535` (8-bit: `/ 255`), premultiplied in E at
  f32 (`c × a`), the crop rectangle as today's `cropSurface` (alpha above
  0, grown by 2, clamped; an empty surface keeps one transparent pixel).
- Compositing: the plan draws a 3D object layer of a Blender composition as
  `Source::Blender` over its crop rectangle in target pixels (today's
  `screenRect × (W, H)`), with the layer's opacity, masks, effects, and
  blend exactly like the native 3D output, flagged as E-encoded so linear
  blending decodes it once (section 9). Uploads are `rgba32float` at 32-bit
  precision (exports) and `rgba16float` (round to nearest even) otherwise.
- The depth and id surface reports a Blender layer's coverage from its
  alpha and no depth (today's Blender frames have none either).

## The Blender stage of an export

```text
per frame: plan without drawing → snapshots (per nested composition)
  → keys → cache (memory, then disk) → Blender for the missing ones
  → decode → BlenderRender::Final → render → encode
```

- Keys (section 14.7, contract 11 invariant 6): snapshot hash, worker
  script hash (SHA-256 of `worker.py`, the same in Rust and in main),
  sorted asset hashes (SHA-256 of the bytes), Blender version, settings,
  resolved device, preview flag, camera hash, size, layer, shutter sample.
- Devices: Blender resolves `auto` per job and the manifest names it
  (Cycles: the enabled devices; EEVEE: the system GPU as Cycles lists it,
  stable from the first render on). The stage keys lookups with the device
  a previous job recorded for (version, engine, requested device), kept on
  disk with the surfaces; Cycles on the CPU is known up front.
- Batching: a frame's layers render in one worker invocation
  (`BATCH_LAYERS`, amendment 11; S7 and `batched_and_per_layer_...` below).
- A surface is rendered at most once per key: a second export of the same
  frames runs no Blender job. The job renders from its frozen document, so
  edits never invalidate its surfaces (appendix A item 31).
- Failures: Blender missing (`blenderMissing`, appendix D's sentence), a
  failed or timed-out job or a surface that does not come back
  (`blenderFailed`) fail the export; a layer drawn without its surface
  fails it too, never routed and never drawn natively (contract 11
  invariant 7). Cancelling the export stops Blender.
- Routing: `plan_export_with(job, true)` drops `Feature::Blender` from the
  static check when the lane has a Blender stage; everything else the
  static check lists still routes (imported glTF models, which are not
  built into snapshots yet, and SVG texture maps). Recipe parts are built
  since Phase 3c, and texture maps of every level 1 still class draw.

## Host services (contract 11 over the host port)

- The job engine mints a `cache {blenderSurfaces}` handle, resolves it, and
  writes `snapshots/<blake3>.json` and converted images as
  `assets/<sha256>.<ext>`; other assets are named by `mediaRead` handles.
- `blenderRender` carries one `SurfaceRequest` per layer of one frame. Main
  checks the worker-script hash against its own script, the handles
  against the session and port, the snapshot's size, and today's snapshot
  rules (`validateBlenderJob`); copies assets in as `<id>.<ext>` under
  today's extension list and the 256 MB input cap; runs the worker; moves
  each PNG to `<area>/<key>.png` under the per-surface cap; and sends
  `blenderSurface` (the crop is the full surface; the engine finds the
  occupied rectangle when it decodes) and `blenderProgress` events.
- The request is answered when the job ends: `ack` after its last surface
  event, or `blenderMissing`, `blenderFailed`, `cancelled`. The job engine
  waits up to the 10-minute job timeout per frame.
- Cancel: the job engine sends `cancelJobs {kind: job}` with the Blender
  job's id; main kills the worker's process group (a queued job is
  dropped) and answers `jobsCancelled`. An engine launch ending cancels its
  jobs. A queue of 16 jobs; one worker process, serialized.
- Least privilege (contract 11 invariant 2, appendix A item 29): the
  worker runs with `--background --factory-startup --disable-autoexec
  --offline-mode --python-exit-code 1`, an environment of its own (fixed
  `PATH`; `HOME`, `TMPDIR`, and `BLENDER_USER_*` in its scratch directory;
  nothing of main's), and its own process group.
- Playback (contract 13, `SlicingPolicy.blender` is `pause`): while
  anything plays, main stops the worker's process group (`SIGSTOP`) and
  continues it (`SIGCONT`) when playback ends; a job that starts during
  playback starts stopped; the 10-minute timeout does not count paused
  time. The job engine therefore waits for an answer (bounded at 6 hours,
  for a host that never answers) rather than timing Blender itself.
- Discovery: the Blender chosen in Settings (`blender-path.json`, as the
  current engine reads it), else the first Blender 4.5 or newer found
  (`discoverBlender`).
- `HostServicesCore` (and pm-host's `CliHost`) keep answering Blender
  requests `unsupported`; NativeEngine serves them when it is given a
  Blender locator (the app always gives one). The CLI's `pm-cli render`
  runs the stage in-process on the installed Blender.

## The paused viewer

Today's viewer draws Blender only while paused in Material Preview
(EEVEE) or Rendered (the composition's engine) shading
(`renderBlenderPreview`, `schedulePreview`); Wireframe, Solid, and
playback draw natively. The interactive engine does the same:

- **Shading reaches the engine.** The viewer's `ViewerState.view` carries
  the 3D viewport's shading whenever the composition has a scene (Rendered
  without a camera used to be sent as no override); the presenter passes
  the override into its renders (`compose.rs`: editor camera, Wireframe and
  X-ray looks, Local View, and the shading).
- **Routing.** The viewer never draws final Blender frames, so
  `Feature::Blender` no longer routes a composition to the current viewer
  (`present/document.rs`); a Blender composition draws natively, or with
  its preview surfaces while paused.
- **Paused frames** (`present/render.rs`): a frame drawn while paused in
  Material Preview or Rendered renders with `BlenderRender::Preview` over
  a `ViewHost` (`present/blender.rs`). Every composition with 3D objects
  builds its preview snapshot (the composition's own size times the preview
  scale, whatever the zoom; preview samples; EEVEE for Material Preview),
  draws the surfaces the presenter has for it, and draws natively while it
  reports the snapshots it lacks. Motion-blur samples of a paused frame
  draw natively (preview frames exist only at the frame's own time, as
  today). Playback frames and the draft shadings render
  `BlenderRender::Native`.
- **The worker** (`PreviewBlender`, thread `pm-viewer-blender`): the lacking
  snapshots become the view's wanted set; after today's 250 ms debounce the
  worker runs them through a `BlenderStage` (section 14.7 keys over the
  Blender Surfaces area, so a frame rendered before, by the viewer or an
  export, is never rendered again) and host services' supervised Blender
  (`HostBackend` over a `HostLink`: host calls queued for the engine's
  thread, drained on `tick` and on every wake-up `pump`, answered by token;
  `blenderSurface` events routed to the presenter). Decoded surfaces stay
  in memory under today's 512 MB preview budget, least recently drawn first
  out, and the view is drawn again when they arrive.
- **Cancel.** A paused frame that wants other snapshots (the playhead
  moved, an edit), a draft shading, or the view closing cancels the running
  job: `cancelJobs` for its Blender job, which kills the worker's process
  group. Main accepts `cancelJobs` from the interactive engine for its own
  launch's Blender jobs only (`EngineBlender.owns`).
- **Playback.** Host services stop Blender (`SIGSTOP`) while anything plays
  (contract 13); a job running when playback starts finishes after it
  stops, and no job starts during playback.
- **Failures** (Blender missing, a failed job) go to the viewer as a
  session failure (`blenderMissing`, `blenderFailed`; the viewer logs it)
  and stop that view's previews until its shading changes, as today's
  `error` stops `schedulePreview` until `setShading`. A host without a
  Blender path (`unsupported`) stops previews for the launch.
- **Today's preview stands aside** when the engine draws the viewer
  (`schedulePreview` in `core/scene3d/rendering.ts` checks the engine
  connection's route), so Blender runs once per preview; the current
  viewer, and the routes it still draws, keep today's preview.
- **Host services** (`native-engine.ts`): `blenderRender` is served for
  both engines (one serialized worker and one queue of 16). The utility
  process calls `pump` on every empty action batch (a wake-up from a thread
  off its own), so host calls from the job lanes and the viewer's worker go
  out at once rather than at the next 100 ms `tick`.

## Tests and measurements

Apple M4 Max, macOS 27, Blender 4.5.14 LTS (`/Applications/Blender.app`),
wgpu 30.0.1 (Metal), corpus `v1-20261012-230504-ae47a3b3`, 2026-10-17.

| Test | What | Result |
|---|---|---|
| `pm-render` `tests/blender_snapshots.rs` | Every Blender snapshot of the corpus (final, preview, export, exportAlpha; every shutter sample; nested compositions) against today's `renderSnapshot`; every frame list against `blenderFrameRequests` | 158 snapshots: 135 equal (geometry: 11,274,896 values, max abs difference 3.5e-16; 9,622 other values exact; asset ids, names, and SHA-256 equal), 23 not built (`3d/gltf`, `3d/gltf-materials`: imported models, the glTF lane); 74 of 74 frame lists equal (times bit for bit). Round 3 (2026-10-19, `v1` plus `threed-20261019-roles-maps`, not-built now a failure): 182 compared, 153 equal, **29 failures** (23 not built, 6 converted-texture differences); 90 frame lists equal; geometry max 3.5e-16 |
| `pm-render` `tests/blender_composite.rs` | Two layers of 16-bit surfaces with gamma and sRGB chunks, off-center crops, 50% opacity, in plan order, against an f64 source-over reference | 32-bit: max 6.8e-8; 16-bit: max 1.06e-3 (half-float storage and accumulation); missing surfaces fail each layer with today's sentence; preview draws what it has and asks for the rest; nested compositions are their own frames |
| `pm-export` `tests/blender_export.rs` | A 2-frame 16-bit PNG export of two models on the installed Blender | One Blender request per frame (layers batched), 4 surfaces, device `Metal {Apple M4 Max (GPU - 40 cores)}`; every exported sample equals the f64 composite of the stored surfaces (max 0 of 65,535); re-export: 0 Blender jobs, 4 cache hits (0.85 s against 0.03 s) |
| same | Batched against one invocation per layer (EEVEE), and two batched runs | Decoded surfaces equal; files byte-identical; no text chunks |
| same | Cancel 1.5 s into a Cycles CPU render (1920 × 1080, 4,096 samples) | `Cancelled`, the worker's process group is gone |
| same | No Blender; a backend that drops a surface | `blenderMissing` with appendix D's sentence; `blenderFailed`; nothing written; without a Blender stage the job still routes |
| `pm-node` `present::blender` tests (stand-in host services: the Blender Surfaces area in a temporary directory, 16-bit PNGs, cancellation as main does it) | A paused frame's wanted set; debounce; a new launch with the same area; a new frame, Solid shading, and the view closing during a job; Blender missing; the memory budget | One request for a frame's two layers (`preview-<port>-<session>-blender-<n>`), surfaces decoded at the snapshot size; a new launch renders nothing (2 cache hits); each change cancels the running job (`cancelJobs` for it) and nothing stays open; `blenderMissing` reported once as a session failure and not retried until the shading changes; least recently drawn surfaces evicted first |
| `pm-node` `present::document` | Routing of a Blender composition | The same route as the scene without `render3d.enabled` (the static check still lists Blender for exports) |
| `pm-blender` unit tests | Settings parse, canonical hashing, key inputs (every input changes the key, asset order does not), surface decoding and crops, cache bounds and disk, device record, discovery, flags, the stage's per-key rendering and failures | Pass |
| `apps/desktop` `src/main/engine/blender.test.ts` | Main's supervision with a stand-in executable: flags, scrubbed environment, surfaces before the answer, batching off, worker-script hash, foreign handles, missing Blender, cancel kills the process, playback stops and continues the process (`ps` state `T`) and holds new jobs | Pass |
| same | Cycles with `device: cpu` and `auto` | Recorded per job: `Cpu`, then `Metal {Apple M4 Max (GPU - 40 cores)}` |
| `apps/desktop` `e2e/engine-blender.spec.ts` (under the Electron lock) | The app, 192 × 108, a turning box and a sphere, EEVEE 8 samples: today's still (Use New Engine off, today's surfaces recorded at main's IPC), then the job engine's still and a 6-frame H.264 MP4 (on) | The MP4 is the job engine's (AVAssetWriter, 6 frames); 12 surfaces in the cache (2 layers × 6 frames; the still reused frame 0's); the job engine's still is 100% within 2 CV of today's surfaces composited per layer (max 2 CV) |

| `apps/desktop` `e2e/engine-blender.spec.ts`, second test (under the Electron lock), 2026-10-17 | The same project in the app with Use New Engine on: the viewer in Solid, then Material Preview paused at 0, Solid again, playback, and a pause at 0.125 s | The engine draws the viewer (route `engine`); 1 Blender request for both layers (EEVEE, `Metal {Apple M4 Max (GPU - 40 cores)}`); the viewer's pixels at 24 flat points equal the two surfaces composited within 1 CV (Solid, native shading: up to 61 CV off them); no Blender run during playback; the paused frame at 0.125 s renders its own 2 surfaces; 0 runs of today's preview at main's IPC; no page errors |

## Found along the way

- **Existing-project alpha upload reproduced (2026-10-18).** The apparent
  one-pixel offset was not geometry: today's ImageBitmap keeps straight RGB
  (including RGB under zero alpha), which WebGL blends as premultiplied.
  Level 1 now reproduces that upload and the Canvas2D alpha-derived crop,
  plus today's PNG8/dither 1 settings. Level 2 keeps PNG16/dither 0 and the
  exact premultiplied path. Viewer, snapshots and export use the same source.
  Against today's capture JPEG and still PNG, each is 100% within 2 CV,
  max 0 CV in the app e2e. Raw surfaces remain diagnostic for level 1.
  See RFC 11's `compatibilityLayers` snapshot field and bridge.md round 4.

## Changes outside the lane's files

- `pm-render/src/plan.rs`: `Source::Blender`; `Build.blender`;
  `build_blender` (with the Blender state; `build_with` calls it); the
  precomp branch pushes and pops the composition path; the 3D object
  branch asks `blender::plan_object` first.
- `pm-render/src/request.rs`: `RenderRequest::blender` (defaults to
  native); `pm-render/src/scene3d/mod.rs`: `Options3d::blender`;
  `scene3d/layers.rs`: `composition_scene` is `pub(crate)`;
  `pm-render/src/exec.rs`: the `Source::Blender` arm;
  `pm-render/src/renderer.rs`: `Renderer::blender_snapshots`;
  `pm-render/src/lib.rs`: the module. `RenderRequest` literals gained
  `blender: Default::default()` (`pm-export/src/render.rs`,
  `pm-node/src/present/compose.rs`).
- `pm-export/src/render.rs`: `JobError::Blender`, `plan_export_with`,
  `run_export_with_blender` (the old entry points call them), a missing
  Blender surface fails instead of routing.
- `pm-engine`: `JobService::host_event` (default no-op) and the routing of
  `blenderSurface` and `blenderProgress` from the host to it.
- `pm-node/src/jobs/mod.rs`: the lane's Blender stage, `call_until`,
  delivered surfaces, and the error mapping.
- `pm-cli/src/render.rs`: the Blender stage on the installed Blender.
- `apps/desktop/src/main/engine/native-engine.ts` and `index.ts`: the
  Blender locator and the routing of `blenderStatus`, `blenderRender`, and
  `cancelJobs` (both engines; the interactive engine's own jobs only).
- `apps/desktop/src/main/engine/engine-process.ts`: an empty action batch
  pumps the engine.
- `pm-engine`: `FramePresenter::drain_host_requests`, `host_answer`, and
  `host_event`, and the engine's `HostCall::Presenter` routing.
- `pm-node/src/present/{mod,render,compose,document}.rs`: the presenter's
  host link and Blender worker, `Command::UseBlender` and `BlenderReady`,
  the paused preview per render, `Composer::set_blender` and the view
  override in viewer renders, the `blenderPreview` stats, and routing
  without `Feature::Blender`.
- `apps/desktop/src/extensions/viewer/engine-present/view-state.ts`: a 3D
  composition always sends its shading.
- `apps/desktop/src/renderer/src/core/scene3d/rendering.ts`:
  `schedulePreview` stands aside while the engine draws the viewer.
- `pm-render/tests/motion.rs`: the request is cloned (as on the integration
  branch).

## Current status and remaining gaps

- **Imported glTF snapshots and converted maps.** Round 4 built all 23
  formerly missing glTF snapshots. In round 5, five independent Chromium
  conversions of the original AVIF sources supplied the exact PNG inputs;
  the complete corpus then passed **182/182 snapshots**, including 29
  converted-texture comparisons by decoded pixels, and 90/90 frame lists.
  The live app then passed both an imported AVIF map and an SVG map through
  the renderer's `compatFrame` Canvas2D PNG producer to the interactive
  engine's Blender worker: one PNG answer, one native job and one surface
  for each, with zero legacy `blender:run` jobs. A cold synchronous capture
  returned today's prepare-frame error immediately, before any PNG ask.
- **The viewport's busy and error state** now follows the interactive
  engine's per-session, generation-tagged preview status. `renderFrame`
  requests the engine's Blender preview. With a paused Material or Rendered
  viewer, it completes after the matching fully backed frame is shown by
  the engine viewer's presentation. During playback, in Solid or Wireframe,
  or with no mounted viewer, it prepares the same Blender cache through the
  engine document session and completes without a viewer frame. Neither path
  pauses, seeks, or changes the viewport shading. `cancelRender` cancels the
  engine job (and resolves after the window worker cancel even if the engine
  connection is unavailable), while `previewState()` and `onPreviewChange`
  receive automatic and explicit job changes. The round-5 native API pilot
  passed 1/1 in Electron, including render, status, cancellation, and zero
  calls to today's `blender:run` preview path. All three Blender motion app
  tests passed with native preview/capture pixels and host-worker engine proof.
- **The editor camera** is sent by the viewer since Phase 3c
  (`scene3d.viewport.state().engineCamera`, docs/engine/scene3d.md); the
  round-5 preview API test now cancels, orbits into the editor camera and
  restarts successfully. Its actual presented frame matches the transmitted
  editor camera, current view sequence, revision and time; native Blender
  jobs increase, capture pixels are nonempty, and no legacy preview job runs.
- **Agent frames** (`render_frames`) first ask the job engine's `render_still`
  path. It checks `unsupported_with_media` without a Blender stage, so a
  Blender composition routes back to renderer orchestration. With Use New
  Engine on, that renderer calls `PM.Export.snapshotAsync`, which uses
  `PM.EngineCaptures` and prepares the Blender frame through the native
  snapshot path; it does not draw through the legacy WebGL compositor.
  The request's `cachedOrPending` policy is not yet applied in `run_frames`.
- **Motion-blurred exports** of supported layer types are drawn natively:
  the plan visits shutter samples, prepares their Blender surfaces, and the
  Blender motion app test passed transparent, opaque, and nested stills.
  Capability routing remains for unsupported layer types, not for motion
  blur itself.
- **Leases**: the job lane's decoded surfaces use a fixed 512 MiB budget,
  not the `blenderSurfaces` lease category; the disk area's bound is 8 GiB.
- **Converted textures:** round 4's TIFF rendition comparisons stay exact.
  Round 5's Chromium PNG producer closes the five AVIF inputs that made six
  captured snapshots differ; the independent converter plus full corpus
  comparison passed 182/182. The live AVIF app round trip passed through
  `compatFrame` with a content-hash-matched 192 × 128 PNG answer.
- **SVG maps in Blender:** the same exact-PNG request path accepts SVG
  sources. The live app rendered a freshly imported SVG as a Blender map
  from a content-hash-matched 32 × 32 PNG answer; the retained Blender
  corpus has no SVG map for a separate snapshot comparison.
- **Fixed in Phase 3c round 2: media-store assets.** Media-store files are
  content-addressed and may have no extension; main's Blender stage refused
  them ("Unsupported Blender asset format"), so a texture imported into the
  store failed every engine Blender job (`blender-motion.spec.ts`'s agent
  spec with the engine on). Their extension now comes from the file's
  first bytes (`assetExtension`, `main/engine/blender.ts`).
  Round 3 completes it: today names a model file by the asset's name
  (`chair.obj`, `scene.gltf`), so text models are recognised by content too
  (glTF JSON with an `asset` member; OBJ by its first statement after
  blank and `#` lines, UTF-8 with or without a byte order mark). EXR and
  HDR files without an extension are no longer passed through: the engine
  passes only PNG, JPEG and WebP images as they are (today's `blob.type`
  rule) and converts every other image to PNG first, as today draws them
  through a canvas (Chromium decodes neither EXR nor HDR, so today's
  conversion fails, and the engine's fails the same way). A linked file
  keeps its own extension. `blender.test.ts` covers each case.
- **A missing Blender is a named error** (verified round 3): the viewer's
  preview reports `blenderMissing` once until the shading changes
  (`present::blender` `a_missing_blender_is_reported_once_until_the_shading_changes`),
  a capture fails with `blenderMissing` (`pm-node` `tests/snapshot.rs`
  `blender_compositions_are_captured_as_today`), and main's stage rejects
  with `blenderMissing` and today's message (`blender.test.ts`).
- **Level 2 surfaces stay 16-bit PNG** (decision 15); level 1 retains
  PNG8/dither 1 for existing-project parity. RFC 11 open question 1 (raw
  or EXR for level 2) is unchanged.

## Phase 3c round 4: imported-model snapshots

Imported glTF snapshots now use the CPU pose kernel for positions, including
relative morph targets and skinning. Normals follow today's snapshot exactly:
normalize the attribute, or apply the bone transform to `normal + position`
and subtract the posed point before normalizing. Instance transforms are not
expanded, matching today's `renderSnapshot` traversal. The renderer's material
state supplies the source colors, factors, and maps. The material snapshots
resume empty-map awaits before new PNG conversions, preserving the captured
texture ids and mesh completion order. Embedded images follow today's
straight bitmap → premultiplied canvas → straight PNG conversion.

Measured on the retained v1 and threed map captures: 182 built, 176 equal,
23 with converted embedded textures equal by decoded pixels; all 90 frame
lists equal. Across 14,118,112 geometry values the maximum absolute difference
was 3.47e-16. All 23 previously unbuilt glTF snapshots now compare equal.
The six snapshots with external converted TIFF/AVIF maps still fail; this
change does not close that separate blocker. The same focused comparison with
`PM_BLENDER_ONLY=3d/gltf` passes. The full comparison continues to fail honestly.

### TIFF conversion and the remaining AVIF differences

Round 4 also preserves the full TIFF rendition samples, including 16-bit
precision and straight alpha; all six captured TIFF maps now match exactly.
The comparison decodes RGBA, RGB, gray, and gray-alpha PNGs at both 8 and
16 bits, compares dimensions, and compares samples at 16-bit precision.
Two unhandled layouts or failed decodes can no longer compare equal.
`PM_BLENDER_DIFFS` optionally writes differing texture pairs outside Git.

The full comparison remains 176/182 equal: each of the six map snapshots
still contains AVIF differences. The five distinct AVIF maps are PQ HDR
(max 174 CV), HLG HDR (117 CV), HDR alpha (210 CV), P3 SDR (1 CV on 66
pixels), and BT.2020 SDR (1 CV on 29 pixels). The previous round's prose
misattributed the PQ failure to a 16-bit TIFF: asset `a1xwvwot` is
`hdr-pq.avif`. The full-depth TIFF comparisons now pass independently.


## Round 5 synchronized app verification

The final engine-on app run passes 25/25 tests, including all 19 required
3D checks, the explicit preview API, live AVIF/SVG PNG conversion, paired
chrome pixels, exact OBJ bundle imports and textured Object3D compatibility.
No legacy preview jobs run in the native preview or map-transport checks.
The converted-PNG caches in the renderer and native process each retain at
most 256 MiB of successful bytes by SHA-256; failures retry and concurrent
requests share one conversion. Snapshot-construction failures are reported
to explicit and automatic preview listeners instead of becoming an empty
Ready frame. Permanent GPU, descriptor, SVG and pool failures also end the
matching generation; temporary pending resources still retry.

The follow-up editor-camera restart check passes after the synchronized
25-test run. The flag-off Blender motion suite also passes 3/3 with the
original WebGL capture and legacy worker-manifest assertions retained.

## Phase 3c round 6: reproducible Blender corpus inputs

AVIF and SVG maps reach Blender as the PNG the editor window's Chromium draws
(decision THREED-2), so `tests/blender_snapshots.rs` needs those PNGs as
inputs. They are corpus data, kept outside Git beside the captures:

- **Where:** `~/Library/Application Support/Powermove Engine Corpus/blender-chromium-pngs/`,
  one `<sha256 of the original file>.png` per source, with the
  `<sha256>.json` need (content hash and source path) that produced it.
  `PM_BLENDER_CHROMIUM_PNGS` overrides the directory. The directory sits at
  the corpus root, not inside a capture, so each capture's own file list
  still verifies with no unlisted files.
- **Generate:** run the comparison once. A missing PNG fails the test, which
  writes a need for every AVIF and SVG still of the failing project and
  names the command. Then, under the Electron lock:

  ```sh
  ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
    bun engine/crates/pm-render/tools/blender-png/run.ts \
    --needs "$HOME/Library/Application Support/Powermove Engine Corpus/blender-chromium-pngs"
  ```

  `run.ts` checks each source's hash, converts it in a hidden Electron
  window through the app's own `BlenderPngs` producer
  (`engine-connection/blender-png.ts`), and writes `<sha256>.png`. It
  refuses a directory inside the repository and never reads the capture's
  expected PNGs.
- **Test:** `cargo test -p pm-render --test blender_snapshots` runs both
  tests with no environment variables. The TIFF full-depth test
  (`tiff_renditions_equal_todays_blender_textures_at_full_depth`) now builds
  `maps-level1` with the same Chromium PNG resolver as the full comparison;
  round 5 left it on the plain resolver, so it failed on `hdr-pq.avif`.

Measured 2026-10-19: both tests pass with no environment set: 182 compared,
182 equal (29 converted textures equal by decoded pixels), 90 frame lists
equal, geometry max |Δ| 3.5e-16, 15,070 exact values. Regenerating the five
AVIF inputs from scratch into an empty directory with the command above
produced five PNGs byte-identical to the stored ones (`cmp`), so the stored
inputs are reproducible from the original media.
