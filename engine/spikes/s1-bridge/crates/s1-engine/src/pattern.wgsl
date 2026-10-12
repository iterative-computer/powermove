// Spike S1 test pattern, rendered at the pool's device-pixel size.
//
// Rows (frame pixels, y down):
//   [0, 32)       frame code: 32 cells of seq bits, then 32 cells of revision bits (32 px each)
//   [64, 320)     ramp 0..1 across the full width, dithered on the 8-bit path
//   [320, 576)    the same ramp, never dithered (reference for banding)
//   [576, 832)    8 patches: 0.25 0.5 0.75 1.0 1.25 1.5 2.0 4.0 (E-encoded, extended)
//   [832, 1088)   a 64 px bar moving at 960 px/s (presentation smoothness)
//   elsewhere     0.18 grey with a 120 px grid
// The dragged marker (overlay geometry) is drawn last, in orange.

struct Uniforms {
    size: vec2f,
    time_s: f32,
    flags: u32,          // bit0: 8-bit target, bit1: dither, bit2: overlay only (3D underneath)
    marker: vec4f,       // x, y, w, h in frame pixels
    seq_lo: u32,
    rev_lo: u32,
    noise_off: vec2u,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var noise: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

const PATCHES = array<f32, 8>(0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0);

fn code_bit(cell: u32) -> f32 {
    if cell < 32u { return f32((u.seq_lo >> cell) & 1u); }
    return f32((u.rev_lo >> (cell - 32u)) & 1u);
}

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let x = floor(pos.x);
    let y = floor(pos.y);
    let eight_bit = (u.flags & 1u) != 0u;
    let overlay_only = (u.flags & 4u) != 0u;
    var c = vec3f(0.18);
    var dither = false;
    var drawn = false;

    if y < 32.0 && x < 64.0 * 32.0 {
        c = vec3f(code_bit(u32(x / 32.0)));
        drawn = true;
    } else if !overlay_only {
        drawn = true;
        if y >= 64.0 && y < 576.0 {
            c = vec3f((x + 0.5) / u.size.x);
            dither = y < 320.0 && (u.flags & 2u) != 0u;
        } else if y >= 576.0 && y < 832.0 {
            let idx = min(u32(x / (u.size.x / 8.0)), 7u);
            c = vec3f(PATCHES[idx]);
        } else if y >= 832.0 && y < 1088.0 {
            let bx = (u.time_s * 960.0) % u.size.x;
            c = select(vec3f(0.2), vec3f(1.0), x >= bx && x < bx + 64.0);
        } else {
            let g = (x % 120.0) < 2.0 || (y % 120.0) < 2.0;
            c = select(vec3f(0.18), vec3f(0.3), g);
        }
    }
    let m = u.marker;
    if x >= m.x && x < m.x + m.z && y >= m.y && y < m.y + m.w && y >= 32.0 {
        c = vec3f(1.0, 0.5, 0.0);
        dither = false;
        drawn = true;
    }
    if !drawn {
        discard;
    }
    if eight_bit {
        c = clamp(c, vec3f(0.0), vec3f(1.0));
        if dither {
            // Blue-noise RPDF of +-0.5 LSB; values already on the 8-bit grid are unchanged.
            let t = (vec2u(u32(x), u32(y)) + u.noise_off) % vec2u(64u);
            let n = textureLoad(noise, vec2i(t), 0).r - 0.5;
            c = floor(c * 255.0 + 0.5 + n) / 255.0;
        } else {
            c = round(c * 255.0) / 255.0;
        }
    }
    return vec4f(c, 1.0);
}

// Synthetic GPU load: a full-screen pass with some ALU per pixel into a scratch target.
@fragment
fn fs_load(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    var v = pos.xy / u.size;
    for (var i = 0u; i < 16u; i++) {
        v = fract(v * 1.618 + vec2f(sin(v.y * 6.28 + u.time_s), cos(v.x * 6.28)));
    }
    return vec4f(v, 0.0, 1.0);
}
