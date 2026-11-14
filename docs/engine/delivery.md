# Delivery: `pm-export` and the app's FFmpeg

Design notes for plan section 17 (export and delivery), section 24 item 1 (FFmpeg),
and Phase 0 amendments 6, 7, 8 and 13. Branch `feat/ne-delivery`.

## `pm-export`

```text
working frame (premultiplied RGBA, E or linear)
  → encoder input (convert.rs: matrix, chroma siting, dither, clipping)   y416 | v216 | 420v
  → Sink
      VideoToolboxSink: VTCompressionSession → AVAssetWriter → verify → atomic move
      test sinks: MemorySink, RawFileSink (feature `test-sinks`)
```

`pm_export::export(spec)` returns a `Pipeline<VideoToolboxSink>`:
`push_frame` (CPU conversion), `push_encoder_input` (planes converted elsewhere: the
GPU encoder-input pass, or Y′CbCr pass-through), `push_audio` (interleaved f32 PCM),
`finish` → `Delivered { path, report }`, `cancel`. `ExportSpec` carries size, rate,
frame count (known up front: it sizes the timecode track and is verified), codec,
container, tags, alpha, working encoding, dither, audio, timecode, destination,
overwrite policy, and file mode (agent exports use `0o600`).

### Encoder input

| Codec | Input | Clip (codes at input depth) | Dither LSB | Dither default |
|---|---|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ | `v216` (amendment 6) | 10-bit 4..1019 (256..65216) | 64 units | off (opt-in) |
| ProRes 4444, 4444 XQ | `y416` | 12-bit 16..4079 (256..65264); alpha full range | 16 units | off (opt-in) |
| H.264 High | `420v` | 8-bit 1..254 | 1 code | on from 1.5 bits per pixel per frame (`H264_DITHER_BITS_PER_PIXEL`, measured in `tests/dither_delivery.rs`); off below, where the encoder codes the noise for no less banding (EXPORT-DITHER-1) |
| PNG, 8-bit | RGBA8 | 0..255 | 1 code | on |

`convert.rs` is S2's prototype pass (`docs/engine/spikes/S2.md`) in
f64 on the CPU: Rec.709 matrix; chroma of rendered frames band-limited by the tent
kernel at each output's siting (`[1, 2, 1] / 4` across on the co-sited column for
4:2:2 and 4:2:0 type 0, `[1, 3, 3, 1] / 8` between rows for 4:2:0), and of untouched
footage (`Renderer::pass_through`, a 4:2:x Rec.709 source at the output's siting) by
the exact inverse of the renderer's upsampling; round half to even (WGSL `round`),
then clipping. Opaque outputs take premultiplied values (over black); straight alpha
unpremultiplies below a 1/4096 guard; premultiplied output re-premultiplies in E when
the working values are linear. It is the reference the GPU pass must match and the
fallback when no GPU pass runs.

### Dither

The 64×64 void-and-cluster tile is generated with `libm::exp`, so it is identical on
every platform; its SHA-256 is pinned in `dither.rs`. macOS's own `exp` gives a tile
that differs in 15 of 4,096 ranks (S2's prototype used it), which is why the portable
`exp` is used. Offsets are `(rank + 0.5)/4096 − 0.5`; each frame and channel shifts the
tile by `pcg_hash(frame × 4 + channel)`. Values on the output grid and neutral chroma
are unchanged with dither on (every 8-bit luma code 16..235, tested), alpha is never
dithered, and on S2's shallow ramp (0.40 → 0.42, 1,920 columns, 64 rows) the
16-column windows of column means are within 0.05 CV dithered and 0.4 or more apart
undithered (tested).

### Muxing

- Color tags are set as VTCompressionSession properties for every codec (amendment
  7) and as buffer attachments. 1-13-1 is "Rec.709 (QuickTime and Safari)" (the
  default: QuickTime and Safari show it as the viewer does); 1-1-1 is "Rec.709", the
  label broadcast QC and colour-managed timelines expect. Both carry the same codes:
  the choice relabels, it never re-encodes (`tests/transfer_tags.rs`).
- Movie and media timescales follow section 17 for all 16 rates (NTSC numerator with
  frame duration 1001, integer f × 1000 with 1000). AVAssetWriter refuses a
  non-default audio timescale; its default is the sample rate, which section 17 asks
  for, and verification checks it.
- Audio: 24-bit LPCM in MOV (today's ProRes export), AAC (128 kbps per channel) in
  MP4, encoded by AVAssetWriter from f32 PCM.
- `tmcd`: one sample with the start frame, `kCMTimeCodeFlag_DropFrame` at 29.97 and
  59.94 DF, the reel as the source reference name, associated with the video track.
  AVAssetWriter refuses a timecode track in MPEG-4, so timecode requires MOV
  (`validate`), and the timecode must count the export's own rate.
- AVAssetWriter interleaves inputs and holds one back while another lags, so samples
  are queued per input and appended when that input is ready; nothing blocks on one
  input while another starves. Callers should push audio as frames advance: audio
  pushed only at the end still works (tested) but holds compressed video in memory
  until it arrives.
- AVFoundation reports misuse by raising Objective-C exceptions; `objc2`'s
  `exception` feature turns them into `Error::Mux` instead of aborting the process.

### Atomic output

The encoder writes `.<name>.<random>.partial.<ext>` in the destination folder. On
success the file gets its mode and is renamed (`Overwrite::Replace`) or hard-linked
and unlinked (`Overwrite::Fail`, no-clobber even when a file appears during the
export). Cancel, failure, or drop removes the temporary file and AVAssetWriter's
`.sb-<uuid>` work files.

### Verification

After `finishWriting` and before the move, `mp4box.rs` reads the container and
`mac/reader.rs` decodes every frame through AVFoundation to the encoder-input format,
four readers at once over their own runs of frames (`software.rs` streams them from
FFmpeg; WebM decodes its first, middle and last frame). The check is its own phase of
the export, after every frame is encoded: the job reports `Progress::checked`, the
engine job `JobStatus::checking`, and the sheet shows "Checking the file · frame k of
N" with Cancel. On a UHD ProRes 422 HQ export of 120 frames (debug test build, load
average 141) the frames took 6.8 s to hand over and the finish, check included, 0.9 s;
the check's decode alone is 4.1 ms a frame with one reader, 3.1 ms with four.

The file is moved into place only when the check passes. Otherwise it is removed and
the person is told one sentence naming the first thing found ("The exported file
failed its check and was removed: frame 3 shows the picture of frame 2."). Nothing is
retried; Try Again is the person's.

#### The file check: a failed export is only for what is certain

An export fails only on evidence that cannot be legitimate codec loss at any data rate
the export admits. Anything that is a matter of degree is judged against what the same
encode does elsewhere, or reported to the person as a note with the file kept
(`VerifyReport::notes`). A threshold that refuses a correct export is wrong.

Fails the export:

- Container and stream facts: sample entry (codec), profile, pixel format, depth,
  `colr` and the H.264/HEVC VUI colour description, range, frame count, every frame's
  time and duration, movie and media timescales, duration, audio codec, rate, channels,
  presented samples and length, alpha (QuickTime depth 32), timecode (start frame,
  drop-frame flag, rate, reel, track reference).
- A frame that is not the frame it should be. Every frame keeps an 8 × 8 grid of the
  means of Y′, Cb, Cr and alpha (`evidence.rs`); a cell is the mean of 1,024 samples of
  an eighth of the frame each way, which every codec keeps at every rate. A frame three
  times nearer a neighbour's reference than its own, while its own distance is three
  times the export's typical one, shows that neighbour's picture (stale, repeated,
  swapped); a frame eight times the typical distance, or one cell eight times the
  typical largest cell, is not its picture.
- A channel lost across a frame: the frame keeps under a quarter of the picture its
  luma, colour or alpha grid carries, and little of any other.
- A level, range or matrix mistake: a shift of most of the picture (the median cell)
  that every frame shares beyond `level_tolerance`, or a gain of every frame's grid 5 %
  or more from one (a video/full range mistake is 14 %). Quantisation of the DC moves
  every frame of a slate, a flat, dark or starved picture by whole steps, so the
  allowance is twice the largest such shift measured at any rate from 0.1 Mb/s
  (`tests/hard_delivery.rs`): H.264 10 codes (measured 5, at 0.1 Mb/s), HEVC 2, HEVC
  Main10 10 of 10 bits, ProRes 2 of 10 bits (measured 0.48: Proxy, magenta slate),
  4444 8 of 12 bits. A smaller shift in H.264 or HEVC Main10 is not certain and is
  delivered.
- Where each block is coded from its own samples (ProRes; a long-GOP codec's first
  frame), a 16-pixel block of a sampled 48 × 48 region that went flat, or holds a
  picture its reference does not explain, while its neighbours of the same kind kept
  theirs; and in every codec, a picture displaced as a whole (most sampled regions
  nearer the reference moved a pixel).

Noted with the file kept: one damaged-looking block of a long-GOP predicted frame (at a
low rate a skipped or intra-predicted block shows a picture that is not the frame's
own), texture lost, contrast, correlation of fine detail, per-patch colour error.

`tests/hard_delivery.rs` holds the rule's other side: 167 correct exports at the hard
places (0.1 to 4 Mb/s H.264 and HEVC in HD and UHD with grain and small text, long-GOP
cuts and a flash frame, primary and secondary slates in every ProRes flavour, very dark
and flat frames, one-frame exports, soft alpha, 12-bit ramps, PQ and HLG, heavy noise)
must all be delivered. Their margins: identity ratio at least 0.88 (refused under 1/3),
least channel kept 0.66 (under 0.25), gain within 0.008 (beyond 0.05), largest frame
2.8 times the typical distance (8 times). `tests/picture_check.rs` holds the certain
failures in real files: a level shift (ProRes), a range mistake, a picture moved a
pixel, a stale frame and a swapped pair (also colour-only and alpha-only), a frame
without its colour, alpha in other places, a foreign block, and in ProRes a flattened
or garbage block.

What the check cannot tell: a stale or swapped frame whose neighbours look alike at the
grid's scale, damage smaller than a grid cell away from the sampled regions, one wrong
block of a long-GOP predicted frame, and a shift within quantisation.

### Measured (M4 Max, macOS 27, release build)

| Export | End to end | CPU convert share | Max block error |
|---|---|---|---|
| ProRes 422 HQ 1920×1080, 48 frames | 55.1 fps | 10.4 ms/frame | 0.068 CV (10-bit) |
| ProRes 4444 1920×1080, 48 frames | 41.6 fps | 15.1 ms/frame | 0.201 CV (12-bit) |
| H.264 16 Mbps 1920×1080, 48 frames | 48.0 fps | 13.0 ms/frame | 1.078 CV (8-bit) |
| ProRes 422 HQ 3840×2160, 24 frames | 15.2 fps | 42.6 ms/frame | 0.066 CV |
| ProRes 4444 3840×2160, 24 frames | 11.8 fps | 59.4 ms/frame | 0.225 CV |

`cargo run --release -p pm-export --example throughput -- <hq|4444|h264> W H N`.
End to end includes generating the test picture, the single-threaded CPU conversion,
encoding, muxing and verification. VideoToolbox reported a hardware encoder in every
case. In the 320×180 test exports, the worst block error was 1.61 CV for ProRes 422
Proxy and at most 0.24 CV for every other flavor.

## The app's FFmpeg

`ffmpeg-static` is gone from `apps/desktop`, `packages/cli` and the lockfile.

- `engine/third_party/ffmpeg/package.sh` verifies a build and stages what ships into
  `apps/desktop/vendor/ffmpeg` (gitignored): `bin/ffmpeg`, `bin/ffprobe`, the seven
  dylibs under their install names, `share/licenses/`, `manifest.json`, and
  `NOTICE.txt` (LGPL notice with the exact source URL and SHA-256).
- electron-builder copies it to `Contents/Resources/ffmpeg`; `dist:mac` and
  `release-mac.mjs` stage it first; `after-pack.mjs` refuses a bundle without it;
  `release-mac.mjs` checks that every binary and dylib there has the app's Team ID and
  the hardened runtime and that the signed `ffmpeg` runs and reports the LGPL.
- `src/main/ffmpeg-binary.ts` resolves it: packaged apps use only
  `Resources/ffmpeg`; development and tests use `$PM_FFMPEG_DIR`, then the staged
  copy, then the shared build cache. Export, proxies, transcription and the agent
  media tools all receive that path.
- The serve and MCP hosts (`src/server/cli.ts` `hostFfmpeg`) use
  `$POWERMOVE_FFMPEG`, then `$PM_FFMPEG_DIR` or the shared build, then `ffmpeg` from
  PATH.
- `render-encoder.ts` (the legacy exporter) encodes with `h264_videotoolbox` (High,
  `-g 250`) and `prores_videotoolbox -profile:v 4444` (`ayuv64le` with alpha,
  `p416le` opaque, which writes no alpha channel), `-allow_sw 1`, and frames tagged
  1-13-1 with `setparams` because VideoToolbox takes primaries and transfer from frame
  attachments.

### Dolby Vision and HDR10+

HLG HEVC carries Dolby Vision Profile 8.4 unless the person turns it off (the Export
sheet's "Dolby Vision 8.4" row; `DeliveryOptions::without_dolby_vision`): macOS's encoder
adds a `dvvC` record and an RPU on every frame in its default HDR metadata mode, and the
file check holds the record (profile 8, HLG compatibility id 4, RPU, no enhancement layer).
The HLG base layer decodes the same either way (`tests/dolby_vision.rs`). Both choices set the
encoder's HDR metadata insertion mode (Auto or None) rather than relying on the
session's default; an encoder that reports the property unsupported fails the export
as its session opens, before any frame, with the VideoToolbox error. That has not been
seen on the macOS 27 encoders tested. The software (FFmpeg) sink writes no RPUs, so it
refuses Dolby Vision on; native and software file checks share one rule for the record
(`verify::expected_dolby_vision`, `verify::dolby_vision_problem`). An imported 8.4
clip is read as its HLG base layer and named in the Project panel and Interpret Footage.

HDR10+ (ST 2094-40) is not preserved, and not generated. Tried on macOS 27 with a
conformant HDR10+ stream (FFmpeg reads its SEI): the documented per-frame key
(`kCMSampleAttachmentKey_HDR10PlusPerFrameData`) is written by neither AVAssetWriter
(on encoded samples) nor VideoToolbox (on input buffers or frame properties). Apple's
decoder puts each frame's bytes on the decoded buffer under an undocumented attachment,
`HDR10PlusData`, and its encoder writes those back exactly when the session is tagged PQ
with insertion mode Auto, also on buffers the app makes. Preserving it on untouched
footage would rest on that undocumented key and needs a real HDR10+ source to prove the
whole path (decode, render of an untouched clip, encode); neither is in place.

### HAP

HAP clips import through the bundled FFmpeg's own HAP decoder (no encoder is built, and
none is needed to read): HAP, HAP Alpha and HAP Q decode to FFmpeg's own pixels, alpha
included (`pm-media` `tests/hap.rs`). HAP Q Alpha, two textures per frame, has no test
file yet. HAP export is not offered. Writing the MOV is simple
(AVAssetWriter passes `Hap1` samples through, and FFmpeg reads them back exactly), but each
flavour needs its own texture encoder at UHD speed (BC1, BC3, and scaled YCoCg in BC3 for
HAP Q), plus Snappy and chunked sections, and an export branch that bypasses VideoToolbox,
with its own file check. HAP Alpha cannot keep alpha exact: a BC3 (or BC4) alpha block
holds at most eight alpha values per 4×4 pixels, so a soft edge changes.

## Open items

- The GPU encoder-input pass (pm-render) is not part of this lane; `convert.rs` is
  the reference it must match bit for bit, including the dither tile.
- HEVC, WebM (libvpx), and PNG sequences are not implemented in `pm-export` yet.
- `.github/workflows/release.yml` does not build FFmpeg; `dist:release` now fails
  until the workflow runs `build.sh` (or restores a cached build) before it.
- `packages/cli` ships no FFmpeg. On Linux a distribution FFmpeg serves the agent
  media tools and proxies, but export needs VideoToolbox (decision 16).
- Base-machine behavior (M1 without a ProRes engine: software ProRes through
  VideoToolbox and through `-allow_sw 1`) is unverified (decision 11).
- The legacy H.264 export now uses VideoToolbox rate control instead of x264; section
  17's VMAF comparison against x264 decides whether a software encoder is offered.

## Test run (2026-10-12, after merging `feat/native-engine` at fe3a8302)

Run serially, one Electron at a time, against the shared LGPL build:

- `cargo test -p pm-export --all-features`: 32 unit and 11 VideoToolbox tests pass;
  clippy `-D warnings` and rustfmt clean.
- Vitest, every suite that spawns FFmpeg (`render-encoder`, `ffmpeg-binary`,
  `media-proxy`, `agent-tools/*` including `media-tools` and `media-ffmpeg`,
  `transcription/*`, `host-ffmpeg`, `reconnect.integration`): 225 tests pass.
- `smoke.ts`: 28 of 28 against the shared build and against the staged
  `apps/desktop/vendor/ffmpeg`.
- Playwright: `export-mp4` (2), `export-video-frames` (9), `video-scrubbing` (2),
  `native-export-performance.spec.ts:55`, `ae-audit-implementation.spec.ts:95`,
  `media-intelligence` (4, plus the real-model caption test with
  `POWERMOVE_E2E_PARAKEET_DIR`), `composited-video-sync`, `video-frame-integrity` (3)
  pass. `scripts/test-mcp.mjs` passes.
- Three failures are not this lane's: each fails the same way on `feat/native-engine`
  with `ffmpeg-static` (checked in a scratch worktree).
  `import-prores-alpha.spec.ts:10` passes the import, proxy and alpha checks, then
  reads `window.powermove`, which the preload no longer exposes (amendment 14's test
  seam). `agent-media-tools.spec.ts:42` gets every media tool result right but sees 8
  images in the last request instead of 4, and
  `media-intelligence.spec.ts:259` (real model) transcribes correctly but finds 4
  caption layers instead of 1; both point at the agent loop repeating work, not at FFmpeg.
- `electron-builder --mac --dir` (unsigned) puts `bin/`, the seven dylibs,
  `NOTICE.txt`, `share/licenses` and `manifest.json` under
  `Contents/Resources/ffmpeg`; the bundled `ffmpeg -L` reports the LGPL and lists the
  three VideoToolbox encoders; nothing from `ffmpeg-static` or `Resources/encoder`
  remains. Team ID signing was not exercised (no release identity used locally).
