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
setting. A 32-bit depth alone
does not require a native-engine stamp; linear blending and other native-only
features still do.


`Renderer::target_format` describes the requested project format;
`device_target_format` applies the shipped compositor's 32-to-16 hardware
fallback when float32 filtering/blending is unavailable. The frame renderer,
viewer scratch target, and asynchronous snapshot targets use the same device
choice. This does not change the saved depth.

The CLI's explicit `--precision 8|16|32` remains functional: it sets Color depth
on a private in-memory render copy. Without that flag, exports use the saved
project depth. Request hints no longer silently promote an 8-bit or 16-bit
project when a source or delivery format has more bits.


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
retain the requested depth. Native compiled shaders also use float32 targets.
Asynchronous frame
snapshots accept mapped RGBA8 rows directly, preserving row order and the
existing premultiplied/straight/opaque snapshot encoding.

## Qualification

The reproducible capture entry point is `engine/corpus/src/color-depth.ts`.
Build the desktop and run its `--prepare` step outside the Electron lock;
then capture under the lock. Supply an external artifact directory. The
`pm-render` example `color_depth_parity` reads that directory, renders every
sample, writes native and amplified difference images and measurements, and
fails if even one output byte differs. It has no residual allowlist.

The initial slice has seven synthetic projects at all three depths: gradient,
blend stack, feathered mask plus matte, blur, glow, vector and rotated text.
All 21 output images now match byte for byte. The first run's rotated text
differed at 293 / 294 / 303 pixels at depths 8 / 16 / 32. The text lane through
`ec1a0c5fd` (including `7ca866a3c`) was already merged; fetching and merging
the remote text branch reported it up to date. Its Canvas raster is exact at
all depths. The difference came from replacing the full Canvas quad with an
ink-cropped quad: f32 vertex and UV interpolation produced different bilinear
weights after rotation. Level 1 now keeps the original Canvas backing size
and quad, matching today's app. This removes every differing output byte
without changing glyph pixels, tolerances or intermediate quantization.
Level 2 retains its ink crop. The capture's `--text-only` mode also records
the actual uploaded Canvas texture, and the native example compares it
separately from composition sampling.

Retaining the full level-1 canvas can use more memory than the former native
ink crop. A maximal 8192² source is 256 MiB; sources beyond half the 192 MiB
text cache budget draw without being retained. This is a correctness tradeoff
to preserve today's sampling geometry, not an added 8-bit quantization point.
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

The video/3D gate (`color-depth-media.ts`, `color_depth_media`) adds two video
times and one visible three.js cube with a translucent 2D overlay at every
depth. All nine samples match both output bytes and raw float values exactly.
The video reference must show the fixture's expected red/green image, and the
3D reference must contain visible geometry and settle to two identical frames.
The seven depth-8 slice projects also export through the native PNG job with
the app's Float32 request hint and dither enabled; all decoded PNG pixels
match the app readback. The full 8-bit delivery matrix, independent codec
decoding and half-alpha witnesses are described in `color-depth-export.md`.

At 320×180, one output texture uses 230,400 / 460,800 / 921,600 bytes at
8 / 16 / 32 bits respectively. The warmed target pools of this slice have the
same 1:2:4 ratio. Concurrent debug-build timings are diagnostic only; no general
throughput win is claimed. The `pm-perfbench` counters are not yet on the merged
integration base; the paired upload, texture-allocation and pass-count gate
remains pending that merge. These synthetic qualification scenes do not replace
the integration owner's complete native-app corpus gate.
