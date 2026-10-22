# Media representation ladder (Phase 4a)

`pm-media` keeps the original reader as the export and analysis source. A
viewer can request `DecodePurpose::DisplayReduced { width, height }` through
`DecodeService::session` or `ViewerDecode::fetch_with_purpose`. The existing
`Display` and `Export` calls still decode full size. Purpose is already part of
the session and decoded-frame cache keys; scrub workers and stand-ins now use
that same purpose in their own identity. A reduced result cannot be returned
for an export or original-viewer request.

For ProRes, `VtDecoder` sets VideoToolbox's
`kVTDecompressionPropertyKey_ReducedResolutionDecode` on its own session. A
failed property set or a full-size output is an error. There is no post-decode
resize hiding an unsupported decoder. ProRes 422 uses the display `x422`
format; `Export` continues to use `v216`. The decoder may choose the smallest
supported size at least as large as the requested width and height, per Apple's
contract; callers should use the frame's actual dimensions.

`image::read_reduced` asks ImageIO for a thumbnail at the requested maximum
edge while `image::read` preserves its existing format-specific original
reader. Tiled mipmap EXRs read the finest stored level that fits the request
through OpenEXRCore. This native EXR path currently requires a zero-origin
data window equal to the display window. Other EXRs build scene-linear,
premultiplied box mips in memory, at float precision, from the exact original
reader. No EXR mip path writes a file.

`image::mip_cache::TiledMipCache` divides a reduced packed-RGBA still into
256-pixel tiles, preserving its sample bits and color metadata. It has a byte
budget and LRU eviction. Its key includes path, file length, modification
time, inode, device, change time, target size, and tile coordinates. Cache
instances should be discarded when a source is relinked. This is a memory
cache, with no generated image files. The current full-layer renderer uses
`read_reduced`; the tile API is for consumers that fetch subregions. Full-
resolution exports never query either reduced API.

The renderer's `set_preview_decode_size(Some((width, height)))` applies the
viewer target in display coordinates. For ProRes it requests a coded size
through VideoToolbox after accounting for clean aperture, pixel aspect ratio,
and rotation. A decoded frame's actual size drives its texture allocation;
the clean-aperture crop is scaled from original coded pixels to that size.
Other video codecs continue to use their original decode. Level 2 stills
use ImageIO thumbnails or EXR mips when the source is larger than the target;
level 1 stills retain the Chromium-compatible bitmap path. Converted-frame
and still-texture keys distinguish original and reduced pixels and their
requested size. Changing preview size clears kept draws. Export requests
force the original reader and decode purpose even if a preview target remains
set on the renderer.

`Renderer::media_representation()` reports `Original` or
`ReducedResolution` for the media actually drawn in the last render, including
cache hits. It resets each render. `set_playing` enables exact original-size
video fetch timing. `decode_measurements()` returns each path's rolling p95
in milliseconds after eight uncached exact fetches. `slow_media()` reports a
path when that p95 exceeds the composition frame budget; reduced frames,
frame-cache hits, stand-ins, scrubs, and exports do not count.

Import-time `probeMedia` also measures the original, full-size display decode
on the utility probe lane. `decode::measure::measure_source_with_pause` opens
the original path and samples up to sixteen fresh exact sequential frames;
the first random seek in each sampled section is reported separately. Its
frame cache is disabled, its decoder session holds one output frame, and it
does no read-ahead or file generation. The probe lane waits before opening,
before each frame, and between decoder input batches whenever playback or
higher-priority job work is active; paused time is excluded from measurements.
Interpret Footage receives readable machine facts: Decode time
(p95), Frame budget, Random seek time, Measured frames, and Proxy recommended.
Insufficient samples return `Unknown`, never an assumed `No`. The measured
recommendation is decode-only evidence; the full policy must combine it with GPU
headroom and the other machine-policy inputs.

Focused tests cover ProRes partial decode dimensions and exact export bytes
before and after, ImageIO thumbnail dimensions and original PNG bytes, stored
EXR mip dimensions, tile size/budget, source-file preservation, and renderer
export pixels before and after reduced preview. The legacy viewer's half-scale
pixel parity remains an integration gate; these APIs do not assert that
different resamplers produce identical pixels.
