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

Sun, point, spot and area lights support intensity, color, position, aim target, range/falloff, cone angle/softness and shadows. Area lights are rectangles of `width` × `height` world units facing their target; Blender renders their soft shadows, while Solid shading previews their light without shadows. The active camera holds projection, lens, target, clipping, and environment (ambient, ambientColor, exposure, shadows, optional background). Resource limits bound geometry, textures, objects, lights and shadow maps; simplify large models before import.

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

## The 3D viewport (manual editing)

When a composition contains 3D layers the composition viewer becomes a Blender
3D Viewport. Compositions without 3D layers show none of it, and Blender keys
apply only while the pointer is over the viewer, the composition has 3D layers
and no 2D layers are selected. Otherwise every key keeps its normal Powermove
(After Effects) meaning, and so does a Blender key that has nothing to act on.

- **Header:** Object Mode, View, Select, Add and Object menus; Transform
  Orientation (Global, Local, View); Pivot Point (Median Point, Individual
  Origins, 3D Cursor, Active Element, Bounding Box Center); Snap; Lock Camera to
  View; Overlays; Wireframe, Solid, Material Preview (EEVEE) and Rendered (the
  composition's engine) shading; Auto Keying.
- **Toolbar (T):** Tweak, Select Box (default; W cycles them), Cursor, Move,
  Rotate and Scale. Only the transform tools show a gizmo.
- **Sidebar (N):** the active item's Location, Rotation and Scale; the view's
  field of view, Lock Camera to View and the 3D cursor.
- **Navigation:** MMB orbits, Shift+MMB pans, Ctrl+MMB or the wheel zooms; Alt+LMB
  emulates the middle button. A trackpad orbits, Shift pans and Ctrl/pinch zooms.
  The navigation gizmo (axis ball plus zoom, pan, camera and projection buttons)
  sits in the top-right corner; click an axis to align the view. Number-row or
  numpad 1/3/7 (Ctrl for the opposite side) give orthographic Front/Right/Top,
  5 toggles perspective, 2/4/6/8 orbit by 15°, 9 flips, 0 toggles camera view,
  Numpad . frames the selection and Home frames everything. The top-left text
  names the view, e.g. "User Perspective" or "Camera Perspective".
- **Camera view** shows the composition camera. Its wheel/trackpad zoom and pan
  move the composition frame, as Blender zooms and pans the camera frame;
  orbiting leaves it for a user view. Ctrl+Alt+0 aligns the active camera to the
  view, Ctrl+0 makes the selected camera active, and with Lock Camera to View any
  navigation in camera view moves the real camera (keyed with Auto Keying, one
  Undo per gesture).
- **Selection:** click selects, Shift+click extends and sets the active item
  (shown in light orange; other selected items in orange). Drag box-selects
  (Shift adds, Ctrl subtracts). Lights and cameras are drawn as wires and are
  clickable. A, Alt+A, Ctrl+I; Select › All by Type.
- **Transform:** G, R and S (R R for trackball) follow the pointer until LMB,
  Enter or Space confirms, or RMB/Escape cancels. X/Y/Z constrain to an axis
  (again for the other orientation, a third time to clear), Shift+X/Y/Z to a
  plane, MMB picks an axis, digits type exact values (Tab for the next axis,
  minus toggles the sign), Ctrl snaps (1 unit, 5°, 0.1) and Shift gives
  precision. G/R/S switch mode mid-gesture. Alt+G/R/S clear location, rotation
  and scale. Each gesture is one Undo; animated channels and Auto Keying receive
  keyframes at the playhead.
- **Objects:** Shift+A adds at the 3D cursor; Shift+D duplicates and grabs; X or
  Delete deletes; H, Shift+H and Alt+H hide and reveal (the layer's visibility,
  as shown in the timeline); I and Alt+I insert and delete transform keyframes;
  Ctrl+P parents to the active item and Alt+P clears it, keeping world poses;
  F2 renames; F3 searches commands. Right-click opens the object context menu.
- **3D cursor:** Shift+right-click (or the Cursor tool) places it on the surface
  under the pointer; Shift+S opens the snap pie; Shift+C centres it and frames all.
- **Pies:** Z shading, ` view, . pivot point, comma orientation, Shift+S snap.
  Tap to open and click, or hold, point and release.
- **More navigation:** Shift+` starts Walk Navigation (mouse looks, WASD moves,
  Q/E down/up, Shift faster, wheel changes speed; LMB confirms, RMB/Esc returns;
  in camera view it moves the camera). Numpad / toggles Local View, showing only
  the selected models in the editor. Shift+B zooms to a dragged region,
  Shift+4/6 rolls the view, B box-selects from anywhere, and Alt+Z toggles X-ray
  so models are see-through in Solid and Wireframe. None of these affect output.
- **Edit Mode (Tab):** edits the active model's points. Click, Shift+click, drag
  (box) and A/Alt+A/Ctrl+I select points; G/R/S transform them with the same
  constraints and typed values; E extrudes the selected region (or edges) and
  moves it along its normal; M merges at the centre; X deletes. The first edit
  turns a primitive or profile into an indexed mesh source, which stops following
  a recipe's regeneration. Each edit is one Undo. Imported models and models with
  several material slots are edited in their source application.
- **Properties:** a selected 3D layer shows Blender's Properties tabs — Render,
  World (the active camera's environment), Object, Modifiers (generated models),
  Data (light or camera) and Material. The last chosen tab is remembered.

Viewport state (view, tools, overlays, shading, cursor) is editor state. It never
changes camera layers, keyframes or project data unless an operator says so,
and exports and agent frame captures always use the actual render camera.
Grouping models creates a native 3D group pivoting on its geometry; clicking a
member selects the group and double-click descends into it.

Extensions drive the same viewport through `api.scene3d.viewport`: `state()` (including `editMode`),
`onChange()`, `inContext()`, `operators` and `run(operator, ...args)` (for
example `run('view.front')`, `run('transform.translate')`, `run('tool','move')`,
`run('shading','material')`), plus `attach(stage, host)` for a viewer surface.
Extensions must use the public API instead of importing Three.js or modifying
editor internals.

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

## Viewport shading, rendered previews, and output

**Solid** (and Wireframe) shading uses native GPU geometry and approximate surface shading for responsive
playback and manipulation. **Material Preview** (EEVEE) and **Rendered** (the composition's engine) use
Blender for the current paused frame, including actual shader graphs and evaluated procedural modifiers.
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
