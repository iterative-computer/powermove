# Studio: the 3D renderer

Studio draws every 3D layer in the viewer, in exports and in agent frames. It
is a path tracer: light bounces between surfaces the way it does in a real
studio, so reflections, soft shadows, glass and glowing screens look
physically right. The design and roadmap are in `docs/3d-studio-redesign.md`.

## What you see

- **Live** while you drag, scrub or play: half resolution, one sample per
  pixel, denoised. Depth of field is off while things move.
- **Refine** when nothing has changed for a moment: the view sharpens a few
  samples per frame until it is clean. The pill in the viewer's corner shows
  the progress ("Refining · 62%", then "Clean").
- **Final** for exports and agent captures: a fixed number of samples per
  frame, set per composition on the camera (Final quality: Draft 16,
  Standard 64, High 256, Best 1024). Motion blur divides those samples
  across its shutter samples, so blurred exports cost no more.

Machines without WebGPU, and orthographic cameras, use the classic renderer.

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
- **Screen** (model inspector): choose a composition to play on the surface
  as a live screen, like a device's display. Its UVs span the composition;
  the emission strength sets its brightness.
- **Drop to floor** (model inspector) rests a model on the floor.

## Light by pointing

**Alt+Shift+click** a model in the viewer to put a softbox's highlight
exactly there: Studio mirrors the camera's view about the surface and places
the softbox where its reflection lands. **Keep dragging** to slide the
highlight across the surface; release commits it as one Undo.
**Alt+Shift+scroll** over the viewer makes the selected softbox larger
(softer highlight) or smaller (crisper).

## Shots

The camera inspector's Shot buttons write ordinary keyframes you can edit in
the timeline:

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
- One trace per depth set serves every 3D layer: each layer receives only its
  own pixels, so masks, effects and opacity stay per layer.
- Live screens render their composition at each frame's time and update
  only their texture.
- Caustics (light focused through glass or off polished metal onto the
  floor or a matte surface) come from photons traced from the lights before
  each batch of samples. Refine and Final only, and only in scenes with glass
  or polished metal.
