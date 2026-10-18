# pm-media: design notes

Production media I/O for plan section 13, built from the S6 spike
(`docs/engine/spikes/S6.md`, `engine/spikes/s6-decode`). Crates:
`engine/crates/pm-media` (probe, index, decode, images, cache, hashing) and the
decode-in half of `engine/crates/pm-gpu` (zero-copy import, normalization,
deinterlacing). This file records how the code meets the plan and where it
departs from the spike; the plan stays the authority.

## Building

`source engine/env.sh` sets `PM_FFMPEG_DIR` to our LGPL FFmpeg build
(`engine/third_party/ffmpeg`). `build.rs` generates bindings from that build's
headers and links its dylibs. dav1d and OpenEXRCore are static libraries from
`native/build.sh` (pinned, checksummed sources in `native/sources.lock`),
built on first use into `~/Library/Caches/powermove-engine/media-native`.

## Probe and index

`probe::probe` returns everything section 13 probe items 1 to 7 name: streams,
timecode (drop-frame aware), reel, UMID, content identifier, pixel aspect,
clean aperture, rotation; color (primaries, transfer, matrix, range, chroma
siting, mastering display, content light, ICC, HDR format) with the section 9
interpretation for untagged sources; the frame index; timing and priming; the
nominal rate; decode cost; poster frame and filmstrip seed. Waveform peaks
are `probe::peaks`.

The frame index (`index::build`) comes from container tables wherever they
exist. Only box, element, or KLV headers and table payloads are read:

| Container | Source | Notes |
|---|---|---|
| MOV, MP4, M4V, 3GP, 3G2 | `stts`, `ctts`, `stss`, `stsz`, `stsc`, `stco`/`co64`, `elst` | QuickTime edit-list semantics: samples decoded but not presented are marked, partly covered first frames kept |
| Fragmented MP4 | `moof`/`trun` headers | |
| Matroska, WebM | Cues for keyframes plus a header-only block scan | Cues list only keyframes, so per-frame timestamps need the block headers; BlockAdditional (alpha) locations recorded |
| MXF | Index table segments via the Random Index Pack | Temporal offsets (long-GOP MPEG-2), constant edit-unit sizes, clip-wrapped OP-Atom |
| AVI | OpenDML `indx`/`ix##` or `idx1` | Codecs without reordering only |
| Raw DV | Constant frame size | |
| MPEG-TS, M2TS, MTS, MPEG-PS, VOB, FLV, ASF/WMV, Ogg | FFmpeg demux scan | No frame tables exist; payloads are demuxed, nothing decoded |
| AVI or elementary streams with B frames | Demux scan plus a decode-order pass | See below |

H.264 and HEVC sync samples in MOV/MP4 and Matroska are tagged with their
first VCL NAL type (one 64-byte read per sync sample, never non-sync samples),
so open-GOP sync samples (HEVC CRA and BLA, H.264 non-IDR I) and their leading
pictures (RASL) are known. On a 108,000-frame H.264 MP4 (432 sync samples) this
takes 3.7 ms of a warm-cache index build, against 155 ms when every sample's
headers were read one byte at a time. A decode for frame N starts at the
last sync sample not after N and skips that sync sample's leading pictures.

**Scanned packets** are addressed by byte position plus their ordinal among
the stream's packets at that position: Ogg reports the page position for
every packet that starts on a page. Reads seek to an earlier packet and read
forward; a backwards timestamp seek is the last attempt, for demuxers whose
byte seeks drop the packet they land on (ASF loses the first packet of the
data object).

**Streams without presentation timestamps on a reordering codec** (H.264 or
MPEG-4 part 2 with B frames in AVI, raw MPEG-2 elementary streams) present in
decode order if indexed naively. The index runs one decode pass with IDCT
and loop filter skipped, each packet tagged with its decode ordinal, and
takes the output order as presentation order; the sorted decode timestamps
are the presentation slots. Pictures the decoder never outputs are hidden.
The index reports `DemuxScanDecodeOrder`. Cost on the corpus: 2.7 to 5.4 ms
for 75 frames at 320×180.

## Timing

- Edit lists, composition offsets (`ctts` version 0 and 1, so negative CTS),
  and stream start offsets are applied in the index; `Frame::pts` is on the
  presentation timeline (PTS 0 is the start).
- Priming: MP4 edit list, then `iTunSMPB`, then the LAME/Xing header plus the
  529-sample decoder delay, then FFmpeg's codec delay; 2112 only for
  Apple-encoded AAC without metadata (`probe::timing`).
- **Nominal rate** (`time::nominal_rate`): the supported rate within ±1
  timebase unit of the median duration over the first 300 frames; with
  several candidates, the one closest to the mean duration over the same
  window (amendment 2); otherwise (last − first) / (frames − 1). The method is
  reported (`Median`, `MedianThenMean`, `Fallback`).
- **Source frame selection** (`time::select_frame`, section 5 rule 7): the last
  frame with PTS ≤ t + q/2, q the timebase unit capped at a quarter of the
  nominal frame. All comparisons are exact rationals (i128, widened to 256 bits
  when a cross product could overflow). Unit tests cover 20,000 frames at
  23.976, 29.97, and 59.94 on 1 ms, 90 kHz, and 1/600 timebases, holes in VFR
  streams, and negative origins.

## Decoders

| Codec | Backends, in order |
|---|---|
| ProRes (all) | VideoToolbox only (amendment 13) |
| H.264 4:2:0 | VideoToolbox, FFmpeg |
| H.264 4:2:2 or 4:4:4 | FFmpeg, VideoToolbox |
| HEVC, HEVC with alpha | VideoToolbox, FFmpeg |
| AV1 | VideoToolbox where the GPU decodes AV1 (M3 and later), dav1d |
| VP8, VP9 | FFmpeg native; with alpha, two paired FFmpeg native decoders |
| DNxHD/DNxHR, CineForm, MJPEG, DV, MPEG-2, MPEG-4, H.263, WMV, ... | FFmpeg |

**VideoToolbox** (`vt.rs`). The format description is built from the sample
entry's child atoms (as AVFoundation does), Matroska CodecPrivate, `avcC`/`hvcC`
extradata (FLV), or Annex B parameter sets (MPEG-TS, AVI). Output formats:
ProRes 422 to `x422` for display and `v216` when the decode can reach an
export (`DecodePurpose::Export`, amendment 6); ProRes 4444 and XQ to `y416`;
10-bit H.264 and HEVC to `x420`/`x422`; 8-bit to `420v`/`420f`; HEVC with alpha
to `v0a8`. Range follows the bitstream and every buffer's format and
attachments are checked against the request. A seek inside a session first
submits an end-of-sequence NAL unit (H.264 type 10, HEVC type 36) as its own
sample (S6 finding 1).

**FFmpeg** (`ff/decoder.rs`). A custom `get_buffer2` puts every plane in pool
memory with its own `AVBufferRef` (S6 finding 2). Decoders that ignore
`get_buffer2` are copied once and flagged `copied`; the regression suite
requires every file to reach the GPU through at least one backend with zero
copies.

**dav1d** (`dav1d.rs`) decodes into pool slots through its picture allocator.
**VP8/VP9 alpha** (`ff/alpha.rs`) pairs two native decoders, color and the
BlockAdditional alpha stream, both rendering into the pool; the suite checks
them against FFmpeg's libvpx decode.

**Sessions** (`decode/session.rs`) give exact random access: continue the
current run when it already passed the needed random-access point, else reset
(end-of-sequence for VideoToolbox, flush for software) and feed from the sync
sample. **The decode service** (`decode/service.rs`) pools sessions per source,
playback instance, and purpose and reuses them across seeks; plans read-ahead
by direction, speed, and GOP (reverse playback decodes GOP by GOP from the
newest); fetches neighbor frames for deinterlacing; and asks a
`HardwareArbiter` before creating a hardware session (without a permit,
ProRes uses VideoToolbox's software decoder and other codecs fall back to the
next backend).

## Frame cache and leases

`cache::FrameCache` holds decoded frames by source, sample, and purpose under a
byte budget, least recently used first. A frame evicted while a renderer
still holds it stays alive.

`MediaUsage::lease_categories` reports the contract 8 frame-cache and decoder
categories, each decoded frame counted once. The frame cache counts the
CoreVideo and pool frames it holds. The decoder category counts every other
decoded frame the process holds: pool slots outside the cache (decoder
references, session windows, frames held after eviction, free slots) and
VideoToolbox's CoreVideo buffers outside the cache. VideoToolbox buffers are
counted through a hold in the frame pool (`FramePool::hold_external`) that
lives as long as any clone of the frame, wherever it is (a session window,
the cache, a renderer). VideoToolbox's internal buffer pools and session state
are not visible to the process and are not in either number.

`DecodeService::apply_lease(MediaLease { frame_cache, decoders })` enforces
both shares. The cache evicts down to its share. Over the decoder share, idle
sessions drop their windows, free pool slots are released, and idle sessions
close until usage fits. A zero share closes every idle session, because the
session state VideoToolbox keeps cannot be measured. A session in use gives its
window back after each request while usage is over, and closes when it returns
if usage is still over (always under a zero share). Frames a renderer holds
stay counted until the renderer lets go, so the acknowledgement can report
more than the lease only while that happens.

## GPU import and normalization (`pm-gpu::decode_in`)

CoreVideo buffers come in per plane through `CVMetalTextureCache` (`R8`/`RG8`,
`R16Unorm`/`RG16Unorm`; `y416` and `v216` as one `RGBA16Unorm`), checked to be
the buffer's IOSurface and plane. Pool frames are wrapped once per slot with
`newBufferWithBytesNoCopy`, each plane a linear texture at the address the
decoder wrote. Both go into wgpu through `texture_from_raw` and
`create_texture_from_hal`. `normalize.wgsl` applies the section 13 constants
(MSB-aligned c = v / 64; `y416`; LSB-aligned n-bit; full range) and the result
is compared with an f64 CPU reference: at most 1 ULP of f32 on every corpus
layout. bwdif deinterlacing runs on normalized Y′CbCr before chroma
upsampling, to field rate or frame rate per `interlace::plan`, and is compared
against FFmpeg's bwdif.

## Content identity

`hash::BackgroundHasher` computes full BLAKE3 hashes on one utility-QoS
thread, waiting until a file's size and modification time have been stable
for the settle period and starting over if the file changes while hashed.
`hash::sampled_fingerprint` is today's v2 fingerprint (byte-identical to the
TypeScript). `hash::ContentHints` uses fingerprints only to find candidates;
`resolve` with the full hash decides, and reports fingerprint collisions
(appendix A item 22).

## Images

The `image` crate (PNG 8/16, JPEG, TIFF 8/16/32f, BMP, ICO, GIF, WebP),
ImageIO (HEIF/HEIC, AVIF, animation frames, everything else macOS reads),
OpenEXRCore (scanline and tiled, every compression including DWAA/DWAB and
HTJ2K), FFmpeg for DPX. Pixels are returned as stored at the file's depth with
the ICC profile classified, EXIF orientation applied where Chromium applies it
(not WebP EXIF, which Blink's WebP decoder never reads). Image sequences are
found from one member, with gaps reported.

## Tests

`cargo test -p pm-media -p pm-gpu` runs the e2e fixtures. The generated
corpus (`engine/crates/pm-media/tests/corpus/gen.sh`, synthetic, written to
`~/Library/Caches/powermove-engine/media-corpus`) runs with `--ignored`:

```sh
source engine/env.sh
cargo test -p pm-media -p pm-gpu --release -- --include-ignored --test-threads=2
```

The random-access suite (`tests/random_access.rs`) is the S6 harness as a
regression suite: for each file and each backend that decodes it, every
composition frame selected by rule 7 is fetched in shuffled order with a new
session per probe and within one session, and must equal the sequential decode
of that frame; backends must agree on normalized output; sequential decodes
must match our FFmpeg build's framemd5 (or its VideoToolbox hwaccel for
VideoToolbox output formats it can download); and some backend must reach the
GPU without a copy. Groups: the S6 corpus; ProRes in MXF and Matroska; ProRes
422 export-bound (`v216`); interlaced MXF (XDCAM HD422 long-GOP 1080i, DNxHD
1080i, OP-Atom); DNxHR HQ, HQX, 444; CineForm MOV and AVI; DV (NTSC raw, PAL
AVI, DVCPRO50); MJPEG AVI and MOV; fragmented MP4, negative CTS, VFR, MPEG-PS;
and container breadth (ASF, FLV, M2TS, MTS, 3GP, 3G2, Ogg, M2V, M4V, AVI with B
frames).

Measured 2026-10-12 on the M4 Max with other lanes running (release build):
68 files, 103 file and backend runs (a few files run in two groups), 17,330
random accesses (both passes), 0 mismatches, 0 copied frames. Mean time per
random access with a new session: VideoToolbox 1.6 to 23.5 ms (23.5 is 4K HEVC
Main10), FFmpeg 0.21 to 117 ms (117 is 4K HEVC Main10 in software), dav1d 0.7
to 2.9 ms, alpha pairs 2.2 to 3.6 ms. Slowest single probe: 189 ms (FFmpeg,
4K HEVC Main10); the suite fails any probe over 2 s and aborts one running past
60 s. Index builds: 0.03 to 1.1 ms from tables and scans on the corpus, 3.7 ms
for a 108,000-frame H.264 MP4, 2.7 to 5.4 ms with a decode-order pass.

## Not done here

- Main-side hardware session arbitration over the protocol: the
  `HardwareArbiter` trait is the seam; `LocalArbiter` enforces limits in one
  process.
- Persisting indexes and probes in the app cache (section 14.7) belongs to the
  cache lane; `FrameIndex` serializes with serde for it.
- `v216` output has no independent reference (FFmpeg cannot download it from
  VideoToolbox); it is checked for random access equal to sequential and by
  the normalization reference in `pm-gpu`.
- Base M1/M2 behavior (no ProRes engine, no AV1 hardware) is untested
  (decision 11).

## Media closure: source previews and failures (Phase 3c)

`pm_media::preview::PreviewReader` is the engine-side source API for poster
and filmstrip consumers. `open_with_service` shares the engine's
lease-controlled `DecodeService`; `open` supplies a small convenience service.
`poster` selects `min(0.5, duration * 0.1)` seconds, `frame_at` reads an exact
source time, and `filmstrip` streams up to 64 caller-ordered requests through
a callback, stopping on cancellation or an error. All requested times are
validated before the first callback. Dropping the reader releases its decode
sessions. `image_bytes` counts outstanding still frames, including frames a
caller retains across source replacement.

The API returns native-depth frames and their color facts. Image orientation
is already applied (as Chromium applies it; WebP EXIF is ignored); `video_probe` supplies video geometry, field, and HDR
facts for the renderer. The consumer must apply source interpretation, video
geometry, resizing, and thumbnail encoding. This is a source decode API;
the renderer's browser poster and filmstrip consumers have not been switched
to it in this change.

`image::animation::read_timing` reads original GIF, APNG, WebP, and AVIF timing.
`AnimationTiming` implements the shipped importer's delay normalization, GCD
slots, cumulative resampling, frame budget, boundary epsilon, and final-frame
hold. It does not change the stored project representation: existing imports
still persist their WebM conversion. `PreviewReader::open_sequence` reads
exactly the host's selected numbered paths, permits mixed padding, rejects
duplicate numbers, and plays available frames consecutively across gaps.
The older directory-discovery and numbered-gap-hold APIs remain available.
Neither API introduces an original-sequence persistence format or changes
copy/link import semantics.

Media probing now distinguishes unsupported readings from unreadable bytes.
Offline or corrupt PNG, SVG, video, and sequence sources produce a named
`LayerFailure`; they do not add an unsupported-rendering reason. The native
presenter also keeps offline media in the engine and sends runtime layer
failure events before the frame, deduplicated until the error or revision
changes. A recovered layer can report a later failure at the same revision.
Other existing capability gates remain.

Imports await the host's media-binding acknowledgement before returning.
This closes the 250 ms binding window in which an immediate native export
could mistake an imported asset for offline media. Linked originals still
resolve through their link; a generated rendition does not replace that
binding. A hidden app regression covers UHD ProRes rendition creation and
asset restore while preserving the original hash and modification time.

### Level-1 still bitmaps and persisted animations

`image::compat::bitmap` builds the 8-bit premultiplied surface before texture
filtering. PNG16 drops the low byte first; ICC converts and clips to sRGB
with Chromium 152's pinned skcms; PNG/JPEG use Blink's rounded premultiply.
WebP uses libwebp's exact `(channel * alpha * 32897) >> 23` with the opaque
fast path. This distinction matters at products exactly divisible by 255.
RGB TIFF retains its ICC, including UNDEFINED-type tag 34675, and its
compatibility bitmap equals the shipped FFmpeg-to-PNG bitmap byte for byte.
The same check covers unsigned RGB/RGBA TIFF at 8 and 16 bits. Associated
TIFF alpha is retained as premultiplied. SDR PNG cICP matching its P3 or
sRGB ICC profile is accepted; conflicting or HDR cICP remains gated.

SDR AVIF uses the Chromium 152 revisions of CrabbyAvif and libyuv, with the
existing pinned dav1d, into an 8-bit premultiplied BGRA surface. Sources,
checksums, licenses and the transitive Rust lock are in `pm-media/native`.
The [Chromium AVIF decoder](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/renderer/platform/image-decoders/avif/avif_image_decoder.cc)
and [DEPS](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/DEPS)
identify this pipeline. Level 2 and Interpret Footage keep the native-depth
reading; the still texture cache includes the compatibility choice. A
1 → 2 → 1 render regression checks that the old bitmap never contaminates
the high-precision cache.

All 21 detailed `render/still-*` corpus projects now draw, including the
15-project still gate classes. Each opaque comparison is within 1 CV at
every pixel; transparent outputs are below 1 CV. `tests/still_compat.rs`,
`tests/corpus.rs` and `tests/stills.rs` enforce the existing bars. Other
unmeasured container structures remain explicit in `stills.rs`. An additional
opaque P3/Adobe RGB PNG/TIFF app capture checks the ICC-to-sRGB boundary.

An additional translucent P3/Adobe RGB PNG/TIFF capture fails the unchanged
2D threshold: quantization around ImageBitmap premultiplication/color
conversion still differs. Those combinations retain their explicit gate;
opaque RGBA containers remain supported by inspecting actual coverage.
The failing capture and side-by-sides are recorded outside Git in the lane
report. Chromium's `createImageBitmap(blob)` decodes a premultiplied source
bitmap, then WebGL's `TexImageStaticBitmapImage` converts its color space;
it is not the HTMLImageElement extractor's unpremultiplied re-decode. See
[ImageBitmap](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/renderer/core/imagebitmap/image_bitmap.cc)
and [WebGL upload](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/renderer/modules/webgl/webgl_rendering_context_base.cc).
The opaque skcms match does not prove this alpha conversion order.

Existing GIF/APNG/WebP/AVIF and numbered-sequence imports store a WebM.
The composition renderer identifies the actual EBML bytes, validates the
video source and renders that rendition through its ordinary video path.
It uses `sequencePlaybackTime`'s CFR slots, 1e-7 boundary epsilon, 1 ms seek
inset and final-slot hold, rather than ordinary video's 40 ms end padding.
An untagged rendition keeps the converter's BT.601 matrix even at HD sizes;
explicit file and Interpret Footage matrices take precedence. This source
reading is part of the converted-frame cache key and disables YUV pass-through
whose generic file interpretation would differ.

Existing level-1 renditions also reproduce Chromium's software video upload:
pinned libyuv I420 → clipped/premultiplied RGBA8 with nearest chroma before
texture filtering. The generic native chroma filter alone smeared color
into pixels counted as patch interiors at the historical projects' 2×
scale. The RGBA8 path makes all nine historical frames exact on their
55,892 video-class interior pixels and exact in alpha. The capture manifest
binds import hashes to original GIF/APNG/WebP files, so the test resolves
those unchanged asset IDs to the actual app-persisted rendition, leaving the
historical documents, transforms and sample times unchanged. Hidden app
comparisons also cover all three sources through import, seek and export.
[Chromium's video conversion](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/renderers/paint_canvas_video_renderer.cc)
and its [default nearest filter](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/renderers/paint_canvas_video_renderer.h)
define that software upload.

Level 2 keeps the native chroma conversion; it compares to FFmpeg + zimg
with the same filter within 1 CV, including chroma edges. The level-1
libyuv bitmap itself differs from nearest-filter zimg by up to 2 CV in
saturated blue: that independent ≤1 CV line is still a gap, not a relaxed
threshold. Its failed measurement and side-by-sides are in the lane report.
The oracle strips alpha before zimg and compares alpha separately; zimg
has no `yuva422p16le`, which would silently insert swscale chroma conversion.
Original animation APIs remain available for new-import work. Today's conversion and storage policy
is unchanged; preserving originals for new imports is outside this round.

### Container transforms, AVIF structure and SVG encodings (review round 3)

Orientation and container transforms are drawn only as Chromium shows
them, and each class has an app comparison:
`e2e/native-still-orientation-parity.spec.ts` imports 54 generated stills
with Use New Engine off and on, exports both over an opaque background and
requires the drawn cases to keep the box today's import measured, route to
the engine, and stay within the 2D class (max ≤ 4, ≤ 0.1% of pixels above
1 CV, alpha ≤ 1). Measured 2026-10-18: every drawn case is max 0 or 1 CV
with no pixel above 1.

- PNG `eXIf` before the image data, orientations 2 to 8, opaque and
  translucent; JPEG (Y′CbCr, all eight orientations, and gray); TIFF tag
  274, 2 to 8 and associated alpha (the kept original against today's
  FFmpeg PNG conversion); HEIF `irot`/`imir` 2 to 8 and `clap` (ImageIO
  against today's `sips` PNG, which carries the orientation as `eXIf`).
  `stills::check` requires the decoder's applied orientation to equal the
  container's; late PNG `eXIf` (Chromium reads chunks before IDAT only)
  stays explicit.
- WebP EXIF: Blink's WebP decoder ignores it, so `pm-media` no longer
  applies it (the reported `still.webp` with orientation 6 is drawn
  160×90, as stored). Opaque and translucent cases match.
- AVIF follows Blink's `AVIFImageDecoder` in `avif_compat`: `irot`/`imir`
  through `kAxisAngleToOrientation`; a valid `clap` anchored at (0, 0)
  crops the decoded planes before conversion (`avifImageSetViewRect`, as
  Blink); any other or invalid `clap` shows the whole image. Strict clap
  validation is off and EXIF ignored, as in Blink. Translucent AVIF, all
  three quarter turns, both mirrors, `clap` at and off the origin, and
  limited range (`e2e/fixtures/still-limited-range.avif`, SVT-AV1) match.
  `avif_compat::unmeasured` keeps 4:4:4, 4:2:2, monochrome and deeper
  AVIF explicit: no available encoder writes those, so they were not
  compared.

Chromium's oriented ImageBitmap is a pixel permutation of the unoriented
premultiplied bitmap: the WebGL texture of a translucent PNG with `eXIf` 6
equals the rotated texture of the same PNG without it, and both equal the
compatibility bitmap byte for byte (texture readback through
`texImage2D(UNPACK_PREMULTIPLY_ALPHA_WEBGL)`, as `compositor.ts` uploads;
translucent AVIF is also exact). `tests/stills.rs` pins the permutation,
the WebP rule and every AVIF transform without the app.

Translucent exports with a transparent background differ from the legacy
export by up to 2 CV in color at high alpha, for unoriented and oriented
stills alike, while the bitmaps above are exact: legacy's export loses up to
2 CV against the source values and native does not. This is an export
compositing difference outside this lane, recorded with its evidence in the
lane report; the orientation spec composites over an opaque background so
it measures the media.

Level 1 SVG images are routed without being read: usvg accepts UTF-8 only,
so a parse at routing time misreported ISO-8859-1 or UTF-16 files that
Chromium's `<img>` shows. An offline level-1 SVG remains a named layer
failure. Level 2 decodes SVG bytes as Blink's XML decoder does
(`image::svg::source_encoding`: byte order mark, UTF-16 by `<?`, the XML
declaration's label through the WHATWG registry, a UTF-16 label on 8-bit
bytes meaning UTF-8, malformed sequences replaced) before usvg; the result
draws the same pixels as the UTF-8 file. Level-1 routing no longer parses
SVGs at all, which removes the routing-time cost and the system font load
for text detection there.

`tests/still_compat.rs` and the animation corpus test are `#[ignore]`d and
fail when their private capture is missing; run them with
`cargo test -p pm-render --test still_compat --test media_sequences -- --include-ignored`.
The animation test binds the app-persisted WebM from
`Powermove Engine Corpus/animation-renditions/<fixture>/native-source.webm`
(or `PM_MEDIA_RENDITIONS`) with no re-encode fallback: all nine frames are
max 0 CV and alpha 0 against Chromium.

`PreviewReader::poster` seeks raw source time for every video, including a
stored sequence or animation WebM. That is today's behavior: `poster.ts`
seeks the asset's video element to `min(0.5, duration × 0.1)` without
`sequencePlaybackTime`'s inset.

### Poster and filmstrip consumer call surface

The consumers lane can resolve its authorized asset handle to the stored
source path, then call these existing Rust methods on a worker:

```rust
let mut reader = PreviewReader::open_with_service(path, decode_service.clone())?;
let duration_seconds = reader.duration()?;
let video_facts = reader.video_probe()?;
let poster: PreviewFrame = reader.poster()?;
reader.filmstrip(&source_times, |index, seconds, frame| {
    // Apply the asset's interpretation, geometry, color, resize and encoding;
    // stream one completed thumbnail, or return an error to cancel the job.
    consume(index, seconds, frame)
})?;
```

`PreviewFrame::Image(StillImage)` carries applied orientation, native-depth
RGBA and ICC/transfer/alpha facts. `PreviewFrame::Video { frame, color }`
carries native planes and the resolved color description; `video_probe`
adds display geometry, interlace and HDR facts. `decoded()` borrows either
frame without copying. Caller order is retained, duplicate times are legal,
and all 1..64 times must be finite and nonnegative before decoding starts.
The callback owns each frame; retaining it retains the decode allocation.
Dropping the reader releases sessions. `image_bytes()` includes still frames
held by the consumer across source replacement. `open_sequence` takes the
exact selected paths and rate when a source is an original numbered sequence.

No new IPC method is implied by this snippet. Capability resolution, job and
cancellation transport, thumbnail encoding and UI consumers belong to the
consumers lane; deprecated browser APIs remain available at apiVersion 1–3.

### SVG compatibility gap

`pm_vector::canvas2d::svg` now replays resolved usvg paths through the vector
lane's Skia emulation, with original SkPath geometry, even-odd fill, strokes,
dashes, opacity layers, gradient shaders, clip unions and linear-RGB Gaussian
layers. Horizontal SVG text shapes and paints through the caller's `pm-text`
resolver (`raster_compat_with_text`). This remains a diagnostic API: only
15 of the 27 canvas SVG cases satisfy the 2D class. Text, gradients, larger
blurs and some non-square bucket edges still fail. Level-1 `SvgImage` stays
gated, and no 34-project composition pass is claimed. `vector.md` records
the source pipeline and measured failures. No tolerance changed.
