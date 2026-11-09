# Studio 3D parity

Studio replaces the page's three.js renderer. The normal layer document,
timeline, inspectors and agent operations remain the editing source of truth.
This checklist distinguishes existing functionality from the requested AE 27.0
targets: an absent feature is not silently counted as preserved.

Status: **Pending** = integration not yet verified; **Kept** = implementation
preserved; **Verified** = covered by a passing check; **Gap** = not implemented.
Main baseline: `d7e9f9205`. Studio source: local `new-3d-system` (`b81b94c66`).

## Main capability inventory

| Capability | On main | How Studio renders it | studio-3d |
|---|---|---|---|
| Ordinary model/light/camera timeline layers | Yes, individual layers, no scene container | Evaluate main document into Studio scene | Verified |
| Existing scene-container/mesh project migration | Yes, upgrade on open | Keep main migration; render converted layers | Verified |
| Primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron | Yes | Mesh triangles and normals | Verified |
| Indexed mesh, lathe and polygon extrusion sources | Yes | Mesh triangles and UVs | Verified |
| glTF/GLB (including mesh instances), OBJ import | Yes | Imported hierarchy converted to triangle instances | Verified |
| Embedded glTF animation/skinning | Yes, animation mixer at layer time | Evaluate deformed geometry before tracing | Kept |
| glTF source material names/indexes, vertex colors and six standard texture slots | Yes | Material per geometry group, stable original index | Verified |
| Extended glTF texture transforms, secondary UVs and physical-material maps | Yes through three.js GLTFLoader/physical materials | Standard color/normal/roughness/metalness/emissive/AO slots work; extra clearcoat/transmission/thickness/sheen/specular/iridescence/anisotropy maps and UV transforms are not yet sampled | Gap |
| Imported parts tree, picking and selection | Yes | Keep main part picking and transport stable part IDs | Verified |
| Part position/rotation/scale/visibility offsets | Yes, sparse pt.KEY.FIELD channels | Evaluated part world matrices | Verified |
| Parent/group transforms, anchors, opacity and solo | Yes | Main world matrices and compositor | Verified |
| Per-layer and group 3D switches | Yes; models always 3D | Include enabled planes/extrusions only | Verified |
| 2D solids/images/video/text/shapes/precomps in 3D | Yes, shared textured planes | Main source raster becomes Studio plane texture | Verified |
| Flat text/shapes remain flat until extrusion enabled | Yes | Main extrusion/switch eligibility | Verified |
| Text outlines, font loading/fallback and holes | Yes | Main extrusion geometry builder | Verified |
| Shape extrusion, compound paths and holes | Yes | Main extrusion geometry builder | Verified |
| Animated extrusion depth/bevel and quality | Yes, geo.KEY | Evaluated geometry and geometry cache | Verified |
| Angular/concave/convex bevels, inner hole bevel depth | Yes | Main surface geometry | Verified |
| Front/Bevel/Side/Back independent materials | Yes, ms.SURFACE.KEY | Geometry groups mapped to distinct Studio materials | Verified |
| PBR color/roughness/metalness/emission/opacity | Yes, keyframeable | OpenPBR material channels | Verified |
| Color/normal/roughness/metalness/emissive/AO image maps | Yes | Studio texture array and UVs | Verified |
| Source materials versus whole-model overrides | Yes | Preserve sparse overrides and source defaults | Verified |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | Yes, mi.KEY and mi.INDEX.KEY | Evaluate main intrinsic channels | Verified |
| Live composition textures on named model materials | Yes, color/emissive | Evaluate composition at receiver time, upload texture | Verified |
| Live layer material sources in same composition | Yes; hidden source supported | Main composition-textures source raster | Verified |
| Layer material source timing, masks, effects; transform/opacity excluded | Yes, video speed/remap/in-point | Preserve source raster semantics | Verified |
| Material source missing/loading/relink/undo/save lifecycle | Yes | Preserve main asset/source handling | Verified |
| Sun/parallel, point and spot lights | Yes, animated position/target/color/intensity | Studio analytic lights | Verified |
| Light distance/decay, spot angle/penumbra and visibility | Yes | Translate evaluated light parameters | Verified |
| Light cast-shadow toggle, model cast/receive-shadow toggles | Yes | Backend visibility and shadow flags | Verified |
| Global shadows and ambient color/intensity/exposure | Yes | Studio environment and exposure | Verified |
| Light markers, select and move/target gizmos | Yes | Keep main viewer overlays and hit testing | Verified |
| Perspective and orthographic cameras | Yes | Studio camera rays for both projections | Verified |
| Animated camera position/target/FOV/zoom/near/far | Yes | Evaluated camera contract | Verified |
| Camera and free/editor view; orbit/pan/zoom/frame | Yes | Keep main navigation and matrices | Verified |
| Move/rotate/scale gizmos, scene/object axes, modal transforms | Yes | Keep main controls over shared geometry graph | Verified |
| Projected bounds, ray picking, model/part selection | Yes | Keep main bounds/picking; expose optional backend equivalents | Verified |
| Mixed 2D/3D depth sets and transparent layer ordering | Yes | Shared opaque trace; separate primary views for blended layers | Verified |
| Per-layer effects/masks/blend modes and opacity | Yes | Keep compositor after per-layer Studio image | Verified |
| Viewer, agent render_frames and still capture | Yes, shared compositor | All use Studio backend | Verified |
| Video/image-sequence export, nested composition and motion-blur sampling | Yes, prepareFrame/renderFrameTo | Final tier through the same backend | Verified |
| Extension api.scene3d and revision-guarded edit_3d/get_3d_scene | Yes | Keep main shapes; add Studio operations | Verified |
| Validation, budgets, undo/redo, serialization and asset bundles | Yes | Main validation and persistence | Verified |

Inventory sources: `apps/desktop/src/extensions/layers-3d`,
`apps/desktop/src/extensions/viewer`, `apps/desktop/src/renderer/src/core/scene3d`,
`legacy/core/space-3d.ts`, `legacy/gl/compositor.ts`, `legacy/gl/raster.ts`,
`apps/desktop/src/main/agent-tools/spec.ts`, and `apps/desktop/e2e/scene3d*.spec.ts`.

## Requested AE 27.0 checklist

These are acceptance targets supplied for this integration, not a claim that
all of them already exist in Powermove.

| Capability | On main | How Studio renders it | studio-3d |
|---|---|---|---|
| Environment Light and HDRI | Uniform hemisphere only; no HDRI | Environment texture sampling required | Gap |
| Parallel / spot / point lights | Yes (parallel named sun) | Analytic lights | Verified |
| Ambient / environment light types | Global ambient only, no dedicated layers | Environment illumination | Gap |
| Material Options: Casts Shadows | Model and light flags | Shadow-ray visibility | Verified |
| Material Options: Accepts Shadows | Model receiveShadow | Per-material direct-light shadow policy | Verified |
| Material Options: Accepts Lights | No dedicated switch | Unlit material policy required | Gap |
| Material Options: Appears in Reflections | No dedicated switch | Secondary-ray visibility required | Gap |
| Layers as materials, same-comp source | Yes; precomp/solid/image/video/text/shape | Live source texture | Verified |
| Keyframeable Opacity/Roughness/Metallic | Yes | Main evaluated material channels | Verified |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | Yes | Main evaluated material channels | Verified |
| Substance SBSAR | No | Substance runtime/import pipeline required | Gap |
| Front / Bevel / Side / Back | Yes for extruded text/shapes | Distinct materials per triangle group | Verified |
| Camera depth of field/focus | No | Studio thin lens from source branch | Kept |
| Camera iris controls | No | Aperture shape controls required | Gap |
| Multiple views | Single camera/free view, no simultaneous panels | Preserve navigation; multi-view UI required | Gap |
| glTF embedded animation | Yes | Pose evaluation before tracing | Kept |
| FBX with embedded animation | No | Importer required | Gap |
| 2D layers casting shadows onto 3D | No; native-plane casts explicitly disabled | Alpha shadow rays are supported; native-plane casts remain disabled | Gap |
| Render passes | Internal per-layer/depth surfaces; no user export passes | AOV export interface required | Gap |

## Studio features to retain

| Capability | On main | How Studio renders it | studio-3d |
|---|---|---|---|
| Live / Refine / Final path tracing | No | Shared pm-studio WGSL | Verified |
| Fast / Quality preview | No | Both tiers use Studio, no three.js fallback | Verified |
| Finish presets and rendered swatches | No | Layered Studio material | Verified |
| Softbox lights and light-by-pointing | No | Area lights and surface normal pick | Verified |
| Auto studio, seamless set, shadow catcher | No | Scene setup and Studio floor | Verified |
| Shots and exploded views | No | Ordinary editable main keyframes | Verified |
| Drop to floor / place on surface | No | Bounds and placement edits | Verified |
| Live screens | Yes through material source UI | Reconcile with main named-material links | Verified |
| STEP/IGES/BREP CAD import | No | Worker tessellation to glTF | Verified |
| Gaussian splats and point clouds | No | Studio depth-aware splat composite | Verified |
| Advanced glass/coat/sheen/film/dispersion/droplets/brushed/subsurface | No | Layered Studio shader; glass/caustics tested, full artistic range needs visual grading | Kept |
| Native backend attachment | No | Serializable Rust scene contract, native Metal process and shared-texture adapter, plus WebGPU recovery | Verified |

## Verification

The WebGPU baseline passed all 53 targeted 3D Electron checks in one serial
run. The combined native integration also covers the main editor, source
materials, masked compositing, camera projection, shutter samples and real
shared-texture transfers; native film validation uses the same export path.

Checklist totals: **59 verified**, **4 kept**, **0 pending**, **11 gaps**.

| Area | Verified | Kept, further visual coverage needed | Gaps |
|---|---:|---:|---:|
| Main capabilities (43) | 41 | 1 | 1 |
| Requested AE targets (19) | 7 | 2 | 10 |
| Studio features (12) | 11 | 1 | 0 |

- Main 3D, source layers, extrusion, lights, parts, switches, camera, masks and
  instancing: 38 Electron checks pass. `scene3d*.spec.ts`, `studio-parity`,
  `studio-camera`, `studio-compositing`, `studio-instances`.
- Geometry/material extraction, document operations, migration, async backends
  and final-frame preparation: 281 unit checks pass across 31 files, including
  native process/transport recovery.
- Studio quality, deterministic Final, temporal Live, caustics and denoising:
  five Electron checks pass in `studio.spec.ts`.
- CAD import, large worker preparation, splats, animated screens, light tool,
  finishes, placement and Shots have individual passing Electron checks.
- Export checks cover a six-second product film, a 25-second nested-composition
  film, and deterministic motion blur. Async shutter transport has its own
  regression check; shutter times reuse a backend session.
- Rust: 22 library, host and GPU checks pass with OIDN and MetalFX enabled.
- Type checking has zero errors; one existing IconCropper accessibility warning.

Viewer and export use the same Studio compositor. three.js remains for model
loading, pose evaluation, geometry, picking and controls; it no longer draws
mesh-layer images. Splats use a separate depth-aware WebGL2 splat pass.

The native implementation from `studio-3d-native` is integrated into this
branch, with main's editing UI and Lane A's dispatch/extractor retained. The
real editor receives native shared textures, keeps newly expanded depth sets
native, and propagates Final sample budgets into nested 3D compositions.
Native initialization/rendering failures recover through WebGPU. See
[native measurements and limits](native-host.md); standalone benchmark rates
do not establish full-editor 60 fps or Blender Cycles equivalence.

Extended imported-material sampling is a known main-renderer parity gap.
The six slots exposed by Powermove's Materials UI are covered, but models using
extra glTF physical-material maps, texture transforms or secondary UV sets can
look different. The texture-array upload currently samples images at 1024². Standard imported anisotropy and dispersion scalar values are
retained. The broader glass/coat/sheen/film/dispersion family also needs further
visual grading; this table does not claim full source-material fidelity.

The audit's AE targets include features absent on main. They remain explicit
gaps rather than being counted as regressions. Environment/world authoring,
HDRI and asset scattering still need additional product work; geometry
instancing now avoids rebuilding a BLAS for every repeated model.

Baseline and Studio PNGs live outside Git at
`/tmp/pm-shots/studio3d/laneA/`, with captions. No screenshots or local
operational reports are part of this repository.

### Reproduce the checks

From the repository root (Electron tests run serially):

```sh
nice -n 10 bun run --cwd apps/desktop test:unit \
  src/renderer/src/core/scene3d src/extensions/layers-3d \
  src/renderer/src/core/studio src/renderer/src/core/studio-native \
  src/main/studio-native \
  src/renderer/src/legacy/core/space-3d.test.ts \
  src/renderer/src/legacy/core/groups-3d.test.ts \
  src/renderer/src/legacy/core/switch-3d-migration.test.ts \
  src/renderer/src/legacy/core/frame-preparation.test.ts \
  src/renderer/src/legacy/core/agent-frame-capture.test.ts

env -u ELECTRON_RUN_AS_NODE POWERMOVE_BACKGROUND_TEST=1 \
  nice -n 10 bun run --cwd apps/desktop test:e2e \
  scene3d studio --grep-invert "product film" --global-timeout=1200000

env -u ELECTRON_RUN_AS_NODE POWERMOVE_BACKGROUND_TEST=1 \
  nice -n 10 bun run --cwd apps/desktop test:e2e \
  studio-film.spec.ts studio-film-extended.spec.ts --global-timeout=1800000

nice -n 10 bun run --cwd apps/desktop typecheck

CARGO_TARGET_DIR="$HOME/Developer/pm-target" nice -n 10 "$HOME/.cargo/bin/cargo" test \
  --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
```

The Electron harness creates and removes isolated temporary user-data folders.
`PM_SHOTS` and `PM_FILM_STILLS` optionally direct the new comparison and film
PNGs to an external folder; those artifacts must never be committed.
