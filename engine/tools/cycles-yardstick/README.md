# Reference mapping and transport checks

This directory is an optional, test-only harness. Blender and Mitsuba are never
app, build, packaging, or download dependencies. See
[`docs/studio/cycles-yardstick.md`](../../../docs/studio/cycles-yardstick.md) for
the original corpus and scoring contract. Generated images, scene exports and
reports must stay outside the repository.

The runner now reports every visible material separately, as well as the
existing broad regions. The whole foreground score still includes every
material; unsupported features are not hidden from it.

## Material conversions

- **Anisotropy is a mapping difference.** Studio stores GGX widths
  `roughness² × (1+k, 1-k)`. Principled uses a product-preserving aspect ratio.
  The harness converts both widths and connects the UV tangent explicitly.
  Principled caps the aspect ratio at 10, while Studio's `k=.85` requests 12.33;
  the remaining cap is reported. Negative anisotropy rotates the tangent.
- **Glass tint is a mapping difference.** Principled applies the square root of
  Base Color at each interface. Its input is squared for fully transmissive
  Studio glass, whose tint applies at each interface.
- **Dispersion is a mapping difference plus sampling variance.** Studio has
  three RGB IORs: `ior + dispersion × .04 × (channel-1)`. The harness renders
  three complete channel-specific IOR scenes and retains the corresponding
  channel. This preserves colored reflections and transmission without adding
  channel-selection noise in the reference. It is not spectral Abbe dispersion.
  `cycles-meta.json` records the number of channel renders. Camera wavelength
  sampling in Studio uses randomized three-path strata; it keeps each path's
  uniform marginal and prevents random channel counts from tinting highlights.
- **Thin film contained a renderer error and a mapping difference.** The old
  RGB cosine multiplier did not model interface energy or spectral bandwidth.
  The engine now integrates polarized Airy reflection, using analytic CIE fits
  and complex conductor indices constrained by F0 and Schlick at `acos(1/7)`.
  Principled interprets F82 as an artist edge-tint parameter when deriving its
  film substrate, so the harness explicitly passes Studio's derived complex IOR
  to a physical Metallic BSDF for uncoated, isotropic filmed metals. Filmed
  layering and spectral/Fresnel approximations remain labeled limitations.
  These inferred optical constants are not measured material data.
- **Sheen and subsurface remain different models.** Energy-layered Charlie sheen and
  Studio's simple isotropic subsurface walk are not equivalent to Principled's
  layered microfiber and calibrated BSSRDF. Their regions remain scored.
- **Droplets remain unmapped procedural normals.** Their error is labeled.

The replay script embeds the small pure conversions in `mapping.py`, so it can
be replayed without importing the repository. Reference reuse verifies the
complete embedded script, scene JSON and dimensions.

## Surface photon energy

A three-dimensional normalized stencil is not a surface irradiance estimator.
The photon grid now uses a bilinear tent in the receiver's dominant-axis
projection, whose projected bin represents `cell_size² / abs(normal_major)` of
surface area. Deposited centroids reject neighboring parallel surfaces. The
same eight-word grid layout is retained. The shared WGSL uses half the
hosts' nominal grid spacing to preserve focused peaks; both hosts retain their
existing parameter layout and allocation. Photon emission also includes the
softbox's rim attenuation.

The GPU regression sends light through an IOR=1 sheet. Such a sheet cannot
focus, reflect or tint light, so its receiver must equal the analytic
Lambertian irradiance. Translation and slope cases guard energy and footprint.
Finite cells, coarse/fine blending, surface edges and curved receivers remain
photon-density approximation limits; this pass is not claimed to be unbiased.

The product `caustics` foreground score measures glass/mirror appearance,
**not** the physical caustic landing on the display-only floor. Its catcher
comparison is separate. Use the physical-floor Mitsuba reference in
`engine/crates/pm-studio/reference/compare.py` and analytic sheet test for photon
energy; ordinary Cycles paths under small mesh emitters can have high caustic
variance. MNEE/filter-glossy settings and supported emitter/object combinations
must be verified before treating a Cycles caustic as ground truth.

## Sampling

Final and Refine rigs with at most eight explicit lights sample each emitter
once at a surface or volume event. MIS uses each emitter's conditional density
without a selection PMF in both directions. This removes light-selection
variance; it does more work per path. Live and large light hierarchies retain
single-light sampling. The radiance model and firefly clamp are unchanged.

A four-sample colored-sun test checks direct energy and variance. White-furnace
tests check lossless film energy and neutral dispersion independently of Cycles.
Native ray queries and portable BVH traversal use the same WGSL changes.

Color-matching reference: [Wyman, Sloan and Shirley, JCGT 2(2), 2013](https://jcgt.org/published/0002/02/01/).
The film quadrature constants are reproducible with `generate_film_response.py`.
No Blender or Mitsuba implementation code or rendered lookup images are embedded.

## Shared finite atmosphere

`Atmosphere.distance` now defines a camera-centered sphere for global fog.
Camera, continuation and shadow rays all intersect this same boundary. The
previous per-segment distance budget could change the optical depth of the
same light path after a bounce. Local boxes and spheres use their own bounds;
the global radius no longer clips a separate local volume. Cycles uses the
same sphere, with a small tessellation approximation. Primary miss rays retain
the same maximum fog distance as before. This changes secondary fog lighting
intentionally; saved scene parameters and the GPU upload layout are unchanged.

Analytic absorption tests verify the shared shadow boundary and a local volume
outside the global radius. The former per-segment implementation was 31% too
dark in the isolated test. In the room corpus the raw luminance ratio improved
from .886 to .970, removing most of the exposure discrepancy; stochastic
indirect/volume noise and the diffuse model remain visible.

Narrow coats above rough bases also receive more continuation samples. Only
the proposal probabilities change; BSDF evaluation and the full mixture PDF
retain physical Fresnel weights. An unclamped HDR environment regression uses
four independent 64-sample estimates against a 4,096-sample estimate. Its RMS
noise fell from .1304 to .0843 while aggregate energy stayed within .2%.

## Sheen energy and wavelength correlation

Charlie's directional albedo is integrated from the engine's own closure by
`generate_sheen_lut.py`. A reciprocal layer reserves that energy from the base
before adding sheen. The low-roughness grazing closure is also normalized by
its maximum directional albedo, avoiding negative base weights. Underlying
emission loses the same exiting share. A white-furnace regression improves from
1.1255 to 1.0003, target 1. Reflection plus underlying white emission also sums
to one in an independent oblique-plane check. This is still Charlie, not the
Principled microfiber model.

At the first dispersive interface, the three wavelength strata share random
continuation numbers. Each individual channel retains its original marginal
estimator; the correlation removes neutral RGB speckles. An almost
nondispersive glass under achromatic HDR illumination reduces channel RMS from
.5332 to .0218 at 96 samples; its independently seeded converged mean is within
.15% of ordinary RGB glass. Primary coverage and the wavelength-dependent IOR
are retained.

## Regression coverage

`tolerances.json` now covers all eleven scenes at 128×72, 128 Studio samples
and 256 Cycles samples. Mean color error, SSIM and fitted exposure are bounded
separately. These are low-sample drift alarms with explicit margins, not the
quality target or a claim of parity. Set `PM_STUDIO_YARDSTICK_OUT` when invoking
Cargo tests to direct every optional reference artifact to a chosen external
directory. The runner rejects destinations inside the repository.

`studio-reference` also exports `studio-scene.json`, the complete shared scene
contract, alongside its original Mitsuba description. This lets an older
`studio-yardstick --input` replay exactly the same physical-floor geometry.
