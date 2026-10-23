# Chromium video upload compatibility

Compatibility level 1 preserves the decoder-to-texture boundary as well as
the encoded source's metadata. Interpret Footage color overrides and level 2
continue to use the native color contract. No frame routes to the old renderer.

The synthetic capture harness is `apps/desktop/scripts/video-parity.mjs`.
Build the app first, generate clips outside the repository and the Electron
lease, then run its `capture` command under `pm-electron-lock`. It records
actual stream tags with ffprobe, Chromium versions, paused Canvas pixels,
direct WebGL texture pixels, VideoFrame timestamps and exposed decoder planes.
Pixels cross the driver as base64 to bound serialization memory. The isolated
app profile is removed in `finally`. Source recipes include detail, gradients,
midtones and motion; no private corpus or desktop screenshots are fixtures.

`pm-media`'s `video_parity_dump` example samples those exact timestamps through
the native service. `pm-render`'s `video_parity` example measures full-frame
Canvas parity and pm-perf work counters, records unsupported formats as failures,
and fails if a format remains unresolved or exceeds the existing 2 CV video
bound. These examples do not launch Electron or create temporary directories.

The measured Chromium build is Electron 44.0.0 / Chromium 152.0.7977.54 on
macOS ARM64. Twenty-nine exposed 8-bit decoder-plane comparisons (H.264,
VP9 and AV1; three timestamps per recipe plus the two-frame midtone witness)
are code-identical to native decoding. P010 planes are not exposed by this
build's VideoFrame.copyTo, so the HEVC conclusion comes from GPU/Canvas pixels.

## Implemented readings

- Incomplete SD H.264 color descriptions with neither primaries nor transfer
  use Chromium's limited BT.601 fallback even with a lone BT.709 matrix tag.
  The HD witness instead uses BT.709. Fully specified tags remain authoritative.
- Untagged opaque VP9 uses Chromium's BT.709 reading at SD. Persisted animation
  renditions retain their separately measured BT.601/libyuv reading.
- Ordinary hardware uploads already use centered bilinear chroma and an RGBA8
  source store. The P010 HEVC compatibility path additionally reproduces Skia's
  8-bit matrix on the MSB-aligned R16Unorm planes. In normalized native YUV this
  is a scale of 65280/65535, with Y bias `(16/219)*(scale-1)` and C bias
  `(128/224)*(scale-1)`. Native/overridden/HDR readings retain native normalization.
- Full-range, fully specified BT.709 H.264 Canvas snapshots use pinned libyuv's
  full-range BT.709 integer conversion with nearest chroma. Native planar and
  biplanar inputs are accepted. All three synthetic Canvas references are exact.
- Limited 8-bit H.264 tagged BT.2020/BT.709 transfer/BT.2020 NCL is drawn by the
  real native source pass. Canvas first clips and quantizes decoded RGB to RGBA8,
  then converts its primaries into sRGB and stores RGBA8 again. Direct playback
  keeps encoded RGB. No other wide-gamut capability reason was removed.

Paused/final sampling and scrubs choose the Canvas reading; playback sampling
chooses direct upload. The reading participates in converted-frame cache keys.
The tests flip Canvas -> live -> Canvas at one timestamp and compare both
independent captures, so a cached texture cannot hide the difference.

Committed fixtures cover six recipes, three timestamps each, both working
precisions on this integration base, plus the BT.2020 live/Canvas cache switch.
Every source pixel is required within 1 CV (stricter than the corpus video
bound); full-range H.264 Canvas is required exact. Small GPU half-code rounding
residuals remain: this is not a claim of universal bit equality.

## Remaining closure work

The direct-upload full-range H.264 reference has a small edge-region difference
from ideal centered bilinear conversion. Canvas/export is exact, but direct
playback still needs its own Chromium boundary reconstruction. Other full-range
matrices, deep VP9/AV1, P3 and HDR need their own captured readings.

Chromium rejects direct ProRes 422/4444 sources. Today's app transcodes them to
VP9 playback renditions (`playbackConverter`); comparing native originals with
those decoded lossy renditions cannot establish original-decoder parity. The
existing-project binding must preserve each actual persisted rendition, with
alpha, and gate its import, paused, playback and export paths. Direct rejection
is recorded as an unresolved measurement, never a passed or omitted codec.

A paired end-to-end decode-cost comparison against today's app remains required.
The source measurements record pm-perf counters but are not a wall-time decode
performance pass. Eight-bit project-depth integration also needs rerunning after
the separate color-depth lane merges; this base exposes Float16 and Float32.

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
[video renderer](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/media/renderers/paint_canvas_video_renderer.cc),
[color space handling](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/ui/gfx/color_space.cc),
and pinned Skia's
[pixel conversion](https://skia.googlesource.com/skia/+/chrome/m152/src/opts/SkSwizzler_opts.inc).
