//! f64 reference for the final image sampler on decoded RGBA texels.
//!
//! This is deliberately independent of the renderer's GPU code. The
//! decoded bitmap is a source boundary; coordinates, Lanczos weights,
//! normalization and anti-ringing are computed here in f64. A caller uses
//! this only where the major singular value of the output-to-source map
//! is at most four texels, so the declared final path reads mip level zero.

use std::f64::consts::PI;

pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA, top-down, in the source's working encoding.
    pub rgba: Vec<f64>,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

pub fn lanczos3(x: f64) -> f64 {
    if x.abs() >= 3.0 {
        0.0
    } else {
        sinc(x) * sinc(x / 3.0)
    }
}

impl Bitmap {
    pub fn mip_zero(dx: (f64, f64), dy: (f64, f64)) -> bool {
        let m00 = dx.0 * dx.0 + dy.0 * dy.0;
        let m01 = dx.0 * dx.1 + dy.0 * dy.1;
        let m11 = dx.1 * dx.1 + dy.1 * dy.1;
        let disc = ((m00 - m11).powi(2) * 0.25 + m01 * m01).sqrt();
        let major = (0.5 * (m00 + m11) + disc).sqrt();
        major.is_finite() && major <= 4.0
    }

    /// Source coordinates are measured from texel edges: first centre 0.5.
    /// `dx` and `dy` are source-texel displacements for one output pixel.
    pub fn sample(&self, at: (f64, f64), dx: (f64, f64), dy: (f64, f64)) -> [f64; 4] {
        let (m00, m01, m11) = (
            dx.0 * dx.0 + dy.0 * dy.0,
            dx.0 * dx.1 + dy.0 * dy.1,
            dx.1 * dx.1 + dy.1 * dy.1,
        );
        let trace = m00 + m11;
        let disc = ((m00 - m11).powi(2) * 0.25 + m01 * m01).sqrt();
        let major = (trace * 0.5 + disc).max(0.0);
        let minor = (trace * 0.5 - disc).max(0.0);
        let mut u = (1.0, 0.0);
        if disc > 1e-4 * trace {
            if m01.abs() > 1e-6 * trace {
                let x = major - m11;
                let len = x.hypot(m01);
                u = (x / len, m01 / len);
            } else if m11 > m00 {
                u = (0.0, 1.0);
            }
        }
        let v = (-u.1, u.0);
        let (kx, ky) = (major.sqrt().max(1.0), minor.sqrt().max(1.0));
        let extent_x = 3.0 * (u.0.abs() * kx + v.0.abs() * ky);
        let extent_y = 3.0 * (u.1.abs() * kx + v.1.abs() * ky);
        let x0 = (at.0 - extent_x).floor() as i64;
        let x1 = (at.0 + extent_x).ceil() as i64;
        let y0 = (at.1 - extent_y).floor() as i64;
        let y1 = (at.1 + extent_y).ceil() as i64;
        let mut sum = [0.0; 4];
        let mut low = [f64::INFINITY; 4];
        let mut high = [f64::NEG_INFINITY; 4];
        let mut weight = 0.0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f64 + 0.5 - at.0, y as f64 + 0.5 - at.1);
                let ax = (d.0 * u.0 + d.1 * u.1) / kx;
                let ay = (d.0 * v.0 + d.1 * v.1) / ky;
                if ax.abs() >= 3.0 || ay.abs() >= 3.0 {
                    continue;
                }
                let texel_x = x.clamp(0, self.width as i64 - 1) as usize;
                let texel_y = y.clamp(0, self.height as i64 - 1) as usize;
                let pixel = &self.rgba[(texel_y * self.width + texel_x) * 4..][..4];
                let w = lanczos3(ax) * lanczos3(ay);
                weight += w;
                for c in 0..4 {
                    sum[c] += pixel[c] * w;
                    if ax.abs() < 1.0 && ay.abs() < 1.0 {
                        low[c] = low[c].min(pixel[c]);
                        high[c] = high[c].max(pixel[c]);
                    }
                }
            }
        }
        if high[0] < low[0] {
            std::array::from_fn(|c| sum[c] / weight)
        } else {
            std::array::from_fn(|c| (sum[c] / weight).clamp(low[c], high[c]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_edges_survive_the_declared_filter() {
        let b = Bitmap {
            width: 2,
            height: 2,
            rgba: vec![0.31, 0.2, 0.1, 0.5].repeat(4),
        };
        for (at, dx, dy) in [
            ((-4.0, 8.0), (1.0, 0.0), (0.0, 1.0)),
            ((0.5, 0.5), (2.0, 0.0), (0.0, 3.0)),
            ((1.2, 1.7), (0.3, 0.4), (-0.4, 0.3)),
        ] {
            let got = b.sample(at, dx, dy);
            for (actual, expected) in got.iter().zip([0.31, 0.2, 0.1, 0.5]) {
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "{at:?} {dx:?} {dy:?}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn mip_zero_selection_uses_the_major_axis_of_the_affine_map() {
        assert!(Bitmap::mip_zero((3.0, 0.0), (0.0, 2.0)));
        assert!(Bitmap::mip_zero((0.0, 3.0), (-2.0, 0.0)));
        assert!(!Bitmap::mip_zero((4.01, 0.0), (0.0, 1.0)));
        assert!(!Bitmap::mip_zero((f64::NAN, 0.0), (0.0, 1.0)));
    }
}
