//! Graphite's gradient shaders for the gradients Blink hands Skia
//! (`SkShaders::LinearGradient`/`TwoPointConicalGradient` → Graphite's
//! `sk_linear_grad_4_shader`/`sk_radial_grad_4_shader` and the 8-stop and
//! binary-search variants), evaluated in `f32` with the `half` roundings the
//! Metal shader performs, the local-matrix block (`lmInverse · ptsToUnit`),
//! the color-space premultiply, the paint alpha, and `sk_dither`.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::blinksvg::{Gradient, GradientKind};
use super::skpath::Mat;

/// `SkGradientBaseShader::kDegenerateThreshold`.
const DEGENERATE_THRESHOLD: f32 = 1.0 / (1 << 15) as f32;
const NEARLY_ZERO: f32 = 1.0 / 4096.0;

/// Rounds an `f32` to the nearest representable `f16` (ties to even), as a
/// value stored in a Metal `half`.
pub fn f16_round(v: f32) -> f32 {
    if !v.is_finite() || v == 0.0 {
        return v;
    }
    let bits = v.to_bits();
    let sign = bits & 0x8000_0000;
    let abs = f32::from_bits(bits & 0x7fff_ffff);
    // Half overflow: values at or above 65520 round to infinity.
    if abs >= 65520.0 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    // Subnormal halves: quantum 2^-24.
    let quantum_exp = if exp < -14 { -24 } else { exp - 10 };
    let quantum = (2.0f64).powi(quantum_exp);
    let q = f64::from(abs) / quantum;
    let r = q.round_ties_even();
    let out = (r * quantum) as f32;
    f32::from_bits(out.to_bits() | sign)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Linear,
    Radial,
}

/// A gradient shader as Graphite's uniforms describe it.
#[derive(Debug, Clone)]
pub struct Shader {
    pub kind: Kind,
    /// A degenerate gradient (a solid color, unpremultiplied).
    pub solid: Option<[f32; 4]>,
    /// The stops padded to 4 or 8 (`GradientData`), or all stops for the
    /// texture/buffer variant.
    pub colors: Vec<[f32; 4]>,
    pub offsets: Vec<f32>,
    pub num_stops: usize,
    pub tile: u8,
    /// The local-matrix block: `lmInverse.postConcat(ptsToUnit)`.
    pub coords: Mat,
}

/// `SkMatrix::invert` for an affine matrix (`invertNonIdentity`).
fn sk_invert(m: &Mat) -> Option<Mat> {
    if *m == Mat::IDENTITY {
        return Some(Mat::IDENTITY);
    }
    if m.kx == 0.0 && m.ky == 0.0 {
        // Scale + translate.
        if m.sx == 1.0 && m.sy == 1.0 {
            return Some(Mat::scale_translate(1.0, 1.0, -m.tx, -m.ty));
        }
        if m.sx == 0.0 || m.sy == 0.0 {
            return None;
        }
        let inv_x = 1.0 / m.sx;
        let inv_y = 1.0 / m.sy;
        let tx = -m.tx * inv_x;
        let ty = -m.ty * inv_y;
        if !(inv_x.is_finite() && inv_y.is_finite() && tx.is_finite() && ty.is_finite()) {
            return None;
        }
        return Some(Mat::scale_translate(inv_x, inv_y, tx, ty));
    }
    // sk_inv_determinant in double.
    let det = f64::from(m.sx) * f64::from(m.sy) - f64::from(m.kx) * f64::from(m.ky);
    if det.abs() <= f64::from(f32::EPSILON) * f64::from(f32::EPSILON) * 8.0 * 0.0 + 1e-30 && det == 0.0 {
        return None;
    }
    let inv_det = 1.0 / det;
    if !inv_det.is_finite() {
        return None;
    }
    let d = |v: f32| f64::from(v);
    let out = Mat {
        sx: (d(m.sy) * inv_det) as f32,
        kx: (-d(m.kx) * inv_det) as f32,
        tx: ((d(m.kx) * d(m.ty) - d(m.sy) * d(m.tx)) * inv_det) as f32,
        ky: (-d(m.ky) * inv_det) as f32,
        sy: (d(m.sx) * inv_det) as f32,
        ty: ((d(m.ky) * d(m.tx) - d(m.sx) * d(m.ty)) * inv_det) as f32,
    };
    [out.sx, out.kx, out.tx, out.ky, out.sy, out.ty].iter().all(|v| v.is_finite()).then_some(out)
}

/// `pts_to_unit_matrix` of `SkLinearGradient`.
fn linear_pts_to_unit(p0: [f32; 2], p1: [f32; 2]) -> Mat {
    let mut vec = [p1[0] - p0[0], p1[1] - p0[1]];
    let mag = (vec[0] * vec[0] + vec[1] * vec[1]).sqrt();
    let inv = if mag != 0.0 { 1.0 / mag } else { 0.0 };
    vec = [vec[0] * inv, vec[1] * inv];
    // setSinCos(-vec.y, vec.x, p0.x, p0.y).
    let (sin_v, cos_v) = (-vec[1], vec[0]);
    let one_minus_cos = 1.0 - cos_v;
    let sdot = |a: f32, b: f32, c: f32, d: f32| a * b + c * d;
    let m = Mat {
        sx: cos_v,
        kx: -sin_v,
        tx: sdot(sin_v, p0[1], one_minus_cos, p0[0]),
        ky: sin_v,
        sy: cos_v,
        ty: sdot(-sin_v, p0[0], one_minus_cos, p0[1]),
    };
    // postTranslate(-p0) then postScale(inv, inv).
    let m = Mat::concat(&Mat::scale_translate(1.0, 1.0, -p0[0], -p0[1]), &m);
    Mat::concat(&Mat::scale_translate(inv, inv, 0.0, 0.0), &m)
}

/// `rad_to_unit_matrix` of `SkRadialGradient`.
fn radial_to_unit(center: [f32; 2], radius: f32) -> Mat {
    let inv = 1.0 / radius;
    let m = Mat::scale_translate(1.0, 1.0, -center[0], -center[1]);
    Mat::concat(&Mat::scale_translate(inv, inv, 0.0, 0.0), &m)
}

/// `SkGradientBaseShader`'s constructor on Blink's padded stops: implicit
/// first/last stops, monotonic pinning, uniform-stop detection, and the
/// duplicate-stop collapse.
fn normalize_stops(colors: &[[f32; 4]], pos: &[f32], tile: u8) -> (Vec<[f32; 4]>, Option<Vec<f32>>) {
    let first_implicit = pos.first().is_some_and(|p| *p > 0.0);
    let mut last_implicit = pos.last().is_some_and(|p| *p != 1.0);
    let mut out_colors = Vec::new();
    if first_implicit {
        out_colors.push(colors[0]);
    }
    out_colors.extend_from_slice(colors);
    if last_implicit {
        out_colors.push(*colors.last().unwrap());
    }
    let mut positions = vec![0.0f32];
    let mut prev = 0.0f32;
    let start_index = if first_implicit { 0 } else { 1 };
    let count = pos.len() + usize::from(last_implicit);
    let uniform_step = pos[start_index.min(pos.len() - 1)] - prev;
    let mut uniform = true;
    for i in start_index..count {
        let mut curr = 1.0f32;
        if i != pos.len() {
            curr = pos[i].clamp(prev, 1.0);
            if curr == 1.0 && last_implicit {
                last_implicit = false;
            }
        }
        uniform &= (uniform_step - (curr - prev)).abs() <= NEARLY_ZERO;
        positions.push(curr);
        prev = curr;
    }
    if uniform {
        return (out_colors, None);
    }
    // Remove duplicate stops with more than two of the same stop.
    let n = out_colors.len();
    let mut dedup_colors = Vec::new();
    let mut dedup_pos = Vec::new();
    let mut i = 0usize;
    for j in 1..=n {
        if j == n || positions[i] != positions[j] {
            let dup = j - i > 1;
            let ignore_leftmost = dup && tile != 0 && positions[i] == 0.0;
            if !ignore_leftmost {
                dedup_pos.push(positions[i]);
                dedup_colors.push(out_colors[i]);
            }
            let ignore_rightmost = tile != 0 && positions[j - 1] == 1.0;
            if dup && !ignore_rightmost {
                dedup_pos.push(positions[j - 1]);
                dedup_colors.push(out_colors[j - 1]);
            }
            i = j;
        }
    }
    (dedup_colors, Some(dedup_pos))
}

impl Shader {
    /// The shader Skia builds for Blink's gradient, as Graphite packs it.
    pub fn from_blink(g: &Gradient) -> Shader {
        let (kind, pts_to_unit, colors, positions): (Kind, Mat, Vec<[f32; 4]>, Vec<f32>) = match g.kind {
            GradientKind::Linear { p0, p1 } => {
                let len = ((p1[0] - p0[0]) * (p1[0] - p0[0]) + (p1[1] - p0[1]) * (p1[1] - p0[1])).sqrt();
                if len.abs() <= DEGENERATE_THRESHOLD {
                    return degenerate(g);
                }
                (Kind::Linear, linear_pts_to_unit(p0, p1), g.colors.clone(), g.positions.clone())
            }
            GradientKind::Conical { p0, r0, p1, r1 } => {
                let d = ((p1[0] - p0[0]) * (p1[0] - p0[0]) + (p1[1] - p0[1]) * (p1[1] - p0[1])).sqrt();
                if d.abs() <= DEGENERATE_THRESHOLD {
                    if (r0 - r1).abs() <= DEGENERATE_THRESHOLD {
                        if g.tile == 0 && r1 > DEGENERATE_THRESHOLD {
                            let front = g.colors[0];
                            let back = *g.colors.last().unwrap();
                            (Kind::Radial, radial_to_unit(p0, r1), vec![front, front, back], vec![0.0, 1.0, 1.0])
                        } else {
                            return degenerate(g);
                        }
                    } else if r0.abs() <= DEGENERATE_THRESHOLD {
                        (Kind::Radial, radial_to_unit(p0, r1), g.colors.clone(), g.positions.clone())
                    } else {
                        // The two-point conical radial variant (focal radius > 0).
                        return Shader::unsupported();
                    }
                } else {
                    return Shader::unsupported();
                }
            }
        };
        if colors.len() == 1 {
            return Shader { kind, solid: Some(colors[0]), colors: Vec::new(), offsets: Vec::new(), num_stops: 1, tile: g.tile, coords: Mat::IDENTITY };
        }
        let (colors, positions) = normalize_stops(&colors, &positions, g.tile);
        let n = colors.len();
        let positions = positions.unwrap_or_else(|| (0..n).map(|i| i as f32 / (n - 1) as f32).collect());
        // GradientData: pad to 4 or 8 with the last stop.
        let storage = if n <= 4 { 4 } else if n <= 8 { 8 } else { n };
        let mut padded_colors = colors.clone();
        let mut padded_offsets = positions.clone();
        while padded_colors.len() < storage {
            padded_colors.push(*colors.last().unwrap());
            padded_offsets.push(*positions.last().unwrap());
        }
        let lm_inverse = sk_invert(&g.local_matrix).unwrap_or(Mat::IDENTITY);
        let coords = Mat::concat(&pts_to_unit, &lm_inverse);
        Shader { kind, solid: None, colors: padded_colors, offsets: padded_offsets, num_stops: n, tile: g.tile, coords }
    }

    fn unsupported() -> Shader {
        Shader { kind: Kind::Linear, solid: Some([0.0; 4]), colors: Vec::new(), offsets: Vec::new(), num_stops: 0, tile: 0, coords: Mat::IDENTITY }
    }
}

/// `MakeDegenerateGradient` for clamp (the last color) and repeat/mirror
/// (the average color).
fn degenerate(g: &Gradient) -> Shader {
    let color = match g.tile {
        0 => *g.colors.last().unwrap(),
        _ => {
            let mut blend = [0.0f32; 4];
            let n = g.colors.len();
            for i in 0..n.saturating_sub(1) {
                let p0 = g.positions[i].clamp(0.0, 1.0);
                let p1 = g.positions[i + 1].clamp(p0, 1.0);
                let w = p1 - p0;
                if i == 0 && p0 > 0.0 {
                    for (b, c) in blend.iter_mut().zip(g.colors[0]) {
                        *b += p0 * c;
                    }
                }
                if i == n - 2 && p1 < 1.0 {
                    for (b, c) in blend.iter_mut().zip(g.colors[n - 1]) {
                        *b += (1.0 - p1) * c;
                    }
                }
                for (k, b) in blend.iter_mut().enumerate() {
                    *b += 0.5 * w * (g.colors[i + 1][k] + g.colors[i][k]);
                }
            }
            blend
        }
    };
    Shader { kind: Kind::Linear, solid: Some(color), colors: Vec::new(), offsets: Vec::new(), num_stops: 1, tile: g.tile, coords: Mat::IDENTITY }
}

#[inline]
fn half4(c: [f32; 4]) -> [f32; 4] {
    c.map(f16_round)
}

#[inline]
fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    // Metal's mix on float4: fma(t, b, fma(-t, a, a)) per component.
    [0, 1, 2, 3].map(|i| t.mul_add(b[i], (-t).mul_add(a[i], a[i])))
}

/// `$colorize_grad_4` / `$colorize_grad_8` / the binary search variant.
fn colorize(s: &Shader, t: f32) -> [f32; 4] {
    let (c, o) = (&s.colors, &s.offsets);
    if s.colors.len() <= 8 {
        // Unrolled searches: the same comparisons as the SkSL.
        let n = c.len();
        if t <= o[0] {
            return half4(c[0]);
        }
        for i in 1..n {
            if t < o[i] {
                return half4(mix4(c[i - 1], c[i], (t - o[i - 1]) / (o[i] - o[i - 1])));
            }
        }
        return half4(c[n - 1]);
    }
    // Texture/buffer gradients: binary search over the stops.
    if t == 0.0 {
        return half4(c[0]);
    }
    if t == 1.0 {
        return half4(c[c.len() - 1]);
    }
    let mut low = 0usize;
    let mut high = c.len() - 1;
    let mut loop_ = 1usize;
    while loop_ < c.len() {
        let mid = (low + high) / 2;
        if t < o[mid] {
            high = mid;
        } else {
            low = mid;
        }
        loop_ += loop_;
    }
    let (lo, hi) = (o[low], o[high]);
    if hi == lo {
        return half4(c[low]);
    }
    half4(mix4(c[low], c[high], (t - lo) / (hi - lo)))
}

/// The 8×8 ordered dither table (`skgpu::MakeDitherLUT`), as the `R8`
/// texels converted to `half`.
fn dither_value(x: u32, y: u32) -> f32 {
    let m = ((y & 1) << 5) | ((x & 1) << 4) | ((y & 2) << 2) | ((x & 2) << 1) | ((y & 4) >> 1) | ((x & 4) >> 2);
    let value = m as f32 * (1.0 / 64.0) - (63.0 / 128.0);
    let byte = ((value + 0.5) * 255.0 + 0.5) as u8;
    f16_round(f32::from(byte) / 255.0)
}

/// The premultiplied fragment color at `local` (the draw's local
/// coordinates) for a pixel at `pixel` (the dither coordinate).
pub fn evaluate(s: &Shader, local: [f32; 2], alpha: f32, dither: bool, pixel: [u32; 2]) -> [f32; 4] {
    let unpremul = match s.solid {
        Some(c) => half4(c),
        None => {
            // The local-matrix block: float2x2 * coords + translation.
            let m = s.coords;
            let x = m.sx * local[0] + m.kx * local[1] + m.tx;
            let y = m.ky * local[0] + m.sy * local[1] + m.ty;
            let mut t = match s.kind {
                Kind::Linear => x + 0.00001,
                Kind::Radial => (x * x + y * y).sqrt(),
            };
            t = match s.tile {
                1 => t - libm::floorf(t),
                2 => {
                    let t1 = t - 1.0;
                    let v = t1 - 2.0 * libm::floorf(t1 * 0.5) - 1.0;
                    v.abs().clamp(0.0, 1.0)
                }
                _ => t.clamp(0.0, 1.0),
            };
            colorize(s, t)
        }
    };
    // Color space transform: premultiply (half).
    let a = unpremul[3];
    let mut color = [f16_round(unpremul[0] * a), f16_round(unpremul[1] * a), f16_round(unpremul[2] * a), a];
    // Paint alpha (sk_blend src-in with the alpha-only paint color).
    if alpha < 1.0 {
        let ha = f16_round(alpha);
        color = color.map(|v| f16_round(v * ha));
    }
    if dither {
        let value = f16_round(dither_value(pixel[0] % 8, pixel[1] % 8) - 0.5);
        let range = f16_round(1.0 / 255.0);
        let d = f16_round(value * range);
        let a = color[3];
        for v in &mut color[..3] {
            *v = f16_round(*v + d).clamp(0.0, a);
        }
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_rounds_to_eleven_bits() {
        assert_eq!(f16_round(1.0), 1.0);
        assert_eq!(f16_round(0.1), 6552.0 / 65536.0);
        assert_eq!(f16_round(1.0 / 255.0), 257.0 / 65536.0);
        assert_eq!(f16_round(1e-9), 0.0);
    }

    #[test]
    fn two_stop_linear_gradient_interpolates() {
        let g = Gradient {
            kind: GradientKind::Linear { p0: [0.0, 0.0], p1: [1.0, 0.0] },
            colors: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]],
            positions: vec![0.0, 1.0],
            tile: 0,
            local_matrix: Mat::scale_translate(100.0, 60.0, 0.0, 0.0),
        };
        let s = Shader::from_blink(&g);
        assert_eq!(s.colors.len(), 4);
        let c = evaluate(&s, [50.0, 10.0], 1.0, false, [0, 0]);
        assert!((c[0] - 0.5).abs() < 2e-3 && (c[2] - 0.5).abs() < 2e-3, "{c:?}");
        // The layout adds 0.00001 to t, so the left edge is a hair past stop 0.
        let left = evaluate(&s, [0.0, 10.0], 1.0, false, [0, 0]);
        assert!(left[0] > 0.999 && left[2] < 0.001 && left[3] == 1.0, "{left:?}");
    }

    #[test]
    fn dither_values_follow_the_bayer_table() {
        // m = 0 at (0, 0): value -63/128, stored as byte 2.
        assert!((dither_value(0, 0) - 2.0 / 255.0).abs() < 1e-6);
        assert!(dither_value(1, 0) > dither_value(0, 0));
        assert!(dither_value(7, 7) > 0.98);
    }
}
