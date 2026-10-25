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
