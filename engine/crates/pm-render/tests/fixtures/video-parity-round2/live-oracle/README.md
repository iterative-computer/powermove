# Round 2 premultiplied live-video upload oracle

These 24 compressed RGBA8 rasters record the legacy app's actual WebGL video
upload with `UNPACK_PREMULTIPLY_ALPHA_WEBGL=true` for the eight public Round 2
synthetic sources. ProRes 4444 contains a nonuniform alpha gradient; its three
live rasters were captured from the video element and are not arithmetically
derived from straight RGBA. The other sources are opaque.

The app reimported the same eight source files to measure this path. Its new
straight video, Canvas, and premultiplied Canvas rasters matched the earlier
capture byte for byte for all 24 frames. ProRes proxy containers can vary
between imports, so the two fresh playback renditions used by this capture
are included and SHA-bound in `manifest.json`. Earlier references, source
videos, and earlier playback renditions remain unchanged. The strict native
test compares all 24 live frames at Float16 and Float32 working precision
with zero tolerance.

All files are synthetic media or compressed pixel references. App profiles,
screenshots, and operational reports stay outside the repository.
