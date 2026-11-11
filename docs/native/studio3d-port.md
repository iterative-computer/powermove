# Studio 3D native port

Implementation handoff for the engine/data lane. Opus owns all presentation.
This lane adds state, commands and worker adapters only.

## Source and boundaries

- Native base: `a93fee099bbaa09519640220388a77879bcdbfc7`.
- Pinned Studio source: `bd30e2037c840e162e4cf2694851f30679bec8f1` (`origin/studio-3d`).
- Import `engine/crates/pm-studio` whole. Its host package is named
  `pm-studio-host`, but its source is `engine/hosts/studio-electron`.
- Native is the shipping target; the Electron branch is frozen. Renderer
  changes stay under `engine/`, native integration under `apps/native/`.
  Cherry-pick compatibility with Electron is no longer a requirement.
- One in-process renderer on the viewer worker and shared wgpu device. Output
  flows through the existing compositor and below-slot EDR CAMetalLayer.
  No Electron helper or IPC texture transport. Blender is test-only.
- Retain portable WGSL traversal when Metal ray queries are unavailable.

## Approved gates and current priority

Owner decisions, 2026-11-11:

- Native Live: **at least 60 fps presented**, with **no stall >=100 ms**.
- Input to presented pixel: **p95 <=50 ms**.
- 1080p product-shot export budgets: **Draft <=1 s/frame**,
  **Production <=3 s/frame**, **Final unlimited**. Publish an export time
  estimate as state for the designer; estimates are not fixed render deadlines.
- Memory: flat trend over **two fixed projects**. A growing-catalog run does
  not establish this gate.
- v1.1: full USDZ import and Cycles matching for fog, subsurface, sheen and sky.
  The chrome caustic grain fix remains in scope.
- On-screen timing remains allowed under the build lock throughout the day;
  pause it whenever `/tmp/pm-shots/USER_ACTIVE` exists.

The UI lane's round-one handoff is `feat/na-studio3d-ui` at `abb01b5b8`,
`docs/native/design/reviews/na-studio3d-ui.md`. Engine/data work proceeds before
remaining noise/performance work in this order: (1) viewer surface picking,
(2) per-layer failure isolation, (3) persisted/undoable text extrusion and
Front/Bevel/Side/Back materials, (4) layer/group/composition material capture,
(5) one/two/four views with per-view camera and tier, (6) Studio backdrop,
(7) framed, grounded import placement. Round two (`1107d800a`) confirmed
picking and failure isolation in the app. Immediately after import placement:
fix Quality fog making the foreground black, the shadow catcher's black void
(possibly shared with the Studio backdrop), and Fast terrain rendering darker
than Quality. Then resume noise/edges and the approved native performance gates.
This lane adds no visual controls.

## Capability audit

Every capability row comes from the pinned `docs/studio/parity.md`, including
its material-source review table. Electron verification does not establish
native parity. **P** = partial baseline plumbing; **G** = known gap;
**U** = unverified. **K** = existing kernel persistence/history machinery,
but this capability still needs a native regression. UI hooks name data and
commands for the designer, not panel designs.

Baseline evidence: `scene3d/src/{schema,operations,service}.rs`,
`kernel/src/commands/scene3d.rs`, `engine-service/src/studio/native.rs`,
`pm-render/src/scene3d/provider.rs`, and `docs/native/{scene3d,studio}.md`.
The schema registers only model/light/camera, the extractor supports four
basic map bindings, and the old renderer facade is not the pinned engine.

**Checkpoint:** the pinned engine and host are imported whole, and the native
worker adapter compiles in the GPUI app. Live uses an in-process MetalFX GPU
texture, with portable temporal filtering as fallback. Refine/Final share one
resolved/denoised image across per-layer compositor targets. Viewer, headless
capture and windowless export use the same native seam. The newer document
contract and quality bridge are integrated after their targeted checks and the
GPUI binary build passed. World sky/HDRI/atmosphere, transformed fog, terrain
regions/erosion, all six nature species and seeded surface scatter now render.
Hidden prototypes and distance LOD retain shared geometry.

## Main capability inventory

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Ordinary model/light/camera timeline layers | Shared engine present; row integration unverified | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Existing scene-container/mesh project migration | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Indexed mesh, lathe and polygon extrusion sources | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| glTF/GLB (including mesh instances), OBJ import | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Embedded glTF animation/skinning | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| glTF source material names/indexes, vertex colors and six standard texture slots | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Extended glTF sampling, texture transforms and physical-material maps | Native 18-slot/UV/transform bindings checked; broader format parity pending | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Imported parts tree, picking and selection | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Part position/rotation/scale/visibility offsets | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Parent/group transforms, anchors, opacity and solo | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer and group 3D switches | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| 2D solids/images/video/text/shapes/precomps in 3D | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Flat text/shapes remain flat until extrusion enabled | Native opt-in routing checked; disabled artwork retains its existing renderer | Native `d.extrusion` / `set_extrusion` | PMV checked | Compound edits checked | Surface ops and channel readback checked | Read geometry/surfaces/channels → `set_extrusion`, `set_surface_material_properties`, `set_surface_material_map`, ordinary animation |
| Text outlines, font loading/fallback and holes | Native broker/full-face outlines, four surfaces and counters checked; image-only glyphs report errors | Native `d.extrusion` / `set_extrusion` | PMV checked | Compound edits checked | Surface ops and channel readback checked | Read geometry/surfaces/channels → `set_extrusion`, `set_surface_material_properties`, `set_surface_material_map`, ordinary animation |
| Shape extrusion, compound paths and holes | Native compound fill rules and expanded strokes; stroked caps subtract fill overlap | Native `d.extrusion` / `set_extrusion` | PMV checked | Compound edits checked | Surface ops and channel readback checked | Read geometry/surfaces/channels → `set_extrusion`, `set_surface_material_properties`, `set_surface_material_map`, ordinary animation |
| Animated extrusion depth/bevel and quality | Native geometry channels evaluated; kernel/keyframe/expression paths registered | Native `d.extrusion` / `set_extrusion` | PMV checked | Compound edits checked | Surface ops and channel readback checked | Read geometry/surfaces/channels → `set_extrusion`, `set_surface_material_properties`, `set_surface_material_map`, ordinary animation |
| Angular/concave/convex bevels, inner hole bevel depth | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front/Bevel/Side/Back independent materials | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| PBR color/roughness/metalness/emission/opacity | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Color/normal/roughness/metalness/emissive/AO image maps | Native retained-image and indexed replacement checks | Image and bounded live source captures connected | Tested sparse records | Tested | Tested | Read maps/source material IDs → `update_object` material.maps or sourceMaterials; capture failures reported per layer |
| Source materials versus whole-model overrides | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | Native sparse values, source IDs and rendered emission checked | Connected | Tested PMV | Tested Undo/Redo | Tested | `update_object` patch `materialProperties` / `sourceMaterialProperties`; `mi.*` / `mi.index.*` channel edits |
| Live composition textures on named model materials | Native composition capture; per-receiver clocks and sparse source-material bindings checked | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live layer material sources in same composition | Native hidden layer/group captures checked; source canvas ignores root transform/opacity | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layer material source timing, masks, effects; transform/opacity excluded | Native masks/effects owner space, descendant visibility and source clocks checked | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material source missing/loading/relink/undo/save lifecycle | Per-receiver errors, source/file invalidation and PMV/Undo checked; relink UI verification pending | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Sun/parallel, point and spot lights | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light distance/decay, spot angle/penumbra and visibility | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light cast-shadow toggle, model cast/receive-shadow toggles | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Global shadows and ambient color/intensity/exposure | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light markers, select and move/target gizmos | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Perspective and orthographic cameras | Shared camera; native projection/clip checks | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Animated camera position/target/FOV/zoom/near/far | Shared engine present; row integration unverified | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Camera and free/editor view; orbit/pan/zoom/frame | Shared engine present; row integration unverified | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Move/rotate/scale gizmos, scene/object axes, modal transforms | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Projected bounds, ray picking, model/part selection | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Mixed 2D/3D depth sets and transparent layer ordering | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer effects/masks/blend modes and opacity | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Viewer, agent render_frames and still capture | Shared engine present; row integration unverified | P | K | K | P | Saved quality, tier, progress → `set_render_quality`, export `studioQuality`; automatic tier scheduling pending |
| Video/image-sequence export, nested composition and motion-blur sampling | Shared engine present; row integration unverified | P | K | K | P | Saved quality, tier, progress → `set_render_quality`, export `studioQuality`; automatic tier scheduling pending |
| Extension api.scene3d and revision-guarded edit_3d/get_3d_scene | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Validation, budgets, undo/redo, serialization and asset bundles | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

## Material-source parity with main reviews #187 / #188

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Groups as Base Color or Emissive | Native bounded subtree capture with hidden root; masks/effects and stable canvas checked | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Cycles and local links | Native recursive source/precomp/matte/effect graph validation; per-receiver failure | P (validated state; rendering pending) | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Stable group source canvas | Native duration/frame-grid/key-time union with bounded evaluation cache | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Safe group capture | Native nested-precomp 3D-descendant rejection checked | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera/UV-driven detail | Shared engine present; row integration unverified | G | U | U | G | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Capture budgets | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Texture sharing/replacement | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Static capture reuse | Source-only keys and receiver-specific clocks checked across immutable adapters | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Evaluated bounds and bounds null | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Comp navigation clocks | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Imported paint alpha | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Path visual bounds | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

## Requested AE 27.0 checklist

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Environment Light and HDRI | Shared world; native Live/Final HDRI checks | P (render connected; broader parity pending) | K | K | P | World/source/atmosphere channels → `set_world`, `add_fog`, `update_fog`; live layer-source capture pending |
| Parallel / spot / point lights | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Ambient / environment light types | Shared lights; native transport checked | P (render connected; broader parity pending) | K | K | P | World/source/atmosphere channels → `set_world`, `add_fog`, `update_fog`; live layer-source capture pending |
| Material Options: Casts Shadows | Shared shader; native policy/pixel checks | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Shadows | Shared shader; native policy/pixel checks | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Lights | Shared shader; native policy/pixel checks | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Appears in Reflections | Shared shader; native policy/pixel checks | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layers as materials, same-comp source | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Keyframeable Opacity/Roughness/Metallic | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Substance SBSAR | G (upstream gap) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front / Bevel / Side / Back | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera depth of field/focus | Shared lens; native focus/defocus checks | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Camera iris controls | Shared iris; native defocus check | P (render connected; broader parity pending) | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Multiple views | Native 1/2/4 pane state, six orthographic axes, named/custom cameras and independent picking checked | G | U | U | G | Read `Views` / `Pane` → `ViewCommand`, `Views::apply_to`, `Viewer::studio_camera` / `studio_state`; UI arrangement pending |
| glTF embedded animation | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| FBX with embedded animation | Shared `pm-import-fbx`; geometry/material render verified | P (skin/clip data retained; broader animation scenes pending) | Tested original + GLB round trip | Tested | `import_media` accepts FBX; model-source edits | Import worker → `import::prepare` / archive receipt → `Scene3dCommand::Import`; MCP import + `add_object` |
| 2D layers casting shadows onto 3D | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Render passes | Shared engine present; row integration unverified | G | U | U | G | Saved quality, tier, progress → `set_render_quality`, export `studioQuality`; automatic tier scheduling pending |

## Studio features to retain

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Live / Refine / Final path tracing | Shared engine; all native worker tiers checked | Saved quality plus transient viewer tier | Quality tested | Quality tested; tier transient | Quality exposed; direct native tier hook | `studio3d.drive` id=tier, `UiState.viewer.studioTier`, `Viewer::studio_preview`; automatic idle refinement pending |
| Fast / Quality preview | Shared engine present; row integration unverified | P | K | K | P | Saved quality, tier, progress → `set_render_quality`, export `studioQuality`; automatic tier scheduling pending |
| Finish presets and rendered swatches | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Softbox lights and light-by-pointing | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Auto studio, seamless set, shadow catcher | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shots and exploded views | Shared engine present; row integration unverified | P | K | K | P | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Drop to floor / place on surface | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Live screens | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| STEP/IGES/BREP CAD import | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Gaussian splats and point clouds | Native splat path; pixel/export checks | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Advanced glass/coat/sheen/film/dispersion/droplets/brushed/subsurface | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Native backend attachment | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

## Environment contract beyond the parity rows

Sky/sun/HDRI and atmosphere, fog volumes, terrain regions/thermal erosion,
seeded scatter/instances, procedural plants/grass/rocks and distance LOD exist
in the pinned engine. Native authoring, channels, actor operations, readback, persistence and inverse
patches now exist. World/HDRI/fog and terrain/nature have native renderer
translation, including scatter/hidden prototypes/LOD. Live source capture remains
a gap.
The kernel is the only document writer.

## Ordered plan

1. Publish this audit and pin. Update evidence as checks land.
2. Replace engine and host whole. Move native-only facade/extraction outside
   pm-studio and adapt the scene/buffer contract. Keep viewer, capture and
   windowless export on the same engine. Verify hardware/portable traversal,
   OIDN and the bounded Cycles regression.
3. Split 1080p product timings into tracing, guides/readback, OIDN guide/beauty
   filters and output readback/encode. Verify a Metal OIDN build/device with
   CPU fallback; document bundling if Homebrew lacks Metal support.
4. Extend native schema, actor commands, persistence/history, MCP readback and
   translation for world/fog/terrain/scatter/maps/iris/import/quality. Test
   Electron-authored scenes and exact save/reopen plus Undo.
5. Add noise convergence, adaptive edge/detail sampling and linear HDR pixel
   reconstruction. Grade enamel/glass and compare numeric reference checks.
6. Measure actual app presentation and input latency under the quiet-machine
   lock. USDZ and Cycles fog/subsurface/sheen/sky matching are excluded.

## Acceptance and measurements

| Gate | Required | Current native evidence |
| --- | --- | --- |
| Rust renderer and native regressions | Pass | 123 renderer/host checks; 18 adapter checks; 8 native scene/export checks pass |
| Bounded Cycles yardstick | Pass | All 11 scenes pass; included in the 123 renderer checks |
| Live presented rate | ≥60 fps | Unmeasured |
| Input → pixel p95 | ≤50 ms | Unmeasured |
| Refine | 32 spp + OIDN ≤2 s | Engine 4.8–7.0 s; full app unmeasured |
| Production | 1080p ≤3 s/frame, no visible noise | Engine diagnostic exceeds target; full app unmeasured |
| Save/reopen and Undo | Every authored operation | 25 kernel, 17 document, 2 source-contract and 9 MCP checks pass; broader Electron project compatibility pending |
| Electron project compatibility | Same evaluated scene | Pending |

Upstream diagnostic measurements in the pinned `docs/studio/native-host.md`
are not native-app acceptance. No performance or Cycles-equivalence claim is
made from them.

### Initial interrupted preflight (superseded by the completed checkpoint)

The unmodified pinned renderer compiled with `oidn,metalfx,yardstick` in the
shared debug target. Before the run was interrupted, **45 library checks**
passed in **3.29 s**, and **2 AE material/light checks** passed in **0.19 s**;
zero failures were reported in those completed targets. This is engine-only
evidence, not native-adapter, full-suite or performance acceptance.

The bounded Cycles regression completed these comparisons before interruption
during `env-landscape`. The overall regression has **no pass result**.

| Scene | Mean ΔE00 | SSIM |
| --- | ---: | ---: |
| hero | 1.227 | 0.9933 |
| edge | 2.436 | 0.9841 |
| macro | 1.317 | 0.9225 |
| back | 0.314 | 0.9901 |
| chrome | 2.223 | 0.8669 |
| materials | 3.659 | 0.9162 |
| caustics | 2.796 | 0.7992 |
| wet | 0.793 | 0.9511 |
| env-room | 5.851 | 0.3613 |
| env-street | 4.849 | 0.6156 |

The installed Homebrew OIDN **2.5.1** supplies its CPU device library and no
Metal device library. GPU OIDN therefore needs a separately verified Metal
build/bundle. No OIDN timing split or native frame timing was measured.

### Initial integration findings and resolution

The pinned host moved to `hosts/studio-electron`; importing the old path would
retain the wrong helper. Native-only extraction/service/splat/status/swatch
modules must move outside `engine/crates/pm-studio`. A generic GPU backend seam
in pm-render can keep that dependency direction clean: immutable document
source → worker backend → per-layer texture → normal compositor.

The pinned shader changed its instance policy words: row 4.w packs a layer
override and instance-color flag; row 5.w controls primary visibility. The old
native extractor used them for material/layer IDs. Reusing those bytes would
silently render the wrong material or hide geometry. Adapt triangle material
bindings, UV1/physical-map records, instancing metadata and the world trailer;
verify hardware and portable paths against the same scene.

The shared GPU device now requests optional Metal ray-query features/limits.
MetalFX now exposes an in-process GPU texture return. The old Electron host
transport remains available independently; the native app uses no helper hop.

## Reproduction and designer handoff

Use the shared native debug target and `/tmp/pm-build.lock` for every build,
test, package and timing run. Never clean it; retain at least 5 GB free before starting;
stop below 4 GB.
Artifacts stay outside Git. Baseline launch (Studio attaches to model layers):

```sh
cd apps/native
export CARGO_TARGET_DIR="$HOME/Developer/pm-native-engine/apps/native/target"
export RUSTY_V8_ARCHIVE="$HOME/Developer/pm-v8-s1/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$HOME/Developer/pm-v8-s1/src_binding.rs"
flock /tmp/pm-build.lock "$HOME/.cargo/bin/cargo" +1.96.1 run -p pm-native-app --bin powermove-native
```

`apps/native/scripts/bundle-dev.sh` describes the separate shader compiler
needed by effects. No release, tag or notarization. All control presentation
and visual acceptance belong to the designer.


## Completed engine/native checkpoint

- `6851cf5a6`: exact whole import at the pin; the host is at its upstream path.
- `e2e6ecfbb`: GPU-only in-process MetalFX output, retained per-layer
  presentation and stage timing. Per-layer targets share one OIDN invocation.
- `a4c396849`: generic worker backend seam and optional Metal ray-query device
  features/limits, retaining portable traversal.
- `25dd2663c`: native adapter outside `engine/`, device/geometry reuse, mapped
  material bindings, safe instance-capacity changes and existing splat support.
- `4464573c8`: opt-in half-resolution OIDN auxiliary filters. Beauty remains
  full resolution, normals are normalized after reconstruction, and default
  OIDN behavior retains full-resolution guides. The quality bridge now selects
  half guides for Refine and non-Final saved presets; Final retains full guides.

The app binary builds with the local pinned V8 archive/bindings and installed
native JS build dependencies. On this Mac, Bun 1.3.14 cannot read the v2 lock
format in remote-dom/v4; dependencies were installed without saving lockfiles.
No lockfile, profile, RUSTFLAGS or vendored UI source was changed for that setup.
The native regression tests cover reflected off-camera geometry, source model
materials, imported multi-object models, splats, deterministic shutter output,
headless service frames and windowless child exports. CAD-specific platform
tests were not enabled. The former engine Studio-hook regression now lives in
`apps/native/crates/studio3d-engine/tests/compositor.rs` with the native adapter.

### 1080p stage split

Serialized M5 engine diagnostics, 1920×1080, hardware RT, starlight hero/chrome
product scenes. These are single-run diagnostic measurements, not full-app or
quiet-display acceptance. Times include synchronous GPU completion. PNG encode
is measured in memory. Different runs have tracing/load variation; auxiliary
filter times isolate the intended optimization more directly than total time.

| Scene / preset | Trace ms | GPU guides ms | Guide readback/packing ms | OIDN albedo + normal ms | OIDN beauty ms | Output readback ms | PNG ms | Total ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Hero / Preview, full guides | 1977 | 60 | 19 | 768 | 386 | 2 | 132 | 3369 |
| Hero / Production, full guides | 1847 | 61 | 17 | 659 | 775 | 2 | 83 | 3456 |
| Chrome / Preview, full guides | 2192 | 131 | 50 | 654 | 382 | 2 | 249 | 3677 |
| Chrome / Production, full guides | 3072 | 129 | 15 | 648 | 676 | 2 | 234 | 4788 |
| Hero / Preview, half guides | 1984 | 61 | 19 | 194 | 457 | 2 | 131 | 2890 |
| Hero / Production, half guides | 2441 | 63 | 17 | 167 | 835 | 2 | 82 | 3651 |
| Chrome / Preview, half guides | 2183 | 123 | 16 | 170 | 396 | 13 | 252 | 3187 |
| Chrome / Production, half guides | 3057 | 131 | 16 | 170 | 701 | 2 | 249 | 4362 |

Refine at 32 spp with high OIDN/full guides measured **4797 ms hero**
(trace 998 ms, OIDN 3530 ms) and **7001 ms chrome** (trace 3461 ms,
OIDN 3133 ms). The 2-second Refine and 3-second Production targets are open.
Chrome's tracing deadline stopped at 28 spp; this diagnostic reproduces the
upstream time-cap behavior, not the requested noise-converged export policy.
No visible-noise acceptance, teapot edge grading or full-app fps/input-latency
claim is made. The half-guide regression covers odd dimensions, deterministic
reuse/resizing, full-size beauty and unchanged alpha; reference grading remains.

Reproduce after building `studio-timing` with the same renderer features:

```sh
flock /tmp/pm-build.lock env STUDIO_TIMINGS=1 \
  "$CARGO_TARGET_DIR/debug/studio-timing" --scenes hero,chrome \
  --presets preview,production --guide-scale 2
```

Homebrew OIDN is CPU-only here. A Metal-enabled OIDN build would need its Metal
backend enabled at configuration, the matching device/runtime libraries
bundled alongside OIDN/core and explicit Metal-device selection in the binding.
That source build was deliberately not performed. Native distribution bundling
of OIDN/TBB/licenses remains a separate gate; dev builds use installed libraries.

## Integrated document/data checkpoint

The data checkpoint adds the pinned World/fog/terrain/scatter/material-option schemas,
material/part sparse data, iris channels, ordinary actor/property paths, quality
presets, MCP operation metadata/readback and source validation. Its targeted
checks pass: **25 kernel**, **17 document**, **2 source-contract**, **9 MCP**; plus **21 import** checks.
They prove exact Undo/Redo, protected edits, atomic failure, saved quality and
PMV reopening. The old source oracle is retained; its 16-light case is explicitly
superseded by Studio's 4096-light budget. No scene rendering claim follows from
these document tests.

The current-composition serialization previously dropped `studioQuality`.
The persistence fix keeps the frozen v1 `COMP_FIELDS` inventory and adds a current
composition-field set used by canonical projection and native PMV envelopes.
The actor remains the sole author; planned structural staging is validated at
the end of each compound edit, before publication.

Still required for full scene parity: wire new scene nodes and
hidden prototypes into native extraction; live environment capture; terrain and
scatter/LOD translation; material source capture and all
imported maps; iris/orthographic transport; quality motion-blur sample integration and automatic
paused Refine scheduling; broader imported animation/map parity; render passes;
noise convergence/filter/edge checks and native presentation/input measurements.
The standalone engine supports more than the native document adapter currently
translates. Treat the capability matrix as partial until each row has evidence.


Source checkpoints (first staged on `feat/na-studio3d-wip`, then integrated): `f62ec5781` (composition quality persistence),
`1dcfc3270` (document/MCP authoring), `492226f10` (explicit failure while new
render translations are pending). The last checkpoint adds a separate passing
native regression that prevents environment placeholders from being drawn as
ordinary planes. All eight native scene/export regressions pass after the quality and FBX bridges.
The data and quality checkpoints are now integrated into `feat/na-studio3d`.


### Quality and export command bridge

`4e195ac4a` connects saved composition quality to actual Final rendering and
adds `ExportRequest.studio_quality` / MCP `export_video.studioQuality` for a
job-only preset or patch. The root composition receives the override; nested
compositions retain their own saved settings. No document write is involved.
The native worker applies the noise threshold, minimum/maximum samples,
bounces, caustics and denoise toggle. Stills/deliveries ignore the saved time
limit and stop on convergence or the sample ceiling. Legacy documents without
`studioQuality` retain their camera/environment sample count.

Refine and non-Final quality presets use half-resolution auxiliary filters and
the balanced OIDN network. The Final preset keeps full-resolution auxiliaries
and the high-quality network. Beauty resolution is unchanged. These native
policy changes passed the adapter/native regressions but have not received a
new 1080p timing or high-sample image grading run. The earlier diagnostic table
is not a measurement of this newer native policy.

The GPUI binary builds. Native tests prove saved-quality sample counts, an
export-only override, rejection of an invalid range, unchanged document bytes,
and delivery completing all eight requested samples despite a tiny saved time
hint. The seven native scene/export checks pass in 14.76 seconds; nine MCP
checks pass in 0.88 seconds. The eighteen adapter checks pass. These are targeted
regressions; the broad workspace suite was interrupted when storage ran low
and has no overall pass result.

Designer hooks: read `get_3d_scene.scene.studioQuality`, write through
`edit_3d {operation:"set_render_quality", preset|patch}`, and send a job-only
`studioQuality` on `export_video`. Native callers can use `studio::attach_with_quality`
or `ExportRequest.studio_quality`. `motionBlurSamples` is preserved but not yet
connected to shutter scheduling. The viewer uses Live by default; Refine is
available on the worker seam but automatic idle/interaction tier selection
remains open. No panel, control, colour or layout was added.

### Deferred Metal OIDN build recipe

This recipe is documented, not executed tonight. Keep Cargo profiles/features
unchanged. Use a separate OIDN staging directory only after storage is available.
OIDN 2.5.1 requires Git LFS plus recursive submodules, CMake, Python and the
CPU backend's ISPC/TBB; its Metal backend needs Xcode 15+ and CMake 3.21+.
[Upstream build instructions](https://github.com/RenderKit/oidn/blob/v2.5.1/README.md#compiling-from-source).

```sh
# Future build only: provide these three task-specific absolute directories.
# OIDN_SRC, OIDN_BUILD and OIDN_STAGE must be outside the shared Cargo target.
git clone --branch v2.5.1 --depth 1 --recurse-submodules \
  https://github.com/RenderKit/oidn.git "$OIDN_SRC"
git -C "$OIDN_SRC" lfs pull
flock /tmp/pm-build.lock cmake -S "$OIDN_SRC" -B "$OIDN_BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 \
  -DOIDN_DEVICE_METAL=ON -DOIDN_DEVICE_CPU=ON \
  -DOIDN_DEVICE_SYCL=OFF -DOIDN_DEVICE_CUDA=OFF -DOIDN_DEVICE_HIP=OFF \
  -DOIDN_APPS=OFF -DCMAKE_INSTALL_PREFIX="$OIDN_STAGE"
flock /tmp/pm-build.lock cmake --build "$OIDN_BUILD" --parallel 2
flock /tmp/pm-build.lock cmake --install "$OIDN_BUILD"
```

Before integration, request `OIDN_DEVICE_TYPE_METAL` explicitly, report its
actual type, and use OIDN device buffers (or verified Metal shared allocations)
for beauty/albedo/normal/output; ordinary host pointers must satisfy that
device's allocation rules. Preserve an explicit CPU-device fallback. Bundle
the matching OIDN/core/Metal/CPU device libraries, TBB and notices with verified
runtime paths, then repeat the stage split and image comparisons using that
bundle. The current binding uses host images and default-device selection;
merely copying a Metal dylib does not complete this work.
[OIDN device and buffer API](https://github.com/RenderKit/oidn/blob/v2.5.1/doc/api.md).


### Native FBX checkpoint

- `c2cc177cb` moves the pinned FBX converter and its fixtures whole into
  `engine/crates/pm-import-fbx`. The Electron host and native import use that
  single implementation, with the same ufbx 0.11.5 pin. No renderer or shader
  change is included in that engine-only commit.
- `d865b301a` admits native FBX files, retains the original alongside a converted
  GLB, and recognizes FBX in native MCP media import. Existing raw FBX assets can
  also render from retained paths with unrelated cache-file extensions; saved
  format/name metadata chooses the decoder.

The worker uses an 8 MiB stack and the converter's existing 256 MiB input,
geometry, allocation and time budgets. It opens no sibling files. Embedded
materials, skin/joint records and baked animation reach the existing glTF
loader. External-texture admission and broader animation/material-map parity
still need real-project coverage.

Checks: **21 native import tests**, **25 kernel scene tests** (including exact
FBX original/GLB save-reopen and Undo/Redo), and **8 native GPU/export tests**
pass. The new GPU test verifies the FBX source's Coral material reaches both
the compositor and windowless export. Eight GPU/export checks take **15.76 s**;
the final GPUI app build takes **30.05 s**. The shared renderer/host/import suite
still reports **123 passing checks**, with all **11 Cycles scenes** passing.
FBX's three checks moved from the host into the shared crate; they are not
counted twice. Two earlier reruns refused the yardstick's own 5.5 GB storage
reserve before rendering; the completed rerun passed after this lane's own
outputs/intermediates were removed. No guard was weakened.

Native import command hook: run `pm_scene3d::import::prepare` on a worker,
admit through `import::archive::Archive`, then send the receipt in
`Scene3dCommand::Import`. The actor owns the asset/layer, media journal and
history changes. MCP can import a `.fbx` as media and reference its asset ID in
`edit_3d`'s `add_object` source. File chooser presentation belongs to the designer.

### Storage stop and remaining acceptance

The final app build completed with approximately **4.7 GB** free. No further
build/test/render was started below the owner's 5 GB floor. Only artifacts
proven to originate in this lane's recorded locked runs were removed; unmatched
shared executables and the other checkout were retained. Raw EXR/array/scene
intermediates are removed from this lane's evidence after scoring, while logs,
summary metrics and comparison PNGs remain outside Git.

The port remains incomplete. World sky/atmosphere/fog translation is now implemented (see the next checkpoint).
Terrain/nature translation is now implemented (see the later checkpoint).
Sparse material/part renderer translation remains unfinished. Live environment capture
and full cross-composition source-cycle analysis remain pending. Remaining gates include
material source capture, extended imported maps, broader camera project parity,
quality shutter samples and automatic idle Refine, complete Electron scene
round trips, pixel filtering/extra edge sampling/enamel/no-visible-noise grading,
and measured native presentation/input latency. The measured Production/Refine
limits above remain open. No on-screen fps, input p95 or packaging acceptance
is claimed from headless tests or an app build.


### World and fog rendering checkpoint

After storage recovered to about 12 GB, implementation continued. World sky,
sun and global atmosphere now reach the native renderer. Transformed box fog
uses world-space AABB half extents, and sphere fog uses the maximum authored
world scale, matching the pinned Electron environment adapter. World and Fog
create no proxy geometry. World has a background-only compositor output; Fog
has a transparent surface output while affecting 3D transport. Camera-hidden
World stays transparent while its environment can still light geometry.

Ambient lights now use the renderer's additive ambient channel; parallel lights
use directional transport. Environment-light World settings are carried into
lighting with their intensity multiplier. HDRI decoding is still pending. The
native extractor retains the exact sky/light trailer so camera/object motion
does not rebake an unchanged physical sky. Changes invalidate the cached
trailer, while surface geometry stays resident.

`set_world` now writes the source command's preset channel defaults, with an
explicit caller patch taking precedence; saved channels are never changed by
parsing. `set_render_quality` now combines a supplied preset and patch, matching
the pinned command. Ordinary property writes and Undo remain the only writer.

Checks: 18 adapter tests pass; 26 kernel scene tests pass; 11 native scene/export
tests pass, plus the extended Live World regression. The World tests verify an
empty geometry scene's full-viewport sky in Live and Final, transparent hidden World/Fog
outputs, rotated/scaled volume extents, visible world edits, deterministic
restoration and geometry reuse. The remaining unsupported-node guard now covers
terrain. These pixel checks do not establish the 60 fps or input-latency gates.


### Retained HDRI resources

`219a984df` fixes the shared environment loader: retained `.bin` paths decode
from their contents, integer images become linear radiance, and HDR/RGBE/EXR
floating values retain their energy above one. The loader caps lighting maps
at 2048×1024 while its admission probe returns the original image dimensions.
Owned floating buffers avoid an unnecessary full HDR image clone. RGB vectors
have a defined packed layout for exact host image transfers/cache keys.

Native World and environment-light sources now resolve durable image asset IDs
through the frozen media catalog. The worker keeps up to four decoded maps
within 96 MiB, applies intensity/rotation/camera visibility, and invalidates its
packed world trailer by exact pixel/settings bytes. This avoids rebaking the
same HDRI distribution on camera motion. Native MCP `import_media` recognizes
HDR/RGBE (`.hdr`) and EXR, probes them with this loader on its worker, and admits
the original bytes through the normal media command. File chooser presentation
remains with the designer. Layer/composition environment captures remain open.

The `.bin` rejection was reproduced before fixing it. Loader checks prove the
PNG midpoint becomes 0.2158605 linear, RGBE values [4,2,1] stay above one, and a
2050-pixel source retains that admission width while its lighting map is capped
to 2048. The full renderer/host suite passes **124 checks**, including all 11
Cycles comparisons. The native adapter passes **19 checks**; exact HDRI pixel
and visibility changes invalidate the cache. All **12 native scene/export**
checks pass; the HDRI check fills the viewport in Live and Final from a retained
path with no image extension. The GPUI app builds. These checks do not establish
image-source capture parity, no-visible-noise acceptance or presented fps.

### Shared terrain/nature geometry prerequisite

The native extractor now preserves per-face materials and packed linear vertex
colors from the pinned engine. Local material palettes bind one immutable mesh
to different scene materials while reusing its geometry cache. Repeated scatter
bindings validate the palette once per extraction. Invalid color/face lengths,
non-finite colors and out-of-range palette entries fail before upload.

The previous loss was reproduced with a colored, multi-material mesh. All 22
adapter checks pass, including palette rebinding, invalid input and GPU layer
regressions. The native app builds. Terrain/nature node translation follows in
a separate increment; this change alone does not enable those nodes.


### Terrain and nature worker translation

`479dc67bc` is an engine-only compatibility fix: signed saved heights retain
negative elevations, retained image paths decode by content, and image heights
use the pinned scene contract's encoded-RGB luminance and endpoint sampling.
These bugs were reproduced before fixing them. The shared renderer/host/import
suite passes **126 checks**, including all **11 Cycles scenes**.

Native terrain now uses the shared heightfield generator, material regions,
natural ground, blended roughness/metalness, detail, thermal erosion and bounded
camera-adaptive grid selection. Height image assets resolve through the frozen
media catalog; eight decoded grids are retained per source. Immutable mesh keys
include decoded heights so a replaced asset cannot reuse an old terrain mesh.
Region triangles bind through local palettes; natural ground carries linear
vertex colors and its procedural surface record. Transforms use the ordinary
native world evaluation. Nature sources use the pinned seeded conifer,
broadleaf, birch, bush, grass and rock generators and their two material slots.
Scatter-specific LOD and hidden prototypes remain pending.

**15 native scene/export checks** pass (14 together plus the new retained-height
check), including signed grids/regions, eroded colored ground, stable geometry
reuse, all six species, retained image replacement and terrain coverage in Live
and Final. The GPUI app builds. No panel/layout changes were made.

| Capability | Engine | Document / kernel | Persistence | Undo | MCP / UI hook |
| --- | --- | --- | --- | --- | --- |
| Terrain grid, signed heights and image source | Shared generator; native translated | `add_terrain`, `update_terrain`, channels | Existing PMV checks | Existing inverse-op checks | `edit_3d`; terrain/height-source data and same commands |
| Regions, blend and natural ground | Shared palettes; native translated | Region records/channels | Stored | Stored edits reversible | `edit_3d`; region material/range records → `update_terrain` |
| Detail, erosion and adaptive density | Shared generator; native translated | Terrain channels | Stored | Stored edits reversible | `edit_3d`; terrain p → ordinary property writes or `update_terrain` |
| Six seeded nature prototypes | Shared geometry; standalone native translated | Object `source.nature`/`seed` | Stored | Stored edits reversible | `edit_3d`; source data → `add_object`, `update_object` |
| Scatter, shared instances and LOD | Shared engine; native translated | Model/surface references/channels | Stored | Stored edits reversible | `edit_3d`; `add_scatter`, `update_scatter` |

Layer/composition height captures remain explicit unsupported errors. Whole
Electron-project parity and the native environment performance gates remain
unverified; the small tests establish transport and state behavior only.


### Seeded surface scatter and hidden prototypes

The hidden-source loss was reproduced: assembly omitted the two hidden resources
created by `add_scatter`. Native assembly now retains active scatters' model and
surface resources within their time ranges, marks them non-visible in evaluated
state and leaves the saved switches unchanged. Geometry is prepared before
visible instances are selected. The shared generator samples the evaluated
world-space triangles, including terrain and imported/posed resource parts.
Density, seed, scale/rotation, height and slope limits use the pinned generator.
The prototype's root translation is removed while its rotation, scale and part
offsets remain. Instances belong to the scatter timeline layer.

Up to four placement sets / 64 MiB are cached by exact settings and immutable
surface identity/world matrices. Camera motion changes conifer/broadleaf/birch/
bush/grass/rock LOD without rerolling placements. Three meshes are shared per
nature source; repeated instances reuse both palettes and BLAS geometry. Zero
accepted placements produce transparent Live/Final compositor outputs.

Checks: **18 document**, **3 scene service**, **2 source-contract**, **21 import**,
**17 navigation/light**, **8 schema unit** and **22 renderer-adapter** tests pass.
The new native checks verify 100 placements from hidden resources, the translated
surface's elevation, deterministic world matrices, one uploaded prototype,
near/far nature LOD with identical placement matrices and transparent empty
scatter in Live/Final. The native scene/export set has 18 checks; 17 passed
in the full run and the remaining placement check passed after correcting its
exact-float test comparison to a 1e-6 tolerance. The GPUI app builds.

Designer hooks remain `edit_3d` → `add_scatter` / `update_scatter`, model/surface
layer IDs, scatter channels and nature source/seed. No controls were added.
Arbitrary parented layer subtrees as prototypes/surfaces and environment-aware
picking/bounds still need parity checks; imported resource parts are included. Performance with large
forests and native presented fps remain unmeasured.


### Material options and primary visibility

Native render bindings now carry Casts Shadows (off/on/only), Accepts Shadows,
Accepts Lights and Appears in Reflections to the pinned shader flags. Shadow-only
instances remain in transport and their primary-camera visibility bit is set.
Unlit materials emit their own base color and retain base-map alpha. Imported
Basic materials use that same unlit path; imported and authored double-sided
flags reach their material records. The previously ignored material-option flags
were reproduced before the fix.

Bindings are specific to the timeline owner. Scatter inherits its prototype's
options unless its own options override them; instances share the resulting
material records/palettes. Ordinary unchanged materials retain their original
bindings. This adds no geometry copies or UI controls. Commands/state remain
`update_object`, `update_scatter`, materialOptions channels and ordinary kernel
property writes, with existing persistence/Undo behavior.

All **22 native scene/export** and **22 adapter** checks pass. Pixel checks cover
unlit red with no lights, an off-camera emitter excluded from reflections and
transparent primary output for shadow-only geometry. A 100-tree scatter shares
its overridden palette and material records. Sparse intrinsic/source-material
properties and extended maps remain separate unfinished adapters. Performance
and distribution acceptance are unchanged.


### Camera transport and camera-motion residency

Orthographic projection now uses the pinned application's 3-unit half-height
and saved zoom. Iris blades/rotation/roundness/aspect, focus/aperture, near/far
clipping and physical scene scale reach the shared camera setters. State and
commands remain the existing camera channels and `set_camera`; no view controls
or layouts were added.

Camera motion previously triggered a Metal scene/TLAS rebuild even when every
resource was unchanged. Exact resource bytes are now compared independently of
the camera/view values, reusing the previous complete key without another full
resource copy. Camera edits reset view accumulation while leaving geometry and
ray-tracing resources resident. Material, map, world and instance edits still
invalidate by exact bytes. Scene scale is now part of the camera key; its missing
Refine reset was reproduced before fixing it.

All **24 adapter** and **24 native scene/export** checks pass. A camera edit
changes fixed-seed pixels with exactly **1 geometry upload / 1 scene update**
across both frames (previously 2 scene updates). Scale changes restart Refine
at 2 spp instead of incorrectly continuing to 6 spp. Native tests verify
orthographic coverage remains within 4 pixels when the camera dollies from 5
to 10 units, iris changes affect defocus, and clip planes remove out-of-range
geometry. The GPUI app builds. Presented fps and input p95 remain unmeasured.


### Native worker timing and retained OIDN filters

`RenderedScene.timings` / `FrameTimings` now expose worker preparation, trace,
GPU guide passes, guide readback, CPU OIDN and total synchronous render time.
The manual `pm-studio3d-engine` timing test uses the real in-process
`StudioService` at 1920×1080 with Metal RT, full-resolution beauty and half-size
auxiliary prefiltering. Production uses the saved 32–256 sample/noise policy
without a 3-second deadline. Refine uses 32 spp and balanced OIDN. Each value
below averages two serialized runs under the shared build/render lock; debug
profile/fingerprints were unchanged. These are headless worker measurements,
not native presentation/input-latency acceptance or visible-noise grading.

| Scene / policy | Before: render ms | Retained filters: render ms | Trace ms | OIDN total ms | GPU guides ms | Guide readback ms | Output readback ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Hero Refine 32 | 2759.2 | 2829.4 | 1104.8 | 1397.1 | 62.0 | 166.4 | 66.9 |
| Hero Production 256 | 3728.8 | 3683.7 | 2069.5 | 1303.4 | 58.0 | 155.4 | 66.8 |
| Chrome Refine 32 | 5258.7 | 5186.2 | 3506.4 | 1293.8 | 122.3 | 153.3 | 68.8 |
| Chrome Production 256 | 23303.1 | 23218.1 | 21519.5 | 1320.4 | 119.4 | 153.0 | 68.7 |

One OIDN device/filter set now follows each worker thread, retaining its buffers
across document replacement, output resizing and guide-quality changes without
sending OIDN handles between threads. Tests preserve deterministic pixels after
those changes. The gain is modest: native OIDN/auxiliary work remains around
1.3 s, while Chrome's tracing dominates. Two runs do not establish a stable
speedup; Hero Refine's total increased with trace-time variance. OIDN's total
includes guide reduction/reconstruction and buffer work, so it exceeds the
three filter-only measurements. After retention, Hero Production's albedo /
normal / beauty filters average 82.8 / 82.8 / 748.0 ms; Chrome Production's are
84.1 / 82.4 / 761.2 ms. Readback is separate; encode timings remain in the
previous engine diagnostic section.

The older capped Chrome Production diagnostic stopped at 28 spp. Its 4.36 s
half-guide result must not be compared as an equal-convergence export to these
256-spp results. Production <=3 s and Refine <=2 s remain open; no noise target
was weakened to improve these timings. Homebrew OIDN remains CPU-only; the
Metal build/bundle recipe is documented above and no source build was performed.

All 24 adapter and 24 native scene/export checks pass with retained filters;
the manual timing check also passes. The GPUI app builds. Reproduce only with
>=5 GB free, under the lock, using `STUDIO_TIMINGS=1` and
`cargo test -p pm-studio3d-engine --test timing -- --ignored --nocapture --test-threads=1`
from the native workspace with the documented shared target/V8 environment.

### Imported vertex alpha, secondary UVs and layer coverage

Imported resources now retain linear RGBA vertex colors and TEXCOORD_1 beside
posed geometry. The native extractor packs alpha with RGB, supplies secondary
UVs/tangents, and honors imported flat normals. Derived attributes are shared by
exact bytes across snapshot replacement; different attributes on the same mesh
cannot alias a GPU binding. Source meshes and the pinned engine are unchanged.

The new native alpha test reproduced an opaque-black-fill bug: the worker kept
the product renderer's opaque void for ordinary timeline layers. Those layers
now leave uncovered/stochastically transparent samples for the compositor.
Product shadow-catcher sets retain their current opaque-void behavior. This also
preserves fractional edge coverage; it does not implement the requested linear
pixel reconstruction filter or establish no-visible-noise acceptance.

All **25 native scene/export**, **25 adapter** and **21 import** checks pass, and
the GPUI app builds. The new retained-GLB check verifies secondary UV records,
packed 0.5 vertex alpha, visible translucent red pixels and unchanged geometry
build counts across source replacement. Distinct/malformed attribute bindings
are checked separately. Existing full-viewport World/HDRI, transparent empty
scatter, reflections, splats and windowless export checks remain green.
The next material step is binding the shared shader's extended physical maps.

### Extended imported physical maps

The native adapter now packs the shared 24-scalar / 18-map contract: Base Color,
Emissive, separate Roughness/Metalness, Normal, AO, coat amount/roughness/normal,
transmission, thickness, specular intensity/color, sheen color/roughness,
iridescence amount/thickness and anisotropy. Each slot keeps its own UV0/UV1,
transform and repeat/clamp/mirror modes. Two glTF samplers referencing one image
share one GPU image layer. Sixteen decoded images (64 MiB at the fixed 1024²
map size) are retained by immutable resource/image identity across snapshots.

Specular/sheen colors, attenuation color/distance, thickness, anisotropy rotation,
film IOR/ranges, normal/coat scales, AO strength, dispersion and alpha cutoff now
reach the shared shader. Unlit and Accepts Lights=false materials use the base
map for emission while retaining its alpha. Secondary and physical-map indices
are checked against the actual image array before GPU work; UV sets above 1
fail explicitly, matching the pinned engine limit.

Checks: **2 physical packing unit**, **26 native scene/export** and **26 adapter**
tests pass; the GPUI app builds. The retained GLB regression carries all 18 slots,
two samplers/one image, UV1 with a nonidentity transform, physical optical values
and alpha cutoff through to a rendered frame. Unit checks cover every slot,
missing-map sentinels, unlit map selection and unsupported UV sets. These checks
do not establish every glTF extension/asset combination or minification-filter
parity. Authored image maps and live layer/group material capture remain separate
adapters, followed by sparse intrinsic/material/part overrides.

### Authored image-backed material maps

The six authored map slots now resolve durable image asset IDs on the worker:
Color, Emissive, Roughness, Metalness, Normal and AO. Primitive, terrain/nature
and imported meshes using the authored material bind the same raw 1024² RGBA8
map array. Separate roughness/metalness/AO channels retain their shader semantics;
color/emission decode sRGB exactly once and color alpha reaches transport.

Imported and authored pixels now share one content-keyed decoded-image cache,
bounded to 16 images / 64 MiB. Source replacement with changed image bytes cannot
reuse old pixels. The native source also bounds its asset-ID lookup cache to
16 entries. Import byte limits and the 256-map GPU array cap remain enforced.
Live layer/group/composition sources report their explicit capture gap.

All **27 native scene/export** and **22 import/image** checks pass; the GPUI app
builds. The new test binds all six slots from an extensionless retained PNG to
one image layer and renders the expected self-lit red material. It checks
missing-image and live-capture errors. Content reuse, changed-image invalidation,
retained alpha and malformed image rejection have a separate image regression.
Authoring remains ordinary `update_object` material.maps data and channel ops;
existing PMV/Undo/MCP routing is unchanged. No UI was added.

### Production defaults and legacy sample commands

Two mismatches were reproduced and fixed. The native worker now resolves absent
saved quality to the pinned source's Production defaults (32–256 samples,
0.03 noise target, 8 bounces, caustics and denoise). Reading/rendering an older
composition leaves its serialized document unchanged. This supersedes the
initial port's legacy fixed-sample fallback; an old camera sample count alone
is not the composition's selected quality policy.

Legacy `set_environment.patch.samples` commands now write the corresponding
custom quality sample ceiling/floor in the same kernel transaction as the
camera environment data, matching the source command. One Undo/Redo restores
both records exactly; invalid counts leave content and revision unchanged.
Explicit saved presets and per-export overrides keep precedence. Still/export
rendering retains the noise target/sample ceiling with no preview time cap.

The native regression fixtures now request their tiny 4-sample policies
explicitly. All **28 native scene/export**, **27 kernel scene**, **18 document**,
**3 scene service**, **8 schema unit** and **2 source-contract** checks pass;
the GPUI app builds. The Production performance/no-visible-noise acceptance
gates remain open; applying the correct default is not a speedup claim.

### App worker tier command/state hook

`studio3d.drive` now accepts `id: "tier"`, `arg: "live" | "refine" | "final"`.
It writes transient `UiState.viewer.studioTier`, and the Studio projection reads
`tier`. `Viewer.publish` takes that state with the immutable document publication;
a same-document-revision tier edit changes the viewer sequence and rejects old
in-flight work. `Viewer::studio_preview(Preview)` is the direct native command
hook. No controls, colors or layouts were added.

Live selects the retained MetalFX/WGSL live path; Refine requests 32 samples with
OIDN; Final uses the composition's saved noise/sample policy (Production if
absent). Captures select Final. Every view supplies its own tier to the shared
worker. Tier changes retire cached pixels, preserve the GPU backend/geometry,
and do not change project data or Undo history. Composition identity is now also
part of source attachment identity so two same-revision views cannot reuse each
other's quality state. Ordinary export overrides retain their separate scope.

The development-only `PM_NATIVE_STUDIO_TIER=live|refine|final` environment hook
sets the initial tier when no explicit viewer state exists. Shipped builds
ignore it. The older Fast/Quality presentation flag remains a compatibility
field; it does not select the Studio tier. Orthographic cameras no longer reject
that Quality command. The designer can bind `tier`, saved `studioQuality`, and
worker status independently. Automatic idle refinement is still pending.

Checks: all **29 native scene/export** and **21 Studio command/session** tests
pass, including all three tiers on one GPU backend, unchanged upload counts,
orthographic command acceptance, invalid-value rejection and unchanged project
and history. All **90 general engine-worker unit** tests also pass (one relaunch
helper intentionally ignored), using a canonical temporary root because the
storage admission tests correctly reject macOS symlink ancestors. The GPUI app
builds with the unchanged profile/features.


### Sparse intrinsic and indexed image material overrides

The engine-only `2709e2943` keeps original glTF material identity alongside each
final material variant. Native retained parts expose that identity; a generated
GPU material slot never substitutes for the file's material index. This change
can be cherry-picked independently of the native adapter.

Native rendering now overlays evaluated whole-model `materialProperties` and
indexed `sourceMaterialProperties`: opacity percent, roughness, metallic,
emission color/intensity and ambient response percent. Indexed values win;
absent values preserve the original material. Emission color and intensity stay
separate, including a colored original with zero intensity. Basic/unlit file
materials accept opacity only, matching the pinned runtime. Terrain regions keep
their authored regional finishes. Scatter retains its prototype material bindings.

Whole-model image maps and indexed `sourceMaterials` maps now replace the six
standard slots on imported materials. Indexed replacements take precedence before
loading an overridden source. Replacement images retain the original slot's UV
set, transform and wrapping. Extra physical maps stay intact. Imported color and
emission factors reset for replacement images before sparse intrinsic edits;
clearing an override on the next immutable snapshot restores file state. Live
layer/group/composition maps still report the capture gap.

Designer hooks: `edit_3d` / `Scene3dCommand::Apply` operation `update_object`,
patch `materialProperties`, `sourceMaterialProperties`, `sourceMaterials`;
ordinary property commands use `mi.opacity`, `mi.roughness`, `mi.metallic`,
`mi.emissionColor`, `mi.emissionIntensity`, `mi.ambientResponse` and
`mi.<source index>.<field>`. `get_3d_scene` publishes their evaluated channels.
These edits use the single kernel writer and existing document/history paths.

Checks: **52 shared geometry unit**, **4 native material unit**, **31 native
scene/export**, **22 import/image**, **28 kernel scene** and **10 MCP scene**
checks pass. Regressions cover a GPU-slot/file-ID mismatch, visible green
emission, exact file restoration, UV1 replacements, all 18 retained physical
slots, zero-intensity emission edits, unlit opacity, per-source channel edits,
PMV reopen, post-reopen Undo/Redo and agent readback. The GPUI app builds
(42.68 s, unchanged profile/features). Part offsets remain explicitly unsupported; complete
Electron-project render equivalence still needs the outstanding capture,
extrusion, mixed-depth and broader import cases.

### Input timestamp and multi-view cache corrections

An input origin now stays pending through stale, dropped or failed work until a
current GPU frame lands. A later publication no longer clears an unpresented
origin, and an older completion cannot consume a newer input. A deterministic
worker barrier reproduces the former missing timestamp and checks that a later
capture does not reuse the consumed origin.

The frame cache includes the Studio tier and exact composition ID. Changing a
source or tier on the shared worker no longer clears unrelated viewers' cached
frames. Two viewers using different tiers retain independent cached pictures;
same-content/revision requests for two compositions cannot alias their pixels.

Checks: **93 worker unit** tests pass (one intentionally ignored relaunch helper),
plus **17 service integration** tests; the app builds (**54.46 s**). The
composition regression alternates two immutable compositions with different
shape colors at an identical content/revision/time.

The previous isolated app diagnostic presented **1,976 Live frames over
73.3326 s = 26.9321 fps**, using a rotating torus and three softboxes in a
1920×1080/60-fps composition at Fit 38%, Full resolution. Worker render wall
median/p95 were **32.668 / 49.843 ms**, which are not input-to-pixel latency.
Other native apps remained running, so this was not quiet-machine acceptance.
The input-origin correction requires a fresh on-screen latency run.

### Linear reconstruction and edge/noise acceptance

Engine-only `f4ec85e59` adds a default separable **Blackman–Harris 1.5 px**
footprint, using an inverse CDF and digitally shifted Sobol camera samples.
Radiance, emission, backdrop/coverage and OIDN guides share that footprint in
linear HDR before denoising and tone mapping. Live retains its temporal jitter.
`Renderer::set_pixel_filter` exposes Box for references and configurable
Blackman–Harris widths; changing it invalidates accumulated samples. Existing
WebGPU hosts with the old buffer allocation retain their former box estimator.

A separate GPU pass classifies first-batch silhouette, normal, depth and color
edges without cross-pixel races in the tracer. Those pixels receive at least
four times their authored sample floor, bounded by the chosen ceiling. Adaptive
stopping checks RGB channel error as well as luminance, so chromatic noise
cannot hide behind equal luminance. Saved still/export policies continue to
converge on their noise target or sample ceiling, with no three-second cutoff.
The extra moments cost 16 bytes/pixel plus a 4 KiB filter table; admission accounts
for them. No panel or controls were designed. UI can keep using saved
`studioQuality`, its noise/floor/ceiling fields and per-export overrides.

Checks: **132 engine tests pass**, including all **11 Cycles yardstick scenes**
with matching Blackman–Harris reconstruction, plus portable and hardware-RT
HDR/alpha coverage tests. The native app builds in **31.80 s**. Numeric image
checks render original procedural teapot, thick-wall glass cup and chrome-sphere
fixtures at **192×128** against fresh **4096-spp** references. Both Production
(32–256 spp, half-resolution guides) and Final (512–1024 spp) pass these normalized
sRGB-code thresholds: foreground luminance RMS < 0.020, smooth-region residual
noise RMS < 0.006, p95 absolute 8×8 block bias < 0.012. Final measured values:

| Fixture | Luminance RMS | Smooth noise RMS | Block bias p95 |
|---|---:|---:|---:|
| Teapot | 0.00166 | 0.00161 | 0.00071 |
| Glass cup | 0.00554 | 0.00321 | 0.00169 |
| Chrome | 0.00237 | 0.00219 | 0.00103 |

These bounded regression fixtures do not establish 1080p quality acceptance for
every imported asset. Their matte enamel uses authored perceptual roughness 0.7
and zero mirror clearcoat, checked through GPU material preparation. Sparse
native roughness overrides already reach that same engine material path. The
owner's exact teapot asset/finish still needs comparison; no unnamed material
preset was silently restyled.

### Native Live timing and presentation invalidation

Engine-only `5ffc0a9cc` splits completed MetalFX guide and denoiser wall time
from packing/submission. `09c4b50b8` also reports the external Metal command
buffer's GPU execution time. These timings do not add GPU barriers.
Development builds emit `STUDIO_FRAME_TIMING` when `STUDIO_TIMINGS=1`; the
immutable request size/tier, preparation, pipeline stages and retained-resource
counters are available to the app/debug lane. Texture-only pack timing measures
submission, not a separate GPU completion.

Direct transport commands now have `Viewer::command_at(command, event_time)`;
the app uses the originating key event for frame steps. The worker rejects
invalid timestamps before enqueueing and retains an accepted origin through
presentation. Time-field edits include the editing interval and are excluded
from input acceptance measurements.

Unchanged guide/gizmo publications previously advanced the viewer sequence,
retiring an in-flight picture even with no selected layer. `safe_areas` now
preserves the sequence when geometry and provider/time ownership are unchanged.
A barrier regression reproduced that invalidation and now confirms the same
frame lands without a stale rejection. Changed geometry still uses the ordinary
invalidation path. Checks: **95 worker unit tests** (one intentionally ignored)
and **17 service tests** pass; the app builds in **27.78 s**.

The diagnostic fixture is a rotating torus with three softboxes, a 1920×1080
60-fps composition, Fit 38%, Auto/Full resolution and Live. The measured worker
output is **1464×824**, internally traced at **732×412** before MetalFX. A repeat
before overlay deduplication presented **1247 frames / 46.4703 s ≈ 26.81 fps**;
after it, **4005 frames over 147.7401 s ≈ 27.10 fps**. FPS uses intervals between
the first and last presented frames, excluding startup. This is a diagnostic,
not a 60-fps acceptance pass. A separate instrumented run's median stage times
were preparation **1.11 ms**, trace **6.46 ms**, guides **3.24 ms**, MetalFX wall
**13.78 ms** (GPU **11.78 ms**), pack submission **0.29 ms**. Live uses no CPU OIDN.

Thirty direct frame steps after five warmups now carry timestamps. After
overlay deduplication they measured input-to-presented-pixel median/p95
**50.67 / 115.73 ms**. The state-to-worker median became **0.075 ms**, with one
sequence increment per step; 29/30 presented frames used a newly rendered
picture. Median GPU-completion-to-display was **21.25 ms**. These measurements
include GPU load from other open apps: total GPU utilization was **95%** during
playback and **46%** just after closing only the test app. Other apps were not
closed. The quiet-machine **60 fps / 50 ms p95** gates remain open.

A headless 1080p native-service diagnostic (30 frames per case, after four
warmups, retained wallpaper input) measured these median wall times. It is not
presented fps or an image-quality gate:

| Scene | Motion | Preparation | Trace | Guides | MetalFX | Total |
|---|---|---:|---:|---:|---:|---:|
| Hero | Static | 0.55 ms | 5.23 ms | 1.03 ms | 5.31 ms | 12.38 ms |
| Hero | Instances rotate | 12.50 ms | 5.68 ms | 1.02 ms | 5.08 ms | 24.50 ms |
| Chrome/glass | Static | 10.32 ms | 17.49 ms | 1.21 ms | 4.81 ms | 34.05 ms |
| Chrome/glass | Instances rotate | 21.28 ms | 14.52 ms | 1.17 ms | 4.69 ms | 42.01 ms |

GPU time confirms that the Live deficit includes real GPU execution and other
on-screen GPU work, not CPU OIDN. Animated preparation also exposes avoidable
texture reuploads; the source image is unchanged when only instances move.

### Retained image uploads during animation

Studio now compares image dimensions, layer count and exact pixel bytes
independently of scene/instance state. A transform, camera change or output
resize retains the GPU image allocation. A same-revision pixel replacement,
removal or restoration still uploads immediately and resets the accumulation.
`StudioService::texture_uploads` and the development timing hook expose the
counter. The image retention regression failed before the fix (two uploads
for one instance move), then passed with visible motion, blue replacement,
removal, exact red restoration and output resize. All **27 native Studio tests**
pass (two manual diagnostics intentionally ignored).

At 1080p with the same retained input and 30 measured frames per case, rotating
Hero preparation fell from **12.50 to 0.75 ms**, total **24.50 to 12.69 ms**;
rotating Chrome/glass preparation fell from **21.28 to 9.85 ms**, total
**42.01 to 30.10 ms**. Every case uploaded images once. These are worker
measurements; the on-screen acceptance gates above remain open.

A new post-filter 1080p Refine/Production split (mean of two frames, CPU OIDN,
half-resolution guides, full-resolution beauty, convergent policy with no time
cutoff) measured:

| Scene / policy | Trace | OIDN incl. guide filtering | Guide dispatch + readback | Worker total | Output readback | RGBA8 PNG helper encode + write |
|---|---:|---:|---:|---:|---:|---:|
| Hero Refine 32 spp | 1026.5 ms | 1434.9 ms | 238.5 ms | 2712.0 ms | 66.6 ms | 519.9 ms |
| Hero Production 32–256 spp | 2601.5 ms | 1318.5 ms | 222.5 ms | 4151.2 ms | 66.4 ms | 515.8 ms |
| Chrome/glass Refine 32 spp | 3643.8 ms | 1321.0 ms | 292.6 ms | 5275.3 ms | 67.2 ms | 802.5 ms |
| Chrome/glass Production 32–256 spp | 22517.6 ms | 1343.8 ms | 323.4 ms | 24210.4 ms | 73.1 ms | 773.7 ms |

Guide dispatch is submitted before readback, so the latter includes completion
of the guide GPU work; these columns must not be counted twice inside worker
total. PNG here uses Studio's default-compression diagnostic helper; native
sequence delivery has its own encoder. OIDN is material to Hero's cost, but
path tracing dominates Chrome/glass. Neither the **2 s Refine** nor **3 s
Production** gate passes for both scenes. The CPU-only OIDN limitation and
Metal-enabled bundle recipe remain documented above; no source build was made.

### UI dependency 1: surface picking

`Viewer::pick_surface(stamp, time, [u, v], SurfaceQueryKind::Click | Hover)`
returns an asynchronous receiver. `SurfaceReply` carries the request stamp,
media time and optional `SurfaceHit { layer, instance, point, normal }`.
Coordinates are normalized from the top-left of the full composition; the
viewer converts fit/pan/zoom/crop into those coordinates. Points and normals
are in the native scene's world coordinates. A miss is `hit: None`.

GPU picking runs on the existing viewer worker, shared renderer, scene and
hardware-RT/portable traversal. It uses the pinhole camera even with depth of
field, and respects primary visibility, sidedness, alpha and material normal
maps. Scene preparation is shared with rendering; a query traces no image
samples and performs no image readback. Querying the same Refine scene preserves
its accumulated samples. Switching query scenes invalidates any incompatible
progressive state before rendering resumes.

The UI should await the receiver outside its event callback and check the
returned stamp/time before applying a point-driven light/gizmo command. Stale
requests and requests that become stale while executing return errors. Up to
eight queries are retained per viewer. Consecutive pending hover requests
coalesce; clicks remain queued. Queries alternate with rendering to prevent
hover traffic starving frames. Retirement, worker restart and shutdown settle
or close pending receivers. No click handling, navigation, gizmo geometry or
visual changes were added by this lane.

Checks: **28 native Studio tests**, **96 worker unit tests** (one intentionally
ignored), **17 service** and **32 native scene/export** tests pass. The app
builds in **55.68 s**. The public viewer regression queries a native sphere and
checks its layer, point and normal, stale-frame rejection and invalid input.
A worker barrier verifies hover coalescing, click preservation, the queue bound
and shutdown. Direct service checks cover misses, both layer identities,
retained GPU uploads and Refine accumulation across hover.

### UI dependency 2: per-layer failure isolation

Native scene contributions are transactional per timeline owner. An unsupported
mesh, unavailable model, image/capture binding, imported part failure, invalid
material/transform, World/fog resource or scatter dependency is omitted while
healthy contributions remain in the shared scene. Partial materials, instances
and image-atlas slots roll back together, so a failed import cannot leave another
layer referencing its discarded resources. Geometry is validated at cache
admission. Failed lights also drop their partial contribution.

`SceneSource::layer_failures(request)` and `RenderedScene.failures` expose
`LayerFailure { composition, layer, message }`. The compositor reports the
original error for the failed layer and preserves the rest of its GPU picture.
Backend failures also enter ordinary frame metadata, including non-surface
contributors such as lights, and deduplicate by composition/layer. The existing
`ViewerStatus.layer_failures` and the UI lane's panel facts are the UI hook;
no error rows or presentation were designed here. Errors clear on the next
healthy immutable source/frame. Document-resolution and device failures retain
their existing failure path.

The reproducing test originally reported both the broken model and its healthy
sibling as failed. It now reports only the broken model and checks visible green
geometry. Further GPU regressions cover partial atlas rollback, missing images,
unready material sources, missing HDRI, dependent scatter failures and error
recovery after replacing the document. **35 native scene/export** and **28
native Studio** tests pass (two manual diagnostics ignored); the app builds in
**53.62 s**. Existing Refine, imports, physical maps, World/fog, terrain/scatter,
source replacement, picking and export regressions remain green.

### UI dependency 3: native text and shape extrusion

The native document retains the pinned Electron `layer.d.extrusion` version-one
contract. Absent settings read as disabled without changing old projects.
`set_extrusion { target, patch }` accepts `enabled`, `depth`, `bevelStyle`
(`none`, `angular`, `concave`, `convex`), `bevelDepth`, `holeBevelDepth` and
geometry `quality` (`draft`, `standard`, `high`). Enabling also sets the ordinary
layer's `threeD` switch in the same kernel transaction. Disabling preserves that
switch, as the source contract does. Geometry quality is separate from render
quality.

Front, Bevel, Side and Back have sparse independent properties and maps.
`set_surface_material_properties` (alias `set_material_properties`) accepts
`target`, `surface` or `materialIndex` 0–3, and `patch`: `color`, `opacity`,
`roughness`, `metallic`, `emissionColor`, `emissionIntensity`, `ambientResponse`.
Null clears an override and restores inheritance. `set_surface_material_map`
(alias `set_material_map`) takes the same surface address plus `slot` and `map`;
null removes it. Durable images render now; live layer/group/composition maps
remain dependent on item 4. Color and Emissive are the live-source slots.

All channel writes cross ordinary kernel property/keyframe/expression handling:
`geo.KEY` and `ms.SURFACE.KEY`. Whole-content staging, the 3D switch and channel
writes share one Undo entry. Save/reopen retains both authoring data and history.
The MCP catalog exposes these operations and `get_3d_scene` returns the
`extrusions` data, extrusion layer records and channel paths. UI reads
`pm_scene3d::extrusion::{parse,channels,SURFACES}` plus those ordinary channels,
and calls `Scene3dCommand::Apply`/`edit_3d`; no panel or control was designed.

`pm-geom3d::extrusion` ports compound caps/counters, four material groups,
continuous cap/perimeter/depth UVs, angular/concave/convex bevel profiles and
hole bevel scaling. It rejects invalid coordinates and enforces the 500,000
vertex budget during generation. The native adapter caches geometry separately
from world transforms and material values, and bounds retained outline geometry.
Ordinary model layers skip outline preparation on the immutable tree before any
JSON copy. The follow-up passes both extrusion integration tests (1.19 s) and
an app rebuild (111 s; shared dependencies were rebuilt without flag changes).

Text uses the existing native render-thread font broker and the same full-face
outline painter as flat text, including its font selection, variations, style
ranges and character layout. Anchored glyph coordinates pass through the
source's six-unit composition bridge and full native `world3D` transform.
Bitmap/image-only glyphs report a layer error because they have no vector
outline. Shapes reuse native editable-path/primitive fills and strokes; compound
fill rules survive, and stroked areas are subtracted from fill caps before
extrusion. The native compositor receives the path-traced layer, rather than a
second flat copy. Empty artwork receives a transparent compositor answer.

Validation so far: **29 kernel tests**, **11 MCP tests**, the engine's compound
hole/four-surface regression and the native glyph/compositor regression pass.
The last regression checks all four material groups, geometry/attribute reuse,
real extrusion depth and authored red emission replacing flat blue paint.
The full **37 native scene/export tests pass in 40.76 s**, including shape
fill/stroke extrusion, empty text, per-layer isolation, picking and existing
material/environment/import regressions. The final native app build passes in
**31.29 s**. Evidence stays outside Git in the lane's extrusion test/build logs.

### UI dependency 4 — native layer/group/composition material captures

The typed Studio compositor source now prepares texture dependencies before
rendering or surface picking. Its retained capture renderer shares the viewer's
wgpu device and ordinary native compositor; the app needs no Electron host.
`material.maps.color` / `.emissive`, indexed `sourceMaterials`, and extrusion
surface maps accept `{layerId}` (local artwork/group) or `{compositionId}`.
`material.screen` emits its composition with white gain and the saved emission
intensity. Existing `update_object` and `set_surface_material_map` commands own
these saved records and their ordinary kernel Undo/Redo. No UI was added.

Layer capture excludes the source root's transform and opacity, draws a hidden
root, and preserves descendant visibility, timing, transforms, opacity, masks,
effects and nested compositions. Groups use one source-local canvas unioned over
the group's duration, frame grid and property keys. Empty groups stay transparent.
Missing sources, 3D descendants (also in nested precomps), receiver feedback,
matte/effect dependency cycles and excessive nesting become per-receiver errors.
They do not blank healthy 3D layers. Sources are read without editing the document.

Composition maps sample the receiver's source clock, floor to the source frame
grid and clamp to its duration. Local layer/group maps use composition time.
The cache fingerprints supplying graphs, dependency clocks, asset/file versions,
fonts, effect definitions and color settings. Static pixels survive playback and
receiver-only edits; entrances/exits and source edits invalidate them. Matching
source images share texture-array storage, including color/emissive reuse. The
existing eighteen-slot imported-map replacement retains UV/sampling metadata.

Capture admission rejects more than 4,194,304 pixels before allocation. Retained
pixels are LRU bounded to 128 MiB; the generic capture API accepts a zero cache
budget and participates in renderer trimming. Native maps currently request
1024²: camera/UV-driven resolution buckets and wiring the app-wide preview cache
allowance remain follow-up work. Animated capture currently reads straight RGBA
back to the CPU before the Studio array upload; static capture avoids this after
the first frame. GPU-direct material capture needs performance measurement.
Sparse named/source-material overrides work; the separate authored `slots` array
still needs its native adapter. These limits do not establish full material
performance/close-up parity with the Electron reference.

UI reads the saved source address and `ViewerStatus.layer_failures`; commands
remain `update_object` (maps, screen or indexed `sourceMaterials`) and
`set_surface_material_map` for Front/Bevel/Side/Back. A failed source can be
replaced or cleared through those same commands. Screen choices can use this
native capture path once the UI lane consumes this increment.

Checks: **42 native Metal scene/export tests** pass in **10.75 s**, including
five material-capture regressions. They cover hidden groups, stable duration
canvas, mask/effect coordinates, child visibility, source-only reuse across
adapters, receiver clocks, screen emission, nested 3D/cycle rejection, zero
retention and oversized admission. **30 native kernel scene tests** pass in
**0.27 s**, including material-source Undo/Redo and PMV reopening. The GPUI app
build passes in **24.26 s**. These are functional checks, not new fps or export
budget measurements.

### UI dependency 5 — independent native views

The engine/data lane adopts the UI lane's `FreeView`/`Viewer::studio_view`
API and extends it without changing any controls. `studio::Views` is validated,
serializable transient data for **1, 2 or 4** panes. Each `Pane` has a `camera`
and `tier`; `Views::active()` identifies the selected pane. Hidden pane settings
survive layout reductions. Commands are `ViewCommand::SetLayout {count}`,
`Activate {pane}`, `SetCamera {pane,camera}` and `SetTier {pane,tier}`; JSON uses
camelCase operation names. Indices are 0–3. These commands edit viewer state,
never the shot or its Undo history.

`ViewCamera` supports `Active`, `Layer {id}`, `Custom {view: FreeView}`, and
`Orthographic {axis, center, span}`. Axis is Front/Left/Top/Back/Right/Bottom;
span is vertical scene units. The native source supplies a valid basis even at
the poles, disables depth of field for orthographic views, and evaluates a named
camera's animated channels and hierarchy from the immutable kernel snapshot.
A missing named camera is an explicit query/render error. Viewer overrides apply
to that viewer's composition; supplying compositions retain their own cameras.

The UI owns surface creation and arrangement. Create ordinary `EngineService`
viewers for the visible panes, then call `Views::apply_to(&viewers)`. It validates
all handles first and updates distinct viewers on one service atomically.
Unchanged data preserves in-flight frames. Release hidden viewer handles when
reducing the visible count; the `Views` data retains their settings. Per-handle
commands/queries are `Viewer::studio_camera`, `studio_pane` and `studio_state`.
Existing `studio_preview` and `studio_view(Option<FreeView>)` remain compatible.
`studio_view_at`/`studio_camera_at` accept the input event's host timestamp for
input-to-pixel measurement. Cache identities include camera, tier and font
generation, and surface picking takes exactly the pane's camera and tier.

Captures/exports retain the active composition camera. No panels, pane placement,
colours or control behaviour were designed here. The designer binds the view
model/commands to its UI and session preferences; no document migration is needed.

Checks: **45 native Metal scene/export tests** pass in **11.52 s**, including
four simultaneous worker camera/tier/pick states, every orthographic pole,
named-camera selection, invalid poses and supplying-composition isolation.
The view-command/serialization unit check and camera-sensitive frame-cache
check both pass. The GPUI app build passes in **22.42 s**. No on-screen fps
or input percentile is claimed by these functional tests.

### UI dependency 6 — studio backdrop and shadow catcher

The black backdrop had two native glue gaps: `Scene.backdrop` was always zero,
and every surface output discarded uncovered pixels. Native now carries the
saved environment background in display sRGB, or the pinned Studio source's
existing `#F6F5F8` backdrop when its catcher is enabled. Lighting remains scene
linear. This ports an existing renderer value; it adds no new visual choice.

Exactly one healthy primary surface owns the studio void, chosen from the
composition's layer order. A visible World keeps ownership of the environment.
The Studio service permits that surface's existing output to include the void
and rejects multiple owners. An explicit background can be opaque without
turning on the catcher. With both disabled, uncovered pixels remain transparent.
The catcher stays at the scene's lowest bound and no floor proxy is introduced.
This also fixes the round-two **shadow catcher black void** report.

OIDN was biasing even the nominally flat sky part of its shade image. The engine
now preserves exact shade outside the floor, and native Refine/Final skip the
shade network entirely when there is no catcher. Actual floor shadow/caustic
shading still uses the denoiser. No quality threshold or render deadline changed.

Checks: **46 native Metal scene/export tests** pass in **11.12 s**, including
12 background/catcher/tier combinations (Live, Refine and Final), transparency,
explicit black and saved/default colors. **29 native Studio checks** pass;
the new ownership check proves that only its designated surface includes the
void and duplicate owners fail. **51 shared engine unit tests** pass. A fresh
Blender/Cycles reference run passes **all 11 existing regression tolerances**;
the first reference-reuse attempt correctly refused a different floating-point
export and fresh references were rendered. The app build passes in **118 s**.
These checks do not establish independent Cycles environment matching or the
native performance gates.

The post-fix no-visible-noise regression also passes both checks in **20.26 s**:
at 192×128 against fresh 4096-spp references, Final luminance RMS is **0.00166**
(teapot), **0.00554** (glass), **0.00237** (chrome), with smooth residual RMS
**0.00161 / 0.00321 / 0.00219**. This small-image acceptance is separate from
the pending 1080p native export budget and the owner's chrome caustic complaint.
A final app rebuild after the defensive background-color parsing change passes
in **13.09 s**.

### UI dependency 7 — import placement

Every admitted OBJ/glTF/FBX/CAD-converted model now lands on the world ground
plane, including imports into an existing rig and imports with studio setup off.
Its normalized bounds are measured once on the admission/recovery worker and
stored as versioned `native3dBounds` asset metadata. Placement reads that small
record on the kernel actor; it does not decode files or pose meshes there.

Existing ordinary models keep their positions. The new model gets a clear
interval beside their bounds along the current camera's horizontal right axis.
Current hierarchy transforms are evaluated. Studio imports reuse and frame the
existing camera/lens around the models, converting the pose through its parent
space; an empty project retains the existing initial studio rig. Locked cameras
and generated edits' protected camera channels are preserved. Imports without
studio setup leave the rig/camera unchanged. Asset addition, placement and any
camera setup remain **one kernel transaction and one Undo**.

Old native/Electron assets without the new record use their normalized unit-fit
envelope for spacing and still bind their retained archives without rewriting
saved data. The stored bounds describe the initial pose; swept animation,
extruded artwork, terrain-aware placement and nature/scatter collision avoidance
remain follow-ups. Terrain/World/fog/scatter are excluded from the product row;
`place_on` remains the explicit operation for a different receiving surface.
`auto_studio` can take an explicit model `target` for initial import framing;
its existing no-target behaviour still frames all models, regardless of selection.

The designer's import command and native MCP import path already use
`Scene3dCommand::Import`, so no UI changes are needed. The same retained asset,
layer channels, saved composition and failure status remain the read-side hooks.

Checks: **31 kernel scene tests** pass in **0.26 s**, including repeated imports,
non-overlapping world bounds, both models touching ground, reuse of the existing
rig, camera framing, one-step Undo/Redo, PMV reopen/history, locked camera,
no-studio imports and pre-bounds archive recovery. **8 native scene unit tests**
and **46 native Metal scene/export tests** pass (the latter **11.14 s**).
The app build passes in **22.38 s**.

### Round-two engine finding — Quality fog blackening

Reproduced a dense white volume lit only by the existing legacy ambient
environment: with a foreground surface, mean centre RGB was **0.00011** in
Refine and **0.00047** in Final, versus **0.74362** in Live. The full transport
sampled direct HDRI/sky light at scattering events but omitted the legacy
hemisphere. It also suppresses environment escape after a scattering event
because direct sampling owns that contribution, so those paths became black.
The engine now phase-samples that hemisphere when no environment map exists.
Mapped environments keep their existing importance sampling.

Native composition now gives uncovered volume radiance one healthy output
owner even when the backdrop and sky are transparent. This also makes fog-only
compositions render without inventing a proxy plane. Camera visibility of the
sky does not disable independent Fog layers; disabling the Fog layer still
removes it. These are render fixes with no authored state or UI changes.

The same 64×64 reproduction now measures Refine **0.58551**, Final **0.62373**
with a foreground surface, and **0.58229 / 0.62345** without one. This is a
regression for missing illumination, not a claim of Cycles fog matching.
**47 native scene/export checks** pass in **11.27 s**; the focused volume test
also passes with Metal RT disabled (**0.97 s**). **29 native Studio checks**
and **51 engine unit checks** pass, and all **11 Cycles yardstick tolerances**
pass using the existing validated references. The native app build passes in
**21.04 s**.

### Round-two engine finding — Fast terrain exposure

The designer's Fast/Quality choices select native Live/Refine. A rolling,
procedurally colored terrain lit by the legacy ambient environment reproduced
the shift: centre luminance was **0.30512 / 0.33660** at 64×64 and
**0.31893 / 0.33687** at 256×256 (Live/Refine). The existing portable GPU
temporal/edge-aware filter kept the same terrain within roughly 1–2% of Refine.
MetalFX's denoised scaling caused this particular dark bias; changing its
automatic exposure did not solve it. Native color/guide jitter was already
aligned by the Rust shader adapter and is unchanged.

Live now selects the existing linear temporal filter when a scene contains
procedurally colored terrain (the prepared ground-material marker). Other
product scenes retain MetalFX denoised scaling. Refine/Final, scene lighting,
authored terrain colors and the saved quality policy are unchanged. This is a
scene-wide denoiser fallback: mixed terrain/product scenes currently use the
portable path together, rather than mixing incompatible per-layer histories.
Its upscale follows the compositor's existing sampling path. Future MetalFX
re-evaluation can replace this policy after the same exposure checks pass.

The in-process MetalFX output APIs also now respect their disabled/unavailable
capability flag. This makes the diagnostic fallback effective and prevents a
host from constructing an unsupported scaler. Normal native rendering chooses
the fallback before calling MetalFX; direct engine callers receive an error.

At 256×256 the fixed rolling-ground result is **0.33988 / 0.33687 / 0.33761**
(Live/Refine/Final); at 64×64 it is **0.34008 / 0.33660 / 0.33788**.
The regression checks plain and natural terrain, ambient and sky lighting, at
both sizes, requiring Live to stay within **5%** of Refine and Final. All
**48 native scene/export checks** pass in **17.52 s**; the same terrain check
passes with Metal RT disabled (**4.13 s**) and MetalFX disabled (**4.45 s**).
**29 native Studio checks** and **52 engine unit checks** pass, including the
direct MetalFX capability guard. The native app rebuild passes in **179 s**.
No presented-fps or 1080p quality claim follows from this small-image regression.

### Unverified quiet-window performance handoff

The measurement driver is `apps/native/scripts/studio3d-quiet-perf.py`. It is
prepared for the next quiet window and has not been exercised on screen yet.
Use a freshly built dev bundle that includes the shader compiler and the UI
navigation lane, and an animated **1920×1080, 60-fps** product composition.
Build/package beforehand under the shared lock, with the existing native
profile, features and target. Close other workloads before measuring. The
script checks storage and USER_ACTIVE, copies the corpus into its own temporary
folder, uses a disposable profile, warms for eight seconds, records 60 seconds
of Live playback, then records 30 seconds of actual native pointer/key input.
The operator or UI automation drives viewer navigation during that last phase;
the script does not substitute command timestamps for NSEvent timestamps.

Exact command (replace the bundle/corpus paths with the retained quiet-window
fixtures; the output directory must be new):

```sh
nice -n 10 flock /tmp/pm-build.lock python3 \
  apps/native/scripts/studio3d-quiet-perf.py \
  --app /tmp/pm-shots/na-studio3d/quiet-bundle/Powermove.app/Contents/MacOS/powermove-native \
  --project /tmp/pm-shots/na-studio3d/quiet-corpus/product-1080p-60.pmv \
  --out /tmp/pm-shots/na-studio3d/quiet-baseline \
  --seconds 60 --input-seconds 30
```

Set the bundle's usual FFmpeg/OIDN library environment before invocation if
needed. Evidence includes the executable SHA-256, full native presentation trace,
Studio stage timings, process logs and `summary.json`. The summary uses real
`presentedTime`, counts every presentation gap of at least 100 ms, and matches
each distinct native input timestamp to its first successful presentation.
Fewer than 100 distinct presented inputs invalidates input acceptance. Repeated
static frames intentionally do not count as presented Live frames; use the
animated fixture. Cold opening/compilation is separate from this warmed gate.
The script and its phase handling must be verified before treating its first
output as acceptance evidence.

Fix order for the recorded roughly 27-fps result:

1. Reproduce with this trace on the combined native/UI head. Attribute CPU
   evaluation/capture preparation, trace, guides, MetalFX wall/GPU, compositor,
   drawable wait and WindowServer delay. Record actual output dimensions and
   render scale. Require >=60 presented fps, no >=100-ms gap and input p95 <=50 ms.
2. Prepare material-source graphs once per frame/request/tier, rather than once
   per compositor output; preserve file/font/clock invalidation and fresh picks.
   Check the existing full-scene trace and overlay invalidation caches next.
3. Wire the existing `Renderer::queue_live_trace` path into the native service
   so trace and guides do not have an unnecessary CPU completion boundary.
   Keep the synchronization before borrowing Metal textures, and retain the
   portable path. Verify pixels and both surface lifetimes before timing again.
4. Address the measured MetalFX stage (earlier about 11.8 ms GPU at the measured
   viewer dimensions): repeated guide rays/specular distance, duplicate full
   images/layer copies, queue barriers and input/output resolution policy. Use
   measured budgets; coordinate any visible resolution policy with the designer.
5. Only then tune native presentation scheduling/drawable acquisition. Do not
   re-present identical paused content or feed observed WindowServer lag back
   into requested presentation times. Audit slow input against event, actor,
   publication, worker, GPU and actual display timestamps.
6. Re-run headless 1080p Draft/Production/Final splits with unchanged convergence
   targets and separately measured readback/encode. Draft budget is 1 s/frame,
   Production 3 s/frame; Final is uncapped. Homebrew OIDN remains CPU-only; use
   the existing half-guide path, with no Metal OIDN source build in tight storage.
7. In the same quiet window, alternate **two fixed projects** in one app process
   for at least ten A/B cycles after warmup. Sample RSS/physical footprint and
   Metal allocated bytes at the same settled points. Require a flat trend;
   exclude the growing-catalog test and do not sum overlapping unified-memory
   resident and GPU totals. Preserve per-cycle logs for the memory lane.

After each fix, repeat the identical corpus/viewport/scale/refresh run; do not
claim a gain from a busier baseline or a different output resolution. No new
presented-fps or input-p95 acceptance result was collected in this round.

### Quiet-window increment: measured export estimates and chrome grain

`Viewer::estimate_studio_export(stamp, time, Size, frames)` is the explicit
asynchronous calibration command. `ViewerStatus.export_estimate` exposes Idle,
Queued, Measuring (completed/total passes), Ready and Failed. The ready result
contains the composition/revision/time, requested size/frame count, bounded
calibration size, extrapolation flag, all four preset rows, per-layer warnings
and explicit exclusions. Each row includes trace, fixed photon setup, CPU OIDN,
guide/readback and preparation/post components, per-frame and whole-export
minimum/maximum durations, and the raw calibration measurements. These ranges
come from each preset's sample floor/ceiling, not a statistical confidence band.

Calibration uses the composition's shot camera at at most 320×180, with 8/16-spp
cold/warm passes for each preset. A fresh photon bootstrap is measured per
preset while the renderer/pipelines remain retained. Pixel/sample costs are
extrapolated; fixed photon setup and scene preparation are separate. Balanced
half-guide filtering matches Draft/Preview/Production; Final measures full-guide
high-quality OIDN. Encode/file I/O, other 2D effects, material-source capture,
extra motion-blur shutter samples and animation variation are explicitly
excluded. This is an approximate Studio render estimate, not an export deadline
or a guaranteed completion time. Gaussian splats currently return unavailable.

Requests coalesce per viewer and run only while the worker is idle, without
playing content, pending viewer work or in-flight presentations. Calibration
executes one pass per worker turn, giving new input priority between passes.
Scene changes, superseding requests, restart, shutdown and retirement invalidate
results. Status never exposes a result for an old viewer stamp. The query makes
no document, camera, quality or Undo edits. The UI can bind the command and
serialize the typed state; no export-sheet layout is added here.

Checks: the measured headless command publishes four presets for a 1080p target
without document edits (**2.80 s**). Three estimate unit/queue checks pass in
**1.22 s**, including coalescing and stale-frame cancellation. All **49 native
scene tests** pass in **18.23 s**. The native app rebuild passes in **168 s**
after one unchanged retry for pruned shared-target dependency metadata.

The noise regression now uses disjoint photon samples per emitter for its
high-sample reference, by rotating an otherwise physically identical light
list and rebuilding the actual map. Changing only the camera seed had reused
the same baked caustic grain in both images. The corpus now also includes the
actual chrome/glass/lacquer/catcher product set. Against a 4096-spp reference,
that set reproduced Final luminance RMS **0.02339**, smooth residual RMS
**0.00677**; more camera samples alone could not clean the fixed map.

The engine now uses an area-preserving quadratic B-spline receiver kernel, a
400-photon fine/coarse confidence transition, and 16,777,216 photons for a fixed
map (same grid storage). The focused chrome set passes unchanged acceptance
limits: Production luminance/smooth RMS **0.01595 / 0.00448**; Final
**0.01579 / 0.00545**. The full original teapot/glass/chrome suite and new set
pass in **32.78 s**, with no loosened thresholds. This remains a 192×128 noise
regression, separate from 1080p performance/quality acceptance.

The translated/tilted index-matched photon-energy check passes (**0.49 s**) and
all **11 existing Cycles yardstick tolerances** pass against retained references.
No Cycles fog/sky/subsurface/sheen matching claim is added. The larger fixed-map
bootstrap cost is included in subsequent timing; Final remains uncapped.

## Designer section (Opus UI lane, `feat/na-studio3d-ui`)

Presentation follows `docs/native/design/surfaces/3d-studio.md`. Screenshots stay
outside Git in `/tmp/pm-shots/na-studio3d-ui/` with `captions.md`.

### Built

| Area | What the person sees | Data / command |
| --- | --- | --- |
| Layer ▸ Add 3D Layer (menu bar, viewer context menu) | Cube … Icosphere · Softbox, Spotlight, Bulb, Sun, Camera · Sky, Fog Volume, Terrain · Pine/Oak/Birch Trees, Bushes, Grass, Rocks. One Undo `Add <thing>`; the new layer is selected | `Scene3dCommand::Apply` with `add_object`, `add_light`, `add_camera`, `set_world`, `add_fog`, `add_terrain`, `add_scatter` and a host layer ID (`pm_panel_studio3d::properties::add_layer`) |
| Sensible defaults | Terrain is natural, eroded ground; nature scatters onto the selected surface or terrain and adds a terrain when there is none; a new landscape with no camera gets an overview camera | the same Apply commands in one transaction |
| Properties header | `Cube · 3D Shape`, `Key · Softbox`, `World · Sky`, `Pine Trees · Scatter`; no 2D alignment strip, canvas size, masks or effects on 3D layers | `studio` flag on the inspector target |
| 3D transform | Position, Rotation and Scale % with X/Y/Z | `position.*`, `rotation.*`, `scale.*` channels |
| World | Lighting (Physical Sky / HDRI / None), Preset, Show in Camera, Sun Height/Direction/Strength/Size, Haze, Clouds, Ground, Brightness, Rotation, closed Atmosphere group | `set_world` (`mode`, `preset`, `p.visible`); `world.*` channels |
| Fog, Terrain, Scatter | Shape; Natural Ground, Ridged Peaks, Height, Feature Size, Erosion, Size and Ground groups; Scatters / On, Count, Density, Seed, Variation, Placement | `update_fog`, `update_terrain`; `fog.*`, `terrain.*`, `scatter.*` channels |
| Material | Finish shelf (11 swatches, no selected state, Undo `Finish: <name>`), Screen, PBR rows, Texture Maps from imported images, Material Options (Casts Shadows On/Off/Only, Accepts Shadows, Accepts Lights, Appears in Reflections) | `apply_material`, `update_object` maps, `set_material_options` |
| Composition | 3D Scene: Sky and Render Quality (Draft, Preview, Production, Final) with a note of samples, bounces and caustics | `set_world`, `set_render_quality` |
| Export sheet | `3D Quality` row (`Composition · <preset>` or a per-export preset); the summary ends `3D <preset>, about <time>` | option `studioQuality` → `ExportRequest.studio_quality`; estimate from the M5 worker timings above |
| Timeline | Kind badges per §1: model `cube` on blue, light `lightbulb` and camera `camera` on gray | the layer's `definition` |
| Viewer 3D group (round 2) | Camera / Free View · Move, Rotate, Scale, Orbit, Pan, Zoom, Light · Frame · Fast / Quality, shown only with 3D layers | UI state `studioLook`, `studioTool`, `studioTier`; `studio.frame` command; F frames in a studio comp |
| Selection and gizmos | Click selects what's drawn (engine pick), empty click deselects; projected bounds and Move/Rotate/Scale handles; ⌃ snap, ⇧ fine, Esc restores; Undo `Move/Rotate/Scale 3D Layer(s)` | `Viewer::pick_surface` (Click) on the displayed frame; drags are gesture `Step`s of ordinary transform channels |
| Navigation | ⌥-drag/middle-drag orbit, ⇧⌥-drag pan, ⌥-scroll zoom; the first gesture switches to Free View; no Undo | transient `FreeView{eye,target,zoom}` → `Viewer::studio_view`, never written to the document |
| Light tool | Click a model: adds a softbox aimed there (`Add Softbox`) or re-aims the selected light (`Aim Light`) | `add_light` / `update_light` with position and target from the pick |
| Arrow nudges | 0.1 scene units a press, 1 with ⇧, up is up | `nudgeSelection` writes `position.x/y` in scene units for 3D layers |
| Defaults (review notes) | First shape in an unlit scene gets sky, key softbox and floor; Pine Trees etc. frame as a postcard; Finish swatches are shaded spheres; Scatter rows are Object / Surface pickers | `starter()`, `overview_camera`, `landscape_sky`, `finish_swatch`, `scatter.model|surface` → `update_scatter` |
| Extrude (round 3) | Properties ▸ Extrude on text and shape layers: one click for depth, a rounded bevel and Front / Bevel / Side / Back; per-surface Color, Roughness, Metalness and Color Map | `set_extrusion`, `set_surface_material_properties`, `set_surface_material_map`; `geo.*` and `ms.SURFACE.*` channels (`pm_panel_studio3d::extrude`) |
| Layers as materials | Color and Emissive maps list 2D layers, groups and compositions; Screen is enabled again | `update_object` maps `{layerId}` / `{compositionId}`; menu values `layer:<id>` and `comp:<id>` |
| 1/2/4 views | Views pop-up, View ▸ 3D and the collapsed `3D` pull-down. Pane 1 is the editor viewer; the others are embedded viewers with a camera and preview pull-down each | UI `studioViews`; shell `viewer_split` and `viewer_tiles`; `Viewer::studio_pane` with `ViewCamera::Orthographic` framed on the scene's bounds (`app/src/studio_panes.rs`) |
| Light panel | Add with the kind to add, the light's name and kind, and Brightness / Size / Distance / Spread | `light::projection` on the viewer model; `studio.light` → `light.adjust.*`; UI `studioLightKind`, `studioLightAdd` |

### Plumbing added for the UI

- `engine/crates/pm-eval/src/address.rs`: Studio's `environmentProperties`
  (World, fog, terrain and regions, scatter, material options) are addressable
  property keys, as in the pinned source's `layer3DProperties`. Without it,
  Properties could not read, write or keyframe those channels.
- `pm_panel_studio3d::properties`: `add_layer`, `scene_summary`,
  `export_quality`, `estimate_seconds_per_frame`, `about`, `environment`,
  `finishes`. `view::commands` handles `scene.quality`, `scene.sky`, `world.*`,
  `fog.shape`, `terrain.*` and `options.*` with specific Undo names.
- Shell: the `new3d` submenu. Submenu parents in auto-enabling menus keep the
  submenu's own action; this also re-enables View ▸ Tools, which was disabled.
- Viewer chrome: `viewer.studio.tier` (`live` | `refine`) writes `studioTier`;
  the bar shows Fast | Quality only with 3D layers (§2 item 5).
- `engine-service`: `studio::FreeView` and `Viewer::studio_view(Option<FreeView>)`
  render the transient view; it is part of the studio content and cache keys,
  and `surface.rs` applies it to picks so clicks match what's shown.
- `pm_panel_studio3d::viewport`: CPU scene/eye model (projection, rays, box
  pick, framing) for gizmo geometry and the bounds-pick fallback;
  `app/src/gizmo3d.rs` draws the handles as viewer overlay lines.
- v4 views forward AccessKit `toggled` to GPUI (`aria_toggled`), so radio
  buttons and toggles report checked to VoiceOver everywhere.
- Panel facts carry the viewer's `layerFailures`; panels republish when the
  status line changes. Inspector: disclosure replies are applied (groups
  previously never opened), section `note`, `finishes`, hidden rows and a
  target `error`.

### Engine findings from the UI pass (for the engine lane)

- **One bad layer blanks the whole 3D set.** (Fixed by UI dependency 2
  above; kept for the record.) `studio/native.rs` `scene()`
  returns an error for the entire scene when one object's mesh can't be
  built (`self.mesh(&object["source"])?`, e.g. a `roundedBox` primitive) or
  when an object uses a Screen material or part offsets. The viewer then
  reports every 3D layer as not drawn. The requirement is that a broken
  model, font or texture affects only its own layer: skip that object and
  report a `LayerFailure` for its layer ID only. Properties already shows
  the per-layer error and names the culprit on the other layers.
- Until composition capture lands, Properties offers Screen compositions
  disabled ("Screens aren’t available in this version yet.") because
  choosing one stops the whole scene from drawing.
- Set Up Studio (`auto_studio`) adds the softboxes and camera, but the
  seamless set draws as a black void in the native renderer.
- Agents' `edit_3d` without a label is now named for the operation
  (`Add Softbox`, `Sky`, `Add Terrain`, `Set Up Studio`…); UI edits carry
  specific names (`Add Pine Trees`, `Import Bottle.obj`, `Finish: Gold`,
  `3D Quality: Final`, `Add Shot: Push-In`).

### Engine findings from round 2

- Fog (`fogDensity` on the world) darkens the foreground to near black in
  Quality, so the landscape defaults use no fog.
- The shadow catcher draws a black void; the first-shape starter uses a
  plain Floor plane instead.
- Fast (Live) draws terrain noticeably darker than Quality. (Fixed by 3077a4128.)

### Still open for the designer

- Status capsule (§3): needs refine progress (samples done / target) in
  `ViewerStatus`.
- Light panel: log sliders (the kit has no `NSSlider`), the Aim segment, and
  motion. Views: navigation and selection in the extra panes.
- 3D handles for extruded text and shapes.
- Export: adopt the export time-estimate state when it lands.
- Engine notes from round 3: one cube face shows a live map mirrored (box
  UVs); Fast shows speckle on extruded faces in an orthographic Front view.
- See `design/reviews/na-studio3d-ui.md` § Round 3 for the keyboard fix
  (v4 tab stops, focus-visible) and what's left there.


### Quiet-window engine findings (2026-11-11)

The finished designer round 3 (`94b3f3c72`) is merged into this branch; no
panels, controls or layout were designed by the engine lane.

- `ViewerStatus.refine_progress` carries `samplesDone`, `samplesTarget` and
  `stage` (`sampling`, `denoising`, `complete`). Counts advance after completed
  four-sample GPU batches, including the first zero and last completed count.
  A superseded viewer stamp cancels between batches and cannot publish stale
  progress. Denoising remains a separately reported stage. The native built-in
  facts expose `studioRefineProgress` and `studioExportEstimate`, with updates
  on state changes. The designer can build the capsule/ring on this data.
- Native primitive/captured-material maps now use top-down image rows through
  an explicit legacy-map V-orientation flag. Authored UVs, tangent frames and
  imported physical/glTF UV transforms are retained. An asymmetric four-colour
  capture checks all six cube faces in Live, Refine and Final; all 18 views
  pass. The reproduced problem was a vertical inversion of the artwork.
- Dense orthographic Front text exposed noisy metallic bevels with half-size,
  single-sample coverage. Orthographic Live now keeps full pixel coverage,
  four stratified samples and the existing linear temporal/edge-aware filter.
  Perspective Live keeps MetalFX and its existing resolution/sampling policy;
  terrain's existing fallback does not gain extra samples. A 640×360 synthetic
  text regression against a 512-sample reference measures dark outliers
  (luminance deficit > 0.12 on bright opaque text). Baseline: **12.78%**;
  fixed: **0%**. Acceptance: <1% dark outliers, luminance RMS <0.05. The
  simple flat-front test also checks standard deviation <0.025.

Noise acceptance already covers independently seeded 4096-sample references
for the teapot, glass cup, chrome sphere and chrome product set. Production
and Final must satisfy luminance RMS <0.02, smooth-region residual RMS <0.006,
block-bias p95 <0.012 and >100 smooth pixels. Independent emitter photon IDs
ensure the chrome reference cannot share the tested image's baked photon map.
These are automated numeric acceptance checks, not a claim of Cycles visual
matching for the deferred fog/subsurface/sheen/sky work.

Validation: **54 native Studio integration tests passed (12.41 s)**, including
viewer-status/stale-progress and cancellation regressions. Engine-service unit
checks: **100 passed, 1 intentionally ignored (23.72 s)**. Their initial run
hit eight pre-existing storage-path refusals because macOS's `/var` temporary
path is a symlink; repeating with a canonical `/private/tmp` test directory
passed all checks. Native app build: **19.12 s**, unchanged build profile.


### Quiet-window performance, first measured increment (2026-11-11)

The driver is now exercised on screen. It rejects stale/failed/null-display
submissions, deduplicates display callbacks and native input origins, includes
phase-boundary gaps, and requires 100 distinct presented inputs for input-p95
acceptance. Offline checks cover known 120 Hz/25 ms traces, duplicate callbacks,
a 200 ms stall, null displays and missing input. Development dylibs are passed
with `--library-dir` after the system lock/Python launcher. The test app quits
in the driver's cleanup path; only private project/profile copies are used.

Same animated 1920×1080 product composition, **1464×824** viewer, Fit 38%, Auto
at Full, 60 fps composition, one view, Metal RT/MetalFX. Each playback phase is
60 s after 8 s warmup. These viewport measurements supersede the older busy
27 fps observation for this configuration; the older viewport differed.

| Measurement | Quiet baseline | Material cache + queued Live trace |
|---|---:|---:|
| Presented fps | 31.6245 | 32.1849 |
| Largest presented gap | 74.999 ms | 91.666 ms |
| Gaps at least 100 ms | 0 | 0 |
| Render submission wall time, median / p95 | 31.624 / 34.776 ms | 31.051 / 38.840 ms |
| Drawable acquisition, median / p95 | 0.017 / 0.030 ms | 0.016 / 0.031 ms |
| Completion to actual display, median / p95 | 20.665 / 23.538 ms | 20.881 / 28.158 ms |
| Native input p95 | insufficient samples | 57.074 ms, 179 inputs |

The native input sequence uses paced Page Up/Down events delivered to the app,
not perf-gate seeks. The initial arrow-key sequence produced no changed frames
and is explicitly excluded. This measures frame-step latency; it does not yet
establish gizmo/orbit latency. **Live and input gates remain failing.**

The first measured fixes cache immutable material receiver references per
composition and prepare material captures once per scene request/tier within
a compositor frame. A new frame and each surface query still observe resource
changes. Perspective MetalFX Live queues tracing directly before guides; the
shared wgpu queue preserves ordering and the Metal-texture handoff retains its
completion barrier. Guide timings include deferred tracing in this mode, so
submission-only `trace_ms` is never presented as a GPU-speed gain. Orthographic
and portable fallbacks retain their synchronized paths. Camera navigation now
attaches the native event origin too and retains the oldest outstanding origin
through superseded frames.

Checks: 54 native Studio integration tests pass after the cache/queue changes;
the shared two-output compositor regression verifies one preparation per frame
and a new preparation on the next frame. Camera supersession/origin regression
passes (1.20 s). Native app rebuild passes (14.23 s).

The headless timing utility now uses the native sample floors/ceilings and no
convergence deadline, includes Draft, uses half-resolution OIDN guides except
Final, and measures fast PNG compression as native export does. Its `ready_ms`
excludes output readback/PNG encoding; `frame_ms` includes both. This is a
single Studio frame diagnostic, excluding animated shutter samples, native
2D compositing, file writes and the surrounding export process.


CPU sampling of the same product scene identified scene resolution/property
queries as a major cost before the renderer's own `prepare_ms` timer: Gaussian
splat detection asks for the shared resolved scene first. The kernel's native
Studio evaluator now returns the exact `evP` constant case directly (a stored
value with no expression or keys). Expressions, keyframes and malformed inputs
keep the original evaluator path; world/parent/opacity evaluation is retained.
A full-scene equivalence test compares constants to the normal expression
`value` at three animated times. Checks: 32 kernel tests (0.26 s), 54 native
Studio tests (11.96 s), native app build (133 s after janitor cache pruning).
No compiler profile, RUSTFLAGS or dependency-feature settings changed.

A diagnostic chrome Production run with caustics disabled still spent
**23.423 s tracing** (256 spp, 1080p), versus **22.394 s** with the fixed photon
map. This excludes photon-grid lookup as the leading cause of the chrome
Production budget failure; no speculative photon-cache approximation was added.


### Queued MetalFX and constant-channel result

MetalFX now retains the renderer's own Metal command queue, following the
native viewer's existing tracked-texture producer/consumer architecture. Native
texture output no longer blocks the CPU after guides or MetalFX. All guide,
scaler, pack, retained-layer copy and viewer work remains ordered on that queue.
Three retained scaler command buffers bound backpressure; context destruction
waits for its own pending buffers. CPU/readback callers retain completed output,
and `STUDIO_SYNC_METALFX=1` selects the waited diagnostic path.

`PipelineTimings.metalfx_deferred` distinguishes queued submission times from
completed work. In that mode `metalfx_gpu_ms` is the most recently completed
scaler buffer, which can initially be zero. Neither queued `trace_ms` nor
queued guide/MetalFX submission times are represented as complete GPU timings.

Three native surface tests pass (0.86 s), including retained-layer masking/flip
and 24 queued-versus-waited camera frames across four size transitions. Maximum
image difference is a few quantization levels, with RMS below 0.5/255. A
second waited run measures MetalFX's own numerical variation as a control;
queued errors must also stay within twice that control (floors 0.25/255 RMS
and 4/255 maximum), with hard limits RMS <0.5/255 and maximum <=8/255. All 54 native Studio tests pass (11.60 s), app build 19.39 s.

At the same 1464×824 viewer and 60 fps product composition: **59.9423 fps**,
**41.666 ms** largest gap, **zero** gaps >=100 ms; native frame-step input p95
**49.163 ms** over **177** distinct presented inputs. Input passes. The strict
>=60 fps gate is still marked failing at this measured composition cadence;
no rounding-up or changed gate is used. The next throughput check uses the
same shot at a higher authored cadence to distinguish a cadence limit from
engine throughput. The 60 fps result is retained independently.


### Quiet-window final handoff (2026-11-11)

**Landed this round:** chrome caustic/noise regression `5e19d23cc`, measured
export-estimate command/state `4ffd5dcfa`, designer round-3 merge `d2fde7a01`,
Refine progress/cube capture UV/orthographic bevel fixes `d5faa07a8`, material
scan caching/queued Live trace `fd11ec095`, constant-channel evaluation
`f4d8a6a01`, and native shared-queue MetalFX `59dd30963`. Remaining changes in
this final increment parallelize CPU image-row work and finish measurement
helpers; no app controls, colours, panel layouts or UX were designed here.

**Live and input:** the same torus/three-light product setup at an authored
120 fps cadence, with a visible key window, one view and **1464×824** Full
viewer output, presents **78.1725 fps** over 60 s. Largest gap **49.9995 ms**,
**zero** gaps >=100 ms. Native frame-step input p95 **40.8202 ms**, **133**
distinct presented inputs. Both throughput/input gates pass in this run.
The separate 60 fps composition result remains **59.9423 fps** / **49.1634 ms**
input p95; it is not rounded into a strict >=60 pass. Input sampling covers
frame stepping, not a claim of gizmo/orbit p95. Larger terrain/scatter sets
have not received this on-screen throughput acceptance run.

**CPU denoising:** guide reduction/expansion, guide packing and RGB copies use
at most eight workers over disjoint rows on large images, retaining the same
per-pixel arithmetic, HDR/filter policy, full-size beauty and half-size guides
except Final. Small images stay serial. Odd dimensions and padded colour
buffers are covered, including preserving extra channels and unused tails.
No build profile, RUSTFLAGS, dependency features, sample targets or convergence
thresholds were lowered for performance.

1080p single-frame Studio diagnostic, seconds (one measured frame per row;
these are not percentile measurements):

| Product / preset | Trace | OIDN incl. guide preparation | GPU guide readback + packing | Image ready | Fast PNG total |
|---|---:|---:|---:|---:|---:|
| Hero / Draft | 0.182 | 1.187 | 0.131 | 1.525 | 1.575 |
| Hero / Refine, 32 spp | 1.051 | 0.941 | 0.122 | 2.130 | 2.173 |
| Hero / Production | 2.589 | 0.947 | 0.127 | 3.679 | 3.722 |
| Chrome / Draft | 1.421 | 0.954 | 0.183 | 2.574 | 2.646 |
| Chrome / Refine, 32 spp | 3.787 | 0.941 | 0.181 | 4.925 | 5.003 |
| Chrome / Production | 22.341 | 0.946 | 0.192 | 23.496 | 23.567 |

Before CPU row parallelization, hero Production was **4.219 s** (OIDN
**1.383 s**) and chrome Production **24.099 s** (OIDN **1.311 s**). Hero
Refine image-ready time was **2.784 s**; chrome **5.662 s**. Output readback is
now about 1–2 ms; fast PNG encoding about 41–76 ms. The first Draft row also
contains cold OIDN/filter overhead. Native 2D compositing, file writes, animated
shutter samples and export-process overhead are excluded. **Draft <=1 s,
Refine <=2 s and Production <=3 s at 1080p remain unmet.** Final retains no
convergence time cap and was not included in this timing sweep. The documented
Metal OIDN build is still deferred as instructed; no OIDN source build occurred.
Chrome needs camera/shadow/transmission-path optimization in addition to OIDN;
disabling caustics did not reduce its leading trace cost.

**Memory:** `studio3d-fixed-memory.py` alternates two fixed project files in
one process (white torus and glass sphere), four warmup A/B cycles followed
by ten measured cycles. It never grows a project catalog. Both use the same
1464×824 output and live tier. Metal allocation stays exactly **613,564,416 B**
for A and **613,531,648 B** for B. RSS stays within **844,955,648–852,852,736 B**
for A and **846,528,512–852,508,672 B** for B. Fitted RSS slopes are
**76,260 B/cycle** and **47,265 B/cycle**; Metal slopes are **zero**. This is a
flat, bounded trend over the requested fixed-project run. RSS and Metal overlap
on unified memory and are reported separately, never added.

**Validation:** full pm-studio Rust suite **105 passed**, followed by the new
padded-buffer regression (**1 passed**); native Studio integration **54 passed**;
engine-service units **101 passed, 1 intentionally ignored**; kernel Studio
**32 passed**; shared two-output compositor preparation regression passes.
The Cycles yardstick regression passes all **11 scenes** against retained
validated Cycles references. Portable traversal (`STUDIO_NO_RT=1`) also passes
the six-face/all-tier capture orientation regression (**1.21 s**). Native app
build passes (**19.50 s** for the final buffer fix). No Blender dependency was
introduced into the app.

The three automated noise tests pass against independent 4096-sample references
for teapot, glass, chrome and the multi-object chrome set. Production/Final
limits remain RMS <0.02, smooth residual RMS <0.006, block-bias p95 <0.012.
These fixtures are **192×128**, not a full-HD noise certification. The separate
orthographic text regression is 640×360. Full-HD image-quality acceptance and
the export budgets above still need further work.

**UI handoff:** the designer can read `studioRefineProgress` and
`studioExportEstimate` facts, call `Viewer::estimate_studio_export`, and use
its measured per-preset ranges/warnings/exclusions. The estimate is extrapolated
from a bounded calibration; CPU parallelism and OIDN scaling make that an
approximation. The existing export sheet's older heuristic summary has not been
redesigned by this lane. Capsule/ring presentation remains with the UI owner.

Reproduce a throughput run, with a fresh evidence directory and a private
product file authored at the stated cadence:

```sh
nice -n 10 flock /tmp/pm-build.lock python3 apps/native/scripts/studio3d-quiet-perf.py \
  --app /path/to/Powermove.app/Contents/MacOS/powermove-native \
  --library-dir "$HOME/Library/Caches/powermove-engine/ffmpeg/out/lib" \
  --project /path/to/product.pmv --out /tmp/pm-shots/na-studio3d/NEW-RUN \
  --focus --seconds 60 --input-seconds 75
```

During the input phase deliver paced native Page Up/Down events to the viewer;
>=100 distinct presented event origins are required. The helper checks the
visible/key window before warmup and drains the last input before quitting.
Both helpers refuse `USER_ACTIVE` and low storage; every launched test app
window has been closed. Evidence stays in `/tmp/pm-shots/na-studio3d/`.

On this Mac, run the final built app with a private copy of the 3D product scene:

```sh
nice -n 10 flock /tmp/pm-build.lock /tmp/pm-shots/na-studio3d/run-review.sh
```

The launcher checks the disk floor and supplies the development FFmpeg path
after the system lock launcher. For a source build, use the native README's
`cargo +1.96.1 run -p pm-native-app --bin powermove-native` from `apps/native`,
with the documented shared target/runtime environment and the same lock.

### Export-budget round: overhead checkpoint (2026-11-11)

Targets remain Draft <=1 s, Refine <=2 s, Production <=3 s at 1920×1080;
Final has no wall-clock cap. Sample floors, error thresholds and numeric noise
limits are unchanged. Engineering measurements below are headless worker
measurements under `nice -n 10 flock /tmp/pm-build.lock`, interleaved with the
UI lane's builds. They are not the final on-screen/acceptance run.

The diagnostic now reports scene preparation/upload separately and the actual
per-pixel sample distribution. Baseline hero preparation/upload: 13.6/13.0 ms;
chrome: 271.9/13.8 ms. Renderer/device/pipeline initialization is separately
reported and excluded from steady-frame totals. Native already retains mesh
BLASes and scene resources across frames; changing camera/instances does not
rebuild unchanged mesh geometry. PNG costs about 40–80 ms and readback 1–2 ms,
so moving encoding alone cannot fix the leading trace or OIDN costs.

Two exact overhead reductions are implemented:

- An unmapped, fully opaque shadow caster does not need a shading frame,
  normal/coat textures or procedural BSDF evaluation. Mapped/vertex alpha,
  noncasters and transmission retain the complete path. White-alpha-map
  fallback parity passes with both portable and hardware traversal.
- Global convergence reads a GPU-reduced maximum sample count (4 bytes),
  instead of downloading the entire RGBA accumulation every 32 samples.
  The stopping inequality and per-pixel sampling decisions are unchanged.
  Odd dimensions, continued refinement and resize are covered.

Initial worker results with these changes (single-frame diagnostics, seconds):

| Product / measure | Baseline | Optimized overhead |
|---|---:|---:|
| Hero / Draft PNG | 1.608 | 1.540 |
| Hero / Refine ready | 2.154 | 2.078 |
| Hero / Production PNG | 3.677 | 3.356 |
| Chrome / Draft PNG | 2.625 | 2.506 |
| Chrome / Refine ready | 4.949 | 4.449 |
| Chrome / Production PNG | 23.715 | 20.521 |

Chrome Production still averages about 91 actual samples/pixel, with its
95th percentile at the 256 ceiling; hero averages 38.7 with p95 88. The
expensive chrome pixels still need structural sampling work. A diagnostic
Draft GPU-guided filter (same trace samples) removes CPU OIDN: hero 0.217 s,
chrome 1.336 s. This diagnostic choice is not yet the native Draft policy.

`noise_quality::full_hd_product_noise_acceptance` is an explicit ignored test
for teapot, glass, chrome and the mixed chrome set at 1920×1080. It retains
4096 independent reference spp, Production/Final thresholds RMS <0.02,
smooth noise RMS <0.006 and block bias p95 <0.012. Held reference caches check
physical scene/sampling metadata, dimensions and decoded pixel fingerprints;
renderer provenance is recorded outside Git. Use `PM_STUDIO_NOISE_SCENES`
to run one fixture per lock acquisition. The first expensive reference
attempt was stopped to prioritize engineering; it is not counted as a pass.

#### Draft filter and occupied-region denoising increment

Native Draft now selects the existing GPU spatial filter through the resolved
`FinalQuality.fast_denoise` policy; the saved/export quality command and measured
estimate calibration carry that choice. Draft retains its 4–16 spp, 0.12 error
threshold, four bounces and full output resolution. Refine, Preview, Production
and Final retain OIDN. The native regression checks no OIDN/guide readback work,
full-sized output and one geometry/texture upload over two frames.

OIDN beauty work may crop a zero-surrounded product with a 64-pixel context halo.
Beauty pixels, guides and output retain their resolution; the guides are still
prefiltered. Full-image row strides, RGB-only copying, additional channels and
buffer tails are preserved. Cropped/full-frame object RMS must stay <0.005 in
the large padded-buffer regression. Nonzero surroundings, small images and a
product covering most of the frame retain full-frame filtering. The denoiser
reports `beauty_pixels` for diagnostics. Final retains full-frame beauty context
as well as full-resolution guides. Initial warm hero Refine is 1.871 s;
hero Production PNG is 3.057 s before the latest classifier refinement. These
are engineering measurements, not the final acceptance table.

Near-mirror, uncoated conductors use the complete visible-normal BSDF estimator
for offline direct light; the complementary emitter/environment MIS weight is
removed too, preserving energy. Live and rough/layered surfaces retain their
existing estimator. A finite-emitter energy regression covers both traversal
paths. Opaque contact-occlusion rays may terminate on an arbitrary opaque hit;
alpha/noncaster cases restart closest-first from the original origin.

The edge classifier now distinguishes statistically significant image contrast
from first-batch Monte Carlo variance. Geometric/coverage edges retain their
unconditional fourfold sample floor; deterministic fine detail keeps the same
contrast threshold. A synthetic GPU regression rejects noisy flat fields and
retains a real detail edge. Pixel error thresholds are unchanged.

An unfinished-pixel compaction prototype preserved pixel samples but produced
no useful chrome speedup; it was removed from the branch. Full-HD acceptance
also measures the product region separately from its floor/backdrop, using the
same numeric limits, so a large clean background cannot hide object noise.
Explicit sample-ceiling trials are logged as trials; the saved Production
ceiling remains 256 pending that acceptance.

#### Export round storage-stop handoff (2026-11-11)

Green increments are `e958100e1` (shadow shading/global convergence readback)
and `ace8a65b4` (GPU Draft, occupied beauty filtering, specular sampling and
variance-aware edge classification). Targeted verification: **57 engine units,
25 engine integration cases, one native Draft regression**, and a native app
build pass. The Final full-context denoiser regression was rerun after its final
guard. No sample/noise target or saved Production ceiling was lowered.

Warm second-frame component measurements, seconds, before the last classifier
refinement (engineering trials, not final acceptance):

| Product | Draft PNG | Refine image ready | Production PNG |
|---|---:|---:|---:|
| Hero | 0.225 | 1.871 | 3.057 |
| Mixed chrome | 1.146 | 3.894 | 18.352 |

These exclude device/pipeline startup, native 2D compositing, motion-blur shutter
samples and export-process overhead. Other UI preview processes became active
during later experiments. No final quiet timing or on-screen gate pass is
claimed for this round. Chrome remains trace-bound; its budgets remain unmet.

Full-HD **1920×1080** Production/Final noise acceptance has completed for teapot
and glass against independent 4096-spp references. The following table reports
the **product region**, using the same limits as the whole-image checks; both
regions pass. Smooth-pixel counts exceed 187,000 in every row.

| Product / policy | Luma RMS (<0.02) | Smooth residual RMS (<0.006) | Block bias p95 (<0.012) |
|---|---:|---:|---:|
| Teapot / Production | 0.001736 | 0.001226 | 0.001569 |
| Teapot / Final | 0.001216 | 0.001127 | 0.000525 |
| Glass / Production | 0.010996 | 0.003900 | 0.004742 |
| Glass / Final | 0.007439 | 0.003842 | 0.003405 |

Chrome and the mixed chrome set still need their full-HD references and gates.
Reference caches retain physical-scene/sampling metadata and decoded pixel
fingerprints; no reference images are checked into Git. Cached reruns can
finish the remaining gates without repeating the completed references.

The tile experiment preserved HDR film and stopping decisions but added
submission/probe overhead, so it was removed. Plain visibility resampling
increased sample variance. Cached visibility and secondary light-tree sampling
remain uncommitted experiments, archived outside Git for the next engineering
round; their final combined source is uncompiled and unverified.

Work stopped when available storage fell below the required 4 GB floor. No
further builds, renders or timing runs were started. Remaining work is the
unverified trace experiments, full-HD chrome acceptance, the complete regression
suite/Cycles yardstick after the final engine choice, and quiet final native
performance measurements. All test app windows launched by this lane are closed
(this round used headless workers only).

#### Lifecycle after UI round 4

Integrated UI round 4 (`d72d3ab2c`) before validating lifecycle and export.
The shell installed its unconditional close callback after the application's
save/durability callback. GPUI retains one callback, so the close button removed
the window without closing its kernel session. A subsequent Quit closed the
remaining visible project but could not release the library. Reproduced the
exact library-release error after a copied session's close-button action. The shell now
retains the application's callback; the application forgets the window session
only after its durable close succeeds.

A separate reopen defect affected file-only library entries whose intact file
was under `recovery-files`. The open route excluded backing files even when no
library body remained, then looked for a journal that a successful close had
already retired. Such entries now open their actual file with normal locking
and recovery. Recovery copies that retain a library body continue to use it,
including changes newer than their media backing file. Library failures retain
their kernel diagnostic in the existing local Error Log.

Verification so far: native app build passed; three kernel lifecycle/recovery
regressions passed; the Error Log regression passed. An explicitly supplied
disposable walkthrough profile opens with 12 accepted undo-history entries.
The recovered portable copy preserves the document, history and embedded media
byte for byte, with all container checksums verified. The original profile is
untouched. The real shell close-callback regression also passes: a factory's
close refusal keeps its window alive. Running-app verification opened the
walkthrough project from Home, opened a second project in a separate window,
closed the second with its close button, retained the first project, and quit
normally. No new performance acceptance is claimed yet.
