# Original direct-video upload oracle

These 38 compressed RGBA8 rasters are the direct WebGL video texture readings
captured by the legacy desktop app from the original public synthetic clips.
Every frame is opaque, so the direct texture equals the app's premultiplied
live upload. They are separate from the existing Canvas rasters; the two
readings differ for some sources. The existing fixture and its hashes remain
unchanged.

`manifest.json` binds every raster to its source media and, for imported
ProRes, the exact playback rendition. It records SHA-256 of the uncompressed
RGBA bytes, source and playback media, frame size, sample time, presentation
timestamp, and Chromium/Electron version. The strict native test compares all
38 frames at Float16 and Float32 working precision with zero tolerance.

Raw app captures and operational reports stay outside the repository. Only
synthetic video bytes and compressed pixel references are included here.
