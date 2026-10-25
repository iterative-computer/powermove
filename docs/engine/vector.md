# Vector rendering (pm-vector, pm-render::vector)

Shape layers, editable paths, solids, path masks and SVG images. Geometry (shape outlines,
corner smoothing, path evaluation, trims, repeaters, mask outlines) is pm-vector's geometry
side: exact ports of the authored formulas, bit-identical native and wasm
(`conformance`, `tests/legacy_geometry.rs`). This note covers how that geometry becomes
pixels.

## Invariants

- Authored intent is exact: outlines, joins, caps, miter limits, fill rules, corner radii,
  smoothing, feather amounts, colours (Canvas2D colour-string semantics, invalid strings
  keep the default style).
- Shapes and paths are drawn at output resolution with exact or analytic area coverage,
  painted in float at the project's working precision. Their cache has no quantized
  density or vector-content resample.
- The preview and the export draw the same pixels.

## Shapes and editable paths

A shape layer's content becomes a `CoverageScene` (`pm_vector::render::scene`): ordered fill
and stroke items in device pixels, each with one solid paint, under the layer's world
transform (with the size anchor). Strokes are expanded in local space by kurbo (Canvas2D
strokes under the current transform).

- **Outline.** Each item's outline is flattened within the device-space tolerance and its fill rule resolved
  into disjoint contours (`CoverageItem::device_outline`, i_overlay): area coverage sums
  winding over a pixel before the rule, so overlapping pieces (a stroke's inner join
  through its pivot, a self-intersecting fill) would cover too much where they meet an
  edge.
- **Reuse.** A bounded cache retains resolved local outlines at an upward scale
  bucket that bounds the transform's largest singular value. Placement streams
  through coverage without copying the outline; the device-space flattening
  tolerance remains unchanged. CPU arrays, coverage buffers and atlas pages are
  reused between submitted scenes and trimmed after large one-off work.
- **Coverage.** `pm_render::coverage` (shared with path masks) computes each item's
  exact area coverage on the GPU, every item of a scene in one batch, into its cell of
  an `r32float` atlas (`AtlasLayout`, 1 px gutters): exact to the flattening and its
  1/16384 px fixed point.
- **Paint.** The engine's own pass (`vector/cache.rs`, `PAINT_WGSL`), in the same
  command buffer, composites `paint × coverage` item by item, premultiplied source-over,
  into a target at the frame's precision, drawn 1:1 into the layer.
- **2.5D planes.** The scene is flattened in layer space at the plane's densest density
  and projected vertex by vertex (`space25d::local`), then covered at output resolution.
- **Extension rasters.** `Renderer::layer_raster` (`PM.raster` through the render
  transport) keeps today's canvas size, anchor and selection, and paints the same scene
  onto that canvas.
- **Cache.** `VectorCache` keys a scene by everything that changes its pixels (the world
  transform included), keeps 192 MiB / 4,096 entries least recently used, never evicts the
  frame's working set, and counts every byte in the lease's render category.

## Solids

A solid is the composite pass's quad (`fs_solid`, `shaders/composite.wgsl`): its box `d.w ×
d.h` (`|| W, H`) with `radius` circular corners (at most half of either side; 0, negative
or missing is square). The quad grows by 1.5 device pixels; each fragment maps its pixel
square into layer units through the screen derivatives, clips it against the box and
subtracts, at each corner it reaches, the corner square's part outside the circle (polygon
∩ disk, closed form). Coverage is exact for affine placement and first order on a 2.5D
plane; interior pixels take 1 at once. No texture, no cache.

`radius` is honoured on every solid: the inspector edits and animates it (`c.radius`,
`ContentSection.svelte`) and the agent commands accept it (decision SOLID-RADIUS-1).

## Text ownership

Text shares the exact area coverage (`pm_render::coverage`) with shapes, masks and
SVG; its raster and blur are described in [text-render.md](text-render.md). The
Canvas2D/Skia emulation (`pm-vector` `canvas2d`) is gone.

## Gates

`pm-render/tests/vector_gate.rs` (G-V1) and the corpus (`tests/corpus.rs`, G-C2, through
`support::reference`) judge shapes and paths against exact coverage (16 × 16 cells per
pixel; strokes as the region their joins, caps, miter limits and dashes define,
`pm_reference::stroke`; numerical duplicate curve points are collapsed before coverage):
interiors within 1 CV of today, edges at least as close as today and per pixel within the
previous bound, geometry (alpha area and centroid) within 1e-3. Solids are judged in the
corpus and in `tests/vector.rs`. Geometry in the corpus uses a render-only copy
of the project at 32-bit depth, with the actual target format asserted, so half-float
rounding of coverage and motion accumulation is not mistaken for shifted geometry.
The default `corpus_own_layers_reference_gate` additionally isolates authored
shape/solid content, masks and shutter samples before later-lane compositing.
Effective 2.5D layers have a local source proof before projection. Full-frame
pending rows still measure and ratchet their L4/L6 context; isolation cannot
exempt a full-frame check. Dash partition in the f64 stroke reference integrates
Bézier arclength independently of the production dash implementation.

## Path masks

When any enabled mask has a path, all enabled masks of that layer use the shared
GPU area-coverage pass. Each outline is feathered with the exact separable Gaussian
at `σ = max(0, feather) × |world x| / 2`, with transparent canvas edges. Add and
subtract compose in authored order; subtract-only stacks start covered. Coverage
stays in float until the final `r16float` store. Work is bounded by each outline
and its feather.
See [composite.md](composite.md#path-masks) and `tests/path_masks.rs` (G-V2).

## SVG images

Paused frames, exports and captures paint every SVG image at output resolution.
The layer's box, fit window, anchor and world transform determine the
device-space drawing. There is no editor-window raster request. An SVG project
can render and export headlessly, including through `pm-cli`.

usvg parses the document and lays out SVG text. System fonts load lazily, once per
process. Paths use exact area coverage. Gradients integrate premultiplied paint
with subcell coverage at an adaptive 1–16 samples per axis. Clips and masks share
a 4× grid with child paint, group opacity and W3C blending, then resolve once.
A group that only carries opacity over one draw folds the opacity into its paint.
Intermediates stay RGBA32F until the project's working-precision store. An
axis-aligned image-box clip is an exact rectangle bound applied at the output,
not a group clip.

**Playback** (the SVG counterpart of the level 2 playback resampler, frames
while playing or scrubbing) reprojects a persistent RGBA16F source raster. It is
keyed by the file's path and stamp, an upward √2 bucket of the transform's
largest scale (2–2.8 source pixels per output pixel) and the image crop in SVG
user space, which is painted into the raster. Translation and rotation reuse it;
a scale beyond the bucket repaints it. Each output pixel integrates its source
footprint (4 × 4 quadrature of the bilinear raster), conserving area and
centroid within 1e-3 and staying several times closer to f64 coverage than
today's bilinear RGBA8 draw. Sources above 32 MiB, or footprints beyond 4.5
texels per axis, paint directly.

Gaussian blur and offset run in float before the group's clip, mask and opacity.
Filter and primitive regions clip output while retaining the source halo. Each
primitive uses its declared `color-interpolation-filters`, defaulting to linearRGB.
SVG 1.1 `filterRes` uses its authored raster grid; zero disables the filtered element.
A Gaussian of at least 3 device pixels runs on the device grid (area average,
variance-matched kernel, bilinear reconstruction on the 4× grid, regions applied
there); kernels and halos extend 6σ.

Filtered and large groups use ordered tiles with a shared 4× grid and Gaussian
halos, the fine scene resolved once and culled per tile. Tiles are as large as
the live scratch targets allow within 128 MiB, and at most two short batches are
in flight. Scratch textures return to a pool after their last encoded read.
Texture scratch, coverage and filter work are preflighted; buffer sizes, binding
sizes, dispatches and texture dimensions are checked against the device limits
before creation. Work beyond those bounds uses the declared resvg fallback rather
than failing an export. `Renderer::svg_fallback_reason` exposes the runtime reason,
including on a cached fallback result.

Unsupported filter primitives, Gaussian duplicate/wrap edge modes, patterns,
embedded images and crisp-edge paint use resvg at output resolution with an RGBA8
store. External resources never load; data URLs may resolve.
The image reader retains the existing corrupt-document and encoding decisions,
and deeply nested documents use a bounded, sized stack.

Eligible arcs and geometric shapes are normalized to cubics within 1e-6 user units
before parsing; context-dependent CSS geometry retains usvg's conversion.

SVG semantics follow the SVG specification and the resvg conformance cases. Where
those leave user-agent choices open, parsing preserves Chromium's authored sizing
and font defaults. Missing root dimensions without a viewBox use the image default
size instead of stretching the content bounds. Rounded rectangles follow SVG's
`rx`/`ry` rules, including `auto`.

`pm-render/tests/svg.rs` (G-V3) checks geometry, paint, scale, cache independence
and preview/export equality against an independent f64 paint reference. It requires
captured legacy-app frames for all placements and all Chromium semantic probes.
`tests/svg_filters.rs` includes public synthetic actual legacy filter witnesses and checks
Gaussian color, regions, transformed covariance and filter grids against independent
f64 math. The desktop gate uses live authored placements, checks geometry at RGBA32F
against f64, and proves the window's RGBA8 endpoint is the same frame after rounding.
Each SVG image resolves before layer compositing; paint, filter, clip and mask within
one image share their subpixel grid. CLI and job-engine tests require an SVG project
to render with no Chromium helper or editor window attached.
