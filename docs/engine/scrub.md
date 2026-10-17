# Long-GOP scrub

Design notes for random access and playhead drags on long-GOP sources
(plan sections 13 "Decode service", 15 "Scrub and gesture frames", 25
"Scrub, long-GOP, no generated media"; contract 2 `SeekMode::Scrub`; RFC
02 version note `FrameQuality.stand_in`). The Phase 2 gate
(`phase2-gate.md`) measured H.264 High, GOP 250, 3 B frames at 2160p:
random seek to draw p50 125.4, p95 347.5, max 736.3 ms against p95 ≤ 300
ms, and a playhead drag that showed 242 of 360 asked frames.

Builds on: [media.md](media.md) (sessions, the decode service, the frame
cache), [tracer.md](tracer.md) (the presenter, transport, the measurement
spec), [pm-render.md](pm-render.md) (media into working textures).

## Where the time went

`engine/crates/pm-media/examples/seek_bench.rs` runs the measurement spec's
60 random seeks (the same seed and generator) through the decode service,
then drags, on the 2160p clip, and prints per seek the samples fed, the
time reading the file, submitting to the decoder, and waiting for its
output (`SessionStats::read_us`, `send_us`, `wait_us`).

| Cost | Measured (2160p, before) | |
|---|---|---|
| VideoToolbox session setup | 0 per seek: the service pools sessions per source and instance (amendment 6) and reuses them; the first open is part of the first seek only | not the cause |
| Decode from the GOP start | 1.41 ms per picture, every picture of the GOP up to the target (217 pictures for a target 237 frames into its GOP: 302 ms of submission) | **the cause** |
| B-frame reordering | none extra: VideoToolbox emits pictures in decode order as they finish; the target is ready after one wait | |
| Waiting for the target's output | 5.5 ms per seek (the hardware's latency for the last picture) | |
| Reading samples | 0.3 to 3.3 ms (page cache) | |
| Upload | zero-copy import (`CVMetalTextureCache`); the source pass converts at the viewer's size | |

The decoder's throughput is the limit: decoding the pictures before the
target without output buffers (`kVTDecodeFrame_DoNotOutputFrame`) and
`kVTDecompressionPropertyKey_RealTime` changed nothing measurable (p95 188.6
against 188.6 ms and 188.4 ms), so neither is used. The fix is to decode
fewer pictures, and to decode them before they are asked for.

## What changed

### Seeks decode only the pictures they need (`decode/skip.rs`, `decode/session.rs`)

To show frame N of a GOP the session feeds the GOP from its sync sample in
decode order. A picture that nothing references and that shows before N
need not be decoded: H.264 pictures whose every VCL NAL unit has
`nal_ref_idc` 0, and HEVC sub-layer non-reference pictures (`TRAIL_N`,
`TSA_N`, `STSA_N`, `RADL_N`, `RASL_N`, reserved `RSV_VCL_N*`) in the highest
temporal sub-layer (`hvcC` numTemporalLayers; unknown means nothing is left
out). No decoded picture changes: such pictures are never prediction
references (H.264 7.4.1, HEVC 3.160), H.264 non-reference pictures do not
advance `PrevRefFrameNum` and carry no `dec_ref_pic_marking`, so leaving
them out creates no `frame_num` gap. AVFoundation drops the same pictures
when it scrubs. The classification reads the sample's NAL headers from the
bytes already read for decoding; malformed samples are never left out.

With x264's three B frames and a B pyramid, 40 to 43% of the pictures of a
GOP are such pictures (100 to 107 of 250 on the measured clip).

Further rules in the session:

- Samples are read and submitted in batches of 8, and decoded frames are
  trimmed to the window as they arrive (before, a 4K GOP held every decoded
  picture until the request ended).
- `Session::sample_with` asks a caller's `stop` between batches; a stopped
  request leaves the run where it got to, so a later request further along
  the same run continues from there (`Session::continues`).
- `set_keep_before(n)` keeps the `n` presented frames before a target,
  unreferenced ones included (reverse scrubbing steps into them next).

Exact random access stays exact: the random-access suite (`tests/random_access.rs`,
`only` with `PM_MEDIA_ONLY` for a subset) compares every composition frame in
shuffled order, in new and reused sessions, with the sequential decode and
FFmpeg's framemd5; it ran on the H.264, HEVC (open GOP, B pyramid, Main10,
4:2:2), UHD, 1/600 MOV, and 90 kHz TS corpus files with no mismatch.

### The viewer's decode worker (`decode/scrub.rs`)

`ViewerDecode` runs one worker thread (user-interactive QoS) per source and
viewer instance; pm-render's `MediaCache` sends every display frame through
it (export frames still decode on the caller's thread, with `DecodePurpose::Export`).

- **Exact requests wait** (`Want::Exact`: playback, a paused frame after an
  exact seek, frame steps). They are never approximated.
- **Scrub requests never wait** (`Want::Scrub`: a paused frame after
  `seek {scrub}`): the exact frame when it is in the frame cache, otherwise
  the nearest decoded frame of that source (presentation distance; the
  newest decoded frame as a last resort) while the worker decodes the exact
  one. Only the very first frame of a source, with nothing to stand in,
  waits.
- **Warm sessions.** The worker takes its session from the service's pool
  for each job and returns it, so a scrub, the paused frame after it, and
  Play all continue the same run.
- **Superseded seeks.** The newest scrub target replaces an older one that
  has not started (`superseded`). A target in flight stops (`stopped`) when a
  newer request lies earlier on the same run (it is reached sooner by
  continuing), or when the newer request's own seek costs fewer samples than
  finishing the one in flight. Otherwise finishing first costs the newer
  request nothing (it is further along the same run) or less than starting
  over, and a frame is shown meanwhile, as today's viewer completes its
  in-flight seek before starting the newest one (`legacy/core/video-seek.ts`).
- **Look-ahead.** A move of at most 16 frames is a drag; its step is the
  stride. After a target lands, the worker decodes ahead by the stride
  into the frame cache: 8 steps forward (within half the cache); in
  reverse, one neighbourhood, because every reverse neighbourhood costs a
  seek from the GOP start: that seek keeps every reference picture up to
  32 frames before its target and the unreferenced pictures on the
  stride (`set_keep_before(frames, stride)`, within three quarters of the
  cache). A forward drag through look-ahead frames is served from the
  cache, and each cache hit moves the look-ahead along. A larger move is a
  jump (a ruler click, a random seek): nothing is decoded ahead of it, so
  speculation never delays the next seek. A request elsewhere stops
  look-ahead.
- **Memory.** The frame cache (the lease's frame-cache share) is 512 MiB in
  the viewer's renderer (about 40 UHD 4:2:0 frames); the decoder share
  applies to the worker's session window as to any session. Idle workers
  exit after 30 s.

### Stand-ins on screen (`pm-render`, `pm-node`, contract 2)

- `Renderer::set_scrub(true)` makes video layers use `Want::Scrub`;
  `Rendered::stand_ins` counts layers drawn with a stand-in, and
  `Renderer::media_landed` changes whenever a worker finishes a target.
  Export requests never draw stand-ins, whatever the setting.
- The presenter sets it for paused frames after `seek {scrub}` only
  (`View::scrub`); exact seeks, steps, Pause, and Play clear it, and timed
  (playback) frames never use it. A frame drawn with stand-ins carries
  `FrameQuality.stand_in`, and the presenter draws the frame again when the
  workers land a frame (polled every 1 ms while such a frame is on screen),
  until a frame without stand-ins is on screen.
- `FrameQuality.stand_in` (RFC 02 version note) reaches the viewer as
  `PresentedFrame.standIn`, so measurements and, later, the viewer label can
  tell a stand-in from the exact frame at the same `pts`.

### Upload (`pm-gpu::decode_in`, `pm-render::media`)

With frames decoded ahead, a drag through cached UHD frames still drew at
most about 120 frames a second: drawing one cached UHD frame into a 1080p
viewer frame cost p50 7.69 ms, p95 8.40 ms (`pm-render` test
`drawing_cached_uhd_frames`, `PM_SCRUB_CLIP`). The CPU part was 0.15 ms;
the rest was fresh GPU memory: every draw normalized into a new
`rgba32float` luma and `rg32float` chroma texture (133 and 17 MB at UHD) and
converted into a new `rgba16float` target (66 MB). The importer now hands
its output textures back once the conversion is recorded
(`DecodeImporter::recycle`; later commands in the queue see them
overwritten only after the reads), and the media cache overwrites a
converted frame that an earlier render retired (every read of it is
submitted). The same draw: p50 1.79 ms, p95 2.49 ms. Pixels are unchanged
(two layers of one clip at different times over several renders equal
fresh renders, `reused_frame_textures_never_change_what_a_render_reads`).

## Exactness

- Exact requests (playback, a paused frame after an exact seek, steps,
  Pause) wait for their exact frame. Exports decode on the export renderer's
  thread through the unchanged synchronous path, never draw stand-ins, and
  never use the viewer workers.
- A frame with a stand-in says so (`FrameQuality.stand_in`) and is followed
  by the exact frame when it decodes.
- Leaving out unreferenced pictures changes no decoded picture
  (random-access suite and `tests/scrub.rs`, below).

## Measurements

Jude's MacBook Pro (Apple M4 Max, 128 GB, macOS 27, built-in 120 Hz
display), release builds, shared with other engine lanes (the 1-minute load
average is given per run). Not a reference machine (section 25), so these
are measurements, not gate verdicts. Clips: `docs/engine/tracer.md`
("Tests"); 2160p is H.264 High, GOP 250, 3 B frames, 50 Mb/s, 29.97 fps.

### Decode service alone (`seek_bench`, the spec's 60 seeks)

| 2160p | Before | After |
|---|---|---|
| Random seek, one pooled session (load 5.0 before, 9.9 after) | p50 103.2, p95 320.9, max 337.6 ms | p50 66.5, p95 188.9, max 204.0 ms |
| Samples fed for the deepest seek (frame 734) | 235 | 138 (97 left out) |
| Forward steps, one frame at a time | p50 3.6 ms | p50 3.6 ms |
| Simulated drag, 120 Hz, 3 ms drawing (exact frames of 360) | | forward 1: 359; reverse 1: 305 to 308 |

### In the app (`e2e/native-engine-tracer-measure.spec.ts`, page timing)

Seek to draw: from `PM.setTime` to the animation frame that drew the exact
frame (60 random seeks, the spec's seed). First frame: the first frame drawn
at that time (a stand-in when the exact one was not decoded). Drags: a seek at
every animation frame for 3 s, 1 or 7 frames apart, forward and reverse;
"shown" is the distinct video frames drawn (exact frames plus the frames
stand-ins showed, from the presenter's stand-in log).

Today's viewer, the same spec with Use New Engine off: seek to draw ends at
the first animation frame after the preview `<video>` element completed a
seek to the target; a drag's frames are the distinct targets the element
completed seeks to (an upper bound on what today drew). Today decodes the
all-intra 1280-pixel editing copy it builds after import once it is ready
(`legacy/core/video-preview.ts`; the spec records it was in use by the end
of each run), so its drags are not decoding the long-GOP original.

Before (Phase 2 gate, `phase2-gate.md`): 2160p seek to draw p50 125.4, p95
347.5, max 736.3 ms; the forward drag showed 242 of 360 asked frames.

After (2026-10-17, final code; the load average was 7.7 to 8.6 during the
engine runs and 8.0 to 15.8 during today's):

| Clip | Engine seek to draw | Engine first frame | Today seek to draw |
|---|---|---|---|
| ProRes 422 HQ 1080p | p50 7.5, p95 8.1, max 9.4 ms | p95 7.9 ms | p50 18.3, p95 43.2 ms |
| H.264 GOP 250 1080p | p50 20.0, p95 58.5, max 64.2 ms | p95 7.8 ms | p50 26.5, p95 116.7 ms |
| **H.264 GOP 250 2160p** | **p50 71.9, p95 205.3, p99 231.5, max 674.1 ms** | p50 8.0, p95 10.9 ms | p50 54.6, p95 326.1, max 351.3 ms |

The 2160p maximum is one seek whose first frame took as long (674.1 ms),
so the whole frame path stalled for it, not the decode; the next largest is
231.5 ms. It did not occur in the previous run of the same code with the
no-output decoding (p95 194.9, max 210.8 ms, load 9.8).

Drags, frames shown of about 360 asked (exact frames in parentheses):

| Clip | Forward 1 | Reverse 1 | Forward 7 | Reverse 7 |
|---|---|---|---|---|
| ProRes 1080p, engine | 362 (361) | 362 (361) | 362 (361) | 361 (361) |
| ProRes 1080p, today | 155 | 163 | 156 | 157 |
| H.264 1080p, engine | 360 (360) | 359 (359) | 362 (361) | 319 (303) |
| H.264 1080p, today | 123 | 164 | 157 | 130 |
| **H.264 2160p, engine** | **362 (360)** | **315 (314)** | **174 (1)** | **188 (134)** |
| H.264 2160p, today | 128 | 137 | 129 | 127 |

Exact frames of a 2160p forward drag reach the screen p50 4.7, p95 5.4 ms
after their request. A 7-frame forward drag at 2160p asks for 840 frames a
second; the worker lands about 130 targets a second (each one decode of
about seven pictures plus the decoder's 5.5 ms output latency), shown as
stand-ins for the newer requests, so few frames are exact while 174 distinct
frames are shown. A 7-frame reverse drag needs a seek from the GOP start for
every 32 frames.

Earlier runs of the same spec while the work landed, for the record: after
the decode changes only (load 12.9), 2160p seek p95 297.0 ms and the
forward drag 256 exact frames (drawing a cached UHD frame cost 7.7 ms);
after the upload fix (load 8.6), seek p95 194.8 ms, forward drag 361,
reverse 7 shown 64; with the stride look-ahead and reverse neighbourhoods
and the no-output decoding (load 9.8), seek p95 194.9 ms, drags 363, 318,
167, 122.

## Tests

- `pm-media`: `decode::skip` unit tests (H.264 `nal_ref_idc`, HEVC
  sub-layer non-reference types and temporal layers, Annex B, malformed
  samples); `tests/scrub.rs` (exact requests equal the sequential decode,
  scrub stand-ins then the exact frame, superseded targets, look-ahead,
  released workers; `--ignored`: long-GOP corpus drags in both directions
  stay equal to the sequential decode); the random-access suite with the new
  `only` group (`PM_MEDIA_ONLY=h264,hevc,uhd-,mov-600,ts-90k`).
- `pm-render`: `tests/scrub.rs` (a scrub render stands in until the exact
  frame lands and then equals an exact render; exports never stand in;
  reused textures change no pixel; `--ignored`: the UHD drawing cost).
- `pm-node`: `tests/scrub.rs` (the composer reports stand-ins after a scrub
  and none once the video landed; exact seeks never stand in).
- vitest: the viewer's presented frames carry `standIn`.
- e2e: `native-engine-tracer.spec.ts` waits for an exact frame before
  reading colors after a scrub; `native-engine-tracer-measure.spec.ts` as
  above (`--grep "scrub latency"`, opt-in `PM_TRACER_MEASURE=1`, under the
  Electron lock).

## Changes outside this lane

Kept small; listed for the owning lanes.

- `engine/crates/pm-protocol` (contract 2): `FrameQuality.stand_in`, the RFC
  02 version note, regenerated TypeScript and wire fixtures; the build id
  moved to `protocol-2e87c1a0e6d5735f`.
- `engine/crates/pm-render`: `media.rs` (display frames through the
  workers, the 512 MiB frame cache, texture reuse), `renderer.rs`
  (`set_scrub`, `media_landed`, `scrub_stats`, `Rendered::stand_ins`).
- `engine/crates/pm-gpu/src/decode_in/mod.rs`: `DecodeImporter::recycle`.
- `engine/crates/pm-node/src/present/`: `compose.rs` (`Composed::stand_ins`),
  `render.rs` (scrub mode per view, stand-in redraws, report counters and
  the stand-in log).
- `apps/desktop/src/extensions/viewer/`: `engine-present/protocol.ts`,
  `presenter.ts` (`PresentedFrame.standIn`), `presenter.test.ts`, `viewer.ts`
  (`V.engineFrame().standIn`).
- `apps/desktop/e2e/native-engine-tracer.spec.ts` (exact-frame wait),
  `native-engine-tracer-measure.spec.ts` (the scrub rows).

## Gaps

- **Fast reverse drags on UHD long GOP** show more frames than today's
  viewer (188 against 127 in the final run) but only 134 exact of 361
  asked, and the margin varied between runs (122 against 124 in one run):
  each 32-frame reverse neighbourhood costs a seek from the GOP start at
  the decoder's 1.41 ms per UHD picture, and the frame cache (512 MiB)
  holds about 40 UHD frames. Keeping reduced-resolution stand-ins (a GOP at
  the viewer's size fits where 40 UHD frames do) or the Phase 4a proxies
  would close it.
- **Fast forward drags on UHD long GOP** draw distinct frames as stand-ins
  rather than exact frames (1 exact of 174 shown): each target waits for the
  decoder's 5.5 ms output latency before the next starts. Submitting the
  next targets while waiting (a pipelined session) is the next step.
- **Leases.** The viewer's frame cache is a fixed 512 MiB (it was 64 MiB);
  the engine does not yet apply its lease's frame-cache and decoder shares to
  pm-render's decode service (`MediaCache` has no lease entry point; the
  service's `apply_lease` exists).
- **Reference machines.** Only this Mac was measured, under other lanes'
  load; no base M1 or M-series Air (no ProRes engine, fewer media engines).
- **The viewer label.** `FrameQuality.stand_in` reaches the viewer, which
  does not show it yet.
- **Corpus timing.** One run of the S6 random-access group (`s6_corpus`),
  while the machine was loaded and with an earlier version that also decoded
  pictures without output buffers (`kVTDecodeFrame_DoNotOutputFrame`),
  failed its 2 s probe limit with a single 60 s VideoToolbox stall on
  `mov-600-23976.mov` (the frame matched). It did not recur in six reruns of
  that file, the B-pyramid file, and the UHD files; the no-output decoding,
  which gained nothing measurable, was removed anyway. Recorded in case it
  recurs.
