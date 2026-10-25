# SVG filter witnesses

Four public synthetic SVGs from `tests/svg_filters.rs`, captured through
legacy `PM.GL.renderToPixels` with the native engine disabled. The manifest
pins each authored source and premultiplied 64 × 64 RGBA8 frame by SHA-256.
The expected paint and coverage are computed independently in f64 by the test;
these bytes serve only as the legacy quality witness.
