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
the ICC profile classified, EXIF orientation applied. Image sequences are found
from one member, with gaps reported.

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
