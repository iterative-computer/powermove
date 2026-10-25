# Quality ledger

Every place the native engine deliberately renders differently from today's app because
today's output is measurably wrong (QUALITY-1, QUALITY-2, [gate-policy.md](gate-policy.md)).
Authored intent never changes here. A row records a correction, the standard it follows,
and the proof.

## Row format

One section per area (for example `## Video`, `## Text raster`). Each section states once:

- **Reference:** how the f64 or supersampled reference is computed and the standard it
  follows (for example "H.273 and BT.2100 in f64", "exact coverage, 16 × 16 cells",
  "W3C Compositing and Blending §10 in E").
- **Gate:** the test that proves the engine is at least as close as today.
- **Units:** 8-bit display code values (CV) unless stated.

Then one row per correction:

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|

## Required evidence

- Both distances come from the row's gate, on the cases it names. Describe cases; never
  name private corpus projects.
- Before/after side-by-sides stay outside Git, in the lane's evidence directory, linked
  from the lane report.
- The corrected content renders the same in the preview and the export.
- A correctness fix needs no decision, even when it shifts every project's look. Only
  a correction where "correct" is genuinely ambiguous (no standard applies, or tags
  conflict) cites the orchestrator's decision.

Rewrite a row when its numbers change. Keep no round history here.

## SDR output gamut

- **Reference:** CSS Color 4 OKLCh chroma reduction with deltaEOK JND 0.02,
  independently evaluated in f64 by `pm-reference`. Mapping operates on straight
  encoded sRGB before output alpha and quantization. BT.709 output tags retain
  their declared transfer; float and Display P3 outputs keep extended RGB.
- **Gate:** `pm-render/tests/color_depth.rs` retains an extended shader value
  through a depth-8 project's float frame, then verifies ordinary and snapshot
  byte targets against the independent mapped result. CPU output, viewer alpha,
  and exact GPU encoder half-byte rounding have separate boundary tests.
- **Units:** CV; maximum, p99.9 and mean per-pixel error on the uniform fixture.

| Correction (standard) | Clipped output vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Preserve extended color to one CSS Color 4 gamut mapping | 159.970, 159.970, 159.970 | 0.066, 0.066, 0.066 | Extended red shader, 8-bit ordinary and snapshot output |

## 3D output and Library cards

The scene pass writes linear RGBA16F samples (4× MSAA, memoryless on Apple GPUs),
resolves them in hardware and applies the preserved r180 ACES curve once at the
composite boundary, after transparent blending (decision `3D-TONEMAP-1`); masks and
effects keep their layer boundaries. Metal truncates f32 to half at a render-target
store, so every 3D color program and the encoded scene output store the IEEE
nearest-even half computed in the shader.

- **Reference:** independent Three.js r180 shading into RGBA32F at eight samples per
  axis, averaged in linear light and output-converted with f64 arithmetic.
- **Gate:** `tests/scene3d.rs` and `tests/scene3d_gltf.rs` (displayed pixels and
  geometry), and the Editor-view Library cards (`e2e/library-card-parity.spec.ts`).
- **Units:** CV; card statistics over the cube's pixels, signed mean per channel.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Masked object depth-tests itself (D-R5) | 135.06, 135.04, 1.436 | 0.95, 0.49, 0.003 | `layer-opacity-and-mask:0` interior |
| X-ray translucency blended in linear light (D-R6, `3D-TONEMAP-1`) | 23.70, 23.66, 10.803 | 0.07, 0.07, 0.016 | `look-xray:2` interior |
| Legacy scene shaded and resolved in linear light | 64.18, 61.66, 0.371 | 3.72, 0.33, 0.006 | `legacy-scene:0` interior |
| Nearest-half 3D and working stores (Editor Library card) | 25, 5, 0.0072 (card, signed +0.0008) | 25, 5, 0.0122 (card, signed +0.0005) | Editor-view card, depth 16 |

| Transmission samples the authored background (r180 `renderTransmissionPass`) | 92.57, 91.62, 2.856 | 5.61, 0.47, 0.004 | `khr-GlassVaseFlowers` interior |

Library cards pass gate-policy's precision-bound rule (all 32). Their remaining
one-code differences are the half-float working frame rounded again to 8 bits: the
Editor card's interior mean is 0.0073 CV (today 0.0004) and its edge mean 0.1601
(today 0.1587). The scene gate passes. In glTF, the two normal-tangent tests keep a few
isolated HDR highlights: one of four samples carries a large specular value,
which the linear resolve keeps a quarter of before the curve, today's
per-sample tone mapping clamps, and the 64-sample reference dilutes; and edge
p99.9 statistics over small edge sets sit up to a few codes above today's.

## Raster sampling and masks

- **Reference:** decoded level-1 source texels, independent f64 affine Lanczos-3
  with the declared anti-ringing clamp and exact outline coverage; parametric
  mask distance, feather and compositing formulas evaluated in f64.
- **Gate:** `pm-render/tests/corpus.rs`, still and parametric-mask cases. Pixel
  classes and precision bounds follow [gate-policy.md](gate-policy.md). Original
  working pixels also have to equal the same-size paused preview/export. Geometry
  is measured separately at Float32 to avoid treating half-float alpha rounding
  as shifted positions.
- **Units:** CV; each entry gives max, p99.9, mean. Other corpus cases remain
  subject to the unchanged bounds, including cases without a complete reference.

| Correction (declared kernel / authored mask formula) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Float sampling and analytic quad coverage | 10.10, 10.10, 0.063 | 0.02, 0.02, <0.001 | Transparent alpha still |
| Same sampling through opaque output quantization | 9.08, 9.08, 0.057 | 0.27, 0.27, 0.002 | Same alpha still, opaque view |
| Parametric mask coverage with float sampling | 144.06, 110.89, 10.156–10.621 | 0.25, 0.22–0.23, 0.068 | Three transparent rectangle/ellipse-mask frames |
| Minified stills read their own box mip chain (declared Lanczos-3 / trilinear kernel) | zone plate min PSNR 17 dB (bilinear, no mips) | min 70.78 dB vs the declared kernel; 29.67 dB Final, 23.08 dB Playback vs the output-space low-pass | `antialiasing_bar_zone_plate`, level 1 and 2, Float16/32 (units: dB) |

A still is uploaded with its full mip chain in the same texture, filled once by
the compute downsampler with nearest-rounded box means. That costs a third more
still memory and no extra texture; `export_120`'s texture-byte baseline was
raised by the two stills' chains (174,760 bytes), the only ratchet change.

The still subset passes all six frames and the mask subset all three. Broader
blend cases expose the existing hard pixel-centre edges of level-1 solids; they
remain red against exact coverage, including at Float32. This is independent of
mask alpha quantization and is not waived.

## Text raster

- **Reference:** the byte-exact uploaded Canvas raster, sampled bilinearly in f64
  at output pixel centres through the authored quad, with its opacity over the
  independently evaluated f64 document background. The quad and full-canvas
  placement remain exact.
- **Gate:** `pm-render` example `color_depth_parity`, using the synthetic
  `color-depth.ts` captures. Resampled text uses `pm_reference::compare` with
  `EIGHT_BIT` at depth 8 and `HALF_FLOAT` at depths 16 and 32; other pixels stay
  byte-exact. The depth-8 PNG export equals the preview. `pm-node`'s
  `render_fonts` also requires exact full-canvas pixels after placing the ink crop.
- **Units:** CV. Maximum, p99.9 and mean differences from today's error must each
  stay within the stated precision bound, with the original per-pixel bar.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Retain only ink without reproducing full-quad f32 interpolation (bilinear pixel-centre reference) | 1.06, 0.98, 0.421 | 1.04, 1.00, 0.421 | Rotated text, depth 8 |
| Same ink-only quad at half-float depth | 0.50, 0.40, 0.078 | 0.52, 0.40, 0.075 | Rotated text, depth 16 |
| Same ink-only quad at float depth | 0.40, 0.37, 0.036 | 0.45, 0.30, 0.033 | Rotated text, depth 32 |

## Video

- **Reference:** each clip's decoded codes in f64 from `pm-reference` (H.273 ranges and
  Kr/Kb, primaries from the published chromaticities through RP 177, sRGB, ST 2084,
  BT.2100 HLG, the BT.2390 EETF, and Lanczos-3 chroma at the tagged H.273 location), with
  each clip's reading written by hand from its tags and the shared table
  (`pm-render/tests/fixtures/video/cases.json`). Chroma filtering is judged against a
  4:4:4 original instead.
- **Gate:** `pm-render/tests/video_reading.rs` (engine within 0.59 / 0.063 / 0.004 CV of
  the reference at 8 / 16 / 32-bit on pixels whose chroma is flat over the
  reconstruction footprint, and at least as close as today everywhere), and
  `pm-render/tests/chroma_truth.rs` for the filter. `video_reading.rs` proves
  the same video reading in paused preview and export. The G-C4 presentation
  gate separately checks exact equal-size viewer/export pixels and snapped
  device geometry; it no longer reproduces the legacy viewer's canvas scaling.
- **Units:** CV. Engine columns are 16-bit projects; their max and p99.9 sit at chroma
  edges, where the engine's Catmull-Rom and the reference's Lanczos-3 legitimately
  differ.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Chroma at its tagged H.273 location (left for H.264, HEVC and untagged) instead of centered | 86, 77, 5.6 | 38, 29, 2.3 | fully tagged 8-bit H.264 |
| Chroma filter: Catmull-Rom across for 4:2:0 and 4:2:2 instead of bilinear, closest of the candidates to a 4:4:4 original subsampled by two encoders' filters, in natural, graphic and edge regions alike (RMS 11.5 vs bilinear 11.8 vs today 13.2 at 4:2:0; 9.4 / 9.7 / 11.5 at 4:2:2). Rows stay linear by choice: Catmull-Rom rows measure 5.6–6.5% lower RMS, but linear rows keep the 4:2:0 export round trip exact, so a re-export loses no generation | against the original: 166, 124, 10.9 | 157, 103, 9.0 (one case's max 0.3 above today's, within the 0.5 CV bound) | `chroma-*` (natural and graphic halves) |
| Full-range H.264 no longer samples the decoder's cleared coded padding at the bottom edge | 167, 154, 6.3 | 39, 26, 2.0 | 258×146 full range |
| 10-bit kept at half float or better, no RGBA8 store | 157, 100, 6.9 | 93, 47, 2.1 | 10-bit HEVC, full and limited |
| BT.2020 and Display P3 SDR converted once in float (BT.2087 linear light); today converts through two RGBA8 stores, or not at all while playing | 157, 106, 6.9 | 103, 57, 2.5 | H.264 and HEVC BT.2020, HEVC P3 |
| Untagged SD primaries by BT.601 system (VIDEO-SD-1): 576 active lines BT.470BG, other SD sizes SMPTE 170M, converted to Rec.709 in linear light; tagged 170M likewise; today reads them as Rec.709 | 89, 79, 15 | 39, 26, 2.2 | SD clips with 170M or no primaries tag |
| Untagged 160×90 H.264 red still in the viewer: SMPTE 170M primaries converted to Rec.709 (VIDEO-SD-1), where today's browser displays the decoded red as Rec.709 | 36, 36, 29.503 | 1, 1, 0.015 | `h264-aac.mp4` at 0.5 s, all 455,118 video-box stage pixels across 18 zoom/preview views, including scaled borders; editing-copy views use today's original-video control. Interior Y′CbCr (81, 90, 240) gives f64 reference (247.553, 35.877, 0) CV, native (248, 36, 0), today (254, 0, 0) |
| Untagged and partly tagged video read by the shared table: untagged SD VP9 BT.601, a lone Rec.709 matrix tag kept | 87, 81, 23 | 39, 28, 2.1 | untagged VP9, matrix-only H.264 |
| Superwhites and sub-blacks kept through linearisation and primaries conversion, allowing exposure to recover highlights before display clips them | 8.233, 8.233, 0.935 | 0.500, 0.500, 0.135 | `superwhite-*`; displayed output of 32-bit projects |
| PQ and HLG decoded per BT.2100 and tone mapped with the BT.2390 EETF to 203 cd/m² SDR white (HDR-1), in float; today: RGBA8 before Skia's RWTMO curve paused, untone-mapped playing | 255, 255, 42 | 170, 99, 2.1 | HEVC/AV1 PQ and HLG, P3 PQ |
| VP9 alpha premultiplied in float (today: an 8-bit straight store, nearest chroma) | 120, 93, 11 | 27, 18, 1.1 | VP9 alpha |
| HEVC alpha read as its `AlphaChannelMode` tag says (premultiplied, VIDEO-ALPHA-1); today ignores the tag | 172, 150, 71 | 37, 22, 1.7 | HEVC alpha |
| ProRes 422 and 4444 decoded natively (today shows a VP9 rendition) | Not applicable to the original stream: Chromium cannot decode it; comparing the VP9 rendition would also measure transcode loss | 0.03, 0.03, 0.02 | ProRes 422, 4444 with alpha |

The superwhite row measures 20 neutral band centers (Y′ 8, 16, 128, 235,
250), at zero and −1 stop exposure, in SD and HD clips. The independent
reference is `clamp(2^exposure × (Y′ − 16) / 219, 0, 1) × 255`.
`e2e/engine-video-superwhite.spec.ts` reads the legacy compositor and native
snapshot routes in Electron. Before display clipping, the Rust float-buffer
gate measures max / p99.9 / mean 0.000067 / 0.000067 / 0.000009 CV against
the unclipped reference over the same band centers.

## Video endpoint selection

- **Reference:** source frame timestamps and final-frame hold, independent of
  composition rate. Each synthetic clip has 20 numbered grey frames.
- **Gate:** `pm-render/tests/video_end.rs`, direct and nested compositions at
  matching and mixed rates. Native frame identities are read from GPU output;
  the legacy column evaluates the former 40 ms seek-padding rule, rather than
  measuring a running browser. Units are source frames, sampled at all 20
  source-frame starts. The hold after the source end is also tested.

| Correction | Legacy rule vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Remove browser seek padding at 30 fps | 1, 1, 0.05 | 0, 0, 0 | direct and nested |
| Remove browser seek padding at 60 fps | 2, 2, 0.15 | 0, 0, 0 | direct and nested |
| Remove browser seek padding at 120 fps | 4, 4, 0.50 | 0, 0, 0 | direct and nested |
| Preserve selection at 10, 24 and 25 fps | 0, 0, 0 | 0, 0, 0 | direct and nested |

## Transparent PNG export

- **Reference:** the premultiplied working value divided by alpha in f64, rounded once.
- **Gate:** `pm-export` `convert::tests::transparent_png_unpremultiplies_the_working_floats`.
- **Units:** CV, over visible alpha.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Straight colour is the working float divided by alpha (today: rounded to 8 bits, truncated on unpremultiply, premultiplied and unpremultiplied again by Canvas2D) | 167, 78, 1.7 | 0.50, 0.50, 0.25 | 400,000 random premultiplied pixels |

## Source monitor

Reference: independent f64 Y′CbCr conversion (H.273), the documented SDR
transfer reading, linear-light primaries and Bradford adaptation into the
capture's display ICC. `e2e/helpers/source-colour-reference.ts` implements the
math independently of production. Gate: `engine-source-compositor.spec.ts`.
Numbers below are CV over flat, opaque RGB samples in the light-theme,
full-size capture at 0.5 s; the gate also checks dark and intrinsic-size
captures. Max, p99.9 and mean describe that same sampled population.

| Correction / case | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean |
|---|---|---|
| Tagged full-range HD H.264 | 11.801, 11.799, 4.163 | 0.538, 0.494, 0.173 |
| Untagged HD H.264 | 11.700, 11.700, 4.409 | 0.524, 0.507, 0.196 |
| Untagged portrait H.264 | 10.568, 8.998, 3.397 | 0.462, 0.453, 0.152 |
| Partially tagged BT.709 matrix with SD primaries | 26.151, 26.151, 6.111 | 0.513, 0.513, 0.154 |
| 10-bit HEVC (P010 decode, presented as I420P10) | 8.820, 8.820, 2.760 | 2.054, 2.054, 0.781 |
| Untagged SD ProRes: today shows its 8-bit VP9 rendition with Rec.709 primaries; the engine converts the original 10-bit 4:2:2 itself and presents sRGB RGBA | 11.085, 11.085, 6.306 | 2.567, 2.567, 0.972 |
| SD VP9 with alpha (opaque samples) | 11.939, 11.939, 2.385 | 0.490, 0.490, 0.113 |
| Tagged BT.470BG PAL | 24.744, 24.744, 8.567 | 0.536, 0.529, 0.187 |
| Untagged 576-line PAL | 26.126, 26.126, 6.550 | 0.536, 0.529, 0.187 |
| Untagged HD VP9 | 11.849, 11.673, 4.193 | 0.529, 0.509, 0.176 |
| Untagged image-sequence rendition, BT.601 matrix | 26.802, 26.802, 7.265 | 0.529, 0.524, 0.226 |
| BT.2020 SDR through the sRGB working gamut | 20.558, 20.558, 6.509 | 12.273, 12.273, 1.446 |

The 8-bit HD corrections remove Chromium's VideoToolbox transfer lift;
partial tags retain the specified matrix. SD primaries follow VIDEO-SD-1,
including BT.470BG for 576-line sources. Sequence renditions retain the
BT.601 matrix used by their encoder.

Residuals remain: Chromium presentation of 10-bit planes exceeds one CV;
engine-converted frames (original ProRes, BT.2020) reach the display as 8-bit
sRGB RGBA, which Chromium converts again into the display gamut; and BT.2020
converted through the sRGB working gamut differs from direct conversion into
the display gamut. These are measured limitations, not
reference-accuracy claims. PQ playback is checked for an SDR capture; this
screen gate does not supply an independent HDR tone-mapping reference.

Fully tagged 8-bit SDR and untagged non-PAL SD H.264 retain their correct
monitor reading. Interpret Footage uses the same native conversion as a
composition layer (`source_snapshot`), and an open paused still/video monitor
repaints Apply, Undo and Redo without seeking (`engine-source-refresh`).
The source-frame/thumbnail gate independently decodes original media for
native source consumers and the actual browser rendition for legacy consumers.
For the solid SD red H.264 fixture at 0, 0.25 and 0.5 s, the f64 sRGB reference
is `(247.554, 35.881, 0)`. All snapshot RGB channels give these CV distances:

| Source snapshot | Today: max, p99.9, mean | Engine: max, p99.9, mean |
|---|---|---|
| H.264 source | 35.881, 35.881, 14.109 | 0.446, 0.446, 0.188 |

The native result rounds to `(248, 36, 0)`. Posters match an independently
synthesized reference encoded at the shipped WebP quality exactly; inline
thumbnails retain that poster. Theme captures additionally compare layout and
ICC-transformed reference colours. This gate requires half-code snapshot
rounding accuracy, not equality with Chromium's Canvas2D colour bug.

Frame identity at open-GOP boundaries. Reference: FFmpeg's sequential decode
in presentation order; the shown frame is the one whose interval holds the
time. Gate: `pm-media/tests/random_access.rs` (`s6_corpus`: every frame of
open-GOP H.264 and HEVC by random access equals the sequential decode). Units
are source frames.

| Correction | Today vs reference | Engine vs reference | Cases |
|---|---|---|---|
| A seek to a leading picture of a CRA shows that picture (today: the CRA, the next keyframe) | 1 frame (frame 30, the CRA, for frame 29) | 0 | 4K HEVC Main10, keyint 30, open GOP |

### Source sampling

- **Reference:** independent f64, normalized Lanczos-3 over binary source
  texels with clamp-to-edge support. The previous source draw's unmipped
  bilinear result is exact at these pixel centres. This follows the declared
  composition sampling kernel (`pm-render::sampling`), with no monitor filter.
  Fractional fit edges use the exact area of each output pixel inside the
  centred source rectangle, independently calculated in f64.
- **Gate:** `source_snapshot::monitor_4k_checker_is_closer_to_f64_than_unmipped_bilinear`,
  all 921,600 pixels classified Resampled under unchanged `EIGHT_BIT` bounds,
  including per-pixel excess. The paused monitor equals the source export.
  `fractional_source_contain_is_closer_to_exact_coverage` also checks alpha area
  and centroid with `same_geometry` at the policy's unchanged 1e-3 bound.
- **Units:** CV, maximum channel difference.

| Correction / case | Previous monitor vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean |
|---|---|---|
| Lanczos-3 when paused, 3840×2160 one-texel checker → 1280×720 | 127.50, 127.50, 127.500 | 0.50, 0.50, 0.500 |
| GPU mip pyramid and trilinear playback, same checker | 127.50, 127.50, 127.500 | 4.01, 0.51, 0.500 |
| Exact fractional fit coverage, 160×90 opaque red → 1464×824 contain | 63.75, 63.75, 0.155 | 0.25, 0.25, 0.001 |

Playback's maximum is at the clamped outer support; every output pixel is
closer than the previous draw. No colour or geometry tolerance is widened.
