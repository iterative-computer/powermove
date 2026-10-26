# After Effects compatibility work

Status: thirteenth batch, with 43 functioning registrations and bounded
temporal Echo. Channel Combiner revision 2, Color Profile Converter and HDR
Compander are retained. Blend and Calculations
are retained. Stroke revision 2, Apply Color LUT, Solid
Composite and Arithmetic are retained. Explicit source-space rasters,
Displacement Map revision 2 and Texturize are retained. Referenced-video shutters,
Compound Blur, Set Channels and the initial 2D layer-input system are implemented.
Shadow/Ramp revision 2 and narrow PointField controls are verified internally.
This is an implementation backlog, not a claim of complete catalog, control or
pixel parity with any AE release.
The target AE release still needs to be pinned before reference acceptance.

## Evidence and scope

Use Adobe's [effect matrix (updated May 13, 2026)](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/effects-and-animation-presets/effect-list.html)
alongside the category documentation and [release notes](https://helpx.adobe.com/after-effects/desktop/what-s-new/whats-new.html).
The matrix alone does not identify every newer addition. Separate Adobe native
effects from bundled CC/Cycore, Keylight, Mocha and CINEWARE integrations; this
project does not bundle or extract those implementations.

This batch uses the public [color-correction behavior descriptions](https://helpx.adobe.com/after-effects/desktop/adjust-colors/work-with-color-correction-effects/color-correction-effects.html#tint_effect).
All shaders are independent implementations. No licensed AE reference renders
have been compared. Defaults/ranges below describe Powermove's implementation;
they are not evidence that every AE version uses those values.

The versioned [catalog backlog](effects-catalog.json) enumerates all 290 rows of
that matrix plus eleven documented supplements (301 targets), including native,
bundled, obsolete, controller, audio, temporal and 3D entries. Every target has
an implementation status and concrete prerequisite IDs defined in the same file.
Provider/dependency classifications are engineering inferences to review against
category documentation, not proof of Adobe internals. Contour Paint is confirmed by the official Stylize category page and remains
explicitly tracked with its analysis, layer-input and editable-flow prerequisites.
The baseline is documentation-dated, not an exhaustive installed-release audit.

## Inventory and parity matrix

The original 16 definitions are in `src/extensions/effects-basic/effects.ts`.
Every entry is an implemented GLSL ES 3.00 shader, not a catalog placeholder.
Numbers below are defaults; px and percent are literal parameter units.
The original revision-1 definitions remain unchanged at the data level (shader
source, parameters, order, defaults and viewport declarations). A regression
digest protects that saved-project baseline. Newly created Blur, Mosaic, Levels and Invert
instances now use revision 2; see the second-batch table below.

| Family / ID | Current controls and defaults | Passes | Compatibility work remaining |
| --- | --- | --- | --- |
| Blur & Sharpen / `blur` revision 1 | amount 24px (0–400) | 2 | Gaussian kernel bounded to 57 taps per axis; no dimension selector or explicit edge mode. Validate radius at preview scale and layer/comp bounds. |
| Blur & Sharpen / `motionblurDir` | amount 30px (0–300), angle 0° | 1 | 17-sample directional box kernel. Validate angle convention, length and edge semantics. This is not optical-flow motion blur. |
| Blur & Sharpen / `sharpen` | amount 40% (0–300) | 1 | Four-neighbor sharpening, RGB clamped 0–4. Verify operator and HDR/alpha behavior. |
| Light & Shadow / `glow` | threshold 55%, radius 60px, intensity 90% | 3 | Soft luminance threshold plus two-axis blur added to original. Missing channel basis, operation/compositing/color modes and alpha/HDR calibration. |
| Light & Shadow / `vignette` | amount 45%, feather 60% | 1 | Composition-centered radial darkening. No independent reference equivalence established. |
| Light & Shadow / `shadow` | distance 20px, angle 135°, softness 40px, opacity 60%, color black | 3 | Offset alpha plus Gaussian-like blur, composited beneath original. Missing shadow-only control and verified angle/bounds/zero-softness behavior. |
| Color / `color` | exposure 0EV, contrast 0, saturation 0, hue 0°, temperature 0 | 1 | Combined convenience operator, not separate AE controls. Linear RGB/HSV approximations; clips 0–8. Keep old behavior while adding documented individual effects. |
| Color / `levels` revision 1 | inBlack 0, inWhite 100, gamma 1 | 1 | RGB master only; no output levels, per-channel controls, histogram or clipping modes. |
| Color / `duotone` | shadow #1B2A4A, highlight #FFD66B, amount 100% | 1 | Smoothstep luminance mapping, not linear Tint; retained for old projects. |
| Color / `posterize` | levels 6 (2–64) | 1 | Rounded channel quantization. Validate level-count definition and integer/HDR handling. |
| Color / `invert` revision 1 | amount 100% | 1 | RGB inversion only. Missing channel/color-space selector and independently exposed blend-with-original semantics. |
| Stylize / `grain` | amount 12%, size 1.4 | 1 | Uniform hash noise; time repeats every second. No film model, channel controls, seed, animation controls or grain matching/analysis. |
| Stylize / `pixelate` revision 1 | size 16px (1–200) | 1 | Samples a square cell center, not cell average. Missing independent horizontal/vertical block counts and sharp-color controls. |
| Distort / `chroma` | amount 6px (0–80) | 1 | Radial red/blue offsets; alpha follows center green sample. Check premultiplication at transparent boundaries. |
| Distort / `displace` | amount 24px, scale 2.2, speed .4 | 1 | Five-octave value-noise warp drifting with time. Evolution is a speed, not an explicit phase channel. Missing displacement modes, pinning, complexity, seeds/cycles. |
| Generate / `gradient` | white→black, angle 90°, center 0/0%, spread 100%, midpoint 50%, radial false, shade false, dither 1, amount 100% | 1 | Layer-attached alpha-preserving ramp. Missing independent start/end points and reference blend/scatter semantics. |
| Color / `tint` (new) | Map Black To #000000, Map White To #FFFFFF, Amount to Tint 100% (0–100) | 1 | Linear luminance map and original mix implemented. Native Swap Colors implemented; calibrated luminance/color space, exact ranges and AE reference comparisons pending. |
| Color / `channelMixer` (new) | Twelve coefficients/constants in output-channel order: identity diagonal 100%, other terms 0%; Monochrome false. All numeric controls −200–200%. | 1 | Documented channel contributions, constants and red-output monochrome implemented. Native Red/Green/Blue disclosures implemented; exact release defaults/ranges not reference-verified. Output clamps to 0–1, including float projects. |

Tint and Channel Mixer are registered in the existing Color family and use
ordinary independently keyframeable Inspector fields. They preserve source
alpha, operate on straight RGB and restore premultiplication. Tint intentionally
has no hard RGB clamp in float targets. Channel Mixer follows the documented
bounded-color operation; its AE documentation lists 8/16 bpc, so Powermove's
32-bit execution must not be described as AE 32-bit parity.

## Second batch: revisions and core image operators

| Effect | New behavior and controls | Remaining reference gaps |
| --- | --- | --- |
| Invert revision 2 | Channel selects RGB, Red, Green, Blue, HLS, Hue, Lightness, Saturation, YIQ, Luminance, In Phase Chrominance, Quadrature Chrominance or Alpha. Blend With Original defaults to 0%. | Exact HLS/YIQ/gamut conventions and release control ranges need AE golden comparisons. |
| Levels revision 2 | RGB master and independent Red/Green/Blue/Alpha groups each provide Input Black 0, Input White 1, Gamma 1, Output Black 0 and Output White 1. Global lower/upper output clipping defaults on. Clipping can be disabled for signed/HDR extrapolation. | Uses normalized numeric controls and disclosures. Histogram, selected-channel interaction and release-specific scaling/defaults remain to implement/verify. |
| Levels (Individual Controls) | Same 25 independent channels and two clipping toggles, with native channel groups. | Exact AE presentation, transfer conventions and reference validation remain open. |
| Exposure | Master and Red/Green/Blue groups each provide Exposure 0 stops, Offset 0 and Gamma Correction 1. Bypass Linear Light Conversion defaults off. Signed gamma preserves the sign of negative float values. | Conversion currently assumes sRGB transfer, rather than a declared ICC/OCIO working space. Exact offset order/defaults/ranges require comparison. |
| Shift Channels | Four independent source selectors (Alpha, Red, Green, Blue), including luminance, HLS and full-on/off sources. Defaults preserve RGBA. | Check source-space and alpha-interpretation semantics against AE. |
| Threshold | Luminance threshold with local Level 128, range 0–255. | Check luminance weights, equality boundary and release control ranges. |

These are functioning independent implementations, not placeholder catalog entries.
All six operators use one image pass. Alpha-creating modes restrict output to the
transformed source bounds using the existing layer coordinate contract; RGB is
unpremultiplied before operations and multiplied by the resulting alpha afterward.
Tint now exposes a native **Swap Colors** action at the playhead. Both swaps are
one source transaction, so one Undo restores the colors; animated channels retain
their tracks. Parameter `group` metadata uses the existing native disclosure control
and does not change channel paths.

### Revision and migration contract

`EffectDefinition.version` defaults to 1. Historical implementations live in its
`legacy` array as complete definitions with the same ID and explicit, distinct,
older version numbers. Nested histories and invalid versions are rejected.
`effects.get(id, version)` resolves an explicit historical implementation; omitting
the argument returns the latest catalog definition.

New instances persist the current version. Unversioned saved source migrates to
version 1, preserving the old units, defaults, shader and parameter animation.
The renderer, Inspector, timeline metadata, high-zoom eligibility, clipboard and
portable web export resolve that same version. A newer unavailable revision stays
missing with its data intact, rather than being rendered through an older shader.
Current and historical programs have separate GPU keys and all revisions are
invalidated when their provider reloads. Portable export bundles historical
implementations as plain data with the latest definition.

The typed project sanitizer also preserves valid extension effect IDs and missing
providers, removing the old hard-coded 16-name whitelist. This is essential for
persisting an expanding effect catalog.

Native asynchronous and retained-static export now resolve each selected revision.
Snapshots watch live source versions, historical implementations, provider registration
identity, shader source, parameter/default changes and reviewed sampling/grid contracts.
A revision switch during a queued GPU read or encoder acknowledgement discards the
stale capture and recaptures that timestamp. Stationary checks inspect the exact
linked historical or current program, rather than the newest catalog ID alone.

Separate transition registrations live in `transitions-basic/transitions.ts`:
Crossfade (no params); Wipe (angle 0°, softness 8%); Slide (direction 0°); Zoom
(amount 25%). These combine the lower stack and incoming layer through
`u_from/u_to/u_prog`; they are not evidence of AE transition-effect coverage.

## Third batch: spatial operators and export revisions

There are now 24 functioning registrations. The original sixteen data definitions
remain intact; the new Blur/Mosaic definitions retain explicit revision-1 histories.

| Effect | Current controls and implementation | Remaining reference gaps |
| --- | --- | --- |
| Gaussian Blur revision 2 | Blurriness 0px (0–400), Both/Horizontal/Vertical dimensions, Repeat Edge Pixels false. Two premultiplied Gaussian passes; filtered mip levels bound wide kernels to 65 taps per axis. Radius follows render density. | Sigma/radius mapping and mip-filtered approximation need AE calibration; no exact AE pixel equivalence is claimed. Off-frame source growth is limited by composition rasterization. |
| Mosaic revision 2 | Horizontal/Vertical Blocks 10 each (1–2048), Sharp Colors false. A layer-local cell atlas computes centers or premultiplied averages once per cell, then maps cells onto transformed layer bounds. | Bounded stratified/mip-assisted averaging is approximate for arbitrary transformed cells. Best-quality edge antialiasing is not implemented. Cropped viewport rendering is conservatively disabled. |
| Unsharp Mask | Amount 50% (0–500), Radius 1px (0–400), Threshold 0 (0–255). Two Gaussian passes, thresholded straight-RGB difference, original alpha. Amount/radius zero bypass; signed/HDR output is unclamped. | Exact AE kernel, threshold metric and transfer-space behavior need reference renders. |
| Find Edges | Invert false, Blend With Original 0%. RGB Sobel magnitude, white background by default; original alpha. Explicit one-composition-pixel crop support. | Independent Sobel implementation; exact AE operator unverified. AE documentation lists 8-bpc support; Powermove's float execution is an extension, not proof of AE float parity. |

Filtered mip allocations count toward the framebuffer pool's existing memory
budget, including retained levels after reuse. Each pass restores base-level
filtering and maximum mip level before returning the texture to the pool. Grid
atlases use independently bounded dimensions and the same working precision.
Chained and containing-group sample support is summed; fixed neighborhoods and
parameter support factors participate in the viewport overscan budget.

Dedicated GPU tests compare Gaussian output with independent convolution math,
Mosaic output with source-cell averages, and sharpening/edge output with neutral,
threshold and alpha expectations. They cover available 8/16/32-bit targets,
scaled output, transformed bounds, signed HDR, mip allocation/sampler restoration,
old/new hydration and high-zoom crop/full-frame agreement. These remain internal
behavior checks, not licensed AE golden comparisons.

## Fourth batch: tonal color operators

There are 28 functioning registrations. Four additive operators follow Adobe's
[public Color Correction descriptions](https://helpx.adobe.com/after-effects/desktop/adjust-colors/work-with-color-correction-effects/color-correction-effects.html).
They do not alter the original combined Color or Duotone effects. All are
one-pass, independently keyframeable and preserve source alpha. The table records
Powermove defaults, not verified release-specific AE defaults.

| Effect | Controls/defaults and independent behavior | Remaining reference gaps |
| --- | --- | --- |
| Tritone | Highlights white, Midtones #7F5F3F, Shadows black, Blend With Original 0%. Piecewise linear luminance mapping through three colors. | Luminance/transfer convention, midpoint interpolation, defaults/ranges and HDR extrapolation need AE reference calibration. |
| Color Balance (HLS) | Hue 0°, Lightness 0%, Saturation 0%. HSL rotation, lightness endpoint interpolation and saturation scaling. Neutral settings preserve signed/HDR input exactly. | Adobe documents this as a legacy compatibility effect. Non-neutral processing uses bounded HSL; exact AE adjustment curves remain unverified. |
| Brightness & Contrast | Brightness 0 (−150–150), Contrast 0 (−100–100), Use Legacy false. Legacy affine adjustment; modern endpoint brightness and midpoint contrast. Neutral output is exact, including signed/HDR. | Both are independent approximations. Exact modern/legacy AE transfer curves and ranges need golden comparisons. |
| Black & White | Reds 40%, Yellows 60%, Greens 40%, Cyans 60%, Blues 20%, Magentas 80%; Tint false; Tint Color #D7B68C. Six hue-sector weights in a native Color Weights disclosure; optional HSL tint. | Hue-sector and tint algorithms, defaults/ranges and source working space need AE calibration. Adobe lists 8/16-bpc; float execution is a Powermove extension. |

GPU fixtures cover endpoints, hue rotation, neutral/negative saturation, brightness
modes, hue-sector weights, tint, alpha, transparent bounds and neutral signed/HDR.
Animation and ordered stacks survive actual project hydration; native controls
are checked in both themes. Reviewed native export schemas include these operators.

## Fifth batch: Shadow and Ramp revision 2

[Drop Shadow](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/perspective-effects.html)
now exposes Shadow Color black, Opacity 50%, Direction 135°, Distance 5px,
Softness 0px and Shadow Only false. Direction uses 0° up and 90° right in layer
coordinates; the layer transform carries both the offset and Gaussian softness.
Zero softness is an exact alpha offset, without a minimum blur. Shadow Only
outputs premultiplied shadow color/alpha; ordinary output composites it behind
the original. Filtered sampling starts on the first blur pass, with projected
layer scale and accumulated viewport support. The independent softness kernel,
subpixel filtering and defaults still need AE reference calibration. Off-composition
source pixels remain unavailable after the existing content rasterization.

[Gradient Ramp](https://helpx.adobe.com/after-effects/using/generate-effects.html)
now has independent Start Of Ramp and End Of Ramp pixel points, black/white
endpoint colors, Linear/Radial Ramp, Ramp Scatter 0 and Blend With Original 0%.
Initial points are source-width/2 at the top and bottom of the source. Points
remain saved absolute layer pixels through transforms and resizing. Coincident
points deterministically use the start color. Scatter is static in layer pixels;
its distribution and the exact release defaults remain uncalibrated. Source alpha
is retained. Native point fields, color fields and one-Undo Swap Colors are reused.

Numeric `defaultFrom: { dimension: 'width'|'height', factor }` metadata supplies
source-sized creation and fallback values while persisting ordinary numeric
channels. The public pure `effects.parameterDefault(param, size)` helper also
provides native reset values. Creation through source commands receives the
actual target layer; hydration uses the containing composition and saved source
size. Older Ramp and Shadow instances retain full revision-1 definitions and
controls; unversioned files still select revision 1.

Both current revisions have reviewed asynchronous/static export contracts.
Queued snapshots also watch contextual-default metadata. The built-in effects
require the new default helper before activation, so older hosts fail explicitly.

## Sixth batch: typed layer inputs and real map effects

There are now 30 functioning registrations. `setMatte` and `displacementMap` are
independent shaders based on Adobe's public
[Channel](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/channel-effects.html)
and [Distort](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/distort-effects.html)
descriptions. Both are deliberately marked partial in the catalog. A pinned AE
release and licensed reference frames remain outstanding.

Set Matte provides a layer/stage picker, channel selection (RGBA, luminance and
HLS), inversion, centered/stretch mapping, original-alpha compositing and optional
map-alpha premultiplication. Displacement Map provides the layer/stage picker,
independent horizontal/vertical channel and pixel maxima, centered/stretch/tile
mapping, wrapping and output expansion within available composition pixels.
None preserves the input. Straight color operations restore premultiplied output;
float color remains signed/HDR rather than being clamped to 8-bit values.

[Layer-input implementation notes](effect-layer-inputs.md) define the persisted
contract, iterative phase graph, bounded render-local cache, visibility/time and
coordinate semantics, missing-reference recovery, command/copy integration and
native/portable export behavior. Unsupported direct 3D/adjustment inputs and
Effects-stage inputs requiring an external backdrop fail explicitly. General
native export is supported; static/retained/asynchronous optimizations stay off.
No placeholder catalog registrations were added.

Internal GPU checks distinguish Source/Masks/Effects, hidden references, matte
stretch/inversion and displacement boundaries, across available 8/16/32-bit targets.
Tests also check HDR/alpha, native point/input controls in both themes, old/new
Shadow/Ramp hydration, copied/deleted/missing references, reverse hidden video,
native bytes, an independent portable player and encoder-ACK mutations. These
are behavioral tests, not evidence of Adobe pixel parity. The remaining catalog
still requires the staged systems and acceptance matrix below.

## Seventh batch: exact shutter inputs, Compound Blur and Set Channels

There are now 32 functioning registrations. The provisional documentation baseline
is pinned in `effects-catalog.json`: the 2026-05-13 public Adobe matrix plus the
Channel and Blur & Sharpen descriptions retrieved 2026-10-17. The installed-release
target remains unverified; lack of licensed golden fixtures does not block these
independent implementations.

Compound Blur exposes Blur Layer/stage, Maximum Blur (20px default, 0–400px),
Stretch Map (Stretch Map To Fit) and Invert Blur. A per-output 2D Gaussian gather uses map
luminance to select sigma; wide kernels use budgeted mip-assisted sampling.
Set Channels copies four separately selected layer/stage sources into output
RGBA, with Alpha/RGB/Luminance/Full On/Full Off choices and native channel groups.
Both retain premultiplied alpha, signed/HDR source color, ordinary parameter
animation, static stable layer selectors and saved data. Reference coordinate,
working-space, complete control/default and exact AE kernel parity remain open.

Capture preparation now follows actual source/effect/group/precomp traversal and
shares exact shutter centers with rendering. Hidden video inputs are prepared
at every sampled time; precomp instance paths distinguish shared nested IDs.
Export/player preparation uses its actual sample count and shutter width, including
nested policies and negative shutter ordering. Missing planned frames fail rather
than reading a shared decoder's last position. The live viewer uses bounded private
decoders and immutable subframes, sharing the preview cap and cancelling on
pressure, edits and context changes. Its presented time follows the completed
invocation. This implements render shutter
sampling, not arbitrary temporal effects or optical-flow frame synthesis.

## Rendering and extension contract

- `EffectDefinition` in `src/renderer/src/kernel/api.ts` accepts numeric, RGB
  color and toggle channels, with numeric/toggle choices and point/ramp hints,
  plus static typed layer, mask/path, decoded image and bounded standalone `.cube`
  LUT asset references. Editable curves, OCIO/ICC configuration and analysis
  results still need their own contracts. Color uniforms are vec3, with no independent color alpha.
- `kernel/glsl.ts` validates IDs, unique parameter keys, nonempty shader source
  up to 65,536 characters and 1–8 passes. Although an API comment mentions a
  32-parameter limit, current parameter validation does not enforce that cap.
- `legacy/gl/compositor.ts` runs each enabled effect in stack order against a
  full render-target texture. Ordinary passes obtain pooled targets at the same
  size and working precision; grid effects first use a bounded cell atlas; optional inputs are the pre-effect original and
  accumulated backdrop. Each effect chain remains serial; selected-layer inputs use the explicit
  phase DAG described above.
- Masks run before effects, track mattes after effects, and compositing after
  both. Adjustment layers process the accumulated lower stack; groups create
  their own compositing boundary. Ordinary layer transform motion blur samples
  content before applying the effect chain; it is not a temporal effect API.
- The compositor uses WebGL2 with premultiplied RGBA8/16F/32F targets and
  clamp-to-edge sampling. Precision availability is hardware-dependent, with
  fallbacks in `core/color-depth.ts`. More float bits do not establish a matching
  working color space, profile, transfer function or HDR algorithm.
- Crop-safe effects declare numeric support keys, optional support factors and
  fixed pixel support. The viewer sums enabled stack and containing-group support.
  Mosaic revision 2 uses the full-frame path. Test small targets, off-frame sources
  and quality/zoom changes before promising AE bounds equivalence.
- Effect instances now persist an implementation version; historical definitions
  and migration rules are described above. Missing providers/revisions retain
  channels and later recover. Hydration adds defaults from the selected revision,
  not from the newest shader.
- `legacy/core/export-effects.ts` duplicates known effect schemas for safe
  asynchronous export eligibility. Reviewed schemas include the 28 earlier operators and retained revisions;
  the six reference operators and source owners deliberately use the general export path.
  Static export accepts the stationary built-ins and checks linked time uniforms;
  `grain` and `displace` stay per-frame. A new effect without reviewed eligibility
  should remain on the general export path.
- Portable web export embeds effect definitions and the common player renderer.
  Changes must preserve native-preview/native-export/web-player behavior.
- Built-in `effects-basic` is 1.12.0 and viewer is 1.0.17 after this batch.
  Inspector is 1.7.0 for typed layer/stage controls; timeline retains 1.1.7.
  Refresh their `.builtin-versions.json` entries after verification.
  Manifest API version remains 1; the effects extension explicitly checks
  `effects.supportsVersions`, `effects.supportsSpatialSampling` and
  `effects.supportsLayerInputs` and `effects.supportsInputSubframes` plus `effects.supportsSourceRasters` before registering,
  because older renderers cannot safely interpret the revision and sampling metadata.

## Staged implementation

1. Additive documented color operators, export schemas, control/default tests,
   real GPU arithmetic fixtures and serialized-animation coverage (implemented).
   Preserve the original 16 render contracts and existing project artwork.
2. Revision infrastructure and the first Levels/Invert refinements are implemented.
   Blur/Mosaic revisions are also implemented. Shadow/Ramp revisions are implemented; calibrate each against a pinned AE release. Check scope/control parity before advertising a match.
3. Add single-frame pixel and spatial operators in coherent families, with
   explicit edge/padding/bounds, alpha, precision and quality-scale contracts.
   Introduce shared kernel primitives only when multiple reviewed effects need them.
4. Initial typed 2D layer inputs, a phase dependency graph, Set Matte and
   Displacement Map are implemented. Extend this to external-backdrop/3D inputs,
   typed paths/assets, further channel matte and layer-driven light/texture
   inputs. Preserve explicit diagnostics for unsupported stages.
5. Add deterministic random-access frame requests, time remapping, temporal
   cache invalidation and video decode preparation for Echo, temporal color
   stabilization, frame blending, time displacement and optical-flow operations.
6. Implement analysis/stateful systems separately: tracking/stabilization,
   segmentation/rotoscoping, simulation/particles and audio DSP (live + offline).
   Existing editable 3D layers do not expose scene depth, normals, IDs, camera or
   arbitrary scene access to effect shaders; add explicit passes for 3D-dependent
   effects. Treat OCIO/LUT operations as color-management infrastructure, not RGB
   shader look-alikes.
7. Track bundled third-party behaviors separately and evaluate independent
   functional equivalents and licensing requirements. Never register a no-op
   effect to inflate catalog coverage.

## Acceptance matrix

For every effect, pin the source documentation and target AE release; record
parameters in order, labels/groups, units/defaults/ranges/enums, animatability,
reset/migration behavior and any unsupported control. Compare reference renders
using solid primaries, gray ramps, impulses, checkerboards, gradients, transparent
and antialiased edges, negative/HDR RGB, non-square pixels, off-frame content,
rotated/scaled/parented layers and precomps. Exercise zero/extreme values, 8/16/32
bit targets, zoom/quality/crops, masks/mattes/adjustments/groups, effect ordering,
keyframes/expressions, fractional time and deterministic seeds. Round-trip saved
projects, copy/paste/Undo and missing-provider reloads. Verify native frame
export, static/async paths, portable player, cold/warm compilation and context
loss. Temporal/analysis effects additionally require seek-order independence,
trimmed/reversed footage, cache invalidation, cancellation and memory budgets.

The focused color-correction E2E tests compare independent arithmetic expectations
with real GPU output; they are not AE golden-image comparisons. Existing opaque
export cases also exercise both additions. Inspect light/dark native controls and
capture representative pixels before broadening the batch.

## Unrelated fixture failures observed during this batch

The broader `contracts-locks.test.ts` outcome matrix omits `remove_marker`, which
was already registered by the timeline-marker work. `FxBrowserPanel.test.ts`
expects six panels although the existing Easing work registers a seventh. Those
fixtures and their underlying marker/Easing behavior were left unchanged.

The broader `video-frame-integrity.spec.ts` repeated-cut check also expects a
nonblack picture at work-area end 1.5s. Its final clip ends at 1.5s while the
composition duration is 3s; the existing activity rule excludes that clip-end
frame (only composition-end is inclusive). This no-input scene uses the legacy
preparation path. Work-area endpoint policy is separate from layer inputs and
was left unchanged. These failures are not a claim that the complete repository
suite is green.

## Eighth batch: unclipped input sources and relief

There are now 33 functioning registrations. Input Space is an additive static
reference field: Composition retains historical rendering; Layer Source keeps the
selected native content extent before transforms and opacity. Source-local masks,
effects and nested inputs are supported, with explicit limits for groups, external
mattes/backdrops and 3D. This fixes off-comp map clipping for the layer-driven
operators. Source-sized owner chains and effect-bound growth are still prerequisites
for complete off-comp displacement/blur parity. See the exact contract and acceptance
cases in `effect-layer-inputs.md`.

Displacement Map revision 2 adds precision-aware neutral mapping while retaining
revision 1 unchanged. Texturize provides independent luminance-gradient surface
lighting, native controls, alpha/HDR preservation and real native/portable delivery.
The provisional documentation baseline is extended by the public Stylize category
retrieved 2026-10-17; no licensed AE golden-frame acceptance is claimed.

Next: extend source-sized processing to effect owners and declared expansion,
then typed mask/path inputs for a consuming stroke/path operator, and typed asset
references for real LUT/texture operators. Those must include history, copy/remap,
deletion, hydration and native/portable export rather than only picker metadata.

## Ninth batch: source owner processing and real typed consumers

At this checkpoint there were 34 registrations. Explicit owner Effect Space preserves
historical Composition behavior while Layer Source runs a supported 2D chain
before transforms, with declared progressive bounds growth. It recovers owner
pixels lost by early composition clipping, including through displacement,
blur/shadow and shutter samples. Backdrops, 3D/group owners and undeclared
historical/third-party revisions remain explicit limits.

Typed mask/path and image-asset references are implemented and proven by Stroke
and Texturize revision 2. They include native controls, animation of consuming
parameters/geometry, stable serialization/copy/deletion recovery, durable asset
packaging and actual native/isolated player output. Stroke revision 1 supplied hard-edged antialiased geometry; revision 2 below
extends its brush, spacing, sequence controls and bounds without mutating old data. Texturize's direct asset input is an independent
extension; exact AE control, working-space, bounds, kernel and lighting calibration
is still unverified. See `effect-layer-inputs.md` for the exact supported contract.

Next milestones: improve the remaining bounded geometry controls and coordinate
calibration, add more real path/layer operators, then typed LUT/config formats and
temporal/analysis systems with deterministic invalidation and exports. No current
registration is a no-op placeholder.

## Tenth batch: fuller Stroke, durable LUTs and Channel operators

There are now 37 functioning registrations. Stroke revision 2 adds hardness,
spacing, independently animated Start/End, All Masks and length-based sequencing.
Conservative path control hulls grow source-owner bounds progressively; distant
referenced geometry survives before final composition clipping. Soft/dab raster
work and geometry are explicitly bounded. Revision 1 remains unchanged. Exact
AE hardness kernels, mask sequencing, curve flattening and bounds calibration
remain unverified.

Apply Color LUT consumes a genuine typed durable `.cube` asset, using 1D linear
or 3D trilinear interpolation, per-channel domains, unchanged alpha, an Amount
control and float table/output handling. Native file selection, missing IDs,
copy/save, replacement Undo, cache eviction, media reload and isolated player
restoration are tested. Supported limits: standalone 1D up to 4096 samples,
3D equal axes up to 64, 16 MiB text, finite values ±65504, actual GPU dimensions.
Combined shaper+3D tables and .3dl/.look/.csp/OCIO/ICC formats remain open. Input
outside the declared domain clamps; exact AE overrange/interpolation behavior is
unverified. This is not a working/input/output color-management pipeline.

Solid Composite adds a real bounded solid behind the source, source/solid opacity
and Normal/Multiply/Screen/Overlay/Darken/Lighten/Difference/Add/Subtract. Arithmetic
adds Add/Subtract/Multiply/Difference/Max/Min/Block Above/Block Below/Screen/And/Or/Xor,
per-channel values, clipping and byte wrapping, preserving alpha. Slice, the
complete blend menu and AE operator calibration remain open. Float-working-buffer behavior beyond documented 8-bit Arithmetic is an
independent extension; clipping/wrapping still bounds its output.

Internal acceptance includes numeric GPU checks, source alpha/edges/HDR, geometry
outside owner bounds, all-mask trimming/spacing, legacy round trips, asset
replacement rejection/Undo, resource recovery, native Inspector editing in both
themes, actual H264 decoding and byte-comparable isolated portable playback.
The full 301-target backlog remains open; no exact AE visual parity is asserted.
Next coherent families are additional Channel layer consumers (Blend/Calculations),
remaining LUT formats/color-management declarations, fuller generators/path
consumers and then temporal/analysis infrastructure. Reference captures against a
pinned licensed AE release are still required for parity acceptance.

## Eleventh batch: real layer compositing and channel extraction

Forty effects are registered. Blend provides all five publicly documented modes,
an animated Blend With Original control and centered/stretched input placement.
Crossfade handles both premultiplied colors and alpha; other modes preserve owner
alpha, with independent HLS tint/color transfer or signed light/dark math.
Calculations extracts RGBA/gray/R/G/B/alpha on both inputs, inverts before
extraction, controls secondary opacity, supports nine common modes and optionally
preserves original transparency. Channel Combiner adds real scalar extraction and
RGB/alpha/HLS display outputs, output inversion and Solid Alpha. Source Layer
None selects the owner; broader multichannel conversion menus and a separate Use
2nd Layer switch remain open. Exact AE menus/defaults/working-space/HLS/alpha
weighting and pixel parity are unverified.

All three are one-pass operators on the existing layer/stage/space scheduler,
including hidden video, masks/effects, source owners, cancellation and memory
limits. The tests cover numeric modes, alpha, signed HDR, tiny positive alpha,
8/16/32-bit consistency, placement/transforms, copy/save, deletion Undo, cycle
rollback and framebuffer recovery. Native Inspector edits/Undo and light/dark
layouts are checked. A real hidden-video chain is compared with decoded native
H264 and isolated portable playback. No new cache or analysis job was introduced.

Next: complete remaining Channel conversion/menu variants using documented
semantics and licensed reference captures; then introduce reusable declared color
spaces/transfer functions for further LUT/color consumers, followed by a bounded
fractional-time source request API for genuine temporal effects. Keep the full
301-target catalog and reference-calibration work open. This checkpoint is not
full After Effects parity or completion of the user's full-catalog request.

The eleventh checkpoint also fixes collapsed-owner fallback coordinates for the
new compositing consumers, without changing historical shader revisions. An
idle-cost regression verifies lazy GPU compilation and zero added GPU work for
unused registrations or statically disabled stacks. Static disabled references
no longer select expensive dependency/video preparation paths. Previously used
programs and surfaces may remain cached within existing resource policies; saved
LUT/media assets retain their separate disk/RAM costs. Packaging excludes built-in
test/spec source files. See the resource contract in `effect-layer-inputs.md`.

## Twelfth batch: channel representations and explicit D65 color transforms

Channel Combiner revision 2 appends twelve real multichannel conversions:
RGB↔HLS, RGB↔HSV, RGB↔YIQ, RGB↔YUV, linear sRGB↔XYZ (D65), and RGB↔YCbCr (709).
These conversions bypass the scalar To control, retain owner alpha (unless Solid
Alpha is enabled), and apply output inversion after conversion. Revision 1 and
its original eleven scalar choices remain unchanged for saved projects. Signed
chroma/XYZ/HDR require floating buffers; 8-bit intermediate storage can clip them.
HLS uses bounded unit-gamut values; HSV retains a nonnegative HDR value component.
No automatic transfer/profile conversion is implicit in representation changes.

A shared independent math module derives D65 RGB/XYZ matrices from published
chromaticities and supplies signed sRGB/P3 transfer functions and Rec.2020 display
gamma 2.4. Color Profile Converter is a real explicit transform among those three
named profiles, with Linear Input/Output switches to interpret or emit linear
RGB, optional output clipping and Amount. Defaults are exact identity. It preserves premultiplied alpha and signed
HDR when clipping is off. HDR Compander is a real signed power compression and
reciprocal expansion with Gain/Gamma; Gain=Gamma=1 is exact identity.

This is fixed-profile effect infrastructure, not full project/import/export color
management or an ICC/OCIO implementation. There are no profile files, dependencies
or background jobs. ICC files, rendering intents, black-point/scene compensation,
white-point adaptation and Project Working Space selection remain open. The
compander curve, all AE controls/defaults/menus and exact AE pixels remain
reference-unverified. Public guidance: Adobe's Utility/Channel descriptions,
W3C CSS Color 4 conversion definitions and standardized RGB chromaticities.

Acceptance covers reference matrix/transfer values, all conversion pairs,
very dark chromatic HLS/HSV round trips, historical revision hydration, signed
HDR/alpha/edges, clipping, source processing,
Undo/copy/save and resource recovery. Native controls in both themes, decoded
H264 and byte-comparable isolated portable playback are tested. The 42-definition
idle fixture retains zero effect GPU allocation/compilation for unapplied or
statically disabled effects. Bundle growth is tracked separately from source,
media assets and unmeasured whole-app/package deltas.

Next concrete batch: temporal input requests with timestamped dependency keys,
per-request transforms/stages and an effect-prefix boundary. Echo-like consumers
must request the owner before the consuming effect at historical timestamps;
feeding the complete Effects stage back into itself would recurse. Extend exact
video preparation/private decoder lifetime, cancellation and frame budgets to
those requests, then prove bounded real Echo/temporal consumers and native/player
seeks. None of that temporal contract is claimed implemented in this color batch.
The full 301-target catalog remains open.

## Thirteenth batch: temporal requests and Echo

Forty-three real effects are registered. Echo consumes bounded immutable owner
prefixes at fractional times, with real Add, Maximum, Minimum, Screen, Composite
In Back, Composite In Front and Blend operators. Its native controls are Echo
Time (-1/30 s), Echoes (1 additional frame, maximum 32), Starting Intensity (1), Decay
(0.5), and Echo Operator (Add). Intensity/Decay use the 0–1 range in this revision.
Number adds to the current frame. Negative/positive times sample past/future;
out-of-timeline samples are transparent. Zero echoes with intensity 1 is identity.
Multiple Echo effects evaluate earlier prefixes rather than recursively sampling
the complete chain. Ordinary outer transform motion does not itself produce a
trail; precomposed internal motion does. Source-owner historical extents grow the
current allocation without stretching old pixels. Existing revisions are unchanged.

The timestamp/stage/space/prefix DAG, count/memory limits, hidden reverse video,
per-request coordinates and cancellation lifecycle are described in
`effect-layer-inputs.md`. No persistent history cache or analysis job is created.
Tests cover numeric seven-operator output in 8/16/32-bit projects, signed HDR and
alpha/edges, masks, final matte ordering, source bounds, prefix convolution,
shuffled seeks, copy/save/Undo, missing inputs/cycles, failure recovery, repeated
precomp instances, native controls/themes, decoded H264 and isolated portable
seeks. Active cost is measured separately from the 43-definition unused/disabled
GPU-work regression and effects-module bundle growth.

This is an independent first temporal consumer guided by Adobe's public
[Time effects documentation](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/time-effects.html).
Exact AE defaults/ranges, source/working-space/alpha/operator details and licensed
pixel comparisons remain unverified. Composition-space prefix buffers retain
existing clipping; Source mode provides native unclipped 2D processing. Temporal
group/adjustment/3D/backdrop owners, general other-layer time-offset inputs,
optical flow and simulation/analysis caches are not implemented by this batch.
Next: add a reviewed temporal other-layer selector/offset contract and genuine
Time Difference/Posterize Time consumers; expand calibrated source/backdrop/3D
semantics separately. Full-catalog and reference acceptance work remain open.
