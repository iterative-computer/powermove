# 3D scene renderer

`pm-render/src/scene3d` evaluates object, camera, light, material, and view state from
the document. `pm-geom3d` supplies the authored primitives and mesh data. The draw
description preserves r180's material and lighting formulas, texture placement, object
order, double-sided behavior, layer depth sets, and editor views. Picking, bounds, and
depth/ID remain exact document answers.

## Color

Object and whole-scene color draw into multisampled `Rgba16Float` in linear sRGB.
After the last draw each sample is weighted by 1/(1 + L), L the luminance the tone curve
sees (`scene3d::karis`), and hardware resolves the four samples in linear light; the
seam undoes the weighting. Opaque draws write straight shader
color; transparent draws blend into a premultiplied linear target. The resolved target
stays linear and untoned. `Rendered3d::color_space` carries the scene exposure and
optional authored display background to the 2D compositor. At that seam, the compositor
unpremultiplies, applies r180 ACES Filmic with `exposure / 0.6` once after all 3D
blending, encodes once for Standard compositing (or retains tonemapped linear values for
linear compositing), and puts the authored background behind the object without tone
mapping it. Legacy OBJ mesh output is already encoded and is identified separately.

The 3D pass writes each object layer into the 2D graph independently. Masks, effects,
opacity, blend mode, and mattes retain their layer positions in that graph. The layer
color target clears transparent even when the bottom layer has a scene background; the
seam adds that background after tone mapping.

## Depth and shadows

An opaque, unmasked object writes the depth shared by its depth set in the prepass. An
object excluded from that prepass by an enabled mask or low layer opacity still
depth-tests and writes its own opaque faces during its color pass, so a far hemisphere
cannot appear through its front hemisphere. That pass invalidates the shared prepass
before the next layer. Transparent materials keep their authored depth-write state and
back-then-front order.

Shadow casters render into `Depth32Float` depth-only targets. Directional and spot
casters store projected depth; point casters write normalized radial distance into the
six-view atlas. A comparison sampler performs each depth test in the existing r180
PCF-soft tap pattern. Bias, normal bias, map and frustum sizes, caster/light selection,
and alpha-tested cutouts remain authored constants.

## Verification

`tests/scene3d*.rs` cover evaluated scenes, ordering, depth/ID, material programs and
uniforms, maps, resource reuse, and editor views. `tests/threed_maps.rs` keeps the
supported map-texel and linear mip checks. Native straight-alpha source normalization is
the media reader's separate contract. The pixel-quality gate compares native and today's
captures separately with an independent supersampled f32 reference; neither captured
WebGL codes nor an enlarged native render are the reference. Library cards use the same
principle at their pinned backing size. See [gate-policy.md](gate-policy.md) and
[quality-ledger.md](quality-ledger.md) for thresholds and measured corrections.
