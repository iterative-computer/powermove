// Bezier path mask coverage (`path_masks.rs`; docs/engine/composite.md
// "Path masks"), with `gaussian.wgsl` appended.
//
// Per mask: `raster` accumulates the signed area of its outline's edges in
// its pixel bounds (`p.src`), `scan` turns each row's running sum into
// nonzero coverage in place, the Gaussian passes feather it, and
// `composite` draws it onto the f32 coverage canvas `acc` (over `p.dst`,
// where any mask reaches) with `source-over` (mode 0) or `destination-out`
// (1). `fill` sets the canvas's start, `fs_store` writes it to r16float;
// a layer's only mask skips the canvas (`fs_single`).

alias Elem = f32;

// Outline coordinates are fixed point, `UNIT` per pixel; areas are
// accumulated in 1/`AREA` of a pixel.
const UNIT: i32 = 16384;
const AREA: f32 = 65536.0;
const AREA_PER_UNIT: i32 = 4;

@group(0) @binding(2) var<storage, read> cov: array<f32>;
@group(0) @binding(5) var<storage, read_write> accum: array<atomic<i32>>;
// The accumulator again, for the scan (plain loads and stores).
@group(0) @binding(11) var<storage, read_write> rows: array<i32>;
@group(0) @binding(6) var<storage, read> segments: array<vec4<i32>>;
@group(0) @binding(7) var<storage, read_write> acc: array<f32>;

fn src_at(i: i32, j: i32) -> f32 {
    return cov[u32(j * p.src.z + i)];
}

fn deposit(row: i32, col: i32, v: i32) {
    if (v != 0 && col >= 0 && col < p.src.z) {
        atomicAdd(&accum[u32(row * p.src.z + col)], v);
    }
}

// An edge piece inside pixel row `row`, from `u0` to `u1` (units right of
// column `base`), of signed height `q` (area units): each cell it crosses
// gets the area to the edge's right, the next cell the rest, so the row's
// running sum is the winding-weighted area. Shares are rounded so that
// they add up to `q` exactly: a closed outline leaves exactly zero.
fn piece(row: i32, base: i32, u0: f32, u1: f32, q: i32) {
    let lo = min(u0, u1);
    let hi = max(u0, u1);
    let unit = f32(UNIT);
    let first = i32(floor(lo / unit));
    let last = max(first, i32(ceil(hi / unit)) - 1);
    var done = 0;
    for (var c = first; c <= last; c++) {
        let a = max(lo, f32(c) * unit);
        let b = min(hi, f32(c + 1) * unit);
        var upto = q;
        if (c < last) {
            upto = i32(round(f32(q) * (b - lo) / (hi - lo)));
        }
        let share = upto - done;
        done = upto;
        let f = (0.5 * (a + b) - f32(c) * unit) / unit;
        let right = i32(round(f32(share) * (1.0 - f)));
        deposit(row, base + c, right);
        deposit(row, base + c + 1, share - right);
    }
}

// One outline edge per invocation (`p.first`, `p.count`), endpoints in
// units from the bounds' origin, x within them, y within their rows.
@compute @workgroup_size(64)
fn raster(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = i32(id.x);
    if (i >= p.count) {
        return;
    }
    let s = segments[u32(p.first + i)];
    var a = s.xy;
    var b = s.zw;
    if (a.y == b.y) {
        return;
    }
    var sign = 1;
    if (a.y > b.y) {
        let t = a;
        a = b;
        b = t;
        sign = -1;
    }
    let base = min(a.x, b.x) / UNIT;
    let ox = f32(a.x - base * UNIT);
    let dx = f32(b.x - a.x);
    let dy = f32(b.y - a.y);
    for (var row = a.y / UNIT; row <= (b.y - 1) / UNIT; row++) {
        let t0 = max(a.y, row * UNIT) - a.y;
        let t1 = min(b.y, (row + 1) * UNIT) - a.y;
        let u0 = ox + dx * (f32(t0) / dy);
        let u1 = ox + dx * (f32(t1) / dy);
        piece(row, base, u0, u1, sign * (t1 - t0) * AREA_PER_UNIT);
    }
}

var<workgroup> sums: array<i32, 256>;

// One row per workgroup: the running sum of the row's accumulator,
// written back as nonzero coverage (f32 bits).
@compute @workgroup_size(256)
fn scan(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let w = p.src.z;
    let row = i32(wg.x) * w;
    let chunk = (w + 255) / 256;
    let start = min(i32(t) * chunk, w);
    let end = min(start + chunk, w);
    var s = 0;
    for (var i = start; i < end; i++) {
        s += rows[u32(row + i)];
    }
    sums[t] = s;
    workgroupBarrier();
    for (var off = 1u; off < 256u; off <<= 1u) {
        var v = sums[t];
        if (t >= off) {
            v += sums[t - off];
        }
        workgroupBarrier();
        sums[t] = v;
        workgroupBarrier();
    }
    var run = 0;
    if (t > 0u) {
        run = sums[t - 1u];
    }
    for (var i = start; i < end; i++) {
        run += rows[u32(row + i)];
        rows[u32(row + i)] = bitcast<i32>(min(1.0, f32(abs(run)) / AREA));
    }
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

// Values are rounded to the nearest f16 here: the target's own store may
// truncate.
@fragment
fn fs_store(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(pos.xy));
    return vec4<f32>(quantizeToF16(acc[canvas(q.x, q.y)]), 0.0, 0.0, 1.0);
}

// A layer's only mask, drawn straight onto the start: covered exactly when
// it subtracts.
@fragment
fn fs_single(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(pos.xy));
    let v = clamp(grid_at(q.x, q.y), 0.0, 1.0);
    return vec4<f32>(quantizeToF16(draw(f32(p.mode), v)), 0.0, 0.0, 1.0);
}
