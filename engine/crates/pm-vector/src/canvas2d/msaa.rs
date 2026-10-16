//! 4× multisample rasterization as the GPU performs it for Chromium's
//! accelerated Canvas2D (Skia Graphite on Metal, docs/engine/vector.md):
//! vertices snapped to the rasterizer's subpixel grid, coverage decided per
//! sample with the top-left rule, per-sample source-over blending in 8-bit
//! unorm, and a box resolve.

/// The standard 4× sample positions (Metal's and D3D's default pattern), in
/// pixels from the pixel's top-left corner, y down.
pub const SAMPLES_4X: [[f64; 2]; 4] = [[0.375, 0.125], [0.875, 0.375], [0.125, 0.625], [0.625, 0.875]];

/// Subpixel bits of vertex snapping (8 bits: 1/256 pixel).
pub const SUBPIXEL_BITS: u32 = 8;

/// Snaps a window coordinate to the rasterizer's fixed-point grid (round to
/// nearest).
#[inline]
pub fn snap(v: f64) -> i64 {
    libm::round(v * f64::from(1u32 << SUBPIXEL_BITS)) as i64
}

/// A sample position in grid units.
#[inline]
fn sample_grid(px: i64, py: i64, s: [f64; 2]) -> (i64, i64) {
    let one = 1i64 << SUBPIXEL_BITS;
    (
        px * one + (s[0] * one as f64) as i64,
        py * one + (s[1] * one as f64) as i64,
    )
}

/// Fill rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    NonZero,
    EvenOdd,
}

/// A coverage mask: 4 sample bits per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleMask {
    pub width: u32,
    pub height: u32,
    pub bits: Vec<u8>,
}

impl SampleMask {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bits: vec![0; (width * height) as usize],
        }
    }

    pub fn count(&self, x: u32, y: u32) -> u32 {
        self.bits[(y * self.width + x) as usize].count_ones()
    }

    /// Unions another mask into this one.
    pub fn union(&mut self, o: &SampleMask) {
        for (a, b) in self.bits.iter_mut().zip(&o.bits) {
            *a |= *b;
        }
    }
}

/// An edge in grid units, `y0 < y1`, with winding direction.
#[derive(Debug, Clone, Copy)]
struct GridEdge {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    winding: i32,
}

/// Fills closed polygons (window coordinates in pixels, already transformed)
/// into `mask` with the given rule. Every contour is implicitly closed.
pub fn fill_polygons(mask: &mut SampleMask, contours: &[Vec<[f64; 2]>], rule: Rule) {
    let mut edges = Vec::new();
    let (mut miny, mut maxy) = (i64::MAX, i64::MIN);
    for c in contours {
        if c.len() < 2 {
            continue;
        }
        for i in 0..c.len() {
            let a = c[i];
            let b = c[(i + 1) % c.len()];
            let (ax, ay, bx, by) = (snap(a[0]), snap(a[1]), snap(b[0]), snap(b[1]));
            if ay == by {
                continue;
            }
            let e = if ay < by {
                GridEdge { x0: ax, y0: ay, x1: bx, y1: by, winding: 1 }
            } else {
                GridEdge { x0: bx, y0: by, x1: ax, y1: ay, winding: -1 }
            };
            miny = miny.min(e.y0);
            maxy = maxy.max(e.y1);
            edges.push(e);
        }
    }
    if edges.is_empty() {
        return;
    }
    let one = 1i64 << SUBPIXEL_BITS;
    let py0 = (miny.div_euclid(one) - 1).max(0);
    let py1 = (maxy.div_euclid(one) + 1).min(i64::from(mask.height) - 1);
    let mut crossings: Vec<(i64, i64, i32)> = Vec::new();
    for py in py0..=py1 {
        for (si, s) in SAMPLES_4X.iter().enumerate() {
            let (_, sy) = sample_grid(0, py, *s);
            crossings.clear();
            for e in &edges {
                // Half-open in y: a sample on a vertex row belongs to the edge
                // below it (the top-left rule's "top" part).
                if sy < e.y0 || sy >= e.y1 {
                    continue;
                }
                // x of the edge at sy as an exact rational: num / den.
                let den = e.y1 - e.y0;
                let num = e.x0 * den + (e.x1 - e.x0) * (sy - e.y0);
                crossings.push((num, den, e.winding));
            }
            if crossings.is_empty() {
                continue;
            }
            for px in 0..i64::from(mask.width) {
                let (sx, _) = sample_grid(px, 0, *s);
                let mut w = 0;
                for &(num, den, wd) in &crossings {
                    // Inside when the edge is left of the sample, or exactly on
                    // it (a left edge includes its samples).
                    if num <= sx * den {
                        w += wd;
                    }
                }
                let inside = match rule {
                    Rule::NonZero => w != 0,
                    Rule::EvenOdd => w & 1 != 0,
                };
                if inside {
                    mask.bits[(py as u32 * mask.width + px as u32) as usize] |= 1 << si;
                }
            }
        }
    }
}

/// A 4× multisampled RGBA8 target (premultiplied), cleared to transparent.
#[derive(Debug, Clone)]
pub struct MsaaTarget {
    pub width: u32,
    pub height: u32,
    /// Four samples per pixel, RGBA8 each.
    pub samples: Vec<[u8; 4]>,
}

/// Float to unorm8 as the GPU converts blend results (round to nearest even).
#[inline]
pub fn unorm8(v: f32) -> u8 {
    let x = (v.clamp(0.0, 1.0) * 255.0) as f64;
    let r = libm::rint(x);
    r as u8
}

impl MsaaTarget {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            samples: vec![[0; 4]; (width * height * 4) as usize],
        }
    }

    /// Source-over blends premultiplied `color` into every covered sample.
    pub fn blend(&mut self, mask: &SampleMask, color: [f32; 4]) {
        for (p, bits) in mask.bits.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for s in 0..4 {
                if bits & (1 << s) == 0 {
                    continue;
                }
                let d = &mut self.samples[p * 4 + s];
                let inv = 1.0 - color[3];
                for c in 0..4 {
                    let dst = f32::from(d[c]) / 255.0;
                    d[c] = unorm8(color[c] + dst * inv);
                }
            }
        }
    }

    /// Box resolve to RGBA8 (premultiplied): the mean of the four samples,
    /// rounded half up (measured: a 26 in one sample resolves to 7).
    pub fn resolve(&self) -> Vec<u8> {
        let mut out = vec![0u8; (self.width * self.height * 4) as usize];
        for p in 0..(self.width * self.height) as usize {
            for c in 0..4 {
                let sum: u32 = (0..4).map(|s| u32::from(self.samples[p * 4 + s][c])).sum();
                out[p * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_pixel_square_covers_two_samples() {
        let mut m = SampleMask::new(4, 4);
        // x from 1.5 to 3: pixel 1 has samples at x 0.625 and 0.875 inside.
        fill_polygons(&mut m, &[vec![[1.5, 0.0], [3.0, 0.0], [3.0, 4.0], [1.5, 4.0]]], Rule::NonZero);
        assert_eq!(m.count(1, 1), 2);
        assert_eq!(m.count(2, 1), 4);
        assert_eq!(m.count(3, 1), 0);
    }

    #[test]
    fn resolve_rounds_like_the_gpu() {
        let mut t = MsaaTarget::new(1, 1);
        let mut m = SampleMask::new(1, 1);
        m.bits[0] = 0b0001;
        t.blend(&m, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(t.resolve()[3], 64);
        m.bits[0] = 0b0111;
        t.blend(&m, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(t.resolve()[3], 191);
    }
}
