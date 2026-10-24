# Video reading fixtures

Synthetic clips (FFmpeg `testsrc2` with midtone and alpha gradients; no
user media), one per behaviour, for `tests/video_reading.rs`. `cases.json`
lists each clip, its sample time, and today's viewer at that time:
`paused` is the app's Canvas source upload and `playing` its direct video
upload (both premultiplied RGBA8 rows, top first, zlib), captured with
Electron 44 / Chromium 152 on macOS ARM64. Where the two readings were
byte-identical one file serves both. Clips today's viewer cannot decode
(ProRes) are compared with the f64 reference alone.

`superwhite-sd.mp4` (256×144, untagged) and `superwhite-hd.mp4` (1280×720,
Rec.709 tagged through `h264_metadata`, so no scaler touches the planes)
are lossless H.264 (`-qp 0`) of five bands of Y′ 8, 16, 128, 235 and 250
with neutral chroma, for the superwhite test.

`chroma-truth444.mp4` and its subsampled versions are the ground truth for
`tests/chroma_truth.rs`: a 256×128 frame, left half a band-limited colour
texture (fractal noise in hue, lightness and saturation), right half hard
edged graphics (shapes, one- and two-pixel lines, glyph strokes), converted
in f64 to 8-bit limited BT.709 Y′CbCr 4:4:4. `chroma-dec420` and
`chroma-dec422` decimate its f64 chroma with the co-sited (left) [1 2 1] / 4
filter across and the two-row average down (4:2:0); `chroma-sws420` and
`chroma-sws422` are FFmpeg's swscale (`scale=out_chroma_loc=left`, bicubic)
from the 4:4:4 codes. All are two-frame lossless H.264 (`-qp 0`) tagged BT.709 with
chroma location 0 through `h264_metadata`.
