# pm-skia-path

`tiny-skia-path` 0.12.0 (BSD-3-Clause, Copyright 2020 Yevhenii Reizner, a
port of Skia's path code, Copyright The Android Open Source Project and
Google LLC), with conics kept as Skia's `SkPath` keeps them instead of
converted to quads where they are added, and Skia m149's
`SkAutoConicToQuads` (`path_geometry::skia_conic_to_quads`).

pm-text frames glyph outlines with its stroker (synthesized bold,
`strokeText`) exactly as Chromium's Skia does: the round joins and caps are
conics in user space, subdivided only in device space where the glyph mask
is filled. Dashing is
removed. See `LICENSE`.

The stroker also retains incoming rational conics until stroking:
`SkPathStroker::conicTo`, `conicPerpRay`, `compareQuadConic`, and
`conicStroke` follow Skia chrome/m152 at
`747cf654ed6f864f4402e7d6eb54a6ede60141e5`. Its curve evaluation,
tangent vectors, recursion limits and terminal-line behavior use that
source's operation order. The old pre-stroke conic-to-quad approximation
is removed. `tests/skia_conics.rs` compares five complete stroke records
bitwise (verbs, point bits and weights); `tests/skia-conic-oracle.cpp`
regenerates their hashes against the pinned Skia library with
`-ffp-contract=off`.

Closed rectangles use that revision's `SkStroke::strokeRect` fast path,
including collapsed interiors and stroke-and-fill framing. Five additional
stroke records and `tests/skia-rect-oracle.cpp` cover its joins and winding.
`Path::is_convex` ports Skia's control-polygon classifier so pm-text's
analytic scan converter can select the same convex edge walk.
