# pm-skia-path

`tiny-skia-path` 0.12.0 (BSD-3-Clause, Copyright 2020 Yevhenii Reizner, a
port of Skia's path code, Copyright The Android Open Source Project and
Google LLC), with conics kept as Skia's `SkPath` keeps them instead of
converted to quads where they are added, and Skia m149's
`SkAutoConicToQuads` (`path_geometry::skia_conic_to_quads`).

pm-text frames glyph outlines with its stroker (synthesized bold,
`strokeText`) exactly as Chromium's Skia does: the round joins and caps are
conics in user space, subdivided only in device space where the glyph mask
is filled (`docs/engine/text-render.md`, "Glyph frames"). Dashing is
removed. See `LICENSE`.
