# Chromium video upload compatibility

Compatibility level 1 preserves the decoder-to-texture boundary and the
encoded source metadata. Interpret Footage color overrides and level 2 use
the native color contract. The implementation draws through the native
renderer; it does not select the legacy renderer.

The capture target is Electron 44.0.0 / Chromium 152.0.7977.54 on macOS
ARM64, with Skia pinned to `b6d106297ff9ef2ff8094033695d045e87775581`.
The original 32 Canvas source captures now match at **max 0**, without changing
those reference bytes. Six actual app-imported ProRes rendition captures and
24 additional matrix/range/alpha captures also match exactly: 62 frames in
all, tested in both Float16 and Float32 compositions. This source result does
not close the separate live-upload and transformed-composition gaps below.

## Conversion order and precision

Twenty-nine exposed 8-bit decoder-plane comparisons (H.264, VP9 and AV1,
including the detailed midtone witness) are code-identical at matching PTS.
The residual source differences came from conversion and storage, rather
than decoder output. Small affine rounding errors can change a byte on a
flat half-code boundary as well as along chroma edges; chroma displacement
or dithering alone does not explain this pattern.

`media/video_compat.rs` and `shaders/video_upload.wgsl` reproduce the measured
Skia upload directly from the imported decoder planes:

- Sample raw unorm planes before native range normalization. Load the
  full-resolution Y texel exactly; reconstruct centered subsampled UV with
  hardware bilinear filtering before applying the matrix.
- Use Skia's default six-decimal affine matrix and independently rounded
  offset, with explicit fused accumulation in offset, Y, Cb, Cr order.
  Reconstructing the offset from Kr/Kb or normalizing first changes half-byte
  decisions. Store directly to RGBA8 without a half-float intermediate.
- P010 BT.601/709 uses the measured generic 8-bit table on MSB-aligned
  R16Unorm samples. BT.2020 HEVC10 uses Skia's named 10-bit table; a 16-bit
  table chosen merely from the texture storage width is visibly wrong.
- BT.2020 Canvas conversion crosses two real RGBA8 texture stores: YUV to
  encoded RGB, then gamut conversion to sRGB. Folding them into one pass
  with an algebraic quantization left a one-pixel error. The second pass
  uses pinned skcms's f32 D50 gamut concatenation and Skia RasterPipeline's
  approximate transfer functions. Its inverse-sRGB coefficients come from
  skcms's approximate inverse construction, rather than the real-number
  inverse formula. A 256-entry decode table uses the existing CPU Skia
  transfer implementation. Two positive-normal divisions in the shader's
  approximate power use exact binary32 rounding because the permitted GPU
  division error feeds a float bitcast and can change the output byte.

Incomplete SD H.264 descriptions with neither primaries nor transfer use
Chromium's BT.601 fallback, including a lone BT.709 matrix tag; the HD witness
uses BT.709. Fully specified tags remain authoritative. Untagged opaque VP9
uses the separately measured BT.709 reading at SD.

Full-range BT.709 H.264 Canvas snapshots use pinned libyuv's integer,
nearest-chroma bitmap conversion. Limited I420A playback renditions use
libyuv's matrix conversion and alpha attenuation before filtering. The
ProRes 4444 reference is the actual Canvas-to-WebGL **premultiplied** upload,
not straight `getImageData` bytes; all 192 captured alpha codes are retained.

Full-range HEVC10 also needs Chromium's decoder output contract:
VideoToolbox is requested to emit video-range P010 even when the bitstream
is full-range. `DecodeReading::Chromium` selects that output only for the
measured unoverridden SDR compatibility class. Native sessions still request
the stream's native range. Decoder sessions, decoded-frame cache entries,
and scrub workers include the reading in their identity and release both
namespaces together. The original bitstream range is preserved as metadata;
the returned pixel format describes the actual surface range.

Paused/final sampling and scrubs choose the Canvas reading; playback chooses
the direct upload. The converted-frame cache includes this distinction. The
cache-switch regression checks Canvas -> live -> Canvas at one timestamp,
so a cached source texture cannot conceal different readings.

## Public synthetic captures

`apps/desktop/scripts/video-parity.mjs` generates detailed, moving midtone
and gradient clips outside Git. Build the app before the Electron lease,
then run `capture` under `pm-electron-lock`. The harness records stream tags,
browser versions, `requestVideoFrameCallback` presentation times, VideoFrame
timestamps, direct WebGL pixels, Canvas straight pixels, and the actual
Canvas-to-WebGL premultiplied upload. Pixel transport is base64. App profiles,
video frames, blob URLs and GPU contexts are released on success and failure.

Chromium rejects direct ProRes in this build. Each MOV is therefore imported
through the real `PM.pickFiles` path. The harness waits for the app's VP9
playback rendition, proves the live video blob equals the durable MediaStore
blob, and saves those exact rendition bytes. Native comparisons decode the
same rendition. They do not compare the original ProRes decoder with a lossy
proxy and call the difference a decoder error.

`tests/fixtures/video-parity` contains all 38 original/imported frames,
including the unchanged original 32. `tests/fixtures/video-parity-round2`
contains six HEVC10 recipes (BT.601/709/2020 x limited/full range) and ProRes
422/4444 imports, three timestamps each. The latter includes a nonuniform
alpha plane. Hashes bind compressed reference rasters to source and rendition
bytes. The round-2 packer rejects incomplete captures, stale timestamps,
changed source/proxy hashes and incorrect premultiplication before writing.
These are small public synthetic media and raw source rasters, not private
corpus data or desktop screenshots.

`pm-render`'s `video_parity` example draws each captured source, writes raw
native pixels and pm-perf counters outside Git, and fails on **any** different
byte or unresolved row. The source-storage tests use the same zero bound for
all 62 frames in both working precisions. `pm-media` tests also verify actual
full-range/native versus video-range/Chromium HEVC surfaces, frame-cache
partitioning, worker identity, and release. P010 raw planes are unavailable
through this build's VideoFrame.copyTo (format is null); its conclusion comes
from the complete captured GPU/Canvas outputs.

## Paired decode/upload timing

`apps/desktop/scripts/video-decode-perf.mjs` and the release-mode
`pm-render/examples/video_decode_perf.rs` compare the same effective media
files, dimensions and timestamps, including the actual ProRes renditions.
The browser endpoint is load/seek through video or Canvas upload, a full-size
WebGL draw and `gl.finish`. Native times `Renderer::render` plus `wait_idle`,
including source open, indexing, decode, conversion and layer composition.
Device/renderer setup is excluded. Cold samples use fresh video elements or
fresh native source aliases after shader priming. Native aliases use RAII;
the browser profile and video/GPU resources use `finally` cleanup.

Two runs of 13 original/imported rows plus eight expanded rows each take
three cold and 18 alternating warm samples per row: 882 samples per side,
zero errors, and zero selected-PTS mismatches. Warm samples prime every
requested seek position first, then continue alternating timestamps; all
measured native samples still have one media conversion miss, so this does
not reuse a converted-frame texture. Native warm p50 and p95 are lower on
all 21 rows: p50 0.74–1.33 ms versus Chromium 2.15–6.45 ms; p95 1.05–2.92 ms
versus 2.98–12.53 ms. Earlier runs priming only one position had first-use
decode spikes and are kept separately in the external report.

Cold p50 is slower on seven rows, so the complete no-regression gate
**does not pass**. These are measured native/Chromium medians in milliseconds:

| Source | Native cold p50 | Chromium cold p50 |
| --- | ---: | ---: |
| H.264 BT.709 limited | 11.18 | 6.85 |
| H.264 SD matrix only | 11.50 | 6.40 |
| H.264 HD matrix only | 22.65 | 16.75 |
| H.264 BT.709 full | 10.10 | 5.95 |
| H.264 BT.2020 limited | 11.22 | 9.85 |
| AV1 BT.709 limited | 13.17 | 9.15 |
| H.264 midtone witness | 10.41 | 8.45 |

The native harness installs `pm-perf`'s counting allocator and records every
counter, both local and all-thread snapshots, alongside submit and GPU-wait
wall times. Cold samples have zero shader/pipeline creations, one media
conversion miss and one queue submit. The slower rows spend most time in
CPU submission (for example full-range H.264: 9.35 ms submit plus 0.77 ms GPU
wait). Fresh probing/indexing and decoder startup are plausible contributors;
the present counters do not isolate them, so the report does not attribute
the entire gap to a particular function. The ordinary warm source path uses
one compute pass; full-range H.264, gamut conversion and alpha renditions
use two. The no-regression follow-up must cover cold startup as well as warm
conversion; these measurements do not establish sustained playback cost or
private-corpus performance.

## Remaining closure work

The separately captured BT.2020 8-bit direct upload still has one pixel at
1 CV in its third sample. Its pre-existing live-upload test bound remains
1 CV; Canvas is now required exact. Full-range H.264's direct-upload boundary
also differs on its 256-pixel bottom row (max 98 CV in all three samples),
although its Canvas/export source is exact. Neither direct result is counted among the 62 exact Canvas captures.

The supplied rotated/opacity midtone composition still differs at 1 and 15
pixels at its two timestamps, max 1 CV, after its untransformed video source
became exact. That is a remaining layer-filter/composition precision gap,
not a passed strict-zero composition result. Operational side-by-sides and
pixel diagnostics stay outside Git and are linked by the lane report.

Deep VP9/AV1, other full-range 8-bit matrices, HEVC alpha, HDR/P3, alternative
decoder backends and the full private corpus need independent captures.
The current integration base exposes Float16/Float32; project depth 8 needs
rerunning after the color-depth lane merges. No broader capability reason
was removed on the strength of these narrower samples.

## Transparent PNG export

`pm-export::convert::legacy_png_rgba` preserves alphaFrame's truncated straight
codes, Canvas's integer premultiplication, then Chromium ARM64 Skia's float32
unpremultiplication sequence and round to even. Integer reciprocal tables and
exact rational division disagree at half codes. An independently captured table
tests all 65,536 channel/alpha pairs, plus three explicit half-code witnesses.
The previous integer reciprocal differed on 436 pairs; the new sequence matches
all pairs and the reported transparent export's 176 differing pixels exactly.
This applies to the shared level-1 PNG8 export seam at every project depth;
neither premultiplied rendering nor alpha values are changed.

Source references: Chromium's
[video renderer](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/renderers/paint_canvas_video_renderer.cc)
and [VideoToolbox output attributes](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/gpu/mac/video_toolbox_decompression_session_manager.mm);
pinned Skia's
[YUV matrices](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/core/SkYUVMath.cpp),
[YUV texture effect](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/gpu/ganesh/effects/GrYUVtoRGBEffect.cpp),
[RasterPipeline](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/opts/SkRasterPipeline_opts.h)
and [skcms](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/modules/skcms/skcms.cc).
