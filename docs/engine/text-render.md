# Text and captions rendering

Design notes for glyph rendering in `engine/crates/pm-text` (`render`
module) and text and captions layers in `engine/crates/pm-render`
(`text` module). Plan: section 12, section 10 (text and captions rows),
section 25 (text class), amendments 10 and 19. Spike: `docs/engine/spikes/S4.md`.
Layout (level 1 `compat`, level 2 on Parley, the font registry) is described
in `docs/engine/text.md`; word states, animator poses, raster boxes, and font
anchors are `pm-eval`'s (`docs/engine/pm-eval-layers.md`).

## What is drawn

| Feature | Status | Evidence |
|---|---|---|
| Text layers, `compat` level 1: every text property (font, weight, size, tracking, leading, alignment, italic, color, paragraph boxes, `fontAxis.*` variable axes, project fonts, synthesized bold and italic) | Drawn | `tests/text_raster.rs`, `tests/text_corpus.rs` |
| Text animators (stagger, range, wave; characters, words, lines; position, scale, rotation, skew, opacity, blur, color, tracking) and range styles | Drawn | `tests/text_corpus.rs` (`text/animator-*`, `text/range-styles`) |
| Captions: every preset, box, stroke, highlight, highlight scale, pop and fade word animations, text case | Drawn | `tests/text_raster.rs` (`caption-*`), `tests/text_corpus.rs` (`captions/*`) |
| Level 1 glyphs on macOS from CoreGraphics masks, as Canvas2D draws them (below, "Glyph masks"): every axis-aligned glyph below the large-text size, synthesized italic included | Drawn, pixel-identical on the corpus | `tests/text_raster.rs` (`families`, `weights`, `italic`, `scripts`, `tracking`, `paragraph`, `colors`, `sizes`), `tests/text_corpus.rs` |
| Apple Color Emoji (`sbix`), drawn by CoreGraphics and placed as Canvas2D places it | Drawn, pixel-identical | `emoji` group (6 to 160 px) |
| Caption boxes, 4× multisampled as the GPU canvas fills paths | Drawn (3 to 9 pixels per case still differ at the corners) | `caption-boxed`, `caption-paper` |
| RTL and complex scripts the level 1 layout shapes (Arabic, Hebrew, Devanagari, Thai, CJK) | Drawn | `scripts` group |
| `compat` level 2 text and captions, at output resolution | Drawn when the host gives level 2 fonts; still routed by the static check (below) | `tests/text_level2.rs` |
| Text and caption colors: hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, `hwb()`, named colors, `transparent`; rejected strings keep the style `save()` restored, as the canvas does | Drawn | `pm_text::render::color` tests (values checked against Chromium's `fillStyle`), `tests/text_colors.rs` |
| Valid CSS colors the engine does not reproduce (`lab()`, `lch()`, `oklab()`, `oklch()`, `color()`, `color-mix()`, relative colors, `calc()`, `currentcolor`, system colors, any functional notation that does not parse) | Refused (`Feature::Text`), not drawn with a fallback | `tests/text_colors.rs` |
| Color vector glyphs (`COLR`) | Refused (`Feature::Text`), not drawn wrong | `scene::tests::color_vector_glyphs_are_refused_not_drawn` |
| Text and captions without configured fonts | Refused (`Feature::Text`). The interactive engine, the job engine, and `pm-cli` give their renderers fonts (`docs/engine/text.md`, "Fonts on render threads") | `tests/text_gpu.rs`, `pm-node/tests/fonts.rs` |
| A registry question the host's font service left unanswered | Refused (`Feature::Text`, "the font registry did not answer"), never drawn in another font | `pm_text::served` tests |

Text and captions have no shadow property today (drop shadow is an effect).

## How a layer is drawn

`plan.rs` dispatches `text` and `captions` to `crate::text` (`text_quad`).

**Level 1** reproduces today's compositor: a canvas raster of the layer at
the density `continuousRasterScale(scaledWorld)` asks for (the layer's
largest singular value in quarter steps, 0.25 to 32; ported exactly in
`continuous_raster_scale`), sampled bilinearly through the layer's quad with
hard UV edges. Canvas sizes and densities follow `textRasterGeometry`
(`min(scale, 8192 / w, 8192 / h)`), `rasterAnimatedText`, and the captions
rasterizer's `draw` (`max(0.05, min(scale, 8192 / w, 8192 / h))`). The canvas
is a `TextScene` of ordered draws in device pixels:

- `rasterText`: one `fillText` per wrapped line (`Compat::line_runs`), at
  `(g.x, pad + lh × i + size × 0.82)`, `fillStyle = d.color || '#fff'`.
- `rasterAnimatedText`: one `fillText(g.text, 0, 0)` per posed glyph
  (opacity above 0) through `density · translate(−x0, −y0) · translate(x,
  baseline) · glyph matrix`, with `globalAlpha`, `filter: blur(blur × density
  px)`, the range-styled font, and `fillStyle = g.color ?? g.style.color`.
  Each glyph draws between `save()` and `restore()`, so a color the setter
  rejects leaves the canvas default, black (never the previous glyph's
  color). `pm_eval`'s `animated_raster_glyphs` gives the
  glyphs with the range-font advance shift and the canvas box.
- Captions `draw`: the box (`roundedRect`, `hexToRgba(boxColor,
  boxOpacity)`), then per word `strokeText` (2 × stroke × size / style size,
  round joins, miter limit 2) and `fillText` (fill or highlight color), each
  word with its state's alpha and scale about the word's center. Each word
  draws between `save()` and `restore()`: a rejected fill or highlight color
  draws in the box fill (black without a box), and a `hexToRgba` string
  holding `NaN` (a short or non-hex code) is rejected the same way, so a box
  fills and a stroke strokes black.

A color string a layer's draws need is classified by
`pm_text::render::color::classify`: parsed to the 8-bit sRGB value the
canvas stores, rejected (the setter keeps the restored style), or
unsupported (a valid CSS color the parser does not reproduce). Unsupported
colors refuse the layer (`Feature::Text`) in `render` and in
`Renderer::text_raster`, which drains the refusal so it never lands on a
later layer.

Only the part of the canvas that holds ink is allocated (`TextScene::ink`,
whole canvas pixels); the quad shrinks to that crop, which samples
identically because the rest of the canvas is transparent.

**Level 2** (`compat ≥ 2`) draws without a raster: level 2 layout (Parley,
`TextFonts::level2`) through the layer's world matrix (size anchor
included) straight into output pixels, with the CoreText profile and exact
positions (`Quad::Device`). Level 2 captions use the same caption frame at
output resolution. Animated or range-styled text and typography anchoring
at level 2 are refused.

## Rendering

`pm_text::render`:

- `outline`: Skrifa outlines at device size and variation location (user
  coordinates for level 1, normalized for level 2), Skia's synthetic italic
  (`skewX = −1/4`) or Parley's oblique, synthesized bold, and the profile's
  expansion (`kurbo::expand_path`, round joins, zero-area contours dropped).
  Where expansion misbehaves (cusps in Marker Felt produced points 30 px
  away) the glyph is drawn as fill plus a round-joined stroke of the
  expansion width: the exact dilation. Cached per face, glyph, size,
  location, and style.
- `bitmap`: color bitmap glyphs (below), decoded, premultiplied, resampled.
- `scene`: `Painter::fill_text`, `stroke_text`, `fill_path`; items are
  *coverage* (paths and a coverage curve) or *images*, each with a color,
  `globalAlpha`, and blur.
- `atlas`: coverage cells 1:1 with device pixels at integer offsets, packed
  in pages of at most 4096 px; larger items are tiled with clips.
- `cpu`, `paint`: `vello_cpu` coverage pages and the float paint pass on the
  CPU (the reference for the GPU pass and the fitting tool's renderer).
- `coretext` (macOS, feature `coretext-reference`): the CoreText reference
  rasterizer for the level 2 gate and the review sheets.

`pm_render::text::gpu::TextGpu`:

- **Coverage:** Vello 0.11 (`AaConfig::Area`) renders every coverage cell in
  white into `Rgba8Unorm` pages; alpha is coverage. Vello submits its own
  command buffers, so pages are pool targets returned only after the frame
  is submitted (a later layer never overwrites a page an earlier paint pass
  still reads). If Vello cannot start, the same pages come from `vello_cpu`
  (`Coverage::Cpu` forces it).
- **Float paint** (`shaders/text.wgsl`): items composite in order with
  premultiplied source-over at working precision (`rgba16float` or
  `rgba32float`). Coverage items paint `color × curve(coverage)` (the curve
  is 17 points, linear between them); image items sample a padded
  premultiplied image bilinearly. A blurred item is painted alone into a
  layer of its reach, blurred by a separable Gaussian (radius 3σ), and
  composited.
- **Raster cache:** painted rasters are kept by content key (a 128-bit hash
  of the scene, its crop, and the font generation) up to 192 MB, so static
  text is not repainted every frame.

## Profiles (amendment 10)

`pm_text::render::profile::Profile` holds, per device font size (linear
between points), horizontal and vertical outline expansion, a vertical ink
offset, and an exponent applied after the coverage curve; coverage curves
for white and for black paint mixed by `luma^mix`; positioning (Skia's
quarter-pixel x and rounded y, or exact); the size above which glyphs draw as
plain paths; a large-text regime; the stroke pass; synthesized bold.

### Canvas2D (level 1)

Measured, not assumed:

- **Coverage curve** (`CANVAS2D_LIGHT`): the bottom edge of Menlo's full
  block, which CoreGraphics smoothing does not move, at 8 to 40 px in 1/4 px
  steps (`text-probe`): plain coverage 1/4, 1/2, and 3/4 become 0.39, 0.71,
  and 0.93 for white paint, the same at every size. Black paint is almost
  linear (0.22, 0.46, 0.72; `CANVAS2D_DARK`); paint in between mixes by luma
  (Skia's luminance-dependent mask "preblend").
- **Dilation:** the block's side edges grow by about 0.005 × size; its top
  edge grows upward (0.09 px at 8 px to 0.47 px at 40 px) and its bottom edge
  not at all, so the profile expands vertically and moves the ink up by the
  same amount. The per-size values (`CANVAS2D_POINTS`) were fitted on
  `text-fit` (10 families × 2 weights × 24 sizes, white) by coordinate search
  of mean ink SSIM, then smoothed; the expansion saturates at 0.305 px from
  64 px.
- **Paths:** text at or above 256 device px (`SubRunContainer` draws masks
  and distance fields only below `maxMaskSize`, 256), or a glyph whose
  plain bounds plus one pixel exceed 256 px in either dimension, draws as a
  plain path (Menlo's 1.03 em full block switches at 250 px, its H at 260
  px).
- **Large text:** from 162 px, glyphs sit at exact positions (no y rounding;
  the H at 170 px sits 0.4 px below a rounded baseline) with expansion
  0.425 / 0.325 px, offset −0.303 px, exponent 0.962, and the linear curve
  (mean SSIM 0.991 on the H at 170 to 250 px).
- **Strokes** (caption `strokeText`): Skia strokes the path itself; fitted on
  `fit-stroke` (7 sizes × 6 widths): width −0.05 px, exponent 0.612 (mean
  0.992).
- **Synthesized bold:** an outset of 0.00875 × size per side (vertical 0.925
  of horizontal), fitted on the `synthetic` cases; Skia's 1/24 to 1/32
  interpolation was 30% too heavy at 14 px.

### CoreText (level 2)

S4's CoreGraphics fit (expansion `min(0.0175 × size, 0.34)`, the ink up by
the same, exact positions) plus a dark-paint curve `1 − (1 − c)^0.55` mixed by
`luma^1.4`, measured against CoreText on the `colors` cases.

## Color bitmap glyphs (emoji)

Canvas2D placement is the rule for both levels (amendment 10). Measured on
`fit-emoji` (😀 and 🇰🇷 at every size from 6 to 64 px):

- CoreText draws Apple Color Emoji at the face's full-em advance for the
  requested size (`FontResolver::em_advance`: 8 px at 6 px, 15 at 12, 20 at
  16, 22 at 20, the size itself from 24 px), not at the requested size.
- The downward shift is whole pixels: `size / 8` rounded with halves down
  from 20 px; below, the measured 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4 for
  6 to 19 px (`apple_emoji_shift`). With these, every pixel matches at strike
  sizes (SSIM 1.000 at 16, 24, 32, 40, 48, 52 px).
- The strike image is downscaled with Lanczos-2, the closest of area,
  linear, Mitchell, Lanczos-2, and Lanczos-3: SSIM ≥ 0.98 from 10 px; 0.92 to
  0.96 at 6 to 9 px.

## Glyph masks (level 1, macOS)

Measured on 2026-10-18 and now the level 1 path: Canvas2D's glyph masks are
CoreGraphics' own coverage, mapped through a per-color table. pm-text draws
the same masks (`render::cgmask`, feature `cg-masks`, on by default in
pm-render on macOS):

- **Masks.** Each glyph is drawn by CoreGraphics (`CTFontDrawGlyphs`) into a
  32-bit RGB bitmap, black on white, antialiased, with subpixel positioning
  on and quantization off, at the glyph's quarter-pixel offset (Skia's
  subpixel positioning: x to the nearest quarter, y rounded), and the mask
  is `255 − luminance`. The CTFont is the face's own bytes at the device
  size with its variation coordinates (`opsz` included). Placed 1:1 at the
  whole device pixel, the masks equal Canvas2D's: on every capture case
  the map from our raw coverage to Canvas2D's is a function (each raw value
  gives one value), so geometry, smoothing, and positions match exactly.
  An alpha-only context gives the same coverage (smoothing has no effect on
  macOS 27).
- **Tables.** That function depends on the paint color through four tables
  (`render::mask_tables`, measured: every coverage value 1 to 255 seen on
  the fit capture's 10 colors, no ambiguous entry, monotonic). The table is
  picked by Skia's Mac luminance rule (`luminance_bucket`): the paint color
  canonicalized (top 3 bits replicated per channel), halved (the Mac scaler
  for smoothed text without subpixel coverage), canonicalized again,
  reduced to one luminance (`(54r + 183g + 19b) >> 8`), canonicalized; its
  top bits are the bucket, 0 to 3 (halving leaves no higher bucket).
  Verified on 14 colors (black, primaries, grays, yellow, cyan, the
  captions' gold and red). Skia's `SkMaskGamma` correcting LUT does not fit
  the tables for any gamma and contrast tried, so they are kept as data and
  must be measured again if an Electron update changes them
  (`tests/text_raster.rs` fails then).
- **Compositing.** A `fillText` call's masks become one premultiplied image
  item: each glyph blended source-over into an 8-bit target with the
  paint's color and alpha (`globalAlpha` included), quantized after every
  glyph, as the GPU canvas blends glyph quads one by one. Overlapping
  glyphs of a translucent paint build up (the whisper preset, `#00000099`
  strokes, `#FFFFFF80` text).
- **Glyph origins** are mapped as Skia maps a text blob: the canvas matrix
  in `f32`, pre-translated by the blob origin (`SkMatrix::preTranslate`),
  then each glyph position, all in `f32` (`Placement::glyph_origin`).
- **Color glyphs.** Color bitmap faces (`sbix`, `CBDT`; not monochrome
  `EBDT`, which draw as masks) are drawn by CoreGraphics into a
  premultiplied RGBA bitmap at the em size CoreText uses for the strike
  (`FontResolver::em_advance`), then moved down by the measured whole-pixel
  shift for Apple Color Emoji (`apple_emoji_shift`): every emoji case from
  6 to 160 px is pixel-identical.
- **Synthesized italic** is Skia's text-matrix shear, drawn by CoreGraphics
  about the glyph origin: exact.
- **Glyph frames: synthesized bold and `strokeText`** (2026-10-18). Skia
  draws a glyph image from its path when the glyph has a frame
  (`SkScalerContext::internalGetPath`, `GenerateImageFromPath`): the
  CoreGraphics outline at the device size (4× in x, then ¼, plus the
  quarter offset), mapped back to user space, framed with `SkStroke`
  (fake bold: stroke-and-fill of `size × interp(size, {9, 36}, {1/24,
  1/32})`, miter; captions: `lineWidth`, round joins, miter limit 2),
  mapped to device space, filled 4× horizontally into A8 and packed
  (`pack4xHToMask`, the mask is generated "from LCD"), then Skia's mask
  gamma for the paint's luminance bucket (`skpath::skia_mask_gamma`; the
  measured tables are this gamma over CoreGraphics' linearized coverage,
  checked for every entry). The fill is a port of Skia m149's analytic
  rasterizer (`render::skaaa`: `SkScan_AAAPath`, analytic edges, the
  edge builder and sort, in its 16.16 fixed point). The stroker is
  `pm-skia-path` (tiny-skia-path 0.12 with conics kept, as `SkPath` keeps
  them): round joins and caps stay conics through the device transform and
  the 4× sampling and are cut into quads only at edge building
  (`SkAutoConicToQuads`, ¼ px, Skia's own subdivision), which is what made
  every caption outline exact. A synthesized bold run under `strokeText`
  strokes with the two widths added. A color bitmap glyph has no outline
  to frame: Skia draws its image in the stroke pass as well, so the fill
  draws it over itself (its antialiased rim thickens); the engine does the
  same.
- **Strike scale.** Skia's strike descriptor keeps the device matrix to
  1/1024 (`sk_relax`), so a scaled word (caption highlight and pop, the pop
  and pulse animators) has its masks generated at `size × relax(scale)`
  (`Placement::strike_scale`); positions keep the exact matrix.
- **Rotated, skewed and non-uniformly scaled glyphs** are masks too: the
  Mac scaler's `computeMatrices(kVertical)` of the strike (the relaxed
  device 2×2, a Givens rotation, the vertical scale as the CTFont size,
  the rest as CoreGraphics' text matrix), bounds from
  `CTFontGetBoundingRectsForGlyphs` through it, quarter-pixel positions in
  x and y (`cgmask::StrikeMatrix`, `draw_transformed`).
- **Caption boxes** are `roundedRect`'s path (`arcTo` corners, quarter
  conics), tessellated as Graphite tessellates a conic (`T = k / 2^L`, `L`
  from Wang's formula at precision 4 in device space, De Casteljau in
  local space), vertices snapped to the rasterizer's 1/256 px, then the
  standard 4-sample pattern with nonzero winding.
- **Glyphs beyond the atlas** (over 256 px, or a bounding box over 256 px)
  are drawn as Skia draws text as paths: the CoreText outline at the
  canonical 64 px (`MaskSource::canonical_path`), under the glyph's matrix
  (the canvas matrix, the glyph position, `size / 64`), each quad as its
  cubic, cubics cut at `T = k / 2^L` (Wang's formula), vertices snapped to
  1/256 px, 4 samples per pixel (`scene::tessellated_coverage`).

- **Paths at exactly the sample positions:** multisampled path coverage
  counts a sample that lies exactly on an edge by the rasterizer's
  top-left rule (a crossing at the sample's x counts). Text at 256 px puts
  the canonical 64 px outline at a scale of exactly 4, so vertices land on
  sample positions; with the strict comparison 1,491 pixels were a sample
  off.

The outline path with the Canvas2D profile (below) remains only for
platforms without CoreGraphics.

## Distance-field text (162 to 256 px, level 1)

2026-10-18 (third pass). Chromium's GPU canvas draws text whose device
size (`SkFontPriv::ApproximateTransformedTextSize`: the text size times the
matrix's largest scale) is at least 162 and below 256 px as distance-field
text (`SubRunControl::isSDFT`, `getSDFFont` on macOS). pm-text does the
same (`render::sdf`, `MaskSource::sdf_glyph`):

- **Strike.** One strike per glyph at 256 px (162 px for text at exactly
  162), the text matrix the identity: CoreGraphics draws the glyph with
  smoothing on (the strike's `kNormal` hinting) into Skia's glyph bounds
  (the CoreText bounding box rounded out, one pixel more each side), the
  pixels are linearized (`gLinearCoverageFromCGLCDValue`, `(v² + 128) /
  255`) and reduced to luminance without the paint's preblend (the SDF mask
  filter ignores it). Synthesized bold and wide strokes generate the A8
  from the glyph's path framed in strike space instead
  (`skpath::frame_a8`: fake bold at the strike's size, the stroke width
  divided by the strike-to-source scale; plain A8, no LCD).
- **Field.** `SkGenerateDistanceFieldFromA8Image` ported line for line
  (`sdf::distance_field`: Gustavson's edge distances, Danielsson's 8SSEDT
  with Skia's pointer walk, `pack_distance_field_val<4>`), padded by 4.
- **Placement.** `prepare_for_SDFT_drawing`: the glyph position mapped by
  the creation matrix (`1 / strikeToSourceScale`, in `f32`), the field's
  bounds inset by 2; the quad drawn through the canvas matrix
  pre-translated by the blob origin and scaled by the creation matrix's
  inverse (Graphite's `VertexFiller`). A field over 256 texels is drawn as
  a direct mask when the mask fits (the SDFT run's rejects go to the direct
  mask case), else as a path.
- **Fragment.** Emulated per pixel (`sdf::draw`): vertices snapped to 1/256
  px, texture coordinates interpolated over the strip's triangles,
  bilinear filtering with 8-bit weights rounded to nearest, `dist =
  7.96875 (texel − 128/255)` in half precision (the one half operation the
  captures show rounding), derivatives over 2×2 quads, `afw = 0.65
  |J·n|`, `smoothstep(−afw, afw, dist)` (no gamma adjustment on Mac),
  stored to 8 bits and blended glyph by glyph like the masks. Fitted on the
  three main-capture cases: snapping to 1/128 or 1/512 px, truncated, 9-bit
  or exact weights, and the whole fragment in half each leave pixels 2 code
  values off; this model leaves none (and 4,400 to 2,300 pixels 1 off per
  case, within the gate).

## Text colors (CSS Color 4 and 5)

`render::color4` (third pass) ports Blink's color model so a text color is
the `SkColor4f` Chromium paints with: `lab()`, `lch()`, `oklab()`,
`oklch()`, `color()` in every predefined space (sRGB, linear sRGB, Display
P3, A98, ProPhoto, Rec. 2020, XYZ D50/D65), `color-mix()` (missing and
analogous components, premultiplied interpolation, hue methods, percentage
normalization), relative colors (`rgb(from …)` with channel keywords and
`calc()`, `min()`, `max()`, `clamp()`), `none`, `currentcolor` (black: the
raster canvas is never in the document) and the system colors measured on
the closure capture (`CanvasText`, `Canvas`, `LinkText`, `GrayText`;
`AccentColor` is not a color to the canvas and keeps the previous style).
Conversions follow `ui/gfx/color_conversions.cc` and skcms (its own `powf`
approximation where `Color::ToSRGB` uses it for linear sRGB); gamut mapping
is not baked in (`kBakedGamutMapping` off, as the capture's out-of-gamut
`oklch()` shows). Colors outside sRGB stay unclamped through the glyph
blend; the 8-bit target clamps them. All 36 colors of the closure capture
match within 1 code value. `light-dark()` and system colors not measured
still refuse their layer.

## Routing

`pm_render::unsupported` and `unsupported_with_media` describe a renderer
without fonts and list text and captions layers (`Feature::Text`).
`unsupported_with_fonts` and `Renderer::unsupported_with_media` describe a
renderer with fonts (`Renderer::set_text_fonts`), where they are drawn. The
viewer presenter (`ViewDocument::set_fonts`), the job engine
(`StillRouting::with_fonts`, `plan_export_with_fonts`), and `pm-cli` use the
check with fonts when their renderers have them, so text and captions no
longer route there.
`compat` level 2 content stays routed (`Feature::CompatLevel2`) until level 2
layer semantics (bounds, picks, anchors) exist in `pm-eval`; the renderer
draws it when asked. `Renderer::set_captions_hidden` is `PM.captionsHidden`
(exports with `captionsBurn: false`).

## Measurements

**2026-10-18 (third pass: distance-field text, CSS Color 4, the closure
capture)**, Apple M4 Max, macOS 27, Electron 44 / Chromium 152, addon built
at the pass's head without LTO (see Gaps).

`tests/text_raster.rs` gates two captures now:

| Capture | Cases | Exact (within 1 code value) | Pinned | Refused |
|---|---:|---:|---:|---:|
| main (`text-raster-20261016-061928`) | 495 | 462 (second pass: 459) | 33 | 0 |
| closure (`text-closure-20261018-205749`) | 78 | 61 | 5 | 12 (`COLR`) |

- The three 162 to 256 px main cases are exact (were 31,975, 30,632 and
  16,169 pixels off, max Δ 148, 147, 126).
- Closure capture: distance-field text at 161, 162, 163, 180, 210, 255,
  256 and 257 px, five families, densities 2, 0.5 and 2.75, colors,
  scripts, tracking, a paragraph, synthetic italic: exact except
  `sdf-sizes-0004` (210 px: 4 pixels by 2), `sdf-synthetic-0016`
  (synthesized bold at 200 px: 16 pixels by up to 6), `sdf-scripts-0023`
  (Arabic at 200 px: 41 pixels by 2, one row where joined glyphs
  overlap), `sdf-caption-0027` (a 6 px stroke at 180 px: 12 pixels by up
  to 21) and `sdf-caption-0028` (`pop` at 160 px: 3 pixels by up to 5).
  All 36 CSS Color 4 and 5 colors are exact; the 12 `COLR` cases are
  refused (Gaps).
- `text_corpus`: 237 frames, 235 exact within 2 code values, 0 failed,
  0 not compared (unchanged; the two `animator-blur` frames stay pinned).
  The isolated-text half compares text without its effects and with a
  parent or matte source that is not kept removed (so at another
  transform): it proves the text, not the user's frame.
- `text_level2`: 400 cases and 20 layers, 0 below their gate. `text_gpu`:
  GPU paint pass vs CPU reference max |Δ| 0. `text_colors`: pass (CSS Color
  4 colors draw; `light-dark()` and unmeasured system colors refuse).
- E2E with `POWERMOVE_NEW_ENGINE=1` under the lock, `--workers=1`, every
  test checking afterwards that the viewer routed to the engine with no
  refused layer, no presenter layer failure and no render error
  (`e2e/helpers/engine-drew.ts`): `canvas-text` 6/6, `captions` 2/2
  (subtitles import, edit, burn into frames, export), `font-anchor` 1/1,
  `split-text` 4/4, `text-animators` 1/1, `text-layout-resize` 2/2. The
  first run failed `font-anchor` because the check judged a session still
  `pending` after the test replaced the project; the check now waits for
  the session to settle, and the rerun passed.
- Canvas text typing in the app with the engine drawing
  (`native-engine-text-typing.spec.ts`, 60 keys each, 0 missed, route
  `engine`, 0 layer failures): title 96 px p95 17.4 ms (p50 11.8, max
  18.1); paragraph 48 px in a 1,400 px box p95 15.7 ms; `system-ui` 24 px
  p95 13.2 ms. Target ≤ 33 ms.

**2026-10-18 (second pass: glyph frames, blending, exactness gates)**, Apple
M4 Max, macOS 27, references from Electron 44 / Chromium 152.

`tests/text_raster.rs` now gates on exactness (every channel of every
pixel within 1 code value), with the remaining cases pinned
(`tests/fixtures/text-raster-pins.json`: the most pixels beyond 1 code
value and the largest difference; pins only shrink):

| Group | Cases | Exact | Pinned: most pixels off / max Δ |
|---|---:|---:|---|
| families, weights, italic, scripts, tracking, paragraph, emoji | 302 | 302 | |
| colors | 21 | 21 | (`#FFFFFF80` now exact) |
| synthetic (italic and bold) | 6 | 4 | bold: 1 and 7 pixels by 2 (was ink SSIM 0.9288, max Δ 220) |
| variable | 27 | 21 | 6 to 448 pixels by 2 to 6 |
| sizes, sizes-regular | 48 | 46 | the two 200 px cases: 31,975 and 30,632 pixels, max Δ 148 |
| density | 24 | 23 | the 192 px case: 16,169 pixels, max Δ 126 |
| large (paths over 256 px) | 2 | 2 | (were 332 and 341 pixels, max Δ 128) |
| caption strokes | 26 | 24 | 2 and 7 pixels by 2 and 4 (were max Δ 98 to 170) |
| caption presets, word states, scripts | 39 | 16 | boxed: all exact; classic, whisper, paper, fade: 1 to 2 pixels by 2; spotlight: 2 pixels by 3; pop: 2 of 6 exact, the rest 2 to 75 pixels by 2 to 13 |
| **All** | **495** | **459** | 36 pinned (first pass: 414 exact) |

`tests/text_corpus.rs` now gates on exactness too (every text pixel within
2 code values; `tests/fixtures/text-corpus-pins.json`) and has no escapes:
the 35 projects refused for features outside text are compared through
their text in isolation (`text-iso-20261018-175328-e9f22842`). 237 frames,
235 exact, 0 failed, 0 not compared. All 111 isolated frames are exact
(effects on text: blur, glow, shadow, color, levels and the rest; 2.5D and
motion-blurred text drawn flat and sharp). Every caption, scaled (`pop`,
`pulse`) and rotated (`tumble`) animator frame is exact; pinned are only
two `animator-blur` frames (1 and 5 pixels by 3). `captions/middle-lines`
(0.9892 before) is exact.

**2026-10-18, glyph masks and fonts on render threads** (Apple M4 Max,
macOS 27, references from Electron 44 / Chromium 152).

Level 1 rasters against Canvas2D (`tests/text_raster.rs`, 495 cases;
"exact" is every channel of every pixel within 1 code value):

| Group | Cases | Exact | Max Δ | Min ink SSIM | Before (min SSIM) |
|---|---:|---:|---:|---:|---:|
| families | 102 | 102 | 0 | 1.0000 | 0.94 to 1.0 |
| weights | 108 | 108 | 0 | 1.0000 | |
| italic (real faces) | 16 | 16 | 0 | 1.0000 | |
| scripts (RTL, complex, CJK) | 42 | 42 | 0 | 1.0000 | |
| tracking, paragraph | 14 | 14 | 0 | 1.0000 | |
| emoji (6 to 160 px) | 20 | 20 | 0 | 1.0000 | 0.92 to 0.965 |
| colors | 21 | 19 | 8 | 1.0000 | |
| sizes, sizes-regular (8 to 200 px) | 48 | 46 | 148 | 0.9835 | 0.871 (small text) |
| variable (`fontAxis.*`) | 27 | 21 | 6 | 1.0000 | |
| density (0.25 to 4) | 24 | 23 | 126 | 0.9921 | |
| synthetic (italic, bold) | 6 | 3 (italic) | 220 | 0.9288 (bold) | 0.9288 |
| large (300 px device, paths) | 2 | 0 | 128 | 0.9978 (about 330 pixels off) | 0.9933 |
| captions (all presets, strokes) | 65 | 0 | 170 | 0.9881 | 0.985 |
| **All** | **495** | **414** | | | 0 exact |

Every case at or below 18 px device size is exact (the cases that are not
are the 200 px sizes and the 192 px density case, which draw in the
large-text regime). Colors and variable fonts differ by at most 8 and 6
code values on 2 to 448 pixels (premultiplied rounding at alpha 0x80;
CoreText's own variation instancing in Chromium's CTFont).

Whole frames (`tests/text_corpus.rs`, served fonts with the capture's
seeds, built-in definitions, text routing forbidden): 126 frames, 0 failed;
static text frames text-pixel SSIM 1.0000 with every other pixel exact;
captions 0.9892 to 0.9999 (strokes) and animator frames with rotated or
scaled glyphs 0.9978 to 0.9999 (outline path); projects not compared are
refused only for features outside text (SVG images, path masks, motion
blur, 2.5D, one extension effect with no definition).

Evaluation (`pm-cli parity eval --wasm`, corpus v1): native and pm-wasm
1,891,448 values compared, 0 mismatched, 0 tolerance questions (810 in the
variable-font ink class, max 1.191e-4 px); text sub-gate 6,162 layouts,
identical breaks, max advance error 0.000000 px.

Canvas text typing, end to end in the app with "Use New Engine" on and the
engine drawing the text (`e2e/native-engine-text-typing.spec.ts`, 60
keystrokes each; keydown to the first animation frame showing an engine
frame of a newer revision; 1920 × 1080; the window on a 2x display): a 96 px
title p50 11.6 ms, p95 16.7 ms; a 48 px paragraph in a 1400 px box p50 11.2
ms, p95 19.6 ms; 24 px `system-ui` p50 11.4 ms, p95 13.1 ms (the row's gate
is p95 ≤ 33 ms). Every keystroke routed to the engine, with no layer
failures or render errors; the hub served 3 to 4 registry round trips per
run.

E2E with "Use New Engine" on (`POWERMOVE_NEW_ENGINE=1`, `--workers=1`):
`canvas-text`, `captions` (import, edit, burn-in export, export back),
`font-anchor`, `split-text`, `text-animators`, `text-layout-resize`: 16 of
16 pass. `engine-fonts-integration.mjs` (real utility processes): both
engines get a font port, the job engine answers with the mirrored project
font, and each engine replays project fonts after a SIGKILL (about 0.33 s).

Earlier measurements (2026-10-16), before the glyph masks: Apple M4 Max,
macOS 27, Electron 44 (Chromium 152) for references, machine load 9 to 20. Captures (private, not in the
repository): `text-raster-20261016-061928` (495 cases),
`text-fit-20261016-062639` (700), `text-probe-20261016-065257` and
`text-probe-20261016-072821` (194), corpus `v1-20261012-230504`.

**Level 1 rasters against Canvas2D** (`tests/text_raster.rs`, ink SSIM):

| Cases | Count | ≥ 0.98 | Min | Mean |
|---|---:|---:|---:|---:|
| Text above 18 px device size | 240 | 237 | 0.929 | 0.995 |
| Captions (every preset and state; 26 stroke cases) | 65 | 65 | 0.985 | 0.993 |
| Text at 8 to 18 px | 190 | 113 | 0.871 | 0.979 |

The three failures above 18 px: synthesized bold at 36 and 80 px (0.951,
0.929) and mixed emoji at 20 px (0.965).

**Whole frames against the legacy renderer** (`tests/text_corpus.rs`): 91
frames of 24 corpus projects (every `text/*` project, including all eleven
animators, range styles, and variable axes, and every `captions/*` project)
all pass: text pixels SSIM min 0.989, every other pixel exact (max 0 CV).
Rerun 2026-10-17 after merging the wave 4 lanes: the corpus now holds 31
projects with text (the 24 above plus `animation/interpolation`,
`composition/fps-*`, `layers/null-parenting` and `retiming/precomp`), 123
frames, all pass, text SSIM min 0.989, every other pixel exact.

**Level 2 against CoreText with smoothing** (`tests/text_level2.rs`): 400
outline-text cases at their exact positions: 399 ≥ 0.98 (min 0.9799, black
at 48 px; group means 0.985 to 0.997). 20 level 2 layers rendered through the
renderer (4 families, scales 0.75 to 2, three alignments, black and colored
text): all ≥ 0.99 (min 0.9907).

**GPU path** (`tests/text_gpu.rs`): the paint pass against the CPU reference
over the same coverage, max |Δ| ≤ 1e-6 (blurred animator, emoji, stroked
caption with box); Vello against `vello_cpu` coverage, ink SSIM ≥ 0.9994;
deterministic; served from the cache.

**Colors** (`tests/text_colors.rs`, rerun 2026-10-16 after the fill-style
fix): `hsl()`, `hsla()`, `hwb()` scenes equal their hex scenes; rejected
colors draw the restored style in plain text, range styles, caption fill,
highlight, box and stroke; six unsupported colors refuse in `text_raster` and
`render` and leave nothing for the next layer. The color parser's values are
those Chromium's `fillStyle` reports (Electron 44, checked 2026-10-16). The
rerun of `text_raster`, `text_corpus`, `text_level2` and `text_gpu` after the
fix gave the numbers above unchanged.

The parser refuses every form it is not sure of rather than guess
(2026-10-17): `none` in the legacy comma syntax, `rgb()` channels mixing
numbers and percentages, split tokens such as `50 %`, and non-finite alpha
are `Unsupported`, so no such string draws as a parsed color or as the
fallback. Percentage `rgb()` channels round to 8 bits as number channels do
(`rgb(100%, 0%, 50%)` is `#ff0080`). Rerun after this change on the merged
branch: `text_colors` all cases as above; `text_raster` 495 cases, 80 pinned,
0 failed, captions min 0.985; `text_level2` 400 cases and 20 layers, 0 below
their gate (min 0.9799, layers min 0.9907); `text_gpu` max |Δ| ≤ 1e-6, ink
SSIM ≥ 0.9994; `text_corpus` 123 frames, 0 failed.

Every numeric component (channel, alpha, hue, percentage, in either branch)
is checked against the CSS `<number>` token grammar before it is parsed
(2026-10-18): an optional sign, digits with an optional fraction or a
fraction alone, an optional exponent. Rust's float parser also takes `5.`,
`inf`, `infinity` and `NaN`, so before this `rgb(0 0 0 / NaN%)` parsed to a
NaN alpha and `rgb(0 0 0 / inf%)` to opaque, where the canvas keeps the
previous style; both, and `rgb(5., 0, 0)`, `rgb(1e, 0, 0)` and values that
overflow to infinity, are now `Unsupported`. Valid spellings (`+1`, `.5`,
`2e0`, `5e-1`, `1.2e2deg`) parse to the same values as before. Rerun:
`pm_text` lib 50/50; `text_colors` all cases equal or refused as above;
`text_raster` 495 cases, 80 pinned, 0 failed, captions min 0.985;
`text_level2` 400 cases and 20 layers, 0 below their gate; `text_corpus`
123 frames, 0 failed, text SSIM min 0.9890, other pixels max 0.

**Typing** (`examples/text_typing.rs`, the engine's share: layout, scene,
coverage, paint, composite at 1920 × 1080, GPU finished; 71 keystrokes): a
96 px title p50 1.39 ms, p95 2.18 ms; a 48 px paragraph in a 1400 px box p50
1.85 ms, p95 3.16 ms; 36 px p50 1.26 ms, p95 1.75 ms. Rerun 2026-10-16 23:30
(load 9.6 to 11.6): p95 1.65, 3.49 and 1.79 ms; an independent rerun under
load measured p95 2.7, 4.2 and 2.2 ms, and a rerun on 2026-10-17 (load
about 11) p95 2.51, 4.34 and 2.66 ms, so expect ±1 ms with machine load. The viewer bridge and
the editor's own handling are not included; the section 25 row is end to
end (Phase 3a gate).

## Review sheets

2026-10-18 (third pass), regenerated at the pass's head:
`~/Library/Application Support/Powermove Engine Corpus/review/text-sidebyside-20261018c-main/index.html`
(the three distance-field cases this pass changed and every pinned case,
36 sheets) and `.../review/text-sidebyside-20261018c-closure/index.html`
(every closure case the engine draws, 66 sheets). Canvas2D top, engine
middle, differences beyond 1 code value ×4 bottom. They supersede
`text-sidebyside-20261018b`.

2026-10-18 (glyph masks): `~/Library/Application Support/Powermove Engine
Corpus/review/text-1792322940/index.html`, 420 sheets (every text case:
today's Canvas2D, engine level 1, CoreText with smoothing, engine level 2),
regenerated after the change; it supersedes the sheets below.


`cargo run --release -p pm-render --example text_review` writes one sheet per
text case (rows: today's Canvas2D, engine level 1, CoreText with smoothing,
engine level 2) and an `index.html` with the scores. Emoji cases are marked:
the engine places emoji as Canvas2D does at both levels (amendment 10), so
their level 2 vs CoreText score is low by design (row 4 compares with row 1,
not row 3). Sheets for Jude's text review (420 cases): `~/Library/Application Support/Powermove Engine
Corpus/review/text-1792218705/index.html` (regenerated 2026-10-16 with the
emoji notes; supersedes `text-1792139695`).

## Tools and reproduction

```sh
source engine/env.sh
# Canvas2D references (hidden Electron, never focused; private corpus)
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/render.ts            # text-raster-*
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/render.ts --set fit  # text-fit-*
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/render.ts --set probe
python3 engine/crates/pm-text/capture/colr-fonts.py "$HOME/Library/Application Support/Powermove Engine Corpus/fonts"   # needs fontTools
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/render.ts --set closure   # text-closure-*
cd engine
cargo test --release -p pm-render --test text_raster   # PM_TEXT_ONLY, PM_TEXT_DIFFS (+ PM_TEXT_DIFFS_PINNED), PM_TEXT_DUMP, PM_TEXT_PROFILE, PM_TEXT_COVERAGE=cpu, PM_TEXT_REPORT
cargo test --release -p pm-render --test text_corpus   # PM_ENGINE_CORPUS, PM_TEXT_ONLY, PM_TEXT_DIFFS
cargo test --release -p pm-render --test text_level2   # PM_TEXT_ONLY, PM_TEXT_DIFFS
cargo test --release -p pm-render --test text_gpu
cargo test --release -p pm-render --test text_colors
cargo run --release -p pm-render --example text_fit -- <capture> sizes|dark|stroke|bold|emoji|probe|large|mid|score [profile.json]
cargo run --release -p pm-render --example text_review
cargo run --release -p pm-render --example text_typing
```

The capture pages draw with today's code: `createCaptionRasterizer` from
`captions/render.ts` itself; `textGeometry`, `textRasterGeometry`, and
`rasterText` copied from `raster.ts` (private to its `install`); each canvas
is read back through a WebGL texture uploaded as `texFor` uploads it.

## Gaps

Each of these is a visible or measurable difference on level 1 content
today (amendment 22), pinned in the exactness gates so it cannot grow, or a
refusal; none is accepted, each is open work for an orchestrator decision.
Updated 2026-10-18 (third pass).

- **`COLR` color fonts** (project fonts only; no installed macOS font has a
  `COLR` table) refuse their layer. The closure capture shows Chromium does
  not draw them as CoreGraphics color images: drawn that way (in an em box
  or in CoreText's glyph bounds, at quarter-pixel or whole-pixel
  positions) they differ by ink SSIM 0.85 to 0.99, max Δ up to 232 (a layer
  outside the base glyph is kept, whole-pixel placement fits best). The
  likely path is Skia's Fontations typeface for `COLR` web fonts (its own
  layer bounds, paths filled by Skia's rasterizer, gradients by its
  shaders). References: `text-closure-20261018-205749`, cases `colr-0030`
  to `colr-0041` (`v0` layered and `v1` gradient fonts at 14 to 200 px).
- **Distance-field residuals** (closure capture): synthesized bold at 200
  px 16 pixels by up to 6; a 6 px stroke at 180 px 12 pixels by up to 21;
  Arabic at 200 px 41 pixels by 2 (overlapping joined glyphs); 210 px 4
  pixels by 2; `pop` at 160 px 3 pixels by up to 5.
- **Caption pop at density 1** (`caption-pop-0455`, `-0456`, `-0458`,
  `-0459`): 2 to 75 pixels by 2 to 13 in `text_raster`, all in the black
  stroke of the scaled word (the corpus's pop frames are exact).
- **Blurred animator glyphs** (`animator-blur`): 1 to 5 pixels by 3.
- **Small residuals**: synthesized bold 1 and 7 pixels by 2; variable fonts
  6 to 448 pixels by 2 to 6 (Chromium's CTFont carries variations
  differently); caption strokes 2 and 7 pixels by 2 and 4; classic,
  whisper, paper, spotlight and fade captions 1 to 2 pixels by 2 or 3
  (8-bit rounding between the box, stroke and fill items).
- **Colors**: `light-dark()` and the system colors not measured
  (`ButtonFace`, `Highlight`, ...) refuse their layer.
- **Level 2**: animated and range-styled text and typography anchoring are
  still refused, and `compat` level 2 content still routes
  (`Feature::CompatLevel2`); level 2 layer semantics (bounds, picks,
  anchors in `pm-eval`) are not built. No existing project holds level 2
  content (it is created only with the new engine), but the brief lists it.
- **`Feature::Text`** stays in `pm_render::unsupported`: it is the reason a
  renderer without fonts, an unsupported color, a `COLR` glyph, or an
  unanswered registry question refuses a layer. Hosts with fonts no longer
  emit it for ordinary text; the derouter lane retires the routing.
- An app web font loaded by the page is available to Canvas2D but not to
  the engine's registry: `Geist` (`onboarding/welcome.css`) is loaded only
  once the onboarding window's styles are in the document, so a text layer
  naming `Geist` draws in Geist after onboarding in that window and in the
  fallback otherwise; the engine always draws the fallback. Not seen in the
  corpus.
- The tables, the emoji shift, and the GPU model of distance-field text are
  data measured from one Electron (Chromium 152) on macOS 27 with default
  font smoothing on Apple silicon; an update that changes Skia's tables,
  CoreText's emoji placement or the GPU's rasterization (or a user-level
  `AppleFontSmoothing` default) needs them measured again;
  `tests/text_raster.rs` fails only when the references are captured
  again.
- Unattended render queue jobs (possibly after a relaunch, no window) and
  `pm-cli` measure with every open window's seeds (or the startup model).
- Release builds with LTO fail on this machine ("failed to get bitcode",
  probably kache-cached rlibs without embedded bitcode); every test and the
  addon were built with `CARGO_PROFILE_RELEASE_LTO=false`.

## Changes outside the lane

2026-10-18 (lane ne-fonts, third pass):

- `apps/desktop/src/preload/index.ts` (bridge's file, one line):
  `reportUiFonts` on `engineFonts` (`engine-fonts:ui-fonts`).
- `apps/desktop/src/renderer/src/legacy/bootstrap.ts` (one entry): the UI
  font census.
- `apps/desktop/src/main/engine/index.ts` (small): the census channel and
  the window list for the seeds.
- `apps/desktop/src/main/engine/agent-jobs.ts`: an export whose window's
  seeds the job engine never takes does not start.
- `engine/crates/pm-cli/tests/render.rs`: text renders with the host's
  fonts; exit 3 is tested with a project color setting from a later
  version.
- `apps/desktop/e2e/helpers/engine-drew.ts`: fails when the status API is
  missing; waits for the session to settle.

2026-10-18 (lane ne-fonts, second pass):

- `engine/crates/pm-skia-path/` (new crate): tiny-skia-path 0.12.0 with
  conics kept, BSD-3-Clause.
- `apps/desktop/src/main/engine/{native-engine,engine-process,ports,agent-jobs,index}.ts`
  (small): `connect` names the window (`owner`); agent jobs tell the job
  engine's font client the window of each session.
- `engine/crates/pm-node/src/{lib,node}.rs`, `present/render.rs`,
  `jobs/mod.rs` (small): `attach` takes the owner; views and job work set
  the seed scope.
- `engine/corpus/src/{capture,text-only}.ts`: `--text-only`.
- `apps/desktop/e2e/{canvas-text,captions,font-anchor,split-text,text-animators,text-layout-resize}.spec.ts`:
  an `afterEach` that, with the new engine on, checks the engine drew the
  text (`e2e/helpers/engine-drew.ts`).

2026-10-18 (lane ne-fonts):

- `engine/crates/pm-node/src/present/{render,document,mod}.rs`: the
  presenter's renderer gets the engine's fonts (`ThreadConfig::fonts`,
  `TextFontsHub`), and the view's capability check uses fonts
  (`ViewDocument::set_fonts`).
- `engine/crates/pm-node/src/jobs/mod.rs`: lanes' renderers get fonts
  (`JobEngineService::start_with_fonts`); stills and exports route with
  fonts.
- `engine/crates/pm-node/src/lib.rs`: the font hub, the service created
  with the host, both engines serving fonts, shutdown order.
- `engine/crates/pm-export/src/render.rs`: `plan_export_with_fonts`,
  `StillRouting::with_fonts`.
- `engine/crates/pm-cli/src/{fonts,render,scope,main}.rs`: fonts for
  `render` and `scope`, `--ui-scale`; `src/parity/*` and
  `scripts/parity-wasm.mjs`: pm-wasm gets the seeds, the variable-font ink
  tolerance class.
- `engine/crates/pm-wasm/src/{engine,abi}.rs`: `pm_fonts_seeds`.
- `apps/desktop/src/main/engine/index.ts` (small): the job engine's font
  client and the seeds tracker.
- `apps/desktop/src/preload/index.ts` (bridge's file, small): `seeds` and
  `onSeedsChanged` on `engineFonts`.
- `apps/desktop/src/renderer/src/engine-doc/{install,fonts,wasm}.ts`: the
  window's seeds into pm-wasm.

Earlier:

- `engine/crates/pm-eval/src/text_geom.rs`: `animated_raster_glyphs`
  exposes the glyphs and canvas box `rasterAnimatedText` paints (the raster
  placement is unchanged; the evaluation corpus still matches with 0
  mismatches).
- `engine/crates/pm-render/src/{plan,exec,renderer,capability,lib,parity}.rs`:
  the text source, dispatch, device quad, executor arm, renderer API,
  routing (`Feature::Text`, `unsupported_with_fonts`), and
  `parity::text_ssim`.
- `engine/crates/pm-render/tests/{headless,corpus}.rs`,
  `tests/support/mod.rs`: the capability assertion for text, and text
  projects left to the text class.
- `engine/crates/pm-node/src/present/document.rs`: the `Feature::Text` arm
  of `describe`.
- `engine/crates/pm-text/capture/build.ts`: one entry for the rendering
  capture page.
- `engine/Cargo.lock`: Vello, `vello_cpu` and their dependencies.
- `docs/engine/pm-render.md`: one table row.
