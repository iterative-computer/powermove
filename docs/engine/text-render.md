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
| Apple Color Emoji (`sbix`), placed as Canvas2D places it | Drawn | `fit-emoji` (6 to 64 px), `emoji` group |
| RTL and complex scripts the level 1 layout shapes (Arabic, Hebrew, Devanagari, Thai, CJK) | Drawn | `scripts` group |
| `compat` level 2 text and captions, at output resolution | Drawn when the host gives level 2 fonts; still routed by the static check (below) | `tests/text_level2.rs` |
| Text and caption colors: hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, `hwb()`, named colors, `transparent`; rejected strings keep the style `save()` restored, as the canvas does | Drawn | `pm_text::render::color` tests (values checked against Chromium's `fillStyle`), `tests/text_colors.rs` |
| Valid CSS colors the engine does not reproduce (`lab()`, `lch()`, `oklab()`, `oklch()`, `color()`, `color-mix()`, relative colors, `calc()`, `currentcolor`, system colors, any functional notation that does not parse) | Refused (`Feature::Text`), not drawn with a fallback | `tests/text_colors.rs` |
| Color vector glyphs (`COLR`) | Refused (`Feature::Text`), not drawn wrong | `scene::tests::color_vector_glyphs_are_refused_not_drawn` |
| Text and captions without configured fonts | Refused (`Feature::Text`) | `tests/text_gpu.rs` |

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
- **Paths:** a glyph whose font size exceeds 256 px, or whose plain bounds
  plus one pixel exceed 256 px in either dimension, draws as a plain path
  (Menlo's 1.03 em full block switches at 250 px, its H at 260 px).
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

## Routing

`pm_render::unsupported` and `unsupported_with_media` describe a renderer
without fonts and list text and captions layers (`Feature::Text`).
`unsupported_with_fonts` and `Renderer::unsupported_with_media` describe a
renderer with fonts (`Renderer::set_text_fonts`), where they are drawn.
`compat` level 2 content stays routed (`Feature::CompatLevel2`) until level 2
layer semantics (bounds, picks, anchors) exist in `pm-eval`; the renderer
draws it when asked. `Renderer::set_captions_hidden` is `PM.captionsHidden`
(exports with `captionsBurn: false`).

## Measurements

Apple M4 Max, macOS 27, Electron 44 (Chromium 152) for references,
2026-10-16, machine load 9 to 20. Captures (private, not in the
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
cd engine
cargo test --release -p pm-render --test text_raster   # PM_TEXT_ONLY, PM_TEXT_DIFFS, PM_TEXT_PROFILE, PM_TEXT_COVERAGE=cpu, PM_TEXT_REPORT
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

- **Small text (≤ 18 px device size) at level 1:** 113 of 190 cases reach
  0.98 (mean 0.979, min 0.871). CoreGraphics smoothing moves the top edges
  of glyphs by font- and size-dependent amounts at small sizes (Menlo's block
  top departs from the trend by −0.6 to +0.4 px at some sizes; Avenir Next
  Bold's cap tops show no upward growth at 16 px), which one dilation per
  size cannot reproduce. The profile is the best fit found (mean from 0.95
  to 0.98); the review sheets show the differences.
- **Engine hosts do not give their renderers fonts yet.** `pm-node`'s
  interactive and job engines create `Renderer`s without `set_text_fonts`;
  the registry lives on the utility process's main thread (`FontService`),
  and a render-thread `TextFonts` needs recorded registry answers over full
  fonts (`ReplayResolver`) plus a way to answer misses. Until then routing
  keeps text and captions on the legacy renderer (the static check lists
  them).
- Synthesized bold: 0.93 to 0.96 at 36 and 80 px.
- Emoji below 10 px: 0.92 to 0.96 (resampling). CoreText's own emoji
  placement (6 px higher at 48 px) is not the rule, so level 2 emoji are not
  compared with CoreText.
- `COLR` color fonts are refused rather than drawn.
- Level 2: animated and range-styled text and typography anchoring are
  refused; routing keeps `compat` level 2 content on the legacy path.
- Routing does not yet separate the measured sub-gate classes (level 1 text
  at 18 px and below, synthesized bold, emoji below 10 px):
  `unsupported_with_fonts` treats all level 1 text as drawn. This has no
  effect while hosts give no fonts; whether to gate those classes before
  fonts are wired is a plan-owner decision.
- Animated text evaluates its posed glyphs twice per frame (inside
  `Scope::text_raster` for the placement and again through
  `animated_raster_glyphs` for the draws). Typing stays at p95 ≤ 4.4 ms for
  the engine's share, so this is left to a `pm-eval` change that returns
  both at once.

## Changes outside the lane

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
