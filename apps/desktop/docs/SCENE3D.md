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
