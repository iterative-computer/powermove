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
contract is a separate, incomplete `feat/na-studio3d-wip` checkpoint; the native
renderer translation for those new document nodes is not finished.

## Main capability inventory

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Ordinary model/light/camera timeline layers | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Existing scene-container/mesh project migration | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Indexed mesh, lathe and polygon extrusion sources | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| glTF/GLB (including mesh instances), OBJ import | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Embedded glTF animation/skinning | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| glTF source material names/indexes, vertex colors and six standard texture slots | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Extended glTF sampling, texture transforms and physical-material maps | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Imported parts tree, picking and selection | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Part position/rotation/scale/visibility offsets | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Parent/group transforms, anchors, opacity and solo | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer and group 3D switches | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| 2D solids/images/video/text/shapes/precomps in 3D | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Flat text/shapes remain flat until extrusion enabled | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Text outlines, font loading/fallback and holes | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shape extrusion, compound paths and holes | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Animated extrusion depth/bevel and quality | Shared engine present; row integration unverified | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Angular/concave/convex bevels, inner hole bevel depth | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front/Bevel/Side/Back independent materials | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| PBR color/roughness/metalness/emission/opacity | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Color/normal/roughness/metalness/emissive/AO image maps | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Source materials versus whole-model overrides | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live composition textures on named model materials | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live layer material sources in same composition | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layer material source timing, masks, effects; transform/opacity excluded | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material source missing/loading/relink/undo/save lifecycle | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Sun/parallel, point and spot lights | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light distance/decay, spot angle/penumbra and visibility | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light cast-shadow toggle, model cast/receive-shadow toggles | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Global shadows and ambient color/intensity/exposure | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light markers, select and move/target gizmos | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Perspective and orthographic cameras | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Animated camera position/target/FOV/zoom/near/far | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Camera and free/editor view; orbit/pan/zoom/frame | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Move/rotate/scale gizmos, scene/object axes, modal transforms | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Projected bounds, ray picking, model/part selection | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Mixed 2D/3D depth sets and transparent layer ordering | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer effects/masks/blend modes and opacity | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Viewer, agent render_frames and still capture | Shared engine present; row integration unverified | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Video/image-sequence export, nested composition and motion-blur sampling | Shared engine present; row integration unverified | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Extension api.scene3d and revision-guarded edit_3d/get_3d_scene | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Validation, budgets, undo/redo, serialization and asset bundles | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

## Material-source parity with main reviews #187 / #188

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Groups as Base Color or Emissive | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Cycles and local links | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Stable group source canvas | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Safe group capture | Shared engine present; row integration unverified | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera/UV-driven detail | Shared engine present; row integration unverified | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
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
| Environment Light and HDRI | Shared engine present; row integration unverified | G | U | U | G | World/source/atmosphere channels → world edits pending; existing `set_environment` |
| Parallel / spot / point lights | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Ambient / environment light types | Shared engine present; row integration unverified | G | U | U | G | World/source/atmosphere channels → world edits pending; existing `set_environment` |
| Material Options: Casts Shadows | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Shadows | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Lights | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Appears in Reflections | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layers as materials, same-comp source | Shared engine present; row integration unverified | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Keyframeable Opacity/Roughness/Metallic | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Substance SBSAR | G (upstream gap) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front / Bevel / Side / Back | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera depth of field/focus | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Camera iris controls | Shared engine present; row integration unverified | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Multiple views | Shared engine present; row integration unverified | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| glTF embedded animation | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| FBX with embedded animation | Shared engine present; row integration unverified | G | U | U | G | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| 2D layers casting shadows onto 3D | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Render passes | Shared engine present; row integration unverified | G | U | U | G | Tier, quality, progress, result → render/export requests; saved quality pending |

## Studio features to retain

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Live / Refine / Final path tracing | Shared engine present; row integration unverified | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Fast / Quality preview | Shared engine present; row integration unverified | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Finish presets and rendered swatches | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Softbox lights and light-by-pointing | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Auto studio, seamless set, shadow catcher | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shots and exploded views | Shared engine present; row integration unverified | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Drop to floor / place on surface | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Live screens | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| STEP/IGES/BREP CAD import | Shared engine present; row integration unverified | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Gaussian splats and point clouds | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Advanced glass/coat/sheen/film/dispersion/droplets/brushed/subsurface | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Native backend attachment | Shared engine present; row integration unverified | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

## Environment contract beyond the parity rows

Sky/sun/HDRI and atmosphere, fog volumes, terrain regions/thermal erosion,
seeded scatter/instances, procedural plants/grass/rocks and distance LOD exist
in the pinned engine. Native document authoring/translation are gaps. Expose
ordinary layer definitions/channels, validated actor operations, readback,
retained asset references and inverse patches. No alternate document writer.

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
| Rust renderer and native regressions | Pass | 123 renderer/host checks; 18 adapter checks; 5 native scene/export checks pass |
| Bounded Cycles yardstick | Pass | All 11 scenes pass; included in the 123 renderer checks |
| Live presented rate | ≥60 fps | Unmeasured |
| Input → pixel p95 | ≤50 ms | Unmeasured |
| Refine | 32 spp + OIDN ≤2 s | Engine 4.8–7.0 s; full app unmeasured |
| Production | 1080p ≤3 s/frame, no visible noise | Engine diagnostic exceeds target; full app unmeasured |
| Save/reopen and Undo | Every authored operation | Baseline tests; new contract pending |
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
override and vertex-color flag; row 5.w controls primary visibility. The old
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
  OIDN behavior retains full-resolution guides. Native preset selection still
  needs to choose this option; Final can retain full guides.

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

## Document/data checkpoint on the WIP branch

The WIP branch adds the pinned World/fog/terrain/scatter/material-option schemas,
material/part sparse data, iris channels, ordinary actor/property paths, quality
presets, MCP operation metadata/readback and source validation. Its targeted
checks pass: **23 kernel**, **17 document**, **2 source-contract**, **8 MCP**.
They prove exact Undo/Redo, protected edits, atomic failure, saved quality and
PMV reopening. The old source oracle is retained; its 16-light case is explicitly
superseded by Studio's 4096-light budget. No scene rendering claim follows from
these document tests.

The current-composition serialization previously dropped `studioQuality`.
The WIP fix keeps the frozen v1 `COMP_FIELDS` inventory and adds a current
composition-field set used by canonical projection and native PMV envelopes.
The actor remains the sole author; planned structural staging is validated at
the end of each compound edit, before publication.

Still required before integrating that checkpoint: wire new scene nodes and
hidden prototypes into native extraction; world/HDRI source loading; terrain,
scatter/LOD, atmosphere and volume translation; material source capture and all
imported maps; iris/orthographic transport; composition quality and per-export
quality overrides into actual rendering; FBX native import; render passes;
noise convergence/filter/edge checks and native presentation/input measurements.
The standalone engine supports more than the native document adapter currently
translates. Treat the capability matrix as partial until each row has evidence.
