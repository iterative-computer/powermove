// Exact area coverage of filled outlines (`coverage.rs`).
//
// `raster` accumulates each edge's signed area per pixel of its item's
// bounds into integer atomics; `scan` (one workgroup per item row) turns
// each row's running sum into nonzero coverage, `min(1, |area|)`, written
// back in place as f32 bits and, for `scan_page`, into the item's cell of
// an r32float page.

// Edge coordinates are fixed point, `UNIT` per pixel; areas are
// accumulated in 1/`AREA` of a pixel.
const UNIT: i32 = 16384;
const AREA: f32 = 65536.0;
const AREA_PER_UNIT: i32 = 4;

struct Item {
    // Start of the item's rows in `accum`.
    offset: u32,
    width: u32,
    height: u32,
    // Its cell in the page.
    at_x: u32,
    at_y: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct Counts {
    edges: u32,
    rows: u32,
};

@group(0) @binding(0) var<uniform> counts: Counts;
@group(0) @binding(1) var<storage, read> items: array<Item>;
@group(0) @binding(2) var<storage, read> edges: array<vec4<i32>>;
@group(0) @binding(3) var<storage, read> edge_items: array<u32>;
// (item, row) of each scan workgroup.
@group(0) @binding(4) var<storage, read> rows_of: array<vec2<u32>>;
@group(0) @binding(5) var<storage, read_write> accum: array<atomic<i32>>;
// The accumulator again, for the scan (plain loads and stores).
@group(0) @binding(6) var<storage, read_write> plain: array<i32>;
@group(0) @binding(7) var page: texture_storage_2d<r32float, write>;

fn deposit(item: Item, row: i32, col: i32, v: i32) {
    if (v != 0 && col >= 0 && col < i32(item.width)) {
        atomicAdd(&accum[item.offset + u32(row) * item.width + u32(col)], v);
    }
}

// An edge piece inside pixel row `row`, from `u0` to `u1` (units right of
// column `base`), of signed height `q` (area units): each cell it crosses
// gets the area to the edge's right, the next cell the rest, so the row's
// running sum is the winding-weighted area. Shares are rounded so that
// they add up to `q` exactly: a closed outline leaves exactly zero.
fn piece(item: Item, row: i32, base: i32, u0: f32, u1: f32, q: i32) {
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
        deposit(item, row, base + c, right);
        deposit(item, row, base + c + 1, share - right);
    }
}

// One edge per invocation, endpoints in units from its item's origin, x
// within the item, y within its rows.
@compute @workgroup_size(64)
fn raster(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= counts.edges) {
        return;
    }
    let item = items[edge_items[id.x]];
    let s = edges[id.x];
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
        piece(item, row, base, u0, u1, sign * (t1 - t0) * AREA_PER_UNIT);
    }
}

var<workgroup> sums: array<i32, 256>;

// The running sum of scan row `r` (`rows_of`) as coverage, in place: this
// invocation's chunk of the row.
fn scan_row(r: u32, t: u32) -> vec2<i32> {
    let at = rows_of[r];
    let item = items[at.x];
    let w = i32(item.width);
    let row = item.offset + at.y * item.width;
    let chunk = (w + 255) / 256;
    let start = min(i32(t) * chunk, w);
    let end = min(start + chunk, w);
    var s = 0;
    for (var i = start; i < end; i++) {
        s += plain[row + u32(i)];
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
        run += plain[row + u32(i)];
        plain[row + u32(i)] = bitcast<i32>(min(1.0, f32(abs(run)) / AREA));
    }
    return vec2<i32>(start, end);
}

@compute @workgroup_size(256)
fn scan(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = wg.x + wg.y * 65535u;
    if (r >= counts.rows) {
        return;
    }
    scan_row(r, t);
}

// `scan`, and each value into the item's cell of the page.
@compute @workgroup_size(256)
fn scan_page(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let r = wg.x + wg.y * 65535u;
    if (r >= counts.rows) {
        return;
    }
    let span = scan_row(r, t);
    let at = rows_of[r];
    let item = items[at.x];
    let row = item.offset + at.y * item.width;
    for (var i = span.x; i < span.y; i++) {
        let c = bitcast<f32>(plain[row + u32(i)]);
        textureStore(page, vec2<u32>(item.at_x + u32(i), item.at_y + at.y), vec4<f32>(c, 0.0, 0.0, 0.0));
    }
}
