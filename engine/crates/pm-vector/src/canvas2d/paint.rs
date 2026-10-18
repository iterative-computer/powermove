//! Skia shader inputs for recorded SVG/Canvas draws. Colors interpolate in
//! unpremultiplied sRGB, then the fragment result is premultiplied for blend.
use super::skpath::{Mat, P};

#[derive(Debug, Clone)]
pub enum Paint {
    Solid([f32; 4]),
    Linear {
        inverse: Mat,
        start: P,
        end: P,
        stops: Vec<(f32, [f32; 4])>,
        spread: u8,
        alpha: f32,
    },
    Radial {
        inverse: Mat,
        center: P,
        focal: P,
        radius: f32,
        stops: Vec<(f32, [f32; 4])>,
        spread: u8,
        alpha: f32,
    },
}
impl Paint {
    pub fn alpha(mut self, a: f32) -> Self {
        match &mut self {
            Self::Solid(c) => {
                for v in c {
                    *v *= a;
                }
            }
            Self::Linear { alpha, .. } | Self::Radial { alpha, .. } => *alpha *= a,
        }
        self
    }
    pub fn at(&self, x: f32, y: f32) -> [f32; 4] {
        let (mut t, stops, spread, alpha) = match self {
            Self::Solid(c) => return *c,
            Self::Linear {
                inverse,
                start,
                end,
                stops,
                spread,
                alpha,
            } => {
                let q = inverse.map([x, y]);
                let d = [end[0] - start[0], end[1] - start[1]];
                (
                    ((q[0] - start[0]) * d[0] + (q[1] - start[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]),
                    stops,
                    *spread,
                    *alpha,
                )
            }
            Self::Radial {
                inverse,
                center,
                focal,
                radius,
                stops,
                spread,
                alpha,
            } => {
                let q = inverse.map([x, y]);
                let p = [q[0] - focal[0], q[1] - focal[1]];
                let d = [center[0] - focal[0], center[1] - focal[1]];
                let a = radius * radius - d[0] * d[0] - d[1] * d[1];
                let b = p[0] * d[0] + p[1] * d[1];
                let c = p[0] * p[0] + p[1] * p[1];
                (((b * b + a * c).max(0.0).sqrt() - b) / a, stops, *spread, *alpha)
            }
        };
        t = match spread {
            1 => t - t.floor(),
            2 => {
                let v = t - 2.0 * (t / 2.0).floor();
                if v > 1.0 { 2.0 - v } else { v }
            }
            _ => t.clamp(0.0, 1.0),
        };
        if stops.is_empty() {
            return [0.0; 4];
        }
        let right = stops.partition_point(|s| s.0 <= t);
        let mut c = if right == 0 {
            stops[0].1
        } else if right == stops.len() {
            stops[right - 1].1
        } else {
            let (a, b) = (stops[right - 1], stops[right]);
            let u = (t - a.0) / (b.0 - a.0);
            [0, 1, 2, 3].map(|i| a.1[i] + (b.1[i] - a.1[i]) * u)
        };
        c[3] *= alpha;
        for i in 0..3 {
            c[i] *= c[3]
        }
        c
    }
}
pub fn inverse(m: Mat) -> Option<Mat> {
    let d = m.sx * m.sy - m.kx * m.ky;
    if d == 0.0 {
        return None;
    }
    Some(Mat {
        sx: m.sy / d,
        kx: -m.kx / d,
        ky: -m.ky / d,
        sy: m.sx / d,
        tx: (m.kx * m.ty - m.sy * m.tx) / d,
        ty: (m.ky * m.tx - m.sx * m.ty) / d,
    })
}
