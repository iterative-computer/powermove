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
| Any number of 3D object layers per composition, depth sets and ordering exactly as section 8 states (inactive layers, lights, cameras, and plain group headers do not end a set; 2D layers and styled groups do; opacity ≤ 0.001 and solo exclusions; runs sorted by the camera depth of the object origin, blocks never reordered) | Drawn | `pm-geom3d` `layering` tests; `tests/scene3d_document.rs`; sweep `depth-sets`, `layer-opacity-and-mask`; corpus `3d/solo-hidden`, `3d/lights`, `3d/primitives` |
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
| The depth and id surface for every 3D layer: linear view depth and id, upright, 3D inside precompositions and group boundaries included | Drawn | `tests/scene3d_document.rs`; corpus 3D masks |
| 3D output written per layer into the 2D graph (masks, effects, opacity, blend stay per layer) | Drawn | Unchanged composite path; corpus |
| Linear blending: the resolved, premultiplied, E-encoded canvas unpremultiplied (guarded), decoded with E⁻¹, re-premultiplied into the working format | Drawn when the working encoding is linear light | `linear_blending_decodes_the_3d_canvas` (all 8-bit codes at 6 alphas, f32 within 2e-6 of `pm_color::linear`) |
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
   legacy scene as a quad through the layer transform), converting it to
   linear light first when linear blending is on.

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
- **Textures** decode through `pm-media` in the still classes the 2D
  renderer draws, upload top row first (WebGL ignores `UNPACK_FLIP_Y` for
  an `ImageBitmap`), sRGB maps as `Rgba8UnormSrgb`, and generate mipmaps
  as box filters (in linear light for sRGB). Maps with transparency are
  routed: the premultiplication Chromium applies to an `ImageBitmap` in
  WebGL has not been measured.
- **Missing textures** draw without the map and add a warning, as today.
- **`doubleSided`** is read as `compositionScene` passes it (absent means
  front side), not with the schema default.
- **Depth and id surface.** Coverage is the layer's own alpha (≥ 0.5/255);
  depth comes from a single-sampled pass, so MSAA-only edge pixels report
  +inf. Inside a precomposition the depth is the nested camera's.
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

- `Renderer::set_view(Some(&view))` applies contract 2's view override to
  the following renders. `RenderRequest::from_frame_request_with_view`
  returns the request and its view (dropped for exports and agent
  captures, `FramePurpose::is_final`); the plain `from_frame_request` still
  refuses a request carrying a view, so a caller that does not apply it
  routes instead of drawing the editor view without its override.
- `Renderer::set_mesh_definitions` registers mesh-layer definition
  snapshots (`assetField`, parameter defaults). The built-in OBJ model
  layer is known without one. The static capability check cannot see
  third-party snapshots and lists those layers until snapshots reach the
  engine.
- `unsupported` lists, for 3D: imported glTF models, recipe parts, unknown
  primitives, map images outside the drawn still classes (when the media
  can be opened), SVG maps, model groups, and Blender rendering.

## Tests and measurements

Apple M4 Max, macOS 27, wgpu 30.0.1 (Metal, safe math), Chromium 152 with
ANGLE on Metal and three.js 0.180.0 for references, 2026-10-15.

| Test | What | Result |
|---|---|---|
| `pm-geom3d` `tests/three_geometry.rs` | 34 sources (every primitive with groups, capsule, icosahedron at detail 0 to 5, lathe, extrude with and without bevel, mesh sources) against three.js; 5 OBJ files against `parseObj` in V8 | Buffers, indices, and groups bit-identical |
| `pm-render` `tests/scene3d.rs` | 31 scenes, 74 outputs against three.js renders through the production `SceneRuntime` (`tools/three-ref/run-scenes.ts`), section 2's 3D bar | All within the bar: every output 100% within 2 CV, max 1 CV everywhere, SSIM ≥ 0.99984 |
| `tests/scene3d.rs` | Programs and uniforms against the recorded draws | 265 draws matched; 23,831 uniform values equal in f32; every recorded draw that writes color or depth matched |
| `tests/scene3d_mesh.rs` | Legacy mesh layers (3 OBJ meshes, the checkerboard) against the compositor's shader in WebGL | Max 1 CV |
| `tests/scene3d_document.rs` | Ordering, depth sets, depth and id surface (upright; in precomps), view overrides, OBJ models, mesh layers, linear blending | Pass |
| `tests/corpus.rs` | Every 3D corpus project the renderer supports (`v1-20261012-230504`, `render-20261014-131536`) | 22 3D frames and the render corpus's 3D projects, 0 failures; `layers/obj-model` 3 frames exact; the full corpus 92 frames, 0 failures |
| Program cache | Cold and warm first frame of `five-casting-lights` (debug build) | 343 ms translating, 18 ms from the disk cache |
| `frame_time_3d_shadows_at_1080p` (ignored, release) | 7 layers, 3 casting lights (sun, spot, point atlas), 1920 × 1080 | Median 9.7 ms, p95 13.9 ms (CPU plan and GPU); 26 to 36 ms median when re-measured at a load average of about 30 from other workers |

## Gaps

- Imported glTF scenes, recipe parts (model groups), Blender surfaces, and
  Studio Cube belong to other lanes and stay routed.
- Third-party mesh layers render natively only with a definition
  snapshot; the static check routes them until the definition snapshot lane
  delivers them (the built-in OBJ model layer needs none).
- 3D picks and bounds (`pm-eval`'s `LayerHost` hooks `pick_object`,
  `object_bounds`, `scene_bounds`, `pick_extension_scene`) still come from
  the host's three.js runtime (section 26, Phase 1: until 3c). Rectangle
  picking of mesh layers is `pm-eval`'s `GL.pick` port and needs no 3D.
- Corpus projects `3d/depth-sets` (shape layers), `3d/transparency` (a
  mask), and `3d/motion-blur` are routed by 2D features other lanes own;
  their 3D behavior is covered by the sweep and the document tests.
- Texture maps with transparency, and maps outside the drawn still classes,
  are routed.
- 3D output stays 8-bit as today's canvas, renders at the full output size
  for a region of interest, and has no render cache yet (the content key
  is ready for one).
- Program text differences that three.js's cache makes history-dependent
  (above) are reproduced as fresh programs, not as history.

## Changes outside the lane

- `pm-gpu/src/glsl.rs`: a public `preprocess` (the existing glslang step).
- `pm-render/src/plan.rs`: `Build` carries the passes' 3D state and
  options; `build_with`; `comp_pass` orders the pass; the extension branch
  plans legacy kinds.
- `pm-render/src/renderer.rs`: `set_view`, `set_mesh_definitions`.
- `pm-render/src/request.rs`: `from_frame_request_with_view`.
- `pm-render/src/exec.rs`: linear conversion of 3D output; nested depth
  for precompositions in the depth and id surface.
- `pm-render/src/capability.rs`: legacy scene definition accepted; the 3D
  check takes the asset resolver.
- `pm-render/src/shaders/id.wgsl`: 3D layers read upright (a fix).
- `pm-render/tests/headless.rs`: the capability test's 3D section.
