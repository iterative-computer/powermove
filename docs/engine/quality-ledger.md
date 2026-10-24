# Rendering quality ledger

QUALITY-1 (owner, 2026-10-23): the engine is the same or better, never worse,
and never changes authored intent. Geometry, layout, timing, selected colours,
effect and blend formulas, extension drawing and API behaviour stay exact.

A rendering cleanup is approved only with:

- a high-precision reference (linear f32, or converged 16×/32× supersampling)
  of the same frozen view, crop, scene and backdrop;
- per-sample, per-channel engine error no larger than today's error everywhere;
- preview equal to export at the same view and quality.

An approved row may replace an exact gate; the row must say which gate. A look
change across every project is an owner decision. A missing or different view,
crop, frame, overlay or chrome is an intent failure, never a quality row.

| ID | Surface | Difference | Reference result | Status |
|---|---|---|---|---|
| C3-Q1 | Live viewer frame of the shipped 1.0.8 fork, 3D camera view, 1464×824 | One interior cube pixel, `(799, 448)`: red 151 today, 150 engine; other channels equal. The Library card is exact. | None captured. Two 8-bit images cannot rank one level. | Open, not approved; owner consumers |

C3 Library cards have no approved residual: all twelve rendered 3D cards
(camera and right view, both themes, current, frozen-current and 1.0.8
viewers) match today exactly. The earlier header and cursor SVG differences
were Chromium tile raster history, present between two engine-off launches.
They are not engine rendering; see [consumers.md](consumers.md).
