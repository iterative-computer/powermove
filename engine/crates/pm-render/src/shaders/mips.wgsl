// The mip pyramid of level 2 sources (docs/engine/motion.md): level 0
// copied texel for texel, each further level the 2×2 box mean of the level
// above (edge texels repeat for odd sizes).

@group(0) @binding(0) var above: texture_2d<f32>;

struct Full {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Full {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    var o: Full;
    o.pos = vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs_copy(in: Full) -> @location(0) vec4<f32> {
    return textureLoad(above, vec2<i32>(in.pos.xy), 0);
}

@fragment
fn fs_down(in: Full) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(above)) - vec2<i32>(1, 1);
    let p = vec2<i32>(in.pos.xy) * 2;
    let a = textureLoad(above, min(p, last), 0);
    let b = textureLoad(above, min(p + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(above, min(p + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(above, min(p + vec2<i32>(1, 1), last), 0);
    return (a + b + c + d) * 0.25;
}
