# Scene-driven SVG renderer

`svg-player.ts` reads `public/hero/scene.json`. It uses `createEngine` from the
original exported `player.js` for property evaluation, temporal Bezier easing,
content resolution, layer-local time, group transforms, opacity, and visibility.
It never initializes the canvas renderer. Each text layer becomes an SVG group
with the original layer ID, a world matrix, and native SVG text.

The website scene retains the export plus the requested opening color/glow
keyframes. No separate phrase list or animation timing is maintained in React.

This adapter supports this composition's single-line plain text and affine
transform groups. Saved font anchors are mapped using SVG glyph bounds. Glow is
an SVG blur behind unfiltered vector text, so its halo can differ slightly from
the WebGL multi-pass effect. This is not a universal renderer for arbitrary
Powermove projects. Masks, paragraph text, styled runs, and text animators are
rejected. The export's already-missing word-slide-swap effects remain inactive,
matching the supplied WebGL export.

Browser validation sampled 20 scene times, checking 180 layer/time combinations
for visibility and (when active) world matrix and opacity against the original
engine. Desktop and mobile rendered without canvas elements or browser errors.
