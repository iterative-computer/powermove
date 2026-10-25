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

## 3D Library cards (C3)

- **Reference:** the engine's render of the same pinned view at 4× per axis (each
  sample 4× MSAA), box-averaged to card pixels in the card's encoding. It is the
  engine's own 8-bit render averaged in gamma space; the 3D lane's linear-light
  resolve should replace it with an independent f32 reference.
- **Gate:** `e2e/library-card-parity.spec.ts`, 3D cases. Pixels away from edges are
  exact. A differing edge pixel (3×3 span above 8 CV in today's card) must be, per
  channel, no further from the reference than today's.
- **Units:** CV, max channel difference, over the pixels where today and the engine
  differ.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| MSAA edge resolve: Metal hardware resolve instead of WebGL's (box filter over samples) | 1, 1, 1.00 | 0, 0, 0.00 | Camera view, 4 viewers × 2 themes (35 edge pixels) |

Open, gate failing: in the Editor view, today is 20, 20, 5.02 and the engine is
19, 19, 4.65 (132 edge pixels, 16 cards), but 4–5 pixels per card are one CV
further from the reference than today's. Owner: 3D lane (linear-light resolve).
The raw camera frame's one interior pixel, `(799, 448)`, is a rounding-midpoint tie:
a surface fit over 2,805 face pixels gives 150.50 ± 0.02, and today (151) and the
engine (150) are both 0.50 away.

## Text raster

- **Reference:** independent f64 outline coverage, starting at 32 × 32 cells and
  refining to convergence, at exact
  glyph positions, premultiplied Porter-Duff source-over in the project's
  blend domain, pixel-integrated Gaussian weights, and source-texel area
  integration for minified bitmap glyphs. Layout is a separate exact gate.
- **Gate:** `text_raster` (G-T1), `text_corpus` (G-T2), and installed-app
  `native-engine-text-app-parity` (G-T5) replace text byte equality with the
  reference-closeness rule in [gate-policy.md](gate-policy.md). Captures are
  today's witness. Unmodeled pixels retain their original bound.
- **Units:** CV, maximum channel error per classified edge or image pixel.
  G-T1 rows below use mapped sRGB8 display pixels; raw float errors are
  reported separately where stated.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Geometric outline coverage without platform dilation or luminance preblend (TEXT-RASTER-1); float coverage and ordered paint | 162, 157, 19.802 | 0, 0, 0 | G-T1 small text case 0 at the sRGB8 boundary; raw float max 0.00016 CV; exact area/centroid gate passes |
| Original emoji texels integrated over each inverse output pixel in the working domain | 131, 101, 2.68881 | 1, 1, 0.02170 | Minified bitmap glyphs, G-T1 case 519; authored Apple placement retained; raw float max 0.01084 CV |
| COLR authored paint graph composited in float, with f64 area coverage | 212, 211, 24.82834 | 1, 1, 0.00998 | Palette-alpha glyph, G-T1 case 525; independent program interpreter; raw float max 0.03353 CV |
| [CSS Color 4 local MINDE](https://www.w3.org/TR/css-color-4/#pseudo-binsearch) at the sRGB8 boundary, rather than channel clipping; extended RGB retained for float/P3 | 88, 88, 88 | 0, 0, 0 | 54 uniform glyph-interior pixels in the out-of-gamut OKLCh case; CPU and independent f64 mapping agree; shared WGSL max 0.0024 CV over 227 samples |

Text placement reaches output pixels through its world matrix. G-T1 keeps
layout exact and compares both raw float and mapped display pixels against
independent references. The CSS rule is authored color intent: sRGB8 maps
with deltaEOK JND 0.02; float and wide-gamut storage retain extended values.
The compositing lane owns final display, snapshot and export integration of
the shared mapper; out-of-gamut installed-app qualification remains pending
that boundary.

The rotated-text depth gates now use actual outline coverage rather than
two samplings of a Canvas raster. At depth 8, engine/reference max is
0.931 CV versus today's 75.437 CV; preview and PNG export are identical.
At depths 16 and 32, maxima are 0.216 and 0.00068 CV versus 75.741 and
75.807 CV. The per-pixel, p99.9 and mean rules also pass.

**TEXT-INK-1** (owner decision 2026-10-25): color-glyph ink (selection and
bounds) is what is drawn, not Canvas metric slack. Typography anchoring is
authored placement, not ink, so it keeps measuring Canvas's box.

| Correction (rule) | Today: case 519 selection | Engine: case 519 selection | Proof |
|---|---|---|---|
| Color-glyph ink is the drawn area: nonzero strike texels through the painter's placement plus its reconstruction reach (bilinear enlargement: `scale/2 - 1/2` px), COLR painted/clip bounds | y0 -24, y1 201, x1 713 px: Skia's round-out plus 1 px outset of CoreText's metric rectangle, its ink edges 1 px above, 3 px below and 8 px right of the drawn pixels; the previous engine measured the `glyf` box (y0 -47, y1 175) | y0 -22.125, y1 197.625, x1 704.875 px; rounded out, exactly the nonzero-alpha box [15, -12, 694, 187] the engine drew | G-T5 case 519 (installed app, exact, RGBA8); `pm-text` painted-pixel test at 12–200 px and four sub-pixel phases, skin-tone included; `pm-eval` `color_ink` |

Typography anchoring maps a layer's recorded bounds (`fontAnchorBounds`,
today's selection) onto the current ones. It measures both as Canvas does:
for an `sbix` glyph CoreText draws without a path, Skia's glyph metrics
from CoreText's rectangle (registry query `SbixMetric`), rounded out and
outset 1 px, through the synthetic-italic skew, on the 64 px canonical
strike past 256 px. A project saved today therefore renders exactly where
today's app renders it, and engine-recorded anchors mean the same in both
engines. `pm-eval` `color_ink` holds that box to today's `measureText` at
12–300 px, italic included, and the resized placement to today's
arithmetic; G-T5 holds case 519's saved anchor to today's placement.

Composed-text rows outside this lane's reference coverage retain their
original bound and remain explicit owned failures: path masks with the
vector lane, projection and mixed composition with compositing/3D, and
layer effects with effects. They are rechecked after those owners integrate.

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
  `pm-render/tests/chroma_truth.rs` for the filter. `e2e/engine-viewer-canvas.spec.ts`
  checks the untagged SD fixture's native viewer colour against the same
  independent f64 H.273 / SMPTE 170M reference. It compares every pixel in
  the video box, including its scaled border, using today's original-video
  coverage with only the source colour corrected. Whole-stage geometry and
  nonvideo pixel parity retain their original bounds; the native snapshot's
  video interior is checked against the reference and the displayed frame.
  The viewer gate uses the largest RGB-channel error per pixel for aggregate
  max, p99.9 and mean, and checks the native-minus-today error at every pixel
  (99.9% at most 1 CV, none beyond 4 CV).
  `video_reading.rs` proves the same video reading in paused preview and export.
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

## Vector coverage

- **Reference:** authored fills and f64 stroke regions with their caps, joins, miter
  limits and dashes, covered at 16 × 16 cells per pixel (`pm-reference`); folded
  stroke regions use 64 × 64 cells. Strokes expand in local space before the world
  transform, as Canvas2D specifies.
- **Gate:** `pm-render/tests/vector_gate.rs` (G-V1), all 474 captured canvases.
  Geometry is checked separately at 1e-3. The corpus also checks paused
  preview/export identity; out-of-scope reference gaps use the measured owned
  pending ratchet in the gate policy.
- **Units:** CV, pooled edge pixels.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Exact area coverage replaces density-bucket rasterization (Canvas2D fill and stroke geometry) | 123.65, 59.01, 0.661 | 3.16, 0.02, <0.001 | 474 shape and path canvases |
| Authored stroke styles covered without raster seams (Canvas2D caps, joins, miter limits and dashes) | 123.21, 58.94, 0.658 | 3.16, 0.03, <0.001 | 209 stroked canvases from the same witness |

## Solid corner radius

- **Reference:** authored rounded-rectangle geometry, independently covered in
  f64 at 16 × 16 cells per output pixel and composited over the authored background.
- **Gate:** `pm-render/tests/corpus.rs` and `vector.rs::solid_radius_is_honored`.
  Actual legacy captures stay in the comparison; corrected interiors additionally
  prove authored paint directly, as described in the gate policy.
- **Units:** CV, worst per-frame max, p99.9 and mean over edge pixels.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Honor the authored solid radius (SOLID-RADIUS-1); today ignores it | 224, 224, 50.888 | 0.62, 0.57, 0.121 | Three captured solid placements; interior engine error is at most 0.50 CV |
| Exact area coverage of solid edges replaces pixel-centre rasterization (the box-filtered area coverage of the vector rows) | 81.6, 81.6, 81.6 | 0.60, 0.60, 0.500 | `pm-node/tests/compose.rs` crop far from the origin: a solid's two fractional edge columns (0.6 and 0.4 covered, 400 pixels) at 500% on a 2x display, through f16 compositing and the 8-bit display |

## Path mask coverage and feather

- **Reference:** f64 area coverage and the normalized separable Gaussian with
  transparent canvas edges, at `σ = max(0, feather) × |world x| / 2`, followed by
  authored add/subtract formulas.
- **Gate:** `pm-render/tests/path_masks.rs` (G-V2), with the captured legacy mask
  witness. All cases pass the per-pixel reference-closeness bar; sharp masks
  also preserve area and centroid at 1e-3. The corpus checks preview/export identity.
- **Units:** CV, worst per-case max, p99.9 and mean over edge pixels.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Exact path coverage and Gaussian feather replace canvas rasterization and the approximate blur | 95.25, 82.62, 3.876 | 0.080, 0.077, 0.016 | 54 masks and stacks, including transformed, subtract-only, HD and UHD cases |

## SVG coverage and paint

- **Reference:** independent f64 SVG geometry, gradients, Porter-Duff paint and
  luminance masks, integrated with exact subcell coverage on a shared grid.
  Text uses raw shaped glyph outlines before the engine's contour resolution.
- **Gate:** `pm-render/tests/svg.rs` (G-V3), against 43 actual legacy-app frames
  and 374 required Chromium semantic probe rasters. Geometry stays within 1e-3;
  every placement checks preview/export identity.
- **Units:** CV, worst per-case max, p99.9 and mean over resampled pixels.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Output-resolution coverage, float gradient integration and joint clip/mask compositing (SVG geometry and paint rules) | 170.45, 163.81, 21.368 | 6.757, 6.245, 0.0133 | Five SVG sources at contain/cover/stretch placements and 25–800% or animated scale, 43 frames |

## SVG playback resampling

- **Reference:** f64 coverage of the authored shapes and image crop under the
  frame's exact transform, 64 × 64 subpixels per output pixel.
- **Gate:** `pm-render` unit test
  `svg_sample::tests::playback_frames_are_closer_to_f64_than_todays_bilinear_draw`.
  Today is the same source raster stored as RGBA8 and sampled bilinearly at pixel
  centers (the legacy textured quad). Area and centroid stay within 1e-3 and 2e-3 px.
- **Units:** CV of alpha, worst case over Float32, Float16 and Unorm8 stores.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Playback frames integrate each pixel's source footprint (4 × 4 quadrature) of a float raster instead of one bilinear RGBA8 sample (area-correct resampling) | 53.01, 44.62, 0.569 | 15.63, 14.13, 0.265 | Rotated, scaled circle and masked stroke with a joint image crop |

Paused frames, exports and captures paint at output resolution (the SVG
coverage and paint row above).

## SVG filters

- **Reference:** Filter Effects Level 1 primitives in f64 (`pm_reference::filter`):
  exact source coverage, the box-reconstruction Gaussian, each primitive in its
  `color-interpolation-filters` space, regions by pixel area; SVG 1.1 filterRes.
  Paint, filter, clip and mask share a grid within an image; each image resolves
  before layer compositing.
- **Gate:** `pm-render/tests/svg_filters.rs` (four SHA-verified legacy-app witnesses,
  transformed covariance, filter grids) and `tests/svg_filter_graphs.rs` (nine public
  synthetic design-tool exports at 1× and 2×, Gaussians of 8 to 400 px, fractional
  regions); `e2e/engine-svg.spec.ts` checks live placements, exact window/native byte
  endpoints and f32 geometry at 1e-3.
- **Units:** CV, worst per-case max, p99.9 and mean. Rows marked "resvg" compare the
  declared RGBA8 fallback these documents drew with before (no legacy capture exists
  for them); its error is the bound the float path must beat.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Float Gaussian at device resolution and per-primitive linearRGB/sRGB domains, source halos and hard filter regions (Filter Effects, SVG 1.1) | 5.1233, 5.1233, 0.50966 | 0.0032, 0.0032, 0.00031 | Anisotropic blur, default linearRGB and explicit sRGB, hard region/source halo, zero blur |
| Shadow, glow and layer-blur graphs (flood, composite incl. arithmetic, merge, blend, colour matrix, component transfer, drop shadow) in float instead of the RGBA8 resvg fallback | resvg: 73.47, 18.06, 1.918 | 0.028, 0.016, 0.0016 | Figma drop/inner shadow and layer blur, Illustrator and Sketch shadows, CSS feDropShadow, neon and tinted glows, every per-pixel primitive; 1× and 2× |
| Gaussians of any size stay in float (the exact separable Gaussian, O(1) taps) instead of the over-budget resvg fallback from ~20 px | resvg: 3.70, 3.62, 1.327 (layer blur 25) | 0.0075, 0.0064, 0.00089 | σ 8, 25, 60, 150 and 400 px against the analytic Gaussian; Figma layer blur at 1× and 2× |
| Sketch's `<rect …></rect>` rounded corners as exact cubics (SVG rx/ry), as `<rect …/>` already were | 0.52, 0.44, 0.0014 | 0.018, 0.014, 0.0001 | Sketch shadow export, 1× |

Scenes beyond the device or filter-memory bounds and the primitives the float path does
not draw use the declared native resvg RGBA8 fallback, counted and named per layer.

## Half-float stores

- **Reference:** the nearest-even IEEE binary16 of each computed value.
- **Gate:** `pm-render/tests/vector_half.rs` and `tests/text_gpu.rs` read back
  values that a truncating store keeps an ulp low.
- **Units:** binary16 ulps.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Vector paint, path-mask coverage (r16float), SVG float outputs and text paint (outlines, images, blur passes) store the nearest half instead of Metal's truncation (IEEE 754 round to nearest even) | 1 ulp low | exact | 9/255 grey paint, 0.3 coverage edge, SVG at 16-bit depth, a text raster at 16-bit depth against the same paint at f32 |

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
