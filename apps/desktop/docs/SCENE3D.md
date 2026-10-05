# Editable 3D layers

Use `edit_3d` and `get_3d_scene` for 3D work. Each model, light and camera is an ordinary timeline layer. There is no scene container to create or select. Models share depth, lights and shadows in the composition; 2D layers separate model depth groups. Empty pixels are transparent by default. Edits use the normal revision guards, locks, keyframes and Undo.

## Agent workflow

1. Read `get_project_state` and `get_3d_scene` before editing. The latter returns actual layer IDs, roles, content and channel paths, plus an evaluated world for rendering.
2. Add models with `add_object`, lights with `add_light`, and cameras with `add_camera`. Every call creates a separate layer and returns `layer.id` / `layer.layerId`.
3. Update an existing layer by `target` or `id`; verify with `get_3d_scene` and `render_frames`.
4. Animate actual layer channels using `apply_commands`.

```json
{"operation":"add_object","object":{"id":"rocket_body","name":"Rocket body","source":{"lathe":[[0,-1],[0.35,-0.8],[0.35,0.5],[0.2,0.85],[0,1.2]],"segments":48},"material":{"p":{"color":"#C7C4FF","roughness":0.3,"metalness":0.65}}}}
```

```json
{"operation":"add_light","light":{"id":"fill","type":"point","p":{"x":2,"y":3,"z":4,"color":"#88BBFF","intensity":25}}}
```

```json
{"operation":"update_object","target":"rocket_body","patch":{"p":{"x":2,"ry":90},"material":{"maps":{"color":"IMAGE_ASSET_ID","normal":"NORMAL_ASSET_ID"}}}}
```

`create` creates a cube layer. Passing a complete `scene` template to `create` splits its objects, lights and camera into ordinary layers in one edit. `duplicate` and `remove` address real layers. `set_camera` and `set_environment` address the selected or active camera, creating one if absent. The top active camera supplies the composition view and environment. Without an explicit camera or lights, useful studio defaults keep new models visible.

## Geometry and materials

Sources accept one of:

- `{"primitive":"box|sphere|plane|cylinder|cone|torus|capsule|icosahedron","parameters":{...}}`. Parameters include width, height, depth, radius, radiusTop, radiusBottom, tube, segments and detail.
- `{"assetId":"MODEL_ASSET_ID"}` for imported OBJ, GLB or self-contained glTF.
- `{"mesh":{"positions":[x,y,z,...],"indices":[a,b,c,...],"normals":[nx,ny,nz,...],"uvs":[u,v,...]}}`. Indices identify triangles; normals are generated if absent. Texturing needs UVs.
- `{"lathe":[[radius,y],...],"segments":48}` for turned shapes.
- `{"extrude":[[x,y],...],"depth":0.2,"bevel":0.02}` for an extruded closed outline.

Models persist source, material, castShadow, receiveShadow and useSourceMaterials under `layer.d.data.object`. Their IDs, names, parenting, visibility, timing and transforms belong to the native layer. Parent transforms use the normal layer hierarchy. Imported models are centered and normalized to a unit bounding sphere. Model coordinates use world units.

Position and light/camera aim targets are expressed in the parent/group coordinate space. Group rotation carries their aim along; ungrouping, regrouping and reparenting preserve the current world aim. Ungrouping an animated group bakes editable transform and aim channels at composition frames.

Materials support color, roughness (0.02–1), metalness (0–1), emissive, emissiveIntensity, opacity, doubleSided, and image asset maps: color, normal, roughness, metalness, emissive, ao. Color and emissive maps use sRGB; data maps are linear. Map edits replace the entire map set. GLB/glTF with `useSourceMaterials:true` preserves imported materials and embedded textures; turn it off to use the editable material.

Sun, point and spot lights support intensity, color, position, aim target, range/falloff, cone angle/softness and shadows. The active camera holds projection, lens, target, clipping, and environment (ambient, ambientColor, exposure, shadows, optional background). Resource limits bound geometry, textures, objects, lights and shadow maps; simplify large models before import.

## Animation

Transform paths on the actual layer:

- `position.x|y|z` in world units.
- `rotation.x`, `rotation.y`, `rotation` (Z), and `orientation.x|y|z`, in degrees.
- `scale.x|y|z` in percent; 100 is original size.
- `opacity` in percent.

Content paths: `m.color|roughness|metalness|emissive|emissiveIntensity|opacity`, `light.KEY`, `camera.KEY`, `environment.KEY`. Camera position uses native `position` channels; targetX/Y/Z, fov, zoom, near and far use `camera.KEY`. Read returned paths instead of guessing.

```json
{"commands":[{"type":"replace_keyframes","target":"rocket_body","path":"rotation.y","keyframes":[{"time":0,"value":0},{"time":2,"value":180}]}]}
```

For semantic creation and `patch.p`, the convenient aliases are x/y/z, rx/ry/rz in degrees and sx/sy/sz with 1 as original size; they map to native transform channels. Standard `{v,kf,expr}` channels are also accepted.

## Manual editing and extension API

Select a 3D layer in the timeline or composition preview to show its gizmo directly over the model. The native toolbar offers Move, Rotate, Scale and Global/Local axes. Drag axis handles or planes. Ctrl snaps; Shift makes fine adjustments; Escape or losing focus cancels. Each completed gesture is one Undo. Existing animated channels receive a keyframe at the playhead; the inspector stopwatches start animation. Objects keep the standard native Transform section plus a Model/Material section. Lights and cameras have their own inspector sections and position controls.

Grouping models creates a native 3D group with its pivot centered on their geometry in world units. Select the group and use the same Move/Rotate/Scale gizmo to transform its members together; selecting a parent and child applies the transform once. Nested model groups work the same way. Double-click descends into a group using the normal selection behavior. Mixed 2D/3D artwork groups retain the existing 2D controls.

The **3D view** menu in the preview offers Select, Orbit, Pan, Dolly, Frame selection and Frame all. Navigation tools use left-drag and scroll to dolly; Escape returns to Select. Middle-drag or Alt-drag orbits without changing tools; Shift+middle-drag or Shift+Alt-drag pans. In a navigation tool with the pointer over the preview, F frames the selection and Home frames all models. Orbit, pan, dolly and framing change only a temporary Editor view. They never change camera layers, keyframes, project data or Undo, and work even when the render camera is locked or absent. The Camera/Editor switch shows the current viewpoint; Camera returns to the rendered shot, while Editor resumes the inspection view. Picking and gizmos follow the current viewpoint. Exports and agent frame captures always use the actual render camera. In Select, normal wheel/trackpad composition zoom and pan remain available (Alt+scroll dollies the editor view).

Right-click the composition or timeline and choose Add 3D layer for primitives, lights, camera or model import. These actions are also searchable in the command palette. Select OBJ together with its MTL and images to pack a durable GLB. GLB and self-contained glTF preserve materials, UVs, textures and animation; external glTF file/URL references must be packed into a self-contained file first.

Extensions use `api.scene3d.edit`, `describe()`, `prepareImport(files)`, template factories `createScene/createObject/createLight`, and ordinary `api.selection`. `createGizmo(element)` provides the shared composition overlay. `getMode/setMode`, `getSpace/setSpace` and `onGizmoChange` keep native controls synchronized. Extensions must use the public API instead of importing Three.js or modifying editor internals.

Existing OBJ/scene layers retain their compatibility renderer. Convert to 3D Layers, also available by double-clicking a legacy layer, splits them into ordinary layers in one Undo. Model/material channels are preserved; camera and auto-rotation animation is sampled at project FPS (up to 2,398 frames). Move a keyed legacy background to a separate layer before converting. Preview, native frame export and portable web export use the same renderer.

## Blender-powered motion modeling

Blender 4.5 or newer is an optional local modeling and rendering engine. Powermove keeps the
composition, individual 3D layers, groups, materials, and animation. Install
Blender, or use **Choose Blender…** in the Rendering section for a custom install.
The chosen executable is remembered locally. Powermove runs one background worker
per editor, with project scripts disabled; agents provide validated JSON, never
Python or shell commands.

**Add 3D layer** includes Rocket, Staircase, and 3D Text. Select the resulting
group to change its model parameters and add Bevel, Subdivision, Solidify, Array,
or Mirror modifiers. Regenerate refreshes its mesh parts in one Undo, retaining
their IDs, material assignments, transforms, keyframes, and easing. Removed parts
are hidden and recovered when the recipe includes them again. Count/topology
controls require regeneration; continuous numeric parameters can be animated.
Select a part to edit its materials, or use its Model button to return to the
construction controls. Imported Blender groups offer **Replace source…** instead, letting you choose an updated `.blend` while keeping matching part IDs and edits.

Agents can also construct turned profiles, extruded outlines, or indexed meshes:

```json
{"operation":"create_model","name":"Product pedestal","recipe":{"kind":"lathe","parameters":{"segments":48},"profile":[[0,-0.5],[0.8,-0.5],[0.8,-0.35],[0.55,-0.3],[0.55,0.5],[0,0.5]],"modifiers":[{"type":"bevel","width":0.025,"segments":3}]}}
```

`create_model` returns the group ID and part count. Read `get_3d_scene` for the
part IDs, model recipes, materials, and keyframe channel paths. Animate a group's
`model.length`, `model.radius`, `model.rise`, or other returned numeric parameter
using `apply_commands`. `regenerate_model {target,recipe?}` refreshes that group.
Its existing numeric parameter animation remains authoritative when present.
If the project changes during generation, the operation stops before applying
its result so the user's intervening edits survive.

`import_blend {sourceAssetId,name?,target?}` imports a durable, packed `.blend`
asset. The normal file importer also accepts `.blend`. Pack external textures in
Blender first. Mesh, text, curve, surface, and metaball objects become native
model layers; parent world transforms are preserved. Actual Blender material
graphs and exposed node-group inputs remain in the packed source. Blender scene
cameras, lights, physics, constraints, and imported scene animation are not
converted into Powermove animation; the import captures evaluated model geometry.

## Actual Blender materials

The native Material section offers Surface, Noise, Checker, Brushed metal,
Glass, and Glow presets. These are real Blender shader graphs. Their exposed
inputs have the same fields and keyframe stopwatches as other properties.
Materials may have several surface slots. Choose an existing material to reuse
it, or **Make unique** to edit one independently. Shared input edits affect all
users, remain aligned to composition time, and undo together; locked users block
shared edits. Texture slots support color, normal, roughness, metalness,
emission, and ambient occlusion, with editable UV scale, offset, and rotation.

Agents can author or replace a material graph through `edit_3d update_object`.
Set `patch.material.shader` for the base material, or `patch.slots` for a model
with surface slots. Obtain the complete current slots from `get_3d_scene` before
replacing them. Preserve their IDs and any existing animation. A shader is:

```json
{
  "id":"product_paint",
  "name":"Product paint",
  "graph":{
    "nodes":[
      {"id":"noise","type":"ShaderNodeTexNoise","inputs":{"Scale":5,"Detail":2}},
      {"id":"bump","type":"ShaderNodeBump","inputs":{"Strength":0.2,"Distance":0.1}},
      {"id":"surface","type":"ShaderNodeBsdfPrincipled","inputs":{"Base Color":"#FF6B30","Roughness":0.3,"Metallic":0.15}},
      {"id":"output","type":"ShaderNodeOutputMaterial"}
    ],
    "links":[
      {"from":"noise","output":"Fac","to":"bump","input":"Height"},
      {"from":"bump","output":"Normal","to":"surface","input":"Normal"},
      {"from":"surface","output":"BSDF","to":"output","input":"Surface"}
    ]
  },
  "inputs":[
    {"id":"scale","label":"Pattern scale","kind":"number","node":"noise","socket":"Scale","min":0.01,"max":100},
    {"id":"color","label":"Color","kind":"color","node":"surface","socket":"Base Color"}
  ],
  "p":{"scale":{"v":5,"kf":[],"expr":null},"color":{"v":"#FF6B30","kf":[],"expr":null}}
}
```

Animate `shader.scale` or `slots.slot0.shader.scale` on the actual model layer.
Only whitelisted shader nodes and properties are accepted. The packaged
`blender-schema.ts` lists them and defines graph, exposed input, recipe, modifier,
and rendering contracts. Nodes include procedural textures, coordinate mapping,
math/vector operations, ramps, normal/bump, mixing, Principled/Glass/Emission,
and output. Image nodes reference durable image asset IDs and accept
`colorSpace:"sRGB"|"Non-Color"`. A Color Ramp node accepts
`ramp:{interpolation:"LINEAR",stops:[{position:0,color:"#112233"},{position:1,color:"#FFEEDD"}]}`.
Graphs are bounded, must have an output, and cannot contain cycles or executable
nodes. Full graph authoring is available to agents; manual editing uses presets,
exposed inputs, slots, and textures rather than a node-canvas editor or UV painter.

## Draft, rendered previews, and output

**Draft** uses native GPU geometry and approximate surface shading for responsive
playback and manipulation. **Rendered** uses EEVEE or Cycles for the current
paused frame, including actual shader graphs and evaluated procedural modifiers.
Paused rendered previews show one sharp frame; motion blur is applied during export.
Changes schedule a refreshed render. Orbiting continues to change only the
independent Editor viewpoint. Rendered output and agent captures use the actual
composition camera. Transparent backgrounds, per-layer masks/effects, group
transforms, depth, and shadows remain part of the composition pipeline.

The Rendering section selects **Native**, **EEVEE**, or **Cycles** for the
composition, with separate final/preview sample counts, preview resolution,
Cycles denoising, and CPU/GPU selection. Generated or imported Blender models
start with EEVEE output unless the composition already has render settings.
Choose Native for a composition that should use draft shading for delivery.
Blender lighting uses physical inverse-square falloff; point and spot ranges
limit influence. Native-only falloff controls stay in Native compositions.
Engine settings are saved with the project and undo normally. Render jobs can
be cancelled; a stopped worker restarts on the next request.

```json
{"operation":"set_rendering","settings":{"enabled":true,"engine":"cycles","samples":128,"previewSamples":16,"previewScale":0.5,"denoise":true,"device":"auto"}}
```

Video, still, and image-sequence export prepares evaluated Blender surfaces at
output resolution before compositing, including shutter samples for 3D motion
blur. Frame captures fail if a required Blender surface is unavailable rather
than delivering draft shading. Cached surfaces are cropped to their occupied
rectangles and kept under a memory budget. Large modifier combinations and mesh
outputs are bounded before allocation; reduce detail or copies if a recipe
exceeds its geometry budget.

Interactive **web/code** output runs in a browser without Blender and uses
native material approximations. Its export warnings call out this difference;
packed `.blend` sources are excluded from that delivery. Use video or image
frames for complete Blender shading. Project files retain the packed sources
and all editable model/material metadata for later editing.
