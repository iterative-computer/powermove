# Synthetic HDR and P3 video references, round 4

Seven short synthetic clips cover AV1 PQ and HLG, HEVC PQ with BT.2020 and
Display P3 primaries, deep VP9 BT.2020 SDR, and two HEVC SDR gamut controls.
Five clips are byte-for-byte copies of the round 3 public sources. The two
controls use the same detailed midtone gradient pattern. Each clip is sampled
at 0.125, 0.625, and 1.125 seconds in the current legacy desktop app.

`references.json` binds each clip and uncompressed raster to SHA-256 hashes,
dimensions, presentation timestamp, and the captured Chromium color-space
metadata. For each frame, `canvas` and `rgba` preserve the straight Canvas2D
and direct-video texture readings. `premult` and `live` are the actual WebGL
uploads with `UNPACK_PREMULTIPLY_ALPHA_WEBGL=true`. The strict native gate
compares `premult` and `live` for every frame and working precision. The other
two readings remain independently checked evidence.

Each frame also carries sRGB GPU Canvas and Display P3 CPU/GPU Canvas probes.
Their straight and premultiplied rasters are checked for byte size and SHA-256
and their reported color spaces are checked. These probes document distinct
browser paths; the strict parity output is the legacy app's sRGB Canvas and
direct-video live texture. App capture errors, probe errors, and native decode
or rendering failures fail the gate rather than being skipped.

To reproduce from the repository root after building the desktop app:

```sh
node apps/desktop/scripts/video-hdr-parity.mjs generate /tmp/powermove-video-parity-round4
~/Library/Caches/powermove-engine/bin/pm-electron-lock nice -n 15 node apps/desktop/scripts/video-parity.mjs capture /tmp/powermove-video-parity-round4
node apps/desktop/scripts/video-hdr-parity.mjs pack /tmp/powermove-video-parity-round4
```

Raw captures, user profiles, and operational reports stay outside Git. Only
synthetic clips, compressed reference rasters, and this metadata are packaged.

The compatibility decision for captured PQ/HLG is **MATCH**. Direct WebGL
uploads retain encoded-looking RGB even though sRGB Canvas visibly tone-maps
it. A future correction to that live appearance needs an explicit behavior
change; this fixture does not silently introduce one. GPU-backed sRGB Canvas
also differs from the app's CPU Canvas by up to three channel codes in these
HDR controls. Both are retained so a backend substitution cannot silently
replace the measured app path. Display P3 probes are read in their own color
space and then independently uploaded to the app's sRGB WebGL context.
