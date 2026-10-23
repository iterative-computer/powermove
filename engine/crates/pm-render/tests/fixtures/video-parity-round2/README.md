# Synthetic video conversion references, round 2

These clips come from FFmpeg's `testsrc2` plus deterministic midtone and
horizontal gradient expressions. They contain no user media. Each clip has
three paused samples at 0.125, 0.625, and 1.125 seconds. HEVC is 10-bit 4:2:0
with fully specified BT.601, BT.709, or BT.2020 non-constant luminance tags,
each in limited and full range. The ProRes sources are 10-bit 4:2:2 and
ProRes 4444 with a nonuniform alpha plane. FFmpeg decodes the latter as
`yuva444p12le`; its alpha spans 64–255 after conversion to RGBA8.

Chromium cannot decode ProRes directly in this Electron build. The harness
imports each MOV through `PM.pickFiles()` in a real app project, waits for the
app's ProRes-to-VP9 conversion, saves the durable `.playback.webm` rendition,
verifies it matches the imported asset's live video blob byte for byte, and
samples a video element backed by that rendition. Native comparisons of
the existing app's playback path must decode that rendition. The MOV is kept
to reproduce the import and distinguish original content from playback media.
`references.json` records both names, their hashes, stream tags, and the
observed browser version.

Each `.rgba.rgba.zlib` file is a zlib-compressed, raw RGBA8 readback of the
video uploaded to an unfiltered WebGL2 texture. Each `.canvas.rgba.zlib` file
is the same paused frame drawn through Canvas2D (`getImageData`, top row
first). The Canvas reference includes the
ProRes 4444 straight-alpha values. Each `.premult.rgba.zlib` file reads back
the *same drawn Canvas2D* after a WebGL2 upload with
`UNPACK_PREMULTIPLY_ALPHA_WEBGL=true`, matching the compositor's `texFor`
boundary. Although WebGL `readPixels` reads upward in framebuffer coordinates,
these raw arrays align with source top-down rows after this texture upload.
The opaque premultiplied uploads equal their Canvas bytes exactly; the 4444
RGB values change while alpha stays unchanged. `nativeReference` names this
upload for native parity; `premultipliedCanvas` gives its file and hash. These are
synthetic media rasters, not screenshots. `references.json` contains SHA-256
values for the uncompressed pixel bytes.

To regenerate with the current app build, from the repository root:

```sh
bun install --frozen-lockfile
bun run --cwd apps/desktop build
node apps/desktop/scripts/video-parity.mjs generate /tmp/powermove-video-parity-round2 --round2
~/Library/Caches/powermove-engine/bin/pm-electron-lock nice -n 15 node apps/desktop/scripts/video-parity.mjs capture /tmp/powermove-video-parity-round2
node apps/desktop/scripts/video-parity.mjs pack /tmp/powermove-video-parity-round2
```

The harness keeps its app user-data directory temporary and removes it after
capture. Keep the external evidence directory for comparisons; only the
synthetic sources, actual playback renditions, compressed references, and
metadata are packaged here.
