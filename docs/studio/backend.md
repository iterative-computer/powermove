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

Final preparation traverses the actual shutter samples. Async render requests
are serialized with scene/time patches, and completed images are copied into
GPU-backed capture canvases before a transport canvas can be reused. Those
images serve the synchronous compositor's readback, then are released on the
next frame's preparation or project close. Shutter samples share a backend
session; they do not create a native host session for each sample. Static
layers receive the full Final sample budget; blurred layers divide it across
their shutter samples, including descendants of a blurred group.

The three tiers use the same geometry/material/light model. Fast preview uses
Studio Live with fewer pixels/samples; Quality refines while idle. In-page
WebGPU remains the fallback for the native backend. Neither backend uses
three.js to draw the picture; three.js remains useful for import, geometry,
pose evaluation, picking and gizmo math.

## Prepared buffer layout

The material stride is **32 floats / 128 bytes** (8 `vec4`s). The first 24
floats retain the source branch's fields. Offsets 24–27 are ambient response,
flags, alpha-test threshold, reserved. Flags: Casts Shadows = 1, Accepts Shadows
= 2, Double Sided = 4, Unlit = 8, Vertex Colors = 16. Offsets 28–31 are independent
roughness, metalness, AO map indexes, then normal-map scale. Map index -1 means
absent. The legacy ORM index at offset 18 remains for older prepared scenes;
independent maps take precedence.

Triangles remain 24 floats / 96 bytes. If Vertex Colors is set, each normal's
unused `w` holds a **bit-cast packed RGBA8 color** (linear RGB). Copy these lanes
as bytes; do not JSON-encode their float interpretation (it may be NaN). The
canonical description represents these safely as optional `vertex_colors` in
`object_ids`, in mesh-vertex order.

Lights remain 16 floats. For point/spot lights `u.y` is finite range (0 means
unlimited) and `u.z` is the attenuation exponent (normally 2). Negative `u.w`
turns that light's shadows off; softbox area uses its absolute value. The
prepared instance stride remains 36 floats. Optional `instancePolicies`,
`materialPolicies` and `lightPolicies` preserve names/policies for JSON and
picking without changing those binary buffers. `Params.opts.w = 1` selects
orthographic camera rays; 0 selects perspective. Camera right/up extents contain
half the orthographic frame span in that mode.

Camera clipping is an optional extension of that same 208-byte Params block:
`opts.w` bit 31 enables it; bit 0 remains the orthographic flag. Bits 1–30
contain the positive near-plane f32 bits (the least significant mantissa bit
is cleared). With clipping enabled, `backdrop.w` holds the far plane: positive
with a shadow catcher, negative without one. Hosts leaving bit 31 clear retain
the original parameter meaning and output. The page's `StudioView.camera`
contains ordinary `near` and `far` numbers; backend implementations need no
bit packing outside their GPU upload. Primary and guide rays use clip planes;
reflections still see the complete scene.

Instance metadata also uses existing, previously unused `sdata` lanes; the
stride stays 36 floats. Within each instance, offset 19 is a color-present flag,
with linear RGB in offsets 27/31/35. Offset 23 hides that instance from primary
camera and guide rays only. Zero in those flags keeps the original behavior.
Shadow/reflection rays still see hidden-primary instances. This lets the host
apply per-layer opacity, masks and effects over the layers behind them without
losing their reflections or shadows. JSON carries the equivalent optional
`object_ids[].primary_visible` field. Ordinary opaque depth sets still share one
renderer job; layers requiring separate compositing use per-layer primary views.

Imported `EXT_mesh_gpu_instancing` becomes one mesh BVH with multiple transform
instances. Cloned geometry within a layer can also share its BVH when original
geometry, material assignment and pose match. The shared CPU cache is capped at
256 MiB. Scene data and the material table retain stable binary strides.

The macOS installer now registers the native implementation at boot. Native
Live receives the output size because the host owns tracing resolution and
MetalFX scaling; WebGPU Live receives half resolution. A finished native trace
is reused across per-layer image requests. Overlapping obsolete depth sets are
released immediately so incremental scene construction cannot consume the two
native session slots with abandoned views.

Captures track their own requested image keys. Viewer paints pause during async
preparation; unrelated cached jobs cannot keep a completed capture pending.
Nested composition renders inherit the parent Final flag. Synchronous readback
calls without preparation use Studio WebGPU; prepared captures consume retained
native frames. The diagnostics `Studio.status().backend` and `frameSamples`
report the completed image's backend and sampling count.
