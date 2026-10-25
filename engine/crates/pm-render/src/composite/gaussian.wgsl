// The exact separable Gaussian (`gaussian.rs`). The including module
// defines `Elem` (f32 or vec4<f32>) and `src_at(i, j)`: the source at
// (i, j) from its origin, inside it.
//
// Stage 1 (`h_pass`, `v_pass`): the box-integrated Gaussian at the nodes
// `p.grid` (every pixel when `p.step` is 1). With nodes `s` px apart,
// stage 2 (`c_h`, `c_v`) adds the rest of σ on the node lattice, `i_x`
// interpolates rows and `grid_at` columns (6-point Lagrange).

struct P {
    // Origin (xy, target px) and size (zw).
    src: vec4<i32>,
    // Stage 1 nodes: origin (px) and counts.
    grid: vec4<i32>,
    out: vec4<i32>,
    dst: vec4<i32>,
    step: i32,
    radius: i32,
    mode: i32,
    wbase: i32,
    _pad0: i32,
    _pad1: i32,
    cradius: i32,
    cwbase: i32,
    // The vertical axis's kernel (`Params::blur_xy`).
    ystep: i32,
    yradius: i32,
    ywbase: i32,
    ycradius: i32,
    ycwbase: i32,
    _pad2: i32,
    _pad3: i32,
    _pad4: i32,
};

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<storage, read_write> hbuf: array<Elem>;
@group(0) @binding(8) var<storage, read_write> ubuf: array<Elem>;
@group(0) @binding(9) var<storage, read_write> h2buf: array<Elem>;
@group(0) @binding(4) var<storage, read_write> vbuf: array<Elem>;
@group(0) @binding(10) var<storage, read_write> ixbuf: array<Elem>;

// The final nodes: stage 1's without stage 2's margin.
fn nodes() -> vec2<i32> {
    return p.grid.zw - 2 * vec2<i32>(p.cradius, p.ycradius);
}

fn origin() -> vec2<i32> {
    return p.grid.xy + vec2<i32>(p.cradius * p.step, p.ycradius * p.ystep);
}

@compute @workgroup_size(64, 4)
fn h_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = i32(id.x);
    let j = i32(id.y);
    if (k >= p.grid.z || j >= p.src.w) {
        return;
    }
    let x = p.grid.x + k * p.step - p.src.x;
    let lo = max(-p.radius, -x);
    let hi = min(p.radius, p.src.z - 1 - x);
    let w0 = p.wbase + p.radius;
    var s = Elem(0.0);
    for (var d = lo; d <= hi; d++) {
        s += weights[u32(w0 + d)] * src_at(x + d, j);
    }
    hbuf[u32(j * p.grid.z + k)] = s;
}

@compute @workgroup_size(64, 4)
fn v_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = i32(id.x);
    let m = i32(id.y);
    if (k >= p.grid.z || m >= p.grid.w) {
        return;
    }
    let y = p.grid.y + m * p.ystep - p.src.y;
    let lo = max(-p.yradius, -y);
    let hi = min(p.yradius, p.src.w - 1 - y);
    let w0 = p.ywbase + p.yradius;
    var s = Elem(0.0);
    for (var d = lo; d <= hi; d++) {
        s += weights[u32(w0 + d)] * hbuf[u32((y + d) * p.grid.z + k)];
    }
    ubuf[u32(m * p.grid.z + k)] = s;
}

@compute @workgroup_size(64, 4)
fn c_h(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = nodes();
    let k = i32(id.x);
    let m = i32(id.y);
    if (k >= n.x || m >= p.grid.w) {
        return;
    }
    let w0 = p.cwbase + p.cradius;
    let row = m * p.grid.z + k + p.cradius;
    var s = Elem(0.0);
    for (var d = -p.cradius; d <= p.cradius; d++) {
        s += weights[u32(w0 + d)] * ubuf[u32(row + d)];
    }
    h2buf[u32(m * n.x + k)] = s;
}

@compute @workgroup_size(64, 4)
fn c_v(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = nodes();
    let k = i32(id.x);
    let m = i32(id.y);
    if (k >= n.x || m >= n.y) {
        return;
    }
    let w0 = p.ycwbase + p.ycradius;
    var s = Elem(0.0);
    for (var d = -p.ycradius; d <= p.ycradius; d++) {
        s += weights[u32(w0 + d)] * h2buf[u32((m + p.ycradius + d) * n.x + k)];
    }
    vbuf[u32(m * n.x + k)] = s;
}

// Lagrange weights of the nodes at -2 … 3 for a point `t` in [0, 1).
fn lagrange6(t: f32) -> array<f32, 6> {
    let a = t + 2.0;
    let b = t + 1.0;
    let d = t - 1.0;
    let e = t - 2.0;
    let f = t - 3.0;
    return array<f32, 6>(
        -(b * t * d * e * f) / 120.0,
        (a * t * d * e * f) / 24.0,
        -(a * b * d * e * f) / 12.0,
        (a * b * t * e * f) / 12.0,
        -(a * b * t * d * f) / 24.0,
        (a * b * t * d * e) / 120.0,
    );
}

// Rows of the final nodes interpolated at every output column.
@compute @workgroup_size(64, 4)
fn i_x(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = nodes();
    let x = i32(id.x);
    let m = i32(id.y);
    if (x >= p.out.z || m >= n.y) {
        return;
    }
    let f = p.out.x + x - origin().x;
    let k = f / p.step;
    var l = lagrange6(f32(f - k * p.step) / f32(p.step));
    var s = Elem(0.0);
    for (var a = 0; a < 6; a++) {
        s += l[a] * vbuf[u32(m * n.x + k - 2 + a)];
    }
    ixbuf[u32(m * p.out.z + x)] = s;
}

// The blur at target pixel (x, y), inside `p.out`.
fn grid_at(x: i32, y: i32) -> Elem {
    if (p.step == 1 && p.ystep == 1) {
        return ubuf[u32((y - p.grid.y) * p.grid.z + x - p.grid.x)];
    }
    let f = y - origin().y;
    let k = f / p.ystep;
    var l = lagrange6(f32(f - k * p.ystep) / f32(p.ystep));
    let col = x - p.out.x;
    var s = Elem(0.0);
    for (var b = 0; b < 6; b++) {
        s += l[b] * ixbuf[u32((k - 2 + b) * p.out.z + col)];
    }
    return s;
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
