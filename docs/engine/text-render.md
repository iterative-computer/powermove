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
| `compat` level 2 text and captions, at output resolution | Simple content draws with level 2 fonts; animated/ranged/typography semantics remain assigned to textl2 (below) | `tests/text_level2.rs` |
| Text and caption colors: hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, `hwb()`, named colors, `transparent`; rejected strings keep the style `save()` restored, as the canvas does | Drawn | `pm_text::render::color` tests (values checked against Chromium's `fillStyle`), `tests/text_colors.rs` |
| CSS Color 4/5 (`lab`, `lch`, `oklab`, `oklch`, `color`, `color-mix`, relative colors, `calc`, `currentcolor`) and detached-canvas system colors | Drawn; invalid `light-dark()` retains the restored paint, as today | `tests/text_colors.rs`, closure capture (74 colors) |
| Color vector glyphs (`COLR` v0 and v1) | Fontations paint graph rasterized natively; 12 captured glyph runs draw, two still fail exactness | `render/colr.rs`, `tests/text_raster.rs` |
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
  each glyph blended source-over into the 8-bit target with that
  unquantized coverage, quantized after every glyph. Fitted on the
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
Round 4 also captures the remaining detached-canvas system colors.
Conversions follow `ui/gfx/color_conversions.cc` and skcms (its own `powf`
approximation where `Color::ToSRGB` uses it for linear sRGB); gamut mapping
is not baked in (`kBakedGamutMapping` off, as the capture's out-of-gamut
`oklch()` shows). Colors outside sRGB stay unclamped through the glyph
blend; the 8-bit target clamps them. The round 4 capture has 74 color
cases, including all the previously unmeasured system names. The detached
canvas rejects `light-dark()` in both native light and dark themes, so it
keeps the restored paint. Highlight is `rgba(128, 188, 254, 0.6)` in both
themes; it is not opaque.

## Routing

`pm_render::unsupported` and `unsupported_with_media` describe a renderer
without fonts and list text and captions layers (`Feature::Text`).
`unsupported_with_fonts` and `Renderer::unsupported_with_media` describe a
renderer with fonts (`Renderer::set_text_fonts`), where they are drawn. The
viewer presenter (`ViewDocument::set_fonts`), the job engine
(`StillRouting::with_fonts`, `plan_export_with_fonts`), and `pm-cli` use the
check with fonts when their renderers have them, so text and captions no
longer route there.
After the compclose integration merge, simple level 2 text can draw, but
animated/range-styled and typography-anchored level 2 content remains
refused (a `Feature::Text` compat-level-2 reason). Semantic closure belongs
to textl2. This lane does not implement level 2 bounds, picks or anchors.
`Renderer::set_captions_hidden` is `PM.captionsHidden`
(exports with `captionsBurn: false`).

## Round 4 verification

2026-10-18/19, Apple M4 Max, macOS 27, Electron 44 / Chromium 152.
The gates no longer load any of the three pin files. Raster failures mean
pixels outside 1 CV; composed text failures mean pixels outside 2 CV.
All refusal pins were removed too.

After merging integration at `e9a80939` (merge `d42913bf`), the rebuilt
addons speak unchanged protocol `protocol-c137021c07039329`. Debug and
release raster runs agree:
main 495 cases, 468 within the bound and 27 failing; closure 116 cases,
110 within the bound and 6 failing; zero refused. The 27 variable-font
cases are now pixel-identical (maximum channel difference 0). CoreText
instancing now follows `SkTypeface_mac_ct`: non-hidden variable fonts are
created from a descriptor at size 0 with every axis's default or clamped
value, then copied at the device size with `NSCTFontOpticalSizeAttribute`
and `NSCTFontUnscaledTrackingAttribute`. Creating the varied CGFont first
was a different CoreText path. Hidden system faces retain their direct
CGFont path, avoiding descriptor fallback to Times.

The closure set includes all 74 captured color cases (all within 1 CV),
12 COLR runs (10 within 1 CV, 2 failing), and the independent distance-field
cases (4 failing). COLR uses Fontations' paint graph, font-unit paths,
CPAL colors, layer bounds, transforms/clips and premultiplied compositing.
No CoreGraphics color-font approximation is used.

The real project's 45 text-in-isolation samples are now included. Before
the merge, the full release corpus compared 412 frames, 405 within 2 CV,
7 failing, with zero projects left unpaired with a comparison. The full
merged release run compares 477 frames: 467 within 2 CV, 10 failing, zero
not compared. It now draws the original real project too (45 samples,
42 within 2 CV). Its failures at 16/18/24 seconds are 2/727/1,550 pixels
beyond 2 CV, maximum Δ 38/33/49; the corresponding isolated failures are
3/924/1,550 pixels, maximum Δ 44/43/49. The extra three failures are the
original real frames, not new isolation failures. The pre-merge full debug
run completes all 412 frames with the same seven failures; merged debug
verification also reruns the animator-blur project. The failures
were the two animator-blur times, each also present in the isolation set,
and real-project times 16, 18 and 24 seconds. This corrects the previous
claim, which omitted the entire `real/` tree. Isolation validates the
reduced text project, not all effects or non-text content of the original.

The full app run with the rebuilt addons after the merge passed 20 of 21
tests: the 19 existing text/export/typing cases and the new census playback benchmark passed;
the new census-derived seed comparison failed and remains a blocker.
Typing p95 was 18.4 ms (96 px title), 21.1 ms (48 px paragraph), and
20.1 ms (24 px system-ui), 60 keys each, zero missed. These meet 33 ms;
a legacy typing baseline was not measured in this round.

The playback benchmark uses a 1,000-layer timeline for ten seconds in
each engine. Legacy: 1,186 animation-frame intervals, p50 8.3 ms, p95
9.5 ms, one interval above 1.5× median. New engine: 1,201 intervals,
p50 8.3 ms, p95 9.3 ms, zero above 1.5× median. The census added 299
scans visiting 301 nodes in 15.4 ms total (about 0.0515 ms per scan),
with no new reports during playback; maximum scan 1.3 ms includes startup.
The scan timer excludes mutation-handler overhead; the frame timings
include it. This run shows no added long frames, not a universal guarantee
under every extension or density.

The font-service, pm-text, pm-cli ink and pm-wasm seed tests pass in debug
and release. Static startup-seed eval parity still reports zero mismatches
across 6,162 text layouts / 74,032 units and 1,815,420 native/wasm replies;
it does not establish that the live census is correct. The variable-font
bounds class remains 405 bounds plus 405 selections, maximum 1.191e-4 px.
Touched TypeScript tests: 101 passed; svelte-check: zero errors and one
pre-existing warning. Release tests and addons use LTO disabled (Gaps).

## Earlier measurements (rounds 1–3)

**2026-10-18 (third pass: distance-field text, CSS Color 4, the closure
capture)**, Apple M4 Max, macOS 27, Electron 44 / Chromium 152, addon built
at the pass's head without LTO (see Gaps).

`tests/text_raster.rs` gates two captures now:

| Capture | Cases | Exact (within 1 code value) | Pinned | Refused |
|---|---:|---:|---:|---:|
| main (`text-raster-20261016-061928`) | 495 | 462 (second pass: 459) | 33 | 0 |
| closure (`text-closure-20261018-205749`) | 78 | 62 | 4 | 12 (`COLR`) |

- The three 162 to 256 px main cases are exact (were 31,975, 30,632 and
  16,169 pixels off, max Δ 148, 147, 126).
- Closure capture: distance-field text at 161, 162, 163, 180, 210, 255,
  256 and 257 px, five families, densities 2, 0.5 and 2.75, colors,
  scripts, tracking, a paragraph, synthetic italic: exact except
  `sdf-sizes-0004` (210 px: 4 pixels by 2), `sdf-synthetic-0016`
  (synthesized bold at 200 px: 16 pixels by up to 6), `sdf-caption-0027` (a 6 px stroke at 180 px: 12 pixels by up
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
- E2E with `POWERMOVE_NEW_ENGINE=1` under the lock, `--workers=1`, addon
  and app built at `3e679840`, every test checking afterwards that the
  viewer routed to the engine with no refused layer, no presenter layer
  failure and no render error (`e2e/helpers/engine-drew.ts`): 19 of 19
  passed: `canvas-text` 6, `captions` 2 (subtitles import, edit, burn into
  frames, export), `font-anchor` 1, `split-text` 4, `text-animators` 1,
  `text-layout-resize` 2, and the three typing measurements. An earlier
  run failed `font-anchor` because the check judged a session still
  `pending` after the test replaced the project; the check now waits for
  the session to settle.
- Canvas text typing in the app with the engine drawing
  (`native-engine-text-typing.spec.ts`, 60 keys each, 0 missed, route
  `engine`, 0 layer failures, 0 render errors): title 96 px p95 18.5 ms
  (p50 11.7, max 19.5); paragraph 48 px in a 1,400 px box p95 21.6 ms (p50
  18.8, max 23.8); `system-ui` 24 px p95 18.1 ms (p50 11.6, max 24.2).
  Target ≤ 33 ms. (A run an hour earlier, same code but the
  distance-field blend: 18.8, 19.8, 12.9 ms.) Load average during the
  run: 6.2 to 6.7 (other lanes' work on the machine).

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
2 code values; the now-deleted `tests/fixtures/text-corpus-pins.json`).
This historical run omitted `real/`; it did not establish zero escapes:
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

Round 4: `~/Library/Application Support/Powermove Engine Corpus/review/text-round4-20261018/index.html`
contains 147 sheets: all 27 variable cases, all 12 COLR cases, all 74 colors,
every remaining raster failure, and the three earlier distance-field fixes.
Seven contact sheets, the real-project sheets and the three live-seed
sheets were visually inspected by the implementation worker; this is not Jude's acceptance. The three
real-project failures are in
`.../review/text-round4-real-20261018/index.html` (16, 18 and 24 seconds).
Live-seed diagnostic sheets are in
`.../review/text-round4-seeds-20261018/index.html`: 30 px / weight 600
(4,917 pixels beyond 1 CV; width 462 vs 422), 30 px / weight 500 (4,751;
453 vs 413), and 20 px / weight 400 (2,136; 295 vs 289), all maximum Δ 255.
These use the actual window's reported seeds and a captured page canvas;
they are visible layout failures, not AA residuals. The final merged corpus
has eight additional sheets in
`.../review/text-round4-final-corpus-20261018/index.html`: all three
original real frames, their three isolated counterparts, and both blur
frames. Their contact sheet was visually inspected. Full and isolated
outputs now have distinct names so one cannot overwrite the other's
review evidence. References, engine images and difference images stay
outside Git.

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
cargo test --release -p pm-render --test text_raster   # PM_TEXT_ONLY, PM_TEXT_DIFFS, PM_TEXT_DUMP, PM_TEXT_PROFILE, PM_TEXT_COVERAGE=cpu, PM_TEXT_REPORT
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

## Round 5: scaled glyphs, stroke precision and live cache identity

The scaled real-project trace uses the production `legacy/gl/raster.ts`
and compositor. At 16/18 seconds the roughly 105 px layer has world scale
3.39832, quarter-step raster density 3.5 and a 5688×991 backing canvas.
At 24 seconds the 73.813 px layer has scale 6.92835, density 7 and a
2366×1540 canvas. Legacy caps density by 8192 divided by each logical
geometry dimension; it rasterizes at that density, then bilinearly samples
that texture over the logical box. It uses LINEAR min/mag filtering and no
mipmaps. World-scale changes select `ceil(scale*4 - 1e-9)/4`, clamped
to 0.25–32. The effective density is part of the raster key: a new bucket
rerasterizes; movement within a bucket reuses the bitmap. There is no extra
temporal hysteresis. Above the 8192 cap, equal effective densities share
the raster even when requested scale changes. Content, geometry, variation,
text-animator state and preview cropping also participate in that key.
The native engine already has this raster-then-composite order;
changing to layout-size or final-device-size glyph rendering would be wrong.

The large crisp-versus-soft stem mismatch instead came from
[`Device::drawPath`](https://github.com/google/skia/blob/chrome/m152/src/gpu/graphite/Device.cpp): Skia recognizes a rectangular glyph contour and selects
[`AnalyticRRectRenderStep`](https://github.com/google/skia/blob/chrome/m152/src/gpu/graphite/render/AnalyticRRectRenderStep.cpp), whose continuous edge coverage differs from the
four-sample path renderer. Native text now recognizes the same rectangular
contours and uses analytic edge distances and half coverage. Curved contours
retain Graphite's tessellation, including its full-canvas viewport round
trip. CPU quadratic-to-cubic conversion and shader interpolation retain
their different arithmetic (`PatchWriter::mix` versus shader `mix`).
The 18/24-second original-frame differences shrink from 727/1550 pixels to
single-digit counts; unresolved single-sample curved-edge differences still
fail the gate. Captured source textures independently reduce from 636/636
and 1114 differing pixels to 7/7/3 pixels beyond 1 CV.

Two source-level differences in the stroke port are corrected:
[`SkPoint::setLength`](https://github.com/google/skia/blob/chrome/m152/src/core/SkPoint.cpp) multiplies each coordinate by its double scale before
rounding, and [`SkPathStroker`](https://github.com/google/skia/blob/chrome/m152/src/core/SkStroke.cpp) keeps tangent vectors directly instead of
adding/subtracting endpoints. The endpoint round trip lost precision and
changed stroke control points. Together they close the captured
`sdf-synthetic-0016`, `sdf-caption-0027` and `sdf-caption-0028` failures,
plus classic, fade, spotlight and several other small caption residuals.
The main raster gate improves from 468/495 to 484/495; closure improves
from 110/116 to 113/116, at the existing 1 CV bound. Chromium explicitly
sets `SK_SUPPORT_LEGACY_CONIC_CHOP` in its
[SkUserConfig](https://chromium.googlesource.com/chromium/src/+/HEAD/skia/config/SkUserConfig.h),
so the modern upstream conic chop experiment was discarded and Chromium's
configured order is retained. Caption-pop remains unresolved.

The live font-cache correction is documented in `text.md`: full platform
identity distinguishes DOM optimizeLegibility from canvas auto while
preserving valid 24/26 sharing. The seed e2e's 23 rows match the page's
Canvas2D advances within 0.00001526 px, including the previously wrong
20/400 and 30/500/600 entries. The typing harness additionally supports
`PM_TEXT_TYPING_PAIRED=1`, running legacy and native for each scenario with
the same input/RAF sampling. Legacy completion requires a successful draw
of the changed resolved text; animated property objects are not compared
by reference.

The final app matrix passes **21/21**, including caption burn-in export,
engine-drew checks, live seeds, the 1000-layer playback benchmark and typing.
Native typing p95 in that run is 19.2/19.6/14.0 ms for title/paragraph/system
text (60 keys each, no misses). The paired run passes **6/6** and measures:

| Scenario | Legacy p95 | Native p95 | Native minus legacy |
|---|---:|---:|---:|
| Title, 96 px | 8.4 ms | 18.8 ms | +10.4 ms |
| Paragraph, 48 px | 7.8 ms | 13.5 ms | +5.7 ms |
| System UI, 24 px | 7.9 ms | 13.2 ms | +5.3 ms |

Each run has 60 samples and zero misses; display scan-out is excluded.
The scenarios use the same harness and project load on the shared machine.
Background one-minute load varies from 12.0 to 25.4 across the paired run;
this is a measured comparison, not a controlled idle-machine claim.
Native meets the 33 ms budget but does **not** meet no regression versus
legacy. The asynchronous engine/layout/frame-bridge path needs profiling;
this measurement does not attribute the difference specifically to census
work. In 10 seconds of 1000-layer playback, legacy/native RAF p95 is
9.9/9.8 ms, with 1190/1202 callbacks and 1/0 long intervals. The census adds
300 scans, 301 visited nodes and 17.9 ms aggregate scan time; startup's
maximum individual scan is 2.3 ms. Playback shows no measured regression.

Both debug and release run all 611 raster cases and all 477 corpus frames.
The raster results agree (597 pass, 14 fail); the corpus results agree
(467 within 2 CV, 10 fail, no comparison escapes). The font checks pass:
77 pm-text unit tests, 5200 layout cases, 26 registry checks, and 9 level 2
layout tests in each profile; pm-node's 65 library tests also pass in each.
CLI tests use its binary target: 28 unit tests and 5 render tests pass in
both profiles, including decoded text-ink validation. Native/wasm evaluation
has zero mismatches over 6162 layouts and 74032 units, and all 1,815,420
native/wasm replies agree. TypeScript checks pass 102 tests; Svelte reports
zero errors and one existing IconCropper warning. These are non-LTO builds.

Review evidence stays outside Git in the private corpus review directory,
`review/text-round5-20261018/index.html`: before/after sheets for every
changed captured raster and all remaining failures, a corrected-stem
magnification, and contact pages reviewed after generation. Per-case
counts, maximum deltas and image paths are in the round 5 JSON report;
visible (`max >3 CV` or `>10` pixels) and sub-visible entries are listed
separately. Both classes still fail their original gates.

## Round 6: shared Canvas2D paths and image filters

Level 1 canonical glyph paths now go through
`pm_vector::canvas2d::Context`, including its rectangle/convex/path dispatch,
Graphite tessellation, original backing-canvas viewport, 1/256-pixel vertex
snapping, sample decisions, stroker and per-sample N32 blend/resolve. The
text-specific copy of the Graphite tessellator and analytic rectangle
rasterizer is removed. `PathRasterizer` is a small host interface in
pm-text, implemented by pm-vector, which already depends on pm-text for
SVG text. It avoids a circular dependency and keeps a single path renderer.

Canonical glyphs of a draw share one recorded pass. Resolving each glyph
separately was wrong for analytic rectangular glyphs next to curved paths:
the curved draw requests MSAA for the pass, including the rectangle's
triangle coverage at its corners. Device-space region rendering keeps the
full canvas's viewport round trip before cropping; its regression test
compares a crop byte for byte with the full canvas for curved fills,
rectangles and round strokes. Canonical fake-bold outlines are framed at
64 px and filled through the same renderer. Strokes above the strike limit
follow the `needsExactCTM` branch in `PathOpSubmitter`: the canonical
outline is first transformed into source space, then stroked with the
unchanged canvas CTM.
`SkFont::setupForAsPaths` re-enables subpixel positioning; the Mac scaler's
4× horizontal path query is therefore retained for the canonical strike too.

The small caption-stroke hypothesis has a different result. Chromium's
[SubRunContainer](https://github.com/google/skia/blob/chrome/m152/src/text/gpu/SubRunContainer.cpp)
tries the SDF/direct-mask routes before the ordinary path route when the
device text size is below the maximum strike size. A nonzero-width stroke
does not bypass these routes. At 54 and 82 px, the requested colored-stroke
and pop cases therefore use
[SkScalerContext::GenerateImageFromPath](https://github.com/google/skia/blob/chrome/m152/src/core/SkScalerContext.cpp),
with CPU analytic coverage, horizontal LCD filtering and mask gamma. They
retain that route; moving them to Graphite would change today's rendering.
Their remaining CPU outline/coverage differences were strict failures in
Round 6; FONTS-R below supersedes that blocker classification.
A counterfactual run forcing the six pop cases through Graphite produced
1,009–11,151 pixels beyond 1 CV (max 156–214), compared with 0–74 pixels
(max 0–13) on the production mask route. The counterfactual is not shipped
and is not counted among the production residuals.

Text animator blur now uses `pm_vector::canvas2d::filter::BlurPlan` and the
existing path-mask GPU executor/shader, parameterized for premultiplied
RGBA8. It shares the scale selection, quantized rescale steps, paired taps,
2D/1D selection, transparent borders and final sampling with path masks.
The initial N32 save layer includes ink outside the output canvas that can
reach it through the filter. Canvas bounds select the filter plan; clipping
to the raster crop happens after filtering. Filter results are retained by
the encoder, rather than returned as unaccounted targets to the target pool.
The old text-only float Gaussian shader is removed.

The CPU reference uses the same plan and a measured Metal UNORM8 sampler:
weights round to 1/256, interpolation operates on integer channel values,
and the result rounds to 1/16 of a code value before normalization. An
independent GPU test sweeps every horizontal weight bucket, with nontrivial
interpolation on both axes and four input channels. Normalizing bytes
before interpolation changes halfway cases.
The CPU model also preserves source origins, border coordinates and N32
storage after each pass. The existing float paint bound remains unchanged.
All 54 path-mask capture canvases remain bit-exact after the RGBA extension.

Typing performance remains owned by the performance lane. The committed
paired harness (`fad1e567`) and the round 5 measurements above are retained.
To rerun the same legacy/native comparison after building the current addons:

```sh
cd apps/desktop
PM_TEXT_TYPING_PAIRED=1 PM_TEXT_TYPING_MEASURE=1 \
  ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bunx playwright test e2e/native-engine-text-typing.spec.ts --workers=1
```

The independent `capture/render.ts --set paths` set adds two 320 px caption
strokes (plain and oblique/highlighted at density 1.25) and two 300 px
synthetic faces. Both synthetic captures are byte-identical, including the
pre-quantization skew correction. The large strokes expose two further
strict failures: `path-stroke-0000`, 154 pixels beyond 1 CV, max 4;
`path-stroke-0001`, one pixel, max 64. The first is a straight analytic
stroke edge (153 alpha deltas of −3 and one of −4); the second is a single
sample change. Sharing the renderer does not establish closure of those
previously uncaptured coordinates. Both remain unpinned.

Fresh app verification after merging document-host integration `f89a239e`:
22/22 Playwright tests pass with the native engine enabled, one worker and
the machine-wide Electron lock, including caption burn-in export, editor
boot/revival, split/resize/anchor/animation, the live seed comparison and
large-timeline census. The seed test compares 23 rows with a maximum width
error of 0.00001525879 px, within its unchanged 0.001 px bound. Targeted
Vitest: 102/102; svelte-check: no errors and one existing cropper warning.
Native typing p95 is 12.7/13.1/13.5 ms over 60 keys each, zero missed keys.
This is an absolute measurement; the round 5 paired regression remains
assigned to the performance lane.

The debug and release gates retain the original bounds and no accepted pins:

| Gate | Within bound | Remaining failures |
| --- | ---: | ---: |
| Main raster, 1 CV | 484/495 | 11 |
| Closure raster, 1 CV | 113/116 | 3 |
| Composed corpus, 2 CV | 473/477 | 4 |
| Source textures, 1 CV | 0/3 | 3 |
| Independent path cases, 1 CV | 2/4 | 2 |

The debug main/closure/source runs produce the same 614 engine RGBA hashes
as release. All 477 composed corpus score rows also agree across profiles.
pm-text's 78 unit tests pass in both profiles, including the
canonical-path skew cache check, alongside the full 5,200-case layout corpus
and registry checks.

The original 12 visible checks are reduced to 10; the independent path
capture adds two more. Across all captures there are 12 visible and 11
sub-visible strict failures. Here “visible” means max >3 CV or more than
10 pixels beyond the comparison's bound. It does not permit the remaining
smaller errors. All 12 COLR cases draw; two retain 1/5 pixels at max 2 CV.
All 27 variable-font cases remain byte-identical; all 74 color cases meet
1 CV. The three requested SDF stroke/bold cases remain within 1 CV.

Fresh `pm-cli parity eval --wasm`: 1,891,448 compared, zero mismatches or
questions; native/wasm identity: 1,815,420 replies, zero differences. The
named variable-font ink class remains unchanged (405 bounds and 405 raster
selections, max 1.191e-4 px).

Round 6 measurements and every remaining side-by-side are recorded in the
lane report. The round 5 counts above remain historical.

## Round 7: FONTS-R residual ratchet

Orchestrator decision **FONTS-R** (2026-10-19), after side-by-side inspection
of `caption-pop-0459` and `real_powermove-ad-18`, classifies the 23 remaining
Round 6 residual checks as visually identical edge-sample decisions, not
owner-rule blockers. This supersedes their earlier blocker classification.
It does not claim byte equality or change a pixel tolerance.

The explicit RATCHET is
[`tests/fixtures/text-residuals.json`](../../engine/crates/pm-render/tests/fixtures/text-residuals.json).
It contains exactly those 23 measurements: 19 raster checks (main, closure,
source textures and the independent large-path capture) and four composed
frames. Every entry records its measured number of pixels beyond the
original bound, maximum channel delta, one-line cause boundary and review
PNG basename. Images and private directory paths stay outside Git; the
lane report resolves the names to the private Round 6 review sheets.

`text_raster` and `text_corpus` share the same checker. The original strict
bounds remain **1 CV for raster/source textures** and **2 CV for composed
text**. All unlisted cases still must meet their original strict bound.
The checker requires exact equality with both listed measurements:

- Increasing either the pixel count or maximum delta fails.
- Decreasing either one fails until its budget is lowered to the observed
  value; remove the entry when the case meets the strict bound.
- Adding new cases or increasing budgets is forbidden by FONTS-R.
- A selected listed measurement cannot disappear through an empty shot,
  refusal, missing media or an early return. Corpus identities include
  the exact project, frame time and original/isolation pass.
- Geometry, text-routing and non-text Standard-class checks remain in
  force. The ratchet cannot waive them.

This is a case-specific, downward-only snapshot ratchet, **not a widened
tolerance**. The summaries retain the strict counts and print ratcheted
counts separately. Existing residual images are still exported for review.
Six capture-independent contract tests cover new residuals, both regression
axes, improvements with stale budgets, lowering/removal, missing selected
measurements, corpus identity collisions and fixture/path validation.

The follow-up **textexact** lane owns reducing these measurements to zero
before beta: line-for-line ports of Skia's CPU AAA scan converter and
SkStroke for glyph masks, Graphite's stroke tessellation for large-path
cases, and the Fontations COLR route. Keep the SDF edge residual in its
coverage list too. Lower the ratchet after each measured improvement and
retain the original references. These ports are not started in Round 7.

The Round 7 integration merge also retains the window-scoped served-font
provider when installing the bridge lane's `RenderFonts`. Both viewer frames
and frame/layer snapshots select their originating port's seeds. The scoped
provider reports a failed registry query through the bridge's font-health
boundary, so later fallback queries in that frame fail immediately and a later
frame can retry. The incoming unscoped broker API remains available for its
independent callers; exports and jobs retain the served hub.

## Gaps

FONTS-R governs the 23 nonzero Round 6 residual checks. They pass only at
their exact recorded snapshot budgets and are not owner-rule blockers.
Every unlisted case keeps its original strict bound; any regression, new
residual or improvement with a stale budget still fails. The Round 4–6
strict-failure counts above are historical; Round 7 adds the explicit
orchestrator-approved ratchet without changing the renderer or references.

- Caption-pop, colored caption stroke, whisper and paper remain nonexact.
  Counts and review filenames are in the ratchet; the lane report resolves
  their side-by-side paths.
  Round joins, miter limits, fake-bold interpolation, descriptor relaxation,
  inverse/forward scale order, normalization precision and tangent storage
  were checked against Chromium's configured Skia. No tolerance or profile
  parameter was changed. The larger caption-pop cause is still unresolved
  and assigned to textexact under FONTS-R.
- The closure distance-field residual `sdf-sizes-0004` remains (4 pixels by
  2 CV). The three stroke/bold SDF failures are closed by the tangent fix.
  The row-local backward distance walk still works with debug overflow
  checks; it never subtracts beyond the first row.
- COLR v0 at 96 and 200 px still differs at a few pixels by 2 CV. The
  CoreGraphics approximation is gone. Fontations supplies unhinted paths,
  all layer bounds, transforms, clips, palettes and gradient geometry; the
  existing Skia analytic filler supplies coverage, with N32 integer solid
  alpha blending. Whole-pixel placement uses the text blob's 1/8-pixel x
  bias, not a 1/2-pixel bias. A direct-span-blitter experiment did not fix
  the residual and was discarded. Radial/sweep paints, composite modes,
  rotations and clipping have implementation coverage but no captured
  pixel-parity evidence beyond the linear-gradient and v0 captures; do not
  treat them as a closed parity class.
- The real project is compared in full and in isolation. Shared Canvas2D
  pass decisions reduce its actual t16/18/24 source-texture residuals to
  6/6/2 pixels beyond 1 CV, max 64/64/5. Sparse curved-edge/MSAA sample
  disagreements remain, including with the shared path renderer. These
  remain nonexact and are covered only by their exact FONTS-R snapshots;
  the 2-CV composition bound is unchanged. The ratchet lists each composed
  frame separately.
- Animator blur now meets the unchanged 2-CV corpus bound through the
  shared quantized Graphite plan/executor, including isolation and the
  original project. It no longer uses the text-only float Gaussian.
- Paired typing is slower than today's renderer by 5.3–10.4 ms at p95,
  although every native scenario remains under 33 ms. The performance lane
  owns the full engine's no-regression typing requirement. The paired
  measurement exposes the gap alongside the absolute budget.
- Level 2 anchoring, animated/ranged semantics, bounds and picks are
  explicitly assigned to **textl2** by decision C. This lane adds no level 2
  semantics or routing removal. The final compclose integration merge
  classifies its remaining animated/ranged/typography-anchor refusals as
  `Feature::Text` with a compat-level-2 reason; those remain failing routes.
- `Feature::Text` remains for a renderer without fonts, unparsed styles or
  an unanswered registry query; the derouter owns retiring that machinery.
- Font-cache entries from pseudo-element text and cache purges under
  memory pressure remain unmodelled. Canvas operations in workers and
  offscreen text scrolling into view without a DOM mutation are also
  unobserved. Arbitrary custom CSS needs further independent captures.
  The previous live-seed mismatch is closed; the current e2e validates
  23 rows, including equal-key DOM and canvas creation orders.
- A page-only web font (for example a font loaded by onboarding) need not
  be present in the engine's registry. Project fonts remain fully served.
- The measured mask tables, emoji placement and SDF GPU arithmetic are
  specific to this Apple-silicon/macOS/Electron reference. Other machines
  and Chromium updates need their own captured comparisons.
- The shipping LTO profile remains unverified. Test/addon builds use
  `CARGO_PROFILE_RELEASE_LTO=false` because the cached build otherwise
  reports missing bitcode.

## Changes outside the lane

2026-10-19 (lane ne-fonts, round 7):

- `pm-render/tests/{text_raster,text_corpus,text_residuals}.rs`,
  `tests/support/{mod,text_residuals}.rs` and
  `tests/fixtures/text-residuals.json`: shared FONTS-R enforcement,
  metadata-only snapshots and contract tests. No production rendering,
  font selection, pixel-reference or tolerance changes.

2026-10-18 (lane ne-fonts, round 6):

- `pm-vector/src/canvas2d/{context,analytic,text,mod}.rs`: shared path
  adapter and region rendering with the full backing-canvas viewport.
  `svg_text.rs` initializes the optional adapter for its existing consumer.
- `pm-vector/src/canvas2d/filter.rs`: RGBA CPU reference for the existing
  Graphite blur plan and independently verified UNORM8 sampling arithmetic.
- `pm-render/src/composite/path_masks.{rs,wgsl}`: parameterize the existing
  GPU executor for R8 or premultiplied RGBA8; retain the path-mask behavior.
- `pm-render/src/shaders/text.wgsl`: remove the text-only float Gaussian
  entry point after switching text image filters to the shared executor.
- `pm-render/examples/text_review.rs` and `pm-render/tests/text_level2.rs`:
  initialize the painter's optional path adapter; no level 2 semantics.
- `apps/desktop/src/renderer/src/engine-doc/{install.ts,strict.test.ts}`:
  preserve seed application during the document-host attach/revive merge,
  and retain initial seeds and change callbacks for synchronous whole-face
  providers. The first post-merge live census caught this integration
  regression; the new unit test exercises both seed deliveries.

2026-10-18 (lane ne-fonts, round 5):

- `pm-skia-path/src/{lib,stroker}.rs`: Skia normalization and tangent-vector
  arithmetic used by text glyph frames; its upstream unit suite passes.
- `pm-render/examples/text_review.rs`, `pm-render/tests/text_level2.rs` and
  `pm-vector/src/canvas2d/svg_text.rs`: initialize the painter's optional
  backing-canvas viewport. The SVG consumer has no supplied canvas size.
- `pm-eval/tests/corpus.rs` and `pm-node/src/fonts/hub.rs`: legacy/static
  seed literals initialize the optional complete cache identity.
- `apps/desktop/e2e/native-engine-text-typing.spec.ts`: paired legacy/native
  measurement using the same harness and scenarios.

2026-10-18 (lane ne-fonts, round 4):

- `apps/desktop/src/main/engine/{index,agent-jobs,render-queue}.ts`: carry
  the queue's starting window to its font session while that window is
  open; renderer ids are runtime-only and never persisted across relaunch.
- `apps/desktop/src/renderer/src/engine-doc/{install,fonts}.ts`: a changed
  seed set invalidates editor answers and redraws the overlays; an old
  startup reply cannot replace a later seeds-changed notification.
- `apps/desktop/src/renderer/src/legacy/bootstrap.ts`: start observing
  canvas font assignments before engines and extensions install.
- `apps/desktop/e2e/helpers/engine-drew.ts`: a missing main-process report
  fails the gate. New seed-order and large-timeline census measurements
  accompany the existing text typing benchmark.
- `engine/corpus/src/capture.ts`: `--text-only` reduces the real project
  too. The CLI exits after closing its capture session.
- `engine/crates/pm-render/tests/support/mod.rs`: discover `real/` as well
  as `projects/`, preserving the `real/` prefix in comparison names.
- `engine/crates/pm-cli/tests/render.rs`: verify actual white text ink in a
  decoded PNG over a black background, rather than only file existence.
- `engine/crates/pm-vector/src/canvas2d/svg_text.rs`: the earlier lane's
  one-line `masks: None` initializer (omitted from earlier lists).

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
