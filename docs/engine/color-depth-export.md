# Color depth 8 export gate

`engine/crates/pm-export/tests/color_depth_export.rs` uses a deterministic
three-patch project saved with `colorDepth: 8`. Every job requests
`Precision::Float32`, as the app's `wireSettings` does, and enables dither for
8-bit PNG only (EXPORT-DITHER-1). The saved project depth still governs the
native frame: the test checks every export plan's device target and an actual
`Renderer::render` result for `Rgba8Unorm`. Jobs run through
`pm-export::render::run_export`. A second scene has a half-alpha patch and a
half-opacity overlap on green, exposing blend and alpha quantization that
opaque flat colors cannot. Temporary directories are removed by `Drop` after
success or failure.

The matrix follows the shipped delivery formats: ProRes Proxy, LT, 422, HQ,
4444, and 4444 XQ; H.264; HEVC 8/10-bit in MP4 and MOV plus 8-bit HEVC alpha;
VP8 and VP9 WebM with and without alpha; and 8/16-bit PNG stills and
sequences, including opaque and transparent output. The live export preview
provides JPEG: the application does not offer JPEG sequences as a delivery
format.

The exporter's own verification checks codec metadata and sampled pixels.
Independently, the test decodes PNG and JPEG, probes video streams with the
shipped FFprobe, and decodes video frames with the shipped FFmpeg. PNG still
and sequence first frames must match byte for byte after decoding. Three
interior patch anchors distinguish top from bottom and left from right; a
transparent corner checks alpha and row handling. The test reads 16-bit PNG
samples as `u16`, avoiding the image decoder's 8-bit downconversion. Opaque
8-bit working samples expand exactly by 257 in 16-bit PNG even though PNG8's
app-style settings enable dither and PNG16's disable it; the translucent
sample keeps alpha 128/32896 and its straight RGB agrees within half an 8-bit
code. This distinction matters because the shipped legacy PNG alpha path
rounds through 8-bit ImageData before Canvas PNG encoding; it does not offer a
16-bit PNG export.

FFprobe confirms BT.709 limited-range tags for every video format. FFmpeg is
given that matrix and range explicitly when decoding all three frames. Against
the 8-bit PNG anchors, maximum channel errors are 2 codes for ProRes 422
profiles, 0 for ProRes 4444, 2 for H.264, 1–3 for HEVC, and 1 for VP8/VP9.
The common video bound is 4 codes. At 128×96, FFmpeg's default VP8 RGB
conversion had overshot one anchor by 26 codes because it did not use the
encoded BT.709 matrix. For VP8 with and without alpha, an additional
dither-off depth-8/depth-16 native encoder pair preserves the prior codec
baseline: all 110,592 decoded RGB bytes across three frames match exactly.
App-settings output (dithered only for 8-bit PNG) keeps the same 4-code anchor bound.
JPEG previews retain their separate 24-code lossy-image bound.

For alpha video, FFprobe exposes `ALPHA_MODE=1` on VP8/VP9 WebM. The test
selects the libvpx decoder by name and independently decodes RGBA; transparent,
opaque, and half-alpha witness pixels decode to 0, 255, and 128 in both codecs.
FFmpeg also decodes the HEVC-with-alpha MOV witness to RGBA 0/255/128, while its
FFprobe video description shows two Main-profile `yuv420p` streams rather than
an alpha pixel format. The native verifier separately checks the HEVC `almo`
sample-description box and sampled decoded pixels. ProRes 4444 straight and
4444 XQ premultiplied are independently decoded to RGBA as well; their
transparent, opaque, and half-alpha witnesses are 0/255/129 in both profiles.

Run with `source engine/env.sh && cargo test -p pm-export --test
color_depth_export -j 4 -- --nocapture` from `engine/`. This gate requires
macOS, VideoToolbox, and the pinned LGPL FFmpeg build.
