# 3D, redesigned: Powermove Studio

Status: proposal, 2026-10-29. Nothing here is built yet.

This proposal starts from the 3D work on `feature/blender-motion-design`
(`0b2bd8d22`, "Unify 2D and 3D into one simple 3D view") and the native
engine's 3D pass on `feat/ne-scene3d` (`docs/engine/scene3d.md`). It keeps
what those got right, replaces what holds them back, and describes a
purpose-built renderer for product shots.

---

## 1. Where 3D is today

Three renderers draw the same scene:

| Renderer | Used for | Problem |
|---|---|---|
| three.js r180 (WebGL, in the editor page) | Draft shading while you work | Looks like a game preview, not a product shot. It runs on the UI thread. Each material compiles its own shader, so a first frame can stall (196 ms cold in the native port). |
| Blender 4.5 (separate process: EEVEE / Cycles) | Material Preview, Rendered, export | Slow (seconds per still frame). It is an extra install. Moving anything drops you back to draft. What you drag is not what you get. |
| Native `pm-render` scene3d pass (wgpu/Metal, Rust) | The new engine's copy of three.js | It rebuilds three.js output to within a 1-code-value bar (bit-exact for programs and uniforms). That is good for migration, but it locks the new engine into three.js's 2014-era lighting model. |

The Blender branch fixed most of the **interaction** model: 3D layers are
ordinary timeline layers, 2D and 3D share one space and one camera, and the
selection box behaves like 2D. Those choices stay.

What still fails the brief (powerful, extremely fast, intuitive, realistic):

1. **Two quality levels that disagree.** The draft and Blender looks differ, so
   people can't trust what they see while they work.
2. **The look depends on an outside app.** The good result needs Blender,
   costs seconds, and the viewer can't play it back.
3. **Controls follow how software works, not how photographers think.** Users
   set light positions with xyz numbers and keep two shading modes in their
   heads. Professional material work means editing a node graph.
4. **The engine has no specialty.** three.js, EEVEE and Cycles are built for
   any scene. Powermove's 3D is almost always *a few hero objects in a
   studio with a moving camera*, and nothing takes advantage of that.

## 2. The idea in one sentence

> **A photo studio inside your composition:** one renderer that looks like a
> finished product shot the instant you drop a model in, stays smooth on a
> base MacBook Air, and keeps getting sharper while you pause, with the export
> matching what you saw.

The renderer can be both fast and photoreal because it only does studio
scenes. A general game engine has to handle a whole city. This engine handles
a bounded stage: the hero product, any other objects in the shot (props,
pedestals, other products, 3D text, 2D layers with 3D on), a set, soft
lights and a camera. It does those extremely well, reflections above all.
That narrow focus is what makes the performance possible.

**Scene budget** (the size every speed target is measured at): up to about
50 objects and 20 million triangles in view, 16 lights, and 4K textures.
Bigger scenes still render, just more slowly; the engine is not built for
open worlds, crowds or terrain.

## 2a. The target: Apple-style product films

The benchmark is Apple's "The new iPad mini: Thinner. Lighter.
Ultraportable." (youtube.com/watch?v=3pQYJ_Um7YI, published 2026-10-26).
The goal is that a film like it can be made **entirely in Powermove**.

### What the film actually is

It was inspected frame by frame on 2026-10-29. Facts:

- 64 s long: 59 s of content, then black. 1080p, 30 fps, SDR (bt709).
- About 19 hard cuts, so roughly 3 s per shot, cut to a music track.
- Almost everything sits on one flat background, `#F6F5F8`. Only the
  display, AI and "OLED" moments go to black.

Rough share of screen time:

| Share | Kind | What happens |
|---|---|---|
| ~45% | **Product shots** (one device, no props except the Pencil) | Edge-on sliver that turns to reveal the face; held in a hand and flipped from back to front; flat lay tilting to camera; sliding in beside a line of type; pulling back from full-frame screen content to the device; macro passes over the lock screen, the Pencil tip drawing, the camera module and the wet screen |
| ~35% | **Kinetic type** on the flat background | Word-by-word reveals with blur and fade; key words in a blue-to-purple gradient; a handwritten wordmark that writes on; a hand-drawn underline that writes on; a line that shrinks and settles; legal fine print |
| ~17% | **Graphic effects** | A radial light-streak burst out of the wallpaper; animated rainbow-gradient type on black; embossed metal chip lettering under a moving light; a chip with an iridescent glow that blurs away; a light-leak flash; a glowing orb, and a glowing pill where text types in with a caret |

### What that means for Powermove

1. **The 3D is exactly the narrow product case.** It is one device on a clean
   background with soft studio light and a faint contact shadow, plus the
   Pencil. This confirms the Studio focus. What has to be perfect:
   - anodized aluminum and the colored finishes;
   - screen glass that reflects the studio;
   - the camera lens;
   - macro depth of field;
   - edge highlights that never shimmer.
2. **Every product shot shows live screen content:**
   - the wallpaper;
   - a video;
   - an editing app;
   - split-screen apps;
   - a FaceTime call;
   - a drawing that grows under the Pencil;
   - the lock-screen clock.

   "Any composition as the screen" (3.4) is therefore the **first-priority
   3D feature**, not a nice-to-have. The screen must also hand off
   seamlessly between full-frame and on-device. The film cuts from a
   full-frame video and pulls back until it sits on the iPad.
3. **Water on glass** (spill and splash shot): droplets on the screen bend
   the content behind them, and drops fall onto it. This needs a droplet
   layer on the material (normal plus refraction of the screen
   composition) and simple falling particles.
4. **Hands.** Two shots hold the device in a real-looking hand. Plan:
   composite a hand plate (footage or a still with a matte) and parent the
   3D device to it, with a shadow and contact pass. A CG hand is out of
   scope.
5. **Kinetic type is a third of the film**, and it is 2D. Powermove's text
   animators (stagger, blur, opacity, offset) and stroke trim cover most of
   it. Check and fill these gaps:
   - gradient fill on single words;
   - write-on of a script wordmark along its stroke path;
   - a typing caret;
   - an animated multicolor gradient on text.
6. **Graphic effects** are 2D effects plus Studio:
   - Radial zoom-streak with color fringing, light leak, glow and bloom, and
     iridescent glow are effect-library items.
   - Embossed metal lettering is 3D Text with a brushed-metal material and a
     Light sweep.
7. **Editing**: cuts landed on music beats (beat markers and snapping), and
   layouts where the product slides in and pushes type aside (2D and 3D in
   one space, 1 and 3.7).
8. **Delivery**: this upload is 1080p SDR. Final must still reach 4K and HDR
   for the masters.

### Acceptance test

Rebuild this 59-second film in Powermove, shot for shot:

- a CAD-quality tablet model and a Pencil;
- screen recordings and videos for the screen;
- one hand plate;
- type and effects made in Powermove.

Include it in the corpus (4.6) and make it the Phase 3 gate. Judge the
rebuild side by side with the reference.

---

## 3. Experience design

### 3.1 The nouns

Every item is still an ordinary timeline layer in the one shared 2D/3D space,
which keeps the branch's main win.

| Noun | What it is | Default on first model drop |
|---|---|---|
| **Product** | The model or its parts (a glTF/OBJ/USD/STEP import, a recipe, or 3D Text) | Centered, scaled to frame, resting on the floor |
| **Set** | Infinite curved backdrop (cyclorama) and floor, or none (transparent) | Seamless sweep in a neutral tone; renders contact shadows only when transparent |
| **Lights** | Softboxes, strip lights, spots, an HDRI | Three-softbox "Studio" rig |
| **Camera** | A real lens (mm, f-stop, focus) | 85 mm, framing the product's bounds at 70% |
| **Look** | Exposure, tone curve, bloom, grain, lens character | "Neutral" (Khronos PBR Neutral tone mapping, so product colors stay true) |

### 3.2 Zero to beautiful

Dropping a model creates the whole studio in **one Undo** and draws the first
frame in under a second, even on a base machine. People start from something
that already looks finished and adjust it. They never start from an empty
grey void.

### 3.3 Light by pointing (highlight painting)

Photographers think "I want a highlight along this edge," not "light at
x=340, z=−120". With the **Light** tool:

- **Click on the product** where you want the highlight. The engine reflects
  the view ray off the surface at that point and moves a softbox to where its
  reflection lands there (specular placement).
- **Drag** to slide the highlight across the surface; the light follows.
- **Scroll** sets the softbox size, which sets how soft the highlight is.
- **⌥-click** places a rim light, aiming from behind at the clicked edge.
- **⇧-click** places a fill light: it brightens that area without adding a
  sharp reflection.

The Lighting section's light ball, already on the branch, becomes the
overview: each light is a dot around the product, and its presets stay.

### 3.4 Materials by dragging

- A **material shelf** in the inspector holds swatches rendered by the engine
  itself. Swatches count as content, so the design language allows color
  there.
- **Drag a swatch onto any part** in the viewer, or hover over a part to
  preview the material live.
- Each preset shows **two or three plain sliders** instead of a node graph.
  Examples: *Anodized aluminum* has Color, Finish (brushed ↔ polished) and
  Wear. *Frosted glass* has Tint, Frost and Thickness. *Soft-touch plastic*
  has Color and Sheen.
- Experts (and agents) can expand to the full layered material. Section 4.3
  covers it.

### 3.5 Camera moves as Shots

A **Shot** writes real keyframes on the camera and product layers, so it stays
editable in the timeline. Presets:

- Turntable
- Push-in
- Orbit reveal
- Hero tilt
- Macro slide
- Rack focus
- Exploded view

Clicking a surface with the Focus tool sets the focus distance, which can be
keyframed for a rack focus.

**Exploded view** gets one slider. It moves a multi-part product's parts
outward along directions computed from the assembly. This is a large share of
real product motion design, and today it means keyframing each part by hand.

### 3.6 No Draft/Rendered switch

The viewer is always showing the best image this machine can produce right
now.

- **While you drag or play**, it draws the Live tier (section 4.2).
- **When you stop**, the same image keeps refining for a second or two.
- A small ring in the viewer corner shows progress ("Refining · 62%"). Click
  it for quality options.

This removes the Draft/Rendered choice, the four shading modes, and the
"Blender rendering…" waits.

### 3.7 What stays from the branch

These parts of the branch stay as they are:

- The selection box with corner scale, a rotation stem and axis spokes.
- Click selects and dragging moves.
- The axis ball, camera button and View menu in one small corner cluster.
- Camera layers pinned to the top of the timeline.
- `api.scene3d` for extensions.

New placement aids:

- **Drop to floor.**
- **Snap to surface:** place one object on another object.
- **Gravity on import.**

### 3.8 Design language fit

`apps/desktop/docs/design-language.md` and `packages/tokens/tokens.css`
already define the look. The 3D tools follow it:

- **Flat surfaces.** Gizmos and handles use the selection ink and `--accent`
  only. Axis colors are muted. No glossy, bevelled "3D-looking" chrome.
- **Elevation only on floating things.** The corner cluster and the
  highlight-painting tooltip are floating elements, so they use
  `--shadow-float`. Inspector sections are flat rows (`--row-h`, `--val-w`
  value column).
- **Color only where it is content:** material swatches, light color wells
  and the light ball.
- **Light and dark themes both checked.** Light is the default.

### 3.9 Agent-native

`edit_3d` gains operations that mirror the human gestures. Every one is a
normal revision-guarded, undoable edit:

```json
{"operation":"light_highlight","target":"bottle","at":{"surface":[0.42,0.18,0.31]},"softness":0.6}
{"operation":"apply_material","target":"bottle/cap","preset":"anodized_aluminum","p":{"color":"#2B2E33","finish":0.7}}
{"operation":"shot","type":"turntable","duration":4,"degrees":360,"easing":"inOut"}
{"operation":"set_look","tone":"neutral","bloom":0.15,"grain":0.05}
{"operation":"explode","target":"headphones","amount":0.6}
```

`render_frames` always renders at Final quality, so an agent checks exactly
what the export will show.

---

## 4. The engine: `pm-studio`

### 4.1 Where it lives

The 3D system on `main` and the Blender branch never shipped, so Studio
**replaces** it outright instead of sitting beside it:

- **No compatibility mode.** Studio is the only 3D renderer. Development
  projects that already have 3D layers are converted once when opened, on a
  best-effort basis; their old look is not preserved.
- **Retired:** the three.js renderer in the editor, Blender as a renderer
  (EEVEE/Cycles previews and export), and the native engine's
  three.js-parity `scene3d/` pass. Pieces that are still useful move over:
  - `pm-geom3d`: primitives, lathe, extrude, OBJ.
  - Layer ordering and depth sets.
  - The depth and id surface.
- **Where the code goes:** `engine/crates/pm-render/src/studio/`, plus a
  `pm-studio` crate for scene prep. The same Rust code serves the viewer,
  export and agent frames.
- **No Blender, anywhere.** Blender is not used to render, model or import:
  - The procedural recipes (Rocket, Staircase, 3D Text) and their modifiers
    (Bevel, Array, Mirror, Subdivision, Solidify) are rebuilt in Rust next
    to `pm-geom3d`'s lathe and extrude. They run instantly, with no install.
  - Models come in as glTF/GLB, OBJ and USD (STEP for CAD in Phase 4).
    `.blend` files are not supported; Blender users export glTF, which
    Blender does natively.
  - Blender procedural shader graphs are replaced by Studio's layered
    material (4.3).
  - Retired along with it: the Blender worker and IPC, the Blender schema
    and shader-node whitelist, Blender render surfaces, and the native
    engine's `pm-blender` crate and RFC 11.
- **Language and API:** Rust, wgpu, WGSL shaders, using the
  `pm-gpu`/`pm-cache`/`pm-media` crates that already exist.

### 4.1a Platforms and the GPUI rewrite

Powermove ships on macOS only. Studio is built for the Mac first, but in a
way that keeps a Windows port cheap and lets it move from today's Electron
shell to the planned GPUI interface without changes.

- **The engine has no UI of its own.** It is a Rust library behind a small
  interface:
  - The host sends it scene edits and the time.
  - The engine returns a finished frame in a **shared GPU image**, plus
    answers to "what is under the pointer?" and object bounds.

  It never draws buttons, handles or menus.
- **The host draws all interaction.** Selection boxes, handles, the
  highlight-painting cursor and the corner cluster are drawn by whichever UI
  hosts the engine: Svelte/Electron today, GPUI later. The engine supplies
  the geometry (projected points, picks). That keeps gizmos instant (4.4,
  point 6) and means the GPUI rewrite reimplements only drawing code, not
  3D logic.
- **Shared GPU images by platform:**

  | Platform | Mechanism | Shown by |
  |---|---|---|
  | macOS | IOSurface (already used by `pm-gpu`'s `bridge_out`) | Electron today; a GPUI surface element later |
  | Windows | DXGI shared texture handle | GPUI's DirectX renderer |

- **Platform-specific code is fenced off.** Everything goes through wgpu
  except three optional accelerators, each in its own module:
  - MetalFX upscaling;
  - Metal hardware ray tracing;
  - the IOSurface output.

  Each has a portable fallback: temporal upscaling in WGSL, a compute BVH
  walk, and a CPU copy. A Windows build turns on wgpu's DX12/Vulkan backends
  and adds DXGI output; no renderer code changes.
- **Denoiser:** Open Image Denoise runs on Metal, CUDA, SYCL and HIP GPUs
  and on the CPU, so it is portable as well.
- **Continuous integration** builds the engine for Windows from Phase 1, even
  before anyone ships it, so Mac-only assumptions are caught early.

### 4.1b No web build

Web animation export was removed from Powermove (2026-10-29), so Studio needs
no browser or WebAssembly build. Every output (viewer, video, image
sequence, agent frames) goes through the native engine.

### 4.2 One light model, three speeds

All three tiers use the same materials, lights and color pipeline. Live is
trained by the path tracer (section 4.4), so the three tiers agree.

| Tier | When | How | Budget on a base M1 Air |
|---|---|---|---|
| **Live** | Dragging and playback | Rasterized forward PBR with precomputed lighting caches; analytic area lights (LTC); signed-distance-field soft shadows and ambient occlusion | ≤ 6 ms GPU at 1080p output (720p internal + temporal upscale) |
| **Refine** | Paused or idle | Progressive path tracing in ≤ 2 ms time slices (RFC 13 time-slicing) on top of Live, denoised | Visually clean in ≤ 2 s at preview resolution |
| **Final** | Export, agent captures | The same path tracer with a fixed sample count per frame, motion blur and DOF, deterministic seeds | ~1–2 s per 1080p frame for a typical product |

### 4.3 What makes it look real

Each item below is aimed at product photography:

- **Layered material based on OpenPBR Surface.** OpenPBR is the 2024
  ASWF/Adobe/Autodesk standard, so MaterialX and USD materials import
  cleanly. Its layers:
  - base (metal or dielectric)
  - specular with **anisotropy** (brushed metal)
  - **coat** with its own roughness and tint (car paint, lacquer)
  - **sheen** (fabric, soft-touch plastic)
  - **transmission** with thickness and **dispersion** (glass, liquid)
  - **subsurface** (skin, wax, soap)
  - **thin film** (iridescent coatings)
  - emission
- **Energy-preserving microfacets** (multiple-scattering compensation), so
  rough metal does not go dull and dark the way it does in three.js.
- **Rounded-edge shading.** CAD models have razor-sharp edges, which look fake
  on camera. A bevel-normal shader rounds them optically by a set radius
  without changing the geometry. Real products catch light on their edges,
  and this gives the most realism for its cost.
- **Big soft lights done properly:**
  - Area lights use linearly transformed cosines (LTC) in Live and are
    sampled exactly in Refine and Final.
  - Softboxes can have grids and edge falloff. Strip lights get crisp
    highlights.
- **The set is analytic.** The cyclorama is a math surface, not a mesh, so
  contact shadows, floor reflections (blurred by floor roughness) and the
  gradient sweep are exact and cheap.
- **Physically based camera:** real-lens DOF with bokeh, a shutter for motion
  blur, and optional vignette, slight chromatic aberration, bloom and grain.
- **Color pipeline:**
  - Lighting is computed in linear ACEScg.
  - Hero-wavelength spectral sampling in Refine and Final handles dispersion
    and colored glass.
  - Output goes through **Khronos PBR Neutral** (true product color) or
    **AgX** (filmic) into Powermove's existing color and HDR pipeline.

### 4.3a Reflections

Reflections are what makes products read as real: chrome showing the
studio, glass doubling a highlight, a glossy floor mirroring the product,
one object reflected in another. Studio treats them as a first-class
feature in every tier, not a screen-space afterthought.

**Refine and Final: exact.** The path tracer follows every reflection
through the whole scene:
- reflections of reflections, up to 8 mirror bounces by default;
- glossy blur from roughness, and stretched highlights from anisotropy;
- the double reflection of a clear coat over a metallic base;
- Fresnel per wavelength for glass and gemstones.

**Live: real rays, not tricks.** Live uses hybrid ray-traced reflections
against the same scene BVH:

1. **Mirror and glossy surfaces** (roughness under about 0.4) trace real
   reflection rays. Every object shows up, including ones off screen or
   behind the camera, which screen-space reflections cannot do.
   - Hardware ray tracing on M3 and newer; a compute BVH walk elsewhere.
   - On a base machine, rays run at half or quarter resolution and are
     reconstructed by a reflection denoiser guided by roughness and hit
     distance.
   - Each hit is shaded with the full material model and Live's lighting.
     A second bounce falls back to the environment.
2. **Rough surfaces** cone-trace the scene distance field (4.4). That gives
   soft, correctly blurred reflections of nearby objects at a fraction of
   the cost of rays.
3. **Flat floors and mirrors** get exact planar reflections, blurred by the
   floor's roughness.
4. **Studio lights appear as their real shapes.** A softbox reflects as a
   crisp rectangle with its grid and edge falloff, and a strip light as a
   clean line. Reflection rays intersect the light shapes analytically
   instead of reading a blurred environment map. This is the signature look
   of studio product photography.

Reflections get up to 2 ms of Live's 6 ms budget on a base machine. When
that is too little, the frame-budget governor lowers reflection resolution
first and never drops the rays entirely.

### 4.3b Caustics

Light focused by glass or a polished part (a bright pool inside a glass's
shadow, a ring of light off a watch case) is nearly invisible to camera paths:
they would have to hit a small light through a curved surface by chance.
Studio traces it from the other end:

1. Before each batch of samples, a `photons` pass shoots photons from the
   lights (softboxes by area, spheres and spots by solid angle, the sun over
   the scene's bounds). A photon is kept only if it passes through glass or
   bounces off polished metal and then lands on a matte surface or the
   studio floor.
2. Landing photons add their flux to a hashed grid at two cell sizes. Cells
   keep a key, so two cells that hash alike never share light.
3. Matte hits and the shadow catcher read the grid: the fine cells where
   enough photons landed, the coarse cells where light is sparse.
4. Camera paths that would find the same light (matte, then glass, then a
   light) stop counting it, so nothing is lit twice.

Caustics run in Refine and Final only, and only when the scene has glass or
polished metal. They add about 20% to Final time in those scenes.

### 4.4 Why it can be fast: the product-shot assumptions

These assumptions only hold because the scene is a bounded studio stage
(section 2's scene budget). A general engine can't make them.

1. **Precompute per model, once, in the background**, cached on disk by
   content hash (`pm-cache`). Each model is usable immediately and gets better
   as the steps finish:
   - A BVH (the ray-tracing acceleration structure) per model. A two-level
     scene BVH places the models; with tens of objects it is rebuilt every
     frame in well under a millisecond, so objects can move freely.
   - A **signed distance field** per model (64³–128³), composed into a scene
     distance field each frame. It gives soft shadows, ambient occlusion and
     contact between objects (a bottle resting on a plinth) from cone
     tracing in a few texture reads, which is the core trick for Live.
   - Bent normals, ambient occlusion and thickness baked per vertex or texel.
     They are intrinsic to the object, so they stay valid when it rotates.
   - LOD chain and cluster simplification (meshoptimizer), so a 10M-triangle
     CAD import draws only as many triangles as the screen has pixels.
   - Textures transcoded to GPU-compressed formats (KTX2 → ASTC/BC).
2. **The light rig is an environment.** Softboxes and the HDRI are baked into
   a prefiltered radiance cubemap plus spherical harmonics. That only reruns
   when the rig changes, which takes about a millisecond in compute. The
   strongest lights also stay analytic so their highlights stay sharp.
3. **The path tracer trains Live.** While idle, Refine's results update the
   light probes and floor-shadow cache that Live reads, so Live keeps getting
   closer to Final the longer you work.
4. **A fixed pipeline set.** There is one uber-shader per material layer
   combination, chosen by specialization constants and precompiled at
   install. There is no per-material compilation, so no first-frame stall.
5. **Frame-budget governor.** GPU timestamps measure every frame. Internal
   resolution, shadow quality and sample counts adjust to the budget, and
   temporal upscaling (MetalFX on Apple Silicon) fills in the rest. A slow
   machine gets a softer image, never a stutter.
6. **Gizmos never wait for the renderer.** Selection, handles and the
   highlight-painting preview are drawn by the UI immediately. The render
   catches up a frame later, so interaction always feels instant.
7. **Unchanged means not redrawn.** Content keys (already in `pm-render`) let
   playback reuse a rendered 3D layer whenever its inputs did not change.
8. **Hardware ray tracing when available** (Metal on M3 and newer, or ray
   query through wgpu). Otherwise a compute-shader BVH walk, which stays fast
   because the scene is bounded and rays are spent only where reflections
   and shadows need them.
9. **Denoising.** Intel Open Image Denoise 2 runs on the Metal GPU, uses
   albedo and normal guides, and is Apache-licensed. It is used for Refine
   and Final. A light temporal filter keeps exported sequences from
   flickering.

### 4.5 Machine tiers

| Machine | Live | Refine | Final |
|---|---|---|---|
| Intel Mac / integrated GPU | 30–60 fps; quarter-resolution reflection rays | Half-resolution, slower | Same output, more time per frame |
| **Base Apple Silicon (M1/M2 Air, 8 GB): primary target** | 60 fps at 1080p; half-resolution reflection rays | ≤ 2 s clean | ~1–2 s per frame |
| Pro/Max, M3+ | 120 fps; full-resolution hardware ray-traced reflections | Near-instant | Fractions of a second per frame |

The base machine is the **primary** measurement target from the first spike.
The native engine's Phase 2 gate deferred base-machine rows; this plan should
not.

### 4.6 How we know it's right

- **Reference oracle.** PBRT-v4 and Mitsuba 3, research renderers built for
  ground truth, render the same test scenes offline in the test suite. They
  are never part of the app. The image-quality bar is measured the same way
  the parity work measured against three.js.
- **Physics checks:**
  - White-furnace tests: materials neither create nor lose energy.
  - Reciprocity checks.
  - Convergence-rate tests.
- **A product corpus:**
  - Bottle (glass + liquid + label)
  - Headphones (anodized metal, soft-touch plastic, fabric)
  - Watch (polished and brushed metal, sapphire, small parts)
  - Sneaker (fabric, rubber, subsurface)
  - Phone (glass back, thin film, a 5M-triangle CAD import)
  - Tabletop (a watch on a marble plinth beside a glass of water and a
    leather strap: contact, refraction and object-to-object reflection)
  - Chrome room (two chrome spheres, a mirror and a lacquered car-paint
    panel: reflections of reflections, coat over base, light shapes in
    reflections)
  - Tablet film (the 2a acceptance test: a CAD tablet with live screen
    compositions, a Pencil, water droplets on the screen, macro passes and a
    hand plate)
- **Performance gates on the base machine**, the same in every phase:
  - Live frame time
  - Seconds to clean in Refine
  - Final seconds per frame
  - Drag input-to-pixel latency
  - Import-to-first-frame time
  - Peak memory

---

## 5. Roadmap (gated, each phase shippable)

| Phase | What | Gate |
|---|---|---|
| **0. Spike** | Standalone Rust binary: the bottle, headphones and Chrome room scenes, Studio rig, Live + Refine on a base M1 Air | 60 fps Live at 1080p with ray-traced reflections; Refine clean ≤ 2 s; side-by-side with the PBRT reference that a designer can't reliably tell apart |
| **1. Engine core** | `pm-render/src/studio/`: OpenPBR materials, area lights, reflections, set, camera, color; recipes and modifiers in Rust; Blender code deleted; viewer via the existing bridge | Corpus within the quality bar against the PBRT reference; base-machine gates hold inside the app |
| **2. Studio UX** | Auto-studio on import, highlight painting, material shelf, Shots, no Draft/Rendered switch, agent ops | Usability: a first-time user gets a publishable shot in < 5 minutes unaided |
| **3. Final output** | Export through Final tier, motion blur, DOF, denoise, temporal stability; live screen compositions; instances and variants | Export parity with viewer when paused; no flicker in the corpus turntables; the 2a reference film rebuilt shot for shot in Powermove |
| **4. Hard materials** | Glass dispersion, caustics on the floor, subsurface, exploded views, CAD (STEP) import, Gaussian-splat products | Per-feature gates |

## 5a. Status (2026-10-30, branch `new-3d-system`)

User and agent guide: `apps/desktop/docs/STUDIO.md`.

| Phase | Built | Not built |
|---|---|---|
| 0. Prototype | `engine/crates/pm-studio`: the wgpu path tracer and its product scenes, measured on a base M5 (Live 3.6–11 ms at 1080p; Refine 64 spp 0.4–3.8 s) | A base M1 Air run (no such machine here); a side-by-side against PBRT/Mitsuba |
| 1. Engine core | The same WGSL in the app on WebGPU: scene extraction, two-level BVH (moving a 153,600-triangle model costs 0.03 ms instead of a 438 ms rebuild), per-layer output, Live/Refine/Final, softbox/sphere/spot/sun lights, color/emissive/ORM/normal maps, glass-tinted shadows, temporal reprojection for Live (16 frames deep while dragging) | Hosting Studio inside the native engine (`pm-render`) and GPUI — the native engine is unmerged and GPUI not yet started; meshlet LODs and KTX2. Hardware ray tracing (2–3× faster) and OIDN run in the Rust host only; WebGPU exposes neither, so the app keeps the software BVH and relies on samples |
| 2. Studio UX | Finishes, light by pointing (click, drag to slide, scroll to soften), Shots, studio set, auto studio (and on first import), drop to floor, place on, exploded views, lens f-stop/focus, refine indicator, finish swatches rendered by Studio; every tool in `edit_3d` | — |
| 3. Final output | Per-composition Final samples, motion blur sharing the budget, depth of field, **live screens** (a composition playing on a product's display), and the acceptance film (`e2e/studio-film.spec.ts`: an original product film made and exported entirely in Powermove) | A shot-for-shot rebuild of the reference film (it needs a CAD-quality device model) |
| 4. Hard materials | Sheen, thin film, dispersion, water droplets, brushed metal (anisotropy), subsurface (random walk), **caustics** (photons through glass and off polished metal, Refine/Final) | STEP import, Gaussian splats; hand plates use ordinary 2D compositing |

## 6. Risks and open questions

- **Caustics and stacked glass** stay the slowest case. Built as a photon
  grid (section 4.3b): Refine and Final only, about 20% more
  Final time in scenes with glass or polished metal and none otherwise. The
  grid trades sharpness for smoothness where photons are sparse.
- **Denoised animation can shimmer.** Final needs enough samples or temporal
  denoising. Measure it on the corpus turntables early.
- **Only one GPU backend is turned on today.** `pm-gpu` enables wgpu's Metal
  backend only. A future Windows build needs the DX12/Vulkan backends, DXGI
  output (section 4.1a) and testing on low-end PC GPUs.
- **Native engine readiness.** The Phase 2 gate failed on latency and cadence
  rows. Studio depends on the engine's viewer bridge being fast. Phase 0 can
  run standalone, but Phase 1 waits on that bridge.
- **Decided (2026-10-29):**
  - Studio replaces the unshipped 3D system; projects upgrade
    automatically.
  - Mac only for now, built so a Windows port and the GPUI rewrite need no
    engine changes.
  - No Blender integration of any kind; `.blend` is not supported.
  - Web animation export is removed from the app, so there is no web build.
  - Scenes hold multiple objects, not just one product, and reflections are
    a first-class, ray-traced feature in every tier.
- **Still open:** is section 2's scene budget (about 50 objects, 20 million
  triangles) the right size to design for?
