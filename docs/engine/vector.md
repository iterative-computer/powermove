# Vector rendering (pm-vector, pm-render::vector)

Design notes for shape layers, editable paths, solids (with and without `radius`), and SVG
images (plan sections 8, 10, and 12; amendment 10). Geometry (shape outlines,
path evaluation, trims, repeaters) is pm-vector's geometry side and is not
described here.

| Content | `compat` level 1 | `compat` level 2 |
|---|---|---|
| `shape` (rect, ellipse, polygon, star, line) | Today's Canvas2D raster at the quantized scale, emulated (below) | Output resolution, analytic edges |
| `shape` with editable paths | Same, `rasterPaths` | Same |
| `solid` | Today's single-sample quad (radius ignored) | Rectangle (rounded when `radius > 0`), analytic edges |
| `image` (SVG) | Routed (resvg rasters are outside the 2D class) | Vectors or resvg at output resolution |
| SVG with `<text>` | Routed | Routed (SVG text goes through `pm-text`) |

## Level 1: Chromium's Canvas2D, emulated

Today a shape layer is `rasterShape(d, scale)` (or `rasterPaths`) drawn into
a canvas at the continuous raster scale, uploaded with
`UNPACK_PREMULTIPLY_ALPHA_WEBGL`, and drawn as a textured quad. The app's
Electron 44 is Chromium 152, whose accelerated Canvas2D runs on Skia
Graphite (Metal). `pm_vector::canvas2d` reproduces that pipeline on the CPU:

1. **Blink** (`blink.rs`): `CanvasPath` in float, the `moveTo`+`lineTo`
   fast path (a line is drawn as a line), `ellipse` as two 180° `arcTo`
   conics, `arcTo` tangent conics, `Path2D` strings through Blink's SVG
   number parser, and the transform forwarded to the recording canvas.
2. **Skia** (`skpath.rs`, `convex.rs`, `context.rs`): `SkPath` verbs,
   conic construction (`BuildUnitArc`), `isRect`/`isLine`, `isConvex`
   (`computeConvexity` and the `Convexicator`), and the canvas matrix as an
   `SkM44` composed in `f32`.
3. **Graphite's renderer choice** (`context.rs`): a line is drawn by the
   analytic line renderer; a pixel-aligned rect by the non-AA bounds
   renderer; other rects by `AnalyticRRectRenderStep` (`analytic.rs`: the
   36-vertex mesh, coverage evaluated at the pixel center or each sample);
   convex paths by the unstencilled convex wedge pass; other paths by the
   stencilled wedges or curves; strokes by the tessellated stroke renderer.
4. **Tessellation** (`tess.rs`, `stroke.rs`): `PatchWriter`'s chopping and
   Wang's formula on the CPU, `tessellate_filled_curve` and the stroke
   vertex function in `f32` as the GPU runs them.
5. **Rasterization** (`msaa.rs`): vertices snapped to 1/256 px (round half
   away from zero), the top-left rule, 4× MSAA at the standard positions
   `(0.375, 0.125) (0.875, 0.375) (0.125, 0.625) (0.625, 0.875)`, per-sample
   8-bit source-over, and a `(sum + 2) / 4` box resolve. Graphite
   multisamples a pass only when a draw requires MSAA: a canvas whose draws
   are all analytic renders single-sampled at pixel centers.

Coverage semantics that matter:

- **Stencilled fills** (concave paths) cover exactly the samples inside the
  polygonized path under the fill rule: the fill reduces to that polygon.
- **Convex fills** draw every wedge triangle `(fan, p0, p3)` and every
  middle-out triangle `(T(i−1), T(i), T(i+1))` directly, with a depth test
  so each sample blends once. Their union is not the polygon: slivers
  between a curve's chords and its polyline add coverage whatever their
  orientation. (Measured: degenerate cubic edges, which `tracePath` writes for
  every corner, flipped samples until this was modelled.)
- **Strokes** are the union of the stroke triangles.

### What the GPU computes (measured with a compute-shader oracle)

A WGSL compute shader running the same expressions on this Mac's GPU
(Metal compiles with fast math, as it does Graphite's MSL) settled three
questions, each over 200,868 random and ellipse inputs:

- Metal's `mix(a, b, t)` is `fma(t, b, fma(−t, a, a))`, not
  `fma(b − a, t, a)` (that matched 76%).
- `abc / uv` is `abc × rcp(uv)` (100% with the GPU's reciprocal).
- The GPU reciprocal is faithful, not correctly rounded: over all 2²³
  mantissas it rounds the other way for 10.3% (`[1, 2)`: 7,520,434 correct,
  859,417 one ulp above, 8,757 one ulp below), independently of the exponent.
  The emulation uses the correctly rounded reciprocal; a 65,536-entry
  threshold table predicts the GPU for all but 0.03% of mantissas but
  changed no captured case, so it is not used.

The NDC round trip and viewport transform measured no effect; other
rounding variants (floor, ceil, half-even snapping; other transform
evaluation orders) were each worse.

### The capture tool

`engine/crates/pm-vector/tools/canvas-ref` captures the references:
`cases.ts` generates the cases (a sweep over every shape type and property,
editable-path scenes, raw canvas diagnostics, SVG images per bucket) and
writes `tests/fixtures/canvas-cases.json`; `page.ts` rasterizes each case
with today's code (`rasterShape` copied verbatim and checked against
`raster.ts`, `rasterPaths` imported) in a hidden Electron window and reads
the uploaded texture back. Run from the worktree root:

```sh
~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bun engine/crates/pm-vector/tools/canvas-ref/run.ts
```

Captures go to `~/Library/Application Support/Powermove Engine Corpus/
canvas-<stamp>-<commit>` (outside the repository). `tests/canvas2d.rs` and
`tests/svg.rs` read the newest complete capture (or `PM_CANVAS_REF`);
`examples/canvas_compare.rs` prints per-case differences (`PIX`, `ROW`/`COL`
for details).

### Measured parity

On `canvas-20261016-075306-cbd1dcfd` (Electron 44, Chromium 152, Apple
M-series GPU), 454 Canvas2D cases:

| Result | Cases |
|---|---|
| Bit-exact | 427 |
| Within 1 code value (analytic renderer rounding) | 22 |
| One to three pixels with a flipped MSAA sample (max 45–64) | 5 |

Every case has at most 0.1% of pixels off by more than 1 code value. The
five cases that miss "no pixel off by more than 4" are pinned in
`tests/canvas2d.rs`: `shape-ellipse-big-s2.75` (3 px), `shape-random153-s3.75`
(1 px), `paths-badge-rotated-scaled-s3` (2 px), `diag-ellipse-380` (2 px),
`shape-corpus-rect-r24-s1` (2 px).

Corpus (`pm-render/tests/corpus.rs`): `shapes/primitives`,
`shapes/editable-paths`, and `shapes/svg-import-paths` are 100.000% exact at
every sample, opaque and transparent float frames. `composition/metadata`
(the radius-24 rectangle, rotated) has 2 to 6 pixels with one flipped sample
(max 43); it is pinned in `KNOWN_RESIDUALS`, and the same raster captured by
the tool shows the same two pixels, so the capture and the corpus agree and
the emulation is what differs.

### Residuals

Each remaining difference is a single MSAA sample within 0.07–0.46/256 px of
a tessellated edge. In the corpus rectangle the sample is 0.145/256 px inside
a middle-out sliver triangle whose vertex lies 0.0004/256 px below a
rounding boundary; the GPU must have placed it at least three float ulps
lower, which no evaluation order, reciprocal model, snapping rule, or NDC
round trip reproduces. They are left as measured.

Performance (release, this Mac): the whole 454-case sweep rasterizes in
1.8 s including PNG decoding; the largest case (1947 × 1397, MSAA) in 0.13 s.

## Level 2: output resolution, analytic edges

A level 2 layer's content becomes a `CoverageScene` (`render/scene.rs`):
ordered fill and stroke items in device pixels, each with one solid paint.

- **Coverage.** Every item is flattened in device space within 1/256 px
  (`CoverageItem::device_outline`; strokes are expanded in local space first,
  as Canvas2D strokes under the transform) and rendered into its own cell of
  an 8-bit atlas (`AtlasLayout`, shelf-packed with a 1 px gutter) by Vello
  0.11 on the GPU (`AaConfig::Area`, `Rgba8Unorm` storage) or by `vello_cpu`
  as the fallback. Left to themselves both flatten curves with a
  quarter-pixel tolerance, which eroded every curved edge by about 0.1 px (a
  radius-50 circle lost 42 px², 0.5%); both now rasterize the same polyline.
- **Paint.** The engine's own pass (`pm-render/src/vector/cache.rs`,
  `PAINT_WGSL`) composites `paint × coverage` item by item with
  premultiplied source-over into an `Rgba16Float` or `Rgba32Float` target at
  the frame's precision (amendment 10).
- **Fallback.** `CoverageBackend::Auto` uses Vello and falls back to
  `vello_cpu` when Vello cannot be created on the device;
  `Renderer::set_vector_backend(CoverageBackend::Cpu)` forces the fallback.

Measured (`pm-render/tests/vector.rs`, against 16 × 16-sample exact
coverage, edge pixels only):

| Content | Level 2 | Level 1 (today's raster, resampled) |
|---|---|---|
| Ellipse 150 × 96, rotated 17°, 130% | 48.2 dB, max 0.015 | 19.1 dB, max 0.32 |
| Line, stroke 9, round caps | 53.7 dB, max 0.011 | 20.7 dB, max 0.15 |
| 7-point star | 49.9 dB, max 0.018 | 19.8 dB, max 0.29 |
| Solid 120 × 80, radius 30 | 48.8 dB, max 0.008 | radius ignored |
| Solid 120 × 80, radius 0 / missing / −5, rotated 23°, 115% | 54.1 dB, max 0.006 (447 of 516 edge pixels partial) | single-sample quad: only 0 or 1 |
| SVG rect, rotated 23° | 54.7 dB, max 0.005 | routed |

Vello on the GPU and `vello_cpu` agree on a stroked star and a rounded rect
except one pixel, by 1/255.

## SVG images

`pm_vector::render::svg::SvgDocument` parses with usvg 0.48; as in an
`<img>`, only `data:` URLs resolve (no file or network loads). Parsing and
the paint-record walk run on a stack sized to the document's nesting
(`element_depth`, 16 MiB plus 64 KiB per level) when it nests more than 64
levels, and on the caller's stack otherwise; a document nested deeper than
Chromium's 5,000-element limit is an error, as Chromium shows it broken.
`raster_compat_with_text` (diagnostics) keeps the caller's font resolver
on the caller's thread and refuses documents nested deeper than 64 levels.

- **Level 1** (today's buckets) is drawn: `svg_raster_dimensions` ports
  `svgRasterDimensions`, and `SvgDocument::raster_compat` paints the bucket
  as Chromium 152 paints `drawImage(img, 0, 0, w, h)` into Canvas2D (below).
  `pm-render` plans it as the image quad (`VectorSource::SvgBucket`, keyed by
  path, stamp and bucket size) with the layer's `fit` window. A document
  Chromium shows as a broken `<img>` paints nothing (`media.md`, "Broken
  level-1 SVG documents"). Only documents the replay paints inside the 2D
  class at every bucket are drawn ("What level 1 draws" below); everything
  else, and a document usvg rejects but Chromium paints (external DTD
  subsets, parameter entities), stays routed as `Feature::SvgImage` with
  the reason `SvgDocument::compat_supported` names. Declared ISO-8859-1
  and UTF-16 sources decode as Chromium does at both levels
  (`pm_media::image::svg::utf8_source`). Offline/unreadable files are named
  layer failures.
- **Level 2**: an SVG of solid fills and strokes in plain groups that stays
  inside its box is a coverage scene (vectors at output resolution). Anything
  else (gradients, patterns, clips, masks, filters, group opacity, blend
  modes, embedded images, `crispEdges`) or content cropped by `fit: cover` or
  the viewport is drawn by resvg into the device rectangle with the box and
  viewport as an antialiased clip. Where today's quad would clamp the
  texture's edge texels over a `contain` box, level 2 draws nothing.
- **Text**: SVGs with `<text>` stay routed at every level until the pm-text
  path below meets SVG’s 2D parity class.

### Chromium SVG paint-record replay

Chromium **152.0.7977.54** is the version in the canvas reference manifest.
Its [SVGImage::DrawInternal](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/renderer/core/svg/graphics/svg_image.cc)
saves the Canvas2D canvas, applies the destination clip and source-to-destination
matrix, and replays `PaintRecordForCurrentFrame` with `drawPicture`. With
Graphite enabled in the capture, that is the analytic and MSAA paint pipeline
the vector lane emulates; Skia is pinned by Chromium's DEPS to
`b6d106297ff9ef2ff8094033695d045e87775581`.

`canvas2d::svg` builds Blink's record from the original elements (ids are
injected so oval, rounded-rectangle and conic SkPath identities survive
usvg): the layout container (`LayoutUnit` floor, pixel-snapped root scale,
`preserveAspectRatio` in double), `PaintChunksToCcLayer`'s transform and clip
ops, `SVGObjectPainter`'s flags (paint-server alpha, dithered gradients),
`StrokeData` dashing (`SkDashPath` with the measured tolerances), clip paths
(`PathToRRect` else enclosing rect plus path), and `feGaussianBlur` as the
color-filter/crop/blur chain. `canvas2d::svgrender` replays it as Graphite's
`Device` does:

- shape recognition and `chooseRenderer` (analytic round rects with
  per-corner radii, tessellated wedges, MSAA fills and strokes), with the
  `ClipStack` (device-space pre-transform, `SkRRect::transform` as Skia now
  implements it, `ConservativeIntersect` with Skia's per-radius validity
  predicates, the legacy rrect clip shader Chromium's build selects, scissor
  snapping);
- Graphite's draw ordering: painters depth per draw, compressed painters
  order from a bounds manager, the draw pass sorted by order, and a
  per-sample depth test (`kDirectDepthLessPass` on `1 - z / max`). Clip
  elements that are neither absorbed nor analytic are drawn into the depth
  attachment at one Z past their last user when they leave the stack, so a
  draw recorded right after a clip's last user is clipped by it, as Chromium
  shows for two half-oval clipped rects at 333 × 150;
- the analytic rrect vertex and fragment shaders, `sk_analytic_clip_legacy`,
  `sk_dither`, gradient layout with `half` colorization, and the blur chain
  (`FilterResult::rescale` steps, `SkShaderBlurAlgorithm` kernels with half
  accumulation, 8-bit intermediates; the color-filtered rescale reads texels
  at float precision and the last step scales the requested amount).

`examples/svg_compat_compare` compares all 27 retained references and writes
side-by-sides outside the repository (`PM_MEDIA_EVIDENCE`; `--cases` and
`--capture` run experiment lists captured by `tools/canvas-ref/run.ts
--cases`); `examples/svg_gate` prints the gate verdict at each case's size
and over its ladder (`--cases` likewise). The replay's environment switches (`PM_SVG_DEBUG`,
`PM_SVG_PROBE`, `PM_SVG_MESH`, `PM_SVG_STRADDLE`, `PM_SVG_S0_RULE`) exist
only in builds with the `svg-diagnostics` feature (`examples/svg_debug`);
the engine never reads them.

The replay runs on the CPU, synchronously on a `VectorCache` miss (each new
bucket). Each draw is rasterized only over the pixels its geometry can touch
(analytic coverage is stored for the mesh's bounds; only tessellated draws
allocate a sample mask). Release build, 200-shape documents (measured
2026-10-18): 200 translucent circles (analytic, one sample) 0.29 s at
2048², 1.10 s and 228 MB RSS at 4096², 4.3 s and 878 MB at 8192²; 200
triangles (4× MSAA with a per-sample depth buffer) 0.11 s at 2048², 0.42 s
and 627 MB at 4096², 1.6 s and 2.4 GB at 8192². Legacy Chromium draws
these on the GPU. The full-target MSAA and depth attachments are the memory
floor; rendering in row bands would bound it and is not done yet.

### What level 1 draws

`SvgDocument::compat_supported(w, h)` is the level 1 gate. It is static:
it judges every bucket `svgRasterDimensions` can choose for an asset of
natural size `w × h` (`svg_bucket_ladder`: the requested scale is a
multiple of 0.25 up to the 8192 bound, so the ladder is finite, 205–512
buckets for the reference documents and 32,768 for a 1 × 1 record), so a
routing decision holds at every frame. The verdict is cached on the parsed
document per source size; judging a ladder replays the record through the
draw model without tessellating or rasterizing (`Canvas::classifier`), about
2 ms for 512 buckets.

What the gate admits is backed by Chromium references in two ways.
Every source construct it admits (element, attribute, property, and the
value forms below) is exercised by a reference document: the 27 canvas
cases and the 83 documents of `tools/canvas-ref/svg-gate-cases.ts` (383
cases at two or three bucket sizes each, captured with `run.ts
--svg-gate`). Each gate document isolates one construct with paint the
replay draws in one kind of pass (analytic shapes only, or tessellated
paths only): root sizing (`width`/`height` equal to, double, fractional,
`px`, percentage, `auto`, one dimension only, no viewBox, no size), a
viewBox origin (positive, negative, comma-separated), root `x`/`y`, all 19
`preserveAspectRatio` values with wide and tall viewBoxes, ten transform
forms on analytic shapes, tessellated shapes and groups, a shape transform
with its own opacity, clip-path and bounding-box gradient, gradient
percentages, `href` chains (`href` and `xlink:href`), stop ordering and
clamping, radial gradient transforms and user space, clip shapes, colors in
every accepted syntax, `currentColor`, inline `style`, `display`,
`shape-rendering`, rect radius rules, degenerate shapes, path command and
number syntax, `points` separators, inert stroke properties and editor
metadata. `canvas2d::svg`'s unit test `every_admitted_construct_has_a_reference`
fails if the allowlists name an element, property or attribute that no
reference document uses.

A document is drawn only when all of these hold:

- **Source** (`canvas2d::svg::check_source`): elements `svg` (root only),
  `g`, `defs`, `path` (no arc commands), `rect`, `circle`, `ellipse`,
  `polygon`, `polyline`, `line`, `linearGradient`, `radialGradient`, `stop`,
  `clipPath` (user-space units), `title`, `desc`, `metadata`. Values are
  checked, not only names: `transform` only on `g`, shapes and `clipPath`,
  in a list both parsers accept and invertible; `viewBox` (four finite
  numbers, positive size) and `preserveAspectRatio` (`none` or an
  alignment with optional `meet`/`slice`, no `defer`) only on the root; root
  `width`/`height` as numbers, `px`, percentages or `auto`; shape lengths as
  numbers or `px` (`rx`/`ry="auto"` is refused: Chromium draws square
  corners where usvg rounds); paints as `none`, `currentColor`, a color
  (hex of 3, 4, 6 or 8 digits, a keyword, comma-separated `rgb()`, `rgba()`,
  `hsl()`, `hsla()`) or `url(#id)` naming a gradient (no fallback);
  `clip-path` as `none` or `url(#id)` naming a `clipPath`; opacities as
  plain numbers; `display` `none` or `inline`; gradient lengths as numbers or
  percentages, `href` naming a gradient. The stroke properties are admitted
  (inert without a stroke; a stroke is refused below). Elements and
  attributes in other namespaces (editor metadata) are skipped, as Blink
  and usvg skip them. No `<style>` sheets, `use`, `symbol`, nested `svg`,
  `switch`, `a`, markers, masks, patterns, filters, images, text,
  animation, `overflow`, `paint-order` or `vector-effect`.
- **Record** (`check_record`): no text, no strokes, no filter layers, every
  shape antialiased (no `crispEdges`/`optimizeSpeed`), and gradients only
  with pad spread, a centered radial (no focal point or focal radius), at
  most four stops after Blink's padding, and not degenerate.
- **Draw passes** (`svgrender::pass_verdict`, at every bucket): no clip
  element drawn into the depth attachment, no analytic draw (round
  rects, ovals, unaligned rects) or analytic clip in a pass that also has a
  multisampled (tessellated) draw, and no filled path of nested rects
  (Graphite's `Device::drawPath` turns `SkPathPriv::IsNestedFillRects`
  paths into `drawRect`/`drawDRRect`, which the replay does not emulate).
  Which renderer a draw or clip gets depends on the bucket (pixel
  alignment, the destination's aspect), so the classification uses the
  replay's own `chooseRenderer` and clip stack at each bucket.

So level 1 draws documents of solid and gradient fills that are either all
analytic shapes (with group or shape opacity and rect or circular
round-rect clips the clip stack resolves analytically or geometrically) or
all tessellated paths without clips, under any admitted root sizing and
transforms. The tests:

- `tests/svg.rs` pins the gate on the 27 canvas cases and the 383 gate
  reference cases (`GATE`, `GATE_REFUSED`; everything else is admitted) and,
  with the captures present, requires every admitted case to be inside the
  2D class against Chromium.
- `pm-render`'s `tests/vector.rs` `level1_svg_images_match_chromium_buckets`
  draws the 46 gate documents with an explicit root size end to end (plan
  gate, `svgRasterDimensions`, replay, upload, image quad, texel for texel)
  and compares the frame with the replayed bucket and, with the capture,
  with Chromium: the icon form, the 200 × 120 documents (viewBox origin,
  shape transform, gradients) and all 42 `preserveAspectRatio` documents.
  It fails on the previous walker (max 228–255 CV on the root-sizing and
  aspect cases).
- The unit tests in `render/svg.rs` and `canvas2d/svg.rs` pin each refusal
  and the value checks.

The previous gate admitted `transform`, `viewBox`, `preserveAspectRatio`
and root `width`/`height` with any value although no reference covered
them, and the walker drew them wrong: usvg wraps a shape whose transform,
opacity or clip-path is set in a group without an id, and puts the root
viewBox transform in a group under the tree root; the walker took usvg's
matrix for both, so a shape's own transform was applied twice and the root
viewBox transform on top of Blink's root transform (an icon
`width="96" height="96" viewBox="0 0 24 24"` drew nothing; reviewer
captures max 229–255 CV). The walker now maps a `<g>` to its element's
transform, a wrapper group to the shape's transform (the shape pushes none)
and usvg's viewport group (detected exactly from usvg's
`ViewBox::to_transform`) to nothing. The gate references also found two
replay defects, both fixed: usvg copies a shared bounding-box gradient
under a generated id and the resolver fell back to default gradient
geometry (max 104 CV; the source element now comes from the shape's
inherited `fill`/`stroke`, and an unresolved gradient is an error), and
the nested-rects path above (max 85 CV; refused).

### SVG replay parity

On `canvas-20261016-075306-cbd1dcfd` (Chromium 152, Apple M-series), 18 of
the 27 cases are inside the 2D class. The gate admits 11 of them, at their
own size (opacity, gradient and even-odd at all three sizes; clip at 64²
and 200²), and all 11 are inside the class. Routed, with what blocks them:

| Case | Gate | Measured replay | Cause |
|---|---|---|---|
| solid 64², 200², 333 × 150 | analytic oval with a tessellated path | in class / in class / max 44 CV on 4 pixels | Chromium's analytic draws inside a multisampled pass take, at pixels along the analytic mesh's edges, one mesh triangle's center-evaluated coverage for every sample. A WebGPU probe of the same mesh on the same GPU (`/tmp` evidence, outside the repository) rasterizes per sample exactly as the emulation does, and the pixels follow the strip triangle's parity and the sample's position in the 2 × 2 quad, so the behaviour is Graphite's draw path, not the rasterizer; it is not modeled. Being in class at 64² and 200² is not evidence for other buckets. |
| stroke 64², 200², 333 × 150 | strokes | max 15 / in class / max 19 CV | the same multisampled-pass behaviour along the analytic round-rect stroke: whole straight-edge columns (x = 4 at 64²; x = 22 and x = 313 at 333 × 150), 9–19 CV, not isolated samples |
| transform 64², 200², 333 × 150 | strokes | in class | a dashed tessellated stroke next to an analytic rotated rect: the same pass mix; strokes are not admitted until the stroke cases reach the class |
| clip 333 × 150 | depth-attachment clip | exact | the stretched circle clip is an elliptical oval: the deferred clip draw goes into the depth attachment of a multisampled pass with analytic draws. Not on the ladder of the reference asset (its buckets keep the source aspect), but routed wherever it occurs |
| blur 64², 200², 333 × 150 | filter | max 2 CV; 14 / 3804 / 5008 pixels at 2 across the blurred interior | residual precision in the rescale and blur passes with the linear-RGB color filter |
| text 64², 200², 333 × 150 | text | max 120–157 CV | pm-text draws the font at the layout size under the device matrix; Blink lays SVG text out with `CalculateScreenFontSizeScalingFactor` (the scaled font, `FixedAscent`, an integer `Ascent()` at paint, `context.Scale(1 / s)`), which the pm-text placement does not yet follow. |

A lead for modeling the multisampled-pass behaviour, from the exp3 capture
set (the 20 ellipse experiments at 333 × 150 with the replay's
per-sample mesh ownership dumps; the analysis scripts and data are under
`/tmp`, outside the repository), counting only straddling pixels where the
per-sample and alternative predictions differ by more than 2.5 CV: every
pixel whose aligned 2 × 2 quad is not fully covered by the draw's mesh
follows the per-sample model (392 of 392). In quads fully covered by
exactly two mesh triangles, pixels split one sample to three take the
fragment of the triangle covering sample 0 for every sample (98 of 102),
while two-to-two splits follow the per-sample model (65 of 69); in quads
touched by three or more triangles most pixels follow the per-sample model
(218 of 234). That points at a quad-level decision in the hardware's
fragment generation for multisampled draws, not at the coverage shader.
No rule found so far predicts every pixel, so it is not modeled.
`PM_SVG_QUAD_RULE` (an `svg-diagnostics` switch) applies the
two-triangle quad rule: on the canvas cases it makes solid 333 × 150 exact
and halves the stroke residuals (stroke 333 × 150: 135 → 60 pixels; the
x = 22 column is fixed), but it breaks solid 64² (one pixel, 11 CV) and
stroke 200² (six pixels, up to 18 CV). Of the 1,530 pixels it changes, 1,423
were already within 1 CV either way, 98 move to the reference and 9 move
away; the nine are one-to-three splits with the lone sample at 1, 2 or 3,
the same features as some of the pixels it fixes, so the deciding input is
still missing.

On the gate references (`svg-gate-20261018-163629-8db61ae3`, Chromium
152; a repeat capture is byte-identical), the gate admits 358 of 383 cases
and all 358 are inside the 2D class. The 25 refused: 16 clips drawn into
the depth attachment (stretched oval, rotated rect, polygon and even-odd
path clips; max 11–55 CV on several of them), the nested-rect path (max 85
CV), `rx="auto"` (max 255 CV), five-stop gradients (in class, refused as
unmeasured beyond four) and a five-digit hex color (in class, outside the
accepted syntaxes). The reviewer's 94 experiment cases (`/tmp/rev-nesvg`)
give 90 admitted, all inside the class; the four refused are a five-stop
radial and the nested-groups document mixing an analytic rect with a
tessellated path (max 7 CV).

Not measured against Chromium and therefore routed: repeat and reflect
spreads, focal radial gradients, more than four stops, `crispEdges`,
arcs, `paint-order`, markers, `non-scaling-stroke`, miter joins,
path clips, nested and multi-shape clips, masks, patterns, embedded images,
other filters and blend modes.

### Corpus

All 34 corpus projects with SVG layers use the corpus asset
`corpus-art.svg` (gradients, a round-rect clip, a tessellated curve under
group opacity next to analytic rects and a circle, and a dashed stroke).
The gate routes it (strokes; the pass mix would route it too), so every one
of the 34 is skipped with the `SVG images` reason: 2 by `tests/corpus.rs`
(`layers/svg-scales`, `mattes/nested-and-animated`) and 32 by
`tests/text_corpus.rs`. Both tests pass on this branch (2026-10-18: 271 and
123 compared frames, none failed). On the
previous round, with the gate open, `layers/svg-scales` measured max 7 CV
on contiguous diagonal edge runs, `mattes/nested-and-animated` failed the
transparent float check at 1.1–2.0 CV, `mattes/modes` max 6 CV and
`compositing/blend-modes-alpha` 99.84% within 1 CV: those are the pass mix
and stroke differences above, now routed. The 34-project composition gate
is not met; it needs the multisampled-pass behaviour modeled and the
stroke and blur cases inside the class.

## Memory

`VectorCache` (pm-render) keeps level 1 rasters (`Rgba8Unorm`), level 2
paint targets (float), and level 2 SVG rasters keyed by a SHA-256 over every
input that changes the pixels plus the precision, and counts every byte:
`Renderer::vector_cache_bytes()` is the render cache category of the lease.

The key covers every pixel input (a level 2 key includes the world
transform), so animated content misses on every frame. The cache therefore
bounds itself while frames render, as today's raster cache does
(`evict()` in legacy/gl/raster.ts): least recently used entries go when it
exceeds its standing budget, by default today's 192 MiB and 4096 entries
(`Renderer::set_vector_cache_budget`). `render_into` starts each frame with
`begin_frame()`, which makes the previous frame's textures evictable and
brings the cache within budget, and every miss makes room before inserting.
A frame's own textures are never evicted while it records, so one frame whose
working set exceeds the budget still draws every layer and the cache returns
within budget at the next frame: the bound is budget plus one frame's working
set, independent of how many frames a work item (a whole export, playback)
renders. Dropping a texture that recorded commands still use is safe (wgpu
keeps it alive until they complete). Measured (`tests/vector.rs`): a rotating
level 2 star, 556 KB per frame, budget 1.67 MB, peaks at 1.14 MB over 40
frames without any `trim`.

`Renderer::trim(bytes)` (lease pressure, pm-node trims to 64 MB between work
items) trims the target pool first and gives the vector cache what the pool
leaves, so together they hold at most `bytes`. Switching the coverage backend
drops the cache. Coverage atlas pages, paint globals, and vertex buffers live
only while a scene renders and are not counted; Vello's internal buffer pool
is not counted either.
