# Synthetic transformed-video composition reference

The same public synthetic H.264 clip as `../video-parity/handoff.mp4` is drawn
inside the app at 320 × 180 over `#283346`. Its 240 × 135 contain layer is
scaled 92% × 83%, rotated 14°, and composited at 67% opacity.

`document.json` preserves the rendering fields of the captured app document.
Local source paths, storage identifiers, fingerprints, timestamps, and random
project/layer IDs were removed or replaced. The six compressed Float32 rasters are
the **unchanged** readback bytes from the actual app at 0.5s and 1.5s
for project color depths 8, 16, and 32. They are render outputs of synthetic
media, not desktop screenshots. `references.json` records each raw raster's
SHA-256 so a test cannot silently refresh a reference.

The app captures encode RGBA8 by converting each Float32Array element to
JavaScript Number and applying `Math.round(value * 255)`. This multiplication
therefore occurs in binary64. The old native parity example multiplied in
binary32 first, making 1 and 15 false half-code differences at depths 32.
The RGBA8 rasters are independently decoded from the app render-output PNGs.
They agree with JavaScript binary64 quantization of the Float32 captures at
all six depth/time pairs.
