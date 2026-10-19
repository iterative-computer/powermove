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
| `image` (SVG) | Chromium's own raster of today's bucket, composited natively (decision S1, below) | Vectors or resvg at output resolution |
| SVG with `<text>` | The same (Chromium draws the text too) | Unsupported (`Feature::SvgImage`; SVG text goes through `pm-text`) |

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
5. **Rasterization** (`msaa.rs`): vertices through the viewport round trip
   (`tess::Viewport`, below), snapped to 1/256 px (round half
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
five cases that missed "no pixel off by more than 4" were pinned in
`tests/canvas2d.rs`; since the viewport round trip (below) the radius-24 one,
`shape-corpus-rect-r24-s1`, is bit-exact and four remain (Residuals).

Corpus (`pm-render/tests/corpus.rs`): `shapes/primitives`,
`shapes/editable-paths`, and `shapes/svg-import-paths` are 100.000% exact at
every sample, opaque and transparent float frames. `composition/metadata`
(the radius-24 rectangle, rotated) had 2 to 6 pixels with one flipped sample
(max 43), pinned in `KNOWN_RESIDUALS`; with the viewport round trip it is
bit-exact at every sample and the pin is gone.

### Viewport round trip

Every tessellated vertex goes from device coordinates to normalized device
coordinates and back to window coordinates before it is snapped
(`tess::Viewport`): `ndc = (x − w/2) / (w/2)`, `window = ndc · w/2 + w/2`, each
step rounded to `f32`, over the target's dimensions. The round trip moves a vertex
by at most a few ulps of `w/2`, which only matters to an MSAA sample that lies
that close to a tessellated edge.

The model is empirical. It was found with the residual that remained at the
corners of radius-24 rounded rectangles (one MSAA sample, two pixels per
rectangle, max 64): of twenty evaluation orders of the round trip (fused or
unfused `x · 2/w − 1`, the division above, the four viewport expressions) over
the target's exact dimensions and over approx-fit attachment sizes (Skia's
`GetApproxSize`, powers of two, multiples of 4 to 256), it is the only one that

- makes the radius-24 rectangles of the corpus (108 × 108 canvas) and of v1
  `space/groups-25d` (138 × 148) bit-exact, the two cases it was chosen on;
- predicts the four canvas sizes captured after it was chosen (`viewport-rect-*`
  at 98 × 178, 208 × 128, 264 × 264, 341 × 219: each a two-pixel residual
  without it, bit-exact with it);
- keeps the 68 × 68 radius-24 rectangle of the corpus's `precomps/cycle` and the
  529 × 303 raster of `shapes/editable-paths` exact (every approx-fit variant
  breaks one of them), and changes no other captured case.

The radius-24 residual is closed by it: `shape-corpus-rect-r24-s1`, the
`shape-groups25d-card-*` cards, `composition/metadata` in the corpus (formerly
pinned in `KNOWN_RESIDUALS`, now bit-exact at every sample), and level 1 shapes
on 2.5D planes, which are no longer routed.

### Residuals

On `canvas-20261018-220349-9a95e00c` (529 cases): 503 bit-exact, 22 within 1
code value (analytic renderer rounding), and four pinned in `tests/canvas2d.rs`:
`shape-ellipse-big-s2.75` (3 px), `shape-random153-s3.75` (1 px),
`paths-badge-rotated-scaled-s3` (2 px), `diag-ellipse-380` (2 px). Each is a
single MSAA sample within 0.07–0.46/256 px of a tessellated edge that neither the
curve evaluation nor the viewport round trip reproduces. They are left as
measured.

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
| SVG rect, rotated 23° | 54.7 dB, max 0.005 | Chromium's bucket raster (decision S1) |

Vello on the GPU and `vello_cpu` agree on a stroked star and a rounded rect
except one pixel, by 1/255.

## SVG images

`compat` level 1 and level 2 SVG images take different paths. Level 1 (every
SVG image in an existing project) is decision S1: Chromium rasterizes it, as
today. Level 2 is native.

### Level 1: Chromium's raster of today's bucket (decision S1)

Three rounds of emulating Blink plus Graphite for SVG `<img>` (branch
`feat/ne-svg`, kept as a reference, not merged; the history is in "Superseded:
the emulation rounds" below) reached 11 to 18 of the 27 reference cases: Blink's
SVG and CSS parsing semantics differ from usvg too broadly to emulate. Today a
level 1 SVG image is:

1. `PM.rasterSvgAsset(asset, w, h)` (legacy/gl/raster.ts): the asset's decoded
   `<img>` (an `image/svg+xml` blob of the file's bytes) drawn with
   `drawImage(img, 0, 0, w, h)` into a cleared canvas of the bucket
   `svgRasterDimensions(sw, sh, bw, bh, scaledWorld(L, T, W, H))` picks;
2. uploaded by the compositor's `texFor` (`UNPACK_PREMULTIPLY_ALPHA_WEBGL`,
   no flip) and drawn as the image quad (box, `fit` UVs, anchor at the box
   center, linear filtering, clamp).

With "Use New Engine" on, step 1 still runs in Chromium, in the renderer that
owns the document; the engine does step 2.

**The plan** (`pm-render/src/plan.rs`): a level 1 SVG image plans today's image
quad over `VectorSource::SvgImage { key, asset, path }`, where the key is the
SHA-256 of the file's bytes (`vector::svg::content_hash`, cached by path and
stamp) and today's bucket (`pm_vector::render::svg::svg_raster_dimensions` over
`scaled_world`). Only the content and the bucket decide the pixels, so a raster
serves every layer, frame, and window that draws that bucket of those bytes. A
document Chromium is measured to show as a broken `<img>` draws nothing and
asks for nothing (`pm_media::image::svg::broken_image_file`); offline media is
a named layer failure. `Feature::SvgImage` is never listed for level 1.

**The store** (`pm-render/src/vector/svg_rasters.rs`, inside `VectorCache`):
`Renderer::put_svg_raster(key, rgba)` keeps the premultiplied RGBA8 rows (192
MiB, least recently used out, counted in `vector_cache_bytes` and trimmed with
the cache; the working set, every raster stored or read since the latest frame
began, is never evicted, as today's raster cache always keeps the raster it
just drew: one bucket over the budget on its own, an 8192 × 6200 raster, or
several large rasters one export frame needs together stay until a later frame
no longer reads them), uploaded on first use as the `Rgba8Unorm` texture today's canvas
upload makes, the same format level 1 shape rasters use (exact on the corpus).
A replaced or relinked file hashes to another content, so an old raster is
never drawn for it; it ages out. An engine restart or a GPU loss starts empty,
and the rasters are asked for again (`e2e/engine-svg.spec.ts`: a replace, its
undo and redo, and a killed engine each draw today's pixels again).

A replace changes the asset's record, and the engine resolves the asset again
through main's media bindings at once. The window used to bind 250 ms later,
so the engine resolved the new record to the old file and kept drawing it
(any media kind, not only SVG). `installHostMediaStore` now binds whatever a
document batch changed before the batch reaches the engine: it subscribes to
the mirror's batches ahead of every engine session (bootstrap installs the
media store before the engine connection), and the bind message is sent
before the batch is posted. Only batches that change the assets, the project
id, or the whole document (`PatchBatch.assets`, set by the barrier) look at
the bindings; a keystroke or a drag costs nothing. The order relies on main
reading the window's bind IPC before the engine's later resolve request
reaches it through host services (it does in every e2e run; it is not
enforced by construction).

The engine never drops a replaced file's rasters itself (keys are content
hashes, so they cannot be drawn for the new bytes); they leave the store by
least recent use.

**A miss** is recorded per frame (`SvgRasterNeed`: the request, the file, the
first layer and composition that drew it):

- *Viewer preview* (`Composer` scopes `Renderer::svg_waiting`): the frame
  is not submitted until every exact bucket is present. Missing buckets are
  requested during render-ahead over the viewer session's port; the view
  retains that frame's schedule position while the port answers, allowing
  other views and audio to continue. The prior frame stays on screen rather
  than publishing a new frame with the old bucket. An edit, view change,
  transport epoch, or new surface pool cancels the obsolete wait. Completion
  wakes the view; unanswered requests retain the existing bounded retry
  policy. `SvgMissPolicy::Substitute` remains available to diagnostic callers
  but the app viewer does not use it.
- *Exports, agent frames, and snapshots* (`SvgMissPolicy::Wait`: an export
  request, `Renderer::set_svg_wait` on a renderer the caller owns, or
  `Renderer::svg_waiting` for a snapshot on the viewer's renderer, a guard
  that gives the viewer its policy back on every return and unwind; a
  snapshot that failed on its fonts used to leave the viewer waiting, so its
  misses failed instead of asking the window, tested in pm-node
  `a_snapshot_that_fails_on_fonts_leaves_the_viewer_asking_for_svg_rasters`):
  nothing is ever drawn in place of a raster. The frame fails with `RenderError::SvgRastersPending` and the
  targets are untouched; with a source attached (`Renderer::set_svg_source`)
  the renderer fetches every missing raster, blocking, and draws the frame
  again; a source that cannot fails the frame with its words
  (`RenderError::SvgRaster`).

**The renderer side** (`apps/desktop/src/renderer/src/engine-connection/svg-rasters.ts`):
`SvgRasterCanvas` is `rasterSvgAsset` statement for statement, then `texFor`'s
upload into a WebGL2 texture with the same pixel-store flags, read back with
`readPixels` (the 2D canvas is never read with `getImageData`, which
unpremultiplies and can move it off the GPU). `SvgRasters` draws a request only
from the window's own asset whose decoded bytes (its object URL) hash to the
engine's content (`stale` otherwise; hashes are taken ahead on every `assets`
change so a synchronous snapshot can check one), and answers:

- the viewer's asks, in row bands on the session's port (`svgPixels`, or
  `svgFailed` with `missing`, `stale`, `decode`, `size`, or `unavailable`),
  once per key in flight, four draws at a time;
- snapshots: `needsRenderer.svg` names the missing buckets; the transport draws
  them on the calling thread, posts them on the window's session, and asks
  again naming `svg:<content>:<w>x<h>` in `uploaded`; the render thread waits
  for those bands. The rasters are shared by every session on the render
  thread. A synchronous snapshot main serves from its own session for the
  window (at boot, before the window's session opens, or while a staged
  resync is in flight) carries the rasters with its next ask instead
  (`svgPixels`, as shader fallback pixels travel), and main posts them in
  bands on its session before asking again: the window may have no session
  to post on. A snapshot in the same task as an import or a replace meets an asset whose
  hash `prime` has not finished: the window then reads the `<img>`'s bytes
  with a synchronous `XMLHttpRequest` of its object URL (the only synchronous
  read a window has) and hashes them on the calling thread
  (`sha256HexSync`, tested equal to `crypto.subtle`), so the snapshot draws
  as today's compositor does instead of failing; the stale check still holds;
- the job engine (`HostSvgRasters`, pm-node `jobs/svg.rs`): an export or agent
  frame asks host services for `compatFrame { source: svgImage, svg: { key,
  asset } }` naming the document (today's `documentId(PM.proj.id)`); main asks
  every editor window (`compat-frames.ts`), the one with the document open
  answers through `provideCompat('svgImage', …)`. With no window, or only
  windows with other documents open, it fails with "SVG images are drawn by
  the editor window that has this project open; open the project in a window
  to export it." Nothing else is drawn.

**Hosts without Chromium.** `pm-cli` sets `SvgRasterizer::Resvg`: resvg draws
each bucket (`SvgDocument::raster`, the user space stretched over the bucket as
`drawImage` does) and every draw is counted; `render` and `scope` print
`pm-cli: notice (approximate SVG): N level 1 SVG image draw(s) were rasterized
by resvg, not Chromium; …` and the render summary carries `approximateSvg`.
`pm-cli parity render --svg-rasters DIR` draws Chromium's captured rasters
instead (`RasterDir`, below). The hidden MCP host renders in Playwright
Chromium today (the legacy path there is this same code); the native hidden
host of section 22 does not exist yet, and its on-demand Chromium is where
`svg-rasters.ts` runs for it (it needs no Electron: `decodeSvg` and
`SvgRasterCanvas` work in any Chromium page).

**Captured rasters for headless comparison.** `RasterDir` reads
`<content>-<w>x<h>.rgba` files and lists the ones it lacks in the directory's
`needs.json`; `engine/crates/pm-render/tools/svg-rasters/run.ts` draws them in
the app's Electron with the app's module (`--needs DIR`), and compares the 27
SVG cases of the canvas-ref sweep with the legacy capture (`--cases`).

### Level 1 measured

Measured 2026-10-18 on this Mac (Electron 44, Chromium 152.0.7977.54, Apple
M4 Max), the machine shared with other lanes.

- **The rasters.** `tools/svg-rasters/run.ts --cases`: the module draws all 27
  SVG cases of the canvas-ref sweep (solid, stroke, opacity, gradient, clip,
  even-odd, blur, text, transform at 64², 200², 333 × 150) byte-identical to
  the legacy capture `canvas-20261018-220349-9a95e00c` (27 of 27).
- **Blink's SVG semantics.** The same tool over `feat/ne-svg`'s 383 gate
  documents (colors, numbers, invalid values, `display`, duplicate ids, the
  root clip, `points` errors, `xml-stylesheet`, sizes and viewBoxes, at three
  buckets each; `--cases-file` with that branch's `svg-gate-cases.json`)
  against their Chromium references `svg-gate-20261018-163629-8db61ae3`: 383
  of 383 byte-identical. These are the documents the emulation could not
  match; Chromium drawing them as today needs no emulation.
- **The corpus, headless, after merging the integration branch of
  2026-10-19** (`tests/svg_corpus.rs`, same capture and rasters): all 34
  projects planned and 33 rendered (105 frames, 9,640,927 SVG pixels; 5
  frames show none), every SVG pixel exact in all 33, including the three
  projects that differed by 1 CV below and `masks/path`, `space/layers-25d`,
  and `layers/extension-missing`, which the composite, masks, 2.5D, and
  extension lanes now draw. The one failure is `effects/extension-typewriter`
  headless only: the corpus's fixture extension defines `typewriter` in the
  app, and the headless run has no definition for it.
- **The corpus, first run** (`tests/svg_corpus.rs`, capture `v1-20261012-230504-ae47a3b3`,
  the 6 buckets the corpus draws captured with the module into
  `svg-rasters-20261018`): all 34 projects with a level 1 SVG image were
  planned with the engine's fonts and the built-in definitions; 31 rendered
  (99 frames, 9,439,743 SVG pixels; 5 frames show no SVG pixel: a solo'd-out
  layer, transition frames). The SVG pixels are exact in 87 of the 99 frames
  and in 28 of the 31 projects, including `layers/svg-scales` (contain at
  25 to 400% and a zoom) and `mattes/nested-and-animated` (no text). The
  other three projects differ by at most 1 CV where something other than the
  raster changes the SVG's pixels, the same rasters being exact in the
  other projects: `compositing/blend-modes-alpha` (inside the blend-mode
  tiles over transparency; the composite lane's appendix A drift, already a
  decision), `masks/parametric` (along the feathered mask edge only), and
  `compositing/adjustment` (along the adjustment layer's blurred edge; its
  non-SVG pixels differ by 1 CV too). Side by side (legacy, engine, the
  differing pixels in red), outside the repository:
  `~/Library/Application Support/Powermove Engine Corpus/svg-rasters-20261018/evidence/*-legacy-engine-diff.png`. Three projects do not
  render for reasons outside SVG: `masks/path` (path masks), `space/layers-25d`
  (2.5D layers), `effects/extension-typewriter` (no definition for the
  `typewriter` effect).

- **The corpus in the app, 2026-10-19** (`tools/svg-rasters/app-corpus.ts`,
  after merging the integration branch, same capture): 34 projects, 108
  frames, 9,936,458 SVG pixels (5 frames show none); today's compositor live
  is 100% exact in every frame, and the engine's SVG pixels are exact in
  **33 of 34 projects**, including the six of the first run below (the 1 CV
  composite and mask residuals, `masks/path`, and `space/layers-25d`, no
  longer routed). The one failure is `layers/extension-missing`: the
  snapshot fails, "no definition for layer com.example.absent-layer"
  (L008), where today draws the project without it (the extension
  compatibility lane; headless, the same project renders exact). The whole
  frame still differs where text is (the text lane's residuals, not SVG
  pixels).
- **The corpus in the app, first run** (`tools/svg-rasters/app-corpus.ts`, 2026-10-18,
  after the bridge lane's merge): the 34 projects opened in the built app as
  the capture opened them, "Use New Engine" on, every sample drawn by
  `PM.EngineRender.snapshotSync` (the engine with this window's rasters) and
  judged against the capture `v1-20261012-230504-ae47a3b3` with the SVG mask
  above (105 frames, 9,790,300 SVG pixels; 5 frames show none). Today's
  compositor drawing the same frames live is 100% exact in all of them, so
  the capture still is today. The SVG pixels are exact in **28 of 34
  projects** (including `effects/extension-typewriter`, which renders in the
  app with its fixture extension). The rest are not the rasters:
  - `compositing/adjustment`, `compositing/blend-modes-alpha`,
    `masks/parametric`: the same at most 1 CV as the headless run, along the
    adjustment layer's blurred edge, inside the blend-mode tiles over
    transparency, and along the feathered mask edge (the composite and mask
    paths over SVG content);
  - `masks/path`: the snapshot fails, "path masks";
  - `layers/extension-missing`: the snapshot fails, "no definition for layer
    com.example.absent-layer", where today draws the project without it;
  - `space/layers-25d`: SVG pixels exact, but the composition is routed
    ("2.5D layers").
  Side by side (capture, engine, differing pixels in red), outside the
  repository: `svg-rasters-20261018/evidence/app-corpus-20261018/`.
- **e2e** (`e2e/engine-svg.spec.ts`, flag on, rerun 2026-10-19): synchronous
  snapshots equal today's compositor byte for byte at 0, 0.75, and 1.5 s
  (three layers: contain, cover rotated at 230%, stretch with animated scale
  and opacity); a replace draws the new bytes at once (equal to today's, and
  its undo and redo too); a killed engine asks for the rasters again and
  draws the same frames. `native-svg-broken.spec.ts`: corrupt documents paint
  nothing and the four documents roxmltree rejects but Chromium paints export
  through the job engine equal to today's (max 0). `import-svg.spec.ts`
  passes with the flag on and off (with the flag on its high-zoom check reads
  the engine's view instead of the legacy compositor's cropped viewport).
- **The viewer** (`the viewer asks for the buckets today's viewer
  rasterizes…`). A presented frame is an IOSurface in main's layer and
  cannot be read back, so the viewer is compared by the buckets it draws:
  today's viewer frames (the `rasterSvgAsset` calls inside `PM.GL.render`;
  thumbnails and captures draw their own sizes and are kept apart) against
  the asks of a restarted engine (holding no raster) in the same viewer
  state (stage 748 × 452 CSS px, DPR 2, quality 1, checked equal in both
  launches). At fit (114.4%: 1160 × 653, 600 × 338, 400 × 225) and at 237%
  with a cropped preview viewport and a pan (2360 × 1328, 1200 × 675, 760
  × 428) the buckets are today's. **At 61% they are not**: today 520 × 293,
  280 × 158, 160 × 90; the engine 600 × 338, 320 × 180, 200 × 113. Not the
  rasters: today's viewer draws at most one output pixel per composition
  pixel when it does not crop (`previewRenderSize` in
  `extensions/viewer/viewer.ts`: `min(1, zoom × DPR) × quality`, the canvas
  scaled up by the browser; also above 100% for projects whose layers make
  the cropped viewport unsafe: masks, mattes, transitions, 3D, adjustment,
  shader, extension and precomp layers, effects not marked viewport safe),
  while the engine's viewer always renders at zoom × DPR
  (`pm-node/src/present/compose.rs`, `placement`). Between 50% and 100% at
  DPR 2 (and when zoomed into those projects) every layer, not only SVG
  images, is drawn sharper than today. The SVG buckets follow the output
  scale, so they are right for the scale the engine draws at; the scale is
  the presenter's. Side by side (today's frame at its render size scaled
  as the browser shows it, the engine's snapshot at its viewer's output
  size, differing pixels in red; a simulation of both presented frames,
  which cannot be read back): 68,829 of 342,859 pixels differ at 781 × 439,
  `svg-rasters-20261018/evidence/viewer-zoom-20261019/`.
- **Playback of a scale animation** (`playback of a scale animation…`, the
  Stretch layer at 60% → 310% over 2 s, idle machine): 62 presented frames,
  17 stand-ins (each a frame whose bucket the window had not yet drawn: the
  previous bucket scaled), 17 asks, the slowest answered in 12 ms; the
  paused frame afterwards is exact. Today draws every one of those frames
  at its own bucket.
- **Exported stills are not yet today's** (not the rasters): the job
  engine's still of the e2e project differs from today's exporter's in
  56,037 channel values by 1 CV, and a PNG control with no SVG in it differs
  the same way (59,924 by 1 CV; 56,037 and 59,892 in the rerun of
  2026-10-19). The SVG still equals the interactive
  engine's snapshot of the same frame and today's `renderFrameTo`; the job
  engine's still alone renders at 32-bit and dithers 8-bit PNG
  (`export.md`), which today's still does not. Side by side:
  `svg-rasters-20261018/evidence/still-export-20261018/`. Owner: export.
  The same control also shows the interactive engine sampling the upscaled
  PNG 1 CV away from today in 6,484 channel values
  (`png-control-interactive-legacy-engine-diff.png`), outside SVG.

### Level 1: what remains

- The hidden MCP host's on-demand Chromium: the native hidden host of
  section 22 does not exist yet; when it does, it loads `svg-rasters.ts` in
  its Chromium page (no Electron needed) and answers `compatFrame
  svgImage` as an editor window does.
- Exported stills (export lane): 32-bit rendering and 8-bit PNG dither
  differ from today's still by 1 CV (above).
- `layers/extension-missing` in the app (extension compatibility lane):
  the snapshot fails where today draws the project without the missing
  layer.
- The viewer's render scale below fit and for crop-unsafe projects (above):
  the presenter lane's `placement` needs today's `previewRenderSize` and
  `previewRenderViewport` rules (the cap at one pixel per composition pixel,
  the crop's safety plan, the quality and explicit resolution settings),
  and the browser's upscale of the smaller frame. Until then the viewer
  differs from today there for every layer kind.
- Preview stand-ins during playback (above: 17 of 62 frames of a scale
  animation): prefetching the buckets of the frames ahead would remove
  them; a decision for the orchestrator.
- Bucket input: the engine sizes the bucket from the asset record's `w` and
  `h`, today from the live decode's `naturalWidth` and `naturalHeight`. They
  are the same decode at import; they would differ only if Chromium's
  natural size for a file changed after the project was saved (an SVG
  without a size, across Chromium versions). Not tested.
- Level 1 broken documents are still decided by an emulation
  (`broken_image_file`); the window's own decode (its `decode` failure)
  could decide instead.
- Preview: a viewer miss shows the content's previous bucket, or nothing for
  a new or replaced file, until the window's bands arrive. Measured in the
  e2e (`a bucket change in the viewer shows a stand-in…`, 10 scale edits of
  one layer, idle machine): one stand-in frame, then the exact frame 4 to
  11 ms after the edit, in 9 of 10 (the tenth reused a bucket already held:
  no stand-in); six buckets asked at once at open took 20.9 to 31.1 ms. Not
  measured under load; today's
  asynchronous `<img>` decode likewise shows nothing until it resolves, and
  today's synchronous canvas redraw on a bucket change has no such frame.
  Paused frames and every snapshot and export are exact.

### Parse depth (level 2)

usvg, roxmltree, resvg's renderer, and the tree's drop recurse per element
level; engine threads have 2 MiB stacks. `SvgDocument::parse` scans the nesting
first without recursion (`pm_vector::render::svg_depth::element_depth`),
refuses a document deeper than Blink's 5,000-level XML limit before anything
recursive runs, and runs the parse, resvg's render, the level 2 scene walk, and
the drop on a scoped thread with 16 MiB plus 64 KiB per level when the document
is deeper than 128 levels (ported from `feat/ne-svg`, only this part). Measured
in a debug build on a 2 MiB thread: a 1,000-level document overflowed inline;
it now parses, draws, and drops there. usvg itself refuses more than 1,024
levels (`NodesLimitReached`), so a level 2 document between 1,025 and 5,000
levels is a named layer failure where Chromium would draw it; no existing
project has level 2 content.

### Level 2

An SVG of solid fills and strokes in plain groups that stays inside its box is
a coverage scene (vectors at output resolution). Anything else (gradients,
patterns, clips, masks, filters, group opacity, blend modes, embedded images,
`crispEdges`) or content cropped by `fit: cover` or the viewport is drawn by
resvg into the device rectangle with the box and viewport as an antialiased
clip. Where today's quad would clamp the texture's edge texels over a `contain`
box, level 2 draws nothing. SVGs with `<text>` are `Feature::SvgImage` at level
2 until the pm-text diagnostic meets SVG's 2D parity class. As in an `<img>`,
only `data:` URLs resolve. `pm_media::image::svg::utf8_source` normalizes
declared encodings (`media.md`).

### Superseded: the emulation rounds

Before decision S1, level 1 SVG images were routed (`Feature::SvgImage`) while
resvg, and then a replay of Blink's paint record through a Graphite emulation
(`SvgDocument::raster_compat`, still in pm-vector as an unenabled diagnostic,
and `svg_compat_compare`), were measured against Chromium. resvg's bucket
rasters differed at edges by one or two MSAA samples (max 64 to 116 CV;
`tests/svg.rs` pins those measurements); the replay reached 15 of the 27
retained references here and 11 to 18 on `feat/ne-svg`. Neither draws in
production.

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

### Perf3: no SVG bucket stand-ins (2026-10-19)

The historical preview stand-in counts above are superseded for the app viewer.
`Composer` now uses the same scoped exact-raster policy as snapshots. The
presenter asks for the missing raster without advancing the render-ahead frame;
it renders that frame when the bands complete. Shader fallback requests also
retain their pending frame rather than advancing past unanswered pixels.

The two targeted `engine-svg.spec.ts` regressions passed under the Electron
lock: ten scale edits with zero stand-ins and exact reference pixels; the
60%→310% scale animation with 63 presented frames, 17 bucket requests and
zero stand-ins, followed by the exact 1.5-second frame (zero differing pixels).
The regression identifies an edited frame by its document revision, so a
previously cached bucket also completes without requiring another upload.
This is functional/pixel evidence; no paired performance pass is claimed while
the display is asleep. The final release addon built successfully.
