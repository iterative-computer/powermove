# Imported glTF models in the 3D scene pass: design notes

The glTF lane (plan section 8 "3D layers", Materials and Animation;
section 13 "3D models"; section 25's 3D class; amendment 11; spike S7).
Code: `engine/crates/pm-geom3d/src/gltf/` (loading, mixer, posing,
picks) and `engine/crates/pm-render/src/scene3d/gltf/` (assets, the
material rule, programs and uniforms, GPU resources), hooked into the
scene pass of docs/engine/scene3d.md. The plan stays the authority.

## What it draws

Everything three.js r180's `GLTFLoader` builds, as today's importer
(`loadGltf`) prepares it and today's runtime (`SceneRuntime.sync`,
`compositionRuntime`) draws it.

| Feature | Status | Evidence |
|---|---|---|
| GLB and self-contained glTF; `loadGltf`'s checks (128 MB, data URIs of PNG, JPEG, WebP and octet streams only, 1.5 M elements per accessor, 1 M predicted vertices, 4,096 nodes, 500,000 triangles, a measurable size) and fit (`Box3.setFromObject`, scale 1 / radius, scene moved by −center) | Drawn | `tests/scene3d_gltf.rs` poses (every world matrix equal to three.js's); refusals agree (3 Khronos files refused by both) |
| Accessors: strides, interleaving, normalized integers (`KHR_mesh_quantization`), sparse accessors, matrix column padding; strips and fans (`toTrianglesDrawMode`); geometry shared per `createPrimitiveKey` | Drawn | Khronos `BoxInterleaved`, `SimpleSparseAccessor`, `MeshPrimitiveModes`, quantized grid cell; poses and sweep |
| The object tree: groups for multi-primitive meshes and multi-attachment nodes, bones, `_getNodeRef` copies (`_instance_n`), unique sanitized names (`createUniqueName`) | Drawn | Poses (1,170 drawables in order, 178,450 values, largest difference 0) |
| Materials with every extension r180 honors: `KHR_materials_clearcoat`, `sheen`, `iridescence`, `transmission`, `volume`, `ior`, `specular`, `anisotropy`, `dispersion`, `emissive_strength`, `unlit`, `EXT_materials_bump`, `KHR_texture_transform` (with `texCoord`), alpha modes (mask cutoff, blend), double-sided, normal and occlusion strength, the default material | Drawn | Synthetic grid and the Khronos `Compare*`, `*Test`, and showcase models; programs byte-identical, uniforms equal as f32 |
| `assignFinalMaterial`: vertex colors (RGB and RGBA), flat shading without normals, derivative-tangent `normalScale.y` flip (twice for instanced meshes, which run it again), `PointsMaterial` (1 px, no attenuation) and `LineBasicMaterial` | Drawn | Programs and sweep (`VertexColorTest`, flat cell, instanced cell) |
| Textures: PNG, JPEG, WebP (`EXT_texture_webp`), AVIF (`EXT_texture_avif`), GIF, BMP, ICO decoded as stored (straight alpha, no color management), samplers (nearest, linear, mipmapped filters, repeat, clamp, mirror), mipmaps only for mipmapped filters, sRGB for color, emissive, sheen color, and specular color maps, second UV sets (`aoMap` on `TEXCOORD_1`, `MultiUVTest`) | Drawn | `webp-avif`, `TextureSettingsTest`, `TextureCoordinateTest`, `TextureLinearInterpolationTest`, `MultiUVTest` |
| Skinning on the GPU (`boneTexture`, `bindMatrixInverse` of the attached mode), weights normalized (`normalizeSkinWeights`, re-quantized for 8 and 16-bit weights) | Drawn | `skinned-*` at 5 times, `CesiumMan`, `Fox`, `BrainStem`, `RiggedSimple`, `RiggedFigure`, `RecursiveSkeletons`, `SimpleSkin`; bone matrices equal three.js's |
| Morph targets on the GPU (`morphTargetsTexture` array texture with position, normal, and color; relative base influence 1) | Drawn | `AnimatedMorphCube`, `MorphPrimitivesTest`, `MorphStressTest`, `SimpleMorph`, synthetic morph sphere; influences equal |
| The mixer rule: every clip at weight 1, looping, blended equally in clip order (`PropertyMixer.accumulate`), linear, step, and glTF cubic-spline interpolants, `Interpolant.evaluate`'s search, mixer time `time − layer.from` (composition) or the legacy scene layer's local time | Drawn | Poses at 30 animated case times; `InterpolationTest`; corpus `3d/gltf` at 5 times |
| `EXT_mesh_gpu_instancing` (`instanceMatrix`, `_COLOR_0` as `instanceColor`) | Drawn | `SimpleInstancing`, instanced cell; instance matrices equal |
| Points and line primitives (segments, strips, loops) | Drawn, in every layer | See "Points and lines" below |
| `KHR_lights_punctual`: directional, point, spot (cone from inner and outer angles), range, intensity; lights of every visible model light every depth set and count toward neither the 16-light nor the 4-shadow limit | Drawn | `LightsPunctualLamp`, `PlaysetLightTest`, `PointLightIntensityTest`, `DirectionalLight`, `LightVisibility`; light positions and targets equal |
| The transmission pass: half-float 4× MSAA target cleared as three.js clears it, the layer's own opaque items untoned and linear, resolved and mipmapped, then back faces of double-sided transmissive items, resolved and mipmapped again; transmissive items sampled trilinearly | Drawn | `transmission`, `transmission-background`, `TransmissionTest`, `TransmissionRoughnessTest`, `TransmissionThinwallTestGrid`, `TransmissionOrderTest`, `AttenuationTest`, `DispersionTest`, `IORTestGrid`, glass showcases |
| `sync`'s material rule: source materials, or the object's material over every mesh (a source standard or physical material is reused and keeps what `sync` does not set: its class, extensions, vertex colors, flat shading, alpha test, bump map, scales; an unlit one becomes a new `MeshStandardMaterial`); slots make an array material that draws nothing on a mesh without groups | Drawn | `pbr-ext-object-material`, `skinned-object-material-transparent`, `slots-draw-nothing`, corpus `3d/gltf` (override layer) |
| Render lists: opaque by `material.id` then depth, transmissive and transparent back to front (`reversePainterSortStable`), double-sided transparent back then front; editor looks on meshes only | Drawn | Every draw in three.js's order; `pbr-ext-wireframe`, `pbr-ext-xray` |
| Shadows of glTF casters: the depth and distance materials with the caster's side, map, alpha test, skinning, morphs, instancing | Drawn | `pbr-ext-shadowed`, `models-in-a-set` |
| Depth sets with scene objects, layer opacity, two model layers, a legacy scene layer with models (whole render) | Drawn | `models-in-a-set`, `legacy-scene-model` |
| Picks on posed models (skinned, morphed, instanced, points, lines, material sides) | `pm-geom3d` API | 7,584 rays on 20 cases, all equal to three.js's raycast; not yet called by `pm-eval` (see Gaps) |

Today's importer also refuses what three.js cannot load without extra
loaders (Draco, required meshopt, required KTX2), external resources,
and over-budget files; this engine refuses the same, and the static
check (`pm_render::unsupported_with_media`) lists a model only when its
file loads in the importer but not here (`imported model the engine does
not load (...)`) or, without a media resolver, conservatively.

## How a model draws

1. **Load** (`pm_geom3d::gltf::load`, cached per file version in
   `scene3d::gltf::asset`). The `gltf` crate reads the GLB container; the
   document is read as raw JSON in `GLTFLoader`'s order (the crate's typed
   schema refuses valid files, e.g. `KHR_animation_pointer` channels
   without a node). Images are decoded once through `pm-media` from a
   content-addressed temporary file, in the formats Chromium decodes; a
   texture whose image does not decode is absent (`loadTextureImage`
   returns `null`), which changes the program as it does in three.js.
2. **Pose** (`pose::Pose::at`, `pose::world`). The mixer samples every
   clip; tracks bind by name to the runtime's clone (`PropertyBinding`:
   tracks of unnamed nodes target their original `uuid`, which the clone
   does not keep, so they do nothing). World matrices are multiplied in
   three.js's order under the layer matrix, the fit's scale, and −center.
3. **Instantiate** (`scene3d::gltf::instantiate`, from
   `SceneBuilder::build`). Each drawable becomes a `Mesh3d` with its world
   matrix, materials after `sync`'s rule (`Material3d` with a
   `MaterialExt`), geometry (`Geometry3d` with `VertexExtra`), and
   `MeshExt` (mode, skinning with the bone texture, instances, morph
   influences, culling sphere, object id). Embedded lights join
   `Scene3d::lights` after the scene's light layers (composition) or
   before them (legacy scene), as the runtime's scene order has them.
4. **Describe** (`describe::prepass`, `describe::color_passes`,
   `describe::whole_scene`). glTF draws use `gltf::material::parameters`
   and `uniforms` (`WebGLPrograms.getParameters` and `WebGLMaterials` for
   the five classes), object uniforms for skinning and morphs, and
   three.js's sorted lists; a transmission pass precedes the color pass.
5. **Execute** (`scene3d::gpu` with `scene3d::gltf::gpu`). Vertex inputs
   are bound by name (constant (0, 0, 0, 1) for attributes a geometry
   lacks, as WebGL), instance attributes per instance, `boneTexture` and
   `morphTargetsTexture` as RGBA f32 textures (the morph one as an array),
   glTF textures with their samplers, the transmission target per canvas
   size; point programs keep Metal's point size (`pm_gpu::msl::write_with`).

## Fidelity notes (today's behaviour reproduced)

- **Points and lines in every layer.** The per-layer render edits only
  `child.isMesh` materials, so points and lines keep their visibility and
  writes: every layer's prepass and color pass draws the points and lines
  of every visible object, with depth writes in both, regardless of depth
  sets. Reproduced (an output defect; appendix A candidate).
- **Embedded spot and directional lights aim at a stale target.** The
  runtime clone's `light.target` is a detached copy (`DirectionalLight.copy`
  clones the target without adding it), whose world matrix stays the one
  the loader computed in the glTF scene's rest pose (the origin for a light
  several nodes share). The direction is the placed light's position minus
  that point. Reproduced (`Model::light_targets`; appendix A candidate).
- **Unnamed animated nodes do not animate** (see Pose above; appendix A
  candidate).
- **The transmission target's clear** uses the canvas clear color while it
  is opaque (the GL clear color then holds sRGB-encoded values, so the
  background is cleared encoded into a linear target), else half-transparent
  white premultiplied; a legacy scene's background renders into it in
  linear. Opaque items there write no depth (the color phase turns depth
  writes off), so they draw in list order.
- **The depth phase's transmission pass** (three.js runs it in the prepass
  render too) has no visible effect and is not executed; the program test
  counts its 106 recorded draws separately.
- **History-dependent programs** are generated fresh, as the scene pass
  does: the shadow pass's shared depth and distance materials keep the map
  of their first build (pixel-irrelevant without alpha test); editor looks
  change `transparent` and `wireframe` without `needsUpdate`, so a look
  after other renders draws with older programs. The look cases render each
  look in a fresh runtime.
- **Skinned meshes are never frustum-culled** (three.js culls with the
  sphere of their first pose, which this engine does not keep); transparent
  skinned items sort by the geometry's sphere center.
- **`material.id` order** follows object then traversal order (clones in
  traversal order, replaced unlit materials after them), not the runtime's
  creation history across frames.
- **Mesh names** are made unique in loading order; three.js names meshes
  as their promises resolve. Names only matter for track binding, which
  resolves each unique name back to its object either way.

## Tests and measurements

Apple M4 Max, macOS 27, wgpu 30.0.1 (Metal, safe math), Chromium with
ANGLE on Metal and three.js 0.180.0 for references
(`tools/three-ref/run-gltf.ts`, capture `gltf-scene3d-20261017-074258`
outside the repository), 2026-10-17, load average about 9 to 10.

| Test | What | Result |
|---|---|---|
| `tests/scene3d_gltf.rs` sweep | 152 cases, 165 outputs (155 cases in the fixture; 3 Khronos files refused by both importers): the S7 synthetic models (skinned at 5 times, per-layer times, extension grid with and without shadows, object material, slots, wireframe, X-ray, transmission with and without background, WebP and AVIF, two models in a set with a scene object, a legacy scene) and 105 Khronos sample files at up to 4 times | All within the 3D bar: worst ≤ 2 CV fraction 99.934% (`TextureTransformMultiTest`), max outside MSAA edges 6 CV (`CompareIridescence`), min object SSIM 0.99973 (`CompareSheen`) |
| programs and uniforms | Every draw's program against the recorded programs, every uniform three.js set | 3,527 draws matched in three.js's order, 386,969 uniform values equal as f32; every recorded draw that writes color or depth matched (67 skinned draws three.js may cull, 106 depth-phase transmission draws counted apart) |
| poses | Drawable world matrices, morph influences, `bindMatrixInverse`, bone matrices (f32), instance matrices, embedded light positions and targets | 1,170 drawables, 178,450 values, largest relative difference 0 |
| picks | Ray grids on 20 cases (skinned at a moved pose, the extension grid with points, lines, and instances, `MeshPrimitiveModes`, `SimpleInstancing`, animated Khronos models) against three.js's raycast with current-pose skinned bounds | 7,584 rays (1,490 hits): all agree (layer, object, face or point index, distance within 1e-6) |
| content keys | Pose changes change the key; debug forms name images, bones, morphs, instances by key | Pass |
| `tests/corpus.rs` | Corpus `3d/gltf` (source and override materials, skinning, two clips) at 5 times and `3d/gltf-materials` at 2 times, captured through the app | 7 frames within the 3D bar (max 2 CV), all 3D corpus frames and the full corpus pass |
| `pm-geom3d` `tests/gltf_picks.rs` | A skinned strip: picks follow the pose; 1,000,000-vertex skinned model (S7's `bigSkinned`, release) | Load 55 ms, BVH 133 ms; first pick after a time change median 15.1 ms, max 15.9 ms (target 33); unchanged frame median 0.002 ms, max 0.013 ms (target 2) |
| `frame_time_gltf_at_1080p` (ignored, release) | Scene build and render of every output, 1920 × 1080, a new time per frame | `BrainStem` 7.9 ms median render (0.41 ms build), `ToyCar` 3.1 ms, extension grid 3.4 ms, transmission 3.4 ms, `Fox` 0.8 ms; not gated, loaded machine |
| scene pass regressions | `tests/scene3d.rs` sweep and programs, `scene3d_document`, `scene3d_resources`, `scene3d_mesh`, `scene3d_layering`, `three_programs`, `headless` | Pass |

Re-run after merging `feat/native-engine` at `ec8f05f2` (wave 4, with
the scene3d lane): the sweep (165 outputs, worst ≤ 2 CV fraction 99.934%
`TextureTransformMultiTest`, max outside edges 6 CV, min SSIM 0.99973
`CompareSheen`), programs and uniforms (3,527 draws, 386,969 values),
poses (1,170 drawables, difference 0), picks (7,584 rays agree), the
render corpus (235 frames, 0 failed; `3d/gltf` and `3d/gltf-materials`
max 2 CV), the scene pass regressions, `pm-geom3d`'s tests, and clippy
with `-D warnings` on `pm-geom3d`, `pm-gpu`, and `pm-render` all pass;
`pm-geom3d` checks for wasm32-unknown-unknown.

Khronos sample models are downloaded into the corpus directory
(`Powermove Engine Corpus/khronos-gltf/`, 108 files), never into the
repository.

## Gaps

- Picks and bounds are wired (docs/engine/scene3d.md, "Phase 3c: 3D
  questions"): `pm_eval::query3d` asks `pm_geom3d::gltf::pick` through
  pm-wasm with the editor camera, measured against today's runtime in
  dual-answer mode on skinned, morphed, and instanced Khronos models. The S7 lazy skinned pick (joint-bounded BVH
  nodes) is not ported; the full re-pose meets the targets natively.
  `pm-geom3d` with the glTF code builds for wasm32-unknown-unknown. Picks
  in wasm (V8) on 7 Khronos models plus a random composition, a new time
  per pick, editor view: p50 2.15 ms, p95 2.27 ms, max 3.73 ms
  (docs/engine/scene3d.md, "Phase 3c"). Since round 6 a model
  `pm_geom3d::gltf::load` rejects is the missing-model box for every 3D
  question and draw, as today's asset without a scene (see
  "Acceptance").
- Round 3: the 3D cursor's surface point uses the same kernel
  (`Scene::cursor_hit`: `Picker::pick_as_runtime` over a model's meshes
  only, with its points and lines skipped, as `placeCursor`'s
  `find(h => h.object.isMesh)` does), equal to three.js on 480 rays
  including skinned and instanced Khronos models
  (`scene3d-geometry.test.ts`). Edit Mode never reads a glTF model's mesh
  (`canEdit` refuses imported models); `objectMesh3d` answers `notMesh`
  for it, the runtime's group.
- The round 5 import adapter supplies `assets.get().object3d` from engine
  geometry when "Use New Engine" is on (see below). The adapter's browser
  comparison of textured models and exact `prepareImport` bytes passes.
- Skinned meshes are not culled and sort transparent items by the
  geometry's sphere (see Fidelity notes); three.js's history-dependent
  programs are generated fresh.
- Image decoding: AVIF goes through ImageIO (Chromium uses libavif and
  dav1d); 16-bit PNGs are reduced to 8 bits by rounding (Skia's exact
  reduction not measured); embedded JPEG EXIF orientation and a texture
  object shared between an sRGB and a linear slot (three.js's last color
  space wins) are not separately tested.
- Names of nodes outside the default scene do not take part in
  `createUniqueName`; shared-node attachment and root ownership across
  scenes follow r180's promise order since round 7 (see below).
- An imported scene over two million vertices: today's runtime throws in
  `sync`. Round 8 compares three accepted 600,000-vertex placements and
  the fourth placement's refusal in both app routes; see scene3d.md.
- Blender rendering of models is the Blender lane's.

## Changes outside the lane

- `pm-gpu/src/msl.rs`: `write_with` (point size for point programs).
- `pm-render/src/scene3d/`: `model.rs` (`MaterialKind` classes,
  `Material3d::ext`, `Geometry3d::extra`, `Mesh3d::gltf`,
  `ModelLookup::Gltf` and `Unloadable`, mixer times on `SceneBuilder`,
  instantiation and embedded light order in `build`, looks skip points and
  lines), `describe.rs` (texture kinds, topology and HDR draw state, glTF
  programs and uniforms, points and lines in every layer, sorted lists,
  the transmission pass, `color_passes`), `gpu.rs` (vertex inputs by name,
  instance steps, topologies, the half-float target, array and data
  textures, the transmission pass), `textures.rs` (`upload_rgba`),
  `layers.rs` (model lookup, static check, mixer times), `mod.rs`.
- `pm-render/tests/scene3d.rs` (the transmission pass in its draw list),
  `tests/scene3d_document.rs` (an offline imported model now draws the
  missing-model box instead of failing).
- `pm-render/tools/three-ref/scene-page.ts`: models through `loadGltf`,
  `sync`'s time and per-object mixer times, instanced draws recorded,
  pose and pick dumps.
- `engine/Cargo.lock`: the `gltf` crate in `pm-geom3d`.
- `pm-render/tests/motion.rs`: the wave 4 motion merge moved the
  request before cloning it; the unblurred render now clones it, so the
  test target compiles again.

## Phase 3c round 4: Blender snapshot integration

The Blender snapshot builder now consumes imported glTF geometry through
pm-geom3d's pose kernel (morphs and skins), source material factors/maps,
and the material-completion order used by today's async snapshot. All 23
retained imported-model snapshots now build and compare equal; the full
Blender run still fails six external AVIF map snapshots. See blender.md
for the full-depth TIFF conversion and the remaining gaps. The public
`assets.get().object3d` adapter and `prepareImport` backend are unchanged
and remain required work.

## Phase 3c round 5: engine import and the compatibility object

With "Use New Engine" on, `pm_geom3d::model_import::import` checks and
normalizes the selected glTF/GLB and `pm_model_import` exposes its scene
graph, attributes, indices, morph targets, skins, materials, embedded
images, cameras, lights, and animation through `pm-wasm`. The renderer's
`object3dFromEngine` builds a traversable, cloneable three.js `Object3D`
from that data, sharing repeated primitive geometry and material variants
as the loader does. It carries glTF extras and unhandled extensions into
`userData`, and reads original JSON material factors at their source
precision. The existing scene runtime and frozen apiVersion 1–3 extensions
can still consume `media.assets.get(id).object3d`; that property is
deprecated and an agent warning is emitted once per asset. The flag-on
import path does not invoke `GLTFLoader`.

`Scene3D.prepareImport` sends the selected OBJ, MTL, and image files to
`pm_prepare_model_import`. For a bundle that produces a GLB, the renderer
imports that GLB through the engine adapter and runs the existing
`GLTFExporter` with Chromium image encoding for the final asset bytes.
The original OBJ name and MTL numeric factors are restored before export.
For oversized bump maps, the original image is resized and converted to
a normal map by the existing Chromium canvas path before export. This
keeps the final encoder consistent with the flag-off importer while the
geometry and import checks come from the engine. Unchanged/invalid
selected-file outcomes follow the existing `prepareImport` return shape.

Verification as of 2026-10-19: two adapter unit tests and seven real
Khronos model comparisons pass against a test-only `GLTFLoader` reference.
Those comparisons cover hierarchy, attributes and indices, material
factors and sharing identity, `userData`, skin inverses, morphs, bounds,
clip interpolants, and animated world and bone transforms at multiple
times. The native `pm-wasm` check passes. In Electron, all eight
`prepareImport` fixtures produce GLBs byte-for-byte equal to the flag-off
importer: plain OBJ, PNG/JPEG/WebP color maps, texture transform,
multi-material, normal-size bump, and >2048-pixel bump resize. Six
textured Khronos models, including physical materials, match the flag-off
loader's object graph, materials, maps, image pixels, and `userData`.
The eight selected OBJ fixtures and six selected glTF models exercise
the current parity suite; they do not exhaust every valid input. Embedded
images currently cross the WASM boundary as base64 JSON, adding temporary
memory use for large models.

## Phase 3c round 6: acceptance

Which files load decides whether a model draws or is the orange
missing-model box, so the engine's readers must accept exactly the files
today's importer (`loadGltf`: three.js r180's GLTFLoader plus the import
budget) loads. `gltf-acceptance.test.ts` builds 53 boundary cases, each as
`.gltf` (data-URI buffer) and `.glb`, and runs today's `loadGltf` in Node
against both engine readers in wasm: the importer behind
`assets.get().object3d` (`importModel`, `pm_geom3d::model_import`) and the
renderer's and picks' reader (`setModel`, `pm_geom3d::gltf`). 106 files:
52 load and 54 are refused, identically by all three, and no reader traps.

Round 6 made the readers follow GLTFLoader where they did not:

| Case | Today | Before | Now |
|---|---|---|---|
| unknown `buffers[].type` | refuses (`loadBuffer`) | importer loaded | refused |
| buffer view past its buffer | loads (`ArrayBuffer.slice` clamps) | renderer refused | clamped; an accessor past the shorter view still refuses |
| matrix of 12, 15 or 17 values | `fromArray`: missing entries NaN; a NaN position makes the bounds NaN (refused); `decompose` ignores the w column | both refused | `fromArray` and three's `decompose` |
| scene node another node lists | `scene.add` moves it to the scene | renderer refused | moved, as listed (twice: to the end) |
| node camera out of range | `loadCamera` throws | both loaded | refused |
| too few inverse bind matrices | entries past the array are NaN | importer refused | NaN entries; bounds refuse if a weighted vertex uses one |
| a joint index past the skin with a weight other than 0 | `setFromObject` throws (`bones[i]` undefined) | both loaded | refused; a zero weight loads |
| a vertex attribute or morph target shorter than POSITION, an index past the last vertex | loads; WebGL draws nothing for the primitive | renderer refused | loaded, padded for safe reads, `Geometry::undrawable` (not drawn; counted, bounded and picked like three.js, triangles with an index past the end never hit) |
| sparse index past the accessor | `setX` past the end writes nothing | both refused | skipped |
| attribute of an unknown accessor type (`VEC5`) | an empty attribute | both refused | absent (POSITION still refuses) |
| unknown required extension | warns and loads | importer refused | loads |
| Draco or a missing material/light | throws | importer loaded | refused |
| animation channel without a node | skipped | importer refused (typed schema) | skipped |

The importer no longer parses with the `gltf` crate's typed schema; it
reads the raw JSON through the renderer reader's container, and its
acceptance is `pm_geom3d::gltf::load`'s (`scene::import` calls it first),
so `object3d` exists exactly when the engine draws the model.

**Round 6 known gap, closed in round 7: two-parent nodes.** GLTFLoader attaches a node two parents
list to whichever parent's `loadNode` promise settles last. That depends
on promise depth and on asynchronous image decoding, so it is not a
static function of the file. At the round 6 checkpoint both readers
refused such a file (the missing-model box); that historical acceptance
result is superseded below.

## Phase 3c round 7: short inputs and shared-node promises

The readers now retain original source counts for attributes and morph
targets while padding CPU arrays for safe bounds and poses. An index past
the vertex range always suppresses a WebGL draw. A short ordinary
attribute suppresses a draw only if that draw's *final* program consumes
that input and the draw fetches beyond the source count (an indexed draw
may leave a short tail unused). The plan retains the mesh; the draw rule
runs after the object's `useSourceMaterials` choice, including a
Basic-to-Standard replacement, and after scene maps that failed to decode
have been removed. A missing override map can therefore leave a short UV
inactive and let the mesh draw. The shadow depth/distance pass applies its
own program's inputs; a short normal can reject a Standard color draw yet
still cast a shadow. Chromium keeps the UV input active for a mapped depth
material even when its map is opaque and alpha testing is off; the fragment
color being unused does not make that vertex input inactive. The rule
considers active map UV
channels 0–3, lighting normals, tangents, and vertex colors. The
compatibility `Object3D` exposes `uv2` and `uv3`, too. Morph targets are
packed into the GPU array texture as `WebGLMorphtargets` does: a short
primary target leaves later texels at zero, and a shorter secondary
target inside that loop stores NaNs. This supersedes the combined
short-attribute/morph entry in the round 6 table, which describes its
dated checkpoint.

For a node listed by two parents, `pm_geom3d::gltf::promise_order`
simulates r180's cached `loadNode`, `_loadNodeShallow`, mesh, geometry,
accessor, buffer, skin, and scene Promise dependencies and runs their
attachment callbacks in microtask order. It also processes all scenes:
a later scene root attachment can move a node out of the default scene,
and the engine importer and renderer both omit that node from the
default scene. For live image decoding, the completion order can vary
between loads; the scheduler's deterministic image policy is decoded
images completed in request order, after URI and Blob construction. An
embedded GLB image waits for its bufferView, then schedules bitmap
completion as a task. Source and texture cache hits retain r180's distinct
Promise continuations. A deterministic test stubs ImageBitmapLoader's
decoded completion and compares a textured shared parent against a
competing data-URI buffer parent, including same-sampler texture and
different-sampler image-source cache hits. Multiple independent data-URI
buffer loads likewise use request order. Texture-bearing material
extension chains and live asynchronous image decoding beyond that policy
are not yet proven against all accepted inputs. A normal-only or color-only morph
with no position target hits an unconditional `morphTargets[i].count`
read in r180's texture packer and fails the legacy render. The native
renderer now returns that error when the mesh reaches a draw, while
import, geometry queries, and Blender scene construction still accept it.
Native render regressions and Chromium captures verify that draw-time
exception without turning the accepted import into a missing-model box.

As of 2026-10-19, a fresh WASM comparison against three.js r180's
`GLTFLoader` passes nine hand-selected shared-node cases, 192 generated
acyclic graphs across data-URI, sparse-accessor, and GLB encoding (with
camera attachments and cached meshes), a two-scene case where a
second buffer delays a scene and moves the shared node out of the
default scene, and data-URI/GLB cases with instancing, geometry cache
sharing, Standard/Physical/Unlit material chains, and an off-scene
animation target. The round 7 parent-order and acceptance tests pass
8/8 against the rebuilt WASM, including the decoded-image policy oracle.
The Chromium r180 short-attribute capture passed all 28 cases and saved
an image and active-program attribute list for each. In particular,
Standard material keeps `NORMAL` active with no lights and with only an
ambient light; both short-normal draws produced `INVALID_OPERATION`.
Depth and distance materials keep the copied opaque map's UV active but
leave short normals and vertex colors inactive. Native pixel regressions
verify both a short-normal caster's retained shadow and a mapped short-UV
caster's rejected shadow, as well as drawing after a late map failure.
The paired legacy/native shared-node and final-material capture is 100%
within 2 CV. Applying the production 3D bar to its PNGs, without any edge
exclusion, gives max 2 CV and object SSIM 0.99999768.

## Phase 3c round 8: consumed NORMAL inputs

Round 7's draw rule treated `NORMAL` as consumed only by unflat Standard
and Physical programs. Two parts of that were wrong against three r180:

- `meshbasic.glsl.js` includes `beginnormal_vertex` under
  `USE_ENVMAP || USE_SKINNING`, so a skinned (or env-mapped)
  MeshBasicMaterial binds `normal`. A skinned `KHR_materials_unlit`
  primitive with a short NORMAL therefore draws nothing today. The same
  holds for the viewport chrome: `overlay.ts` draws its grid-occlusion depth
  and outline mask with MeshBasicMaterial overrides, so a skinned glTF mesh
  with a short NORMAL has no outline and no grid occlusion today. The native
  chrome uses Basic overrides too (`chrome_gpu.rs`) and now applies the
  same rule to its depth, ID and mask draws.
- The physical vertex shader includes `beginnormal_vertex`
  unconditionally, so flat shading does not deactivate `normal`. Only
  `USE_TANGENT` depends on `flatShading === false`. No current file reaches
  this, because GLTFLoader and the compatibility `Object3D` set flatShading
  only when NORMAL is absent; the rule now matches the measured program.

`consumes_short_attribute` now reads: depth/distance programs consume
NORMAL only with a displacement map; Lambert/Phong/Standard/Physical
always; Basic (and LineBasic, which shares `meshbasic_vert`) only when
skinned or env-mapped. The four lit mesh shaders retain their normal input
under flat shading, morph normals, environment maps and normal maps.
Evidence: the Chromium probe (`e2e/scene3d-short-attributes.spec.ts`) has
36 cases, 8 of them new: skinned Basic with full and short normals,
env-mapped Basic, flat and skinned-flat Standard, and `overlay.ts`'s mask
override on unskinned short, skinned short and skinned full normals. Every
short case that binds `normal` gave `INVALID_OPERATION` and black; the
others drew red. `pm-render`'s `viewport_chrome` test renders the color
frame and all three chrome planes for skinned and unskinned, unlit and
Standard short-normal GLBs (complete-normal controls draw everywhere), and
fails against the round 7 rule.

`scene3d-normal-matrix.spec.ts` also captures the Cartesian product of
Basic, Lambert, Phong, Standard and Physical with skinning, morph normals,
environment map, flat shading and normal map. Each normal-map state has
tangent-space variants with and without vertex tangents, plus object-space
normal mapping. Every variant has a complete-normal control and a short
normal draw: 640 Chromium captures, 320 pairs. Morph-normal targets include
POSITION, so these exercise valid morph-texture packing. Environment maps
have a valid equirectangular size. Program reflection, defines, GL errors,
coverage and center pixels accompany every capture.

Of those pairs, 272 have drawable controls. Every active NORMAL rejects
the short buffer with `INVALID_OPERATION` and zero coverage; inactive
NORMAL draws exactly like its control. The remaining 48 pairs mutate
MeshBasicMaterial with a normalMap, which r180 does not support: even the
complete-normal control throws during uniform refresh before drawing.
They are explicitly recorded as pre-draw exceptions, rather than evidence
of short-attribute refusal. GLTFLoader excludes normalTexture when it
constructs MeshBasicMaterial, so imported unlit models cannot reach that
unsupported mutation. All combinations without that mutation are proven.

The synthetic program-reflection oracle lives in
`pm-render/tests/fixtures/short-normal-programs.json`; the native test checks
NORMAL, TANGENT and TEXCOORD_0 against all 272 valid programs and separately
counts the 48 invalid controls. Vertex tangents remain active only with a
normal map and without flat shading. No import-wide NORMAL refusal was
added: the importer retains original short lengths, and each final color,
shadow or chrome program decides whether it consumes the attribute.
