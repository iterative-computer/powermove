# Studio renderer boundary

Public interface: `apps/desktop/src/renderer/src/core/scene3d/studio-backend.ts`.
Rust JSON: `pm_studio::scene::SceneDocument` in
`engine/crates/pm-studio/src/scene.rs`. Version 1. Vectors are `{x,y,z}`;
meshes are evaluated world-space positions, normals, UV pairs and triangle
index triples. `scene` is the Rust `Scene` serialized directly. The surrounding
fields retain layer/part identities, textures and material/camera/light policies
which the original standalone spike did not have.

A backend is created per composition/depth set. Register a native factory with
`registerStudioBackend(context => new NativeStudioBackend(context))`; passing
`null` restores in-page WebGPU. `context.invalidate()` requests another viewer
frame when an async result is ready. Registration increments a revision so
existing jobs are released on the next frame. Disposal must release all process
and GPU resources owned by that instance.

The optimized path is `setPreparedScene(scene, maps, description)`, followed by
`setView(view)`, `setTime(seconds)` and `render(request)`. `scene` and `view` are
the existing `core/studio/extract.ts::StudioScene` and
`core/studio/gpu.ts::StudioView`. `description()` lazily returns the canonical
Rust JSON model; the in-page backend does not need to copy large meshes into
JSON. The optimized path is optional: a backend implementing `setScene` and
`setCamera` alone receives the same description. Scene edits must reset sample
accumulation. Equal `geometryKey`s allow persistent geometry/BLAS reuse;
`sdata` carries changed instance transforms and lights. Texture sources have
the current frame's pixels; mutable canvases and arrays must be uploaded on
scene updates. Texture RGBA is straight alpha and row 0 corresponds to UV v=0.

`render` may return a frame immediately or a Promise. A frame's `source` is a
canvas/image that the existing compositor can upload. The requested layer is
masked from a trace of the complete depth set, preserving other objects in
reflections and occlusion. `background` assigns the backdrop/shadow catcher to
the bottom layer only. Final output waits for async results; the viewer keeps
the latest completed frame and invalidates when a newer one arrives. A backend
must not mutate a returned frame before the next request for that layer; call
`release` when a transported frame is no longer needed. `pick` and `bounds` are
optional because the host retains the main scene graph for gizmos and picking.

The three tiers use the same geometry/material/light model. Fast preview uses
Studio Live with fewer pixels/samples; Quality refines while idle. In-page
WebGPU remains the fallback for the native backend. Neither backend uses
three.js to draw the picture; three.js remains useful for import, geometry,
pose evaluation, picking and gizmo math.
