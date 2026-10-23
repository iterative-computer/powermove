# Canvas PNG alpha rounding reference

`canvas-alpha-arm64.bin` is a table of synthetic channel codes, not image or
user data. It contains 65,536 bytes, indexed by `alpha * 256 + channel`.
The output byte is Chromium's decoded PNG red channel after writing
`min(255, floor(channel * 255 / alpha))` into all three ImageData channels,
with the given alpha. At alpha zero the input channels are zero.

Captured in the hidden desktop app with Electron 44.0.0 / Chromium
152.0.7977.54 on macOS ARM64. `scripts/video-parity.mjs png OUTSIDE_REPO`
regenerates the exhaustive Canvas2D `putImageData` -> `toBlob('image/png')`
reference; decode its PNG and retain every fourth byte. The SHA-256 is
`e0eb855fa1548fc7bc31b46b8c824d0a6d1b37aa591c896fc694c5d712ceb2e9`.

The implementation follows Chromium's pinned Skia
[`SkSwizzler_opts.inc`](https://skia.googlesource.com/skia/+/chrome/m152/src/opts/SkSwizzler_opts.inc),
`unpremul_simulating_RP` and ARM64 `common_rgbA_to_RGBA`, including every
binary32 rounding boundary and the final round to even. Replacing this
sequence with an integer reciprocal or exact rational division changes the
reference. This table tests all alpha values, including opaque, zero and
all half-code witnesses, independently of that implementation.
