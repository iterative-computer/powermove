# Video reading fixtures

Synthetic clips (FFmpeg `testsrc2` with midtone and alpha gradients; no
user media), one per behaviour, for `tests/video_reading.rs`. `cases.json`
lists each clip, its sample time, and today's viewer at that time:
`paused` is the app's Canvas source upload and `playing` its direct video
upload (both premultiplied RGBA8 rows, top first, stored losslessly as RGBA PNG pixels), captured with
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
`chroma-sws422` are FFmpeg's swscale from the 4:4:4 codes. Regenerate, in an
empty scratch directory:

```sh
python3 chroma-gen.py .   # truth444.yuv, dec420.yuv, dec422.yuv
for f in 420 422; do
  ffmpeg -f rawvideo -pix_fmt yuv444p -s 256x128 -i truth444.yuv \
    -vf "scale=out_chroma_loc=left:flags=bicubic,format=yuv${f}p" \
    -f rawvideo -pix_fmt yuv${f}p sws$f.yuv
done
TAG=h264_metadata=colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1:video_full_range_flag=0:chroma_sample_loc_type=0
for spec in "truth444 yuv444p" "dec420 yuv420p" "dec422 yuv422p" "sws420 yuv420p" "sws422 yuv422p"; do
  set -- $spec; cat $1.yuv $1.yuv > two.yuv
  ffmpeg -f rawvideo -pix_fmt $2 -s 256x128 -r 30 -i two.yuv -c:v libx264 -qp 0 \
    -pix_fmt $2 -bsf:v $TAG chroma-$1.mp4
done
```

Each is two identical lossless frames (a video needs two); decoding frame 0
returns the raw planes exactly.

`stripes-709-limited.mp4` (480×480, two identical lossless frames, Rec.709
limited range) is vertical 3-px stripes, two columns of Y′ 235 and one of
Y′ 16, neutral chroma: detail finer than a reduced preview's pixels, for the
preview downscale test in `src/media/source_storage_tests.rs`. Regenerate:

```sh
python3 -c "y=bytes((16 if x%3==2 else 235) for x in range(480))*480; c=bytes([128])*(240*240); open('s.yuv','wb').write((y+c+c)*2)"
ffmpeg -f rawvideo -pix_fmt yuv420p -s 480x480 -r 30 -i s.yuv -c:v libx264 -qp 0 \
  -pix_fmt yuv420p -bsf:v $TAG stripes-709-limited.mp4
```
