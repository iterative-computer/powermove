# Editable Figma layer data

Figma import emits ordinary shape, text, image and group layers. A bitmap in the
renderer cache is not an imported image asset. Existing projects without these
optional fields keep their previous rendering. Project container format stays
unchanged; optional JSON content and channel records round-trip through `.pmv`.

## Properties for inspector integration

Channels use the existing `{v,kf,expr}` representation. `set_property` accepts
these paths; scalar drawing fields become channels on first edit. Enum, boolean
and string channels use discrete values. Array membership and range endpoints
are structural edits rather than interpolated values.

| Path                                                                  | Values                                                        | Animation        |
| --------------------------------------------------------------------- | ------------------------------------------------------------- | ---------------- |
| `c.fillGradient.type`, `c.strokeGradient.type`                        | linear / radial / angular / diamond                           | Discrete         |
| `c.{fillGradient,strokeGradient}.transform.0..5`                      | Affine matrix, normalized object to paint coordinates         | Numeric          |
| `c.{fillGradient,strokeGradient}.stops.N.position`                    | 0..1                                                          | Numeric          |
| `c.{fillGradient,strokeGradient}.stops.N.color`                       | RGBA hex                                                      | Color            |
| `c.{fillGradient,strokeGradient}.opacity`                             | 0..1                                                          | Numeric          |
| `c.strokeStyle.align`                                                 | center / inside / outside                                     | Discrete         |
| `c.strokeStyle.{cap,startCap,endCap}`                                 | Figma cap names                                               | Discrete         |
| `c.strokeStyle.join`                                                  | miter / bevel / round                                         | Discrete         |
| `c.strokeStyle.{miterLimit,dashOffset}`                               | number                                                        | Numeric          |
| `c.strokeStyle.dashes.N`                                              | dash/gap pixels                                               | Numeric          |
| `c.strokeStyle.weights.0..3`                                          | top/right/bottom/left pixels                                  | Numeric          |
| `c.arc.{start,end,inner}`                                             | radians, radians, 0..1                                        | Numeric          |
| `c.imageMapping.mode`                                                 | fill / fit / crop / tile                                      | Discrete         |
| `c.imageMapping.transform.0..5`                                       | Normalized crop matrix                                        | Numeric          |
| `c.imageMapping.{scale,rotation}`                                     | Tile scale, degrees                                           | Numeric          |
| `c.runs.N.{font,size,weight,italic,color,tracking,leading}`           | Font family, px, weight, bool, RGBA, px, absolute line height | Scalar/color     |
| `c.runs.N.{decoration,textCase,listType}`                             | Figma enum names                                              | Discrete         |
| `c.runs.N.{paragraphSpacing,paragraphIndent,listSpacing,indentation}` | px except list nesting level                                  | Numeric          |
| `c.runs.N.fillGradient.*`                                             | Same gradient fields                                          | Scalar/color     |
| `c.runs.N.paints.M.{color,gradient.*}`                                | Stacked text paints                                           | Scalar/color     |
| `c.{paragraphSpacing,paragraphIndent}`                                | px                                                            | Numeric          |
| `c.{verticalAlign,truncation,maxLines}`                               | top/center/bottom; DISABLED/ENDING; integer                   | Discrete/numeric |
| `l.matteSources.N.mode`                                               | alpha / luma / vector and inverted alpha/luma                 | Discrete         |
| `c.isolate` (groups)                                                  | boolean; isolate child blend modes from the lower stack       | Discrete         |
| `c.{textStroke,textStrokeColor}`                                      | px, RGBA                                                      | Numeric/color    |
| `c.textPaint`                                                         | fill / stroke (stacked text paints)                           | Structural       |

`d.compound` joins subpaths for filling instead of painting holes independently.
Per-path `g.<id>.fillGroup` and `fillRule` retain separate computed regions.
`g.<id>.startCap` / `endCap` retain vector-network endpoint overrides. Geometry
vertices remain the existing editable/keyframeable path channels.

`matteSource` remains the first matte, with `matteSources` storing additional
intersections. Nested clipped groups keep their own boundaries; group masks
render their descendants. Vector mattes use geometric coverage independent of paint alpha and effects. Additional matte references are remapped during re-sync, duplication and precomposition. Imported soft-light uses `blendVersion: 2`; older projects retain the historical soft-light formula.

Shape Shadow (`figmaShadow`) owns offsetX/Y, radius, spread, opacity, color,
inner and blendMode channels. Its source alpha precedes the effect chain, so
stacked shadows do not cast further shadows. Background Blur uses the existing
backdrop effect mechanism on a separate shape coverage layer below the fills.
`figmaLayerBlur` retains progressive blur endpoints and radii. Shape Noise
(`figmaNoise`) retains density, size, colors, type, opacity and blend mode. These
are new effect IDs; previous Shadow, Gradient Ramp and Blur versions are retained.

Missing font families stay in editable source text and use the renderer's font
fallback. `receiveFigma` returns a `warnings` array; the import toast names the
missing fonts once each and offers Show Layers.

## Properties controls

The inspector edits these fields with the existing rows (`src/extensions/inspector`):
Fill and Stroke color switch between a solid and a gradient (type, stop strip,
stop color/position, Angle and Scale derived from the transform, Opacity); a
stroke shows Position, Dash/Gap, caps (dash caps on closed shapes, Start/End on
open paths), Join and Miter limit, and per-side widths on rectangles; ellipses
show Start/Sweep/Ratio; image fills show Mode and, for tiles, scale and rotation.
Styled text edits the whole text (Mixed when ranges differ, writing every range)
and lists its ranges under Styles, each with its own channels. Additional mattes
appear as further Track Matte entries; groups show Isolate (blending). Structural
edits (adding a stop, creating a stroke style) are built from each selected
layer's own data and never copied from the primary layer.

## Last-resort cases

The capability check is shared by the plugin and the app. The plugin exports a
PNG only for the scopes this check reports. The following cases remain explicit:

- Glass refraction/dispersion and texture deformation: the engine has no matching
  optical model. Noise is supported, but substituting noise would change these
  effects' appearance.
- Video paints: the transport has no video bytes/timing source. A still preview
  cannot become an editable video asset.
- Pattern paints that reference other Figma nodes: the transport does not yet
  package external pattern dependencies or their repetition/layout contract.
- Complex strokes when Figma fails to return their computed outline. Normally,
  mixed vector widths/joins are converted with the public `outlineStroke()` API
  into editable filled contours; the temporary outline is removed immediately.
  Source nodes stay intact. Endpoint caps remain strokes where possible.
- Clipboard region style IDs without their private vector-style data, missing
  clipboard text overrides, or image hashes without their actual blob bytes:
  importing guessed colors, fonts or unrelated preview pixels would lose data.
- Singular transforms, damaged/missing geometry/styles, safety limits, unknown
  future node/paint/effect/blend types and unsupported child hierarchies.

Clipboard fallback uses an isolated preview crop only when safe; overlapping
pixels promote it to the smallest enclosing subtree. The plugin supplies precise
per-node exports and avoids that clipboard limitation.

Styled ranges remain in one text layer to retain source offsets, wrapping and text
animators. Font shaping uses the browser’s installed/fallback fonts; exact glyph
metrics require the source font. Lists use editable paragraph fields rather than
baking their markers into images. Noise uses a deterministic procedural model;
Figma does not expose its random seed, so individual grain pixels can differ.

Source metadata records `importedContentKeys`. Re-sync clears obsolete static
imported fields (for example, a gradient replaced by a solid fill) while retaining
authored channels, expressions and text animators. This optional metadata also
round-trips through the existing project schema.
