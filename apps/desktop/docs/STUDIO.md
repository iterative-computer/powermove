# Studio: the 3D renderer

Studio draws every 3D layer in the viewer, in exports and in agent frames. It
is a path tracer: light bounces between surfaces the way it does in a real
studio, so reflections, soft shadows, glass and glowing screens look
physically right. The design and roadmap are in `docs/3d-studio-redesign.md`.

## What you see

The viewer's toolbar, for a composition with 3D layers, left to right:

- **Camera / Free view:** look through the shot's camera, or around the scene
  without changing the shot.
- **Tools:** Move, Rotate and Scale the selection; Orbit, Pan and Zoom the
  view; Light (below). Each says in its tooltip what a drag does. Move,
  Rotate and Scale add a **Scene axes / Object axes** switch: whether the
  handles line up with the scene or with the object, however it is turned.
- **Frame:** fit the selection, or everything when nothing is selected
  (F, Home).
- **Fast / Quality:** both use Studio. Fast keeps the low-resolution Live
  preview; Quality refines the image when you pause. Exports and agent
  frames use Final quality. The choice is remembered. These controls are
  in the viewer footer; navigation tools remain in the header.

Studio has three rendering tiers:

- **Live** while you drag, scrub or play: half resolution, one sample per
  pixel, denoised. Depth of field is off while things move.
- **Refine** when nothing has changed for a moment: the view sharpens a few
  samples per frame until it is clean. The pill in the viewer's corner shows
  the progress ("Refining · 62%", then "Clean").
- **Final** for exports and agent captures: a fixed number of samples per
  frame, set per composition on the camera (Final quality: Draft 16,
  Standard 64, High 256, Best 1024). Motion blur divides those samples
  across its shutter samples, so blurred exports cost no more. Each frame is
  seeded by its time: re-rendering a frame gives the same pixels, and each
  frame (and each shutter sample) gets its own noise, so grain never sits on
  the screen while the picture moves under it. Final and stills are then
  denoised (`denoise.wgsl`): an edge-avoiding filter guided by the surfaces'
  albedo, normals, distance and coverage (`aov.wgsl`) and by each pixel's own
  measured noise, which smooths light, screens, shallow-focus blur and the
  floor's shadows while edges, paint and the backdrop stay exact.

Perspective and orthographic cameras both use Studio, including camera zoom
and near/far clipping. The macOS app uses a native Metal renderer through
[`StudioBackend`](../../../docs/studio/backend.md). Other platforms use WebGPU. Native initialization or rendering failures fall back to WebGPU.
There is no three.js picture-rendering mode.

## Building a product shot

- **Import a model into an empty composition** and it arrives in a finished
  studio: resting on the floor, framed by a camera, lit by key, fill and rim
  softboxes, on the seamless set. One Undo removes all of it.
- **Set up studio** (camera inspector) does the same for any composition.
- **Studio set** (camera inspector) turns the background into a seamless
  backdrop (`#F6F5F8` unless a background color is set) with a floor at the
  lowest model that shows only the shadows models cast on it.
- **Finishes** (model inspector): anodized aluminium, brushed metal,
  polished steel, gold, clear and frosted glass, gloss lacquer, soft-touch
  plastic, glazed ceramic, matte rubber and a display under glass. Each is a
  starting point; the material fields refine it.
- **Materials** (model inspector): select the whole model or a named source
  material, then choose an image, composition or same-composition layer as
  its Color or Emission source. Sources update at the frame's time. Emission
  intensity controls screen brightness. Existing Studio screen links still
  render, and the `screen` field remains available to agents.
- **Drop to floor** (model inspector) rests a model on the floor.

## Large models and CAD

**STEP, IGES and BREP** files import directly: OpenCascade tessellates them
in a worker (fine enough for close-ups) into a GLB that keeps the
assembly's parts, names and colors, so each part can take its own finish.
A typical 18-part assembly takes under half a second.

CAD assemblies import as GLB (up to 512 MB, 10 million triangles in up to
65,536 parts) or OBJ (up to 10 million triangles). Studio prepares large
geometry in a worker; the corner pill reports preparation while controls stay
responsive. Exports and agent frames wait for preparation before reading back
pixels. Repeated geometry shares its BVH, including imported glTF instances;
transform changes rebuild the small instance hierarchy rather than geometry.
The maximum usable scene also depends on the GPU's buffer and memory limits.

## Gaussian splats

Captured scenes and products (3D Gaussian Splatting `.ply`, or `.splat`)
import like any model and play back in real time: each splat is drawn as a
soft ellipse, sorted far to near whenever the view changes (about 5 ms for
a million splats). Plain colored point clouds (`.ply` with red/green/blue)
become small round splats. Captures are y-down by convention and arrive
turned upright, framed on their dense core (stray far-away splats are
ignored for framing).

Splats are not path traced: a splat layer is rasterized and composited with
Studio's layers by layer order. Keep a splat layer **above** the product
layers it surrounds; it is depth-tested against their geometry, so the
product hides the splats behind it and the splats in front cover it.

## The light tool

Put light where you point (after Light Wrangler for Blender). Choose
**Light** in the toolbar; its panel opens under it.

- **Reflect:** click a model and the selected light moves so its highlight
  lands exactly there (Studio mirrors the camera's view about the surface).
- **Direct:** the light faces the spot you click, lighting it straight on.
- **Orbit:** drag to circle the light around what it points at.
- **Add** (with Softbox, Spotlight, Bulb or Sun): the next click adds that
  light there. With no light selected, a click adds one too.
- Keep dragging to slide the light; release commits the move as one Undo.
- The panel's **Brightness, Size, Distance** and a spotlight's **Spread**
  sliders move the light live; releasing is one Undo. Distance keeps the
  light on the product as bright (power follows the square of the distance).
- **Esc** leaves the tool. Alt+Shift+click still puts a softbox's highlight
  under the pointer from any tool.

## Shots

Shot buttons write ordinary keyframes you can edit in the timeline.
Turntable is in the model inspector; camera moves are in the camera inspector:

| Shot | Moves |
|---|---|
| Turntable | the selected model or group turns 360° |
| Push-in | the camera moves 30% closer to its aim |
| Orbit reveal | the camera orbits 90° around its aim |
| Hero tilt | the camera rises into a low hero angle |
| Rack focus | the focus distance moves from near to far |

## Materials

Beyond color, roughness, metalness, emission and opacity, materials have:

| Field | Effect |
|---|---|
| Clear coat, coat roughness | a lacquer or cover-glass layer over the base |
| Transmission, index of refraction | glass and liquids |
| Specular | dielectric reflection strength (0 for a matte panel under glass) |
| Sheen | the soft rim of fabric and soft-touch finishes |
| Thin film (nm) | iridescent coatings |
| Dispersion | glass splitting light into colored fringes |
| Water droplets | beads on the surface that bend reflections and the screen |
| Brushed | highlights stretched along the surface (anisotropy, −1 to 1) |
| Subsurface | how far light travels inside (wax, soap, skin, jade), in scene units |

Glass and polished metal also **focus light**: a glass on the studio floor
casts a bright pool inside its shadow (caustics), in Refine and Final.

Lights: sun, point, spot and **softbox** (area: width × height facing its
target; larger is softer). Cameras have an **f-stop** (0 keeps everything
sharp) and **focus** distance (0 focuses on the aim point).

## For agents

`edit_3d` has Studio's operations alongside the ordinary ones:

```json
{"operation":"apply_material","target":"body","preset":"anodized","p":{"color":"#3B3C40"}}
{"operation":"light_highlight","at":[0.3,0.07,-0.4],"normal":[0,1,0],"intensity":4}
{"operation":"shot","shot":"turntable","target":"product_group","start":0,"duration":4}
{"operation":"set_studio","on":true}
{"operation":"auto_studio"}
{"operation":"explode","target":"product_group","amount":0.5,"duration":2}
{"operation":"drop_to_floor","target":"body"}
{"operation":"place_on","target":"cap","on_target":"bottle"}
{"operation":"soften","target":"softbox_layer","amount":1.5}
{"operation":"update_object","target":"display","patch":{"material":{"screen":"COMPOSITION_ID"}}}
```

`roundedBox` (width, height, depth, radius) is the primitive for device
shells. Verify with `render_frames`: agent frames always use the Final tier.

## How it works

- The tracer (`engine/crates/pm-studio/src/shaders/trace.wgsl`) is shared
  by the Rust engine and the app, which runs it on WebGPU
  (`src/renderer/src/core/studio`).
- Each mesh keeps a BVH in its own space; a small top-level BVH over the
  models' world bounds is rebuilt when anything moves, so animating a large
  model costs almost nothing.
- Opaque depth sets share one trace. Layers with opacity, masks or effects
  receive separate primary-camera views so the compositor can reveal layers
  behind them. All geometry remains available to reflections and shadows.
- Backend frames can arrive asynchronously. Final capture waits for scene
  preparation and transport readiness; closing a project releases sessions
  and late frames. See [`docs/studio/backend.md`](../../../docs/studio/backend.md)
  for the shared Rust/TypeScript scene and binary layout.
- Live screens render their composition at each frame's time and update
  only their texture.
- CAD import uses [occt-import-js](https://github.com/kovacsv/occt-import-js)
  (OpenCascade compiled to WebAssembly, LGPL-2.1; licenses ship in the app's
  `Resources/cad`). It loads only when a CAD file is imported and runs in a
  worker that may compile WebAssembly and evaluate its own bindings but has
  no network access (`src/main/security-policy.ts`).
- Caustics (light focused through glass or off polished metal onto the
  floor or a matte surface) come from photons traced from the lights before
  each batch of samples. Refine and Final only, and only in scenes with glass
  or polished metal.

## Compatibility and remaining targets

Main's layer document, timeline, material sources, parts, Front/Bevel/Side/Back
surfaces, native controls and agent operation shapes remain the editing API.
The checklist and verified limitations are in
[`docs/studio/parity.md`](../../../docs/studio/parity.md). HDRI lighting,
Substance SBSAR, FBX animation, camera iris controls, simultaneous views and
user-exportable render passes still require implementation. Studio does not
claim Blender Cycles parity.

Imported glTF models using additional physical-material texture maps, texture
transforms or secondary UV sets still have an appearance-parity gap. The six
standard image slots exposed in Powermove's Materials UI are supported; the
extended import cases are listed separately in the checklist.
