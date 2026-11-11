# Studio 3D native port

Implementation handoff for the engine/data lane. Opus owns all presentation.
This lane adds state, commands and worker adapters only.

## Source and boundaries

- Native base: `a93fee099bbaa09519640220388a77879bcdbfc7`.
- Pinned Studio source: `bd30e2037c840e162e4cf2694851f30679bec8f1` (`origin/studio-3d`).
- Import `engine/crates/pm-studio` whole. Its host package is named
  `pm-studio-host`, but its source is `engine/hosts/studio-electron`.
- Subsequent renderer fixes are separate commits containing only `engine/`.
  Native integration belongs under `apps/native/`.
- One in-process renderer on the viewer worker and shared wgpu device. Output
  flows through the existing compositor and below-slot EDR CAMetalLayer.
  No Electron helper or IPC texture transport. Blender is test-only.
- Retain portable WGSL traversal when Metal ray queries are unavailable.

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
| Flat text/shapes remain flat until extrusion enabled | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Text outlines, font loading/fallback and holes | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shape extrusion, compound paths and holes | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Animated extrusion depth/bevel and quality | Shared engine present; row integration unverified | P | K | K | P | Saved quality, tier, progress → `set_render_quality`, export `studioQuality`; automatic tier scheduling pending |
| Angular/concave/convex bevels, inner hole bevel depth | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front/Bevel/Side/Back independent materials | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| PBR color/roughness/metalness/emission/opacity | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Color/normal/roughness/metalness/emissive/AO image maps | Native retained-image and indexed replacement checks | Connected image sources; live capture pending | Tested sparse records | Tested | Tested | Read maps/source material IDs → `update_object` material.maps or sourceMaterials; live sources pending |
| Source materials versus whole-model overrides | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | Native sparse values, source IDs and rendered emission checked | Connected | Tested PMV | Tested Undo/Redo | Tested | `update_object` patch `materialProperties` / `sourceMaterialProperties`; `mi.*` / `mi.index.*` channel edits |
| Live composition textures on named model materials | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live layer material sources in same composition | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layer material source timing, masks, effects; transform/opacity excluded | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material source missing/loading/relink/undo/save lifecycle | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
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
| Groups as Base Color or Emissive | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Cycles and local links | Shared engine present; row integration unverified | P (validated state; rendering pending) | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Stable group source canvas | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Safe group capture | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera/UV-driven detail | Shared engine present; row integration unverified | G | U | U | G | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
| Capture budgets | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Texture sharing/replacement | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Static capture reuse | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
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
| Multiple views | Shared engine present; row integration unverified | G | U | U | G | Camera/iris channels, transient views → `set_camera`, navigation; multiple views pending |
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
