# Compositing

The graph preserves the document's nine blend formulas, masks, mattes, group
boundaries, adjustment layers, effect order, opacity and timing. Standard projects
composite premultiplied display values; linear projects decode inputs and encode
the finished frame. Gradients retain their authored interpolation space. Project
color depth selects delivery depth and float precision. Eight-bit delivery uses
RGBA16F intermediates so extended colors survive to gamut mapping; 16/32-bit
projects use RGBA16F/RGBA32F.

A layer boundary draws content, applies masks, runs effects, applies its matte,
then composites. Fused mask/matte arithmetic stays in f32 until an actual texture
store. There are no synthetic intermediate quantizations or GPU rounding probes.
Effect programs continue to receive their declared display-encoded inputs.

Blend layers render into conservative device bounds, including analytic sampling
coverage. The compositor snapshots only the covered destination into scratch
storage and writes the blend into that region with a load/scissor pass. Uncovered
accumulator pixels keep their exact bits. Unmasked layer targets and scratch
storage return to the pool after their last recorded read; depth/ID readers retain
what they need until submission. Effects and transitions retain the complete pass
rectangle because they can read the backdrop or produce pixels beyond the input.
Region-of-interest output keeps full-frame coordinates and expands for effect
padding, so a region equals the corresponding full-frame pixels.

Image, video, precomp, shader and extension raster layers use the same sampler at
every compatibility level. Final and paused frames use the declared Lanczos-3
kernel with its anti-ringing clamp; playback uses trilinear filtering. Quad edges
use exact pixel-area coverage. Draws whose kernel reads only mip zero borrow the
source directly; minification uses a box-filtered pyramid. Retained pyramids
are bounded by both 16 sources and 128 MiB. Mip zero borrows the source; physical
levels retain only a float downsample tail. The first box level can be evaluated
in the draw. Known source revisions reuse texels; changed or unknown revisions
regenerate before use.
Authored text layout, vector geometry and extension raster override semantics
remain separate contracts.

The viewer renders Full directly at its snapped device box in the project's
working format, without an intermediate opaque RGBA8 canvas. Half/Quarter are
explicit resolution choices. Pinned snapshots keep project precision until their
API output conversion. At 100%, paused preview and export use the same frame
pipeline. Equality requires the same motion-blur sample count and scene view;
viewer-only camera/look overrides and the authored per-output sample settings
remain explicit request inputs. Video frame selection follows source timestamps
and holds the last source frame on every output. Paused compatibility lookahead
starts after the exact visible frame presents; a seek invalidates that warmup,
and future rasters yield to current-frame dependencies.

3D color reaches the compositor as resolved, premultiplied linear float values.
The output boundary applies the authored ACES curve once after 3D blending and
resolve, joins the display-referred background in linear light, and encodes once
for Standard compositing. Masks and effects then run in their existing layer
order. Legacy OBJ shader output has a separate encoded contract.

At an 8-bit sRGB output, the shared CSS Color 4 mapper reduces OKLCh chroma
before final quantization. It operates on straight encoded RGB, then restores
premultiplication when requested. Float and Display P3 outputs retain their
extended values. BT.709-tagged delivery converts the mapped sRGB result to the
declared transfer. Genuine byte rasters retain their existing byte contract.

The correctness gates are described in [gate-policy.md](gate-policy.md).
`tests/composite.rs` guards formulas, isolation, region equivalence, exact identity
outside blend bounds and the cold blend-stack texture budget. `tests/sampling.rs`
applies the declared-kernel quality and flicker bars to both compatibility levels.
`tests/corpus.rs` keeps today's captures as authored-intent witnesses and compares
modeled content with the independent f64 reference. Unmodeled content retains its
previous bound and remains a failing gate when the bound is exceeded.
