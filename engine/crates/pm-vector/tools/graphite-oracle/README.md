# Chromium 152 Graphite source oracles

`../graphite-shader-oracle.cpp` invokes Skia's Graphite SkSL compiler modules.
The stage wrappers call the unmodified analytic rectangle, fill/stroke
curve tessellation and SDF text functions, with ShaderInfo's viewport
expression. The Skia revision is
`747cf654ed6f864f4402e7d6eb54a6ede60141e5` (chrome/m152).

Electron 44.0.0 pins Chromium 152.0.7977.54. Its Skia revision is
`b6d106297ff9ef2ff8094033695d045e87775581`; the relevant Graphite shader
modules and analytic/stroke render-step sources are identical at both
revisions. Its Dawn revision is
`571752c9cb3ed36a3194854374984427794c4dda`.

Chromium's `gpu/command_buffer/service/dawn_context_provider.cc` disables
`SkiaGraphiteF16` by default. Skia's `DawnGraphicsPipeline.cpp` sets
`fForceHighPrecision` when the device lacks ShaderF16, as well as
`fForceNoRTFlip` and `fSharpenTextures`. `generate.py` uses those settings.
Dawn's `ShaderModuleMTL.mm` selects relaxed Metal math unless strict math
is requested; the generated MSL carries that pragma.

Build pinned Skia with `skia_enable_graphite=true` and `skia_use_metal=true`.
Compile the extractor against that build, for example on macOS:

```sh
clang++ -std=c++20 -O2 -ffp-contract=off -DSK_GRAPHITE -DSK_METAL \
  -I"$SKIA_SRC" ../graphite-shader-oracle.cpp \
  "$SKIA_OUT/libskia.a" "$SKIA_OUT/libskcms.a" \
  -framework Metal -framework Foundation -framework CoreFoundation \
  -framework CoreGraphics -framework CoreText -framework IOSurface \
  -o graphite-shader-oracle
```

Build the pinned Dawn `tint` CMake target with the WGSL reader and MSL
writer enabled. Other writers, tests, samples, protobuf and graphics
backends are unnecessary. Run:

```sh
python3 generate.py /path/to/graphite-shader-oracle /path/to/tint
```

Outputs are the adjacent pm-render crate's `src/text/graphite/*.wgsl` and
`*.metal`. The script removes only Chromium's diagnostic directive and
remaps the separate texture/sampler bindings to the common draw layout.
It uses Tint's single-entry-point writer with symbol renaming; the host
bounds every patch access by its vertex and instance draw ranges.

`counts.cpp` independently calls Skia's `LinearTolerances`, Wang formulae
and `FixedCount{Wedges,Strokes}::VertexCount`. Build it against the same
Skia library with `-ffp-contract=off`. It reports the line, quad, cubic and
rational-conic allocation counts at scales 1 and 5, with round, bevel and
miter joins. These synthetic records contain no fonts or captured pixels.
