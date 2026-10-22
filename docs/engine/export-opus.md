# WebM export parity: VP9/VP8 and Opus

The current exporter submits canvas `VideoFrame`s to Chromium 152 WebCodecs
with `latencyMode: "quality"`, a target bitrate from the export quality tier,
the selected frame rate, and a forced keyframe every rounded two seconds. Its
audio path renders a stereo 48 kHz `OfflineAudioContext` buffer and submits
planar float `AudioData` in 20 ms chunks to `AudioEncoder` with `codec: "opus"`
and `bitrate: 160000`. It muxes the returned chunks with a small EBML writer
in `legacy/core/exporter.ts`; the audio track has `OpusHead` with 312 pre-skip
samples but no `CodecDelay`, `SeekPreRoll`, or final `DiscardPadding` element.

Electron 44.0.0 ships [Chromium 152.0.7977.54](https://releases.electronjs.org/release/v44.0.0).
At that Chromium tag, [Blink's Opus config](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/blink/renderer/modules/webcodecs/audio_encoder.cc)
sets complexity 9 on desktop and leaves bitrate mode unset when the JavaScript
config omits it. The [Opus IDL](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/blink/renderer/modules/webcodecs/opus_encoder_config.idl)
defaults to 20 ms frames, application `audio`, signal `auto`, zero expected
packet loss, FEC off, and DTX off. [Chromium's Opus encoder](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/audio/audio_opus_encoder.cc)
feeds interleaved float samples to `opus_encode_float` at the total bitrate and
adds silence on flush to cover encoder lookahead. An omitted bitrate mode
keeps libopus's default VBR. The native FFmpeg libopus wrapper now asks for
these settings explicitly. The encoder path shifts FFmpeg's negative Opus
packet timestamps by its lookahead and removes packet side data that would
write `DiscardPadding`. FFmpeg 9.0.2 always writes an Opus `SeekPreRoll`
element, so opaque WebM exports now remux the encoded packets with the current
exporter's EBML track and cluster structure. This removes `CodecDelay` and
`SeekPreRoll` and retains the untrimmed encoded tail. In paired one-second
48 kHz fixtures, both paths produce 51 packets at 0 through 1000 ms and
decode to 48,648 samples per channel.

The [Chromium VPX encoder](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/video/vpx_video_encoder.cc)
uses libvpx's realtime deadline even for WebCodecs `latencyMode: "quality"`;
the latter disables frame dropping. It uses CPU-used 7 for VP9 and -6 for
VP8, one-pass VBR, zero lookahead, 2..58 quantizers, 50% under/overshoot,
a 1000 ms rate-control buffer initially filled to 600 ms, static threshold 1,
and maximum intra rate 600%. VP9 uses row threading and floor(log2(ceil(width
/ 256))) tile columns. [Software thread tiers](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/base/video_encoder.cc)
are 1 below VGA, then 2, 4, 8, and 16 at VGA, 720p, 1080p, and UHD. The
native FFmpeg libvpx wrapper now uses the exposed matching controls and a
microsecond timebase. Chromium passes timestamp zero to libvpx on every frame
and tracks output timestamps separately; the native mux path does the same.
It writes unspecified primaries and transfer fields,
matching the current WebM stream's description, while retaining BT.709
matrix and limited range for VP9. VP8's untagged bitstream and the legacy EBML
track leave matrix and range unspecified.

Chromium's [VideoEncoder accelerated canvas readback](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/blink/renderer/modules/webcodecs/video_encoder.cc)
requests a REC709 NV12 frame from the GPU before libvpx. A paired 320×180
capture proved this branch matters: the canvas RGBA bytes were identical, but
the legacy encoded solid `[232, 58, 94]` near YUV `[100, 126, 203]`, whereas
libyuv's default BT.601 path produced `[114, 117, 201]`. The fallback
[CPU VideoFrame converter](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/media/base/video_frame_converter_internals.cc)
uses that BT.601 conversion, but it is not the observed path for this canvas.
The native WebM sink now starts from the same RGBA8 render-target boundary
and applies the limited-range REC709 matrix with floating-point 8-bit writes.
Pinned libyuv's integer BT.709 conversion got Y and U right for the paired
solid but gave V=202 where the legacy decoded frame has V=203; the floating
matrix produces 203.

The paired 320×180, 30 fps opaque fixtures establish bounded **video** parity.
Native and WebCodecs input RGBA8 buffers are byte-identical in the plain and
varied-color projects. The latter contains three solid colors and layer edges.
All 30 VP9 packets have identical encoded payloads and presentation timestamps
in each project, and all 6,912,000 decoded RGBA bytes match. The legacy repeat
export is also byte-identical. Gradients, transparency, other sizes and
bitrate tiers, and VP8 remain unmeasured; these fixture results do not prove
general video parity.

**Audio parity still fails.** In the plain project, pre-encoder float PCM
differs in 47,920 of 96,000 samples (maximum absolute difference
0.00000611, RMS 0.00000305). All 51 Opus packet payloads differ:
legacy totals 20,851 encoded bytes, native 23,446. Of 97,296 interleaved
decoded float samples, 97,256 differ (maximum 0.004879, RMS 0.000361).
The advanced project, with gain automation, fade, retime, and nested audio,
has larger PCM differences: 95,840 of 96,000 float samples differ (maximum
0.066420, RMS 0.043610). Its Opus packet payloads all differ, with 20,851
legacy encoded bytes versus 19,716 native, and all 97,296 decoded float
samples differ (maximum 0.068056, RMS 0.043374). Counts, timestamps, and
track metadata match; sample and encoded-byte identity does not.

To isolate the codec from the PCM mismatch, captured legacy PCM was encoded
directly with the bundled FFmpeg libopus using the exposed Chromium settings.
It still produced 51 different packets, 23,454 bytes in total versus
Chromium's 20,851. Thus audio rendering and encoder implementation are
independent blockers. The bundled
[Opus source is 1.6.1](../../engine/third_party/ffmpeg/sources.lock), whereas
[Chromium pins commit 55513e81d8f606bd75d0ff773d2144e5f2a732f5](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/opus/README.chromium),
which differs from the 1.6.1 release tag. The bundled libvpx is 1.17.0;
[Chromium pins ade52487a37ef76a0f209bd39bea9fe67d6db4c4](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/libvpx/README.chromium)
(listed as 1.16.0). FFmpeg's libvpx wrapper also derives the optimal buffer
level as five sixths of its size, whereas Chromium sets 600 of 1000 ms. These
version and configuration differences remain risks outside the measured video
fixtures; the observed Opus mismatch is not accepted parity. The
opt-in `POWERMOVE_BACKGROUND_TEST=1` and `PM_EXPORT_INPUT_CAPTURE_DIR`
capture writes `native.rgba`, `native.i420`, and `native.f32` for paired input
checks. The harness that sets the capture directory owns its cleanup.

The varied-color strict Playwright run found three failing WebM media checks,
all audio: pre-encoder PCM identity, encoded Opus payload identity, and decoded
sample identity. Under concurrent machine load, this native export took
2,153 ms, versus 1,744 ms and 623 ms for the two legacy runs. Treat this
as an observed speed regression for that run, not a stable performance ratio.

Alpha needs a separate parity decision. The current WebCodecs call omits the
`alpha` encoder option, and [Chromium 152 explicitly rejects alpha encoding
and strips alpha-bearing frames before VPX](https://chromium.googlesource.com/chromium/src/+/refs/tags/152.0.7977.54/third_party/blink/renderer/modules/webcodecs/video_encoder.cc).
The native WebM muxer retains its existing libvpx alpha stream for an explicit
alpha export. That capability does not prove parity with the legacy WebM path;
a paired alpha export is required before treating it as matched.
