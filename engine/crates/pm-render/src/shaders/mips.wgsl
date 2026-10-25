// The physical tail of a level 2 source pyramid. The source is borrowed as
// logical level 0; each stored level is the 2×2 box mean of the level above,
// with the same floor-halved dimensions and clamped edge reads as before.

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
fn fs_down(in: Full) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(above)) - vec2<i32>(1, 1);
    let p = vec2<i32>(in.pos.xy) * 2;
    let a = textureLoad(above, min(p, last), 0);
    let b = textureLoad(above, min(p + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(above, min(p + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(above, min(p + vec2<i32>(1, 1), last), 0);
    return (a + b + c + d) * 0.25;
}
