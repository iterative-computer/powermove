# Video reading fixtures

Synthetic clips (FFmpeg `testsrc2` with midtone and alpha gradients; no
user media), one per behaviour, for `tests/video_reading.rs`. `cases.json`
lists each clip, its sample time, and today's viewer at that time:
`paused` is the app's Canvas source upload and `playing` its direct video
upload (both premultiplied RGBA8 rows, top first, zlib), captured with
Electron 44 / Chromium 152 on macOS ARM64. Where the two readings were
byte-identical one file serves both. Clips today's viewer cannot decode
(ProRes) are compared with the f64 reference alone.
