# Studio environments

Environment work extends `pm_studio::scene::Scene`; existing serialized scenes
remain valid. `Scene.meters_per_unit` defaults to 0.001 for legacy millimetre
product scenes; set it to 1 for metre-sized environments. It converts focal
length/aperture from millimetres into scene coordinates. All distances and
extinction coefficients use the scene's own units (the older product corpus uses
millimetres). Geometry and volume transport run in
the shared WGSL, including portable WebGPU traversal. Metal ray queries accelerate
intersections but do not change the lighting model. Environment ray tolerances
scale with scene bounds so millimetre trim/glass in a metre-sized room stays
intersectable; the legacy product tolerance is retained without world features.

## World contract

`Scene.world` is optional; omitted means the existing studio dome/backdrop.
Its fields (all optional, defaults below) are:

- `sky`: null or `{elevation:45, azimuth:0, turbidity:2,
  ground_albedo:{x:0.2,y:0.2,z:0.2}, intensity:1, sun_intensity:1,
  sun_radius:0.266, overcast:0, visible:true}`. Angles are degrees; azimuth zero
  faces +Z and +90 faces +X; +Y is up. `Sky::preset` accepts `clear-noon`,
  `golden-hour`, `overcast`, `blue-hour`, `night`.
- `hdri`: null or `{width,height,pixels:[{x,y,z},...], rotation:0,
  intensity:1, visible:true}`. Pixels are linear HDR RGB, row-major from the
  north pole, column zero at +Z. Rotation is degrees around +Y. HDRI replaces
  sky background/dome lighting when both are provided; the sky's sun still
  contributes. `EnvironmentMap::load` decodes Radiance HDR/RGBE and OpenEXR.
  Decode files before sending evaluated scene data; no local file paths cross
  the rendering boundary. Maximum 2048×1024; the Rust loader downsizes larger
  files to this budget. Floats use linear sRGB/Rec.709 primaries; convert other
  EXR working color spaces at import. Visibility affects camera rays only;
  reflections and illumination remain.
- `atmosphere`: `{density:0, color:{x:0.9,y:0.9,z:0.9}, anisotropy:0,
  base_height:0, height_falloff:0, distance:10000}`. Extinction at point y is
  `density * exp(-max(y-base_height,0)*height_falloff)`. `color` is scattering
  albedo, not emission. `distance` bounds transport on each ray segment.
- `volumes`: array (maximum 32) of `{shape:"box"|"sphere",center:{x,y,z},
  size:{x,y,z},density,color:{x,y,z},anisotropy}`. Box size is half extents;
  sphere radius is size.x. Overlapping extinction/scattering add. Albedos are
  0..1; HG anisotropy is -0.95..0.95, positive for forward scattering.
- `adaptive`: `{enabled:false,min_samples:64,relative_error:0.025,
  absolute_error:0.001}`. Refine/Final skip pixels whose estimated standard
  error is below max(absolute_error, |mean luminance| × relative_error).
  Live always runs. Requested sample count remains a ceiling; pixel counts
  are retained individually. Early stopping is a biased noise heuristic.

The sky integrates Rayleigh and Mie single scattering in a spherical atmosphere
into a small 128×64 linear map. A bounded diffuse closure approximates higher
orders, overcast and night; this is not full spectral/multiple-scattering
Bruneton, Hosek-Wilkie or Cycles parity. The separate sun disc is importance
sampled at its true solid angle (radius 0.266° by default), with atmospheric
attenuation. Environment maps use a luminance × exact texel-solid-angle CDF;
sampling is uniform in solid angle within each pixel, including polar rows.
A 1% uniform-sphere mixture avoids starving dark map regions.
Surface lighting uses MIS against BSDF sampling. Radiance lookup is bilinear
with longitude wrapping and polar clamping; the PDF remains the exact piecewise
constant sampling distribution.

Refine/Final use delta tracking for medium events, exact analytic shadow
transmittance for the current height/box/sphere media, HG phase sampling, and
multiple volume/surface bounces. Live uses
two primary-ray single-scattering steps with an ambient approximation and
analytic height/box/sphere optical depth. Both produce geometry-dependent shafts
and aerial perspective. Dense/very large
media can be expensive; tracking clips to potentially occupied intervals and
tightens the global majorant with height; there is no full empty-space hierarchy
yet.

## GPU transport

The existing uniform and buffer strides are unchanged. Optional world data is
appended after `4*lightCount + 9*instanceCount` vec4 rows in `Prepared.sdata`.
Header version 1 is six vec4 rows:

| Row | Values |
|---|---|
| 0 | magic 735031, version 1, map width, map height |
| 1 | rotation radians, intensity, camera visible 0/1, pixel offset (6) |
| 2 | global scattering albedo RGB, density |
| 3 | global g, base height, height falloff, segment distance |
| 4 | local-volume offset, count, light-CDF offset, light-tree offset (0: absent) |
| 5 | adaptive enabled, min samples, relative error, absolute error |

Offsets are relative to the header in vec4 rows. Map pixels (RGB,0) precede one
CDF row per pixel (cumulative probability, solid-angle PDF,0,0). Each volume uses
three rows: center XYZ/shape (0 box,1 sphere), half-size XYZ/density,
albedo RGB/g. Light rows contain cumulative selection probability, PMF, tree
leaf index,0.
Above eight lights a balanced spatial tree follows: three rows per node,
(min XYZ,left child or light index), (max XYZ,right child or -1 for leaf),
(finite power,infinite power,parent,0). Selection estimates power/distance²
from node bounds, with a small probability floor. Reverse traversal computes
the exact position-dependent selection PMF for MIS. Refine/Final and fresh Live
reservoir candidates use this hierarchy; finite-emitter intersection also walks
its bounds. This is a light tree, not hundreds of independent visibility rays.
Legacy uploads without a trailer continue to use uniform light selection.
Within the existing nine-row instance block, row 3.w remains the BLAS root;
row 4.w packs the instance-color flag in bit 0 and `(layer override + 1)`
in the remaining bits (0 inherits). Row 5.w retains the existing primary-ray
hidden flag. Packed flag integers remain exactly representable as f32; layer
overrides must be below 8,388,607.

## Validation and measurements

Measured 2026-11-09 on Apple M5, 16 GB unified memory, macOS 27.3,
Electron 44.0.0. Another desktop lane was active; machine load and clock state
were not locked. These are actual measurements, not extrapolated GPU estimates.

The hidden Electron integration harness made 45 camera/time edits per fixture.
Output was 1920×1080; native Live traced 640×360, one sample, three surface
bounces, direct-light reservoirs, Metal ray queries and MetalFX. Live frames used
Electron shared textures. Timing starts before the camera edit and ends after a
canvas pixel read forces completion; it includes IPC and composition, excludes
scanout/vsync, and is not the full editor compositor. Refine used 16 samples at
1920×1080 plus CPU OIDN. Sixteen samples are a preview, not convergence.

| Scene | Total placements / lights | Live median / p95 | Refine + OIDN | Peak native RSS |
|---|---:|---:|---:|---:|
| Room: practicals, window sun, global + local fog | 47 / 3 | 17.3 / 18.4 ms | 3.31 s | 1.38 GiB |
| Blue-hour street: 50 lamps | 689 / 50 | 20.9 / 21.2 ms | 3.08 s | 1.42 GiB |
| Golden-hour terrain: 50k scattered trees/rocks | 50,004 / 1 | 15.0 / 15.2 ms | 2.64 s | 1.46 GiB |
| Same landscape: 100k copies | 100,004 / 1 | 16.4 / 16.6 ms | 2.85 s | 1.48 GiB |

The landscape has 33,284 unique triangles and four prototypes (trees, displaced
rocks and two terrain material regions). The scatter count is exactly 50,000 or
100,000; total placements also include two source placements and two terrain
regions. Room/street have 1,832/8,268 unique triangles. This proves large instance
counts with simple procedural content, not production foliage realism.

Host render medians, excluding the rest of the Electron path, were
15.16 / 18.71 / 12.92 / 14.40 ms in table order. Cold first frames took
1.42 / 0.37 / 0.19 / 0.21 seconds, including target/pipeline setup. Maximum
camera-edit latency was 27.2 ms in this final run. Earlier runs during heavier
concurrent work had 100–130 ms tails; these timings do not guarantee sustained
60 fps. The cheaper Live budget trades fine detail for responsiveness;
Refine/Final retain output resolution.

Peak RSS is `getrusage` for the native process after Live, full-resolution Refine
and 960×540 Final. It excludes Electron and is not a total unified GPU-memory
measurement. The standalone prepared scene buffers measured 14.54 MiB for 50k
and 23.24 MiB for 100k; acceleration structures and render targets are additional.
Standalone preparation took 56.2 / 90.6 ms and device creation + upload
86.7 / 84.7 ms, with warm driver caches, respectively.

At 960×540, 64 samples and denoising disabled, all four native/WebGPU comparisons
passed. Mean absolute RGB differences were 0.0628 / 0.0437 / 0.0033 / 0.0036
code values out of 255; isolated noisy edge pixels differ more. The harness
waits for browser GPU completion before reading comparison pixels. A populated
to empty transition passed. Killing the owned native process replayed the room
into WebGPU with verified pixels in 81.4 ms in a separate recovery run.
The original hero/chrome product comparisons also passed (mean differences
0.0043 / 0.8114 code values). Hardware-ray-query/MetalFX-disabled smoke passed.

A separate Blender 5.2.0 Cycles room reference is saved with the evidence. It
uses the same meshes, baked sky and sun, approximate Principled materials, and
finite global/local fog bounds. It is a qualitative transport reference, not an
independent sky validation or proof of Cycles equivalence. Its two-thread CPU,
64-sample + denoise run took 72.5 seconds; this is not a hardware-matched speed
comparison with Studio.

Evidence, PNG renders and captions are under `/tmp/pm-shots/studio3d/laneC/`,
outside Git. No screenshots or binary assets are committed.

### Reproduce

```sh
CARGO_TARGET_DIR=~/Developer/pm-target CARGO_BUILD_JOBS=2 nice -n 10 cargo test --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
CARGO_TARGET_DIR=~/Developer/pm-target CARGO_BUILD_JOBS=2 nice -n 10 cargo build --manifest-path engine/Cargo.toml -p pm-studio-host -p pm-studio --bins --release --features oidn,metalfx
PM_STUDIO_SCENES=env-room,env-street,env-landscape,env-landscape-100k PM_STUDIO_FRAMES=45 PM_STUDIO_SHOTS=/tmp/pm-shots/studio3d/laneC/final POWERMOVE_STUDIO_HOST=~/Developer/pm-target/release/pm-studio-host nice -n 10 node apps/desktop/scripts/studio-native-electron.mjs
nice -n 10 ~/Developer/pm-target/release/studio-environments --scene env-landscape --spp 64 --out /tmp/pm-shots/studio3d/laneC/preparation
bun run --cwd apps/desktop test -- src/main/studio-native/manager.test.ts src/renderer/src/core/studio
bun run --cwd apps/desktop typecheck
```

Use a freshly built matching host when running several checkouts against the
same Cargo target directory. The integration harness expects the optional
`studio-surface.node` beside the host for shared textures; otherwise it measures
the binary fallback. The native build script produces that addon. No app release
or publication is required.

Final feature validation: 49 Rust unit/GPU/host tests and 24 targeted desktop
tests passed. Full desktop typecheck passed (two pre-existing Svelte warnings),
as did the no-default-features Cargo check, real Electron transport, all four
native/WebGPU comparisons and process recovery. The native room smoke also
passed with hardware ray queries, reservoirs, MetalFX and shared textures disabled.

### Remaining quality limits

- RGB single-scattering sky with approximate higher orders/twilight/overcast;
  no spectral sky LUT, weather clouds, moon or stars.
- Live fog is an approximation; Refine/Final use tracked volume transport.
  Low sample counts can lose fine shafts through denoising. Media are world
  transport, not separately pickable geometry or independent layer IDs.
- HDRI ground projection is not implemented; the existing shadow catcher works
  with environment illumination. Large decoded HDRIs use prepared binary
  uploads, since canonical JSON packets are limited to 4 MiB.
- Terrain uses bounded camera-dependent uniform grids, rather than local GPU
  microdisplacement. Thermal erosion is available; hydraulic erosion, out-of-core
  paging, collision avoidance, normal-aligned scatter and density textures remain
  gaps. Instance LOD switches meshes without crossfading.
- The corpus uses deterministic procedural assets without external textures.
  These improve default content; formal Cycles parity and sustained full-editor
  performance remain gates.

Physical references: [Bruneton and Neyret's atmosphere model](https://ebruneton.github.io/precomputed_atmospheric_scattering/),
[PBRT transmittance](https://www.pbr-book.org/4ed/Volume_Scattering/Transmittance),
and [HG phase functions](https://www.pbr-book.org/4ed/Volume_Scattering/Phase_Functions).

## Terrain and instancing contract

`Scene.instances`, `Scene.scatters` and `Scene.terrains` default to empty arrays.
With no explicit instances, each object has its usual identity placement. A
nonempty `instances` array is the full placement list: source objects become
prototypes and only the listed placements render. Every placement is
`{object, position:{x,y,z}, rotation:{x,y,z}, scale:{x,y,z}, layer:null}`.
`object` indexes `Scene.objects`; rotation is degrees applied X, then Y, then Z;
scale defaults to (1,1,1), position/rotation to zero. Optional numeric `layer`
overrides the prototype's layer for output masks. Positive scale only.

Each scatter is `{objects:[0],seed:1,count:100,density:1,terrain:null,
surface:null,center:{x:0,y:0,z:0},size:[100,100],scale:[0.8,1.2],
rotation:[0,360],min_height:-1e9,max_height:1e9,min_slope:0,max_slope:60,exclusions:[]}`.
`objects` selects prototypes; `count` is the number of candidates (maximum
one million), and density (0..1), height/slope and exclusion rejection can
reduce the accepted count. The sequence remains stable when filtering changes.
A candidate uses uniform yaw and uniform isotropic scale in the requested
ranges. `terrain` indexes `Scene.terrains`, `surface` indexes a mesh (area-weighted
triangle sampling); both null means a flat rectangle. Supply only one source.
`size` is the X/Z sampling rectangle for flat/terrain scatter. Each exclusion
is `{center:{x,y,z},radius}` and excludes an X/Z disc. Slopes use degrees from
+Y; height is world Y. Scattered models stay upright; normal alignment, collision
avoidance and variable density maps are not implemented.

Each terrain is `{center:{x:0,y:0,z:0},size:[100,100],resolution:64,
amplitude:10,frequency:0.02,octaves:5,seed:1,ridged:false,heights:[],
material:0,layers:[]}`. Resolution is 2..512 quads per side. Empty heights uses
seeded fBm/ridged value noise. Otherwise supply `(resolution+1)²` normalized
row-major grayscale values; height is center.y + amplitude × value. Load/resize
an image outside the scene contract before sending heights. Material layers
are ordered `{material,min_height,max_height,min_slope,max_slope}` regions;
the first matching triangle center/face slope wins, otherwise base material.
With `blend:0,natural:false` these retain the original discrete regions.
Nonzero `blend` smoothly combines region colors, roughness and metalness at
shared vertices. Terrain gets smooth vertex normals. Scatter evaluates the actual piecewise planar mesh grid, so
instances do not hover over an independently evaluated noise function.

`Scene::expanded()` resolves terrain/scatter once. Shared source vertices and
one BLAS per prototype remain resident; the TLAS and nine-vec4 transform rows
scale with placements. Both the standalone renderer and native scene protocol
use this path. Transform patches address placement input order when explicit
instances are present. Maximum one million total placements. Optional distance-based mesh LOD is available; paging is not.

## Desktop integration

`StudioSceneDescription.scene` accepts the same optional `world`, `instances`,
`terrains`, `scatters`, `displacements` and `meters_per_unit` fields.
`compileSceneDescription` resolves procedural geometry, makes one Three
`InstancedMesh` per prototype, and packs the shared GPU trailer; native prepared uploads and the browser backend consume identical
buffers. Direct graph extraction also handles `THREE.InstancedMesh` without
copying triangles. Changing instance capacity rebases BLAS roots correctly.
Scene-description round trips remove the automatically generated sun before
serializing `world.sky`, so editing a scene cannot duplicate its sun.

The desktop serializer writes the scale explicitly. Older desktop descriptions
without it keep their existing 0.1 metres/unit convention; old Rust scenes keep
0.001. This preserves previous camera apertures while new documents can state
their actual scale.

Import `StudioWorld` from `core/studio/environment.ts` and the geometry types
from `core/studio/environment-geometry.ts`. `packWorld` and `worldSun` are the
portable preparation helpers for evaluated app data. HDR/EXR decoding stays at
the file-import boundary; `hdri.pixels` carries linear floats, not display PNGs.
`Terrain::load_height_image` is the Rust grayscale decoding/resizing helper.

Camera-only native edits use capability-gated `preparedView {key,view}` messages
with no binary geometry, TLAS, materials or instance rows. Older hosts retain
the existing prepared-patch fallback. Camera edits preserve the native placement
cache and temporal history; changing projection, lighting or output policy resets
history. Browser diagnostics explicitly wait for queued GPU work before pixel
readback. A lost browser device also discards its compiled pipeline cache.

## Geometric displacement

`Scene.displacements` defaults to `[]`. Each entry is
`{object,width,height,values,strength,mid_level:0.5,subdivision:0}`. The target
is a source object, not an individual placement. `values` is width×height
normalized grayscale, first row at UV v=0. Clamped bilinear lookup offsets
vertices along their source normals by `(height-mid_level)*strength`, in scene
units. Subdivision levels 0..5 share edge midpoints; area-weighted shading
normals are rebuilt after displacement. Limits: one million output triangles
per displaced prototype and two million displaced triangles per scene.
Displacement is real geometry, affecting silhouettes, shadows, picking, BLAS
and both backends. Scatter on a mesh uses the displaced source surface.

This is CPU preparation on source edits, not adaptive GPU microdisplacement.
Use continuous height maps across UV seams. The browser adapter rejects
subdivision of vertex-colored source data until those colors are baked to a
texture; UV/normal attributes are interpolated, vertex-color extras are not.
The landscape fixture displaces one shared rock prototype before scattering.

Fog-heavy native Live scenes start at one third of output width/height (640×360
tracing for 1920×1080 output), then use MetalFX or the portable temporal filter.
This fixed initial budget avoids mid-drag target/scaler reallocations. The
`STUDIO_LIVE_SCALE` diagnostic override still applies. Refine/Final trace the
requested dimensions. Sky-only scenes keep the existing Live budget.

## Natural landscape content

Existing scenes retain their old defaults. New desktop Terrain content opts into:

- `adaptive:true`: grid resolution follows camera distance and field of view,
  rounded to a power of two between the requested resolution and 256. This is
  chosen during preparation, not a per-ray quadtree. Image heights are resampled;
  procedural heights are evaluated at the selected grid density.
- `detail` (0..0.25): fine geometric noise as a fraction of height. The shared
  shader adds object-space grain and close-range bump; unresolved grain fades
  with the pixel footprint.
- `erosion` (0..64): conservative four-neighbour thermal erosion steps, with a
  34-degree talus. Meshes and scatter sample the same baked grid.
- `natural:true`: slope and curvature mix grass, soil and rock. `water_level`
  adds beach sand; `snow_level` adds snow above the chosen elevation, reduced on
  steep faces. These are ground materials, not generated water geometry.
- `blend` (0..1): ordered authored regions feather over that ground. Height
  feather is amplitude × blend; slope feather is 45 degrees × blend. Region
  roughness and metalness remain effective alongside color.

Rust `nature::Nature {species,seed}` produces three texture-free shared meshes
and two materials. Species are `conifer`, `broadleaf`, `birch`, `bush`, `grass`
and `rock`. Trees have tapered branching wood and folded leaf cards; grass has
curved blades. Leaves use a thin diffuse sheet: 65% reflection and 35% diffuse
transmission, with total energy bounded by leaf albedo. This is translucency,
not a volumetric subsurface random walk. Displaced rocks weld their UV-seam
normals and place their lowest point at the root plane.

A scatter's optional `lods:[[near,medium,far],...]` refers to source-object
indices; each near source also appears in `objects`. Source diameter × instance
scale / camera distance selects full detail at 0.09 or above, medium at 0.02 or
above, and sparse distant cards below. Changing LOD preserves placement, seed,
rotation and scale. There are no billboard image assets. The desktop reuses
placements while the camera moves and updates LOD groups in eight-unit camera
cells; it retains full-detail source models for direct editing. Camera-only
resident native updates reuse their prepared LOD until geometry is prepared
again. Extremely close views can reveal card silhouettes.

Desktop model sources accept `{nature:"birch",seed:41}`. Scatter accepts an
existing model layer as before, or `model:"nature:conifer"` (also broadleaf,
birch, bush, grass and rock). A built-in choice creates/reuses a hidden ordinary
source model and stores its actual layer ID in Scatter. Geometry variation is
`source.seed`; Scatter's seed varies placement. Add Terrain in an empty
composition creates the lit/framed landscape, mixed trees, meadow, stones and
bushes in one Undo. Existing compositions retain their lighting and camera.

Mesh JSON adds optional `colors` (linear RGB per vertex) and
`triangle_materials` (material index per face). Both default to empty. Vertex
tints use the existing packed-normal color channel; GPU buffer strides are
unchanged. `Material.procedural` defaults to zero and occupies material policy
row `.w`: 1 ground, 2 stone, 3 bark, 4 thin leaf, 5 grass, 6 birch bark. Ground
uses object-space mapping; its UV pair carries interpolated roughness and
metalness. Native and portable traversal use the same surface shader.

The Cycles test exporter carries vertex tint, per-face materials and the thin
leaf closure. Its procedural grain/bark/vein texture mapping remains approximate.
The dense forest regression uses 384 Studio / 768 Cycles samples to resolve
fine foliage; its existing ΔE00, SSIM and exposure limits are unchanged.


### Natural-content measurements

Measured 2026-11-09 on the base Apple M5 with 16 GB memory. Other development
lanes were active; clocks and thermals were not locked. The hidden Electron
harness applied 45 small camera/time edits. Live traced 640×360 for 1920×1080
output, with Metal ray queries, shared textures and MetalFX. Timing includes
IPC and a canvas readback that forces pixel completion, not display scanout or
all editor compositing. Refine traced 16 samples at 1920×1080 and used OIDN;
this is a denoised preview, not convergence.

| Fixture | Placements | Live median / p95 | Refine + OIDN |
|---|---:|---:|---:|
| Natural landscape fixture | 18,619 | 30.4 / 33.0 ms | 4.12 s |
| Trees, rocks and grass close-up | 1,507 | 22.8 / 23.3 ms | 2.56 s |
| Forest: 50k trees + ground cover | 77,002 | 33.5 / 34.5 ms | 4.90 s |
| Forest: 100k trees + ground cover | 154,002 | 37.0 / 37.8 ms | 5.73 s |

The 100k forest contains exactly 100,000 tree placements plus 50,000 grass
tufts, 4,000 bushes/rocks, one off-camera source placement and terrain. All
three LODs share 211,878 unique triangles across 19 prototypes. Its prepared
CPU/GPU-upload data was 62.87 MiB, preparation 158.2 ms and device/upload
100.7 ms. Peak native-process RSS during the Electron run was 1,609 MiB;
this excludes Electron and is not total unified GPU memory. These results
show interactive roughly 27 fps in this harness, not sustained full-editor
60 fps. The landscape fixture uses the same ground and nature system as the
new desktop default; the desktop arranges its six scatter layers separately.

At 960×540, 128 samples and OIDN, standalone close-up and 100k forest trace/post
took 2.95 / 6.45 seconds. At 64 samples with denoising disabled, all four
native/WebGPU comparisons passed the existing threshold. Mean RGB code-value
differences were 0.873 / 0.205 / 2.631 / 2.496 out of 255 in table order;
isolated stochastic foliage-edge differences are larger. Seeded Rust/TypeScript
mesh comparisons agreed within 0.00001 scene units for terrain and every
species. Full validation passed 81 Rust tests including all 11 Cycles regression
scenes, 251 Studio/scene desktop tests, 11 environment e2e tests, typecheck,
12 yardstick metric tests and extension boundary/version checks.

Evidence, exact production Add Terrain capture, before/after PNGs and captions
are under `/tmp/pm-shots/studio3d/laneI/`, outside Git. Reproduce with:

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=~/Developer/pm-target CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 cargo test --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
PM_STUDIO_SCENES=env-default-landscape,env-nature-closeup,env-landscape,env-landscape-100k PM_STUDIO_FRAMES=45 PM_STUDIO_SHOTS=/tmp/pm-shots/studio3d/laneI/benchmark POWERMOVE_STUDIO_HOST=apps/desktop/resources/studio/pm-studio-host nice -n 10 node apps/desktop/scripts/studio-native-electron.mjs
nice -n 10 ~/Developer/pm-target/release/studio-environments --scene env-landscape-100k --spp 128 --denoise --out /tmp/pm-shots/studio3d/laneI/after
```
