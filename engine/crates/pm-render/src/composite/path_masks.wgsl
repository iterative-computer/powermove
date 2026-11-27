// Bezier path masks (`path_masks.rs`; docs/engine/composite.md "Path
// masks"), with `gaussian.wgsl` appended.
//
// Per mask, its coverage (`crate::coverage`, f32 over its pixel bounds
// `p.src`) is feathered by the Gaussian passes and `composite` draws it onto
// the f32 canvas `acc` (over `p.dst`, where any mask reaches) with
// `source-over` (mode 0) or `destination-out` (1). `fill` sets the canvas's
// start, `fs_store` writes it to r16float; a layer's only mask skips the
// canvas (`fs_single`).

alias Elem = f32;

@group(0) @binding(2) var<storage, read> cov: array<f32>;
@group(0) @binding(7) var<storage, read_write> acc: array<f32>;

fn src_at(i: i32, j: i32) -> f32 {
    return cov[u32(j * p.src.z + i)];
}

fn canvas(x: i32, y: i32) -> u32 {
    return u32((y - p.dst.y) * p.dst.z + x - p.dst.x);
}

@compute @workgroup_size(16, 16)
fn fill(@builtin(global_invocation_id) id: vec3<u32>) {
    let q = vec2<i32>(id.xy);
    if (q.x < p.dst.z && q.y < p.dst.w) {
        acc[u32(q.y * p.dst.z + q.x)] = f32(p.mode);
    }
}

@compute @workgroup_size(16, 16)
fn composite(@builtin(global_invocation_id) id: vec3<u32>) {
    let q = vec2<i32>(id.xy);
    if (q.x >= p.out.z || q.y >= p.out.w) {
        return;
    }
    let x = p.out.x + q.x;
    let y = p.out.y + q.y;
    let v = clamp(grid_at(x, y), 0.0, 1.0);
    let i = canvas(x, y);
    acc[i] = draw(acc[i], v);
}

fn draw(a: f32, v: f32) -> f32 {
    if (p.mode == 0) {
        return v + a * (1.0 - v);
    }
    return a * (1.0 - v);
}

// Values are rounded to the nearest-even f16 here (`nearest_half.rs`): the
// target's own store, and `quantizeToF16`, may truncate.
@fragment
fn fs_store(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(pos.xy));
    return vec4<f32>(pm_scene_nearest_half(acc[canvas(q.x, q.y)]), 0.0, 0.0, 1.0);
}

// A layer's only mask, drawn straight onto the start: covered exactly when
// it subtracts.
@fragment
fn fs_single(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(pos.xy));
    let v = clamp(grid_at(q.x, q.y), 0.0, 1.0);
    return vec4<f32>(pm_scene_nearest_half(draw(f32(p.mode), v)), 0.0, 0.0, 1.0);
}
