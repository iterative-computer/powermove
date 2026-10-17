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
  static check lists still routes (imported models, recipe parts, and the
  native pass's own limits such as texture maps with transparency).

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

`BlenderRender::Preview` draws the preview surfaces a `PreviewSurfaces`
host has for each composition's preview snapshot (preview samples and
scale; EEVEE for Material Preview; drawn stretched over the frame as
today) and reports the snapshots it lacks, drawing natively meanwhile.
The interactive engine's presenter does not pass the view override or the
paused state into its renders yet (`compose.rs` renders with `view: None`),
so the viewer's Material Preview and Rendered shading still come from the
current engine; see Gaps.

## Tests and measurements

Apple M4 Max, macOS 27, Blender 4.5.14 LTS (`/Applications/Blender.app`),
wgpu 30.0.1 (Metal), corpus `v1-20261012-230504-ae47a3b3`, 2026-10-17.

| Test | What | Result |
|---|---|---|
| `pm-render` `tests/blender_snapshots.rs` | Every Blender snapshot of the corpus (final, preview, export, exportAlpha; every shutter sample; nested compositions) against today's `renderSnapshot`; every frame list against `blenderFrameRequests` | 158 snapshots: 135 equal (geometry: 11,274,896 values, max abs difference 3.5e-16; 9,622 other values exact; asset ids, names, and SHA-256 equal), 23 not built (`3d/gltf`, `3d/gltf-materials`: imported models, the glTF lane); 74 of 74 frame lists equal (times bit for bit) |
| `pm-render` `tests/blender_composite.rs` | Two layers of 16-bit surfaces with gamma and sRGB chunks, off-center crops, 50% opacity, in plan order, against an f64 source-over reference | 32-bit: max 6.8e-8; 16-bit: max 1.06e-3 (half-float storage and accumulation); missing surfaces fail each layer with today's sentence; preview draws what it has and asks for the rest; nested compositions are their own frames |
| `pm-export` `tests/blender_export.rs` | A 2-frame 16-bit PNG export of two models on the installed Blender | One Blender request per frame (layers batched), 4 surfaces, device `Metal {Apple M4 Max (GPU - 40 cores)}`; every exported sample equals the f64 composite of the stored surfaces (max 0 of 65,535); re-export: 0 Blender jobs, 4 cache hits (0.85 s against 0.03 s) |
| same | Batched against one invocation per layer (EEVEE), and two batched runs | Decoded surfaces equal; files byte-identical; no text chunks |
| same | Cancel 1.5 s into a Cycles CPU render (1920 × 1080, 4,096 samples) | `Cancelled`, the worker's process group is gone |
| same | No Blender; a backend that drops a surface | `blenderMissing` with appendix D's sentence; `blenderFailed`; nothing written; without a Blender stage the job still routes |
| `pm-blender` unit tests | Settings parse, canonical hashing, key inputs (every input changes the key, asset order does not), surface decoding and crops, cache bounds and disk, device record, discovery, flags, the stage's per-key rendering and failures | Pass |
| `apps/desktop` `src/main/engine/blender.test.ts` | Main's supervision with a stand-in executable: flags, scrubbed environment, surfaces before the answer, batching off, worker-script hash, foreign handles, missing Blender, cancel kills the process, playback stops and continues the process (`ps` state `T`) and holds new jobs | Pass |
| same | Cycles with `device: cpu` and `auto` | Recorded per job: `Cpu`, then `Metal {Apple M4 Max (GPU - 40 cores)}` |
| `apps/desktop` `e2e/engine-blender.spec.ts` (under the Electron lock) | The app, 192 × 108, a turning box and a sphere, EEVEE 8 samples: today's still (Use New Engine off, today's surfaces recorded at main's IPC), then the job engine's still and a 6-frame H.264 MP4 (on) | The MP4 is the job engine's (AVAssetWriter, 6 frames); 12 surfaces in the cache (2 layers × 6 frames; the still reused frame 0's); the job engine's still is 100% within 2 CV of today's surfaces composited per layer (max 2 CV) |

## Found along the way

- **Today's still export draws Blender surfaces off by up to a pixel.**
  Against today's own Blender surfaces of the same frame (recorded at
  main's IPC) composited per layer, the current exporter's still is 97.7%
  within 2 CV with a maximum of 207 CV: model edges move by a pixel and
  read as aliased, while the job engine's still is 100% within 2 CV (the
  8-bit dither of today's PNGs). Today's worker and the pinned worker
  cover the same pixels (same bounding boxes; the 16-bit surfaces keep
  faint edge pixels the 8-bit ones round to 0). Candidate for appendix A;
  the engine does not reproduce it.

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
  the job engine's `cancelJobs`.

## Gaps

- **Imported glTF models and recipe parts** are not built into snapshots;
  their compositions route (the static check lists them). The comparison
  harness lists the glTF corpus snapshots as not built.
- **The paused viewer's Blender shading** (Material Preview and Rendered)
  is not connected in the interactive engine: the presenter renders
  without the view override or the paused state, and the interactive
  engine has no Blender path to host services. The renderer side
  (`BlenderRender::Preview`) is built and tested.
- **Agent frames** (`render_frames`) of Blender compositions route to
  today's capture (contract 12's `cachedOrPending` and `render` policies are
  not implemented in the job engine's still path).
- **Motion blur** is routed by pm-render; the stage handles shutter samples
  once the plan visits them, and the frame-list port covers them.
- **Leases**: the job lane's decoded surfaces use a fixed 512 MiB budget,
  not the `blenderSurfaces` lease category; the disk area's bound is 8 GiB.
- **Native-pass limits on Blender compositions**: the static check still
  routes Blender compositions for limits only the native pass has (texture
  maps with transparency, image formats the native pass cannot draw).
- **Surfaces stay 16-bit PNG** (decision 15); RFC 11 open question 1 (raw
  or EXR) is unchanged.
