# Project color depth

`project.colorDepth` is the single saved working precision: 8-bit normalized,
16-bit float, or 32-bit float. The Project Settings Composition card owns the
Color depth row and writes it through `set_composition`, so each change is one
Undo step. The native-only Color card retains the linear-light switch; it has
no second precision control.

Older native-engine projects may have saved `project.color.precision` without
`colorDepth`. On file hydration, legacy deserialization, and typed snapshot
sanitization, TypeScript maps `float32` to depth 32 and `float16` to depth 16,
then removes the old precision field. An explicit `colorDepth` of 8, 16, or 32
wins. Projects with neither setting open at depth 16. Migration copies the
project and color object, so callers' source documents are preserved.

The protocol's `ProjectColor.precision` remains a derived transport value for
the native engine: depths 8, 16, and 32 map to Unorm8, Float16, and Float32.
The native renderer selects its working format from the same `colorDepth`
setting. A 32-bit depth alone does not require a native-engine stamp; linear
blending and other native-only features still do.

`Renderer::target_format` describes the requested project format;
`device_target_format` applies the shipped compositor's 32-to-16 hardware
fallback when float32 filtering/blending is unavailable. The frame renderer,
viewer scratch target, and asynchronous snapshot targets use the same device
choice. This does not change the saved depth.

The CLI's explicit `--precision 8|16|32` remains functional: it sets Color depth
on a private in-memory render copy. Without that flag, exports use the saved
project depth. Request hints no longer silently promote an 8-bit or 16-bit
project when a source or delivery format has more bits. The direct CLI's ANGLE
sidecar and the native hidden host carry binary32 input and output for depth-32
shader fallback, and retain u16 wire slots for depth 8 and 16.

## Storage and render boundaries

Working targets use RGBA8Unorm, RGBA16Float, or RGBA32Float throughout the
compositor, effects, transitions, resampling, text/vector paint, media source
conversion, legacy mesh overlays, and export. Targets and content caches retain
format/precision in their keys. Compatibility source bitmaps remain separate:
an RGBA8 working target must not accidentally select Chromium's compatibility
video chroma reconstruction for an interpreted/native source.

Parametric mask coverage and mask application were previously fused in the
native compositor. Depth 8 now rounds/clamps at the same coverage-store and
mask-application boundaries as today's separate RGBA8 WebGL passes. There is
no added intermediate dither; the reference app leaves GL DITHER enabled and
the measured Metal offscreen output matches the direct RGBA8 writes.

Fallback shaders at depth 8 allocate RGBA8 attachments, upload/read exact byte
codes, and carry those codes in the existing u16 wire slots. They never round
through binary16 before the RGBA8 write. Requests, pending results, program
parity verdicts, and textures distinguish working depth. Older runner requests
omit depth and retain their existing 16-bit interpretation. At depth 32 the
runner uses RGBA32F/FLOAT throughout, with `OES_texture_float_linear` enabled.
Inputs, results, PBO reads, session bands, snapshot relays and job scratch files
carry little-endian binary32 values in `rgba32f`; the legacy `rgba16f` field is
empty. No binary16 conversion occurs. Cache keys, byte budgets and validation
retain the requested depth.

Source-cache format tags keep the prior float16/float32 encodings. Parity
verdicts and persisted probe pins use a new depth-scoped namespace at **every**
depth, including 16. Old unscoped pins are re-probed on first use; their keys
are not preserved. This changes first-use work, not the rendered result. Native
compiled shaders also use float32 targets. Asynchronous frame snapshots accept
mapped RGBA8 rows directly, preserving row order and the existing
premultiplied/straight/opaque snapshot encoding.

## Qualification

The reproducible capture entry point is `engine/corpus/src/color-depth.ts`.
Build the desktop and run its `--prepare` step outside the Electron lock;
then capture under the lock. Supply an external artifact directory. The
`pm-render` example `color_depth_parity` reads that directory, renders every
sample, and writes native and amplified difference images and measurements.
It fails if any output byte differs, with one reference-judged exception:
text sampling (below). It has no residual allowlist.

The slice has seven synthetic projects at all three depths: gradient, blend
stack, feathered mask plus matte, blur, glow, vector and rotated text. The
text sample's uploaded Canvas raster must match today's byte for byte (the
capture records it). Its composition is judged against an f64 reference
(gate-policy.md): that raster sampled bilinearly at each output pixel centre
through the layer's quad, times its opacity, over the depth's background
capture. Pixels within 2 px of sampled ink are Resampled under
`pm_reference::compare` (`EIGHT_BIT` at 8, `HALF_FLOAT` at 16 and 32). Every
other pixel stays byte-exact, and the depth-8 PNG export must equal the
engine's preview. Cached level-1 and level-2 rasters keep only their ink
crop, drawn through the part of the quad it covers. The crop's quad has the
same geometry as the full Canvas quad, but its f32 interpolation rounds
differently, so a rotated layer's bilinear weights differ from today's by
less than one subtexel step. The engine is at least as close to the
reference, and the crop costs a third of the full canvas's texture memory
while typing.

Parity verdicts compare exact byte codes at 8 and exact binary32 bits at 32;
the existing binary16 bound is unchanged. The 32-bit policy can keep programs
with small arithmetic differences on the WebGL runner; its throughput is not
qualified by the ordinary native composition counter slice.

The real WebGL fallback runner preserves 24,576 RGBA8 codes and 24,576 binary32
bit patterns exactly across synchronous/asynchronous readback, three passes,
and depth switches. The float32 witness includes values binary16 cannot
represent. A separate depth-32 shader-layer project with a three-pass effect
uses the production first-use ANGLE parity path for both programs. Its two
runner requests settle to the same 8,192 float32 channels as today's app,
with zero differing float bits. The reference includes 6,144 channels that
would change if rounded to binary16. This gate uses
`color-depth-fallback.ts` and the prebuilt `color_depth_fallback` example;
build and prepare outside the Electron lock. Native transport regressions
also cover interactive float32 input/result bands, job `.rgba32f` scratch
files, short-file rejection, and snapshot validation. Targeted
Rust tests exercise all depth switches, caller-owned targets, a native 3D scene
with a 2D overlay, source conversion/cache isolation, and actual session frame
snapshots. Settings/migration and runner unit tests pass. Protocol fixtures and
generated TypeScript are regenerated from the contract sources.

The video/3D gate (`color-depth-media.ts`, `color_depth_media`) captures
15 samples: two flat red/green frames, two detailed mid-tone frames with
scaling, rotation and partial opacity, and a visible three.js cube with a
translucent 2D overlay, each at all three depths. Source Canvas pixels are
recorded separately from composition sampling. The flat video and 3D samples
match output bytes and raw float values. The detailed source differs from
Chromium's YUV conversion at all depths by design: video now has one reading,
proven against an f64 reference rather than Chromium's bytes
([video-parity.md](video-parity.md)), so those six comparisons measure
closeness to that reference, not byte equality.

`color-depth-alpha.ts` captures actual legacy transparent PNGs with
identical overlapping solids and a blurred layer at depths 8, 16 and 32.
Every readback and PNG contains 113 alpha codes. All three premultiplied
frames match, including raw floats. Decoded PNGs differ at 176 / 2,968 / 179
pixels respectively, each by one RGB code with unchanged alpha. Transparent
8-bit PNG export now divides the premultiplied working floats by alpha and
rounds once, instead of reproducing Canvas2D's truncated 8-bit unpremultiply,
so a translucent pixel is within half a code of the f64 value
([quality-ledger.md](quality-ledger.md)).
Fresh rotated text sources and compositions are exact at all three depths,
including runner retention with zero differing byte codes or binary32 bits.
Capture preparation happens outside the Electron lock; prebuilt opt-in
Playwright batches hold a shared slot only for actual app execution. Each
app profile is removed in `finally`.

The extended depth-8 gate (`color-depth-expanded.ts`) adds 52 composition
captures: every registered blend mode over opaque and transparent backdrops;
layer, group and precomp motion blur at 3/6/16/20 samples with translation,
rotation, scaling and partial alpha; all four transitions in and out; masked
adjustment chains; nested transformed precomps; hard and feathered Bezier
path masks; every track-matte mode; and blur, glow, shadow, color and levels
effects. All 52 compare byte for byte, with raw CV 0. This is a composition
gate; separate actual PNG exports and the delivery-format matrix cover export.

The extended gate exposed a shared blend defect over translucent destinations.
The former native equation mixed straight colors by source alpha and then
multiplied by union alpha, which attenuated the source twice. Native WGSL
and the CPU reference now match today's premultiplied blend expression:
`(1-sa)*dst.rgb + (1-da)*src.rgb + sa*da*blend(dst/da,src/sa)`.
This fixes the eight non-normal transparent blend cases and the nested
screen-blended precomp. An independent CPU expectation and an RGBA8 GPU
witness preserve red/green source contributions over a translucent backdrop.
No tolerance or reference fixture was changed to obtain the 52-image pass.

The full render suite then exposed 25 primary-corpus comparisons and eight
SVG/text comparisons against captures made before main changed that equation
in `2b1fa379a`. Three PNG files are shared by the SVG and text gates, so these
failures involve 30 distinct reference kinds. Under explicit decision BLEND-1,
the main-merged app with Use New Engine off reproduced every original exactly
after changing only the two historical blend expressions: 20 float captures
bit for bit and ten decoded opaque images byte for byte, including ordinary
and isolated-text captures. Restoring the current equation produced
independently repeated exact captures. All 30 explained reference kinds were
recaptured through the corpus tool; none remained unexplained. An independent
PNG decoder and hash audit also verified the original and current artifacts.
Original files are retained, and both project and manifest
metadata record `recaptured after main 2b1fa379a blend change`, original and
current hashes, historical pixels, app build and merged head. The corpus
verifier checks that provenance. Comparison bounds are unchanged; see
`corpus.md` for the proof and publication workflow.

A separate depth-8 shader layer, transformed and screen-blended at partial
opacity over a gradient, uses a registered three-pass user effect. Both
production first-use parity requests settle through the actual WebGL runner;
all 8,192 final channels match today's app in byte codes and normalized float
bits. The same harness retains the independent binary32 proof at depth 32.

The seven depth-8 slice projects also export through the native PNG job with
the app's Float32 request hint and dither enabled; all decoded PNG pixels
match the app readback. The full 8-bit delivery matrix, independent codec
decoding and half-alpha witnesses are described in `color-depth-export.md`.

At 320×180, one output texture uses 230,400 / 460,800 / 921,600 bytes at
8 / 16 / 32 bits respectively. The warmed target pools of this slice have the
same 1:2:4 ratio. Concurrent debug-build timings are diagnostic only; no general
throughput win is claimed. On the same seven projects at depths 8 and 16,
`pm-perfbench` counted one cold frame per project, then two unmeasured warm-up
frames and twelve measured frames, recycling each output target as production
exports do. The cold totals at depth 8 versus 16 were 12,726,988 versus
21,021,412 texture-allocation bytes (0.605 ratio), 132,852 versus 132,884
upload bytes, 47 versus 49 render passes, and 4 versus 4 compute passes.
Warm twelve-frame totals were zero texture-allocation bytes at both depths,
203,136 upload bytes, 552 render passes, and 48 compute passes. Depth 8 did
not exceed depth 16 on any guarded counter in any project or phase. The
retained output-frame targets were 1,612,800 versus 3,225,600 bytes across
the seven pairs. The source and counts are `pm-perfbench/examples/color_depth.rs`;
timings are informational under concurrent machine load. These synthetic
qualification scenes do not replace the integration owner's complete native-app
corpus gate.

For comparison with today's compositor, the counter example also has a
`--composition` mode using the production `render_snapshot_into` endpoint.
Both endpoints return project-depth GL-row targets, complete GPU work and
recycle the output; neither includes readback or presentation. The native
caller-owned target is allocated inside the cold span. Unmeasured readback
and CPU row reversal verify all 14 outputs byte for byte. The generic
frame/export counts above retain their GPU row-reversal pass.

`color-depth-today-counters.ts` opens a fresh app/context for each project and
depth, suppresses automatic preview before project replacement, rejects warm
source textures and working targets, and measures one cold frame, two
discarded warmups and twelve warm frames. The measured cold texture bytes
for the composition endpoints are:

| Depth | Native | Today WebGL |
| --- | ---: | ---: |
| 8 | 11,114,188 | 4,586,544 |
| 16 | 17,795,812 | 8,964,144 |

Warm texture allocation is zero for both engines at both depths. Over the
84 warm frames, native records 444 draws, 468 render passes and 48 compute
passes; today records 456 draws, 372 clears and 336 framebuffer changes.
These API operations have distinct meanings. Native upload counts are
195,072 bytes at either depth; today has no image/buffer upload and 50,640
bytes of uniform setter payload. Packed/aligned wgpu buffers and WebGL
uniform setters are different units; neither measures physical bus traffic.

The native-versus-today allocation no-regression gate is **not met**. The
largest differences are the blend-stack's retained intermediate targets
(2,995,200 versus 691,200 bytes at depth 8), and native text's glyph atlas
(4,578,964 versus 355,328 bytes). Today's Canvas2D text rasterization happens
below the instrumented WebGL compositor and its private atlas is excluded,
so these counts are not total VRAM. Both raw results must be retained; the
8-versus-16 improvement does not establish parity with today's costs. No
throughput win or complete engine performance qualification is claimed. The
blend pool and text atlas follow the same allocation paths at depths 8 and 16;
these shared costs are routed to the performance lanes. Depth 8 has no extra
target or pass count compared with depth 16 in any measured project or phase.
