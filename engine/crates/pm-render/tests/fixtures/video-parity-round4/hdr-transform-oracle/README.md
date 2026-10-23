# Synthetic HDR RGB transform oracle

This fixture isolates Chromium 152's default HDR-to-sRGB Canvas transform from
video decoding and YUV sampling. Its opaque RGBA8 input contains 36,374 distinct
colors from the public synthetic HLG source readings, plus a small grayscale
ramp. The 256 × 143 texture has 36,608 texels; unused texels are opaque white.

An Electron 44/Chromium 152 `VideoFrame` was constructed from these RGBA bytes
with full-range RGB HLG/BT.2020, PQ/BT.2020, and PQ/Display P3 tags. For each
tag pair, `drawImage` into an sRGB, unorm8 Canvas produced a CPU reading with
`willReadFrequently: true` and a separate GPU-backed reading with
`willReadFrequently: false`. The latter is retained as evidence; the strict
test compares the native Canvas tone map against each CPU reading, byte for
byte. These paths are intentionally distinct.

`manifest.json` records the requested and actual `VideoFrame` tags, returned
Canvas context settings, browser version, dimensions, and SHA-256 of every
uncompressed raster. The seven `.zlib` files contain only synthetic RGBA data.
The strict test verifies every compressed raster, all metadata, the distinct
input color count, and all three actual CPU transform outputs. No app profile,
screenshots, or operational report is included.

To regenerate evidence, first capture the synthetic HLG N32 source readings
outside the repository, then run `apps/desktop/scripts/video-hdr-transform-probe.mjs`
under `pm-electron-lock`. Its raw output should be reviewed and packed with
zlib while preserving the source and probe SHA-256 values. Changing the
Chromium or Skia version requires a new fixture and explicit parity review.
