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
| Blend modes over translucent stacks | Drawn; today's drift not reproduced (below) | `composite/blend-modes-alpha`: 99.944% within 1, max 5 CV opaque, 4.61 CV float |
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
| Bezier path masks | Routed (`Feature::PathMasks`) | Gaps |
| Masks on 3D object layers | Routed (`Feature::Masks`) | `composite/3d-mask`, `composite/3d-blend-mask` (below) |

Corpus figures are from capture `composite-20261016-042859-be1e4467` (Chromium,
ANGLE Metal, Apple M4 Max), compared by `tests/corpus.rs` under section 25's
Standard class (≤ 1 CV for 99.9% of pixels, ≤ 4 CV anywhere) and, for transparent
captures, the float frames within 1 CV.

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

## Today's composite drift (proposed appendix A item)

`FRAG_COMPOSITE` runs over the whole frame for every blend layer and sends every
pixel through `(d.rgb / d.a) · d.a`. On a translucent stack, the RGBA16F target's
conversion loses up to an ulp per pass where the round trip lands low; across the
48 blend layers of `composite/blend-modes-alpha`, pixels no layer covers drift by up
to 4.6 CV. Measured: the legacy frame differs from the same stack rendered without
any blend layer by up to 3.9 CV in cells no blend layer touches; with today's
formula evaluated literally, the engine drifts by up to 46 f16 ulps at different
pixels (the exact rounding outcome depends on each pass's arithmetic).

The engine passes the destination through exactly where the layer has no
coverage (`s.a == 0`: the formula's exact value is `d`, or zero color under the
0.0001 guard). This is closer to section 25's alpha class reference (an f64
reference) than today's frame, and makes pixels outside a layer independent of
how many blend layers exist. `tests/corpus.rs` lists the one affected case in
`COMPOSITE_DRIFT` with its measured bounds (5 CV float, 5 CV opaque); every other
case keeps the full class. Opaque stacks (`d.a = 1`) are exact either way.

## Masks on 3D object layers

Today a masked 3D object layer goes through the slow path and its scene pass shows
turned by 180° (the sun's highlight moves from the upper left to the lower right;
`composite/3d-mask` against the unmasked `composite/3d-blend`, which matches within
the 3D bar). The engine draws the scene pass upright and the mask through the
projected plane inverse as `applyMasks` computes it, so the cases differ by up to
171 CV. Until the plan owner decides whether to reproduce the turn, masks on 3D
object layers are routed. Blend modes on 3D layers are drawn, and their linear view
depth stays in the depth and id surface.

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

## Tests

| Test | What | Result |
|---|---|---|
| `tests/composite.rs` | Blend modes (5 color pairs, 9 modes, 2 opacities, 3 color setups, 2 precisions), styled groups, mask coverage, matte modes and visibility, matte depth and cycles, group matte sources, adjustment layers, isolation, regions with padding, determinism, project settings | Pass |
| `tests/linear.rs` | 50% white over black and a 2× downscaled checkerboard: 188 with linear blending, 128 without | Pass |
| `tests/color.rs` | Invariant (2), pipeline transparency (ProRes 4444 XQ → `y416`, ProRes 422 HQ → `v216`), and invariant (1) for 4:2:0, each with linear blending on too | Pass: 0 samples differ |
| `tests/corpus.rs` | Every supported corpus project (v1, render, composite captures) | Pass |
| `pm-color` unit tests | Composite and matte references | Pass |
| App: `project-settings.test.ts`, `settings/project-color.test.ts`, `shared/project-color.test.ts`, `engine-stamp.test.ts`, `e2e/project-color.spec.ts` | The Color rows, Undo, the stamp, hidden without the new engine | Pass |

Recapture the corpus cases with
`pm-electron-lock bun engine/corpus/src/capture.ts --only composite/ --label composite --skip-scenarios --skip-real --no-eval`.

## Gaps

- **Bezier path masks** are routed. Today they rasterize every mask of the layer with
  Canvas2D and blur each by CSS `blur()` (Skia); matching needs a Skia-equivalent
  antialiased fill and Gaussian blur within 1 CV. `pm_vector::mask::PathMasks`
  already holds the geometry and the blur radius.
- **Masks on 3D object layers** are routed (above).
- **Effects, transitions, motion blur** remain routed until the effects lane installs
  an `EffectHost` and adds transitions and motion blur to the graph. Section 2's third
  linear-blending line (a Gaussian blur of a white line conserves energy within
  0.5%) needs the blur effect and is not measured here.
- **The run-in-E option** has no UI (the plan names only the two rows).
- **Today's composite drift** is not reproduced (above); needs the plan owner's
  decision as an appendix A item.
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
  variants, `corpus.rs` and `support/mod.rs` (composite captures, the drift entry).
- `engine/corpus/src/projects/composite.ts` (new) and `index.ts`.
- App: `legacy/ui/project-settings.ts` (Color section), `settings/install.ts`,
  `settings/project-color.ts` (new), `shared/project-color.ts` (new),
  `shared/engine-stamp.ts` (project color is native-only), `e2e/project-color.spec.ts`.
