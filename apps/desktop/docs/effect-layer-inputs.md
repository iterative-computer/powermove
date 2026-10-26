# Layer effect inputs

Status: Channel compositing checkpoint, October 17, 2026. Set Matte, Displacement
Map, Compound Blur, four-input Set Channels, Texturize, Stroke, Blend, Calculations
and Channel Combiner are real GPU effects using this contract. AE release/control/pixel acceptance
is still unverified. This is one batch of the full-catalog work.

## Implemented contract

`Effect.inputs` is a static dictionary separate from `Effect.p` and animated
`ChannelValue` data. A layer parameter is `{ k: 'map', label: 'Map Layer',
type: 'layer', def: '' }`; the empty default is an API metadata sentinel, never
an animated channel. Instances initialize `inputs.map` to
`{ layerId: null, stage: 'source' }`. The native layer picker and Input Stage
selector use the ordinary Row/SelectField controls without a stopwatch.

References are composition-local stable layer IDs. Optional `space` is
`composition` (also the meaning of an absent field) or `source`. Layer visibility and Solo
are ignored for the selected input itself. Time range, trim, speed, time remap,
source dimensions, parent transforms and working precision still apply. A selected
group renders its normal child stack while ignoring visibility inherited from
that selected group. Inputs outside their timeline range render transparent.

- Source: transformed content before the selected layer's masks, effects and matte.
- Masks: Source with that layer's masks.
- Effects: Masks with its enabled effect chain and track matte, before ordinary
  layer blending/transitions into the surrounding stack.

The default Composition space uses transformed, premultiplied composition buffers. Ordinary layer opacity
is applied with content; group opacity is applied at the group's boundary, as in
the existing compositor. Normalized layer bounds and forward/inverse matrices
allow centered, stretched or tiled sampling. Pixels already clipped by composition
rasterization are unavailable. These semantics are not a claim of AE source-space
or off-composition parity.

Generated shaders receive `u_<key>` (sampler2D), `u_has_<key>` (float),
`u_<key>UV` (composition pixels to normalized layer bounds), `u_<key>FromUV`
(the inverse), `u_<key>Res` (layer pixel dimensions), and `sample_<key>(uv)`.
The sample helper returns transparent outside the selected source bounds or
render target. `u_layerUV`, `u_ownerFromUV` and `u_layerRes` describe the owner.
None is explicitly flagged. Set Matte, Displacement Map and Compound Blur
preserve the image with None; each Set Channels None input reads the current image.
Layer Source space is described below. At most four layer inputs are allowed per effect, also checked against GPU units.

## Scheduling, lifetime and diagnostics

The iterative phase DAG models Source -> Masks -> Effects, effect inputs,
track mattes and group child outputs. Backdrop relationships participate in
command-time topology checks. Source/Masks self-inputs are valid; Effects
self-inputs, multi-layer cycles and group/matte/backdrop cycles are rejected
with a readable layer/phase path. Graph-changing commands validate within the
existing history transaction; failure rolls back the whole batch. Ordinary
scalar drags do not rebuild a dependency graph.

Each render invocation prepares referenced phases in topological order before
its ordinary stack. Input frames are memoized for that invocation only, at the
same composition, time, dimensions and precision. Render and preparation share
exact shutter sample centers and carry sample count/width through nested precomps. They remain pinned until the
invocation ends. Planned input bytes plus working-surface reserve must fit the
framebuffer budget, and every new/reused framebuffer checks live leased bytes.
Success, error, abort and context-loss diagnostics release invocation-owned
leases. There is no cross-frame/provider/context input cache to invalidate.
Input-dependent video capture also bounds its owned decode canvases by the
texture budget and drains pending seeks on cancellation without publishing a
partial prepared frame map. Last-consumer release is a future optimization; the initial conservative plan
keeps all requested input phases within the explicit budget until frame end.

Direct 3D, groups containing 3D, audio, null and adjustment sources are rejected. An Effects input
whose selected layer needs the surrounding backdrop is explicitly unsupported;
rendering a copied single-layer project would give the wrong image. A group's
internal adjustment/backdrop rendering uses its actual local stack. Generalized
external-backdrop scheduling and calibrated 3D inputs remain future work.

Missing/deleted IDs stay in project data for Undo/recovery. Active unresolved or
invalid inputs produce runtime/export diagnostics. Typed sanitization and actual
hydration preserve unknown revisions and future input records. Those records
are not silently reinterpreted. Whole-block layer copying, agent duplication,
composition duplication and effect clipboard copying preserve/remap references;
internal copied IDs are remapped and external IDs remain stable.

## Export and verification

Native output validates input graphs before starting an encoder. The general
per-frame path renders input effects; retained/static and asynchronous effect
readback eligibility intentionally exclude all four input operators. Hidden
referenced videos/precomps participate in both exact capture preparation and
normal timeline video enumeration, including reverse source-time seeks. Exact capture uses microsecond-scale seek
checks, prepares each group/footage shutter timestamp, and keys nested instances
by their full precomp path plus layer ID and composition time. A missing planned
frame fails explicitly rather than sampling whichever frame a shared decoder last
visited. The live viewer freezes exact shutter frames on private decoders; validated
paused seeks are captured before playback can skip a high-frame-rate target. A
pending invocation holds the last complete picture, then reports its actual
presentation time to transport. Temporary bitmaps share the preview memory cap
(also bounded by the texture allowance), release on completion/cancellation/pressure,
and are invalidated on project/provider/context changes. Input scenes do not
speculatively warm retained preview frames. Ordinary projects keep the existing non-input preparation path.
Portable export packages references and effect definitions and uses the shared
renderer. Invalid active references prevent export; editable JSON project saves
remain possible for recovery.

`e2e/layer-input-effects.spec.ts` exercises GPU stage distinctions, hidden maps,
matte inversion/stretch, signed displacement/wrap/expansion/neutral controls,
8/16/32-bit availability, signed HDR and fractional alpha, native controls and
narrow Ramp fields in both themes, serialization, deletion/Undo, copy remapping,
memory/cancellation/context-loss diagnostics, hidden reverse video, native raw
output and real H.264 encode/decode, a portable player in an isolated browser window, and edits during held
encoder acknowledgements. Context-loss injection here checks cleanup/diagnostics;
it does not replace hardware loss/restoration testing on every supported GPU.
Unit suites also cover a 12,000-layer DAG, cycle families, command atomicity,
static-input/non-animation behavior, clipboard data, and future revisions.

## Next work

1. Pin an AE release and compare licensed reference frames/controls. Calibrate
   Set Matte's premultiplication and Displacement Map's midpoint, sign, filtering,
   centered/tiled map and expansion conventions.
2. Calibrate implemented source-owner bounds/edges, then add real external
   matte/backdrop coordinate conversion and 3D input scheduling.
3. Extend dependency fingerprints before enabling retained/static/asynchronous
   input optimizations; consider last-consumer release within existing budgets.
4. Compound Blur and Set Channels are implemented. Add further layer-driven
   operators and typed paths/assets separately. Exact shutter inputs are tested;
   arbitrary time-offset effects, optical flow and analysis/stateful systems still
   need deterministic, cancellable frame-request and analysis infrastructure.


## Second batch: blur maps, four sources and shutter inputs

The provisional behavior baseline is the public Adobe effect matrix dated
2026-05-13 plus the Channel and Blur & Sharpen category pages retrieved
2026-10-17. No installed AE was available in `/Applications`; these documentation
sources support independent implementation, not a pixel-equivalence claim.

Compound Blur uses premultiplied map luminance to choose one Gaussian radius
per output pixel. Every sample for that output uses the same radius in a genuine
2D gather; a variable map is not approximated as two separable variable-radius
passes. Maximum Blur is in layer pixels, with local sigma = maximum * luminance / 2.
Black maps and zero maximum are exact neutral paths; Invert Blur reverses the map,
and None preserves the image. Stretch Map To Fit normalizes the map onto owner
bounds and extends its boundary values for output growth; otherwise the map is
centered in owner pixels and transparent outside its bounds. Input pixels outside
the available composition buffer are transparent. Sampling preserves premultiplied
alpha and signed/HDR color. The 17-by-17 maximum gather uses filtered mip levels
for wide radii. That bounded approximation, luminance/working-space conventions,
defaults and off-composition support still need AE calibration.

Set Channels exercises all four sampler slots with independent Red/Green/Blue/Alpha
layer/stage selectors and channel choices. Each None input uses the current image.
Selected sources map their normalized bounds onto the owner, RGB is extracted in
straight color, and output is premultiplied by the selected clamped alpha. Signed
and HDR RGB are retained. Exact AE mapping, full channel options and defaults are
still partial. The layer selectors remain static; animations on referenced content
and ordinary parameters survive save/load and rendering.

`e2e/input-shutter.spec.ts` uses a lossless 64-fps step signal to distinguish the
four-sample shutter mean from the central frame, with positive/negative shutters,
seek-order changes, live private decoding, native delivered bytes, isolated portable
video playback, repeated nested instances, and explicit missing-frame failure.
`e2e/compound-blur.spec.ts` compares real GPU output with independent per-output
2D convolution, including a varying diagonal map, neutral/inverted maps, opacity
and parameter animation, transforms, signed HDR, mip cleanup, native controls,
H.264 decode and isolated portable output. `e2e/set-channels.spec.ts` checks all
four hidden sources, stages, HDR/alpha, copied reference IDs, persistence and native
grouped controls/layer-picker interaction in both themes.

## Third batch: explicit source space

The native Input Space picker selects Composition or Layer Source independently
of Source/Masks/Effects stage. Existing project records are not rewritten and
continue to use Composition. The phase key includes space, so one input may use
the raw source while another consumes the same layer in the composition. Cross-space
cycles are rejected by the same iterative DAG; copy, hydration, Undo and export
retain the optional field. Unknown future spaces remain preserved data and are
diagnosed when used.

Layer Source renders the complete native content quad into a source-sized buffer,
before the selected layer's position/anchor/parent/rotation/scale, ordinary opacity,
blend and transitions. Content alpha, timeline range, trim/remapping, source content
animation and working precision apply. Masks are evaluated in source-local pixels;
Effects applies the selected layer's enabled chain within that source extent.
Nested effect inputs may independently choose either space. Selected precomps
render their entire source composition. Hidden and zero-opacity referenced videos
are still decoded at each required shutter timestamp, including repeated instances.

This resolves map pixels that used to disappear beyond composition edges in Set
Matte, Displacement Map and Compound Blur. It does not yet move the effect owner's
whole processing chain out of the composition buffer. Owner pixels already clipped
before displacement/blur remain unavailable. At this third-batch checkpoint, effects expanding beyond the selected native
content extent remained clipped there; the opt-in source-owner contract below
adds declared progressive expansion. Group source-space inputs,
source-space Effects with external track mattes, direct 3D and surrounding-backdrop
inputs are explicitly unsupported; use Composition or a precomp where appropriate.
The existing shader/extension internal raster-resolution ceilings remain in force.

Source dimensions are checked against GPU limits; source-byte estimates join the
composition working reserve, and every actual lease checks live bytes. Abort,
context diagnostics and failed allocations unwind the invocation and restore raster
coordinate state. A selected source is not silently resized to fit a budget.

`effects.supportsSourceRasters === true` advertises this contract. Generated input
shaders additionally receive `u_<key>Midpoint`: 128/255 for actual 8-bit buffers,
0.5 for floating buffers. Input mapping matrices address the consumer raster's
virtual pixel coordinates (divide by `u_res` to obtain texture coordinates), not a
promise of a particular physical input texture size. `sample_<key>(uv)` remains
the recommended normalized, bounds-checked sampler. Input `Res` is the selected
native content extent in source space.

Displacement Map revision 2 maps the actual buffer midpoint to zero and each end
to its full signed displacement, with Off explicitly zero. Revision 1 retains the
original `2*x-1` mapping. This independently implements a documented byte-gray
neutral convention while retaining exact floating-gray neutrality; licensed AE
precision/rounding/sign acceptance remains unverified.

Texturize is a real per-pixel height-gradient relief operator, with a texture layer,
light direction, contrast and tile/center/stretch placement. A four-neighbor
luminance gradient supplies a surface normal; directional gain multiplies the
premultiplied source RGB while retaining alpha and signed/HDR values. None and
zero contrast are exact neutral paths. Kernel, lighting convention, defaults and
full AE parity remain unverified. Public behavior reference: Adobe's
[Stylize documentation](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/stylize-effects.html).

`e2e/source-raster-inputs.spec.ts` covers off-comp pixels, raw/mask/effect stages,
mixed-space chains, native bounds of precomps, encoded midpoint revisions,
precision/HDR/alpha, GPU-dimension failures, cancellation/context recovery,
animated native controls in both themes, actual H.264 encode/decode and isolated
portable output. `input-shutter.spec.ts` now also runs source-space maps whose
ordinary opacity is zero and whose transforms place them completely off-comp.

This third-batch milestone was source-sized owner processing and typed mask/path
and asset references; the fourth batch below implements that initial foundation. Generalized backdrops, 3D and temporal/analysis systems
remain separate work; these records are still static selectors, not numeric channels.

## Fourth batch: source owners, bounds and typed references

Layer `effectSpace` is static, with absent/`composition` preserving the historical
pipeline. The native Effect Space picker explicitly opts a 2D content owner into
`source`; it never silently changes an existing project. Source owners render
content and masks in native source coordinates, apply the complete enabled effect
chain, then transform/opacity-composite the result. Track mattes remain applied in
composition coordinates after this transform. Groups, adjustment/audio/null and
direct 3D owners are unsupported. Each enabled revision must declare
`sourceProcessing`; historical definitions without that declaration remain
available in Composition mode. Backdrop effects are explicitly rejected.

Source processing declares `{padding?:string[],factor?:number,pixels?:number,
when?:string}`. Padding uses evaluated absolute parameter values in source pixels;
`when` gates it with a toggle (Displacement Map's Expand Output). The planned frame
includes the sum of supported extents. Each effect gets its own progressively
expanded domain, so later alpha-creating operations cannot fill unused padding.
Native owner coordinates stay stable inside the padded frame. Gaussian/Compound
Blur, shadow and displacement now have space to grow before composition clipping.
Shutter samples prepare/evaluate their own dependency contexts and source chains,
then average the transformed results. General static/async export optimizations
remain disabled for source owners; memory, cancellation and context diagnostics
use the invocation-owned lease path. Texture/source dimensions remain subject to
explicit budgets and GPU/internal renderer limits; no silent resizing is added.

Static `Effect.inputs` now additionally supports:

- Path: `{kind:'path',layerId:'id',pathId:'mask-or-shape-path-id'}`; both IDs null
  means None. Select a mask ID (analytic rect/ellipse or its Bézier path), or a
  shape-path ID in the same composition. Geometry is evaluated on its provider
  and mapped in owner-local coordinates; provider visibility/mask enablement do
  not hide the geometry. Mask/path transforms, hierarchy, repeats and geometry
  animation apply. This is not a projected 3D/world-path contract. Partial trims
  use the existing bounded path-flattening implementation.
- Image asset: `{kind:'asset',assetId:'id'}`; null means None. Decoded `image`
  assets are supported independently of timeline layers, with native image bounds.
  LUT assets use the explicit `assetKind:'lut'` contract below. Animated/video
  assets and arbitrary analysis/OCIO configuration data remain unsupported.

Declare `type:'path'` or `type:'asset'`, `def:''`. Path params may name a numeric
`pathWidth` parameter. All references share a four-sampler limit and stay outside
Channel animation data. Generic shader samplers/mappings apply to these types too.
Path coverage is bounded by raster/texture budgets; asset dimensions and aggregate
reference bytes are checked before upload. Uploading a reference restores the
source texture binding before drawing, keeping first-upload output identical to
cached/hydrated frames. Missing records remain in editable data for recovery and
are shown by native pickers; active unresolved references diagnose/fail export.
Layer duplication remaps path-provider IDs; asset IDs remain global stable IDs.
Portable export gathers referenced asset bytes even without a media layer.

Stroke revision 2 adds Brush Hardness, Spacing, Start/End, All Masks and Stroke
Sequentially to color/size/opacity and Original/Transparent/Reveal painting.
The selected path chooses its provider; All Masks enumerates that provider's
masks, including disabled masks. Start/End measure arc length, independently per
path or over the combined length when sequencing. Dabs use maximum coverage so
overlaps do not build extra opacity. Zero spacing is continuous; hardness 100
uses native antialiased round Canvas strokes. Soft/spaced coverage is bounded by
64 Mi pixel evaluations and 65,536 dabs; exceeding a limit diagnoses rather than
freezing or silently dropping geometry. Brush width and spacing are owner-local
pixels and follow affine layer transforms. Curves retain bounded 32-segment
flattening for trims/soft brush sampling; hard full paths in revision 1 retain
the original exact Canvas Bézier trace. No licensed brush-kernel parity is claimed.

Revision 2 declares `sourceProcessing.geometry:['path']`. In source owner mode,
conservative transformed control hulls plus brush radius grow both allocation and
progressive per-effect domains. Geometry beyond the original owner is no longer
clipped to a width-only envelope. Native composition clipping still applies to
the final transformed layer. Revision 1 preserves its width-only bounds and old
hard outline rasterizer exactly. Projected/3D geometry remains unsupported.
The public behavior source is Adobe's
[Generate documentation](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/generate-effects.html).

Texturize revision 2 adds an optional Texture Asset. A selected image takes priority
over its layer map; None falls back to the layer. Revision 1 remains unchanged.
Both consumers support native/project/player serialization and ordinary parameter
animation; selectors themselves remain static. Hosts advertise
`effects.supportsSourceOwners` and `effects.supportsTypedReferences` separately.

Acceptance is in `source-owner-effects.spec.ts`, `typed-effect-references.spec.ts`
and the extended shutter fixture: off-comp recovery, exact legacy mode return,
blur/shadow growth, animated mask/Bézier references, copy/remap, missing-provider
Undo, first-upload/hydration equality, actual H.264 and isolated portable playback.
The existing 3D/backdrop and licensed AE calibration limits still apply. Next work is calibrated brush/bounds/edge conventions, further geometry/layer
consumers, additional LUT formats and declared color management, followed by
temporal/analysis infrastructure; the full catalog remains open.

## LUT/config asset foundation and Channel consumers

`type:'asset',assetKind:'lut',def:''` selects durable standalone `.cube` assets,
with a native Choose LUT command. Image remains the default asset kind. A LUT is
not footage and cannot create a timeline layer or composition. Static asset IDs
survive clipboard/save/copy and replacement; canonical media coalescing remaps
asset IDs in effect references. Deletion retains missing IDs for Undo/recovery,
even if old runtime data is still retained by history. Active missing references
fail rendering/export. Inspector values re-read live references after import,
ordinary editing and Undo instead of retaining an old inputs object.

The parser accepts standalone 1D (2–4096 entries) or equal-axis 3D (2–64 entries
per axis) IRIDAS `.cube`, optional TITLE, comments and increasing per-channel
DOMAIN_MIN/MAX. Red varies fastest. File/text limits are 16 MiB; numeric values
must be finite within ±65504. Combined shaper+3D cubes, includes, unknown
directives, duplicate/late headers and mismatched row counts fail before import.
No OCIO/ICC profile or executable/general configuration loader is implied.

Apply Color LUT uses linear 1D or trilinear 3D interpolation on straight RGB,
clamps input coordinates to the declared domain, then premultiplies the result
by unchanged alpha. Amount zero/None is exact identity; negative/overrange table
outputs are not clamped by the operator in floating working buffers. Tables use
nearest RGBA32F textures and explicit interpolation; they share texture-budget,
replacement/version, eviction, cancellation and context cleanup infrastructure.
Packed dimensions are checked against the actual GPU. Project suspension/clear
releases reference textures. The portable player restores `.cube` bytes through
the same bounded parser, rather than shipping only file metadata.

Solid Composite is a real premultiplied background with separate source/solid
opacity and nine blend modes. The solid is restricted to native owner bounds;
existing expanded content outside those bounds survives. Arithmetic has twelve
real straight-RGB operators (including 8-bit bitwise logic), alpha preservation,
clipping and explicit byte wrapping. Slice and the full AE blend menu are not
implemented; both remain partial/reference-unverified. Their shared delivery
fixture chains Solid Composite → LUT → Arithmetic through native H264 and an
isolated portable player. Adobe's public
[Channel](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/channel-effects.html)
and [Utility](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/utility-effects.html)
descriptions guide behavior, without proprietary code or reference renders.

Hosts advertise `supportsBrushGeometry` and `supportsLutAssets`. Brush metadata
names hardness/spacing/start/end numeric keys and allMasks/sequential toggles;
`coordinates:'layer'` supplies generated owner UV/resolution uniforms to input-free
operators such as Solid Composite. Contracts are validated before registration.
Acceptance lives in `stroke-lut-effects.spec.ts`, `channel-operators.spec.ts`, the
parser/geometry/reference unit tests and the preserved earlier suites.

## Blend, Calculations and Channel Combiner

These three independent operators reuse the static layer/stage/space DAG and
bounded sampler contract. Each uses one input and one effect pass. No new decode
or framebuffer cache is introduced. Owner source processing is explicit; existing
composition-space and historical definitions remain unchanged.

- Blend supports Color Only, Tint Only, Darken Only, Lighten Only and Crossfade,
  Blend With Original, and Center/Stretch placement. Crossfade interpolates
  premultiplied RGBA, including transparent regions, rather than stacking a
  fading foreground. Other modes retain owner alpha and weight changes by the
  control layer's alpha. Color/Tint transfer bounded HLS hue/saturation while
  retaining owner lightness and its HDR peak scale; Tint multiplies donor and
  original saturation so achromatic pixels stay achromatic. Light/Dark and
  Crossfade preserve signed HDR math. None or 100% Blend With Original is neutral.
- Calculations has RGBA/Gray/Red/Green/Blue/Alpha extraction for both layers,
  inversion of straight channels and alpha before extraction, secondary opacity,
  Center/Stretch, and nine common blend modes. Extracted inputs composite with
  explicit premultiplied source-over math. Preserve Transparency restores the
  original alpha while retaining the composite's straight color. None still
  permits extraction/inversion of the first input; the default RGBA/None is
  identity. Alpha generation is limited to native owner bounds; expanded existing
  content outside those bounds is retained. The full AE blend menu remains open.
- Channel Combiner extracts eleven scalar channels (RGB, alpha, luminance, HLS,
  saturation multiplied by minimum distance to black/white, and min/max RGB).
  Eight destinations cover RGB, individual RGB, Alpha Only and HLS displays.
  Invert acts on the resulting color channels, or alpha for Alpha Only; Solid
  Alpha forces opacity afterward. Scalar RGB/min/max/luminance preserve signed
  HDR; HLS operations use bounded unit-gamut colors. None means the owner image;
  a selected layer is the source. This replaces a separate Use 2nd Layer switch.
  Multichannel RGB/HLS/YIQ conversion menu variants remain unimplemented.

Public behavior guidance is Adobe's [Channel documentation](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/channel-effects.html).
All controls/defaults/math are independent implementations, without licensed AE
render comparison. Alpha weighting, working space, HLS transfer, exact ranges,
menus and edge/placement conventions are reference-unverified.

`layer-compositing.spec.ts` verifies numeric modes/extraction, 8/16/32-bit output,
signed HDR and tiny positive alpha, center/stretch and stages, transformed source
owners, hydration/copy/remap, deleted-reference Undo, cycle rollback, budget and
cancel/context lease recovery. Its native-control fixture uses hidden video
through Blend → Calculations → Channel Combiner, reverse seeks, actual decoded
H264 and an isolated portable player. All prior consumers/revisions are retained.

## Idle and disabled resource behavior

Registration stores validated JavaScript definitions and shader text. The Effects
browser reads registry labels/groups directly without translating unused GLSL. It does not
compile GPU shaders, upload effect textures or start background jobs. Programs
compile lazily when an enabled effect renders. Never-applied definitions do not
add per-layer render passes. A disabled applied effect keeps saved controls and
requires an enable check; statically disabled/missing references no longer force
the input DAG, exact-video preparation or the preview-cache exclusion. Disabled
parameter animation/expressions also no longer force otherwise-static artwork
onto the live-preview path. Animated
or expression-based enables remain conservatively planned and are evaluated at
the requested time. Unsupported future processing-space metadata still diagnoses.

An effect used earlier may leave a compiled program and reusable framebuffers or
textures in existing caches after disabling/removal. Disabling stops rendering;
it does not promise immediate VRAM release. Existing cache budgets/LRU/context
and project cleanup govern those resources. Imported assets are separate from
the catalog: project media restoration still loads/parses stored LUT assets,
including unused ones; a 64³ float table contains 4 MiB of decoded RGBA data.
No LUT is shipped or imported merely by registering Apply Color LUT. Source files
can be up to 16 MiB, held separately in the durable media library/project assets.

The idle-cost fixture compares no effects with all registered effects disabled,
counts compile/program/texture/upload/draw calls, checks graph preparation and
records memory/timing diagnostics. Timing is a small local readback fixture,
not an application-wide FPS or exact heap measurement. Production packaging
excludes built-in test/spec sources; release scripts clean obsolete development
chunks before packaging. Full-app storage deltas need comparable packaged builds,
not totals from the development output folder or an older unrelated package.

New compositing shaders require `supportsOwnerValidity` and use `u_ownerValid`.
A singular/collapsed owner must not paint a reference through composition-space
fallback coordinates. Adjustment fallback remains valid; source-owner rendering
retains its normal local coordinates. Historical shaders ignore the new uniform
and preserve their prior output.

## Versioned representations and explicit color spaces

Channel Combiner revision 2 adds twelve multichannel From modes while retaining
revision 1 exactly: RGB↔HLS, RGB↔HSV, RGB↔YIQ/YUV, linear sRGB↔XYZ D65 and
RGB↔YCbCr (709). To is bypassed for these modes; Invert applies after conversion.
Scalar choices retain their numeric IDs and alpha semantics. HLS inputs/results
are bounded; HSV has nonnegative HDR Value. Matrix representations preserve signed
components in floating buffers. A negative chroma component cannot round-trip
through an 8-bit unsigned intermediate without clipping. These are mathematical
representation conversions on supplied RGB values, without implicit profiles.

The shared `effects-basic/color-math.ts` derives RGB/XYZ matrices from D65 primary
chromaticities, and provides CPU/GPU signed transfer functions. Explicit named
profiles are sRGB, Display P3 (sRGB transfer) and Rec.2020 display gamma 2.4; the last
is not a camera OETF, PQ or HLG. Color Profile Converter decodes the declared input,
converts linear primaries via XYZ, optionally encodes the output, and premultiplies
by unchanged alpha. Linear Input treats supplied values as linear; Linear
Output leaves output linear. Clip Output is optional, Amount zero/default matching
profiles are exact identity. No import/project/output working space is silently
assigned. Profiles are baked math/metadata; no external ICC files are read.

HDR Compander compresses `sign(x)*(abs(x)/gain)^(1/gamma)` and expands with its
inverse `gain*sign(x)*abs(x)^gamma`, preserving alpha. Floating intermediates are
needed for signed HDR; precision/representable-range limits still apply. The
chosen power curve is independent and not AE-reference-calibrated.

Both operators use one ordinary pass, no input textures or background jobs, and
support explicit source owners, stock Inspector controls, animation, history,
serialization and native/portable delivery. ICC/OCIO, intents, project-wide color
policy, scene compensation and arbitrary profile assets remain separate work.
The CPU reference tests and `color-space-effects.spec.ts` verify the supported
contract; exact licensed AE render comparison remains unverified.

## Bounded temporal owner-prefix requests

`effects.supportsTemporalInputs` advertises the first deterministic temporal
contract. A raw, single-pass definition declares
`temporal:{step:'echoTime',count:'echoes',maxSamples:32}`. Both keys name ordinary
numeric, animatable parameters. The host evaluates them at the consumer time and
requests the owner at `T + i*step`, for i=1..round(count), immediately before the
consuming effect's exclusive array index. Earlier effects and their reference
inputs are evaluated at each provider time; later effects and the owner's final
track matte are excluded. Source/Masks/complete Effects remain distinct phases.

Frame keys include composition-local layer ID, stage, space, exact timestamp and
optional exclusive effect prefix. Untimed topology keys retain their old format.
Multiple temporal effects form a bounded iterative DAG; zero offsets never feed
the consuming effect back into itself. Same-frame self/cross-layer/group/matte
cycles still diagnose. The per-composition plan permits at most 256 retained input
frames and 65,536 phase nodes. One effect permits at most 32 additional samples,
with finite steps within ±3600 seconds; invalid requests diagnose rather than
silently truncating. Nested video traversal permits 8192 visits and 1024 unique
instance/timestamp captures. These count limits supplement live framebuffer,
texture, raster and private-preview budgets; they do not override them.

The first raw temporal contract accepts scalar/color controls, without direct
reference params, keepOrig, sampling/grid/coordinates contracts or extra declared
source padding. Prefix effects keep their own normal reference/kernel contracts.
Every frame uses its own provider time for transforms, content, masks, parameter
tracks, layer references and trim/speed/remapping. Samples outside the composition
or owner timeline range are transparent, with the existing exact composition-end
convention retained. Underlying footage time boundaries retain ordinary media
playback semantics. Hidden referenced videos are prepared at all needed times,
even when central-time timeline enumeration omits them. Full precomp instance
paths remain part of video keys. Source owners union historical native extents
before allocation and retain absolute source pixel positions as dimensions change.
Composition-space historical buffers retain composition clipping; their affine
coordinates align back to the current owner so outer layer motion alone does not
create an Echo trail. Source owners are the unclipped 2D processing path.

The temporal shader runs once per sample with `u_temporal` (the immutable sample),
`u_tex` (accumulated result), sample index/count, validity and provider/current
coordinate metadata. Only two ping-pong working surfaces are needed; historical
prefix frames stay pinned for this invocation within the existing budget. No
cross-frame history, background analysis, new global cache or seek-dependent
simulation state is introduced. Cancellation/context diagnostics unwind leases;
exact preparation drains pending seeks before a failed request returns.
Publication waits for font readiness and checks the initiating project/GPU context
again; replacement, context loss or final-wait cancellation retains the last
complete frame map and releases the cancelled job’s owned captures.

Echo is the first real consumer: signed Echo Time, Number of Echoes, Starting
Intensity, Decay, and Add/Maximum/Minimum/Screen/Composite In Back/Composite In
Front/Blend. Number excludes the current frame. Intensity and Decay weight
premultiplied RGBA; Blend divides the weighted sum by total frame count, Add keeps
HDR RGB while bounding alpha, and compositing uses premultiplied source-over.
Floating buffers retain signed/overrange RGB; unsigned intermediates clip it.
Controls, interpretation of semitransparent/HDR operators and exact AE pixels
remain reference-unverified. The implementation follows Adobe's public
[Time effects description](https://helpx.adobe.com/after-effects/desktop/apply-effects-and-animation-presets/list-of-effects/time-effects.html),
without proprietary binaries/code. Direct 3D, group and adjustment temporal owners,
surrounding-backdrop dependencies, arbitrary temporal other-layer inputs, optical
flow and stateful analysis remain open.

Echo's saved data is an ordinary versioned effect with numeric channels; transient
request graphs are rebuilt on seek. Copy/clipboard/Undo use the copied owner's own
prefix and remap preceding static references normally. Native and portable output
use this same scheduler; temporal effects stay off retained/static/asynchronous
single-frame optimizations. Disabled temporal effects do not select exact-video or
dependency work. Registration adds metadata/shader text only and GPU compilation
remains lazy. `temporal-effects.spec.ts`, `temporal-inputs.test.ts` and the extended
preparation tests cover numeric operators, bounds, multiple prefixes, shuffled
seeks, hidden reverse video/repeated precomps, history, cancellation, native H264,
isolated portable playback and measured active/disabled resource cost.
