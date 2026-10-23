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
all, tested at Unorm8, Float16 and Float32 after the colordepth integration.
The six original
full-range BT.709 and limited BT.2020 direct-upload samples now also match
exactly. The transformed midtone witness matches the actual app's raw float
readback and exported byte mapping at both captured times and all three
working precisions. Round 3 adds 48 fresh frames from 16 recipes: all 36
SDR Canvas frames are exact, bringing the source total to 98. Of the 36 new
SDR live frames, all 36 are exact. Round 4's six HEVC SDR P3/BT.2020 control
frames also match Canvas and live exactly, bringing the Canvas total to 104.
All 12 captured PQ/HLG/P3 frames now match Canvas and live exactly, bringing
the total to 116 exact Canvas and live source cases. The independent RGB transfer oracle
also matches all 36,374 colors for HLG BT.2020, PQ BT.2020, and PQ P3.

The original 62 frames now also have independent direct-upload regressions,
including freshly imported ProRes renditions with measured premultiplied
alpha. Their Canvas references remain unchanged. Limited H.264 and HEVC
direct uploads use matrix products before translation, matching the measured
Skia operation order at the nine remaining half-code channel boundaries.

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
  offset. Canvas uses explicit fused accumulation in offset, Y, Cb, Cr
  order; the measured direct BT.2020 upload applies the offset after its
  matrix product. At the remaining half-code witness, offset-first gives
  125.5, while offset-last gives 125.4999847, preserving Chromium's byte 125.
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

Full-range BT.601/709/2020 H.264 Canvas snapshots use pinned libyuv's
integer, nearest-chroma bitmap conversion. BT.2020 then runs the same real
RGBA8-to-sRGB gamut pass. Limited I420A playback renditions use
libyuv's matrix conversion and alpha attenuation before filtering. The
ProRes 4444 reference is the actual Canvas-to-WebGL **premultiplied** upload,
not straight `getImageData` bytes; all 192 captured alpha codes are retained.

Deep VP9's planar 10-bit samples need the physical MSB-aligned unorm
storage of Chromium's P010 surface before texture filtering. Multiplying a
filtered low-bit R16 sample by 64 left one-code differences; shifting each
stored source code before importing the texture removes that filter precision
seam. This compatibility copy leaves the native decoded cache and interpreted
readings unchanged. HEVC's `v0a8` alpha surface uses the same centered YUV
matrix pass, with alpha attenuation before its RGBA8 store. VP9 alpha has
distinct Canvas and live readings: Canvas retains libyuv's bitmap conversion;
live samples the four actual decoder planes and attenuates RGB before storing
RGBA8. Explicit alpha choices supplied through Interpret Footage keep the
native reading and visible texture domain.

Deep VP9's direct matrix product also needs Skia's dot-product order:
`p0=y*c0; p1=fma(u,c1,p0); rgb=fma(v,c2,p1)+bias`. The final chroma
product is fused into the accumulated Y/U terms before the translation.
Keeping every product separate closes two witnesses but leaves the red tie
at (86,107) one code high; preserving this order closes all 36 SDR live
frames without changing Canvas arithmetic. A Metal sampler probe returned
the same sampled bits for six equivalent coordinates at that pixel, ruling
out a coordinate adjustment as the correction.

Full-range HEVC10 also needs Chromium's decoder output contract:
VideoToolbox is requested to emit video-range P010 even when the bitstream
is full-range. `DecodeReading::Chromium` selects that output only for the
measured unoverridden SDR compatibility class. Native sessions still request
the stream's native range. Decoder sessions, decoded-frame cache entries,
and scrub workers include the reading in their identity and release both
namespaces together. The original bitstream range is preserved as metadata;
the returned pixel format describes the actual surface range.

Full-range H.264's Chromium software reading also preserves the decoder's
aligned coded plane dimensions, separately from the displayed crop. The
256×144 witness has 256×162 luma and 128×81 chroma planes. Pool slots start
cleared before decode, including reused slots; decoded hidden macroblock
rows remain intact. The upload samples over that coded texture domain and
then crops RGB to the visible output. Sampling a visible-height chroma
texture instead produced the entire 256-pixel bottom-row defect (max 98).
This is a decoder-layout fix, with no special-case bottom-row weighting.

Full-range opaque 8-bit VP9 has a separate coded-width boundary. Chromium's
libvpx wrapper uses the decoder's luma stride as coded width: the 272×160
fixture exposes 352×160 Y and 176×80 U/V. Its Chromium reading therefore uses
`libvpx-vp9` and copies those actual stride-wide rows, after checking each
plane's extent against its owning AVBuffer. Visible dimensions stay 272×160.
Clamping UV at the displayed right edge instead changed the final column.
VP9 alpha also uses libvpx for its Chromium reading, preserving all four
owned plane strides (320-pixel Y and A rows in the 256-pixel-wide fixture).
Native and interpreted alpha readings keep their existing decoder policy.

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

`tests/fixtures/video-parity-round3` contains 16 source clips and 48 frames,
with four independent rasters each: direct video, premultiplied direct video,
Canvas, and premultiplied Canvas. All source and raster hashes are checked
before packing. The alpha clips must have nonuniform alpha and distinct
premultiplied RGB for both upload paths. Fresh captures include coded and
visible dimensions; external full-coded plane copies show actual H.264
padding differs with the crop geometry. No fixed zero-edge rule is used.
All 16 recipes decode in today's app, including HEVC alpha, deep VP9/AV1,
full-range 8-bit BT.601/709/2020, BT.2020 PQ/HLG and P3 PQ.

The registered Round 3 test compares Canvas and live at all three precisions for
every captured case, then aggregates all failures. All 288 comparisons are
exact, including HDR. It has no skipped or ignored case. The separate
alpha-interpretation regression verifies compatibility
cannot change the native reading's pixels or visible sampling domain.

Round 4 adds seven HDR/P3 controls at three timestamps, including six new
HEVC SDR frames with BT.709 matrices and either Display P3 or BT.2020
primaries. Their live path keeps encoded RGB; Canvas crosses its RGBA8 source
store, decodes with Skia's sRGB transfer, applies the pinned named-gamut D50
matrix, and re-encodes. All 36 new SDR reading/precision comparisons match
exactly. Every frame also retains independent CPU/GPU Canvas captures in
sRGB and Display P3, with actual context attributes and hash-bound rasters.
All 60 reused main rasters are unchanged from Round 3.

The explicit legacy HDR decision is MATCH: direct WebGL retains encoded-looking
RGB while the sRGB Canvas tone-maps it. The CPU and GPU Canvas HDR paths can
differ by up to three codes in these controls. Both are captured; a future
appearance correction must be a deliberate compatibility change.

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

The final consecutive browser/native pair covers 21 dataset rows, each with
six fresh cold and 36 alternating warm samples, plus separately labeled
native prepared-first samples. All 882 ordinary samples on each side have
valid matching PTS and source hashes. Native is faster on every row at both
p50 and p95, cold and warm. Cold p50 spans 2.73–10.04 ms natively versus
4.30–14.10 ms in Chromium; warm p50 spans 0.64–0.83 ms versus 1.70–5.75 ms.

Representative final paired measurements, in milliseconds:

| Source | Native cold p50 | Chromium cold p50 | Native cold p95 | Chromium cold p95 |
| --- | ---: | ---: | ---: | ---: |
| H.264 BT.709 limited | 2.82 | 4.90 | 3.06 | 5.75 |
| H.264 HD matrix only | 10.04 | 13.60 | 12.55 | 14.23 |
| AV1 BT.709 limited | 3.52 | 6.95 | 4.49 | 7.35 |

The earlier corrected pair is also retained. Pooling both pairs gives 12
cold and 72 warm samples per row: every cold and warm median is lower
natively. Pooled cold p95 is lower on 10 of 21 rows and warm p95 on 20 of 21.
The first pair had substantially higher native host load; its tails remain
part of the evidence, so these results do not establish tail parity under
all host contention. No samples are discarded from the pooled result.

Warm samples prime every seek position first, then alternate timestamps;
each measured sample still has a media conversion miss, so converted source
textures are not reused. Cold primes the source converter before opening a
fresh source/decoder. The browser harness now explicitly primes its video
conversion too. Both reported Round 4 pairs use this corrected endpoint;
earlier unprimed browser measurements do not establish the current gate.

Opt-in current-thread stage timing separates probe input, index, fingerprint,
decoder creation, packet reader and first frame decode. The probe's FFmpeg
Input now serves extradata and the first session, eliminating two redundant
opens (1.39–7.94 ms in representative pre-change rows). For limited SDR
H.264 through 1920×1080, the Chromium reading uses FFmpeg's direct software
planes instead of paying hardware decoder startup. The original 62-frame
zero-byte gate passes with this policy. Full-range H.264's software policy
is separately required for its coded-plane contract; larger full-range
geometry remains a coverage limit. Native, color-overridden, HDR and level-2
backend policy is unchanged. Chromium-compatible Main 8-bit 4:2:0 AV1 through
64K pixels prefers dav1d, with VideoToolbox retained as a fallback. Eighteen
visible plane comparisons at 256×144 and 320×180 were identical between the
two decoders. Larger and high-depth AV1 retain the native backend order.

Final H.264 BT.709 limited cold medians are 0.783 ms probe, 0.058 ms
index, 0.080 ms decoder creation and 0.788 ms first decode. HD H.264 records
3.024 ms probe, 0.082 ms index, 0.075 ms creation and 5.477 ms first decode.
AV1 records 0.466 ms probe, 0.035 ms index, 0.078 ms creation and 1.825 ms
first decode. Independent demuxer opening can
overlap index preparation. Actual decoder creation still uses the completed
container configuration/index; the optimization does not claim otherwise.

Open/import preparation queues the same planned video sources, instance IDs,
reading and decode purpose as rendering. A lazy worker hashes/probes/indexes
and parks a decoder and the selected decoded frame on the retained source
Arc and bounded decode cache; the presenter does not wait
for media I/O. There are at most two active preparation jobs process-wide,
16 queued jobs per renderer and eight prepared sessions per decode service.
Idle workers retire; closing cancels hashing, while an in-progress index may
finish after closure with its active slot retained. Completed stale sources
are discarded. Full queues or unfinished preparation use normal native cold
opening. Tests cover direct and nested sources, display preparation, shared
source identity, session bounds and idle capacity across renderers. Completed
display and export preparation require zero source opens, decoder creations,
and frame decodes on the first planned render. Prepared and cold display
readbacks must be byte-identical.

The explicit preparation cache keys self-contained container bytes by full
BLAKE3 plus extension, validates file version before/after hashing, and
bounds entries, metadata estimates and stat routes. Unix versions include
inode, size, mtime and ctime; Windows lookups reverify content. Playlists and
renamed manifests are excluded because sibling media is outside their hash.
For files of at least 128 KiB, hashing overlaps probing/indexing when no
same-size/container cached entry could provide a copy hit. Foreground sampled
identity reads also overlap probing/indexing at that size. Each cache publish
validates the file generation; small inputs keep the inline path. Startup
diagnostics distinguish VideoToolbox configuration and session creation, and
record overlapping durations without treating their sum as wall-clock time.
Prepared-first-frame, preparation time and preparation-plus-frame time are
reported separately; fresh cold remains the ordinary regression gate.
Across the final pair's row medians, preparation costs 1.77–6.82 ms, the
prepared first render 1.01–1.38 ms, and their combined cost 2.78–8.20 ms.

The native harness records every pm-perf counter with local/all-thread scopes
and the counting allocator. H.264/AV1 keep one compute pass, one conversion
miss and 720 upload bytes; software H.264 imports three planes instead of
two. Planar alpha uses a lazy additional pipeline and texture binding; opaque
and HEVC uploads retain the original seven-entry bind group. Making that
binding universal had added about four allocations per converted frame.
Before integration, all nine pm-perfbench ratchets passed unchanged bounds;
`scrub_24` records 13,013 thread and 15,430 all-thread heap allocations,
within the existing 15,507 allowance. No baseline is relaxed.

The final consecutive browser/native pair above includes corrected converter
priming. Sustained-playback measurement remains integration work. Raw run
metadata, binary hashes, every sample and earlier runs stay outside Git.

The post-colordepth full ratchet ran all nine benches against unchanged
bounds. Five pass; four exceed eleven counters: `export_120` thread heap
bytes (+523,458), `typing_60` texture bytes (+73,247,656 in both scopes),
`open_large_2000` heap counts (+3,583) and bytes (+508,984 thread,
+508,462 all-thread), and `import_3d_600k` heap counts (+2,405) and bytes
(+371,598 thread, +371,576 all-thread). These post-merge results supersede
the earlier nine-pass claim for the integrated tree; no baseline was raised.
These four benches contain no video. The increased first-use pipeline counts
and unchanged texture counts point to merged lazy initialization and full-canvas
text storage; this attribution is an inference, not an isolation experiment.
The video bench, `scrub_24`, passes its existing bounds.
The paired decode timings above remain the completed pre-integration
measurements and were not rerun. Strict clippy also flags imported scene3d
argument counts and a shader fallback closure; the full affected-crate
check completes with warnings. All nine color-depth render tests, the synthetic
still bitmap/cache test, and the presenter video-preparation test pass. The
conditional colordepth watch is finished; no recurring maintenance is required.

## Remaining closure work

The original direct-upload defects are closed at max 0 in Unorm8, Float16 and
Float32 source-storage tests. Canvas -> live -> Canvas cache switches use
all three timestamps, with no cache flush between readings.

The apparent 1/15-pixel transformed-composition discrepancy was in the
comparison harness's float-to-byte mapping. The actual app promotes its
Float32Array values to binary64 before multiplying by 255 and calling
`Math.round`; multiplying in f32 first introduced false half-code ties.
The native and app raw float buffers were already byte-identical. The
regression stores and checks both the exact raw floats and the app's actual
RGBA8 mapping; it does not replace or relax either reference.

Colordepth landed on integration at `eb25accb14`; this lane merged integration
through `38f02d097f`. The source regression loops compare all 116 Canvas
and 116 direct-live cases at each of Unorm8, Float16 and Float32, all exact.
The composed regression now checks all six saved depth/time references
(8/16/32 bits at 0.5/1.5 seconds), both byte-identical raw float readback and
RGBA8 mapping. It also verifies the target format and that saved project
depth overrides a Float32 export request. No references were recaptured,
replaced or relaxed. All 16 targeted media tests pass. Native Unorm8 working
storage retains native chroma siting/filtering; compatibility bitmap storage
retains Chromium's centered bilinear conversion independently of target depth.
The complete app playback/scrub/export and private-corpus gates remain
integration work.

The last Round 3 SDR gap, VP9 10-bit BT.2020 live at 1.125 seconds,
pixel (86,107), is closed by the matrix order above: native and today both
produce `[35,203,84,255]`. The captured reference is unchanged.

Default HDR conversion is exact for the captured 10-bit AV1/HEVC PQ/HLG
BT.2020 and PQ P3 sources. `media/hdr_compat.rs` and its shader preserve
Skia's sequence: stored N32 source -> destination sRGB -> linear Rec.2020
working format -> default RWTMO gain curve -> sRGB. Collapsing the first
two gamut transforms loses one-code ties. HLG also requires the CPU's exact
byte-to-float values, reconstructed transfer constants, and reverse-nested
FMA luminance accumulation. Direct HDR uses the same dot-before-translation
matrix order as deep VP9, and retains encoded RGB. All 126 Round 4 source
comparisons and the independent three-signal RGB oracle are exact.

Mastering/content-light driven gain curves, Dolby Vision, and unmeasured HDR
codec/primaries classes retain explicit capability gates. Interpret Footage
color overrides and compatibility level 2 keep the native color contract.

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

After the colordepth merge, the three original transparent image captures at
8/16/32 bits are exact in both native raw readback and actual PNG export
(zero different pixels, max 0). The complete 65,536 channel/alpha capture
table and half-code witnesses pass, as does the depth-8 translucent overlap
PNG export regression. Original legacy export references remain unchanged.

Source references: Chromium's
[video renderer](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/renderers/paint_canvas_video_renderer.cc),
[VP9 decoder planes](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/filters/vpx_video_decoder.cc),
and [VideoToolbox output attributes](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/gpu/mac/video_toolbox_decompression_session_manager.mm);
pinned Skia's
[YUV matrices](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/core/SkYUVMath.cpp),
[YUV texture effect](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/gpu/ganesh/effects/GrYUVtoRGBEffect.cpp),
[RasterPipeline](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/opts/SkRasterPipeline_opts.h)
and [skcms](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/modules/skcms/skcms.cc).
