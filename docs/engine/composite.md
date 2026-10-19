# Compositing: design notes

The compositing half of Phase 3a (plan sections 8, 9, 10, 25, and 26 Phase 3a):
layer masks, track mattes, the nine blend modes, adjustment layers, styled groups
as compositing boundaries, linear blending, 16- and 32-bit precision, region of
interest with effect padding, and isolation. Code: `engine/crates/pm-render/src/composite/`
(plan nodes, executor, passes, effect seam, regions, project color),
`engine/crates/pm-color/src/linear.rs` and `wgsl.rs` (blend formulas and
linear-light conversions in f64 and WGSL), and Settings › Project › Color in the
app. The plan stays the authority; this file records what is drawn, how it matches
today, and what is not done.

## What it draws

| Feature | Status | Parity evidence |
|---|---|---|
| Nine blend modes (normal, add, screen, multiply, overlay, soft light, difference, lighten, darken), keyframed blend modes, with opacity | Drawn | Corpus `composite/blend-modes`, `composite/blend-animated`: ≤ 1 CV, 100% within 1; `tests/composite.rs` against the f64 `FRAG_COMPOSITE` in both encodings and precisions |
| Blend modes over translucent stacks, including today's drift | Drawn | `composite/blend-modes-alpha`: bit-exact (opaque and transparent float), see "Today's composite drift" |
| Unknown and keyed blend modes, unknown and keyed matte modes | Drawn | `composite/unknown-modes` (4 samples): bit-exact |
| Parametric masks: rect and ellipse, add and subtract, subtract-only coverage, feather (smoothstep, the 0.01 threshold), rotation, negative and tiny sizes, animated channels, animatable enable and mode, the 8-mask limit, masks on rotated, scaled, and skewed layers, singular world matrices | Drawn | `composite/masks` (3 samples), v1 `masks/more-than-eight`: ≤ 1 CV, transparent float ≤ 0.25 CV; `tests/composite.rs` coverage against `pm_vector::ParametricMasks::coverage` |
| Track mattes: alpha, alpha inverted, luma (Rec.709 on premultiplied E), luma inverted; hidden sources still matte and never draw; source opacity, masks, and blend mode apply; nested mattes; keyframed modes; sources outside their time span give an empty matte; cycles stop at depth 8 | Drawn | `composite/mattes` (3 samples): ≤ 1 CV, float ≤ 0.62 CV; `tests/composite.rs` |
| Styled groups (opacity, blend, masks, matte) as compositing boundaries, nested; transform-only groups flattened with blended members | Drawn | `composite/groups`: ≤ 1 CV, float 0.25 CV |
| Precomps with blend modes and masks, mattes inside precomps, precomps as mattes | Drawn | `composite/precomps` (2 samples) |
| Output scale | Drawn | `composite/scaled-capture` |
| Blend modes on 3D object layers | Drawn | `composite/3d-blend`: 3D bar 100% within 2, SSIM 0.99998 |
| Adjustment layers | Drawn through an effect host; no-op without a rendering effect | `composite/precomps` (empty adjustment); `tests/composite.rs` with a test host |
| Layer and group effects | Drawn through an effect host (the effects lane) | `tests/composite.rs` with a test host |
| Linear blending, HDR-safe and run-in-E blend domains | Drawn | `tests/linear.rs` (section 2 bar), `tests/color.rs` (invariants with linear blending), `tests/composite.rs` |
| 16- and 32-bit precision, project precision as a floor | Drawn | `tests/composite.rs` |
| Region of interest with effect padding | Drawn | `tests/composite.rs`: regions equal the full frame bit for bit with masks, mattes, blends, adjustments, and padded effects |
| Bezier path masks: today's Canvas2D coverage route with every other mask of the layer, feather (Chromium's blur), add, subtract, subtract-only, animated vertices, transforms, and modes, masks switching off, on solids, images, shapes, text, captions, groups, precomps, adjustment layers, 3D planes, layers in 3D groups, 3D object layers, and with motion blur | Drawn (below) | `tests/path_masks.rs`: 54 coverage canvases bit-exact; `composite/path-masks`, `composite/path-masks-nested`, `composite/path-masks-more`: bit-exact opaque and float (3D object: 3D class, max 1 CV); `composite/path-masks-text`: text class |
| Masks on 3D object layers, animated and keyed | Drawn | `composite/3d-mask`, `composite/3d-blend-mask`, v1 `3d/transparency`, `motion/blur-3d`: 3D class, max 1 CV (below) |
| Masks that are not an array | A named layer failure, as today's `.some` throws | `tests/composite.rs` |
| Motion blur with masks, blend modes, mattes, effects, styled groups | Drawn (the accumulated samples are the layer target) | `motion/blur-slow`: bit-exact float, opaque ≤ 1 CV |

Corpus figures are from captures `composite-20261016-042859-be1e4467`,
`compclose-20261018-121218-230f9731`, `compclose2-20261018-124247-4857382c`, and
`compclose3-20261018-174842-bcfa91ad`
(Chromium, ANGLE Metal, Apple M4 Max), compared by `tests/corpus.rs` under
section 25's Standard class (≤ 1 CV for 99.9% of pixels, ≤ 4 CV anywhere) and,
for transparent captures, the float frames within 1 CV. The corpus tests fail
any project routed for masks, motion blur, 2.5D, `compat` level 2, the
`requiresEngine` stamp, or project color (`support::routing_forbidden`). Bezier
path masks no longer have a routing reason (`Feature::PathMasks` is removed),
and neither do level 1 shapes on 2.5D planes (`Feature::PlaneShapeRaster`, a
route of the previous round, is removed: the Canvas2D emulation's residual it
stood for is closed; motion.md, "2.5D"). Level 2 text now draws animators,
range typography and typography anchoring natively. The old text routing
reason remains forbidden as a regression defence.

## Graph

`plan::draw_layer` reads a layer's compositing properties as `renderProject` does
(`composite::build`: `BLEND_ID`, `applyMasks`'s filtering and coercions,
`matteMode || 'alpha'` with `startsWith`/`endsWith`, `proj.layers.find`). A layer
that leaves today's fast path (masks, effects, a blend mode, or a matte) becomes a
`plan::Content::Composite` draw holding a `composite::Node`:

- `LayerNode`: the layer's normal draw (with its opacity; a group's boundary at
  opacity 1 with the group's opacity kept for the composite), masks, effects, the
  matte pass, the blend mode, the composite alpha.
- `AdjustmentNode`: prepared effects, masks, blend mode, opacity.

The matte pass is a `CompPlan` holding only the source, drawn with `{ ...source,
on: true }` (`Scope::active_forced_on`), carrying today's `matteDepth` (8, carried
into precomps as `PM._renderOptions3D` carries it). A group used as a matte source
recurses until the stack overflows today; here it reports the group and mattes
with nothing, the same picture.

`composite::exec` splits the target's render pass at each node. Plain draws
between nodes stay one pass. A layer node draws its content into its own target,
then one fused pass applies masks and the matte (each step rounded to the storage
precision with `quantizeToF16`, as today's separate RGBA16F passes store it), with
effects between them when present. Normal blending joins the next pass as a
premultiplied source-over draw (`drawFbo`); other modes run `FRAG_COMPOSITE` into a
new accumulator. An adjustment runs its effects on the accumulator, masks the
result, and composites it with `FRAG_COMPOSITE` (normal included, as today).
Inputs are read with `textureLoad` at the pixel; today's passes sample texel
centers with no filtering, which gives the same values (measured: sampling at the
interpolated `v_st` instead changes no pixel of `composite/blend-modes-alpha`).

The passes compile with wgpu's default Metal fast math. Safe math was measured on
`composite/blend-modes-alpha`: 163 pixels over 1 CV against 62 with fast math, so
fast math is what today's ANGLE programs use as well.

## Today's composite drift (reproduced)

`FRAG_COMPOSITE` runs over the whole frame for every blend layer and sends every
pixel through `(d.rgb / d.a) · d.a`. An `RGBA16F` render target stores a
fragment's f32 output **rounded toward zero** on this GPU (measured with a
probe pass: 0.75 − 1 f32 ulp stores as 0.74951, half of 64 values below an f16
value truncate), so where the round trip lands an f32 ulp low, the stored value
loses an f16 ulp, once per blend pass. On a translucent stack of 48 blend layers
(`composite/blend-modes-alpha`) pixels no layer covers drift by up to 4.6 CV in
today's frames.

The engine now evaluates the formula literally at every pixel, as today's
program does; its targets are the same `rgba16float` with the same rounding, so
the drift is reproduced. Fused mask and matte steps round as today's separate
passes store them: the renderer probes how the device stores an `f32` output
into `rgba16float` on first use (`composite/half_store.rs`: 0.75 + ¾ of an f16
ulp) and the fused steps truncate (`half_rtz`) or round to nearest (`half_rne`)
to match. Today's compositor on another GPU family stores with that GPU's
rounding, so the probe, not a constant, decides; `tests/half_store.rs` checks
that the probe agrees with the measured truncation on this GPU. The residual
that remained on the rotated soft-alpha images came from image sampling: today's
image layers sample Blink's 8-bit premultiplied bitmap as an `RGBA8` texture,
and the GPU's bilinear filter weighs 8-bit texels differently from half float
ones. Level 1 stills in the display encoding are now uploaded and sampled as
`Rgba8Unorm` (`media.rs`, `rgba8_bitmap`; linear light keeps the converted
texture). With both, `composite/blend-modes-alpha` is bit-exact in the opaque
and transparent float frames (was 252 float pixels over 1 CV, max 4.61 CV), and
the corpus test's drift exemption is removed.

## Path masks (today's Canvas2D coverage route)

When any enabled mask of a layer has a path, today's `applyPathMasks` takes
every enabled mask (parametric ones too, with no count limit): each is filled
white on its own canvas of the output size (the item) through the layer's
`scaledWorld` and the mask's `translate`/`rotate` (and the path's own
`translate`/`rotate`/`scale`, then `tracePath` closed and untrimmed; a
parametric mask as `ellipse` or signed `rect`), and drawn onto one coverage
canvas with `ctx.filter = blur(max(0, feather) · |world x| / 2 px)` and
`source-over` (add) or `destination-out` (subtract); the canvas starts white when
no mask adds. The layer is multiplied by the canvas's alpha, sampled with
`texFor`'s linear filtering at each output pixel (through the inverse plane
composed with the world matrix for a 3D layer, `u_covTransform`), zero outside
the canvas.

The engine (`composite/path_masks.rs`, `path_masks.wgsl`;
`pm_vector::canvas2d::{mask, filter}`):

- **Items** are filled by pm-vector's Canvas2D emulation on the CPU (the same
  path, `rect`, and `ellipse` calls; Blink's `rect` was added to it) and kept
  across frames (least recently used dropped past 192 MB), so a static mask is
  filled once.
- **The blur** is Chromium 152's Skia Graphite blur image filter
  (`b6d106297ff9ef2ff8094033695d045e87775581`), planned pass for pass in `f32`
  by `pm_vector::canvas2d::filter::BlurPlan` and run on the GPU in 8-bit
  single-channel images with a transparent 1-pixel border: the save layer
  (the canvas outset by ⌈3σ⌉), `IsEffectivelyIdentity` (σ ≤ 0.03), the
  downscale steps of `FilterResult::rescale` about the layer's center
  (`downscale_step_count`, `scale_about_center`, `RoundOut` with
  `kRoundEpsilon`), the low resolution σ, the single 2D pass (nearest, area ≤
  28) or two 1D passes with `Compute1DBlurLinearKernel`'s paired bilinear taps
  (padded to the effect's loop count), the output bounds and their 1-pixel
  outset, and the final draw (pixel aligned, or bilinear back up). The SkSL
  effects declare `half` sums, weights, and offsets, but Graphite evaluates
  them at full precision on Metal: `f32` sums are bit-exact with today's
  canvases, `f16` sums leave thousands of pixels 1 code value off.
- **The composite** onto the coverage canvas is hardware blending into an 8-bit
  target (`One`/`OneMinusSrcAlpha` or `Zero`/`OneMinusSrcAlpha`), as Graphite's.
- **The apply** is the composite op's `FLAG_PATH_MASKS` step, fused with the
  matte as the parametric masks are (rounded between steps as today's separate
  passes store them).

Measured: `tests/path_masks.rs` compares 54 coverage canvases captured from
today's code by `pm-vector/tools/mask-ref` (the coverage half of
`applyPathMasks`, sliced verbatim from `compositor.ts` and run in the app's
Electron; capture `masks-20261018-171840-8e3cd12e`): a feather sweep from 0 to
160 across every pass structure (identity, one 2D pass, 1D passes, 1 to 5
downscale steps), odd canvas sizes and fractional placement, masks crossing the
canvas edges, rotated and scaled worlds, add, subtract, and subtract-only
combinations, 10 parametric masks with a path, signed rectangles, and 1920 ×
1080 and 3840 × 2160 canvases: **every case bit-exact**. The corpus cases
`composite/path-masks` (3 samples), `composite/path-masks-nested` (2) and
`composite/path-masks-more` (2: motion blur, a 3D group member, an adjustment
layer, a 3D object) are bit-exact in opaque and transparent float frames (the 3D
object within the 3D class, max 1 CV); `composite/path-masks-text` (text and
captions) is within the text class with every other pixel exact. v1
`masks/path` holds an SVG image (the media lane's routing) and is not compared.
`pm-vector`'s `tests/mask_coverage.rs` runs the same plan on the CPU with exact
bilinear weights as a model: within 2 CV (the GPU's filter weights differ).

## Masks on 3D object layers

Today a masked 3D object layer writes no shared depth (`userData.layerOccludes`
is false while an enabled mask exists), so its color pass draws without a depth
test against itself and the far hemisphere's inside shows (the "turned by 180°"
look of `composite/3d-mask`). The 3D scene lane's pass already models
`layerOccludes`, so the routing is removed: `composite/3d-mask`,
`composite/3d-blend-mask`, and v1 `3d/transparency` match within the 3D class
(max 1 CV, object SSIM ≥ 0.99998), and `motion/blur-3d` adds a masked blurred
object. The mask maps through the projected plane inverse, as `applyMasks`
computes it.

## Effects (seam with the effects lane)

The effects lane owns the effect table, the GLSL pipeline, and the built-ins. The
compositor owns where effects run: after masks and before the matte for a layer,
and on the accumulator (also the backdrop) for an adjustment layer.
`composite::EffectHost::prepare` is called at plan time for a layer with renderable
effects and returns `PreparedEffects` (total padding, whether a backdrop is read,
and `EffectPasses` that record the chain). `Renderer::set_effect_host` installs the
host; without one, effects and adjustment layers with effects stay routed
(`Feature::Effects`, `Feature::Adjustment`), and the static capability check keeps
listing them. A chain that records nothing (`Ok(None)`) leaves the layer or the
stack unchanged, as a missing program does today; an error fails the layer.
Targets a chain takes from `EffectContext::pool` go into `EffectContext::taken`,
the returned one included.

## Region of interest

A region renders the full frame's geometry through an offset viewport, so region
pixels equal the full frame's. A pass holding nodes grows to the region plus the
largest padding of its nodes' effects, clamped to the frame (`Padding::Full` is the
limit: the whole frame), and the region is copied from it at the end. Groups and
matte passes render over the pass rectangle.

## Color

- **Project settings.** `color: { mode, linearBlending, precision, blendDomain }` at
  the project root (contract 4 `ProjectColor`); absent keys are the default, unknown
  values are routed (`Feature::ProjectColor`). The renderer reads them from the
  document; the request's precision is raised to the project's.
- **Linear blending.** Solids and solid backgrounds are decoded with E⁻¹ when drawn;
  gradient backgrounds are interpolated in E as today and decoded; sources decode
  in the source pass (contract 4); the 3D pass output (E) is unpremultiplied,
  decoded, and re-premultiplied when drawn. The finished frame is encoded with E
  when it is copied out, so `Frame::color` is premultiplied E in both encodings and
  presenters and encoders need no change.
- **Blend domains.** Normal, add, multiply, difference, lighten, and darken run in
  the working encoding. With linear blending, screen, overlay, and soft light use
  the HDR-safe formulas, or run in E with `blendDomain: "displayEncoded"` (no UI
  yet). Luma mattes read Rec.709 luma of premultiplied E values in both encodings.
- **Settings › Project › Color** (new engine only): "Blend in linear light" (the
  Settings switch) and "Precision" (16-bit float, 32-bit float). Each change is one
  Undo step through history; the default removes the field, so migrated projects
  open in Standard with linear blending off. A non-default setting stamps
  `requiresEngine: "native"` (`shared/engine-stamp.ts`).
- **The stamp draws natively.** The static check used to route every stamped
  project, so linear blending was drawn by the legacy engine in the app. The
  stamp is now only the legacy engine's refusal: every stamping field is drawn
  (linked media, interpretation, `keepFrameRate`) or judged by its own check
  (project color, clip pitch). Measured with
  `e2e/native-engine-p3a-measure.spec.ts -g 23.976` (Use New Engine on, three
  UHD ProRes 422 HQ layers with Blur 24, 60 s loop, load average 5 to 7):
  linear off 1,438 frames shown, 0 skipped, 0 repeated, display interval p50
  41.5 ms, max 49.8 ms; **"Blend in linear light" on (stamped `requiresEngine:
  "native"`), drawn by the engine: 1,439 shown, 0 skipped, 0 repeated, p50
  41.5 ms, max 49.8 ms**, engine compose mean 1.8 ms. The same run at
  `feat/native-engine` 9263858a routed the linear row to the legacy engine.
  (A first run of this branch found the display asleep: 0 frames presented;
  rerun with the display awake as above.)
  The 29.97 and 59.94 rows (`-g "29.97|59.94"`, 2026-10-18, clips generated as
  the spec describes, other lanes' jobs running: load average 7 to 16):
  29.97 off 1,799 shown of 1,798, 0 skipped, 0 repeated, p50 33.2 ms, max
  41.5 ms; **29.97 linear on, drawn by the engine: 1,799 shown, 0 skipped, 0
  repeated, p50 33.2 ms, max 43.4 ms**; 59.94 off 3,597 shown, 0 skipped, p50
  16.6 ms, max 24.9 ms; **59.94 linear on: 3,499 shown of 3,596, 98 skipped,
  79 presenter drops (late), max 268 ms** (engine compose mean 1.6 ms, 25
  late-skipped). The 59.94 linear row fails real time on this loaded run; it
  needs the clean rerun phase3a-gate item 6 already asks for before it can be
  read either way. (The per-row results are kept with the lane's evidence,
outside the repository.)

## Tests

| Test | What | Result |
|---|---|---|
| `tests/composite.rs` | Blend modes (5 color pairs, 9 modes, 2 opacities, 3 color setups, 2 precisions), styled groups, mask coverage, matte modes and visibility, matte depth and cycles, group matte sources, adjustment layers, isolation, regions with padding, determinism, project settings | Pass |
| `tests/linear.rs` | 50% white over black and a 2× downscaled checkerboard: 188 with linear blending, 128 without | Pass |
| `tests/color.rs` | Invariant (2), pipeline transparency (ProRes 4444 XQ → `y416`, ProRes 422 HQ → `v216`), and invariant (1) for 4:2:0, each with linear blending on too | Pass: 0 samples differ |
| `tests/corpus.rs` | Every supported corpus project of the default captures (v1, render, effects, composite, motion, compclose, compclose2, compclose3) | 2026-10-19: 342 frames, 5 failures, all video class (`motion/blur-video`, the appendix A item; `retime/speed` and `space25d/precomps`, the media lane's chroma-edge decode, confined to the video plane); `space25d/vector` and `composition/metadata` bit-exact; skips: SVG images (`layers/svg-scales`, `mattes/nested-and-animated`), animated images, playback proxies |
| `tests/text_corpus.rs` | Text and captions projects of the same captures, with the built-in definitions | 2026-10-19: `space/groups-25d` drawn: text within the text class, every other pixel exact (max 0); `space/layers-25d` skipped (SVG images, the SVG route) |
| `tests/path_masks.rs` | 54 path mask coverage canvases against today's (`pm-vector/tools/mask-ref`) | Pass: bit-exact |
| `pm-vector` `tests/mask_coverage.rs`, `canvas2d::filter` unit tests | Item rasters exact where nothing is blurred; the CPU model of the blur within 2 CV; Skia's step counts and kernels | Pass |
| `tests/half_store.rs` | `rgba16float` stores truncate on this GPU, and the renderer's probe agrees | Pass |
| `tests/space25d_picks.rs` | 2.5D layers draw where they are picked; level 2 text on planes | Pass: 338,211 drawn pixels, all picked; level 2 placement 0 misses (motion.md). Samples where a layer draws nothing (today's capture is transparent there too) check no box |
| `pm-vector` `tests/canvas2d.rs` | Level 1 shape and path rasters against today's, including the viewport sweep | 529 cases: 503 exact, 22 within 1, 4 pinned; the radius-24 cases and every `viewport-*` case must be bit-exact |
| `pm-color` unit tests | Composite and matte references | Pass |
| App: `project-settings.test.ts`, `settings/project-color.test.ts`, `shared/project-color.test.ts`, `engine-stamp.test.ts`, `e2e/project-color.spec.ts` | The Color rows, Undo, the stamp, hidden without the new engine | Pass |

Recapture the corpus cases with
`pm-electron-lock bun engine/corpus/src/capture.ts --only composite/ --label composite --skip-scenarios --skip-real --no-eval`.

## Gaps

- **Animated path masks fill on the CPU every frame.** An item is the whole
  output-size canvas through pm-vector's emulation: 33 to 40 ms per mask at
  1920 × 1080 with pm-vector optimized (`tests/mask_coverage.rs`, `hd-*`; 2 ms
  at 480 × 300), about four times that at 3840 × 2160; the GPU blur and
  composite are small next to it. Static masks are filled once (the item
  cache). Filling only the path's device bounds (a windowed MSAA target in
  canvas coordinates, so the samples stay exact) is the next step for real-time
  animated masks at UHD; not measured in the viewer.
- **Level 2 text** draws animators, range styles and typography anchoring
  through its shared evaluated frame (`text.md`). Its projects are included
  in `allProjects`; the corpus routing checks forbid any return of the old
  level 2 text reason, and `text_level2_semantics` checks drawing and geometry.
- **The run-in-E option** has no UI (the plan names only the two rows).
- Today's masks have no expansion, opacity, or inversion; there is nothing to port.

## Changes outside the lane

- `pm-eval`: `Evaluator::active_forced_on` and `Scope::active_forced_on`.
- `pm-render`: `plan.rs` (composite nodes in `draw_layer`, matte passes,
  `build_with_effects`), `exec.rs` (pass split, `resolve_draws`, `draw_pass`,
  linear decode of solids, backgrounds, and 3D output), `renderer.rs` (project color,
  precision floor, effect host, encoded flip), `pipelines.rs` and
  `shaders/composite.wgsl` (shared E functions, gradient decode, 3D decode flag),
  `media.rs` (converted stills and frames keyed by working encoding),
  `capability.rs` (blend modes, parametric masks, mattes, and supported project
  color no longer routed; path masks and masks on 3D objects routed).
- `pm-render/tests`: `headless.rs` capability expectations, `color.rs` linear
  variants, `corpus.rs` and `support/mod.rs` (composite captures; the drift
  entry, since removed; `routing_forbidden`), `text_corpus.rs` (the capture's
  16 motion-blur samples, routing forbidden).
- Phase 3c closure (lane compclose). Outside `composite/`, `motion/`,
  `space25d/`, `sampling/`, `vector/`, and pm-vector, with the reason:
  - `plan.rs` (about 300 lines): the blurred slow path, per-sample 3D state of
    blurred 3D objects, level 2 on planes, the path mask route through
    `layer_masks` (now given the output size);
  - `capability.rs`: the `requiresEngine` stamp, 3D masks, non-array masks,
    `TrackMatte`, `BlendMode`, and `PathMasks` removed, motion blur, 2.5D, and
    level 2 for every type (`PlaneShapeRaster` added in one round and removed
    in the next),
    `unsupported_with_definitions_and_fonts` (for the text corpus test);
  - `scene3d/layers.rs` (about 130 lines): `Pass3d::at_time` and depth sets for
    blurred 3D objects;
  - `media.rs`: `rgba8_bitmap`. This changes sampling for every level 1 still in
    the Standard encoding (the media lane's still class, not only compositing);
    the texture is `Rgba8Unorm` and sample-only whatever the precision, which
    `MediaCache::image` documents and a debug assertion checks;
  - `exec.rs`, `shader_layers/plan.rs`: level 2 pyramids and target sizes;
  - `pm-node` `present/document.rs`: the removed reasons' and the new reason's
    descriptions;
  - tests outside the lane: `text_corpus.rs` (built-in definitions, the
    fonts-and-definitions check, failure images of the "rest" pixels),
    `support/mod.rs` (closure captures in the default set), `headless.rs`.
- `engine/corpus/src/projects/composite.ts` (new) and `index.ts`.
- App: `legacy/ui/project-settings.ts` (Color section), `settings/install.ts`,
  `settings/project-color.ts` (new), `shared/project-color.ts` (new),
  `shared/engine-stamp.ts` (project color is native-only), `e2e/project-color.spec.ts`.

## COMPCLOSE-2 follow-up: windowed mask fills

The animated-mask CPU-fill gap above is superseded by a windowed white-fill
path in `pm_vector::canvas2d::Context`. Tessellation and the viewport transform
still use the original output canvas, including its f32 round trip. The raster
window is the resulting device mesh bounds, clipped to the canvas with a one-pixel
AA/snapping margin. MSAA and analytic samples keep **global** pixel coordinates;
vertices and fragment interpolation are never translated to the window. White
fill alpha resolves directly from the sample mask/analytic coverage, avoiding a
full-canvas multisampled RGBA allocation and resolve.

Polygon fills use exact rational scanline crossing thresholds and fill winding
spans, rather than comparing every pixel with every edge. Both nonzero and even-odd
rules retain the original top-left/sample tests. Alpha outside the window is still
present and transparent in the full-sized item returned to pm-render. The blur's
save-layer bounds, feather margin, downsample phase, GPU passes and composite
therefore stay unchanged; cropping the blur source would change pixels.

Verification on the existing captures: every one of the 54 GPU mask canvases is
bit-exact, including HD, UHD, all feather pass structures, off-canvas paths and
subtract-only combinations. The composite path-mask corpus and a direct full-canvas
versus windowed-fill test cover placement, clipping, reflections, analytic and
polygon/triangle paths at 317×233, 1920×1080 and 3840×2160. The 529-case Canvas2D
gate retains exactly its previous result: 503 exact, 22 within one, four pre-existing
pinned cases; no reference, tolerance or exception was changed. Those existing
pins remain the vector lane's gaps, not accepted differences from this change.

The GPU/item cache and full-sized alpha/blur upload are unchanged. Windowing saves
CPU fill work; it does not establish that every animated-mask project beats today's
GPU Canvas2D path. Paired viewer results and remaining limits are recorded in perf.md.

Round 4 (after merging the wave 5 lanes, including perf2's pm-render caching and
svgcompat): `tests/path_masks.rs` (54 canvases bit-exact), the pm-vector canvas2d
unit tests and the 529-case Canvas2D gate (503 exact, 22 within one, the same four
known exceptions, capture `canvas-20261018-220349-9a95e00c`) pass on the merged
head. A new unit test, `windowed_pixel_aligned_rects_equal_full_canvas_at_every_pixel`,
covers the non-AA bounds renderer (`Polygons { msaa: false }`, the 1X pattern) that
the window test did not: device-aligned rectangles, within the pixel tolerance,
mirrored, quarter-turned and partly or wholly offscreen, at 317x233, HD and UHD,
asserting that path is taken. The windowing code is unchanged since round 1; the
composite `path-masks*` corpus was not rerun this round: no pm-cli build exists in
this worktree, and building one with 45 GB free risked the 40 GB disk floor. Its
round 1 result (bit-exact) stands. The
playback cadence tail is issue P4 (perf3 lane).
