// The physical tail of a level 2 source pyramid. The source is borrowed as
// logical level 0; each stored level is the 2×2 box mean of the level above,
// with floor-halved dimensions and clamped edge reads. One compute pass
// writes every level, a dispatch each. `STORE` is the storage texel format;
// `store` rounds half-float levels to nearest (Metal's conversion truncates).

@group(0) @binding(0) var above: texture_2d<f32>;
@group(0) @binding(1) var below: texture_storage_2d<STORE, write>;

@compute @workgroup_size(8, 8)
fn cs_down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(below);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let last = vec2<i32>(textureDimensions(above)) - vec2<i32>(1, 1);
    let p = vec2<i32>(id.xy) * 2;
    let a = textureLoad(above, min(p, last), 0);
    let b = textureLoad(above, min(p + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(above, min(p + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(above, min(p + vec2<i32>(1, 1), last), 0);
    textureStore(below, vec2<i32>(id.xy), store((a + b + c + d) * 0.25));
}
