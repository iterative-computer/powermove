# Studio: the 3D renderer

Studio draws every 3D layer in the viewer, in exports and in agent frames. It
is a path tracer: light bounces between surfaces the way it does in a real
studio, so reflections, soft shadows, glass and glowing screens look
physically right. The design and roadmap are in `docs/3d-studio-redesign.md`.

## User guide

This section is for people making 3D motion graphics in Powermove. Names in
**bold** are the exact words in the app. If you know After Effects, most
things are where you expect them; the differences are called out.

### Adding 3D

- **An empty composition** offers **3D Layer** beside Text, Shape, Solid and
  Null in the Timeline.
- **Right-click** the composition or an empty part of the Timeline and choose
  **Add 3D Layer**, or search the command palette for it.

The **Add 3D Layer** menu has three groups:

| Group | Items |
|---|---|
| **Shapes** | Cube, Sphere, Plane, Cylinder, Cone, Torus, Capsule, Icosphere, **Import 3D Model…** |
| **Environment** | **World** (pick a sky), **Terrain**, **Scatter**, **Fog Volume** |
| **Lights and Camera** | **Sun Light**, **Point Light**, **Spot Light**, **Softbox Light**, **Ambient Light**, **Environment Light**, **Camera** |

Every 3D thing is an ordinary layer: it has a Timeline row, keyframes,
parenting (**Link Layers**), Undo, and the same icon in the menu, the Timeline
and the Properties header. **Scatter** stays disabled until there is a model to
copy and a Terrain to cover; the menu says which one is missing.

After Effects equivalents: **Sun Light** is a Parallel light, **Softbox Light**
is an area light (AE has none), and **World** combines AE's environment light
with a sky and sun.

### Importing a model

**Import 3D Model…** (or drop a file on the composition) accepts GLB,
self-contained glTF, OBJ (select the OBJ together with its MTL and images),
FBX, STEP/STP, IGES/IGS, BREP and Gaussian splats (PLY, .splat).

- The **first model in an empty composition arrives in a finished studio**:
  resting on the floor, framed by a camera, lit by softboxes, on a seamless
  set. One Undo removes all of it.
- **From File** (model properties) keeps the materials stored in the file;
  turn it off to give the whole model one editable material.
- A file that can't be read says so in plain words, for example "Couldn’t
  import Broken.glb. The file isn’t a readable GLB model — it may be damaged
  or incomplete." Nothing is added, and nothing else changes.

See [Large models and CAD](#large-models-and-cad) for sizes and limits.

### 3D text and shapes

Select a text or shape layer. **Geometry Options** ▸ **Extrude** makes it real
3D geometry (it also turns on the layer's 3D switch, in one Undo). Then:

- **Extrusion Depth**, **Bevel Style** (None, Angular, Concave, Convex),
  **Bevel Depth**, **Hole Bevel Depth** and **Geometry Quality**, all
  animatable like AE's Geometry Options.
- **Material Options** lists **Front**, **Bevel**, **Side** and **Back**. Each
  has its own material, and **Use Front for All** copies Front to the rest.

Extruded text needs the font's outlines, so **the font must be installed on
this Mac**. If it isn't, Geometry Options says "“Font name” isn’t installed
on this Mac, so this text can’t be extruded" — choose an installed font, or
install it and reopen the project. (Flat 2D text falls back to a similar
font; geometry can't.)

### Materials and finishes

Select a model. **Material** shows a shelf of **finishes** (hover for each
name): Anodized aluminium, Brushed metal, Polished steel, Gold, Clear glass,
Frosted glass, Gloss lacquer, Soft-touch plastic, Glazed ceramic, Matte rubber
and Display under glass. A finish is a starting point; the fields below refine
it: **Color**, **Roughness**, **Metalness**, **Emission**, **Emission
Strength**, **Opacity**, **Clear Coat**, **Coat Roughness**, **Transmission**,
**Index of Refraction**, **Specular**, **Sheen**, **Thin Film**,
**Dispersion**, **Water Droplets**, **Brushed** and **Subsurface**.

**Image Maps** take a **Color Map**, **Normal Map**, **Roughness Map**,
**Metalness Map**, **Emissive Map** and **Occlusion Map**. A material's
**Base Color** or **Emission** can also be a composition or a layer —
including a group of 2D artwork — so a screen can play your animation (see
[Artwork as a material](#artwork-as-a-material)). If a source is deleted, the
picker says **Missing image**, **Missing comp** or **Missing layer** and the
model shows its original material until you choose another.

**Material Options** use After Effects' names: **Casts Shadows**, **Accepts
Shadows**, **Accepts Lights** and **Appears in Reflections**.

### Lights

| Light | Use | Main properties |
|---|---|---|
| **Sun Light** | Parallel light from far away | Intensity, Color, Casts Shadows |
| **Point Light** | A bare bulb | Intensity, Color, Falloff Distance, Falloff, Casts Shadows |
| **Spot Light** | A cone of light | as Point, plus Cone Angle, Cone Feather |
| **Softbox Light** | Soft studio light and reflections | Intensity, Color, Width, Height, Casts Shadows |
| **Ambient Light** | Even light from everywhere | Intensity, Color |
| **Environment Light** | Light from a sky or image all around | Intensity, Color, Environment |

**Ambient Light**, **Environment Light** and **World** light the whole scene
from everywhere, so they have no position: no Transform, no gizmo, no **Link
Layers**, and the viewer's tool strip says "Lights the whole scene · no
position" instead of offering Move, Rotate and Scale.

**Light by pointing.** Choose **Light** in the viewer's tool strip. The panel
picks what to add (**Softbox**, **Spot**, **Point**, **Sun**) and how it aims:
**Reflect** (the highlight lands where you click), **Direct** (the light faces
where you click) or **Orbit** (drag to circle the light around its target).
Click a model to place the light; drag to slide the highlight. Each placement
is one Undo. See [The light tool](#the-light-tool).

### Camera and depth of field

Add a **Camera** to set the shot. Its properties, in AE's words:
**Projection**, **Angle of View** (perspective) or **Zoom** (orthographic),
**Near Clip**, **Far Clip**, **F-Stop** and
**Focus Distance**. **Lens** shapes the out-of-focus highlights: **Iris
Shape** (Round or 3–10 blades), **Iris Rotation**, **Iris Roundness** and
**Iris Aspect Ratio**. **Render Quality** on the camera names the
composition's quality; click it to change it (see below).

The camera also offers **Set Up Studio** (frame, light and set the scene in
one Undo), **Studio Set** (a seamless backdrop with a floor that only shows
shadows) and **Background**.

### World, sky and sun

**Add 3D Layer ▸ World** and choose a sky: **Clear noon**, **Golden hour**,
**Overcast**, **Blue hour** or **Night**. Select the World layer to change it:

- Drag the sun on the **Sun position** dome, or type **Sun Height** and **Sun
  Direction**; **Sun Strength** sets its brightness.
- **Haze**, **Clouds**, **Ground Color** and the **Fog** settings (**Fog
  Color**, **Fog Falloff**, **Fog Base**) shape the air.
- **Environment** chooses what lights the scene: the generated **Sky**, or an
  image (an HDRI panorama). **Visible to Camera** shows or hides it as the
  background.

A composition has one World. With a World, **Add 3D Layer ▸ World** selects it.

### Terrain, Scatter and Fog Volume

1. **Terrain** adds ground. Choose its shape in the shape grid, or **Shape
   From** a height image instead of **Generated** noise; **Width**, **Depth**,
   **Height**, **Feature Size**, **Detail** and **Ridges** shape it, and
   regions color it by slope (**Steeper Than**, **Flatter Than**) or height.
2. **Scatter ▸ Copies of** a model **▸ Over** a surface covers the terrain
   with copies. The model being copied is hidden in the same Undo so only the
   copies show. **Count**, **Smallest**/**Largest**, **Turn From**/**Turn to**,
   **Flatter Than**/**Steeper Than** control placement; **New Variation**
   reshuffles it.
3. **Fog Volume** adds a box or sphere of fog: **Shape**, **Width**,
   **Height**, **Depth**, **Density**, **Color** and **Forward Glow** (how
   much it glows when you look toward a light).

### The viewer

- **Move**, **Rotate**, **Scale** and **Light** (header), with **Global** or
  **Local** axes. Ctrl snaps; Shift is fine adjustment; Escape cancels. Each
  drag is one Undo.
- **Fast** / **Quality** (footer): Fast keeps the Live picture; Quality
  refines when you pause.
- **Active Camera** / **Free View**: look through the shot, or look around
  without changing it. Middle-drag or Option-drag orbits; Shift adds pan.
- **View layout**: **1 View**, **2 Views** or **4 Views**, as in AE. Each
  pane's corner label picks Active Camera, Front, Left, Top, Back, Right,
  Bottom or Custom View. The outlined pane is the one you work in; click
  another pane to work there. Side panes say what they show in their corner:
  **Rendering…**, **Updating…** (after an edit), **Live**, or **Can’t draw**.
  Side panes stay at Live quality; only the selected view refines.
- The **3D navigation** menu: Orbit, Pan, Dolly, Frame Selection (F), Frame
  All (Home), and look from Front, Left, Top, Back, Right or Bottom.

### Live, Refine and Final

Studio draws 3D in three tiers:

- **Live** while you drag, scrub or play: fast and slightly soft.
- **Refine** when you pause: the picture sharpens toward the composition's
  **Render Quality**.
- **Final** for exports and agent frames: the full Render Quality (or the
  export's override).

The pill in the viewer's corner says where it is: **Preparing 1.2M
triangles** (a big model getting ready), **Live**, **Refining · 62%**, then
**Refined · Production** (the quality it reached). Hover it for details.

### Render Quality

With nothing selected, a composition that has 3D shows **Render Quality** in
its properties. Pick a preset; the line below it says what it is for and how
long a frame can take on this Mac.

| Preset | For | Time per frame, at most |
|---|---|---|
| **Draft** | Blocking out motion and timing. Fast and grainy. | about 0.5 s |
| **Preview** | Reviews and quick drafts to share. | about 2 s |
| **Production** | Delivery; clean, with light focused through glass. The default. | about 3 s |
| **Final** | Hero shots and stills. The cleanest and slowest. | about 20 s |
| **Custom** | Your own settings, starting from the current preset. | your **Time Limit** |

Times are upper limits: simple scenes finish sooner, and denoising adds a
little. **Custom** reveals the settings:

| Setting | What it does |
|---|---|
| **Max Samples** | The most light paths traced per pixel in each frame. |
| **Min Samples** | Paths traced before Studio checks whether a pixel is clean enough to stop. |
| **Noise Limit** | Stop a pixel once its noise is below this. Lower is cleaner and slower; 0 always traces Max Samples. |
| **Time Limit** | The longest Studio traces one frame. 0 means no limit. |
| **Max Bounces** | How many times light can bounce. More shows deeper glass and interiors. |
| **Caustics** | Light focused through glass and off mirrors onto other surfaces. |
| **Denoise** | Remove the remaining grain after tracing. |
| **Blur Samples** | Moments in the shutter blended for 3D motion blur. 1 turns 3D motion blur off. |

Render Quality is saved with each composition; every change is one Undo.
The full numbers are in [Render quality reference](#render-quality-reference).

### Shots, turntables and exploded views

- **Shots** (model properties) animate the camera in one Undo:
  **Turntable**, **Push-in**, **Orbit reveal**, **Hero tilt** and **Rack
  focus**. **Turntable** and **Drop to Floor** are also one-click buttons.
- A **group of models** gets **Exploded View** (with **Spread**: A little,
  Medium or Far apart) and **Turntable** in its properties.
- **Parts** (imported models with named parts) lets you move or animate
  each part on its own; **Whole Model** selects the model again and **Reset**
  returns a part to where it was.

### Exporting

**Export** works as for 2D, plus:

- **3D Render Quality** (under Rendering): **Composition · Production** uses
  the composition's saved quality; choose a preset or **Custom…** to override
  it for this export only (it is never saved to the project). The line below
  estimates the whole export, for example "Up to about 15 min for 300 frames".
- **Render Passes**: choose **Images**, then **PNG sequence**, and tick
  **Beauty**, **Depth**, **Normal**, **Albedo**, **Object ID**, **Material
  ID** and **Shadow** for 16-bit PNG images per pass. With a video format the
  row stays visible but unavailable, and says how to get passes.

### When something goes wrong

| You see | What it means | What to do |
|---|---|---|
| "Couldn’t import *file*. The file isn’t a readable … model" | The file is damaged, incomplete or not that format. | Re-export it from the source app; for glTF, pack it into a single GLB. |
| "“*Font*” isn’t installed on this Mac, so this text can’t be extruded" | Extruded text needs the font's outlines. | Choose an installed font, or install it and reopen. |
| **Missing image** / **Missing comp** / **Missing layer** in a material | The material's source was deleted. | Choose another source, or Original. |
| **3D can’t render** in the viewer's corner | Studio can't draw this scene; hover for why. Usually the scene is too large for the GPU. | Simplify models, lower the Scatter count, or export at a smaller size. Undo the last change if it just happened. |
| "The 3D renderer restarted on its backup" | The Mac's native renderer stopped; Studio carried on with its backup. Your work is safe. | Keep working; reopen the project to use the native renderer again. |
| **Can’t draw** on a side view | That view failed; the selected view and your export are unaffected. | Switch that pane to another view, or back to 1 View. |

Each 3D action is named in Undo history (for example "Add Terrain", "Change
World", "Render Quality"), including changes made by agents.

## Render quality reference


| Preset | Noise threshold | Min / max samples per frame | Tracing limit | Bounces | Caustics | Denoise | Shutter samples |
|---|---:|---:|---:|---:|---|---|---:|
| Draft | .12 | 4 / 16 | .5 s | 4 | Off | On, fast GPU filter | 1 |
| Preview | .06 | 16 / 64 | 2 s | 6 | Off | On, fast OIDN on native | 2 |
| Production | .03 | 32 / 256 | 3 s | 8 | On | On, balanced OIDN on native | 2 |
| Final | .015 | 512 / 1024 | 20 s | 8 | On | On, high OIDN on native | 2 |

**Custom** exposes `noiseThreshold` (0–1), `minSamples` / `maxSamples`
(1–4096, min must not exceed max), `timeLimitSeconds` (0–3600),
`maxBounces` (1–32), `caustics`, `denoise`, and `motionBlurSamples` (1–64).
Zero noise threshold disables error stopping; zero seconds removes the tracing
cap. The minimum is the noise test's starting point. The time cap can finish
before it; a running GPU batch and filtering/readback can extend wall time.
All sample and tracing budgets are for the output frame, divided across its
shutter times within each Studio depth set. Styled boundaries and nested Studio
compositions may require several bounded traces; this is a tracing limit, not
an overall export timeout. Enabling motion blur on layers remains a separate
composition choice. Draft disables shutter blur by using one time sample.

The saved value is `composition.studioQuality`. Changes use normal Undo/Redo,
save/reopen, composition isolation and revision guards. The old camera
`environment.samples` field remains readable; an explicit `set_environment`
sample edit also updates the composition's Custom sample ceiling.

The controls API is `api.scene3d.renderQuality`: `presets`, `get()`,
`set(presetOrPatch, editMeta?)`, and `onChange(listener)` returning a disposable.
Presets are `draft`, `preview`, `production`, `final`; a patch starts from the
current quality unless it supplies a preset. Individual field edits become
Custom. A preset selection resets its fields to the table above.

Agents can use:

```js
edit_3d({operation:'set_render_quality', preset:'production'})
edit_3d({operation:'set_render_quality', patch:{timeLimitSeconds:2, minSamples:16}})
export_video({format:'mp4', studioQuality:'draft'})
render_frames({times:[1,2], studioQuality:{preset:'production', denoise:false}})
```

`get_3d_scene` returns the saved quality. Exports and agent frames accept a
`studioQuality` preset or partial settings override without changing the saved
composition. Internal exports use `PM.Export.run({studioQuality:...})`;
`PM.Export.snapshotAsync(time,width,{studioQuality:...})` supports the same
capture override. Overrides also reach nested Studio compositions and are
restored after completion, cancellation or failure.

Native prepared captures combine eligible normal 3D depth sets into one linear
shutter accumulation, average path-following guides, then run OIDN once for the
frame. Each stratum takes the same sample count. Styled layers, mixed blur
switches and compositing boundaries keep separate pictures; WebGPU retains
its portable per-time filtering. Adaptive per-pixel stopping is disabled inside
shared shutter accumulation so quiet pixels cannot miss later motion times.
Unblurred Final/Refine still use the noise threshold. Caustic-free scenes skip
photons; Production builds a stable photon estimate before tracing and keeps
it through camera-only changes, invalidating it on scene or texture edits.
Temporal denoiser history is disabled for exports.

Perspective and orthographic cameras both use Studio, including camera zoom
and near/far clipping. The macOS app uses a native Metal renderer through
[`StudioBackend`](../../../docs/studio/backend.md). Other platforms use WebGPU. Native initialization or rendering failures fall back to WebGPU.
There is no three.js picture-rendering mode.

## Building a product shot

- **Import a model into an empty composition** and it arrives in a finished
  studio: resting on the floor, framed by a camera, lit by key, fill and rim
  softboxes, on the seamless set. One Undo removes all of it.
- **Set Up Studio** (camera inspector) does the same for any composition.
- **Studio Set** (camera inspector) turns the background into a seamless
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
- **Drop to Floor** (model inspector) rests a model on the floor.

## Artwork as a material

A material's **Base Color** and **Emissive** sources can be a composition or a
layer in the model's composition, including a group of 2D artwork. A hidden
source still works. A group's children keep their timing, visibility, masks,
effects, blending and relative transforms. The source root's transform and
opacity stay outside the captured artwork. Groups containing 3D layers cannot
be used as material sources; source loops are rejected when assigning a link.

A group keeps one canvas covering its artwork across its duration. Children
entering, leaving or moving therefore keep their position within the material.
Static artwork reuses its captured pixels during playback. Editing its
children, replacing an image, loading a font or changing a mask/effect refreshes
the source; moving only the receiving model or camera keeps those pixels when
the existing detail is sufficient.

Studio chooses source detail from the surface's UVs and the actual viewer or
export camera. Closeups receive more detail within a pixel and memory budget;
extreme magnification can reach that limit. Matching material slots share
textures. SVG/Figma paint transparency is retained when the artwork is extruded.
Opening a precomp follows its offset, trim and playback speed/remapping, so
the playhead follows the same source frame seen on a 3D screen.

Extensions can query `api.scene3d.geometryBounds(ids, time)` and create a shared
controller with `api.groups.createBoundsNull(ids, position)`. Positions are
0–8, top-left to bottom-right in a three-by-three grid, with middle depth.
The controller preserves existing animation and hierarchy and is one Undo.
`{replace:true}` moves a static identity controller's pivot; animated controllers
must be wrapped in a new one. Null outlines follow the viewer's actual camera.

## Large models and CAD

**FBX** files (up to 256 MB) import through the existing model picker or drop
flow. A disposable native worker uses ufbx to retain the node/parts tree,
materials, embedded PNG/JPEG color/normal/emission textures, cameras and
point/directional/spot lights. Node animation is baked to linear translation,
quaternion rotation and scale tracks; skeletal poses use the existing glTF
skin evaluation before tracing. The saved asset is a self-contained GLB,
so reopening needs no FBX reader or original file. External texture files are
not read. Skins retain the four strongest normalized weights per vertex;
area/volume FBX lights currently use a point-light approximation. The import
worker has memory, geometry and time limits and cannot block the renderer.
See [import licenses](../../../docs/studio/import-licenses.md).

Skinning currently runs on the CPU before rebuilding the posed mesh's BVH.
The generated 10,000-triangle / 30,000-vertex benchmark on a base M5 measured
20.5 ms median and 27.8 ms at the 95th percentile over 60 poses (2026-11-09).
That measurement excludes rendering and IPC; larger animated models cost more.
Reproduce it with `bun scripts/measure-studio-fbx.ts` from `apps/desktop`.

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

Extruded text samples font curves by their visible detail, including variable
font instances. High-quality Inter titles no longer exhaust the artwork budget
just because the font uses many short curves. An unreadable font, oversized
extrusion or invalid mesh/material skips that layer and logs its layer ID;
other layers keep rendering.

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
- **Add** (with Softbox, Spot, Point or Sun): the next click adds that
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
- Live screens sample their composition at the receiver's source time. Static
  inputs reuse captured pixels; changing inputs update the shared texture.
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
[`docs/studio/parity.md`](../../../docs/studio/parity.md). Substance SBSAR still
requires implementation. Studio does not
claim Blender Cycles parity.

Imported glTF/GLB materials retain `KHR_texture_transform`, independent
TEXCOORD_0/TEXCOORD_1 selection, repeat/clamp/mirror wrapping and all eighteen
standard/physical texture slots. Supported extension families include specular,
IOR, volume, clearcoat (including its own normal), sheen, iridescence,
anisotropy, emission strength and transmission. Source materials keep their
colors, scalar values, normal scales and per-map sampling when saved/reopened.
The existing whole-model finish override clears source maps and physical
settings; switching back restores the source material.

Material images now use up to 4096×4096 texels. Studio chooses a common array
size from source detail, resident geometry and device-memory hints, with a
256 MiB texture cap and a 512 MiB texture-planning target including geometry.
More images or larger geometry reduce texture resolution automatically. Both
renderers also enforce a 512 MiB geometry allowance and a conservative 2 GiB
session allowance for render targets, scene buffers and textures. Inactive
quality targets are released when needed; project close releases geometry
caches and workers. Requests that still exceed the allowance return an error
with advice to reduce detail, scatter count or output size. Some full-resolution
4K outputs exceed this allowance; out-of-core rendering is not supported.
Animated scatter counts that together exceed one million are reduced in the
evaluated scene, including exports, with a warning; the saved counts and
keyframes stay intact.
The native host independently enforces texture dimensions and total bytes.
KTX2/Basis LDR ETC1S and UASTC images decode in an isolated worker to RGBA8;
the decoder is loaded only for imports that need it. Source files stay embedded
in the saved asset. See [import licenses](../../../docs/studio/import-licenses.md).

Limits: UV sets beyond TEXCOORD_1, KTX2 HDR/array/cubemap textures and a nested
volume-medium stack are not supported. Volume attenuation uses geometric
path length in the model's normalized units; thin transmission has no volume
absorption. These import checks do not establish Blender Cycles equivalence
for every physical-material combination.

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
- **Terrain** arrives as smooth rolling hills with blended grass, soil, rock,
  beach and snow materials. In a composition without a sky or camera it also adds a
  golden-hour World, a camera, mixed trees, meadow grass, bushes and rocks, in one Undo. Shape buttons
  (Rolling hills, Mountains, Dunes, Nearly flat), **Shape from** (generated
  or a height image/layer/composition) and each ground cover's limits.
- **Scatter** offers Pine, Oak, Birch, Bush, Grass and Rock alongside user models.
  Copies of, Over, New variation, Count, Density, size and turn
  ranges, slope and height limits.
- **Material Options** (After Effects' wording) on models and on every 3D
  solid, footage, text, shape and pre-comp layer; extruded text and shapes
  show them with Front/Bevel/Side/Back.
- **Lens** (camera panel): Iris shape, rotation, roundness and aspect ratio.
- **Shots** (group of models): Exploded View with a Spread, and Turntable.
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
Legacy material regions keep their ordered slope/height selection. New natural
Terrain blends colors, roughness and metalness over shared smooth vertices.
`p.natural`, `blend`, `detail`, `erosion`, `adaptive`, `waterLevel` and `snowLevel`
are ordinary editable channels; erosion is thermal (0–64 steps). Slopes are degrees from up.
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

Built-in nature is procedural and deterministic. Agents can add a model with
`source:{nature:"birch",seed:41}` or scatter with `model:"nature:conifer"`;
other species are broadleaf, bush, grass and rock. A built-in scatter stores a
real reusable source-layer reference. Source seed changes the plant/rock shape;
Scatter seed changes placements. Thin leaves transmit diffuse light, and distant
copies switch to simpler shared meshes. Saved/imported user models retain their
existing behavior. Native/WebGPU data and measured limits are documented in
[`environments.md`](../../../docs/studio/environments.md#natural-landscape-content).
