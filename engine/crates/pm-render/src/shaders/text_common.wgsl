// The ordered paint pass of text rasters (docs/engine/text-render.md).
//
// The shared area renderer writes float coverage pages 1:1 with device pixels.
// This pass paints each draw in order with premultiplied source-over in
// floating-point working precision:
//
// - kind 0, coverage: `color × coverage`;
// - kind 1, image: original bitmap texels in the working domain, with exact
//   area integration for downscaled output pixels, times draw alpha;
// - kind 2, layer: a pre-painted layer (a blurred draw) copied 1:1;
// - kind 3, direct: cached resolved line contours integrated in this pixel.
//
// Targets are top-down (row 0 at the top), like images.

struct Item {
    // The quad in target pixels: x0, y0, x1, y1.
    rect: vec4<f32>,
    // Page (or layer) texel = floor(fragment) + map.xy.
    map: vec4<f32>,
    // Coverage: premultiplied paint. Image: alpha multiplier in w.
    color: vec4<f32>,
    // x: kind, y: image needs area integration, zw: target size.
    params: vec4<f32>,
    // Image: target pixel to source pixel, rows (a, c, e) and (b, d, f).
    inv0: vec4<f32>,
    inv1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> it: Item;
@group(1) @binding(0) var src: texture_2d<f32>;
