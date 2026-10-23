# Synthetic video conversion references, round 3

Sixteen short public clips extend the paused-frame source gate to VP9 and AV1
10-bit streams, VP9 and HEVC alpha, full-range 8-bit H.264 in BT.601/709/2020,
BT.2020 PQ/HLG, and Display P3 PQ. Their `testsrc2` pattern is modified with
deterministic luma/chroma midtone gradients, and the alpha clips have a
horizontal alpha gradient. The H.264 full-range clips use 256×160, 272×160,
256×150, 258×146, and 272×144 geometry to expose coded/visible-height
boundaries. Each clip is
sampled at 0.125, 0.625, and 1.125 seconds.

`references.json` records the source SHA-256, stream tags, actual Chromium
VideoFrame format, coded/visible/display dimensions, copyTo plane offsets and
strides when exposed, selected PTS, color space, and uncompressed pixel hashes.
The external capture directory additionally retains full-coded `VideoFrame`
plane copies (`.coded`) and their SHA-256 values, including padding beyond the
visible rectangle. Those diagnostic planes are not packaged as goldens.
The four compressed RGBA8 references for each captured frame are direct video
WebGL upload (`rgba`), that same video uploaded with
`UNPACK_PREMULTIPLY_ALPHA_WEBGL=true` (`live`), straight-alpha Canvas2D
(`canvas`), and the same drawn Canvas uploaded with that premultiply flag
(`premult`). The Canvas upload is `nativeReference`; the video upload is
`nativeLiveReference`. Rows are in source top-down order. These are synthetic
pixel rasters, not screenshots.

HEVC alpha is encoded by macOS VideoToolbox via FFmpeg's
`hevc_videotoolbox -alpha_quality` path. Rewriting that HEVC bitstream's color
metadata damages the auxiliary alpha on the tested encoder, so its color tags
remain unspecified. Chromium interprets this SD-sized video as BT.601. The
clip is identified by its decoded nonuniform alpha and `references.json`
captures Chromium's actual interpretation. VP9 alpha has
a WebM `alpha_mode=1` track. FFmpeg's native VP9 decoder may omit that alpha;
`libvpx-vp9` exposes it. `gaps` in the manifest lists any actual app decode
error. Such cases have no invented reference or implicit pass. Packaging also
rejects constant alpha or missing premultiplication in either upload path.
A native decoder refusal for a captured case fails the strict test.

To regenerate in the same app environment, from the repository root:

```sh
bun install --frozen-lockfile
bun run --cwd apps/desktop build
node apps/desktop/scripts/video-parity.mjs generate /tmp/powermove-video-parity-round3 --round3
~/Library/Caches/powermove-engine/bin/pm-electron-lock nice -n 15 node apps/desktop/scripts/video-parity.mjs capture /tmp/powermove-video-parity-round3 --round3
node apps/desktop/scripts/video-parity.mjs pack /tmp/powermove-video-parity-round3 --round3
```

The harness launches a temporary, hidden legacy app profile and removes it on
success or failure. Keep raw reports and app data outside Git. Only synthetic
media, compressed references, and their metadata belong here.
