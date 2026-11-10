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
| Extended glTF sampling, texture transforms and physical-material maps | Yes through three.js GLTFLoader/physical materials | Eighteen map slots with independent affine UV transforms, UV0/UV1, wrapping, normal scales and physical scalars; LDR KTX2/Basis worker; memory-budgeted images up to 4096² | Verified |
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

## Material-source parity with main reviews #187 / #188

Studio carries the independent material, bounds and navigation changes from
#187 (`c6549c519`) and #188 (`cfd068abe`) on its native/WebGPU path. No
three.js picture renderer is reintroduced.

| Capability | Studio implementation | Regression coverage |
| --- | --- | --- |
| Groups as Base Color or Emissive | Same-composition group subtree, hidden root, nested child timing/visibility, transforms, opacity, masks and effects through the ordinary compositor | `material-parity`, `layer-materials`, `studio-group-materials` |
| Cycles and local links | Group membership, nested compositions, effect inputs, single and stacked mattes participate in validation; source lookup stays local to the receiver | `material-parity`, `layer-materials` |
| Stable group source canvas | Union of descendant bounds across the group's duration on the composition frame grid, plus property key times; cached until edits/assets/fonts change | `material-source-bounds` |
| Safe group capture | Groups with 3D descendants, including nested precomps, are rejected before scene capture | `material-parity` |
| Camera/UV-driven detail | Cached UV patch footprints evaluated with the final viewer/export camera; bucketed requests retain detail during small camera moves | `material-resolution` |
| Capture budgets | At most 4,194,304 pixels per capture, additionally limited by the preview allowance divided by 64 working bytes/pixel and by device dimensions; direct oversized calls reject before allocation | `material-parity`, `material-resolution`, `compositor` |
| Texture sharing/replacement | Matching source and UV/sampling settings share storage across a model's meshes and slots; replacement updates all 18 supported slots and fallback bindings before disposal | `material-parity`, `material-performance`, `material-resolution` |
| Static capture reuse | Static source dependency trees retain pixels across playback and receiver-only edits; entrances/exits, descendant edits, fonts, assets, matte/effect programs and source clocks invalidate as needed | `material-performance` |
| Evaluated bounds and bounds null | Mesh vertices include posed parts, morphs and skin; one nine-position identity null preserves existing channels, timing, parent/group branches and Undo; null outlines use the real viewer camera | `geometry-bounds`, `studio-bounds-null`, `studio-bounds-null-placement` |
| Comp navigation clocks | Offset, trim, speed and remapping carry playhead time into/out of nested precomps, including repeated/frozen instances | `compositions` |
| Imported paint alpha | SVG/Figma RGB and alpha remain separate; extrusion vertex/default paint alpha reaches both renderers | `material-parity`, `convert`, `extrusion` |
| Path visual bounds | Segment half-width, actual joins/caps and transforms determine selection extent; bitmap padding stays outside geometry/pivot bounds | `material-parity`, `compositor` |

Group duration bounds cover authored frame samples and property key times;
arbitrary expressions can have unsampled sub-frame extrema. Capture-budget
reductions preserve aspect ratio and can intentionally soften extreme closeups.
A zero preview-cache allowance disables retained captures; transient captures
still obey the hard pixel cap.
Shared textures keep separate storage when UV transforms, wrapping, orientation
or color-space requirements differ. This is functional parity evidence, not
new performance acceptance or old-project appearance approval.

After #187 merges: reconcile its `material-resolution`, `geometry-bounds`,
`control-space`, group graph/compositor and null-overlay helpers with these
Studio ports. Preserve Studio's pixel/working-byte cap, stable duration canvas,
3D-descendant rejection and all-slot replacement. Bring over main's broader
SVG/Figma stroke/compound-fill fidelity changes as one importer integration;
this lane only changes paint alpha and measured path extent.

After #188 merges: reconcile `material-static`, `composition-textures`,
`composition-time` and the precomp-open call sites, retaining source-only
invalidation and instance-aware clocks. No listed Studio material/bounds/time
capability is waiting for either merge; these notes describe merge reconciliation.

## Requested AE 27.0 checklist

These are acceptance targets supplied for this integration, not a claim that
all of them already exist in Powermove.

| Capability | On main | How Studio renders it | studio-3d |
|---|---|---|---|
| Environment Light and HDRI | Uniform hemisphere only; no HDRI | World sky/HDR/EXR or live comp/layer source; shared environment sampling | Verified |
| Parallel / spot / point lights | Yes (parallel named sun) | Analytic lights | Verified |
| Ambient / environment light types | Global ambient only, no dedicated layers | Dedicated ambient diffuse illumination and environment source layers; parallel aliases sun | Verified |
| Material Options: Casts Shadows | Model and light flags | Keyframeable Off/On/Only, independent primary/shadow visibility | Verified |
| Material Options: Accepts Shadows | Model receiveShadow | Per-material direct-light shadow policy | Verified |
| Material Options: Accepts Lights | No dedicated switch | Keyframeable unlit base-color policy on models and native 3D artwork | Verified |
| Material Options: Appears in Reflections | No dedicated switch | Keyframeable secondary camera-path visibility; shadow policy remains independent | Verified |
| Layers as materials, same-comp source | Yes; precomp/solid/image/video/text/shape | Live source texture | Verified |
| Keyframeable Opacity/Roughness/Metallic | Yes | Main evaluated material channels | Verified |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | Yes | Main evaluated material channels | Verified |
| Substance SBSAR | No | Substance runtime/import pipeline required | Gap |
| Front / Bevel / Side / Back | Yes for extruded text/shapes | Distinct materials per triangle group | Verified |
| Camera depth of field/focus | No | Studio thin lens from source branch | Kept |
| Camera iris controls | No | Keyframeable blade count, rotation, roundness, aspect; shared uniform aperture sampling | Verified |
| Multiple views | Single camera/free view, no simultaneous panels | 1/2/4 View layouts; each pane looks through the Active Camera, Front/Left/Top/Back/Right/Bottom or a Custom View; the selected pane is the interactive viewer, the others are Studio Live pictures | Verified |
| glTF embedded animation | Yes | Pose evaluation before tracing | Kept |
| FBX with embedded animation | No | ufbx native worker converts meshes/materials/cameras/lights, skins and baked TRS tracks to durable GLB; existing parts/animation path | Verified |
| 2D layers casting shadows onto 3D | No; native-plane casts explicitly disabled | Native 3D planes cast alpha shadows and support receiving light/shadows through Material Options | Verified |
| Render passes | Internal per-layer/depth surfaces; no user export passes | Beauty, depth, world normal, albedo, object/material ID and shadow as true RGBA16 PNG sequences | Verified |

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

The 2026-11-09 import/material pass covers Inter, Geist and Geist Mono variable
font instances, a static font, invalid-layer isolation, generated static and
skinned animated FBX, saved-asset reopening, all eighteen physical texture
slots, transformed UV1 sampling and LDR KTX2/Basis decoding. The Rust package
run passes **87 checks** with OIDN and MetalFX, including both GPU traversal
paths, lit layer IDs/coverage, Beer-Lambert attenuation and the optional
Blender comparison. CPU skin evaluation plus BVH preparation measures
20.5 ms median / 27.8 ms p95 for 10,000 triangles over 60 poses on the base M5;
rendering and IPC are excluded. The benchmark script retains the measurement
outside Git.

The complete desktop unit run passes **7,368 tests**, with seven skipped
(719 files passed, three skipped). This includes the preload Electron mock,
Codex IPC contract and native surface authentication tests. The surface test
builds its addon from this checkout, avoiding stale packaged artifacts.
Svelte and main/preload type checks pass with zero errors and the existing
IconCropper accessibility warning. Built-in extension version checks pass.

The complete non-film 3D Electron run passes **71 of 71 cases**, including the
optional UI screenshot review, with a freshly built native host. The font,
FBX and KTX2/glTF regressions explicitly assert native Studio on macOS; FBX
also checks the rendered pose and retained clips after saving/reopening.
Renders and captions stay outside Git. Product-film exports were excluded
from this pass with `--grep-invert 'product film'`.

The black Inter frame came from fixed samples per outline curve multiplying
through bevel rings until the extrusion exceeded its 500,000-vertex budget.
Adaptive curve flattening keeps the packaged-font regression below 250,000
vertices. Font preparation, extrusion and mesh/material extraction isolate
malformed layers, roll back their resources and log a diagnostic with the layer
ID, preserving healthy layers in the same frame.

Historical integration evidence follows; these smaller counts predate the
import/material pass.

The earlier integration baseline passed **55 3D Electron checks**: 53 non-film cases in
one serial run, plus the two export films in a separate run. The extended film
uses the corrected Final quality through nested shot compositions. The earlier
WebGPU baseline also passed its complete 53-case matrix.

Checklist totals: **69 verified**, **4 kept**, **0 pending**, **1 gap**.

| Area | Verified | Kept, further visual coverage needed | Gaps |
|---|---:|---:|---:|
| Main capabilities (43) | 42 | 1 | 0 |
| Requested AE targets (19) | 16 | 2 | 1 |
| Studio features (12) | 11 | 1 | 0 |

- Main 3D, source layers, extrusion, lights, parts, switches, camera, masks and
  instancing: 38 Electron checks pass. `scene3d*.spec.ts`, `studio-parity`,
  `studio-camera`, `studio-compositing`, `studio-instances`.
- Geometry/material extraction, document operations, migration, async backends
  and final-frame preparation: 282 unit checks pass across 31 files, including
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
- The native Electron harness confirms hardware ray tracing, Live
  `sharedTexture`, empty-scene handling, GPU picks and native/WebGPU pixel
  parity. Killing its host recovers through WebGPU in 47.5 ms in this run.
- Desktop build and built-in version checks pass; staged secret scans pass.

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

Extended glTF import preserves specular/IOR/volume/clearcoat/sheen/iridescence/
anisotropy/emissive-strength/transmission factors and maps. All eighteen slots
retain per-map UV0/UV1, affine texture transforms and wrapping; clearcoat has an
independent normal. Material images reach 4096² within a 256 MB array budget,
reduced according to device-memory hints and resident geometry. LDR ETC1S/UASTC
KTX2/Basis images decode in an isolated worker. Whole-model finishes clear the
source physical settings and maps; source mode restores them. Limits remain
explicit: UV2+, KTX2 HDR/arrays/cubes and nested volume-medium stacks are not
implemented. The broader artistic glass/coat/sheen/film/dispersion family still
needs further visual grading; this table does not claim Cycles equivalence.

The audit's AE targets include features absent on main. They remain explicit
gaps rather than being counted as regressions. Environment/world, fog, terrain and scatter authoring now have document and
agent contracts and inspector/menu UI (Add 3D Layer, World, Fog Volume,
Terrain, Scatter, Material Options, iris and Export render passes). Geometry
instancing avoids rebuilding a BLAS for every repeated model.

Baseline and Studio PNGs live outside Git at
`/tmp/pm-shots/studio3d/laneA/`, with captions. No screenshots or local
operational reports are part of this repository.

### Reproduce the checks

From the repository root (Electron tests run serially):

```sh
nice -n 10 bun run --cwd apps/desktop test:unit

env -u ELECTRON_RUN_AS_NODE POWERMOVE_BACKGROUND_TEST=1 \
  PM_SHOT_DIR=/tmp/pm-shots/studio3d/ui \
  CARGO_TARGET_DIR="$HOME/Developer/pm-target" CARGO_PROFILE_RELEASE_DEBUG=0 \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
  nice -n 10 bun run --cwd apps/desktop test:e2e \
  scene3d studio --grep-invert "product film" --global-timeout=1800000

env -u ELECTRON_RUN_AS_NODE POWERMOVE_BACKGROUND_TEST=1 \
  nice -n 10 bun run --cwd apps/desktop test:e2e \
  studio-film.spec.ts studio-film-extended.spec.ts --global-timeout=1800000

nice -n 10 bun run --cwd apps/desktop typecheck

CARGO_TARGET_DIR="$HOME/Developer/pm-target" CARGO_BUILD_JOBS=2 \
  CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 \
  nice -n 10 "$HOME/.cargo/bin/cargo" test \
  --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
```

The Electron harness rebuilds the native host and creates/removes isolated
temporary user-data folders.
`PM_SHOTS` and `PM_FILM_STILLS` optionally direct the new comparison and film
PNGs to an external folder; those artifacts must never be committed.


### Environment document integration

The environment engine branch is integrated. A composition's World is an
ordinary layer independent of the active camera. Fog Volume, Terrain and
Scatter layers use the same document/edit boundary, keyframes, Undo, locks,
serialization, protected channels and revision checks. Sky presets, HDR/EXR,
live composition/layer sources, height fog, local volumes, procedural/image
terrain with slope/height materials, and seeded surface scatter reach both
native and WebGPU through the same prepared buffers. Hidden scatter prototypes
remain usable; missing sources clear their copies and Undo restores them.
See [the document/agent contract and examples](../../apps/desktop/docs/STUDIO.md#environment-document-and-agent-contract).

The earlier environment acceptance suite added four Electron cases. Its
non-film 3D command included 57 cases. Its targeted unit suite
contained 356 checks across 39 files; Rust had 57 library/host/GPU/reference checks
after integrating the Cycles comparison and lighting-accuracy branch. Its
optional Blender regression and six Python metric/LUT checks pass. See the
[Cycles yardstick and measured limits](cycles-yardstick.md). The four environment
acceptance cases also pass with native transport disabled, forcing WebGPU.
Both additional export films pass: the six-second product film and the
25-second film spanning eleven still checkpoints. Film stills use asynchronous
Final preparation; its initial readiness check seeks to a 3D shot because the
opening 2D card does not need to retain native sessions.
Typecheck has zero errors and the existing IconCropper accessibility warning.
The native/WebGPU transport harness passes hero and room comparisons (mean
RGB differences below 0.006 code values out of 255), empty-scene transitions
and native-process crash recovery. These checks do not establish Cycles parity
or a full-editor performance guarantee.

Additional verification covers actual HDR pixels above 1, live hidden-layer
lighting, grayscale height input, deterministic seeded instances, source-cycle
and stale-revision rejection, shadow-only alpha artwork, all seven 16-bit
pass files through the normal sequence delivery, and deterministic iris
shape/rotation/aspect changes in Final. Rust GPU tests check dedicated Ambient
energy and reflection visibility independently with portable and hardware
traversal. PNG byte checks verify a 16-bit IHDR and samples beyond 8-bit
precision. Captures and captions are outside Git in
`/tmp/pm-shots/studio3d/laneF/`.

Remaining implementation limits are explicit:

- Multi-view panes other than the selected one are pictures: they render at
  Studio Live quality without gizmos; one Custom View per pane, not AE's
  three shared Custom Views.
- Fog boxes use world axis-aligned bounding extents when rotated. Sphere
  non-uniform scale uses its largest axis.
- Reflection visibility excludes all secondary camera paths, including
  transmission/indirect paths. It preserves primary visibility and the
  independent shadow flag.
- Image-sequence passes are the evaluated 3D world. They omit later 2D
  compositor effects/overlays and motion blur; data passes use a center ray
  with a 0.5 opacity threshold. Shadow is a 32-sample direct analytic shadow
  fraction. Auxiliary passes run the shared portable shader on WebGPU even
  when the viewer uses native Metal. Per-frame metadata records ID mappings.
- EXR output and iris diffraction fringes are not implemented. PNG output is
  genuinely 16 bit, from floating GPU buffers, without RGBA8 expansion.
- Sources/modes/preset selection are structural edits; evaluated settings use
  ordinary timeline channels. EXR input assumes linear sRGB/Rec.709.


Natural environment defaults now include smooth camera-dependent terrain grids,
thermal erosion, blended procedural ground, deterministic branching plants,
grass and displaced rocks, plus shared distance-based mesh LOD. Leaves use a
thin diffuse transmission closure. These are shared native/WebGPU features;
local GPU terrain tessellation, hydraulic erosion, continuous LOD crossfades,
HDRI ground projection and production-asset/Cycles equivalence remain gaps.
See [the environment contract](environments.md#natural-landscape-content).
