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
| Ordinary model/light/camera timeline layers | Yes, individual layers, no scene container | Evaluate main document into Studio scene | Pending |
| Existing scene-container/mesh project migration | Yes, upgrade on open | Keep main migration; render converted layers | Pending |
| Primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron | Yes | Mesh triangles and normals | Pending |
| Indexed mesh, lathe and polygon extrusion sources | Yes | Mesh triangles and UVs | Pending |
| glTF/GLB, OBJ import | Yes | Imported hierarchy converted to triangle instances | Pending |
| Embedded glTF animation/skinning | Yes, animation mixer at layer time | Evaluate deformed geometry before tracing | Pending |
| glTF source material names/indexes and textures | Yes | Material per geometry group, stable original index | Pending |
| Imported parts tree, picking and selection | Yes | Keep part identities in backend picks | Pending |
| Part position/rotation/scale/visibility offsets | Yes, sparse pt.KEY.FIELD channels | Evaluated part world matrices | Pending |
| Parent/group transforms, anchors, opacity and solo | Yes | Main world matrices and compositor | Pending |
| Per-layer and group 3D switches | Yes; models always 3D | Include enabled planes/extrusions only | Pending |
| 2D solids/images/video/text/shapes/precomps in 3D | Yes, shared textured planes | Main source raster becomes Studio plane texture | Pending |
| Flat text/shapes remain flat until extrusion enabled | Yes | Main extrusion/switch eligibility | Pending |
| Text outlines, font loading/fallback and holes | Yes | Main extrusion geometry builder | Pending |
| Shape extrusion, compound paths and holes | Yes | Main extrusion geometry builder | Pending |
| Animated extrusion depth/bevel and quality | Yes, geo.KEY | Evaluated geometry and geometry cache | Pending |
| Angular/concave/convex bevels, inner hole bevel depth | Yes | Main surface geometry | Pending |
| Front/Bevel/Side/Back independent materials | Yes, ms.SURFACE.KEY | Geometry groups mapped to distinct Studio materials | Pending |
| PBR color/roughness/metalness/emission/opacity | Yes, keyframeable | OpenPBR material channels | Pending |
| Color/normal/roughness/metalness/emissive/AO image maps | Yes | Studio texture array and UVs | Pending |
| Source materials versus whole-model overrides | Yes | Preserve sparse overrides and source defaults | Pending |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | Yes, mi.KEY and mi.INDEX.KEY | Evaluate main intrinsic channels | Pending |
| Live composition textures on named model materials | Yes, color/emissive | Evaluate composition at receiver time, upload texture | Pending |
| Live layer material sources in same composition | Yes; hidden source supported | Main composition-textures source raster | Pending |
| Layer material source timing, masks, effects; transform/opacity excluded | Yes, video speed/remap/in-point | Preserve source raster semantics | Pending |
| Material source missing/loading/relink/undo/save lifecycle | Yes | Preserve main asset/source handling | Pending |
| Sun/parallel, point and spot lights | Yes, animated position/target/color/intensity | Studio analytic lights | Pending |
| Light distance/decay, spot angle/penumbra and visibility | Yes | Translate evaluated light parameters | Pending |
| Light cast-shadow toggle, model cast/receive-shadow toggles | Yes | Backend visibility and shadow flags | Pending |
| Global shadows and ambient color/intensity/exposure | Yes | Studio environment and exposure | Pending |
| Light markers, select and move/target gizmos | Yes | Keep main viewer overlays and hit testing | Pending |
| Perspective and orthographic cameras | Yes | Studio camera rays for both projections | Pending |
| Animated camera position/target/FOV/zoom/near/far | Yes | Evaluated camera contract | Pending |
| Camera and free/editor view; orbit/pan/zoom/frame | Yes | Keep main navigation and matrices | Pending |
| Move/rotate/scale gizmos, scene/object axes, modal transforms | Yes | Keep main controls over shared geometry graph | Pending |
| Projected bounds, ray picking, model/part selection | Yes | Backend bounds and surface pick contract | Pending |
| Mixed 2D/3D depth sets and transparent layer ordering | Yes | One trace per depth set with per-layer output | Pending |
| Per-layer effects/masks/blend modes and opacity | Yes | Keep compositor after per-layer Studio image | Pending |
| Viewer, agent render_frames and still capture | Yes, shared compositor | All use Studio backend | Pending |
| Video/image-sequence export, nested composition and motion-blur sampling | Yes, prepareFrame/renderFrameTo | Final tier through the same backend | Pending |
| Extension api.scene3d and revision-guarded edit_3d/get_3d_scene | Yes | Keep main shapes; add Studio operations | Pending |
| Validation, budgets, undo/redo, serialization and asset bundles | Yes | Main validation and persistence | Pending |

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
| Parallel / spot / point lights | Yes (parallel named sun) | Analytic lights | Pending |
| Ambient / environment light types | Global ambient only, no dedicated layers | Environment illumination | Gap |
| Material Options: Casts Shadows | Model and light flags | Shadow-ray visibility | Pending |
| Material Options: Accepts Shadows | Model receiveShadow | Per-material direct-light shadow policy | Pending |
| Material Options: Accepts Lights | No dedicated switch | Unlit material policy required | Gap |
| Material Options: Appears in Reflections | No dedicated switch | Secondary-ray visibility required | Gap |
| Layers as materials, same-comp source | Yes; precomp/solid/image/video/text/shape | Live source texture | Pending |
| Keyframeable Opacity/Roughness/Metallic | Yes | Main evaluated material channels | Pending |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | Yes | Main evaluated material channels | Pending |
| Substance SBSAR | No | Substance runtime/import pipeline required | Gap |
| Front / Bevel / Side / Back | Yes for extruded text/shapes | Distinct materials per triangle group | Pending |
| Camera depth of field/focus | No | Studio thin lens from source branch | Pending |
| Camera iris controls | No | Aperture shape controls required | Gap |
| Multiple views | Single camera/free view, no simultaneous panels | Preserve navigation; multi-view UI required | Gap |
| glTF embedded animation | Yes | Pose evaluation before tracing | Pending |
| FBX with embedded animation | No | Importer required | Gap |
| 2D layers casting shadows onto 3D | Shared plane geometry supports casts | Textured alpha plane in shadow rays | Pending |
| Render passes | Internal per-layer/depth surfaces; no user export passes | AOV export interface required | Gap |

## Studio features to retain

| Capability | On main | How Studio renders it | studio-3d |
|---|---|---|---|
| Live / Refine / Final path tracing | No | Shared pm-studio WGSL | Pending |
| Fast / Quality preview | No | Both tiers use Studio, no three.js fallback | Pending |
| Finish presets and rendered swatches | No | Layered Studio material | Pending |
| Softbox lights and light-by-pointing | No | Area lights and surface normal pick | Pending |
| Auto studio, seamless set, shadow catcher | No | Scene setup and Studio floor | Pending |
| Shots and exploded views | No | Ordinary editable main keyframes | Pending |
| Drop to floor / place on surface | No | Bounds and placement edits | Pending |
| Live screens | Yes through material source UI | Reconcile with main named-material links | Pending |
| STEP/IGES/BREP CAD import | No | Worker tessellation to glTF | Pending |
| Gaussian splats and point clouds | No | Studio depth-aware splat composite | Pending |
| Advanced glass/coat/sheen/film/dispersion/droplets/brushed/subsurface | No | Layered Studio shader | Pending |
| Native backend attachment | No | Serializable Rust scene contract | Pending |

## Verification

Pending. Baseline and Studio screenshots belong outside Git at
`/tmp/pm-shots/studio3d/laneA/`. No screenshots or local operational reports
are part of this repository.
