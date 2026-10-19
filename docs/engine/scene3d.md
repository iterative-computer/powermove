# The native 3D scene pass: design notes

The Phase 3a 3D lane (plan section 8 "3D layers" and "Depth and ID
surface", section 9 "3D layers", section 10's `layer3d`, `scene3d`, and
mesh rows, section 25's 3D class, amendment 11). Code:
`engine/crates/pm-render/src/scene3d/` (the pass),
`engine/crates/pm-render/src/three/` (three.js r180's program generation,
material parameters, math, sampler specialization, program cache), and
`engine/crates/pm-geom3d` (geometry, OBJ, depth sets and ordering). It
extends the Phase 2 primitive (docs/engine/pm-render.md, "The 3D scene
pass") into the full pass. The plan stays the authority.

## What it draws

| Feature | Status | Evidence |
|---|---|---|
| Any number of 3D object layers per composition, depth sets and ordering exactly as section 8 states (inactive layers, lights, cameras, and plain group headers do not end a set; 2D layers, 3D nulls, and styled groups do; opacity ≤ 0.001 and solo exclusions; runs sorted by the camera depth of the object origin, blocks never reordered) | Drawn | `tests/scene3d_layering.rs` (80 compositions against today's TypeScript); `pm-geom3d` `layering` tests; `tests/scene3d_document.rs`; sweep `depth-sets`, `layer-opacity-and-mask`; corpus `3d/solo-hidden`, `3d/lights`, `3d/primitives` |
| The shared depth prepass of a set (depth writes only at layer opacity ≥ 0.999 without an enabled mask), recorded once and reused by the set's other layers | Drawn | Sweep (prepass draws matched to three.js's, cached outputs issue only color draws) |
| The set's bottom layer clears to the solid background (sRGB-encoded, opaque, not tone mapped) | Drawn | Sweep (every case with a background) |
| Shading: exposure, ACES Filmic with r180's `exposure / 0.6`, E in every fragment shader; blending, back-then-front double-sided transparency, 4× MSAA resolve on E values | Drawn | Sweep `transparency`, `hemisphere-exposure`, `look-xray` |
| `MeshStandardMaterial` with color, normal (tangent space, screen derivatives), roughness, metalness, emissive, and AO maps; UV placement (`setUvTransform` of offset, repeat, rotation); slots over geometry groups; node-material previews (`materialPreview`); source materials (`new MeshStandardMaterial()`) | Drawn | Sweep `maps-placement`, `slots-and-groups`, `node-material-and-source`; corpus `3d/mesh-slots-maps` |
| Hemisphere ambient (ground 0.35 × sky), sun, point, spot, rectangular area lights (LTC tables, no native shadows) | Drawn | Sweep `area-light`, `point-*`, `spot-*`; corpus `3d/lights` |
| Shadows reproducing r180: PCF-soft, 1024² maps (4096 × 2048 for the point cube atlas, 6 viewports), bias −0.0002, normal bias 0.025, sun frustum ±10 near 0.01 far 200, spot field of view from the cone angle near 0.5 far `distance \|\| 500`, RGBA8 packed depth compared with nearest fetches, casters the set's visible meshes with `castShadow`, the first 4 casting lights in layer order, three.js's frustum culling | Drawn | Sweep `sun-shadow*`, `point-shadow`, `spot-shadow`, `five-casting-lights`, `invisible-casting-light`, `shadows-off` |
| Perspective and orthographic cameras (half-height 3 / zoom), roll from the camera layer's world up | Drawn | Sweep `orthographic-*`, `camera-roll-fov`; corpus `3d/camera-*` |
| Every source kind: the eight primitives, lathe, extrude (with bevel), mesh sources (index, normals computed when absent, UVs optional, groups), OBJ model assets | Drawn | `pm-geom3d` 34 sources and 5 OBJ files bit-identical to three.js and `parseObj`; sweep `primitives`, `lathe-extrude-mesh`; corpus `3d/lathe-extrude` |
| Missing models: the orange wireframe box (`MeshBasicMaterial`, wireframe lines, tone mapped), casting wireframe shadows | Drawn | Sweep `missing-model`; corpus `3d/missing-model` |
| Editor looks under the view override only: X-ray (opacity × 0.45, transparent, no depth writes; draft shadings only), Wireframe (1-pixel edges over the wireframe index, shadows included), Local View, the editor camera | Drawn | Sweep `look-*`; `tests/scene3d_document.rs` `view_overrides_change_only_viewer_renders` |
| Legacy `scene3d` layers: the layer's own scene (its hierarchy, visibility, `o.<id>.` channels) rendered whole at most 4,096 pixels a side with its own background, drawn through the layer's 2D transform | Drawn | Sweep `legacy-scene`; corpus `3d/legacy-scene` |
| Legacy OBJ mesh layers: the compositor's fixed shading into a half-float target with a 16-bit depth buffer, the checkerboard without a mesh | Drawn: the built-in OBJ model layer (`powermove.3d.obj-model`) always, third-party mesh definitions given their snapshot (`Renderer::set_mesh_definitions`) | `tests/scene3d_mesh.rs` (4 cases against the compositor's own shader); corpus `layers/obj-model` exact |
| The depth and id surface for every 3D object layer and legacy scene layer: linear view depth and id, upright, 3D inside precompositions and group boundaries included. Legacy OBJ mesh layers report coverage and id only (no depth: their fixed-shading camera has no place in the composition's 3D space) | Drawn | `tests/scene3d_document.rs` (top level and nested, off center); corpus 3D masks |
| 3D output written per layer into the 2D graph (masks, effects, opacity, blend stay per layer) | Drawn | Unchanged composite path; corpus |
| Linear blending: the resolved, premultiplied, E-encoded canvas unpremultiplied (guarded), decoded with E⁻¹, and re-premultiplied once, where the compositor draws it (`pm_premultiplied_e_to_linear` in `composite.wgsl`, the compositing lane's) | Drawn when the working encoding is linear light | `linear_blending_decodes_the_3d_canvas_once` (an opaque 3D layer equals its linear-blending-off frame within 1e-4 at 32-bit; a second decode fails it by 0.29) |
| Imported glTF scenes, recipe parts, Blender surfaces, 2.5D (`threeD` on non-3D layers), Studio Cube | Routed (other lanes) | `unsupported` |

## How a frame draws 3D

1. **Plan** (`plan.rs` → `scene3d::layers`). Each compositing pass calls
   `order_pass`: Local View filters object layers (root composition, view
   override only); a pass without 3D roles keeps `depthOrderedLayers`;
   otherwise the composition's scene is built once (shared with its group
   passes, keyed by composition, time, and size) from `pm-eval`'s
   `compositionScene`, placed as `compositionRuntime` places it (objects at
   `world3D`, lights at world positions aiming through their parent space,
   the topmost active camera layer's pose with its world Y as up), then 3D
   runs are sorted and every object layer gets its depth set and whether it
   is the set's bottom. `plan_layer` hands the layer a `Render3d::Layer`.
   Legacy scene layers (`powermove.3d.scene`) and mesh layers with a known
   definition are planned by `scene3d::legacy::plan_extension`.
2. **Describe** (`scene3d::describe`). The draws three.js issues, as data:
   shadow passes (per casting light, per viewport, the set's casters after
   frustum culling), the prepass, the color pass (opaque items in slot
   order, then transparent items, double-sided ones back face first), each
   with its program parameters (`three::materials`), uniform values
   (`WebGLMaterials`, `WebGLLights` with its shadow-casting-first stable
   sort, camera and object matrices), textures, and fixed-function state.
   A legacy scene renders whole (`whole_scene`: transparent items sorted by
   the clip depth of their bounding-sphere centers).
3. **Execute** (`scene3d::gpu`). Programs are generated byte for byte
   (`three::program`), preprocessed, sampler-taking functions specialized
   per call (`three::specialize`; naga cannot take sampler parameters and
   the opaque inliner is not linked), compiled through `pm_gpu::glsl`,
   their texture bindings renumbered compactly (wgpu allows at most 1,000),
   written as MSL with Metal's safe math mode (amendment 11), and cached on
   disk with their reflection. The prepass is recorded when the scene or the
   set changes; the color pass loads its depth and resolves 4× MSAA into an
   RGBA8 target in GL row order. The depth surface pass writes linear view
   depth.
4. **Composite.** The executor draws the output as a screen-space quad (a
   legacy scene as a quad through the layer transform); with linear
   blending on, that draw decodes it (the compositing lane's quad shader).

## Fidelity notes

- **Programs.** Every program the sweep draws equals the program WebGL
  received, except where three.js's program cache makes the text
  history-dependent: `SHADER_NAME` (materials whose parameters agree share
  the first one's program), and the shadow pass's shared depth and distance
  materials, which keep the side defines, light counts, and map transform
  of their last rebuild (three.js rebuilds them only on a program-relevant
  change; their fragments pack depth only). The pass generates the fresh
  program in those cases; the pixels cannot differ.
- **Shadow maps** are recorded in the prepass, as `shadowMap.autoUpdate`
  is on only there: a layer's color pass sees its own set's casters.
- **Textures** are today's level 1 content: the runtime's texture getter
  hands three.js the asset's `ImageBitmap`, the bitmap image layers draw,
  and WebGL ignores every unpack flag for an `ImageBitmap`. Maps therefore
  decode through pm-render's level 1 still-compatibility bitmap
  (`media::level1_bitmap`: PQ/HLG/cHRM/cICP PNG, CMYK JPEG, the TIFF
  structures, the AVIF classes, Chromium's gamma and ICC readings) and
  keep Chromium's premultiplied bytes, which three.js samples as straight
  alpha. Deep and arithmetic JPEGs have no 8-bit bitmap: the importer keeps
  an image element whose `texSubImage2D` fails, so the storage stays zeroed
  (transparent black). Rows upload top row first, sRGB maps as
  `Rgba8UnormSrgb`, and mipmaps are box filters (in linear light for sRGB),
  equal to Chromium's `generateMipmap` (Phase 3c round 2, below). Probes
  are cached per file version (`Textures::probe`). Nothing a level 1 image
  layer draws is an unsupported map; SVG maps (an image element rasterized
  by Chromium) are the one map class still listed.
- **Missing textures** draw without the map and add `Missing texture: <id>`
  to `Scene3d::warnings`, as today's texture getter returns nothing for an
  image without a bitmap. That covers an image asset that is offline (the
  resolver may still name a path), does not decode, or vanishes between
  the plan and the render (the render drops it from its copy of the scene).
  `Error3d::Failed` is left for GPU or program failures, which fail the
  layer. The warnings have no channel to the host yet (today the compositor
  shows them through `setShaderMeta`).
- **GPU resources** follow the scenes being drawn, as today's runtime
  disposes textures its scene no longer references. Each frame
  (`Renderer::render_into` calls `Scene3dRenderer::begin_frame`), the first
  3D render releases the meshes, maps, and legacy mesh-layer buffers that
  neither it nor the previous 3D frame drew (`KEEP_FRAMES` = 2, so a viewer
  alternating two frames does not re-upload). `Renderer::scene3d_resident`
  reports the counts and bytes.
- **Geometry keys.** Scene sources are keyed by a hash of their parameters
  and point lists written as raw bits (`Source::write_key`, not formatted
  text). OBJ model geometry is keyed by its file version (path, size,
  modification time; `legacy::obj_mesh_versioned`), never by an address, so
  a re-saved or newly parsed file cannot reuse another file's GPU buffers.
  Built geometry (OBJ triangles included) is shared across scenes and
  frames. `Geometry3d`'s debug form names the key, not the vertices, so
  content keys stay cheap.
- **`doubleSided`**: `compositionScene` returns `parseScene`'s output, so a
  3D object layer's material without `doubleSided` is double-sided (the
  schema default; `pm-eval` fills it since Phase 3c). A legacy scene layer's
  raw objects are read as stored (absent means front side).
- **Depth and id surface.** Coverage is the layer's own alpha (≥ 0.5/255);
  depth comes from a single-sampled pass, so MSAA-only edge pixels report
  +inf. Inside a precomposition the depth is the nested camera's; a legacy
  scene layer reports its own camera's depth; a legacy OBJ mesh layer
  reports coverage only.
- **Lights in the pass order.** The engine sorts light and camera layers
  at depth 0 within a 3D run, as section 8 states. Today sorts lights by
  their world position (`runtime.objects` holds lights, so
  `compositionOrderedLayers` finds them). Lights draw nothing and never end
  a depth set, and a stable sort keeps the objects' relative order, so
  object order and set bottoms are unchanged (`tests/scene3d_layering.rs`
  compares everything but lights and cameras).
- **Spot and point shadow cameras** use `far = distance || 500`. three.js
  r180 computes `light.distance || camera.far`, and `camera.far` keeps the
  last non-zero distance, so today's long-lived `SceneRuntime` differs from
  a fresh one after a light's distance is animated through 0. The engine
  renders each frame from the document alone (a viewer frame equals the
  export frame); this history dependence is not reproduced.
- **Content keys.** `Object3dPlan::content_key` hashes everything a render
  depends on, map files by size and modification time included.

## Extension points (glTF lane)

- `model::ModelSource` resolves `assetId`: `ModelLookup::Imported` is the
  glTF lane's (routed today); add a variant carrying the loader's meshes.
- An `Object3d` holds a list of `Mesh3d`s with their own world matrices and
  `Materials`; `MaterialKind` and `three::materials` take new material
  classes (`MeshPhysicalMaterial` parameters are all in
  `program::Parameters`).
- `three::specialize` already handles every sampler-taking function in the
  r180 chunks (`bicubic`, `textureBicubic`, `textureCubeUV`), and
  `describe` passes `TextureRef`s for any sampler a program declares.
- The transmission pass needs its own target in `gpu::record`; points and
  lines need topology in `DrawState` (lines exist for wireframes).

## Integration notes

Legacy `powermove.3d.scene` animation channels are hydrated on project open,
including nested compositions. Previously the legacy renderer adopted their
temporal-ease fields only when it evaluated a frame; the native renderer did
not write them, so switching off could change a saved document depending on
which frames had drawn. Hydration adopts the scene's existing linear defaults
before general keyframe normalization, preserving old handles and numeric
interpolation. The regression in `legacy/app.test.ts` checks all scene channel
groups, nested scenes, quarter/midpoint values, evaluation without a document
write, and idempotent reopen. It fails before this fix.

- The view override travels with each render: `RenderRequest::view`
  (contract 2's `FrameRequest.view`). `RenderRequest::from_frame_request`
  carries it for viewer frames and drops it for exports and agent captures
  (`FramePurpose::is_final`; contract 2 already refuses a final request
  that carries one), and an export request (`RenderRequest::export`) never
  draws one. Nothing in the renderer remembers a view, so an agent capture
  or thumbnail rendered after a viewer frame draws the document's own
  scene. (`Renderer::set_view` and `from_frame_request_with_view` were
  removed; `RenderRequest` is `Clone`, no longer `Copy`.)
- `Renderer::set_mesh_definitions` registers mesh-layer definition
  snapshots (`assetField`, parameter defaults). The built-in OBJ model
  layer is known without one. The static capability check cannot see
  third-party snapshots and lists those layers until snapshots reach the
  engine.
- `unsupported` lists, for 3D: imported glTF models, recipe parts, unknown
  primitives, unknown light types, map images outside the drawn still
  classes (probed when the media can be opened, by the recorded container
  otherwise, as for image layers), SVG maps, model groups, and Blender
  rendering. The objects and lights inside a legacy scene layer meet the
  same checks (`scene3d::check_legacy_scene`), so a routing decision holds
  at every frame (`the_static_check_looks_inside_legacy_scenes`: every
  listed change that the pass cannot build also fails at render time). A
  legacy scene object's `generation` is listed conservatively although the
  legacy runtime ignores it.

## Tests and measurements

Apple M4 Max, macOS 27, wgpu 30.0.1 (Metal, safe math), Chromium 152 with
ANGLE on Metal and three.js 0.180.0 for references, 2026-10-15.

| Test | What | Result |
|---|---|---|
| `pm-geom3d` `tests/three_geometry.rs` | 34 sources (every primitive with groups, capsule, icosahedron at detail 0 to 5, lathe, extrude with and without bevel, mesh sources) against three.js; 5 OBJ files against `parseObj` in V8 | Buffers, indices, and groups bit-identical |
| `pm-render` `tests/scene3d.rs` | 31 scenes, 74 outputs against three.js renders through the production `SceneRuntime` (`tools/three-ref/run-scenes.ts`), section 2's 3D bar | All within the bar: every output 100% within 2 CV, max 1 CV everywhere, SSIM ≥ 0.99984 |
| `tests/scene3d.rs` | Programs and uniforms against the recorded draws | 265 draws matched, in three.js's order; 23,831 uniform values compared, all equal as f32 (the test requires bit equality; samplers and the shadow materials' stale `mapTransform` are left to the pixel sweep); every recorded draw that writes color or depth matched |
| `tests/scene3d_layering.rs` | 80 seeded compositions (objects, 2D solids, 3D nulls, lights, cameras, plain groups, inactive and invisible layers, solo) through today's compositing pass, `compositionOrderedLayers`, and `compositionDepthIds` under bun (`tools/layering/generate.ts`) | 208 object draws: depth sets and set bottoms (103) equal; draw order equal (180 adjacent pairs) |
| `tests/scene3d_mesh.rs` | Legacy mesh layers (3 OBJ meshes, the checkerboard) against the compositor's shader in WebGL | Max 1 CV |
| `tests/scene3d_document.rs` | Ordering, depth sets, depth and id surface (upright, also off center in precomps), view overrides per request (exports ignore them, the next render without one is unchanged, final purposes never carry one), OBJ models, mesh layers, linear blending end to end, the static check inside legacy scenes | Pass |
| `tests/scene3d_resources.rs` | Maps with transparency routed by the static check and by a render; offline and undecodable maps draw as no map with the warning; a map gaining transparency after the plan routes (`Error3d::Unsupported`), one vanishing draws as no map; a re-saved OBJ draws its new geometry, as a fresh renderer does; meshes and maps of 12 frames of edited geometry and new map files stay at most 2 each, and drop to what is drawn | Pass |
| `tests/corpus.rs` | Every 3D corpus project the renderer supports (`v1-20261012-230504`, `render-20261014-131536`) | `PM_RENDER_ONLY=3d/`: 23 frames, 0 failures (`3d/depth-sets` now compares, its shape layers drawn since the vector lane merged); `layers/obj-model` 3 frames exact; the full corpus 228 frames, 0 failures (2026-10-17, re-run after merging wave 4: effects, text, motion) |
| Program cache | Cold and warm first frame of `five-casting-lights` (debug build) | 196 ms translating, 6 ms from the disk cache (2026-10-16; earlier 343 ms and 18 ms on a busier machine) |
| `frame_time_3d_shadows_at_1080p` (ignored, release) | 7 layers, 3 casting lights (sun, spot, point atlas), 1920 × 1080 | 2026-10-16 after the review fixes (geometry keyed by raw bits instead of formatted text, maps resolved once per scene, eviction): median 4.34 ms, p95 5.70 ms (CPU plan and GPU) at a load average of about 9. Earlier: median 9.7 ms, p95 13.9 ms; 26 to 36 ms median at a load average of about 30. Not gated here; idle and base machines not measured |

## Gaps

Current as of Phase 3c round 7 (2026-10-19); the round sections below are
history.

- Nothing about a 3D layer routes. `Feature::Scene3d` no longer exists:
  imported models draw or are today's missing-model box (`gltf.md`,
  "Acceptance"), a scene the schema refuses draws as today (below,
  "Corrupted scenes"), and a material map's still has the level 1 image
  layer's verdict, feature for feature (`threed_maps.rs`). What an image
  layer of the same still reports is the stills lane's to close.
- Shared-parent glTF nodes follow GLTFLoader's promise resolution order;
  image-dependent ordering uses the already-decoded-image policy described
  in `gltf.md`, "Acceptance".
- Native composition and legacy scene boundaries validate the same strict
  keys, ranges, source and mesh refinements, material graphs, IDs, parenting,
  and geometry budget as `parseScene`; invalid composition scenes fail their
  3D pass, while invalid legacy scenes draw the missing-definition checkerboard.
  Native picks and bounds refuse the same corruption. The four limits that
  ordinary editing can exceed (512 objects, 16 lights, 160-character display
  names, and 8 MB of serialized scene JSON) are accepted by the native engine.
  For legacy queries, the original parser exception is returned for other
  invalid scenes.
- Blender snapshots of glTF primitives that WebGL refuses to draw (an
  index past the last vertex is written as null as today; a vertex
  attribute shorter than its positions is read as zeros where today reads
  `undefined`) are not compared against a capture.
- Scene warnings (`Missing texture`, `Missing model`) travel with native
  frame descriptors to `uiState.getShaderMeta().sceneWarnings`; diagnostics
  update when the matching frame is presented, including nested scenes.
- 3D output stays 8-bit as today's canvas, renders at the full output size
  for a region of interest, and has no render cache yet (the content key
  is ready for one).
- Program text differences that three.js's cache makes history-dependent
  (above) are reproduced as fresh programs, not as history.
- The paired post-D 3D orbit row (pointer to pixel) is unmeasured (round
  5's attempt ran with the display asleep); it moves to the Phase 3c
  measurement window.

## Changes outside the lane

The lane owns `pm-render/src/scene3d/`, `pm-geom3d`, and this note; the
tests and tools named above are its own files. Everything else it touched:

- `pm-gpu/src/glsl.rs`: a public `preprocess` (the existing glslang step).
- `pm-render/src/three/`: `materials.rs` (`getParameters` of the standard,
  basic, depth, and distance materials), `specialize.rs` (per-call
  specialization of sampler-taking functions), `cache.rs` (the program
  cache stores texture bindings; format 2, older entries are misses),
  `mod.rs` (the modules).
- `pm-render/src/plan.rs`: `Build` carries the passes' 3D state and
  options; `build_with` (3D options next to the compositing lane's effect
  host, merged with `build_with_effects`); `comp_pass` orders the pass; the
  extension branch plans legacy kinds.
- `pm-render/src/renderer.rs`: `set_mesh_definitions`, the 3D options of
  each request passed to the plan, `scene3d_resident`, and
  `scene3d.begin_frame()` next to the vector cache's.
- `pm-render/src/request.rs`: `RenderRequest::view` (no longer `Copy`);
  `from_frame_request` carries the view of non-final requests.
- `pm-render/src/exec.rs`: nested depth for precompositions in the depth
  and id surface; `Error3d::Unsupported` becomes
  `MediaFailure::Unsupported`. (The scene pass's own linear conversion was
  removed when the compositing lane's decode of 3D output merged.)
- `RenderRequest` lost `Copy`, and literals gained `view: None`:
  `pm-export/src/render.rs` (`request.clone()`, `view: None`),
  `pm-export/tests/job_export.rs`, `pm-node/src/present/compose.rs`,
  `pm-cli/src/render_parity.rs`, `pm-render/tests/corpus.rs`, and
  `pm-render/tests/headless.rs` (one line each).
- `pm-render/src/capability.rs`: legacy scene layers checked inside
  (`check_legacy_scene`), the built-in OBJ model layer accepted; the 3D
  check takes the asset resolver.
- `pm-render/src/shaders/id.wgsl`: 3D layers read upright (a fix).
- `pm-render/src/scene3d.rs` (the Phase 2 primitive) became the
  `scene3d/` module.
- `pm-render/tests/headless.rs`: the capability test's 3D section.
- `docs/engine/pm-render.md`: its 3D section points here.
- `engine/Cargo.lock`: `pm-geom3d` in `pm-render`'s dependencies.


## Hydration follow-up (K1 and R3b, 2026-10-18)

Registered effect parameters still go through both hydration passes (the loose
pass, then the definition's pass), with today's value and keyframe semantics.
Before its second pass each channel releases its own key ids from the
project-wide set, so saved numeric, color and boolean keys keep their ids on
every reopen. Duplicate ids across different channels still go through the
existing uniqueness check. See `gate-fixes.md` (K1) for the parity cases.

The legacy-scene hydration regression now includes color and boolean tracks and
keys without ids in root and nested compositions. It compares quarter/midpoint
answers with the unhydrated legacy evaluator, retains the absolute numeric
22.5/45 assertions, checks assigned ids, and requires unchanged JSON after
evaluation and a second hydration. Removing scene adoption fails the expanded
regression; no easing behavior or renderer tolerance changed.

## Phase 3c: 3D questions and model groups on the engine (3D lane, 2026-10-18)

Plan section 8, appendix C rows C9, C10 and C14 (model groups), amendments
21 and 22. Code: `engine/crates/pm-geom3d/src/query.rs`,
`engine/crates/pm-eval/src/query3d.rs` and `recipe.rs`,
`engine/crates/pm-wasm/src/geometry.rs`,
`apps/desktop/src/renderer/src/core/scene3d/engine-geometry.ts`.

### What changed

- **pm-wasm answers every 3D question the mirror used to refuse.** The
  `RefuseHost` is gone. `pm_eval::bounds::LayerHost` no longer has the five
  three.js hooks; it offers `geometry()` (`pm_eval::query3d::Geometry3d`:
  model assets, the editor camera, caches), and pm-eval answers:
  - `GL.pick` on 3D object layers: `compositionRuntime(...).pick(ndc,
    compositionDepthIds, viewportCamera)`, with Local View;
  - `GL.bounds` on 3D object layers: `runtime.bounds(proj.w, proj.h, [id],
    camera)` (the camera resized to the project, as today);
  - `groups.bounds` / `GL.bounds` on model groups: `modelGroupBounds`,
    including `z0` and `z1` (`Bounds::z`), and the native `groupPlacement`
    edit writes the group's `anchor.z` / `position.z` for 3D groups as
    today;
  - extension scenes without a background (legacy `powermove.3d.scene`
    layers and mesh definition layers): `sceneRuntime(...).bounds` and
    `.pick`, with `parseScene`'s refusals of `sceneForLayer`'s scene (an
    empty background throws a `ZodError` today and here).
- **Geometry and semantics** (`pm_geom3d::query`): `Mesh.raycast` step by
  step (bounding sphere unless it contains the origin, local bounding box,
  draw range or per-group triangles with each group's material side, a
  group without a material throws three.js's `TypeError`, an array
  material on a mesh without groups finds nothing), the closest hit whose
  object passes `pick`'s filter (visible chain, depth-set ids, not every
  material at opacity 0); `Raycaster.setFromCamera` for both projections;
  `worldBounds` (visible meshes' geometry boxes, lights, points, and lines
  excluded), `Vector3.project`, and `Box3.setFromObject` (posed skinned
  vertices, instances, points and lines included). Imported glTF models
  are picked by the glTF lane's kernel (`gltf::pick::Picker`, 7,584 rays
  equal to three.js) under `sync`'s material rule. Primitive, lathe,
  extrude and mesh sources come from the bit-identical generators; OBJ
  models from `obj::parse_obj`; the missing-model box is `BoxGeometry(1, 1,
  1)`. Triangle meshes keep a BVH (built once per geometry and per source
  key in the mirror).
- **Model assets reach the mirror on demand.** A question that meets an
  asset it lacks answers `{"models": [ids]}`; the renderer adds each one
  from its asset store as `SceneRuntime.makeObject` reads it (a glTF
  model's source bytes, kept by `loadGltf` per `object3d`; an OBJ model's
  text; otherwise "missing", the orange box) and asks again
  (`resolveModels`). A record that changes (relink, reload, removal) is
  forgotten and fetched again (`refreshModels`). New ABI: `pm_model_set`,
  `pm_model_forget`, `pm_models_clear`.
- **The editor camera and Local View travel with each question**
  (`geometryRequest`: `matrixWorld`, `matrixWorldInverse`,
  `projectionMatrix` and its inverse exactly as the renderer holds them,
  and the lens for questions that resize it). `editorCamera(PM)` gives the
  camera without building the composition's three.js runtime.
- **The three.js answer fallback for 3D picks is removed**
  (`engine-doc/native.ts` no longer returns `fallback: '3d'`); 3D bounds,
  group bounds, and geometry-dependent edits (`edits.ts`) carry the
  geometry request too.
- **Animated model-group recipes draw natively.** `animatedPart` and the
  recipe geometry (`recipeParts`) moved to `pm_eval::recipe`:
  `compositionScene` replaces a part's source with the recipe's geometry for
  the evaluated parameters and moves its transform channels by the new base
  minus the stored one; `compositionRuntime`'s parent-space offset is
  `Scope::recipe_world_offset`, used by the render placement and the 3D
  questions. Staircase steps are `BoxGeometry` through `rotateY`
  (positions through the matrix, normals through the normal matrix and
  normalized, stored as f32), extrusions are translated and their normals
  renormalized as `applyNormalMatrix` does. pm-render no longer lists
  "model groups" or "model-group recipe parts", pm-wasm's `compositionScene`
  answers recipe parts, and 3D object, light, and camera layers inside a 3D
  group draw (they are placed by `world3D` through the group; the
  `TwoAndAHalfD` reason stays for other layer types).

- **The engine viewer renders the editor view with the editor camera and
  Local View.** `scene3d.viewport.state()` gains `engineCamera` (contract
  2's `EditorCamera`: the editor camera's world-to-camera matrix and lens;
  an orthographic view's half height is `3 / zoom`) and `localView`; the
  viewer's view override carries them (`view-params.ts`), as today's
  compositor renders `viewportCamera` and filters Local View in the editor
  viewport. Blender previews in the editor view get the editor camera the
  same way (docs/engine/blender.md, "The paused viewer").
- **Blender snapshots of recipe parts** carry `generated` (the group's
  `evaluatedRecipe` and the part name) as `renderSnapshot` does; the
  recipe-part refusal is gone (`blender/snapshot.rs`).
- **`doubleSided`**: `compositionScene`'s `parseScene` default (absent means
  double-sided) is filled for 3D object layers.

### Tests and measurements

Apple M4 Max, macOS 27, 2026-10-18, load average about 9 to 12.

| Test | What | Result |
|---|---|---|
| `engine-doc/scene3d-geometry.test.ts` (real pm-wasm, dual-answer mode against today's `GL.pick`, `GL.bounds`, `PM.groupBounds` with three.js r180) | 5 seeded compositions (every primitive, lathe, extrude with and without bevel, a mesh source with groups and one or two slots, a hand-built animated GLB, an OBJ model, the missing box, untextured Khronos models: `BoxAnimated`, `RiggedSimple`, `RiggedFigure`, `BrainStem`, `RecursiveSkeletons`, `AnimatedMorphCube`, `SimpleInstancing`; keyframed transforms, opacity 0, double-sided, inactive, solo, layer opacity, 2D barriers, plain and model groups, a camera layer; a legacy scene layer with a hierarchy and models, a mesh definition layer, one with an invalid empty background) at 4 times, in the camera view, the editor view (orbited, then orthographic), and Local View | 6,128 compared answers (picks, bounds, group bounds), 0 mismatches beyond the dual-answer classes (identity for picks; 1e-6 for bounds, amendment 20), 0 fragile differences, 0 fallbacks; picks hit 373 object layers and 341 extension scenes, 807 throws agreed (the invalid background) |
| same | Animated rocket and staircase recipes: `compositionScene` from pm-wasm against today's at 5 times; picks and bounds in the camera and editor views | Scenes equal (every number exact, key order aside), 0 mismatches, 0 fallbacks |
| same | Model assets fetched once; a relinked record fetched again; a removed one draws the missing box | Pass |
| same | Pick latency in wasm (V8, Node), the 7 Khronos models above plus a random composition, a new time per pick (skinned models re-posed), 400 picks, editor view | p50 2.15 ms, p95 2.27 ms, max 3.73 ms |
| same | `viewportBounds`' boxes (`Box3.expandByObject`) from pm-wasm (`objectBoxes3d`) against three.js on 3 compositions with the Khronos models, 3 times | 138 boxes, every number exact (difference 0) |
| `pm-geom3d` `query` unit tests | Nearest object, depth-set ids, invisible and transparent objects, slots and groups (throw, nothing, hit), back faces by side, projected bounds, `setFromObject`, orthographic rays | Pass |
| `pm-eval` `tests/layers_unit.rs` `object_picks_raycast_the_scene` | A box seen by the studio camera, a 2D layer below, no geometry host (`HostOnly`), Local View | Pass |
| `pm-render` `tests/corpus.rs`, capture `threed-modelgroup-20261018-165040` (new corpus project `3d/model-group`, captured through the app) | Animated rocket and staircase recipes in turning 3D groups at 0, 1.5, 3 s | 3 frames: 100% exact outside 3D, 3D bar ≤ 2 CV 100%, max 1 CV, SSIM 0.99998 |
| `pm-render` `tests/blender_snapshots.rs`, capture `threed-modelgroup2-20261018-170236` (the same project with Blender on, recorded at main's IPC) | Recipe-part snapshots (final, preview, export, exportAlpha) against today's `renderSnapshot`, with `generated` | 9 compared, 9 equal; geometry 122,832 values, max difference 0; 1,755 exact values; 6 of 6 frame lists equal |
| `pm-render` `tests/blender_snapshots.rs`, corpus v1 | Regression | 158 compared, 135 equal, 23 not built (`3d/gltf`, `3d/gltf-materials`, imported models), as before |
| `pm-wasm` `tests/geometry.rs` | Models asked for once, the missing box, an OBJ that does not parse (kept missing, named), the editor camera from behind, Local View | Pass |
| `core/scene3d/engine-camera.test.ts`, `viewer/engine-present/view-params.test.ts` | The editor camera as contract 2's camera (perspective and orthographic), Local View in the snapshot and the view override | Pass |
| `e2e/viewport3d.spec.ts` with `POWERMOVE_NEW_ENGINE=1` (under the Electron lock) | Blender keys (transform, duplicate, hide, select, key), Properties tabs, box select, X-ray, Walk, Local View, Zoom Border, area lights, Edit Mode | 4 of 4 pass |
| `e2e/scene3d.spec.ts` with `POWERMOVE_NEW_ENGINE=1` | | 4 of 8 pass (navigation and framing, portable glTF export, OBJ bundles, normal maps and shadows); 4 fail where the spec reads today's WebGL canvas (`#gl`, `PM.GL.canvas` pixel sums of 0 while the engine draws the viewer) or calls `PM.Export.snapshot` before the window's engine session exists (`noSession`); none fails at a pick, bound, or gesture |
| `pm-render` `tests/corpus.rs`, `PM_RENDER_ONLY=3d/` (v1 and render captures) | Regression after the capability changes | 30 frames, 0 failed (`3d/motion-blur`, `3d/transparency` still routed by motion blur and masks) |
| `pm-render` `scene3d_document`, `headless`, `scene3d_layering`, `scene3d_resources` | Capability expectations updated (recipe parts and 3D objects in 3D groups draw) | Pass |
| `pm-geom3d`, `pm-eval`, `pm-edit`, `pm-wasm` test suites; clippy `-D warnings` | | Pass |

### THREED-1 behavior decisions (binding round 4; round 5 corrections)

- Exactly equal-distance picks must match today's object insertion history.
  **Corrected in round 5:** the native geometry host retains insertion
  history across source replacement and asset relinking, and synchronizes it
  at presented-frame boundaries as well as geometry queries. Edits before
  one frame do not create intermediate runtime objects.
- The skinned bounding sphere uses the current pose: **KEEP**, a recorded
  bug fix. Today's first-pose sphere can miss a mesh after it moves outside
  that sphere. The loaded-pose bounding box still matches today.
- Corrupted legacy scenes must throw the same `ZodError` as today's
  `parseScene`. **Corrected in round 5:** renderer geometry queries rethrow
  the original parser error when they reach the scene, including its class,
  message, and issues. The native frame-sync query uses the same parser.
- Headless and agent picks return native 3D answers where today returns
  null: **KEEP**, a recorded fix.

The local bounding-box pre-test always runs (three.js runs it only after a
bounds question computed the box); it only prunes. These decisions are
settled, with no further approval requested. Direct headless pm-wasm callers
receive a serialized validation marker; the renderer rethrows today's
original JavaScript `ZodError` object.

### Phase 3c round 2 (review fixes, 2026-10-19)

- **Models the engine cannot load fail by name.** A model three.js holds
  that `pm_geom3d::gltf::load` rejects, or an `object3d` without the bytes
  it was loaded from, is `ModelAsset::Unloadable`: every question about its
  scene replies `{unsupported: why}` (`MODEL_UNLOADABLE`), which the
  renderer counts as `<op>:unsupported` and warns with the reason. It is
  never answered with the missing-model box. (Corrected in round 3: this
  said "strict hosts refuse", which no code did in round 2. Since the
  document host's round 5 merge, the renderer and the document host install
  the engine document strict whenever "Use New Engine" is on
  (`legacy/bootstrap.ts`, `document-host/chain.ts`): `onFallback` turns any
  fallback into a named `EngineFailure` with the editor's toast, so three.js
  never answers. Round 3 adds the test.)
- **Third-party mesh definitions and overrides by id.** Geometry requests
  carry `meshFields` (definition id to `renderer.assetField`, from the
  host's `PM.layerDefinition`, as `sceneForLayer` reads it), so a user's
  own mesh definition, and an override of the built-in OBJ model layer,
  are scenes picked and bounded like today, not rectangles. Without a
  registry (no `meshFields`), only the built-in OBJ model layer is a mesh
  layer.
- **Viewport boxes go through the answer path.** `viewportBounds` asks
  `objectBoxes3d` through `NativeApi.answer` (counted fallbacks, dual
  compared, a named failure with the setting on) with the composition being viewed; it
  no longer builds the three.js runtime itself when the engine is on.
- **3D maps on level 1 bitmaps** (Textures, above). Measured through the
  app (`e2e/tools/threed-maps-capture.ts`, the uploads three.js r180 makes,
  read back texel for texel; hashes in
  `pm-render/tests/fixtures/stills2/three-maps.json`).
- **Blender textures** converted from the level 1 bitmap
  (docs/engine/blender.md).
- **Proof for 3D layers in 3D groups.** Corpus `3d/group-roles`: plain 3D
  groups with anchors, animated non-uniform scale, X/Z rotation and
  orientation, a nested 3D group, a spot and a point light inside groups, a
  camera inside a 3D group, parenting to a grouped object, and 3D layers
  in a flat group. (The integration branch's compositing closure lists no
  3D group reason for 3D object, light, or camera layers; this corpus case
  is the evidence for that.)
- **The viewport's render camera from pm-wasm.** The camera view, view
  framing, the view's field of view, and the gizmo's camera read
  `renderCamera(PM)` (core/scene3d/render-camera.ts): pm-wasm's `camera3d`
  (the composition scene's render camera, lens and `matrixWorld`) through
  the answer path, not `compositionRuntime(PM).camera`.
- **Transform targets from pm-wasm.** The gizmo, modal G/R/S, and the snap
  and pivot operators resolve targets from each object's `matrixWorld`
  (`objectWorlds3d`) and a flat group's members' boxes (`objectBoxes3d`)
  through `targetScene(PM)` (core/scene3d/engine-targets.ts).
- **Unknown primitives draw nothing.** A 3D object layer's scene is synced
  unvalidated today, so `primitiveGeometry` returns nothing for a primitive
  it does not know and the object draws nothing; the pass now does the
  same instead of listing it. A legacy scene layer's `parseScene` refuses
  such a scene, so it stays listed there (the layer fails today too).
- **Blender assets named by content** (docs/engine/blender.md): media-store
  files without an extension no longer fail the engine's Blender stage.
- **e2e with "Use New Engine" on.** `scene3d.spec.ts` reads what the
  viewport shows from the window (`capturePage`) when the engine presents,
  and compares the engine's own frames (`snapshotAsync`, after they
  settle) and the project's layers before and after its gestures and their
  Undo. `blender-motion.spec.ts` reads the engine capture's own pixels where
  it read today's compositor after a capture (the WebGL compositor has no
  prepared Blender frame when the engine captured).
- **The 3D orbit row** is measured (`scripts/frame-bridge-measure.mjs`
  runs `p3c-orbit-*`, an orbit input mode in the app measurement; below).

| Test | What | Result |
|---|---|---|
| `pm-render` `tests/threed_maps.rs` | `decode` of 128 fixture files (stills2 classes and the e2e still fixtures) against the texels the app stored | 112 classes texel-exact at level 0, 14 failed uploads zeroed, 2 rejected imports |
| same, `map_mip_levels_against_todays_generate_mipmap` | The pass's level 1 mip, sRGB and linear, against Chromium's `generateMipmap` | 224 of 224 equal |
| `pm-render` `tests/corpus.rs`, capture `threed-20261019-roles-maps` (through the app) | `3d/group-roles` (3 frames), `3d/maps-level1` (25 color maps and 5 linear maps of every class, 2 frames), `3d/model-group` (3 frames) | 8 frames, none skipped: 100% exact outside 3D, 3D bar ≤ 2 CV 100%, max 1 CV, SSIM ≥ 0.99998 |
| `pm-render` `tests/corpus.rs`, `PM_RENDER_ONLY=3d/` (v1 and render captures) | Regression after the merges and the map change | 35 frames, 0 failed, none skipped (`3d/transparency` and `3d/motion-blur` included) |
| `pm-render` `tests/blender_snapshots.rs`, v1 | Regression | 158 compared, 135 equal, 23 not built (imported glTF), as before |
| same, capture `threed-20261019-roles-maps` | `3d/group-roles`, `3d/model-group` snapshots; `3d/maps-level1` snapshots | 18 of 24 equal; the 6 `3d/maps-level1` snapshots differ in converted textures (docs/engine/blender.md, Gaps) |
| `engine-doc/scene3d-geometry.test.ts` (real pm-wasm) | The dual suite with a third-party mesh definition (`acme.turntable`, asset in its own field); viewport boxes through `answer`; an unloadable model | 0 mismatches, 0 fallbacks, third-party hits counted; boxes exact; the unloadable model reports `gl.pick:unsupported` and `gl.bounds:unsupported` with the reason |
| `pm-wasm` `tests/geometry.rs` | A rejected glTF and an unloadable model: picks, bounds, and `objectBoxes3d` reply `unsupported`; a loadable model answers again | Pass |
| `engine-doc/scene3d-geometry.test.ts`, render camera | 12 cameras (perspective and orthographic, animated, inside a turned 3D group) from `camera3d` against `compositionRuntime(PM).camera` | Exactly equal (lens and `matrixWorld`); no fallback |
| same, transform targets | 108 targets (primitives, OBJ and glTF models, model groups, a flat group), 3 seeds, 3 times; then every target's edits for a move, a turn, and a scale | 6 numbers 1 ulp apart (relative 1.1e-16, pm-eval's `world3D` product), the rest exact; 303 gesture edits identical |
| `pm-render` `tests/scene3d_resources.rs` | An unknown primitive on a 3D object layer | Nothing listed; the frame equals the frame without the object |
| e2e with `POWERMOVE_NEW_ENGINE=1` (Electron lock), after the fixes below | `scene3d` (8), `viewport3d` (4), `comp-thumbnails` (1), `blender-motion` (3), `engine-blender` (2) | Run 2: 17 of 18 (`blender-motion` model and render spec failed on the unbound `.blend` source, fixed in `27ffb184`); run 3 (`blender-motion`, `scene3d`): 10 of 11, the one failure being the import defect below (the gizmo spec's first engine frame after the OBJ import is the missing-model box; the same spec passed in run 2). Gestures, cancels, and Undo leave the project's layers and the engine's frame identical wherever the model was drawn before them. `engine-blender`: the job engine's still 100% within 2 CV of today's (max 0), the paused viewer's Material Preview max 1 CV from the Blender surfaces |
| same with `POWERMOVE_NEW_ENGINE=0` | `scene3d`, `blender-motion`, `viewport3d` with the adapted specs | 15 of 15 |
| `scripts/frame-bridge-measure.mjs` `p3c-orbit-*` r1 (`docs/engine/phase3c-measurements/`): MMB orbit in the editor view of the primitives scene with a sun casting shadows, 1920 × 1080, 120 Hz panel, 2,401 events; load average 25 to 68 from other lanes during the runs | Pointer-to-pixel p50 / p95 / p99 | Today idle 25.7 / 38.5 / 42.6 ms (56 unpresented); engine idle 28.2 / 44.9 / 53.3 ms; today under the gate load 26.0 / 88.2 / 113.3 ms; engine under load 26.6 / 84.9 / 101.6 ms. The p95 ≤ 33 ms row is not met by either path; idle the engine is 2.5 ms worse at p50 and 10.7 ms at p99 (amendment 5's comparison fails idle, passes under load) |

### Phase 3c round 3 (review fixes, 2026-10-19)

- **No three.js answer with the setting on.** The renderer
  (`legacy/bootstrap.ts`) and the document host (`document-host/chain.ts`)
  install the engine document strict whenever "Use New Engine" is on (the
  document host lane's round 5): `onFallback` turns every fallback of a
  query (`unsupported`, `context`, `exception`, `host`, `unavailable`, a
  trap) into a named `EngineFailure` with the editor's toast, so the
  three.js closures passed to `answer` (picks, bounds, `objectBoxes3d`,
  `objectWorlds3d`, `camera3d`, and round 3's `objectMesh3d` and
  `cursorHit3d`) run only in the development modes (dual answers, as the
  reference). A test installs the renderer's flag-on configuration (mode
  `on`, strict) and checks that the 3D questions are answered natively
  with none of the closures called, and that a model the engine cannot
  load throws `The new engine could not answer gl.pick (unsupported)` with
  the toast, never three.js. The round 2 wording "refused by strict hosts"
  (false then) is corrected throughout.
- **Edit Mode and the 3D cursor on pm-wasm.** `objectMesh3d` answers what
  `runtime.objects.get(id)` holds for Edit Mode's `currentMesh` (missing,
  not a mesh, or its geometry's buffers: f32 positions and uvs, the index
  when it has one; the missing-model box and OBJ models included), and
  `cursorHit3d` answers `placeCursor`'s surface point
  (`pm_geom3d::query::Scene::cursor_hit`: the first visible mesh hit by
  distance, no material filter, a model's meshes by their own visibility,
  points and lines skipped). `core/scene3d/engine-edit.ts` builds a
  `BufferGeometry` with today's buffers from the answer, so `meshFromGeometry`
  runs unchanged; Edit Mode's point targets, overlay and extrude normal
  read the object's world matrix through `targetScene` (pm-wasm's
  `objectWorlds3d`). Box select of models uses `render.gl.bounds`
  (`layerWorldBounds`), which is pm-wasm's bounds with the editor camera;
  Edit Mode's box select tests the overlay's projected points, which now
  come from pm-wasm's mesh and world matrix.
- **The import race.** A model imported with the engine on was drawn as
  the orange missing-model box for its first frames. Cause: the import
  writes the asset record into the document before its window binds the
  file (`flushBindings` follows), the engine asks main for the new asset at
  once, main has no binding yet and answers offline, and the engine's first
  retry is 250 ms later, so the layer that arrives after the bind draws
  with offline media. Main's media store now holds the viewer engine's
  read of an asset no window binds yet until a binding names it (up to
  1 s; `MediaStoreService.resolveAsset` with the interactive process); the
  engine shows nothing missing while a read is outstanding (its route stays
  pending). Jobs and other callers never wait, and a bound but missing file
  is offline at once. While a read waits the composition's frames wait
  too (the route is pending), so an asset no window ever binds (IndexedDB
  media not migrated yet) holds the viewer for up to 1 s at open where it
  drew that layer's offline failure at once before. The e2e helper that
  waited for frames to settle is
  replaced by one that requires the engine's frame to be stable at once.
- **Document host 3D geometry (issue D1).** A worker test adds 3D
  objects, groups them into a model group and a mixed group (group
  placement measures 3D bounds), reads the state and the 3D scene: 0
  fallbacks, 0 failures, 0 dual mismatches, with `edit.groupPlacement` and
  `camera3d` answered by pm-wasm (`hosts.test.ts`).

| Check | Scope | Result (2026-10-19) |
|---|---|---|
| `scene3d-geometry.test.ts` (real pm-wasm, `PM_WASM_REQUIRED=1`) | 10 cases: the round 2 suite, Edit Mode meshes and cursor rays on 4 seeded compositions with models (skinned, morphed, instanced Khronos models, OBJ, the missing box, every primitive, lathe, extrude, mesh sources with slots) at 3 times, transform targets on 10 seeds, the flag-on strict configuration | 246 meshes and 480 rays (134 hits) exactly equal to three.js; transform targets 339 compared, 29 numbers 1 ulp apart (worst relative 2.2e-16), 939 gesture edits identical; strict: 0 closures called, the unloadable model a named failure |
| `document-host/hosts.test.ts` 3D case | apply_commands with 3D objects, model and mixed groups, get_project_state, get_3d_scene, in `audit` mode | 0 fallbacks, 0 failures, 0 mismatches |
| `media-store/service.test.ts` | the viewer read before the bind, an unbound asset, jobs, a bound missing file | 15 of 15 |
| e2e `POWERMOVE_NEW_ENGINE=1` (Electron lock) | `scene3d` 8, `viewport3d` 4, `blender-motion` 3, `engine-blender` 2, `comp-thumbnails` 1 | **18 of 18**; the OBJ import gizmo spec (stable engine frame at once) 6 of 6 more with `--repeat-each=6` |
| e2e `POWERMOVE_NEW_ENGINE=0` | `scene3d`, `viewport3d`, `blender-motion` | 15 of 15 |
| `pm-render` `blender_snapshots` (v1 and `threed-20261019-roles-maps`; nothing allowed) | every Blender snapshot and frame list | 182 compared, 153 equal, 29 failures (23 glTF snapshots not built, 6 converted textures); fails by design until closed |

### Open items at the end of round 4 (historical)

- The 3D viewport chrome (`core/scene3d/overlay.ts`) still renders the
  scene with three.js for grid occlusion and selection outlines, so the
  composition's three.js runtime is still synced while the 3D viewport is
  open (for the chrome only; no answer comes from it with the setting on).
  Binding decision D requires replacing that redraw while retaining today's
  pixels. The current depth/ID contract is insufficient on its own:
  - the surface's depth is view-space depth in `rgbaf16` (about 11 bits
    of mantissa: 0.008 world units at 10 units away), where today's grid
    is depth-tested against a 24-bit depth buffer of the same scene, so the
    grid's occlusion near surfaces at the floor (y = 0) would differ;
  - the id channel names the topmost covering layer per pixel, where
    today's outline mask draws every selected mesh whole (its occluded
    parts included, `drawOutlines`), so outlines of partly hidden objects
    would differ;
  - the frame descriptor carries no id-to-layer legend (pm-render's
    `Frame` has it), and the surface covers the composition, not the stage
    the overlay draws over.
  Matching today needs a selection-mask surface (selected objects' coverage
  without depth against others) and a full-precision depth surface from the
  engine (a contract 2 change). Keeping the three.js scene redraw is not an
  accepted alternative. This implementation is still missing. The light and
  camera glyphs box select reads (`glyphsIn`) are drawn by the same chrome.
- The `scene3d` API is still assembled by `makeScene3DAPI` over today's
  modeling, rendering, and controller code. Its geometry questions are
  pm-wasm's (above), and `describe` evaluates through the engine document,
  but with the engine drawing the viewer `renderFrame` and `cancelRender`
  still run and cancel today's renderer-side Blender preview, and
  `previewState` reports that preview, not the engine's (whose jobs are in
  the presenter's `blenderPreview` status, which no renderer channel
  carries yet). `assets.get().object3d` is still the importer's three.js
  object, not one built from the engine's geometry.
- Blender: all 23 previously unbuilt imported-glTF snapshots now match,
  as do all six captured TIFF maps at full sample depth. Five AVIF maps
  still differ when converted through Canvas2D (the six map snapshots
  remain failures); see blender.md's round 4 results.
- SVG maps use Chromium's compatibility upload channel with a distinct
  material-map key, straight alpha and flipY. Blender's SVG-to-PNG asset
  conversion still needs to consume the Chromium canvas raster; it does
  not yet do so. The no-media static check still lists `imported models`
  and map containers conservatively. `Feature::Scene3d` has not been
  retired.
- The 3D orbit row misses p95 ≤ 33 ms on both paths in r1 (2026-10-19,
  load average 25 to 68: engine idle p50/p95/p99 28.2/44.9/53.3 ms against
  today's 25.7/38.5/42.6 ms; under the gate load the engine is
  26.6/84.9/101.6 ms against today's 26.0/88.2/113.3 ms;
  `docs/engine/phase3c-measurements/p3c-orbit-*-r1.json`). The r2 rerun was
  queued behind other lanes' jobs on the Electron lock for over an hour in
  round 3 and cancelled unrun; it needs a quiet machine before the gate.

### Phase 3c round 4

Merged the integration branch, the bridge branch, and the accepted SVG
compatibility branch (it was not yet in the integration ref when this lane
started). The three SVG merge conflicts keep both draw reuse/performance
state and SVG raster delivery. Protocol TypeScript and both addons were
regenerated/rebuilt, as was pm-wasm.

Imported-model Blender snapshots now use pm-geom3d's CPU pose for vertices
and skin-adjusted normals, the renderer's source materials, and the
asynchronous material-completion order for embedded texture ids and mesh
order. The retained comparison builds all 182 snapshots; 176 are equal,
including all 23 imported-glTF snapshots. All 90 frame lists match, with
geometry max 3.47e-16 over 14,118,112 values. All six TIFF map conversions
match at full decoded PNG depth. The six snapshots with AVIF maps remain
failures; nothing is excluded or given a larger tolerance.

The real-wasm geometry suite passes 10/10, including strict flag-on
`objectMesh3d` and `cursorHit3d` failures with visible toasts and zero legacy
closures called. The 3D gesture snapshots now assert occupied native pixels
before comparing images, so two empty captures cannot establish parity.

SVG material maps use S1's Chromium channel. A separate key distinguishes
three.js's straight, flipped image-element upload from the 2D premultiplied
bucket. Native color and data maps get their mip chains as before. Exports
and synchronous captures wait for the exact requested raster. Missing
preview maps are requested through the existing SVG miss channel; the
presenter keeps the previous frame until those exact texels arrive,
without presenting an absent 3D object. Oversized SVG maps follow
three.js's 16,384 px resize rule. Corrupt SVGs recognized as broken images
remain missing textures. pm-cli's resvg map raster is explicitly counted
as an approximation, as S1 requires.

Round 4 verification (same source and rebuilt addons, flag on):

| Check | Observed result |
|---|---|
| `scene3d`, `viewport3d`, `engine-blender`, `blender-motion`, `comp-thumbnails` e2e | Required 18/18 pass, one worker; native gesture captures assert occupied pixels and multiple colors before Undo image comparisons |
| SVG material-map e2e | Pass; 1,600 occupied pixels, 1,156 interior pixels compared, max 0 CV, no SVG failures, engine route |
| `svg_level1` and `threed_maps` | 11/11 pass; all 112 retained still classes have exact texels, 14 failed uploads retain zero storage, two rejected imports remain missing textures; every retained sRGB/linear mip matches |
| SVG transport and main snapshot unit tests | 45/45 pass |
| SVG protocol key/band unit tests | 3/3 pass |
| Real-wasm 3D geometry suite | 10/10 pass |
| `cargo test -p pm-geom3d -p pm-eval -p pm-wasm -p pm-protocol` | Pass, four test threads |
| `pm-geom3d` / `pm-render` library clippy | Pass with warnings denied |

The complete Blender snapshot comparison remains a failing gate (176/182).
Decisions D/E are unimplemented, so G was not re-measured: the earlier idle
orbit regression remains a blocker. THREED-1's insertion-history tie break
and corrupted-scene `ZodError` corrections also remain open. No visible
difference is accepted by these test results.

### Phase 3c round 5: query behavior corrections

The geometry host now retains each scene's object insertion order across
queries and native presented-frame syncs. A changed source or relinked asset
removes and reinserts its object,
matching `SceneRuntime.sync`'s `Map` behavior for equal-distance ray hits;
document reordering alone leaves the existing order intact. The history is
keyed by project and composition/layer and survives ordinary document edits.
The skinned-model sphere still uses the current pose, per THREED-1.

Renderer geometry queries validate legacy scene data with the same
`parseScene` function as today's runtime. When a native pick or bounds query
actually reaches a corrupted scene layer, it rethrows that parser's original
error object, preserving the `ZodError` name, message, and issues. This does
not run three.js geometry. Regression tests cover a tied pick after a source
replacement, source/order history, a corrupt stored scene, and a corrupt
generated model scene; the frame-sync path also checks the stored scene's
original `ZodError`. The real-wasm geometry suite passes all 13 cases.


### Phase 3c round 5: native viewport chrome and API compatibility

The native presenter now owns the viewport geometry passes. A stage-sized
`Depth32Float` target supplies window depth. Its f32 bits are split 11/11/10
across an RGBA16F transport plane as exact binary fractions, so half-float
storage does not reduce depth precision. A second plane carries the visible
22-bit object ID and selected/active coverage. Selected objects are rendered
independently of other objects' depth, in the same selected-then-active order
as today's outline mask. The descriptor includes the object legend, expanded
model-group selection and evaluated light/camera glyphs. Revision, view
sequence, pool generation and frame sequence identify the complete pair.

The fractional CSS composition rectangle determines the chrome projection,
matching the overlay camera rather than the separately snapped color frame.
`overlay.ts` restores the encoded depth for the existing grid shader and
uses the independent coverage for the existing outline colors, radius and
X-ray behavior. `controller.ts` does not synchronize `compositionRuntime`
when the engine setting is on. Edit targets, navigation and glyph box select
use native geometry and frame-aligned glyphs. A missing native answer is a
named engine failure; it never selects the reference runtime.

Chrome and color share slot ownership. The viewer retains the displayed
chrome pair until replacement and until every WebGL read fence completes;
color presentation alone cannot release a slot that the overlay still reads.
Pool retirement also waits for those readers. The native bridge accepts 64
slots, enough for both planes paired with each of the maximum 32 color slots.
The preload's final-slot test and the replacement/drop/fence lease tests
exercise this ownership boundary.

The renderer's preview API uses the presenter's explicit Blender controls
and status events. A render resolves only after its matching Blender frame
is actually presented; cancel/restart invalidate the previous generation.
Automatic viewer previews use separate generations. The native importer and
Object3D compatibility adapter are described in gltf.md. The adapter keeps
the deprecated API functional without loading engine-on models through
GLTFLoader, and warns extension authors when they read it.

Five paired hidden-window Chromium captures exercise grid occlusion by
intersecting boxes, selected/active outlines including an occluded object,
X-ray, box-selected light glyphs, and a transformed imported glTF. They pass
the existing 3D comparison bar: 99.989–100% of pixels within 2 CV, zero CV
outside the independently measured reference MSAA edges. An additional
overlay-ink SSIM witness scores 0.999773–1.0 against a 0.995 threshold; it
is an overlay quality check, separate from S7's per-object metric.
Selected/active color masks have zero mismatching pixels, and X-ray is pixel-identical. The reference-only
witness renders today's unchanged depth/grid pass with and without default
framebuffer MSAA and dilates the difference by one pixel; it includes grid
intersections inside meshes, where MSAA also changes depth-test coverage.
It never reads native pixels to choose exclusions and never excludes an
outline-color pixel. At most 879 of 864,000 captured pixels are excluded.
The helper reads the actual transient camera through the public viewport
API, avoiding a second bundled module's independent viewport WeakMap.

The historical round 4 gaps and results above describe that earlier source,
not accepted differences in the new implementation.


### Round 5 synchronized verification and remaining gate

The rebuilt native addon and generated TypeScript both speak
`protocol-e0820481c9e45c9e`. With Use New Engine on, the synchronized app
run passes **25/25 tests**, with no skips: all 19 required scene3d,
viewport3d, Blender, motion, thumbnail and SVG-map checks, plus the new
preview API, two live PNG-transport cases, chrome comparison, selected-file
import parity and textured Object3D compatibility checks. Native gesture
captures retain their nonempty-pixel and document/Undo assertions.

The complete Blender corpus passes 182/182 snapshots (all 23 imported
models included), 29 converted-map pixel comparisons and 90/90 frame lists.
The real-wasm geometry suite passes 13 cases. Native preview failure tests
pass 11/11, Blender compositor/chrome integration 7/7, and lossless depth
packet boundaries 1/1. Protocol tests pass 96 unit, two integration and 11
wire/generation cases; the fixture-update test remains intentionally
ignored. All-target Clippy passes with warnings denied. Targeted renderer,
main, preload and presenter tests pass, including replacement/fence lifetime,
retired generations, cancellation and cache eviction. The restored flag-off
Blender motion checks also pass 3/3. A follow-up preview API check passes
cancellation and restart after an editor-camera orbit, with matching actual
presented frame tags, nonempty native pixels, new native jobs and zero legacy
preview jobs.

**G is unmeasured after D.** The paired `p3c-orbit-today-idle` and
`p3c-orbit-engine-idle` run was attempted under the Electron lock after the
synchronized app checks. The harness skipped both because the main display
was asleep and the session locked. No post-D p50/p95/p99 samples were
produced, and no performance pass is claimed. Run this same pair with the
display awake and session unlocked on a quiet machine; compare all three
percentiles and profile any regression. The r1 values above remain
historical pre-D results, not measurements of the new chrome.

## Phase 3c round 6: no 3D routing, cancel, and Blender inputs

### Corrupted scenes

`corrupt-scenes.test.ts` pins today's behavior. `compositionScene` runs
`parseScene` on the assembled scene, so an unknown primitive or light type
on any 3D layer throws for the whole composition: the compositor's catch
draws nothing for every object layer and an export fails. A legacy scene
layer whose `sync` throws, or whose scene is absent, falls through to the
missing-definition checkerboard. The engine now does the same:

- `SceneBuilder` refuses unknown primitives in composition scenes too (the
  old empty-geometry rule was wrong); a scene that does not build makes
  `plan_layer` fail every object layer with a `LayerFailure` (nothing
  drawn, exports fail), not a routing reason
  (`scene3d_resources.rs` `unknown_primitives_fail_the_composition_scene_as_today`).
- A legacy scene or mesh layer that does not build plans
  `shader_layers::plan::missing_placeholder`, the checkerboard program,
  at the extension size; `scene3d_document.rs`
  `the_static_check_looks_inside_legacy_scenes` compares its pixels with
  a missing-definition extension layer's for an unknown primitive, an
  unknown light type and an absent scene.

### Material maps

`MapError::Unsupported` and `Error3d::Unsupported` carry the image
feature (`crate::media::probe_image`'s), not a 3D reason; the static check
lists it on the 3D layer as `check_media` would on an image layer.
`map_verdicts_are_the_image_layer_verdicts` checks every still fixture
(120 drawn, 2 unreadable, 0 unsupported): map and image layer verdicts are
equal, and every drawn still decodes to the single RGBA8 plane a map
uploads. The old "texture map image layout" reason is now an internal
failure with a debug assertion; the 112-class texel test runs every
compat output through it. Without media, a map's recorded container gets
`stills::static_format`'s feature, as an image layer's does. The LTC
table load failure is an internal `Failed`, not a routing reason.

### glTF acceptance

`gltf.md`, "Acceptance": both engine readers now accept exactly the files
`loadGltf` loads over a 106-file boundary table, except two-parent nodes.
A refused file is `ModelLookup::Missing` and pm-wasm's `ModelAsset::Missing`
(the orange box), as today's asset without a scene.

### cancelRender

`cancelBlender` with the engine on cancels the window's Blender worker
through `bridge().blender.cancel()` again: the worker still runs
`scene3d.model`'s Regenerate and `.blend` import jobs, and the Model
Inspector's Cancel relies on it. The native preview is cancelled alongside;
`EnginePreviewUnavailable` (no native session, route not `engine`) and
`EnginePreviewCancelled` resolve, as today's cancel always does. Tests:
`rendering.test.ts` (a pending model job is rejected by the cancel; the
unavailable cases resolve; other errors still reject) and the app test
`engine-scene3d-cancel-model.spec.ts` (real Blender, engine on).

### Round 6 verification (2026-10-19)

- Rust: `pm-render`, `pm-geom3d`, `pm-wasm` and `pm-eval` tests pass with
  4 threads (448 passed, 9 ignored, with `corpus_render_parity` run separately), apart from `corpus_render_parity`'s
  five known video-class frames (`motion/blur-video`, `retime/speed`,
  `space25d/precomps`; composite.md), which this lane does not touch; its
  48 `3d/` rows are all within the 3D bar. All-target Clippy with warnings
  denied passes for `pm-geom3d`, `pm-render`, `pm-wasm` and `pm-node`.
- Blender: both `blender_snapshots` tests pass with no environment set
  (182/182, 29 converted textures by pixels, 90 frame lists).
- Vitest with `PM_WASM_REQUIRED=1` over core/scene3d, scene3d-geometry,
  engine-connection, layers-3d, viewer and main/blender: 46 files, 408
  tests pass, including the glTF acceptance table and
  `corrupt-scenes.test.ts`.
- App tests with Use New Engine on, under the Electron lock, one worker:
  26 of 27 passed in the batch (scene3d, viewport3d, engine-blender,
  blender-motion, comp-thumbnails, chrome, preview API, PNG transport, SVG
  maps, captures, compat and import parity); the new
  `engine-scene3d-cancel-model.spec.ts` failed there only because its
  recipe exceeded the geometry budget, and passes with a valid recipe
  (1/1). Chrome pixels match round 5 (max 51-52 CV only inside reference
  MSAA edges, 0 CV outside).

## Phase 3c round 7: scene metadata and compatibility decisions

THREED-7 adds `FrameDescriptor.scene3dMeta`: composition/layer groups with
the scene's warnings and optional construction error. The native render
plan supplies the metadata, including map decode failures found while drawing,
legacy checkerboard failures, nested compositions, and reused draws. Layers
sharing diagnostics share one wire record, so a large scene does not repeat
its full warning list for every object. Scene identities are weakly retained
for decode diagnostics; this does not keep unused meshes resident.

The viewer applies these records only after painting their frame. Session,
sequence and revision checks reject stale metadata. The `uiState` patch has
today's shape: successful scenes set `definition`, `missing: false` and
`sceneWarnings`; failures set `definition`, `missing: true` and `sceneError`.
Other extension metadata is retained. As in today's merging `setShaderMeta`,
success does not erase a historical `sceneError` string. This path builds no
three.js scene and performs no extra geometry evaluation.

THREED-3 is a recorded bug fix for Jude's beta list: native scenes continue
drawing after ordinary editing exceeds 512 objects, 16 lights, a 160-character
display name, or 8 MB serialized scene JSON. Today's parser instead makes the
whole composition fail. Other strict keys, ranges and scene refinements keep
the existing refusal, for both composition scenes and legacy scene layers.
Synthetic parser-oracle vectors are generated by today's `parseScene` and
checked by the shared native validator.

### Round 7 verification (2026-10-19)

- Real-WASM 3D, viewer, connection and Blender host units: 52 files,
  462 tests pass. The glTF differential tests include 192 generated graphs,
  nine selected graphs and three decoded-image/cache ordering cases.
- Rust: 241 core tests, 84 presenter tests and 172 native render tests pass.
  Five timing-only tests remain ignored.
  The paired Chromium/native capture passes the production 3D bar separately.
  All-target Clippy with warnings denied passes for the changed crates.
- Blender: 182/182 snapshots, all 29 converted textures by decoded pixels,
  90 frame lists, and the full-depth TIFF test pass. All 224 map mip
  comparisons are texel-exact against Chromium.
- Under the shared Electron lock, one worker: 29/29 flag-on tests and
  16/16 flag-off tests pass. These include viewerless, Solid and playback
  preview captures, model cancellation, scene warnings, compatibility imports,
  deprecated Object3D, chrome, and 28 Chromium short-input probes.
- Chrome: zero selected/active outline-mask mismatches; X-ray exact;
  max 0 CV outside reference MSAA edges. The paired glTF capture has max
  2 CV and object SSIM 0.99999768 with no edge exclusion.
- Full TypeScript checks retain nine renderer and 75 node diagnostics in
  other lanes; no changed-lane diagnostics remain. Paired idle orbit is
  deferred to the orchestrator's quiet, awake measurement window; no current
  performance pass is claimed.

## Phase 3c round 8: skinned short normals and session startup

- Draw rule: skinned or env-mapped MeshBasic programs consume `NORMAL`,
  and flat shading no longer exempts Standard/Physical (gltf.md, round 8).
  This covers a skinned unlit glTF color draw and the native chrome's
  Basic override draws (depth, IDs, outline mask), which today's
  `overlay.ts` MeshBasicMaterial overrides lose for a skinned mesh with a
  short NORMAL.
- `DocumentSession.start()` is back to the bridge lane's semantics. A
  refused or interrupted first resync no longer throws: a session closed
  while starting logs no spurious warning, and a session whose first resync
  failed stays open for a later gap or load resync. Only the cache
  preview's wake (`documentWake` in `engine-connection/preview.ts`) refuses
  an unaccepted document, with `EnginePreviewUnavailable`.
- Decision kept: with the setting on and no `PM.EngineConnection` (the
  install returned null: no frame bridge or kernel services),
  `renderFrame` still throws `EnginePreviewUnavailable`. Rendering it
  through today's three.js path would be the silent legacy routing the
  owner rules forbid; `cancelRender` resolves because there is nothing to
  cancel.
- Still open for the orchestrator: whether the 500,000-vertex geometry
  budget joins THREED-3's beta-list bug fixes (it stays a refusal, like
  today), and the paired idle 3D orbit row (G).
