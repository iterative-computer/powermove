// An SVG Gaussian on a filter grid aligned with its axes: the exact
// separable Gaussian of `composite/gaussian.wgsl` (appended), any σ at
// O(1) taps per pixel, with `svg_filter_common.wgsl` prepended.
// `p.mode`: 1 reads the input's alpha only, 2 converts it to linearRGB.

alias Elem = vec4<f32>;

@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(11) var<uniform> fe: Fe;

fn src_at(i: i32, j: i32) -> vec4<f32> {
    var c = textureLoad(src, vec2<i32>(i, j), 0);
    if ((p.mode & 1) != 0) { c = vec4<f32>(0.0, 0.0, 0.0, c.a); }
    if ((p.mode & 2) != 0 && c.a > 0.0) {
        c = vec4<f32>(fe_srgb_to_linear(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}

// The blur at the output pixel `pos` (`p.out`, grid pixels), stored back
// clipped to the output region.
@fragment fn fs_gauss(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let g = floor(pos.xy) + fe.origin.xy;
    return fe_out(grid_at(i32(g.x), i32(g.y))) * fe_cover(fe.s, g);
}
