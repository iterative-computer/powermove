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
| C3-Q1 | Raw 3D camera frame, 1464×824 (every viewer, both themes; merged build, 2026-10-23) | One interior cube pixel, `(799, 448)`: red 151 today, 150 engine; green and blue equal. Every Library card is exact. | The native 3D colour pass resolves to RGBA8 like WebGL's framebuffer, so no pre-quantization readback exists. A least-squares surface fit over up to 2,805 surrounding face pixels has residual RMS 0.29, i.e. pure 8-bit quantization noise. It puts the true value at 150.50 ± 0.02, so both renders are 0.50 ± 0.02 levels from it. | Tie at a rounding midpoint: neither is measurably closer. Not approved as a cleanup; recorded as equal error. Owner consumers |
| C3-Q2 | 3D MSAA edges in Library cards, Metal hardware resolve (after reverting the ANGLE-resolve emulation, 2026-10-24) | Raw frame: 16 edge pixels per 3D view differ by one code (hardware resolve rounds 3-of-4-sample edges up). Cards: camera view 5 edge pixels, Editor view 12–18 edge pixels, all one level; no interior card pixel differs. | Engine 4× supersampled render of the same pinned view (16 pixels × 4× MSAA, box-averaged): camera-view edges are never further from it than today. In the Editor view, 4–5 edge pixels per card are one level further (e.g. engine 114, today 113, reference 107). | Camera view passes the reference rule. Editor view fails it: not approved; owner 3D lane (resolve moving to linear light) |

C3's 3D cards use the reference rule (pixels away from edges exact; a
differing edge pixel no further from the supersampled reference than today's)
since the 3D pass uses Metal's hardware resolve. 2D cards stay exact. Earlier
header SVG differences were Chromium tile raster history and are not engine
rendering; see [consumers.md](consumers.md).
