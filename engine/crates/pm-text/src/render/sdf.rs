//! Distance-field text as Chromium's GPU canvas draws glyphs from 162 to
//! 256 device px on macOS (`sktext::gpu::SubRunControl`, SDFT): each glyph
//! is generated once at the strike size (256 px; 162 px at exactly 162) as
//! CoreGraphics' smoothed A8 mask, linearized and without the paint's
//! preblend (the SDF mask filter ignores it), turned into an 8-bit signed
//! distance field (`SkGenerateDistanceFieldFromA8Image`, padded by 4), and
//! drawn as a quad inset by 2 texels whose fragments sample the field
//! bilinearly and shade it with Graphite's `sdf_text_coverage_fn` in half
//! precision.
//!
//! [`distance_field`] is a port of `src/core/SkDistanceFieldGen.cpp` from
//! Skia m149 (the A8 path), in the same `f32` arithmetic and the same
//! pointer walk; [`draw`] emulates the draw: the vertex positions in `f32`,
//! the rasterizer's subpixel snapping, attribute interpolation, bilinear
//! filtering, and the fragment function with every `half` operation rounded
//! to IEEE binary16, with screen-space derivatives taken over 2×2 quads.
//!
//! Skia is Copyright 2014 Google Inc., used under its BSD-3-Clause license:
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are
//! met: (1) Redistributions of source code must retain the above copyright
//! notice, this list of conditions and the following disclaimer.
//! (2) Redistributions in binary form must reproduce the above copyright
//! notice, this list of conditions and the following disclaimer in the
//! documentation and/or other materials provided with the distribution.
//! (3) Neither the name of the copyright holder nor the names of its
//! contributors may be used to endorse or promote products derived from
//! this software without specific prior written permission. THIS SOFTWARE
//! IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY
//! EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
//! IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
//! PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

// A line-for-line port: Skia's control flow and arithmetic are kept as
// written so the two can be diffed.
#![allow(
    clippy::needless_range_loop,
    clippy::excessive_precision,
    clippy::approx_constant
)]

/// `SK_DistanceFieldPad`: the field's border beyond the A8 image.
pub const PAD: i32 = 4;
/// `SK_DistanceFieldInset`: the quad drawn is the field inset by this.
pub const INSET: i32 = 2;
/// `SK_DistanceFieldMagnitude`.
const MAGNITUDE: f32 = 4.0;

/// A glyph's distance field in strike space.
#[derive(Debug, Clone)]
pub struct SdfGlyph {
    /// The field's top-left texel relative to the glyph origin (strike px,
    /// y down): the A8 mask's, less [`PAD`].
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// The field, row by row (128 is the edge; larger is inside).
    pub field: Vec<u8>,
}

impl SdfGlyph {
    /// The field of an A8 mask whose top-left pixel is at (`left`, `top`).
    pub fn from_a8(a8: &[u8], width: usize, height: usize, left: i32, top: i32) -> SdfGlyph {
        SdfGlyph {
            left: left - PAD,
            top: top - PAD,
            width: (width + 2 * PAD as usize) as u32,
            height: (height + 2 * PAD as usize) as u32,
            field: distance_field(a8, width, height),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DfData {
    alpha: f32,
    dist_sq: f32,
    dist_vec: (f32, f32),
}

const LEFT: u32 = 0x01;
const RIGHT: u32 = 0x02;
const TOP_LEFT: u32 = 0x04;
const TOP: u32 = 0x08;
const TOP_RIGHT: u32 = 0x10;
const BOTTOM_LEFT: u32 = 0x20;
const BOTTOM: u32 = 0x40;
const BOTTOM_RIGHT: u32 = 0x80;
const ALL: u32 = 0xff;

/// An edge is where the image crosses 128, or where two nonzero pixels are
/// both below 128 (`found_edge`).
fn found_edge(image: &[u8], at: usize, width: usize, flags: u32) -> bool {
    let w = width as isize;
    let offsets: [isize; 8] = [-1, 1, -w - 1, -w, -w + 1, w - 1, w, w + 1];
    let curr_val = image[at];
    let curr_check = curr_val >> 7;
    for (i, off) in offsets.iter().enumerate() {
        let neighbor_val = if (1 << i) & flags != 0 {
            image[(at as isize + off) as usize]
        } else {
            0
        };
        let neighbor_check = neighbor_val >> 7;
        if curr_check != neighbor_check
            || (curr_check == 0 && neighbor_check == 0 && curr_val != 0 && neighbor_val != 0)
        {
            return true;
        }
    }
    false
}

/// `init_glyph_data`: the padded copy into the data grid at (`pad`, `pad`).
fn init_glyph_data(
    data: &mut [DfData],
    edges: &mut [u8],
    image: &[u8],
    data_width: usize,
    image_width: usize,
    image_height: usize,
    pad: usize,
) {
    let mut d = pad * data_width + pad;
    let mut im = 0usize;
    for j in 0..image_height {
        for i in 0..image_width {
            data[d].alpha = if image[im] == 255 {
                1.0
            } else {
                f32::from(image[im]) * 0.003_921_568_62_f32
            };
            let mut mask = ALL;
            if i == 0 {
                mask &= !(LEFT | TOP_LEFT | BOTTOM_LEFT);
            }
            if i == image_width - 1 {
                mask &= !(RIGHT | TOP_RIGHT | BOTTOM_RIGHT);
            }
            if j == 0 {
                mask &= !(TOP_LEFT | TOP | TOP_RIGHT);
            }
            if j == image_height - 1 {
                mask &= !(BOTTOM_LEFT | BOTTOM | BOTTOM_RIGHT);
            }
            if found_edge(image, im, image_width, mask) {
                edges[d] = 255;
            }
            d += 1;
            im += 1;
        }
        d += 2 * pad;
    }
}

fn nearly_zero(x: f32) -> bool {
    x.abs() <= 1.0 / 4096.0
}

/// Gustavson's distance to the edge from a pixel's coverage and the edge
/// normal (`edge_distance`).
fn edge_distance(direction: (f32, f32), alpha: f32) -> f32 {
    let (mut dx, mut dy) = direction;
    if nearly_zero(dx) || nearly_zero(dy) {
        0.5 - alpha
    } else {
        dx = dx.abs();
        dy = dy.abs();
        if dx < dy {
            std::mem::swap(&mut dx, &mut dy);
        }
        let a1num = 0.5 * dy;
        if alpha * dx < a1num {
            0.5 * (dx + dy) - (2.0 * dx * dy * alpha).sqrt()
        } else if alpha * dx < (dx - a1num) {
            (0.5 - alpha) * dx
        } else {
            -0.5 * (dx + dy) + (2.0 * dx * dy * (1.0 - alpha)).sqrt()
        }
    }
}

/// `SkPointPriv::SetLengthFast(p, 1)`: scaled in `double`.
fn set_length_one(x: f32, y: f32) -> (f32, f32) {
    let (xx, yy) = (f64::from(x), f64::from(y));
    let dmag = (xx * xx + yy * yy).sqrt();
    let dscale = 1.0 / dmag;
    let (nx, ny) = (
        (f64::from(x) * dscale) as f32,
        (f64::from(y) * dscale) as f32,
    );
    if !(nx.is_finite() && ny.is_finite()) || (nx == 0.0 && ny == 0.0) {
        return (0.0, 0.0);
    }
    (nx, ny)
}

/// `SK_ScalarSqrt2` (1.41421356f).
const SQRT2: f32 = std::f32::consts::SQRT_2;

fn init_distances(data: &mut [DfData], edges: &[u8], width: usize, height: usize) {
    for j in 0..height {
        for i in 0..width {
            let c = j * width + i;
            if edges[c] != 0 {
                let a = |k: usize| data[k].alpha;
                let (prev, next) = (c - width, c + width);
                let gx = a(prev + 1) - a(prev - 1) + SQRT2 * a(c + 1) - SQRT2 * a(c - 1)
                    + a(next + 1)
                    - a(next - 1);
                let gy = a(next - 1) - a(prev - 1) + SQRT2 * a(next) - SQRT2 * a(prev)
                    + a(next + 1)
                    - a(prev + 1);
                let grad = set_length_one(gx, gy);
                let dist = edge_distance(grad, data[c].alpha);
                data[c].dist_vec = (grad.0 * dist, grad.1 * dist);
                data[c].dist_sq = dist * dist;
            } else {
                data[c].dist_sq = 2_000_000.0;
                data[c].dist_vec = (1000.0, 1000.0);
            }
        }
    }
}

#[inline]
fn take(data: &mut [DfData], curr: usize, dist_sq: f32, vec: (f32, f32)) {
    if dist_sq < data[curr].dist_sq {
        data[curr].dist_sq = dist_sq;
        data[curr].dist_vec = vec;
    }
}

fn f1(data: &mut [DfData], curr: usize, width: usize) {
    // upper left
    let check = data[curr - width - 1];
    let v = check.dist_vec;
    let d = check.dist_sq - 2.0 * (v.0 + v.1 - 1.0);
    take(data, curr, d, (v.0 - 1.0, v.1 - 1.0));
    // up
    let check = data[curr - width];
    let v = check.dist_vec;
    let d = check.dist_sq - 2.0 * v.1 + 1.0;
    take(data, curr, d, (v.0, v.1 - 1.0));
    // upper right
    let check = data[curr - width + 1];
    let v = check.dist_vec;
    let d = check.dist_sq + 2.0 * (v.0 - v.1 + 1.0);
    take(data, curr, d, (v.0 + 1.0, v.1 - 1.0));
    // left
    let check = data[curr - 1];
    let v = check.dist_vec;
    let d = check.dist_sq - 2.0 * v.0 + 1.0;
    take(data, curr, d, (v.0 - 1.0, v.1));
}

fn f2(data: &mut [DfData], curr: usize) {
    let check = data[curr + 1];
    let v = check.dist_vec;
    let d = check.dist_sq + 2.0 * v.0 + 1.0;
    take(data, curr, d, (v.0 + 1.0, v.1));
}

fn b1(data: &mut [DfData], curr: usize) {
    let check = data[curr - 1];
    let v = check.dist_vec;
    let d = check.dist_sq - 2.0 * v.0 + 1.0;
    take(data, curr, d, (v.0 - 1.0, v.1));
}

fn b2(data: &mut [DfData], curr: usize, width: usize) {
    // right
    let check = data[curr + 1];
    let v = check.dist_vec;
    let d = check.dist_sq + 2.0 * v.0 + 1.0;
    take(data, curr, d, (v.0 + 1.0, v.1));
    // bottom left
    let check = data[curr + width - 1];
    let v = check.dist_vec;
    let d = check.dist_sq - 2.0 * (v.0 - v.1 - 1.0);
    take(data, curr, d, (v.0 - 1.0, v.1 + 1.0));
    // bottom
    let check = data[curr + width];
    let v = check.dist_vec;
    let d = check.dist_sq + 2.0 * v.1 + 1.0;
    take(data, curr, d, (v.0, v.1 + 1.0));
    // bottom right
    let check = data[curr + width + 1];
    let v = check.dist_vec;
    let d = check.dist_sq + 2.0 * (v.0 + v.1 + 1.0);
    take(data, curr, d, (v.0 + 1.0, v.1 + 1.0));
}

/// `pack_distance_field_val<4>`.
fn pack(dist: f32) -> u8 {
    let d = (-dist).clamp(-MAGNITUDE, MAGNITUDE * 127.0 / 128.0);
    let d = d + MAGNITUDE;
    (d / (2.0 * MAGNITUDE) * 256.0 + 0.5).floor() as i32 as u8
}

/// `SkGenerateDistanceFieldFromA8Image`: the distance field of a `width` ×
/// `height` A8 image, `(width + 8) × (height + 8)` texels.
pub fn distance_field(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    // The source in a zero border.
    let (cw, ch) = (width + 2, height + 2);
    let mut copy = vec![0u8; cw * ch];
    for y in 0..height {
        copy[(y + 1) * cw + 1..(y + 1) * cw + 1 + width]
            .copy_from_slice(&image[y * width..(y + 1) * width]);
    }
    let pad = PAD as usize + 1;
    let dw = width + 2 * pad;
    let dh = height + 2 * pad;
    let mut data = vec![DfData::default(); dw * dh];
    let mut edges = vec![0u8; dw * dh];
    init_glyph_data(&mut data, &mut edges, &copy, dw, cw, ch, PAD as usize);
    init_distances(&mut data, &edges, dw, dh);

    // Forwards in y.
    let mut curr = dw + 1;
    for _ in 1..dh - 1 {
        for _ in 1..dw - 1 {
            if edges[curr] == 0 {
                f1(&mut data, curr, dw);
            }
            curr += 1;
        }
        curr -= 1;
        for _ in 1..dw - 1 {
            if edges[curr] == 0 {
                f2(&mut data, curr);
            }
            curr -= 1;
        }
        curr += dw + 1;
    }
    // Backwards in y (the walk starts one texel before row dh - 2, as
    // Skia's does).
    let mut curr = dw * (dh - 2) - 1;
    for _ in 1..dh - 1 {
        for _ in 1..dw - 1 {
            if edges[curr] == 0 {
                b1(&mut data, curr);
            }
            curr += 1;
        }
        curr -= 1;
        for _ in 1..dw - 1 {
            if edges[curr] == 0 {
                b2(&mut data, curr, dw);
            }
            curr -= 1;
        }
        curr -= dw - 1;
    }

    let mut out = Vec::with_capacity((dw - 2) * (dh - 2));
    for j in 1..dh - 1 {
        for i in 1..dw - 1 {
            let c = &data[j * dw + i];
            let dist = if c.alpha > 0.5 {
                -c.dist_sq.sqrt()
            } else {
                c.dist_sq.sqrt()
            };
            out.push(pack(dist));
        }
    }
    out
}

/// `x` rounded to the nearest IEEE binary16 value (ties to even), as a
/// `half` operation's result is.
pub fn half(x: f32) -> f32 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let a = x.abs();
    if a >= 65520.0 {
        return f32::INFINITY.copysign(x);
    }
    if a < 6.103_515_6e-5 {
        // Subnormal halves: multiples of 2^-24.
        let q = 5.960_464_5e-8_f32;
        return ((x / q).round_ties_even()) * q;
    }
    let bits = x.to_bits();
    let lsb = 1u32 << 13;
    let rem = bits & (lsb - 1);
    let mut b = bits & !(lsb - 1);
    if rem > lsb / 2 || (rem == lsb / 2 && b & lsb != 0) {
        b += lsb;
    }
    f32::from_bits(b)
}

/// How the GPU's fixed-function stages are emulated (see [`draw`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuModel {
    /// Vertex positions snap to 1/`snap` px (0: no snapping).
    pub snap: f32,
    /// Bilinear weights round to 1/`filter_steps` (0: exact).
    pub filter_steps: f32,
}

/// The Apple GPUs the captures were made on: positions snapped to 1/256
/// px, bilinear weights rounded to 8 bits. Fitted on the `sizes` and
/// `density` cases (1/128 and 1/512 px snapping, truncated, 9-bit and
/// exact weights each leave pixels 2 code values off).
pub const APPLE_GPU: GpuModel = GpuModel {
    snap: 256.0,
    filter_steps: 256.0,
};

/// The coverage the GPU canvas draws `glyph` with (the fragment's value,
/// before the blend writes the 8-bit target), for a glyph whose quad's strike-space top-left is
/// `left_top` (`SubRunContainer`'s `prepare_for_SDFT_drawing`: the glyph
/// position mapped into strike space plus the field's inset bounds) under
/// the strike-to-device matrix `m` (`[a, b, c, d, e, f]`: `x' = a x + c y
/// + e`, the `subRunDeviceMatrix`): `(x0, y0, width, height, coverage)` in
/// device pixels, or `None` when nothing is drawn.
pub fn draw(
    glyph: &SdfGlyph,
    left_top: (f32, f32),
    m: [f32; 6],
    gpu: GpuModel,
) -> Option<(i64, i64, usize, usize, Vec<f32>)> {
    let (fw, fh) = (glyph.width as i32, glyph.height as i32);
    if fw <= 2 * INSET || fh <= 2 * INSET {
        return None;
    }
    let size = ((fw - 2 * INSET) as f32, (fh - 2 * INSET) as f32);
    // `text_vertex_fn`: baseCoords × size + xyPos, through the device
    // matrix (each product and sum in f32).
    let vertex = |bx: f32, by: f32| -> (f64, f64) {
        let lx = bx * size.0 + left_top.0;
        let ly = by * size.1 + left_top.1;
        let x = m[0] * lx + m[2] * ly + m[4];
        let y = m[1] * lx + m[3] * ly + m[5];
        let snap = |v: f32| {
            if gpu.snap > 0.0 {
                f64::from((v * gpu.snap).round() / gpu.snap)
            } else {
                f64::from(v)
            }
        };
        (snap(x), snap(y))
    };
    // The triangle strip's vertices (`sk_VertexID >> 1, sk_VertexID & 1`)
    // and their unnormalized texture coordinates in the field (inset).
    let base = [(0.0f32, 0.0f32), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)];
    let pos: Vec<(f64, f64)> = base.iter().map(|&(bx, by)| vertex(bx, by)).collect();
    let uv: Vec<(f32, f32)> = base
        .iter()
        .map(|&(bx, by)| (bx * size.0 + INSET as f32, by * size.1 + INSET as f32))
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &pos {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    // Whole 2×2 quads around the pixels the quad covers.
    let ix0 = ((x0 - 0.5).floor() as i64) & !1;
    let iy0 = ((y0 - 0.5).floor() as i64) & !1;
    let ix1 = (x1 + 0.5).ceil() as i64;
    let iy1 = (y1 + 0.5).ceil() as i64;
    let w = ((ix1 - ix0 + 1) as usize + 1) & !1;
    let h = ((iy1 - iy0 + 1) as usize + 1) & !1;
    if w * h > 4096 * 4096 {
        return None;
    }
    let tris = [[0usize, 1, 2], [1, 3, 2]];
    let edge = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    // Barycentric interpolation of the texture coordinates over a triangle
    // (helper lanes outside the quad extrapolate the first triangle's
    // plane); whether the pixel center is inside.
    let interp = |p: (f64, f64)| -> ((f32, f32), bool) {
        let mut inside_tri = None;
        for (k, t) in tris.iter().enumerate() {
            let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
            let area = edge(a, b, c);
            if area == 0.0 {
                continue;
            }
            let s = area.signum();
            let (e0, e1, e2) = (edge(b, c, p) * s, edge(c, a, p) * s, edge(a, b, p) * s);
            if e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 && (e0 > 0.0 || e1 > 0.0 || e2 > 0.0) {
                inside_tri = Some(k);
                break;
            }
        }
        let k = inside_tri.unwrap_or(0);
        let t = tris[k];
        let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
        let area = edge(a, b, c);
        if area == 0.0 {
            return ((0.0, 0.0), false);
        }
        let l1 = (edge(c, a, p) / area) as f32;
        let l2 = (edge(a, b, p) / area) as f32;
        let (ua, ub, uc) = (uv[t[0]], uv[t[1]], uv[t[2]]);
        (
            (
                ua.0 + l1 * (ub.0 - ua.0) + l2 * (uc.0 - ua.0),
                ua.1 + l1 * (ub.1 - ua.1) + l2 * (uc.1 - ua.1),
            ),
            inside_tri.is_some(),
        )
    };
    let texel = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= fw || y >= fh {
            0.0
        } else {
            f32::from(glyph.field[(y * fw + x) as usize]) / 255.0
        }
    };
    let sample = |u: f32, v: f32| -> f32 {
        let (tu, tv) = (u - 0.5, v - 0.5);
        let (fu, fv) = (tu.floor(), tv.floor());
        let (mut ax, mut ay) = (tu - fu, tv - fv);
        if gpu.filter_steps > 0.0 {
            ax = (ax * gpu.filter_steps).round() / gpu.filter_steps;
            ay = (ay * gpu.filter_steps).round() / gpu.filter_steps;
        }
        let (ix, iy) = (fu as i32, fv as i32);
        let top = texel(ix, iy) * (1.0 - ax) + texel(ix + 1, iy) * ax;
        let bottom = texel(ix, iy + 1) * (1.0 - ax) + texel(ix + 1, iy + 1) * ax;
        top * (1.0 - ay) + bottom * ay
    };
    let mut dist = vec![0f32; w * h];
    let mut uvs = vec![(0f32, 0f32); w * h];
    let mut inside = vec![false; w * h];
    let (k_mul, k_zero) = (half(7.968_75), half(0.501_960_784_31));
    for y in 0..h {
        for x in 0..w {
            let p = ((ix0 + x as i64) as f64 + 0.5, (iy0 + y as i64) as f64 + 0.5);
            let (t, inn) = interp(p);
            // The filtered texel in full precision; the distance in half
            // (`half dist = 7.96875 * (texColor - 0.50196078431)`, the one
            // half operation the captures show rounding).
            let tex = sample(t.0, t.1);
            dist[y * w + x] = half(k_mul * half(tex - k_zero));
            uvs[y * w + x] = t;
            inside[y * w + x] = inn;
        }
    }
    let mut cov = vec![0f32; w * h];
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !inside[i] {
                continue;
            }
            // Fine derivatives within the 2×2 quad.
            let (qx, qy) = (x & !1, y & !1);
            let ddx = |row: usize| (row * w + qx + 1, row * w + qx);
            let ddy = |col: usize| ((qy + 1) * w + col, qy * w + col);
            let (dx1, dx0) = ddx(y);
            let (dy1, dy0) = ddy(x);
            let g = (dist[dx1] - dist[dx0], dist[dy1] - dist[dy0]);
            let len2 = g.0 * g.0 + g.1 * g.1;
            let n = if len2 >= 0.0001 {
                let r = 1.0 / len2.sqrt();
                (g.0 * r, g.1 * r)
            } else {
                (0.7071, 0.7071)
            };
            let j = (
                (uvs[dx1].0 - uvs[dx0].0, uvs[dx1].1 - uvs[dx0].1),
                (uvs[dy1].0 - uvs[dy0].0, uvs[dy1].1 - uvs[dy0].1),
            );
            // float2x2(dFdx(uv), dFdy(uv)) × n: columns are the derivatives.
            let grad = (j.0.0 * n.0 + j.1.0 * n.1, j.0.1 * n.0 + j.1.1 * n.1);
            let afw = 0.65 * (grad.0 * grad.0 + grad.1 * grad.1).sqrt();
            // smoothstep(-afw, afw, d).
            let t = ((dist[i] + afw) / (2.0 * afw)).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t);
            if s > 0.0 {
                any = true;
            }
            cov[i] = s;
        }
    }
    any.then_some((ix0, iy0, w, h, cov))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_to_nearest_even() {
        assert_eq!(half(1.0), 1.0);
        assert_eq!(half(0.501_960_784_31), 0.501_953_125);
        assert_eq!(half(7.968_75), 7.968_75);
        // 1 + 2^-11 is halfway between 1 and 1 + 2^-10: ties to even (1).
        assert_eq!(half(1.0 + 1.0 / 2048.0), 1.0);
        assert_eq!(half(1.0 + 3.0 / 2048.0), 1.0 + 2.0 / 1024.0);
        assert_eq!(half(-0.65), -0.649_902_34);
    }

    #[test]
    fn a_filled_square_has_its_edge_at_128_and_saturates_four_texels_away() {
        // A 10 × 10 square, fully covered, in a 20 × 20 image.
        let (w, h) = (20usize, 20usize);
        let mut a8 = vec![0u8; w * h];
        for y in 5..15 {
            for x in 5..15 {
                a8[y * w + x] = 255;
            }
        }
        let f = distance_field(&a8, w, h);
        let fw = w + 8;
        assert_eq!(f.len(), fw * (h + 8));
        let at = |x: usize, y: usize| f[(y + 4) * fw + x + 4];
        // Inside the square's middle: as far inside as the field holds.
        assert_eq!(at(10, 10), 255);
        // Far outside: as far outside as it holds.
        assert_eq!(at(0, 10), 0);
        // The square's first column is half a texel inside its edge.
        let inside = at(5, 10);
        let outside = at(4, 10);
        assert!(inside > 128 && outside < 128, "{inside} {outside}");
        assert_eq!(
            i32::from(inside) - 128,
            128 - i32::from(outside),
            "symmetric about the edge"
        );
        // Monotone across the edge.
        let row: Vec<u8> = (0..10).map(|x| at(x, 10)).collect();
        assert!(row.windows(2).all(|p| p[0] <= p[1]), "{row:?}");
    }

    #[test]
    fn a_glyph_drawn_at_strike_size_covers_its_inside_and_not_its_outside() {
        let (w, h) = (24usize, 24usize);
        let mut a8 = vec![0u8; w * h];
        for y in 4..20 {
            for x in 4..20 {
                a8[y * w + x] = 255;
            }
        }
        let g = SdfGlyph::from_a8(&a8, w, h, -2, -22);
        // Strike-to-device: identity at the glyph origin (100, 100).
        let lt = ((g.left + INSET) as f32, (g.top + INSET) as f32);
        let (x0, y0, cw, _ch, cov) =
            draw(&g, lt, [1.0, 0.0, 0.0, 1.0, 100.0, 100.0], APPLE_GPU).expect("drawn");
        let at = |x: i64, y: i64| cov[((y - y0) as usize) * cw + (x - x0) as usize];
        // The square spans x 2..18 from the origin (100 + 2 .. 100 + 18).
        assert_eq!(at(110, 90), 1.0);
        assert_eq!(at(100, 90), 0.0);
        // Edges fall off across about a pixel.
        assert!(at(102, 90) > 0.0 && at(102, 90) < 1.0, "{}", at(102, 90));
    }
}
