# Synthetic Chromium video references

These public synthetic clips come from FFmpeg `testsrc2` with deterministic
midtone/gradient expressions in `apps/desktop/scripts/video-parity.mjs`.
They contain no user media. The separate `handoff.mp4` witness is 160x90,
30 fps, `testsrc2` with RGB `lutrgb=r=val*0.55+50:g=val*0.55+50:b=val*0.55+50`.
The others use 256x144 at 12 fps, except the 1280x720 matrix-only HD witness.
All actual sizes and sampling times are in `references.json`.

Captures use the hidden desktop app, Electron 44.0.0 / Chromium
152.0.7977.54, on macOS ARM64. Build before acquiring `pm-electron-lock`;
generate outside the lease. The first 32 source references are byte-identical
to the original capture set. Six more frames are the app-imported ProRes
422/4444 playback renditions. Chromium cannot decode the MOVs directly:
`PM.pickFiles` creates the persisted VP9 WebM used by today's timeline.
Both MOVs and the actual, byte-verified `.playback.webm` files are retained.

The 38 `.rgba.zlib` rasters are the actual paused Canvas2D source uploaded
through WebGL with `UNPACK_PREMULTIPLY_ALPHA_WEBGL=true`, top row first,
RGBA8. They are raw synthetic source pixels, not desktop screenshots.
All these original recipes are opaque, so this premultiplied upload equals
the original Canvas bytes exactly. The three `.live.rgba.zlib` rasters are
separately captured direct video-to-WebGL uploads for the BT.2020 cache test.
`references.json` binds media names, dimensions, times and uncompressed
SHA-256 values. Additional tag/range and nonuniform-alpha references are
in the adjacent `video-parity-round2` directory.

The source regression draws all 38 frames in Float16 and Float32 working
compositions and requires **max 0**. The cache-switch regression checks
Canvas -> live -> Canvas without flushing between readings. Its Canvas
bound is also zero; one direct-upload pixel remains at the pre-existing
1-CV live bound, explicitly recorded in `docs/engine/video-parity.md`.
No original reference or bound was relaxed to obtain the exact source pass.
