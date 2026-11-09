# Cycles yardstick (test only)

Cycles is an optional offline reference, never a runtime, build, download,
packaging or app dependency. The normal desktop build compiles/copies only
`pm-studio-host`. The diagnostic `studio-yardstick` binary requires the explicit
`yardstick` feature. No Blender code or binaries are bundled. No `.blend` file is
needed. All generated scripts, scene JSON, EXRs, PNGs and reports go outside Git.

## Run

Blender 4.4+ with Cycles Metal and Python 3 with NumPy/Pillow are required for
comparisons on this Mac. The runner returns success with `SKIP` before importing
optional Python packages when Blender is absent. An installed Blender that fails
to render is a test failure, not a skip. CPU fallback is deliberately disabled.

```sh
python3 engine/tools/cycles-yardstick/run.py
python3 engine/tools/cycles-yardstick/run.py --scenes hero,env-room --size 320x180 --samples 256 --cycles-samples 512
python3 -m unittest discover -s engine/tools/cycles-yardstick -p 'test_*.py'
```

`BLENDER` or `--blender` selects an executable; otherwise the runner checks
`/Applications/Blender.app/Contents/MacOS/Blender` then PATH. Default output is
`/tmp/pm-shots/studio3d/laneE/baseline`. `--out` must be outside the repository.
Each render has a 175-second process timeout, runs with `nice -n 10`, uses two
CPU threads and one Metal GPU, and checks for at least 8 GB free before each
scene. Cargo uses two jobs and `~/Developer/pm-target` by default.

The exported corpus uses the `starlight` finish, time zero (edge uses .125,
since the exact edge-on pose is subpixel at diagnostic sizes), the same generated
512×744 screen texture and expanded procedural geometry. It includes all eight
product scenes and `env-room`, `env-street`, `env-landscape` (50,000 scatter
candidates). The landscape retains shared prototype meshes and exact placement
transforms; no reduction of its population is made for the reference.

For a rerun after engine changes, reuse *unchanged* reference scenes:

```sh
python3 engine/tools/cycles-yardstick/run.py --out /tmp/pm-shots/studio3d/laneE/after --reuse-cycles /tmp/pm-shots/studio3d/laneE/baseline --before /tmp/pm-shots/studio3d/laneE/baseline/scores.json
```

Do not reuse reference renders after changing geometry, materials, lights,
camera, the exporter, or image dimensions. `--score-only` recomputes reports
from existing images. Report paths are `report.md`, `report.html`, aggregate
`scores.json`, and one scene directory with linear EXRs, native Studio PNG,
Cycles PNG, side-by-side PNG, region masks and replay scripts/logs.

## What is measured

Studio runs the shared Final tracer at full requested size, eight bounces,
1,024 samples, clamp 64 and no denoising/adaptive early stopping. Cycles defaults
to 2,048 samples, eight surface/volume bounces and no denoising or clamping.
Both use pixel jitter with a narrow reconstruction filter. This is a quality
comparison, not a timing race; preparation and GPU compilation differ.

`Renderer::linear_beauty` includes directly visible emission, unlike the older
`radiance` diagnostic. Its RGB is scene-linear Rec.709, premultiplied by scene
coverage, before exposure. Alpha excludes Studio's display-only void/catcher.
EXRs preserve these values; scoring also uses lossless float arrays to avoid
requiring a separate EXR library in system Python.

One robust scalar exposure gain is fitted from median log-luminance ratios
over the valid region. It is capped at ±4 stops, recorded in stops and never
fitted per material or RGB channel. Scores include unfitted luminance ratios
so a global energy error cannot disappear behind exposure matching.

Both images receive the scene exposure and the same Khronos PBR Neutral curve
from `post.wgsl` in the scorer. CIEDE2000 uses D65 Lab from these display-linear
Rec.709 values, with mean, p50, p95 and p99. Cycles saves linear EXR with Standard
view settings; neither AgX nor an additional Blender display transform enters
the scores. SSIM uses an 11×11 Gaussian window (sigma 1.5, K1 .01/K2 .03) on
sRGB luminance. RGB MAE and luminance MAE remain scene-linear HDR. Optional
`flip_evaluator` supplies LDR FLIP; otherwise SSIM/MAE are explicitly reported.
The metric tests include published CIEDE2000 test pairs, symmetry and exposure.

Masks use Studio's first-hit IDs, eroded two pixels. They identify metal,
transmission/strong coat, physical floor and sky. With DOF these are diagnostic
nearest sampled hits, not compositing mattes. The shadow mask is the darkest
quartile of the floor: a labeled proxy, not a true isolated direct-shadow AOV.
No pixels in a region produces an explicit unavailable entry, not a zero score.

Product scenes have a display-referred void and a synthetic floor with a
shadow ratio; these are not physical surface radiance. Their EXR scores cover
foreground with both coverages above .98, avoiding fake comparisons against a
lit physical floor. Cycles maps the floor to its shadow catcher (secondary
diffuse reflectance .82) and composites its alpha over the exact backdrop.
The native Studio PNG and Cycles composite are compared separately for the
display floor/shadows and whole-frame display SSIM. Environment EXR scores
cover the whole image. Product and environment mean scores therefore have
different domains and must not be averaged as a single parity number.

## Material mapping

Colors are already linear; do not apply sRGB decoding to numeric material
colors. Geometry is converted to metres in Blender, including instances,
camera focus and volume coefficients, so DOF uses the same physical aperture.
Meshes retain custom vertex normals and UVs; the screen's V coordinate is
flipped for Blender's image-row convention.

| Studio parameter | Cycles Principled mapping | Interpretation / remaining mismatch |
|---|---|---|
| `base` | Base Color | Metal reflection tint / dielectric diffuse tint. Studio multiplies transmission tint at every interface; Cycles tint is its own transmission closure, not measured Beer absorption. |
| `metallic` | Metallic | Same interpolation intent. Studio now uses Kulla-Conty compensation for isotropic, unfilmed metal; Cycles uses its own multiscatter GGX/conductor Fresnel. |
| `roughness` | Roughness | Both square perceptual roughness for GGX. Cycles energy compensation and diffuse layering differ. |
| `ior` | IOR | Both transmission and opaque reflection use this IOR. Studio now uses exact dielectric Fresnel. |
| `specular` | Specular IOR Level = .5 × weight | Unit weight preserves the IOR's physical F0. Studio scales its whole dielectric Fresnel term; Cycles adjusts its dielectric model. Zero removes it. |
| `coat` | Coat Weight | Fixed Coat IOR 1.5, white tint. Final now traces the full coat BSDF; Live retains its one-bounce shortcut. Rough-layer multiple scattering still differs. |
| `coat_roughness` | Coat Roughness | Same nominal roughness; only Live treats values below .05 specially. |
| `transmission` | Transmission Weight | Same share; rough transmission, internal energy, caustics and tint can differ. |
| `emission` | Emission Color × Strength 1 | Scene-linear radiance. Studio now attenuates emitted light by the coat on exit, including camera emission. Rough-coat directional albedo remains approximate. |
| `screen` | Screen image × emission | Same procedural RGBA8 sRGB image, linear-filtered. |
| `sheen` | Sheen Weight, white tint, roughness from base | Studio additive Charlie vs Cycles energy-conserving microfiber closure. A model mismatch and an outstanding Studio energy issue. |
| `film_nm` | Thin Film Thickness, IOR 1.38 when available | Studio three-wavelength cosine modulation differs from Cycles optical interference; always labeled. |
| `dispersion` | Central IOR only | No equivalent Principled RGB dispersion input. Labeled unsupported, not renderer error. |
| `droplets` | No procedural normal mapping | Studio's cap-normal perturbation has no direct Principled parameter; labeled unsupported. |
| `anisotropy` | absolute Anisotropic, rotate .25 turns for negative | Same UV tangent intent; exact distribution stretch can differ. |
| `subsurface` | Subsurface Weight 1, Scale = mean free path in metres, Radius (1,1,1) | Different random-walk/albedo models; not a measured skin/wax equivalence. |

## Lights, sky and media

Softboxes become one-sided emissive meshes with the exact half extents and
`smoothstep(1,.82,max(abs(u),abs(v)))` rim. Surface emission uses radiance
directly, avoiding point/area power conventions. Sphere/spot lights become
small emissive spheres of the same radius/radiance; the spot uses the same
stored direction and smooth cone ramp. Finite sphere tessellation is a mapping
error; nonphysical range/decay settings are explicitly flagged. Emitter meshes
are camera/shadow invisible, matching Studio explicit-light visibility.

Sun angle is twice Studio's radius; irradiance is
`radiance × π × sin(radius)²`, with the same direction and RGB. This avoids
confusing finite-disc radiance with sun irradiance. The small difference
between a uniform solid-angle cone and Cycles' disc remains a sampling detail.

HDRI uses the exact linear pixels, intensity and rotation. Generated sky uses
Studio's actual 128×64 transport map, plus its separately calculated sun.
The Y-up/longitude conversion is explicit. Legacy studio dome gradients are
baked at 256×128, including below-horizon darkening. This validates sky-lit
transport, **not** independent sky-model realism against Cycles' Nishita sky.

Medium scattering is `density × color`; absorption is `density × (1-color)`;
HG anisotropy is copied, including sign. Final shadow transmittance now uses
the exact height/primitive optical-depth integral; free-flight events retain
delta tracking and multiple scattering. Blender's Volume Absorption color
input is complemented by its closure internally, so it receives the *same*
color as Volume Scatter. Box/sphere volume bounds and additive overlaps match.
Global height falloff is a world-position shader. Studio caps each ray segment
at atmosphere.distance; Cycles uses a camera-centered sphere of that radius.
Primary distances agree approximately, secondary/shadow distances differ.
This is a documented mapping limitation, not evidence of an HG transport bug.

Reference documentation: [Principled BSDF](https://docs.blender.org/manual/en/4.4/render/shader_nodes/shader/principled.html),
[Cycles units](https://developer.blender.org/docs/features/cycles/units/),
[Kulla-Conty microfacet energy compensation](https://fpsunflower.github.io/ckulla/data/s2017_pbs_imageworks_slides_v2.pdf),
[CIEDE2000 supplementary data](https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/).

The GGX directional-albedo constants in `trace.wgsl` are generated by
`generate_energy_lut.py`: a 16×16 table with 65,536 deterministic visible-normal
quadrature samples per cell, integrating Studio's actual correlated Smith GGX.
The extra reciprocal lobe restores lost energy and uses Schlick's hemispherical
average for colored-metal absorption. The metric suite checks the embedded
constants against the generator. The same quadrature integrates Schlick's
F0=0 term to estimate dielectric directional reflectance. Diffuse entry/exit
weights use that rough-specular albedo and a reciprocal normalization instead
of smooth Fresnel applied twice without returning internally reflected energy.
The actual dielectric specular evaluation still uses exact Fresnel. This
split-sum layer estimate is approximate, especially for unusual IORs.
Anisotropic and thin-film layers need their own integrals and retain the
previous model; full dielectric/coat microfacet multiscatter is also still a
gap. No reference-renderer code or image lookup textures are embedded.

## Interpretation

The current corpus is procedural and deliberately small in artistic scope.
Matching it is not proof of Blender Cycles realism. Noise, unsupported material
features, global fog bounds, catcher policy, finite light tessellation and
transport implementation must be separated before attributing a difference.
Large ΔE at an unmapped droplet/film/dispersion feature is not grounds to tune
unrelated engine lighting until that score falls.
