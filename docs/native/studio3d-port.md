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

## Main capability inventory

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Ordinary model/light/camera timeline layers | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Existing scene-container/mesh project migration | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Primitives: box, sphere, plane, cylinder, cone, torus, capsule, icosahedron | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Indexed mesh, lathe and polygon extrusion sources | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| glTF/GLB (including mesh instances), OBJ import | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Embedded glTF animation/skinning | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| glTF source material names/indexes, vertex colors and six standard texture slots | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Extended glTF sampling, texture transforms and physical-material maps | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Imported parts tree, picking and selection | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Part position/rotation/scale/visibility offsets | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Parent/group transforms, anchors, opacity and solo | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer and group 3D switches | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| 2D solids/images/video/text/shapes/precomps in 3D | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Flat text/shapes remain flat until extrusion enabled | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Text outlines, font loading/fallback and holes | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shape extrusion, compound paths and holes | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Animated extrusion depth/bevel and quality | U (pinned engine; native adapter pending) | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Angular/concave/convex bevels, inner hole bevel depth | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front/Bevel/Side/Back independent materials | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| PBR color/roughness/metalness/emission/opacity | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Color/normal/roughness/metalness/emissive/AO image maps | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Source materials versus whole-model overrides | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material opacity/roughness/metallic/emission color/intensity/ambient response | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live composition textures on named model materials | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Live layer material sources in same composition | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layer material source timing, masks, effects; transform/opacity excluded | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material source missing/loading/relink/undo/save lifecycle | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Sun/parallel, point and spot lights | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light distance/decay, spot angle/penumbra and visibility | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light cast-shadow toggle, model cast/receive-shadow toggles | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Global shadows and ambient color/intensity/exposure | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Light markers, select and move/target gizmos | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Perspective and orthographic cameras | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Animated camera position/target/FOV/zoom/near/far | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Camera and free/editor view; orbit/pan/zoom/frame | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Move/rotate/scale gizmos, scene/object axes, modal transforms | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Projected bounds, ray picking, model/part selection | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Mixed 2D/3D depth sets and transparent layer ordering | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Per-layer effects/masks/blend modes and opacity | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Viewer, agent render_frames and still capture | U (pinned engine; native adapter pending) | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Video/image-sequence export, nested composition and motion-blur sampling | U (pinned engine; native adapter pending) | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Extension api.scene3d and revision-guarded edit_3d/get_3d_scene | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Validation, budgets, undo/redo, serialization and asset bundles | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
## Material-source parity with main reviews #187 / #188

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Groups as Base Color or Emissive | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Cycles and local links | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Stable group source canvas | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Safe group capture | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera/UV-driven detail | U (pinned engine; native adapter pending) | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Capture budgets | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Texture sharing/replacement | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Static capture reuse | U (pinned engine; native adapter pending) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Evaluated bounds and bounds null | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Comp navigation clocks | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Imported paint alpha | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Path visual bounds | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
## Requested AE 27.0 checklist

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Environment Light and HDRI | U (pinned engine; native adapter pending) | G | U | U | G | World/source/atmosphere channels → world edits pending; existing `set_environment` |
| Parallel / spot / point lights | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Ambient / environment light types | U (pinned engine; native adapter pending) | G | U | U | G | World/source/atmosphere channels → world edits pending; existing `set_environment` |
| Material Options: Casts Shadows | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Shadows | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Accepts Lights | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Material Options: Appears in Reflections | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Layers as materials, same-comp source | U (pinned engine; native adapter pending) | P | K | K | P | Material channels, slots, source dependencies → `apply_material`; new slot/source edits pending |
| Keyframeable Opacity/Roughness/Metallic | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Keyframeable Emission Color/Emission Intensity/Ambient Response | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Substance SBSAR | G (upstream gap) | G | U | U | G | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Front / Bevel / Side / Back | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Camera depth of field/focus | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Camera iris controls | U (pinned engine; native adapter pending) | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Multiple views | U (pinned engine; native adapter pending) | G | U | U | G | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| glTF embedded animation | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| FBX with embedded animation | U (pinned engine; native adapter pending) | G | U | U | G | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| 2D layers casting shadows onto 3D | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Render passes | U (pinned engine; native adapter pending) | G | U | U | G | Tier, quality, progress, result → render/export requests; saved quality pending |
## Studio features to retain

| Capability | Engine | Document / kernel ops | Persistence | Undo | MCP / agent | UI hook (reads → commands) |
| --- | --- | --- | --- | --- | --- | --- |
| Live / Refine / Final path tracing | U (pinned engine; native adapter pending) | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Fast / Quality preview | U (pinned engine; native adapter pending) | P | K | K | P | Tier, quality, progress, result → render/export requests; saved quality pending |
| Finish presets and rendered swatches | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Softbox lights and light-by-pointing | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Auto studio, seamless set, shadow catcher | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Shots and exploded views | U (pinned engine; native adapter pending) | P | K | K | P | Camera channels, transient views → `set_camera`, navigation; iris/views pending |
| Drop to floor / place on surface | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Live screens | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| STEP/IGES/BREP CAD import | U (pinned engine; native adapter pending) | P | K | K | P | Import progress, retained asset, parts → worker admission + `Scene3dCommand::Import` |
| Gaussian splats and point clouds | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Advanced glass/coat/sheen/film/dispersion/droplets/brushed/subsurface | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |
| Native backend attachment | U (pinned engine; native adapter pending) | P | K | K | P | Resolved model/light/camera and selection → `edit_3d`; ordinary property/keyframe commands |

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
| Rust renderer and native regressions | Pass | Not yet run on port |
| Bounded Cycles yardstick | Pass | Not yet run; Blender test-only |
| Live presented rate | ≥60 fps | Unmeasured |
| Input → pixel p95 | ≤50 ms | Unmeasured |
| Refine | 32 spp + OIDN ≤2 s | Unmeasured |
| Production | 1080p ≤3 s/frame, no visible noise | Unmeasured |
| Save/reopen and Undo | Every authored operation | Baseline tests; new contract pending |
| Electron project compatibility | Same evaluated scene | Pending |

Upstream diagnostic measurements in the pinned `docs/studio/native-host.md`
are not native-app acceptance. No performance or Cycles-equivalence claim is
made from them.

## Reproduction and designer handoff

Use the shared native debug target and `/tmp/pm-build.lock` for every build,
test, package and timing run. Never clean it; retain at least 8 GB free.
Artifacts stay outside Git. Baseline launch (Studio attaches to model layers):

```sh
cd apps/native
export CARGO_TARGET_DIR="$HOME/Developer/pm-native-engine/apps/native/target"
flock /tmp/pm-build.lock cargo run -p pm-native-app --bin powermove-native
```

`apps/native/scripts/bundle-dev.sh` describes the separate shader compiler
needed by effects. No release, tag or notarization. All control presentation
and visual acceptance belong to the designer.
