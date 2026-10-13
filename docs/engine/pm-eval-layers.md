# pm-eval: layers, 2.5D, groups, retiming, text, captions, bounds, picks

Design notes for the layer half of `engine/crates/pm-eval` (plan section 7,
section 5 rules 4 to 10, section 8 "2.5D", section 10, appendix B; Phase 1).
The evaluation core (`ev`, `evP`, `active`, `resolveContent`, keyframes,
addresses, caches) is described in `pm-eval.md`; this note covers what is
built on it. Everything here is a pure function of a document, a composition,
and a time, and runs unchanged on wasm32.

## Modules

| Module | Ports | Contents |
|---|---|---|
| `scope` | `PM.curComp()`, `PM.proj`, `PM.L`, `parentOf` | `Scope`: the evaluator, the composition being evaluated, and the open composition (some legacy fallbacks read `PM.proj` inside nested compositions) |
| `transform` | `legacy/core/anim.ts` | `localMatrix`, `transformParentMatrix`, `worldMatrix`, `worldOpacity`, `wouldCycle` |
| `space3d` | `legacy/core/space-3d.ts` | `local3D`, `parent3D`, `world3D`, `is3DLayer`, perspective per 3D group, `planeMatrix` homographies, `planeContains`, `depthOrderedLayers` (coplanar runs keep timeline order), `poseValues3D` |
| `parenting` | `legacy/core/parenting.ts` | world aims of cameras and lights (stored in parent space), `aimValues3D`, `worldPoseValues` (the reads behind pose-preserving reparenting and ungrouping; the edits are `pm-edit`'s) |
| `groups` | `legacy/core/layer-groups.ts`, compositor group rules | membership, `groupSpan`, `expandGroups`, `transformRoots`, model groups, renderable effects, transitions, the compositing boundary, `compositingLayers` |
| `matte` | `legacy/core/matte.ts` | `validMatteSource` |
| `anchor` | `size-anchor.ts`, `font-anchor.ts` | size bounds, size and font anchor offsets |
| `retime` | `legacy/core/retiming.ts` | `sourceTime`: time remap, constant speed, variable speed (section 5 rule 5) |
| `text_anim` | `legacy/core/text-animation.ts` | animator state (stagger, range, wave; characters, words, lines), range styles, posed glyphs, `countTextUnits`, unit ranks and weights |
| `text_geom`, `text_measure` | `legacy/gl/raster.ts` text geometry | raster box, anchor, selection bounds, Blink ink boxes; `TextMeasure` over `pm-text` level 1 |
| `captions` | `captions/model.ts`, `layout.ts`, `render.ts` frame | style normalization, tokens, word timings, layout, placement, word states, `PM.captionBounds` |
| `bounds` | `GL.bounds`, `nativeBounds`, `GL.pick`, `groupBounds`, `pathValues`, `groupMatrix` | 2D bounds of every layer type and picking; 3D object picks and bounds go to a `LayerHost` |
| `scene3d` | `core/scene3d/layers.ts` `compositionScene`, `service.ts` placement | the evaluated 3D scene: objects, lights, the topmost active camera and environment, studio defaults, aims in parent space, shared-material timing |

## API changes in this lane

- `Pose2d::rotation` is a `JsValue`: for a layer collapsed to a point
  (`allow_zero`) `worldPoseValues` returns the stored rotation unconverted,
  and the ungroup bake concatenates onto a string one. `Pose2d` is no longer
  `Copy`.
- `TextMeasure::spaced_width(d, text)`: `measureText(text).width` with
  `font = fontStr(d)` and `letterSpacing = d.tracking px`, the captions
  measuring canvas. `CompatMeasure` implements it with `Compat::measure`.
- `LayerHost::caption_bounds` is gone: caption bounds are native
  (`Scope::caption_bounds`). `HostOnly` now only covers three.js answers.

The evaluator core's API is unchanged.

## Time

`sourceTime` has two integration steps for variable speed. `Retiming::Legacy`
is `retiming.ts`: a midpoint rule at `1 / (max(30, PM.proj.fps) × 2)` s, so a
nested layer's source time depends on which composition is open (appendix A
item 9). `Retiming::Engine` (the default) is section 5 rule 5: one midpoint
sample per frame of the owning composition's exact rate, frame boundaries on
the tick grid, plus a partial step to the requested time. Parity tests use
`Legacy`; `layers_unit.rs` checks that `Engine` follows the owning rate and
integrates per composition frame.

3D content (section 5 rule 10): shared-material channels (`shader.`,
`slots.<id>.shader.`) evaluate their keys in composition timing, so keys may
lie before `layer.from` (down to −10⁸ s local); glTF clips evaluate at
`max(0, t − layer.from)` (`Scope::clip_time`).

## Captions

Section 10 puts captions word states in `pm-eval`. The module answers what
the rasterizer's `frame` answers without drawing: the cue showing at a layer
time, its layout (line-budget shrink to 60%, balanced lines, word positions),
the block placement, per-word alpha, scale, and highlight (pop, fade,
highlight scale), and the margin for strokes; `idle_block` is the column the
next caption will occupy. Measurement goes through `TextMeasure::spaced_width`.

The rasterizer uses `content.style` as stored when it has a `maxLines` key and
normalizes anything else; the port does the same, reading a stored style with
JavaScript's coercions (`ToNumber` for numbers, truthiness for flags, `===`
for enums with the legacy fallbacks).

## Allowed differences

1. **Parenting cycles in `worldMatrix`.** The legacy memo of intermediate
   ancestors within an evaluation window makes the answer for a cycle depend
   on query order; the engine answers as the first query of a window does
   (`transform.rs`).
2. **Variable-speed retiming** integrates at the owning composition's rate
   (above).
3. **Captions layout cache.** The legacy rasterizer caches a cue's layout by
   its word *count*, so editing word times without changing the count keeps
   stale times until the cache evicts them. The engine always lays out fresh.
4. **Stored caption styles with non-numbers.** Where the legacy code adds a
   stored value (`x0 + style.offsetX`, `style.stroke + …`, a cue's `start`
   in the proportional word timing) a string concatenates; the engine adds
   numbers. A stored weight outside 1 to 1000 or a non-positive size makes
   Chrome's canvas keep its *previous* font (history-dependent); the engine
   measures with `pm-text`'s default font. Project validation and every
   captions command store normalized styles and numeric cue times, so
   documents do not contain these.
5. **Unpaired surrogates in measured text** are measured as U+FFFD
   (`TextMeasure` takes `&str`). Tokens, lengths, and case mapping keep them.
6. **Text ink with subsets.** The metrics-and-shaping subsets `pm-wasm`
   receives have no outlines, so glyphs contribute no ink there; natively the
   face's own bytes are used.
7. **Picking 3D objects and model-group bounds** stay with three.js through
   `LayerHost` in Phase 1 (section 26 Phase 1: "3D geometry queries keep
   their three.js answers through shims until 3c").

## Tests

| Test | What it compares | Measured (2026-10-12) |
|---|---|---|
| `tests/layer_parity.rs` | `tests/vectors/layer-vectors.json.gz` from `generate-layer-vectors.ts`: the TypeScript layer queries under bun over seeded random documents (parents and cycles, groups, 3D groups, cameras and lights, retimed media, text animators, range styles), plus `GL.bounds` and `GL.pick` through the compositor's own install | 148,721 numbers within section 25's tolerance (140,232 bit-identical, max relative error 6.6e-14), 17,181 other values, 318 user-facing error messages identical; 714 bounds and 2,592 picks match (82 fragile, all agree). 10x seeds 2 to 7 (layer queries): 1.39 to 1.46 million numbers each, all within tolerance; 10x seeds 2, 5, and 6 with bounds and picks: about 8,000 bounds and 26,000 picks each, all non-fragile cases match |
| `tests/caption_parity.rs` | `caption-vectors.json.gz` from `generate-caption-vectors.ts` (captions model, layout, frame under bun, synthetic canvas) | 3,300 cases, 91,003 numbers, all bit-identical; 10x seeds 2 to 4: 0.89 to 0.92 million numbers each, all bit-identical |
| `tests/corpus.rs` | every evaluation dump of the newest `v0-*` capture (`PM_ENGINE_CORPUS` for another) | 422 dumps: 0 mismatches in every compared category (below); v1 capture identical |
| `tests/layers_unit.rs` | picks (locked groups, 3D depth, host objects), the compositing boundary, transition edges, `Retiming::Engine`, camera aims through the parent space | 9 tests |
| `examples/support/check-wasm-identity.ts` | native and wasm32 digests of the evaluation, layer, and caption replays | identical: 243,703 / 148,721 / 91,003 numbers; also on 10x seeds |

Corpus categories compared on v0 (all 0 mismatched): active 41,414;
channels 773,554; content 41,414; localMatrix and worldMatrix 41,269 each
(max relative error 6.9e-17 and 2.5e-16); worldOpacity 41,269; planeMatrix
238; sourceTime 258; bounds 33,222; group bounds 514; text bounds 672; text
layouts 852; text rasters 672; variable-font bounds and rasters 180 each
(ink within 2e-4 px: CoreText instances outlines with its own variation
arithmetic); text animators, glyphs, and styles 57 each; paths 49; 3D scenes
32. Not compared, with the reason printed by the test: 107 bounds the host
answers (3D objects, model groups, extension scenes without a background); 6,570 bounds and 5,310 text layouts of
the real project that `pm-text` lays out differently from the capture; 4
caption bounds whose shrunk size is off Blink's font-size grid (next
section).

**Fragile cases.** A bounds or pick case is fragile when its TypeScript
answer changes under a 1e-12 change of time, a 1e-7 move of the point, a
point on a layer's edge (a zero-width layer is hit only exactly on its line),
or a group matrix whose condition exceeds 1e5. Last-bit differences between
`libm` and JavaScriptCore's `sin` and `cos` decide those; the test reports
them apart (on the 10x seeds, 811 to 964 fragile cases, 7 to 32 answered
differently).

Run:

```sh
source engine/env.sh
cd engine && cargo test -p pm-eval --release
bun engine/crates/pm-eval/tests/vectors/generate-layer-vectors.ts     # regenerates; deterministic
bun engine/crates/pm-eval/tests/vectors/generate-caption-vectors.ts
bun engine/crates/pm-eval/examples/support/check-wasm-identity.ts
PM_LAYER_SCALE=10 PM_LAYER_SEED=2 PM_LAYER_OUT=/tmp/lv.json.gz bun engine/crates/pm-eval/tests/vectors/generate-layer-vectors.ts
PM_LAYER_VECTORS=/tmp/lv.json.gz cargo test -p pm-eval --release --test layer_parity -- --nocapture
```

## Findings for pm-text

1. **Blink measures at `EffectiveFontSize`.** In Blink,
   `FontDescription::EffectiveFontSize` is `floorf(size * FontCacheKey::PrecisionMultiplier())
   / PrecisionMultiplier()` with a multiplier of 100, in `float`: Chrome lays
   text out at the size floored to 1/100 px. `pm_text::compat::canvas_font`
   uses the size as given. Evidence: the corpus caption
   `captions/middle-lines` shrinks to 15.9048 px; Chrome's line is
   255.30812072753906 px wide, which is exactly `pm-text`'s width at 15.9 px
   (255.385 at 15.9048). The real project's Menlo text at 9.3245 px and
   10.1722 px matches 9.32 and 10.17. With the floor applied in
   `canvas_font` (tried locally, not committed: pm-text is another lane's),
   the corpus compares 2,337 text layouts with 0 mismatches (up from 852),
   all Menlo, Geist, and Geist Width layouts match, and all 28 caption
   bounds match; 135 newly compared Geist text bounds then differ from the
   capture by up to about 4e-6 px (ink boxes at fractional sizes, beyond the
   1e-6 px bounds tolerance). It must be applied in exactly one place: the
   floor is not idempotent in `float`.
2. **`system-ui` at 24 to 26 px in the real project** (3,825 layouts)
   measures about 3.7% narrower than the capture (`1` at 24 px: 10.805 vs
   11.203; capture advances are multiples of 1/64 px). The S4 fixture's
   `system-ui` cases at 11, 17, 32, 96, and 240 px pass. Not explained yet.

## Left to later lanes

- `pm-geom3d`: 3D object bounds and picks, model-group bounds, recipe parts
  (`animatedPart`), `parseScene`'s geometry budget checks.
- `pm-text`: the two findings above; then the 4 off-grid caption bounds and
  the real project's text geometry join the comparison.
- `pm-wasm`: the editor-helper shims over these functions
  (`validMatteSource`, the space-3d math, `countTextUnits`, ...).
- `pm-edit`: the parenting and grouping edits that write what
  `world_pose_values` and `aim_values3d` compute.
