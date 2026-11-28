// The source is logical level zero. Each level box filters the level
// above, truncating odd dimensions and extending one-texel edges.
@group(0) @binding(0) var above: texture_2d<f32>;
@group(0) @binding(1) var below: texture_storage_2d<STORE, write>;
@group(0) @binding(2) var second: texture_storage_2d<STORE, write>;
@group(0) @binding(3) var third: texture_storage_2d<STORE, write>;
@group(0) @binding(4) var fourth: texture_storage_2d<STORE, write>;

fn first_box(id: vec2<u32>) -> vec4<f32> {
    let last = vec2<i32>(textureDimensions(above)) - vec2<i32>(1);
    let p = vec2<i32>(id) * 2;
    let a = textureLoad(above, min(p, last), 0);
    let b = textureLoad(above, min(p + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(above, min(p + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(above, min(p + vec2<i32>(1, 1), last), 0);
    return store((a + b + c + d) * 0.25);
}

@compute @workgroup_size(8, 8)
fn cs_down(@builtin(global_invocation_id) id: vec3<u32>) {
    if (all(id.xy < textureDimensions(below))) {
        textureStore(below, vec2<i32>(id.xy), first_box(id.xy));
    }
}

var<workgroup> tile: array<vec4<f32>, 64>;

fn seed(id: vec2<u32>, lane: u32) {
    if (all(id < textureDimensions(below))) {
        let value = first_box(id);
        tile[lane] = value;
        textureStore(below, vec2<i32>(id), value);
    }
}

// Each participating lane owns a disjoint 2×2 group of the previous
// level. Keep that group's result in its top-left lane. A size of one
// in either direction repeats the same lane, just like the texture path.
fn next_box(lane: u32, stride: u32, size: vec2<u32>) -> vec4<f32> {
    let dx = select(stride, 0u, size.x == 1u);
    let dy = select(stride * 8u, 0u, size.y == 1u);
    let value = (tile[lane] + tile[lane + dx] + tile[lane + dy] + tile[lane + dx + dy]) * 0.25;
    let stored = store(value);
    tile[lane] = stored;
    return stored;
}

fn write_second(id: vec2<u32>, lane: u32) {
    if (all(id % 2u == vec2<u32>(0)) && all(id / 2u < textureDimensions(second))) {
        textureStore(second, vec2<i32>(id / 2u), next_box(lane, 1u, textureDimensions(below)));
    }
}

fn write_third(id: vec2<u32>, lane: u32) {
    if (all(id % 4u == vec2<u32>(0)) && all(id / 4u < textureDimensions(third))) {
        textureStore(third, vec2<i32>(id / 4u), next_box(lane, 2u, textureDimensions(second)));
    }
}

@compute @workgroup_size(8, 8)
fn cs_down2(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    seed(id.xy, lane);
    workgroupBarrier();
    write_second(id.xy, lane);
}

@compute @workgroup_size(8, 8)
fn cs_down3(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    seed(id.xy, lane);
    workgroupBarrier();
    write_second(id.xy, lane);
    workgroupBarrier();
    write_third(id.xy, lane);
}

@compute @workgroup_size(8, 8)
fn cs_down4(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    seed(id.xy, lane);
    workgroupBarrier();
    write_second(id.xy, lane);
    workgroupBarrier();
    write_third(id.xy, lane);
    workgroupBarrier();
    if (lane == 0u && all(id.xy / 8u < textureDimensions(fourth))) {
        textureStore(fourth, vec2<i32>(id.xy / 8u), next_box(lane, 4u, textureDimensions(third)));
    }
}
