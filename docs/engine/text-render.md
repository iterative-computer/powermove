# Text rendering

Text uses one raster pipeline at both compatibility levels: exact outline
coverage in output pixels, followed by ordered float paint. Compatibility
selects layout, not raster quality. The font registry and layout contract
are described in [text.md](text.md).

## Authored layout

Level 1 keeps `Compat::line_runs` and Blink measurement over HarfRust.
Advances, UTF-16 wrapping, baseline (`size × 0.82`), leading, tracking,
effective size, variation coordinates, optical sizing and window font-cache
seeds are unchanged. Range typography and animator poses still come from
`pm-eval`. Level 2 uses the shared Parley frame for layout, carets, bounds,
picks and painting.

The painter takes each run's exact layout positions through the layer's
world matrix. It does not snap glyph origins, relax strike sizes, or choose
a raster-density bucket. Production layers use `Quad::Device`; transforms
therefore affect outline coverage directly at the requested output size.
The compatibility raster service still paints the requested layer canvas
and density, preserving its geometry and crop contract.

Layout is an exact gate independent of pixels. Raster improvements must
never be accepted as a reason for changing an advance or line break.

Ink boxes describe what is drawn, at both levels (TEXT-INK-1). Outline
glyphs keep their control boxes; level 1 keeps Blink's canonical-path
arithmetic for them. A color glyph's box is its drawn area from the
painter's own placement (`scene::drawn_color_glyph_ink`, shared by native
and WASM): the strike's nonzero texels at the full-em size with the
authored Apple shift, grown by the bilinear reach when enlarged, or the
COLR painted and clip bounds. A device pixel is nonzero exactly when its
square meets that box. Canvas measured emoji by CoreText's metric
rectangle rounded out and outset 1 px; selections do not show that slack.
Typography anchoring is placement, so it keeps Canvas's box
(`MeasuredLine::anchor_ink`, `TextRaster::anchor_box`): recorded
`fontAnchorBounds` are that box, and saved layers render where today's app
renders them. Advances and line breaks are unchanged.

## What is drawn

Plain text paints every wrapped line with its layout alignment and baseline.
Animated text uses the existing evaluated range font, color, opacity,
transform and blur for each posed unit. Equal poses may share a draw, but
source-over order remains authored order.

Captions paint the rounded box, then each word's stroke and fill. Word
opacity and scale act about its center. Caption stroke width remains
`2 × stroke × size / styleSize`, with round joins and miter limit 2.
Hidden captions draw nothing, including in snapshot and export requests.

Rejected CSS fill setters keep the restored style: plain text starts with
its default; an animated glyph starts with black; caption fill restores
the box fill, or black without a box. Invalid `hexToRgba` output follows
the same rejection rule. Unsupported styles report a text failure rather
than silently substituting a color.

## Outlines and coverage

Skrifa reads each glyph outline at its run's variation location. Paths and
placement use kurbo f64 geometry. User coordinates at level 1 and normalized
coordinates at level 2 keep their original meanings. Font identity, glyph,
size, variation and synthetic style participate in outline cache identity;
a font-generation change drops the old outlines and bitmap glyphs.

Synthetic italic retains the y-down shear `−1/4`. Synthetic bold retains
the authored framing width:

```
t = clamp((size − 9) / 27, 0, 1)
width = size × (1/24 + (1/32 − 1/24) × t)
```

The outline is filled and framed with that width, using kurbo geometry.
Caption strokes also use kurbo, rather than a fixed-point or A8-mask
stroker. Path tolerances are measured in device pixels.

Eligible outlines paint their pixel area directly from cached glyph-local
contours and a conservative row-to-edge index. Whole and fractional placement
are separated before f32 upload. Curves are flattened within 1e-4 device
pixels, or 1e-5 for outlines no larger than 32 device pixels. A geometric
certificate preserves already disjoint nonzero boundaries; uncertain paths
use the nonzero union resolver.

Large or compound draws use the shared R32Float area renderer. Its cells
align with device pixels, retain 1/16384 pixel coordinate precision and tile
large items. Both paths use the same resolved f64 line geometry and ordered
paint. A bounded 8 MiB path cache is independent of glyph translation, and
an 8 MiB GPU geometry cache retains compact row-indexed contours. The explicit
CPU comparison mode uses `vello_cpu`; its 8-bit coverage is not production
precision. Coverage has no luminance preblend or stem dilation.

**TEXT-RASTER-1:** exact outline coverage replaces the previous CoreText
smoothing fit, CoreGraphics masks, Skia preblend tables and distance-field
band. Removing that raster darkening is an explicit renderer decision;
it does not alter font selection, layout or synthetic-bold amount.

## Paint and blend domain

CSS colors retain their parsed float values through painting. The sRGB8
output rule is CSS Color 4 binary-search gamut mapping with local MINDE:
constant OKLCh lightness and hue, chroma reduction, deltaEOK JND 0.02.
`pm_color::css_srgb` and `wgsl::CSS_GAMUT` provide that boundary operation.
Float and wide-gamut working values remain extended. Alpha is applied after
mapping the straight output color; ordinary in-gamut values stay exact.

Coverage multiplies paint alpha. Ordered premultiplied source-over runs in
float at working precision. It does not quantize after each glyph. The
project's existing blend-domain selection controls color compositing;
coverage itself never carries a color-space transfer. Color glyph images
retain premultiplication throughout filtering and paint.

The GPU paint pass is `pm-render/src/shaders/text.wgsl`. Its CPU counterpart
checks paint arithmetic over known coverage. A CPU/GPU agreement check is
an implementation check, not the independent raster-quality reference.

Animator text blur retains its authored sigma, transformed to output pixels
by the world scale. A separable Gaussian paints and filters a float save
layer, including ink outside the output rectangle that can contribute
through the kernel. The kernel uses erf CDF differences, reaches
`ceil(8 × sigma)` pixels, and is not renormalized. Transparent samples
outside the source are zero. It uses neither the old quantized downscale
plan nor an 8-bit save layer.

## Color glyphs

COLR v0/v1 keeps Fontations' paint graph as an authored command program:
palette and foreground colors, transforms, clips, solid/linear/radial/sweep
paints, layer order and composite modes. The consumer rasterizes the program
in its working encoding before upload. Stable content keys include every
command, color and transform. The independent reference interprets the same
commands with separate f64 coverage, paint and blend formulas. Integer N32
span blitters are not part of the renderer.

Color bitmap glyphs retain their authored placement at both levels. Apple
Color Emoji uses the face's full-em advance at the requested size, including
the small-size advance differences. Its existing whole-pixel downward
`apple_emoji_shift` remains. Minification clips each inverse output-pixel quadrilateral against source
texel squares and integrates premultiplied values in the working domain.
This also handles transforms that shrink one axis while enlarging another.
Enlargement uses centered bilinear reconstruction. Transforms use the exact glyph
position. Emoji size and shift are geometry, separate from the filter.

## Storage and lifetime

Coverage pages are temporary frame resources. Coverage and paint record
into the same frame command encoder. A page returns to the target pool
after submission; another layer cannot overwrite coverage still in use.

Painted rasters retain only bounded ink crops, keyed by scene contents,
crop, working encoding and font generation. A private target pool and 4 MiB
raster cache reuse exact dimensions. Evicted targets can be recycled only
at the next frame, after the prior submission; at most 2 MiB waits for that
handoff. Explicit trimming releases handles without destroying textures that
recorded commands may still reference. No full level 1 backing canvas,
platform mask atlas, SDF atlas or per-glyph N32 paint image remains.

The font registry retains its existing window/session seed scoping. Both
preview and export use the originating window's layout seeds. Changing a
font generation invalidates glyph resources before the next layer.

## Routing

A renderer with fonts paints text and captions natively. `Renderer::set_text_fonts`
installs the host's full-font provider; the viewer, job engine and CLI use
the same path. A renderer without fonts, an unanswered font query or an
unsupported color reports a named text failure. The extension raster
service and deprecated compatibility APIs remain available while the Use
New Engine setting exists.

Level 2's shared evaluated frame remains the source for interaction and
painting: resting styled ink anchors typography, posed ink sizes the
compatibility raster, and the padded resting selection supports bounds and
picks even while staggered text is hidden. See [text.md](text.md) for those
semantics.

## Quality gates

The text pixel gates explicitly replace byte equality with today's app
with [gate-policy.md](gate-policy.md)'s reference-closeness rule. Today's
captures remain a witness; they are not the raster target. The historical
FONTS-R residual ratchet is superseded by that policy and must not return.

The independent reference uses f64 compositing and supersampled outline
coverage from `pm-reference`, at exact layout positions. It judges engine
and today against the same reference, including maximum, p99.9, mean and
per-pixel regression bounds. Reference-unmodelled content keeps its prior
bound against today; it must be reported rather than silently excluded.
No case may hide a worse maximum behind a better mean. A 32-cell reference
miss triggers whole-image refinement through 256–2048 cells. Consecutive
references must converge within 1e-4 on geometry and 0.01 CV per modeled
channel; the selected whole image faces the original gate bounds. G-T1
records separate raw float and mapped sRGB8 results.

- `pm-text` layout tests and the layout-only corpus comparison keep line
  breaks, advances and glyph positions exact.
- `text_raster` checks requested raster geometry and reference quality on
  captured plain text, captions, color glyphs and transformed text.
- `text_corpus` checks composed text, with the original projects and the
  text-isolated witnesses both counted.
- `text_gpu`, `text_colors`, `text_level2` and `text_level2_semantics` check
  paint precision, color semantics, transformations and interaction.
- `pm-text`'s bitmap test holds color-glyph ink to the painted pixels at
  several sizes and sub-pixel phases; `pm-eval`'s `color_ink` holds level 1
  measurement to it; G-T5 checks the installed app's selection and font
  anchor against its own drawn pixels.
- G-C2 checks the complete frame and preview/export contract.
- `pm-reference` tests the independent coverage, color and blend formulas.

Measured corrections and their current evidence belong in
[quality-ledger.md](quality-ledger.md). Captures, per-case measurements and
review images stay outside Git. A failing or incomplete reference class
remains a gap in the lane report; a changed raster alone never qualifies it.

## Reproduction

Build outside the machine-wide Electron lock:

```sh
source engine/env.sh
cd engine
cargo check --workspace
cargo clippy --workspace --all-targets
cargo test -p pm-text --features render,corpus-tools
cargo test -p pm-eval --test color_ink
cargo test -p pm-render
cargo test -p pm-reference
cargo run -p pm-protocol --bin pm-protocol-gen -- --check
cargo run -p pm-perfbench --example perfbench -- text_cold typing_60
```

`text_cold` and `typing_60` retain downward-only counter ratchets. Engine
wall-clock numbers under shared machine load are diagnostic. Paired app
latency is qualified only in the orchestrator's quiet window, without
loosening the existing threshold.

Before the app gates, build the addons and renderer in `apps/desktop`.
The bridge must exist at `engine/target/pm-bridge/pm_bridge.node`; removing
`engine/target` removes the development bridge too.

```sh
cd apps/desktop
bun run build:engine
bun run build
POWERMOVE_NEW_ENGINE=1 \
  ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bunx playwright test --config playwright.consumers.config.ts --workers=1 \
  e2e/canvas-text.spec.ts e2e/captions.spec.ts e2e/font-anchor.spec.ts \
  e2e/split-text.spec.ts e2e/text-animators.spec.ts \
  e2e/text-layout-resize.spec.ts e2e/native-engine-text-level2.spec.ts
```

Only the actual Electron/Playwright invocation holds the lock. Use the
consumers configuration to avoid rebuilding inside it. Disposable profiles
and harness temporary directories must be removed on success and failure.
