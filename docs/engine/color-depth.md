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
omit depth and retain their existing 16-bit interpretation. The pre-existing
WebGL fallback transport for a 32-bit project also remains binary16; a real
float32 fallback transport and its parity proof remain unqualified. Native
compiled shaders do use the project's float32 targets. Asynchronous frame
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
Eighteen of 21 output images match byte for byte. The rotated text still fails:
293 pixels at depth 8 (maximum 1 CV), 294 at depth 16 (maximum 0.748 CV), and
303 at depth 32 (maximum 0.729 CV). Merging the latest native text implementation
did not clear these three cases. They remain a DEPTH-1 blocker, not an accepted
difference or a relaxed test.

The real WebGL fallback runner preserves 24,576 checked codes exactly across
synchronous/asynchronous readback, three passes, and depth switches. Targeted
Rust tests exercise all depth switches, caller-owned targets, a native 3D scene
with a 2D overlay, source conversion/cache isolation, and actual session frame
snapshots. Settings/migration and runner unit tests pass. Protocol fixtures and
generated TypeScript are regenerated from the contract sources.

At 320×180, one output texture uses 230,400 / 460,800 / 921,600 bytes at
8 / 16 / 32 bits respectively. The warmed target pools of this slice have the
same 1:2:4 ratio. Concurrent debug-build microbenchmarks are diagnostic only:
8-bit was faster on gradient, blending, mask and blur, but slower on glow,
vector and text in this run. No general throughput win is claimed. The
`pm-perfbench` counters are not yet on the merged integration base; a quiet
paired performance gate remains required, together with broader video/3D,
export-format and native-app qualification after the text blocker is fixed.
