# Studio environments

Environment work extends `pm_studio::scene::Scene`; existing serialized scenes
remain valid. All distances and extinction coefficients use the scene's own units
(the older product corpus uses millimetres). Geometry and volume transport run in
the shared WGSL, including portable WebGPU traversal. Metal ray queries accelerate
intersections but do not change the lighting model.

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
  the rendering boundary. Maximum 4096×2048. Visibility affects camera rays
  only; reflections and illumination remain.
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
Surface lighting uses MIS against BSDF sampling. Map lookup is piecewise constant.

Refine/Final use delta tracking for medium events and ratio tracking for shadow
transmittance, HG phase sampling, and multiple volume/surface bounces. Live uses
four jittered single-scattering steps and eight-step shadow transmittance. Both
produce geometry-dependent shafts and aerial perspective. Dense/very large
media can be expensive; there is no empty-space hierarchy yet.

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
| 4 | local-volume offset, count, light-CDF offset, reserved |
| 5 | adaptive enabled, min samples, relative error, absolute error |

Offsets are relative to the header in vec4 rows. Map pixels (RGB,0) precede one
CDF row per pixel (cumulative probability, solid-angle PDF,0,0). Each volume uses
three rows: center XYZ/shape (0 box,1 sphere), half-size XYZ/density,
albedo RGB/g. Light rows contain cumulative selection probability and PMF.
Legacy uploads without a trailer continue to use uniform light selection.

## Validation and measurements

Work in progress. The final measurements and corpus invocation are recorded here
once rendered; screenshots and local logs are kept under
`/tmp/pm-shots/studio3d/laneC/`, outside the public repository.

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
rotation:[0,360],min_height:-1e9,max_height:1e9,max_slope:60,exclusions:[]}`.
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
These are discrete regions, not blended splat materials. Terrain gets smooth
vertex normals. Scatter evaluates the actual piecewise planar mesh grid, so
instances do not hover over an independently evaluated noise function.

`Scene::expanded()` resolves terrain/scatter once. Shared source vertices and
one BLAS per prototype remain resident; the TLAS and nine-vec4 transform rows
scale with placements. Both the standalone renderer and native scene protocol
use this path. Transform patches address placement input order when explicit
instances are present. Maximum one million total placements; no LOD or paging.

## Desktop integration

`StudioSceneDescription.scene` accepts the same optional `world`, `instances`,
`terrains` and `scatters` fields. `compileSceneDescription` resolves procedural
geometry, makes one Three `InstancedMesh` per prototype, and packs the shared
GPU trailer; native prepared uploads and the browser backend consume identical
buffers. Direct graph extraction also handles `THREE.InstancedMesh` without
copying triangles. Changing instance capacity rebases BLAS roots correctly.
Scene-description round trips remove the automatically generated sun before
serializing `world.sky`, so editing a scene cannot duplicate its sun.

Import `StudioWorld` from `core/studio/environment.ts` and the geometry types
from `core/studio/environment-geometry.ts`. `packWorld` and `worldSun` are the
portable preparation helpers for evaluated app data. HDR/EXR decoding stays at
the file-import boundary; `hdri.pixels` carries linear floats, not display PNGs.
`Terrain::load_height_image` is the Rust grayscale decoding/resizing helper.
