// The bridge test source (pm-gpu bridge_out::test_source): what the frame
// bridge's tests and the S1 harness show until pm-render draws real frames.
// Writes working values (display encoding E, extended past 1.0) into the
// rgba16float working target; the display transform turns them into the
// pool surface's format.
//
// Rows (frame pixels, y down; checked by tests and the harness):
//   [0, 32)      frame code: 64 cells of `cell` px; 32 bits of seq, then 32
//                bits of the view sequence
//   [64, 320)    continuous ramp 0..1 across the width (dithered on bgra)
//   [320, 576)   the same ramp on the 8-bit grid (exact through bgra and capture)
//   [576, 832)   8 patches: 0.25 0.5 0.75 1.0 1.25 1.5 2.0 4.0
//   [832, 1088)  a 64 px bar moving at 960 px/s (presentation smoothness)
//   elsewhere    0.18 grey with a 120 px grid, the composition rectangle's
//                outline, and the marker (the overlay geometry), in orange

struct Uniforms {
    size: vec2f,
    time_s: f32,
    flags: u32,          // bit0: overlay only (3D underneath)
    marker: vec4f,       // x, y, w, h in frame pixels
    composition: vec4f,  // x, y, w, h in frame pixels
    seq_lo: u32,
    view_lo: u32,
    cell: f32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

const PATCHES = array<f32, 8>(0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0);

fn code_bit(cell: u32) -> f32 {
    if cell < 32u { return f32((u.seq_lo >> cell) & 1u); }
    return f32((u.view_lo >> (cell - 32u)) & 1u);
}

fn inside(x: f32, y: f32, r: vec4f) -> bool {
    return x >= r.x && x < r.x + r.z && y >= r.y && y < r.y + r.w;
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let x = floor(pos.x);
    let y = floor(pos.y);
    let overlay_only = (u.flags & 1u) != 0u;
    var c = vec3f(0.18);
    var drawn = false;

    if y < 32.0 && x < 64.0 * u.cell {
        c = vec3f(code_bit(u32(x / u.cell)));
        drawn = true;
    } else if !overlay_only {
        drawn = true;
        if y >= 64.0 && y < 320.0 {
            c = vec3f((x + 0.5) / u.size.x);
        } else if y >= 320.0 && y < 576.0 {
            c = vec3f(round((x + 0.5) / u.size.x * 255.0) / 255.0);
        } else if y >= 576.0 && y < 832.0 {
            let idx = min(u32(x / (u.size.x / 8.0)), 7u);
            c = vec3f(PATCHES[idx]);
        } else if y >= 832.0 && y < 1088.0 {
            let bx = (u.time_s * 960.0) % u.size.x;
            c = select(vec3f(0.2), vec3f(1.0), x >= bx && x < bx + 64.0);
        } else {
            let g = (x % 120.0) < 2.0 || (y % 120.0) < 2.0;
            c = select(vec3f(0.18), vec3f(0.3), g);
            let r = u.composition;
            let outline = inside(x, y, r) && !inside(x, y, vec4f(r.x + 2.0, r.y + 2.0, r.z - 4.0, r.w - 4.0));
            if outline { c = vec3f(0.85); }
        }
    }
    if inside(x, y, u.marker) && y >= 32.0 {
        c = vec3f(1.0, 0.5, 0.0);
        drawn = true;
    }
    if !drawn {
        discard;
    }
    return vec4f(c, 1.0);
}

// Synthetic GPU load: a full-screen pass with some ALU per pixel into a
// scratch target (the S1 harness's engine load).
@fragment
fn fs_load(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    var v = pos.xy / u.size;
    for (var i = 0u; i < 16u; i++) {
        v = fract(v * 1.618 + vec2f(sin(v.y * 6.28 + u.time_s), cos(v.x * 6.28)));
    }
    return vec4f(v, 0.0, 1.0);
}
