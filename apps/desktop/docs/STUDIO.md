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
- **View layout:** 1 View, 2 Views (side by side) or 4 Views, in the
  footer as in After Effects. Each pane chooses Active Camera, Front, Left,
  Top, Back, Right, Bottom or Custom View from its corner label; a side
  view opens framed on the scene. The selected pane is outlined and is the
  viewer itself: gizmos, picking, navigation and refine work there. The
  other panes are Studio Live pictures from their own sessions, redrawn
  round-robin (about 20 a second in total) only when what they show
  changes, so the selected pane keeps its frame rate. One click in another
  pane selects that pane and the layer under the pointer.

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
[`docs/studio/parity.md`](../../../docs/studio/parity.md). Substance SBSAR, FBX animation and simultaneous (2-up/4-up) views still
require implementation. Studio does not
claim Blender Cycles parity.

Imported glTF models using additional physical-material texture maps, texture
transforms or secondary UV sets still have an appearance-parity gap. The six
standard image slots exposed in Powermove's Materials UI are supported; the
extended import cases are listed separately in the checklist.

## Building environments

- **Add 3D Layer** (viewer or timeline context menu, or the palette) has
  **World** (choose a sky), **Terrain**, **Scatter** (choose what to copy,
  then what to cover; terrain first), **Fog Volume**, and **Ambient** and
  **Environment** lights.
- **World** (select the World layer, or the camera, whose panel shows the
  same World): sky thumbnails (Clear noon, Golden hour, Overcast, Blue hour,
  Night); **Environment** Sky, Image (an HDR/EXR or picture, a composition or
  a layer) or None; the sun on a small dome (drag it, or use the arrow keys;
  Shift moves 10°); Sun height, direction, strength, Haze, Clouds, Intensity,
  Rotation, Visible to camera; and **Atmosphere** height fog. With the World
  selected, dragging the sky in the viewer puts the sun under the pointer.
- **Terrain** arrives as rolling hills with grass, rock on steep slopes and
  light grass on top. In a composition without a sky or camera it also adds a
  golden-hour World and a camera that frames it, in one Undo. Shape buttons
  (Rolling hills, Mountains, Dunes, Nearly flat), **Shape from** (generated
  or a height image/layer/composition) and each ground cover's limits.
- **Scatter**: Copies of, Over, New variation, Count, Density, size and turn
  ranges, slope and height limits.
- **Material Options** (After Effects' wording) on models and on every 3D
  solid, footage, text, shape and pre-comp layer; extruded text and shapes
  show them with Front/Bevel/Side/Back.
- **Lens** (camera panel): Iris shape, rotation, roundness and aspect ratio.
- **Shots** (group of models): Exploded view with a spread, and Turntable.
- **Views**: the viewer's navigation menu looks from the Front, Left, Top,
  Back, Right or Bottom (Free View; the shot is unchanged).
- **Export ▸ Images ▸ PNG sequence ▸ Render passes**: Beauty, Depth, Normal,
  Albedo, Object ID, Material ID and Shadow as 16-bit PNGs.

Every value above has a keyframe diamond and a Timeline row with the same
name.

## Environment document and agent contract

A composition can own one **World** layer. Fog Volume, Terrain and Scatter are
also ordinary timeline layers. They use the existing `powermove.3d.object`
definition with, respectively, `object.world`, `object.fog`, `object.terrain` or
`object.scatter` data. The source plane is a serialization placeholder; World,
fog and scatter do not render that plane. The interface (below) uses
`api.scene3d.edit` with the same operations as agents.
World belongs to the composition's layer stack, independently of which camera
is active. Disabling/deleting its layer disables/removes that world. A dedicated
Environment light's world is used when there is no enabled World layer.

Examples for `edit_3d` (read `get_3d_scene` first):

```json
{"operation":"set_world","id":"world","world":{"mode":"sky","preset":"golden-hour","p":{"elevation":8,"azimuth":-70,"haze":2.5,"intensity":0.7,"sunIntensity":1,"visible":true,"metersPerUnit":1,"fogDensity":0.003,"fogColor":"#E6D4BE","fogHeightFalloff":0.15}}}
{"operation":"add_fog","id":"mist","fog":{"p":{"shape":"box","width":6,"height":3,"depth":5,"density":0.035,"color":"#E6D4BE","anisotropy":0.5}}}
{"operation":"update_object","target":"mist","patch":{"p":{"y":1.5}}}
{"operation":"update_fog","target":"mist","patch":{"p":{"density":{"v":0.02,"kf":[{"t":0,"v":0.02},{"t":2,"v":0.05}]}}}}
{"operation":"add_terrain","id":"ground","terrain":{"p":{"width":30,"depth":30,"resolution":64,"amplitude":3,"frequency":0.12,"octaves":5,"seed":41,"ridged":false},"regions":[{"p":{"minSlope":32,"color":"#8C7D64"}},{"p":{"minHeight":1.4,"color":"#A3A078"}},{"p":{"maxHeight":1.4,"color":"#687348"}}]}}
{"operation":"add_object","object":{"id":"tree","source":{"primitive":"cone","parameters":{"radius":0.4,"height":2,"segments":8}},"material":{"p":{"color":"#334A2E","roughness":0.85}}}}
{"operation":"add_scatter","id":"trees","scatter":{"model":"tree","surface":"ground","p":{"count":450,"density":1,"seed":71,"scaleMin":0.6,"scaleMax":1.4,"rotationMin":0,"rotationMax":360,"minSlope":0,"maxSlope":32,"minHeight":-2.5,"maxHeight":2.6}}}
{"operation":"update_scatter","target":"trees","patch":{"p":{"count":1000,"seed":42}}}
```

World modes: `sky`, `hdri`, `none`. Sky presets: `clear-noon`, `golden-hour`,
`overcast`, `blue-hour`, `night`. Selecting a preset sets its sun/haze defaults;
explicit `p` values in that edit take precedence. Azimuth and rotation are
degrees around +Y; elevation is degrees above the horizon. `haze` maps to
physical sky turbidity. `sunRadius` is the angular radius in degrees.
`visible:false` hides the environment from camera rays and retains its lighting
and reflections. `metersPerUnit` defaults to 0.1 for the desktop document's
existing product scale and controls physical lens conversion.

`set_world` updates the existing World, or creates one when absent. Its `p`
channels are `intensity`, `rotation`, `visible`, `metersPerUnit`, `elevation`,
`azimuth`, `haze`, `sunIntensity`, `sunRadius`, `overcast`, `fogDensity`,
`fogColor`, `fogAnisotropy`, `fogBaseHeight`, `fogHeightFalloff`, `fogDistance`,
`adaptive`, `minSamples`, `relativeError`, `absoluteError`. `none` disables the
sky/map while retaining atmosphere settings.

Import Radiance `.hdr` or OpenEXR `.exr` with `import_media`; set the returned
image asset ID as `world.source`. Linear HDR pixels are retained for lighting;
a separate SDR poster serves the media browser. Runtime maps are bounded to
2048×1024, 128 MB encoded input and 32 million decoded pixels. EXR data is
interpreted as linear sRGB/Rec.709; convert other working spaces before import.
A composition or a same-comp layer can also supply the environment:

```json
{"operation":"set_world","world":{"mode":"hdri","source":"IMPORTED_EXR_ASSET","p":{"intensity":1.5,"rotation":35,"visible":false}}}
{"operation":"set_world","patch":{"mode":"hdri","source":{"compositionId":"SKY_COMP"}}}
{"operation":"set_world","patch":{"mode":"hdri","source":{"layerId":"SKY_VIDEO_LAYER"}}}
{"operation":"update_terrain","target":"ground","patch":{"heightSource":{"layerId":"GRAYSCALE_LAYER"}}}
{"operation":"add_light","light":{"type":"ambient","p":{"intensity":0.2,"color":"#DCE5FF"}}}
{"operation":"add_light","light":{"type":"environment","world":{"mode":"hdri","source":"IMPORTED_HDR_ASSET"},"p":{"intensity":1}}}
```

Composition/layer sources use the existing live-material source raster:
hidden sources work, masks/effects are included, source transforms and opacity
are excluded, and video/precomp timing is evaluated at the receiver's time.
Reference cycles are rejected before edits commit. HDRI sources use linear HDR
values; ordinary image, composition and layer sources are decoded from sRGB.

Terrain can use procedural noise, `heightSource` (image asset/layer/comp), or
normalized `heights` with exactly `(resolution+1)^2` entries. Image luminance
becomes height multiplied by amplitude. Resolution is 2–256; octaves 1–12.
Material regions are evaluated in order, first matching slope/height wins;
otherwise the terrain's ordinary material is used. Slopes are degrees from up.
There are at most eight regions. Their color, roughness, metalness and limits
are channels, including `terrain.region0.color` and `terrain.region0.minSlope`.

Scatter retains references to a model and surface/terrain layer. It samples
surface triangles by area after parent/anchor transforms, reuses prototype
geometry, and assigns all copies to the Scatter layer for compositing and
picking. Seeded edits repeat exactly. Count is the number of candidate placements;
density (0–1) and slope/height limits can reject candidates. Scale/rotation
jitter uses min/max pairs. A source model's geometry, material and scale are
retained; its position is replaced by each placement. Changing source geometry,
materials or surface animation rebuilds placement data. Maximum 100,000
candidates per Scatter and one million across a composition. Model/surface
sources cannot themselves be scatter/fog/World layers. Hidden prototype/surface layers remain usable. Missing/deleted sources
stay serialized and render no copies until restored.

Fog volumes support box and sphere shapes. Width/height/depth are full local
dimensions; sphere diameter is width. Position/scale use normal layer channels.
The current transport contract has axis-aligned boxes: rotating a box uses its
world bounding box. Use overlapping smaller volumes for angled shafts. There
are at most 32 volumes. Density uses inverse scene units; anisotropy is −0.95
to 0.95. Positive anisotropy scatters forward. The World supports global height
fog through its fog fields.

All evaluated settings use normal timeline channels: `world.KEY`, `fog.KEY`,
`terrain.KEY`, `terrain.regionN.KEY`, `scatter.KEY`. Object transforms remain
`position.x/y/z`, `rotation.x/y/rotation`, `scale.x/y/z`. Fields accept a plain
value or `{v,kf,expr}`. `set_world` and the update operations route channel
changes through the normal autokey, Undo, layer lock, hand-edit protection and
revision guard. Source references, mode and preset selection are structural
edits. `get_3d_scene` returns evaluated World/fog/terrain/scatter collections,
raw layer content, channel paths and the revision baseline. No project JSON
editing is required.

## AE material, light and camera data

Light types are `parallel` (alias `sun`), `spot`, `point`, `ambient`,
`environment`, and Studio's `area` softbox. Ambient layers add uniform diffuse
illumination, without a direction or shadows, independently of the world map.
Environment layers use the World source contract and their light intensity
multiplies its illumination.

```json
{"operation":"set_material_options","target":"MODEL_OR_3D_ARTWORK","patch":{"castsShadows":"only","acceptsShadows":true,"acceptsLights":true,"appearsInReflections":false}}
{"operation":"set_camera","patch":{"p":{"fStop":1.4,"focusDistance":4,"irisShape":6,"irisRotation":30,"irisRoundness":0.25,"irisAspect":1.4}}}
```

Material Options are channels under `materialOptions.KEY`: casts `off/on/only`,
accepts shadows, accepts lights and appears in reflections. Shadow-only layers
remain in shadow/reflection transport but disappear from primary camera rays.
Turning off lights gives an unlit base-color material. Turning off reflections
excludes the layer from secondary camera paths (including transmission and
indirect paths in this implementation), independently of its shadow flag.
Old castShadow/receiveShadow documents retain their values until options are
installed. Ordinary solids/images/video/text/shapes/precomps with 3D enabled
share Studio's surface/shadow transport when Material Options are installed
(or in an extrusion composition). This explicit opt-in preserves older flat
artwork/group behavior. Their alpha texture also controls their shadow silhouette. Explicitly accepting
lights allows their textures to receive shaded lighting and shadows.

Iris shape is 0 for circular or 3–16 for a polygon's blade count. Rotation is
degrees, roundness 0–1 blends toward a circle, and aspect stretches horizontally
(0.1–10). These are `camera.irisShape/irisRotation/irisRoundness/irisAspect`
channels. The same uniformly sampled aperture is used by native and WebGPU
Final/Refine; Live keeps depth of field disabled while moving. Diffraction
fringes are not implemented.

## Render-pass image sequences

Agents can call `export_video` with `format:"png"` and `passes`:

```json
{"format":"png","passes":["beauty","depth","normal","albedo","object_id","material_id","shadow"],"range":"work","audio":false,"motionBlur":false}
```

Programmatic callers can use `PM.Export.run({format:'png',passes:[...],...})`.
`PM.Export.renderPasses(time,width,height,passes)` is an async iterator of
`{pass,blob,metadata}` for hosts that own delivery. In the Export dialog,
Images ▸ PNG sequence shows **Render passes** checkboxes for compositions with
3D layers. Authenticated agents receive a unique output folder without a dialog;
normal interactive exports retain the existing destination chooser.

All pass files are genuine **RGBA16 PNG**, from floating GPU buffers, with no
RGBA8 intermediate. Beauty is display-referred sRGB after Final path tracing
and guide denoising; data passes have no sRGB tag. Auxiliary passes use the
shared portable WGSL on WebGPU even when the viewer uses native Metal. EXR
output is not supported by the desktop delivery pipeline yet.

- Depth: normalized `(distance-near)/(far-near)`; background alpha is zero.
- Normal: world XYZ mapped from −1..1 to 0..1.
- Albedo: linear sRGB, without lighting.
- Object/material ID: red channel stores the exact integer ID in 16-bit form;
  zero is the background. IDs start at one.
- Shadow: fraction of direct analytic illumination occluded (0 lit, 1 blocked),
  using 32 light samples; this does not include indirect lighting/caustics.

Each frame has `passes_00000.json` metadata with depth encoding and the current
object-ID mapping. IDs reflect that frame's prepared geometry/material order;
metadata is written per frame because animation/visibility can change it.
Pass export is the evaluated 3D world; ordinary 2D overlays, per-layer compositor
effects and motion blur are not applied to the data passes. Beauty uses the
same 3D renderer without those later compositor operations. Data passes use a
single center ray (with the camera aperture) and an opacity threshold of 0.5.
