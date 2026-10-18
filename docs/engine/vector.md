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
`<img>`, only `data:` URLs resolve (no file or network loads). Text is parsed
for the pm-text diagnostic below; the production SVG text gate remains.

- **Level 1** (today's buckets): `svg_raster_dimensions` ports
  `svgRasterDimensions`, and `raster` draws the bucket as `drawImage(img, 0,
  0, w, h)` does, the user space stretched over it (measured: stretching
  matches the captures; `meet` does not). resvg is not Chromium, which paints
  SVG shapes through Skia's analytic and multisampled renderers:

  | SVG (64², 200², 333 × 150) | Pixels off by more than 1 | Mean difference |
  |---|---|---|
  | Solid shapes | 1.2–4.0% | 0.20–0.81 |
  | Strokes | 2.8–12.9% | 0.69–2.87 |
  | Group and fill opacity | 0.7–2.7% | 0.14–0.45 |
  | Gradients | 2.2–5.4% | 0.63–0.72 |
  | Clip path | 1.0–3.4% | 0.18–0.42 |
  | Even-odd | 1.4–4.8% | 0.36–1.26 |
  | Blur filter | 12.1–18.5% (max 8) | 0.76–0.83 |
  | Transform, dashes | 1.3–5.0% | 0.27–1.11 |

  Edges differ by up to one or two MSAA samples (max 64–116). That is
  outside the 2D class, so level 1 SVG images stay routed
  (`Feature::SvgImage`); `tests/svg.rs` pins the measurements. Only a
  document Chromium is measured to show as a broken `<img>` paints empty
  (`media.md`, "Broken level-1 SVG documents"); roxmltree errors libxml2
  tolerates (external DTD subsets, parameter and external entities) stay
  gated. Unsupported usvg paint is not corruption. Declared ISO-8859-1 and UTF-16
  sources remain valid. Offline/unreadable files are named layer failures.
  `pm_media::image::svg::utf8_source` normalizes declared encodings
  (`media.md`).
- **Level 2**: an SVG of solid fills and strokes in plain groups that stays
  inside its box is a coverage scene (vectors at output resolution). Anything
  else (gradients, patterns, clips, masks, filters, group opacity, blend
  modes, embedded images, `crispEdges`) or content cropped by `fit: cover` or
  the viewport is drawn by resvg into the device rectangle with the box and
  viewport as an antialiased clip. Where today's quad would clamp the
  texture's edge texels over a `contain` box, level 2 draws nothing.
- **Text**: SVGs with `<text>` stay routed at every level until the pm-text
  diagnostic below meets SVG’s 2D parity class.

### Chromium SVG paint-record replay (media closure round 2)

Chromium **152.0.7977.54** is the version in the canvas reference manifest.
Its [SVGImage::DrawInternal](https://chromium.googlesource.com/chromium/src/+/152.0.7977.54/third_party/blink/renderer/core/svg/graphics/svg_image.cc)
saves the Canvas2D canvas, applies the destination clip and source-to-destination
matrix, and replays `PaintRecordForCurrentFrame` with `drawPicture`. It does
not pre-rasterize SVG with Skia's CPU scan converter. With Graphite enabled
in the capture, that means the same analytic and MSAA paint pipeline the
vector lane emulates. Skia is pinned by Chromium's DEPS to
`b6d106297ff9ef2ff8094033695d045e87775581`.

`SvgDocument::raster_compat` is a separate, unenabled bucket diagnostic.
It associates original element geometry with the resolved usvg tree so oval,
rounded-rectangle and conic SkPath identities survive parsing. The Canvas2D
context now records gradient paint, fill rules, analytic rounded rectangles,
clip unions/intersections and opacity/linear-RGB blur layers. SVG text uses
`raster_compat_with_text` and a caller-owned `pm_text::Compat` resolver; usvg
text outlines are never used by that path. Unsupported text positioning,
paint servers, masks, filters and embedded bitmap records return explicit
errors. Original parsing for the existing level-2 scene is unchanged.

The diagnostic compares **all 27** retained source references and writes
side-by-sides outside the repository. Invoke `svg_compat_compare` with
`PM_MEDIA_EVIDENCE` and `--expect-parity` to enforce the 2D bar. The current
result is **15/27**, so `SvgImage` is retained: opacity, even-odd and transform
cases pass at all three sizes; solid passes at 64²/200²; stroke at 200²; clip
at 64²/200²; blur at 64². Remaining maxima are solid 44, stroke 19, gradient
3 (too many pixels over 1), non-square clip 103, larger blur 7, and pm-text
text 157 CV. These are blockers, not accepted differences. The 34-project
composition gate cannot pass while its source raster fails. No production
level-1 bucket/cache hook is enabled from these measurements.

### SVG/still closure audit

The unchanged 27-case Chromium upload comparison still passes only 15 cases.
The blockers remain solid 333×150 (44 CV), strokes (15/19), gradients
(2–3 with too many pixels above 1), non-square clipping (103), larger blur
(6/7), and SVG text (120–157). Interpolating fragment attributes from
unsnapped vertex positions did not resolve the failures; using a tessellated
non-square clip lowered its maximum to 84 but still failed the class. Neither
experiment is enabled. The `--expect-parity` diagnostic fails with the
original 2D thresholds, and all 27 side-by-sides remain external evidence.

Masks, patterns, embedded images and non-Gaussian filter paint records are
still explicit walker errors. Gaussian blur intermediates and pm-text glyph
mask rasterization also remain unfinished. Consequently the 34-project SVG
composition deliverable is unproven and `Feature::SvgImage` remains. A
successful parse or the existing level-2 resvg output is not a level-1 proof.

Corrupt level-1 image documents are handled independently: XML/image-root
errors whose libxml2 outcome was measured paint the empty image Chromium
shows, with no SVG route or named layer failure; documents libxml2 accepts
keep the SVG route. Level 2 parsing (pm-vector's usvg document) still
recurses on the caller's stack, so a level 2 SVG nested thousands of
elements deep can still overflow a 2 MiB engine thread (reported by the
lane review; not changed here). Five persisted corrupt documents pass the actual native app
export path exactly. This does not remove the gate on any valid SVG paint
record; usvg's unsupported features are not corruption. The flagged editable
SVG import regression also passes; it checks native-route completion before
undo, and retains its exported-player pixel and geometry assertions.

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
