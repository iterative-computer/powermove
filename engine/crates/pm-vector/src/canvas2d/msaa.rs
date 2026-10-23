//! 4× multisample rasterization as the GPU performs it for Chromium's
//! accelerated Canvas2D (Skia Graphite on Metal, docs/engine/vector.md):
//! vertices snapped to the rasterizer's subpixel grid, coverage decided per
//! sample with the top-left rule, per-sample source-over blending in 8-bit
//! unorm, and a box resolve.

/// The standard 4× sample positions (Metal's and D3D's default pattern), in
/// pixels from the pixel's top-left corner, y down.
pub const SAMPLES_4X: [[f64; 2]; 4] = [[0.375, 0.125], [0.875, 0.375], [0.125, 0.625], [0.625, 0.875]];

/// A single sample at the pixel center (a render pass without MSAA).
pub const SAMPLES_1X: [[f64; 2]; 1] = [[0.5, 0.5]];

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
#[derive(Debug, Clone, PartialEq)]
pub struct SampleMask {
    pub width: u32,
    pub height: u32,
    /// Pixel origin in the original canvas; preserves the global sample grid.
    pub origin: [u32; 2],
    /// The sample positions (4× or 1×).
    pub pattern: &'static [[f64; 2]],
    pub bits: Vec<u8>,
}

impl SampleMask {
    pub fn new(width: u32, height: u32, pattern: &'static [[f64; 2]]) -> Self {
        Self {
            width,
            height,
            origin: [0, 0],
            pattern,
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
    let py0 = (miny.div_euclid(one) - 1).max(i64::from(mask.origin[1]));
    let py1 = (maxy.div_euclid(one) + 1).min(i64::from(mask.origin[1] + mask.height) - 1);
    let mut crossings: Vec<(i64, i64, i32)> = Vec::new();
    for py in py0..=py1 {
        for (si, s) in mask.pattern.iter().enumerate() {
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
            // Each edge changes winding at the first integer pixel whose
            // sample is on its right. Compute that threshold as an exact
            // rational ceil: no float intersection can flip an edge sample.
            // Filling spans avoids testing every pixel against every edge.
            let sx = (s[0] * one as f64) as i64;
            let mut events: Vec<(i64, i32)> = crossings.iter().map(|&(num, den, wd)| {
                let n = i128::from(num) - i128::from(sx) * i128::from(den);
                let d = i128::from(one) * i128::from(den);
                let x = n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0);
                (x.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64, wd)
            }).collect();
            events.sort_unstable_by_key(|e| e.0);
            let x0 = i64::from(mask.origin[0]);
            let x1 = x0 + i64::from(mask.width);
            let row = (py as u32 - mask.origin[1]) as usize * mask.width as usize;
            let (mut x, mut w) = (x0, 0);
            for (edge, wd) in events.into_iter().chain(std::iter::once((x1, 0))) {
                let end = edge.clamp(x0, x1);
                let inside = match rule { Rule::NonZero => w != 0, Rule::EvenOdd => w & 1 != 0 };
                if inside && end > x {
                    for bits in &mut mask.bits[row + (x-x0) as usize..row + (end-x0) as usize] {
                        *bits |= 1 << si;
                    }
                }
                w += wd;
                x = end;
            }
        }
    }
}

/// A 4× multisampled RGBA8 target (premultiplied), cleared to transparent.
#[derive(Debug, Clone)]
pub struct MsaaTarget {
    pub width: u32,
    pub height: u32,
    /// The sample positions (4× or 1×).
    pub pattern: &'static [[f64; 2]],
    /// `pattern.len()` samples per pixel, RGBA8 each.
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
    pub fn new(width: u32, height: u32, pattern: &'static [[f64; 2]]) -> Self {
        Self {
            width,
            height,
            pattern,
            samples: vec![[0; 4]; (width * height) as usize * pattern.len()],
        }
    }

    /// A mask matching this target's sample pattern.
    pub fn mask(&self) -> SampleMask {
        SampleMask::new(self.width, self.height, self.pattern)
    }

    /// Source-over blends premultiplied `color` into every covered sample.
    pub fn blend(&mut self, mask: &SampleMask, color: [f32; 4]) {
        // A sample's new value depends only on its old one: the same
        // `unorm8(color + dst × (1 − α))` for each of the 256 values,
        // looked up instead of computed per sample.
        let inv = 1.0 - color[3];
        let mut lut = [[0u8; 256]; 4];
        for (c, table) in lut.iter_mut().enumerate() {
            for (dst, out) in table.iter_mut().enumerate() {
                *out = unorm8(color[c] + (dst as u8 as f32 / 255.0) * inv);
            }
        }
        let n = self.pattern.len();
        let full = (1u8 << n) - 1;
        for (p, bits) in mask.bits.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            if *bits == full {
                for d in &mut self.samples[p * n..p * n + n] {
                    for c in 0..4 {
                        d[c] = lut[c][usize::from(d[c])];
                    }
                }
                continue;
            }
            for s in 0..n {
                if bits & (1 << s) == 0 {
                    continue;
                }
                let d = &mut self.samples[p * n + s];
                for c in 0..4 {
                    d[c] = lut[c][usize::from(d[c])];
                }
            }
        }
    }

    /// Source-over blends `color × coverage` into each sample an analytic
    /// draw covers.
    pub fn blend_coverage(&mut self, samples: &super::analytic::Samples, color: [f32; 4]) {
        for (i, c) in samples.coverage.iter().enumerate() {
            let Some(c) = *c else { continue };
            let src = [color[0] * c, color[1] * c, color[2] * c, color[3] * c];
            let d = &mut self.samples[i];
            let inv = 1.0 - src[3];
            for k in 0..4 {
                let dst = f32::from(d[k]) / 255.0;
                d[k] = unorm8(src[k] + dst * inv);
            }
        }
    }

    /// Box resolve to RGBA8 (premultiplied): the mean of the four samples,
    /// rounded half up (measured: a 26 in one sample resolves to 7).
    pub fn resolve(&self) -> Vec<u8> {
        fn mean<const N: usize>(samples: &[[u8; 4]], out: &mut [u8]) {
            let half = N as u16 / 2;
            for (pixel, samples) in out.chunks_exact_mut(4).zip(samples.chunks_exact(N)) {
                for c in 0..4 {
                    let mut sum = 0u16;
                    for s in samples {
                        sum += u16::from(s[c]);
                    }
                    pixel[c] = ((sum + half) / N as u16) as u8;
                }
            }
        }
        let mut out = vec![0u8; (self.width * self.height * 4) as usize];
        match self.pattern.len() {
            4 => mean::<4>(&self.samples, &mut out),
            1 => mean::<1>(&self.samples, &mut out),
            n => {
                for (pixel, samples) in out.chunks_exact_mut(4).zip(self.samples.chunks_exact(n)) {
                    for c in 0..4 {
                        let sum: u32 = samples.iter().map(|s| u32::from(s[c])).sum();
                        pixel[c] = ((sum + n as u32 / 2) / n as u32) as u8;
                    }
                }
            }
        }
        out
    }
}

/// Debugging aid: for pixel `(px, py)`, each sample's nearest edge distance
/// in grid units (signed, exact rational as f64) for the given polygons.
pub fn explain(contours: &[Vec<[f64; 2]>], px: i64, py: i64) -> Vec<(usize, f64, bool)> {
    let mut out = Vec::new();
    for (si, s) in SAMPLES_4X.iter().enumerate() {
        let (sx, sy) = sample_grid(px, py, *s);
        let mut best = f64::INFINITY;
        let mut w = 0;
        for c in contours {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                let (ax, ay, bx, by) = (snap(a[0]), snap(a[1]), snap(b[0]), snap(b[1]));
                if ay == by {
                    continue;
                }
                let (x0, y0, x1, y1, wd) = if ay < by { (ax, ay, bx, by, 1) } else { (bx, by, ax, ay, -1) };
                if sy < y0 || sy >= y1 {
                    continue;
                }
                let den = y1 - y0;
                let num = x0 * den + (x1 - x0) * (sy - y0);
                let d = (sx * den - num) as f64 / den as f64;
                if d.abs() < best.abs() {
                    best = d;
                }
                if num <= sx * den {
                    w += wd;
                }
            }
        }
        out.push((si, best, w != 0));
    }
    out
}

/// Rasterizes triangles (window coordinates) into `mask` as their union,
/// with the same snapping and top-left rule as [`fill_polygons`].
pub fn fill_triangles(mask: &mut SampleMask, tris: &[[[f32; 2]; 3]]) {
    let one = 1i64 << SUBPIXEL_BITS;
    for t in tris {
        let v: Vec<(i64, i64)> = t.iter().map(|p| (snap(f64::from(p[0])), snap(f64::from(p[1])))).collect();
        // Zero-area triangles cover nothing.
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if area == 0 {
            continue;
        }
        let (minx, maxx) = (v.iter().map(|p| p.0).min().unwrap(), v.iter().map(|p| p.0).max().unwrap());
        let (miny, maxy) = (v.iter().map(|p| p.1).min().unwrap(), v.iter().map(|p| p.1).max().unwrap());
        let px0 = (minx.div_euclid(one)).max(i64::from(mask.origin[0]));
        let px1 = (maxx.div_euclid(one)).min(i64::from(mask.origin[0] + mask.width) - 1);
        let py0 = (miny.div_euclid(one)).max(i64::from(mask.origin[1]));
        let py1 = (maxy.div_euclid(one)).min(i64::from(mask.origin[1] + mask.height) - 1);
        if px1 < px0 || py1 < py0 {
            continue;
        }
        let edges: Vec<GridEdge> = (0..3)
            .filter_map(|i| {
                let (a, b) = (v[i], v[(i + 1) % 3]);
                if a.1 == b.1 {
                    return None;
                }
                Some(if a.1 < b.1 {
                    GridEdge { x0: a.0, y0: a.1, x1: b.0, y1: b.1, winding: 1 }
                } else {
                    GridEdge { x0: b.0, y0: b.1, x1: a.0, y1: a.1, winding: -1 }
                })
            })
            .collect();
        for py in py0..=py1 {
            let row = (py as u32 - mask.origin[1]) as usize * mask.width as usize;
            for (si, s) in mask.pattern.iter().enumerate() {
                let (_, sy) = sample_grid(0, py, *s);
                let sx = (s[0] * one as f64) as i64;
                // Each crossing adds its winding from the first pixel whose
                // sample is on or right of it (`num <= sample × den`): an exact
                // rational ceil, as `fill_polygons` decides it, so only the
                // covered span of the bounding box is written.
                let mut events = [(0i64, 0i32); 3];
                let mut n = 0;
                for e in &edges {
                    if sy < e.y0 || sy >= e.y1 {
                        continue;
                    }
                    let den = e.y1 - e.y0;
                    let num = e.x0 * den + (e.x1 - e.x0) * (sy - e.y0);
                    let q = i128::from(num) - i128::from(sx) * i128::from(den);
                    let d = i128::from(one) * i128::from(den);
                    let x = q.div_euclid(d) + i128::from(q.rem_euclid(d) != 0);
                    events[n] = (x.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64, e.winding);
                    n += 1;
                }
                if n == 0 {
                    continue;
                }
                let events = &mut events[..n];
                events.sort_unstable_by_key(|e| e.0);
                let (mut x, mut w) = (px0, 0);
                for &(edge, wd) in events.iter().chain(std::iter::once(&(px1 + 1, 0))) {
                    let end = edge.clamp(px0, px1 + 1);
                    if w != 0 && end > x {
                        let from = row + (x - i64::from(mask.origin[0])) as usize;
                        let to = row + (end - i64::from(mask.origin[0])) as usize;
                        for bits in &mut mask.bits[from..to] {
                            *bits |= 1 << si;
                        }
                    }
                    w += wd;
                    x = x.max(end);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fill_triangles` as it tested every pixel of each bounding box.
    fn fill_triangles_reference(mask: &mut SampleMask, tris: &[[[f32; 2]; 3]]) {
        let one = 1i64 << SUBPIXEL_BITS;
        for t in tris {
            let v: Vec<(i64, i64)> = t.iter().map(|p| (snap(f64::from(p[0])), snap(f64::from(p[1])))).collect();
            let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
            if area == 0 { continue; }
            let (minx, maxx) = (v.iter().map(|p| p.0).min().unwrap(), v.iter().map(|p| p.0).max().unwrap());
            let (miny, maxy) = (v.iter().map(|p| p.1).min().unwrap(), v.iter().map(|p| p.1).max().unwrap());
            let px0 = (minx.div_euclid(one)).max(i64::from(mask.origin[0]));
            let px1 = (maxx.div_euclid(one)).min(i64::from(mask.origin[0] + mask.width) - 1);
            let py0 = (miny.div_euclid(one)).max(i64::from(mask.origin[1]));
            let py1 = (maxy.div_euclid(one)).min(i64::from(mask.origin[1] + mask.height) - 1);
            if px1 < px0 || py1 < py0 { continue; }
            let edges: Vec<GridEdge> = (0..3).filter_map(|i| {
                let (a, b) = (v[i], v[(i + 1) % 3]);
                if a.1 == b.1 { return None; }
                Some(if a.1 < b.1 { GridEdge { x0: a.0, y0: a.1, x1: b.0, y1: b.1, winding: 1 } }
                     else { GridEdge { x0: b.0, y0: b.1, x1: a.0, y1: a.1, winding: -1 } })
            }).collect();
            for py in py0..=py1 {
                for (si, s) in mask.pattern.iter().enumerate() {
                    let (_, sy) = sample_grid(0, py, *s);
                    for px in px0..=px1 {
                        let (sx, _) = sample_grid(px, 0, *s);
                        let mut w = 0;
                        for e in &edges {
                            if sy < e.y0 || sy >= e.y1 { continue; }
                            let den = e.y1 - e.y0;
                            if e.x0 * den + (e.x1 - e.x0) * (sy - e.y0) <= sx * den { w += e.winding; }
                        }
                        if w != 0 {
                            mask.bits[((py as u32 - mask.origin[1]) * mask.width + px as u32 - mask.origin[0]) as usize] |= 1 << si;
                        }
                    }
                }
            }
        }
    }

    /// A small deterministic generator (no dependency).
    fn lcg(state: &mut u64) -> f32 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*state >> 33) as f32) / ((1u64 << 31) as f32)
    }

    #[test]
    fn triangle_spans_cover_exactly_the_samples_point_tests_cover() {
        let mut state = 7u64;
        for case in 0..400 {
            let pattern: &'static [[f64; 2]] = if case % 3 == 0 { &SAMPLES_1X } else { &SAMPLES_4X };
            let mut tris = Vec::new();
            for _ in 0..(1 + case % 5) {
                let mut t = [[0f32; 2]; 3];
                for p in &mut t {
                    // Some vertices on the pixel and subpixel grids, some outside the mask.
                    let x = lcg(&mut state) * 70.0 - 12.0;
                    let y = lcg(&mut state) * 60.0 - 10.0;
                    *p = match case % 4 { 0 => [x.round(), y.round()], 1 => [(x * 4.0).round() / 4.0, y], _ => [x, y] };
                }
                tris.push(t);
            }
            // A fan, as convex fills are tessellated.
            if case % 7 == 0 {
                let c = [30.3f32, 25.7];
                for k in 0..24 {
                    let a = k as f32 * std::f32::consts::TAU / 24.0;
                    let b = (k + 1) as f32 * std::f32::consts::TAU / 24.0;
                    tris.push([c, [c[0] + 28.0 * a.cos(), c[1] + 20.0 * a.sin()], [c[0] + 28.0 * b.cos(), c[1] + 20.0 * b.sin()]]);
                }
            }
            let mut fast = SampleMask::new(48, 41, pattern);
            let mut slow = SampleMask::new(48, 41, pattern);
            if case % 2 == 1 { fast.origin = [5, 3]; slow.origin = [5, 3]; }
            fill_triangles(&mut fast, &tris);
            fill_triangles_reference(&mut slow, &tris);
            assert_eq!(fast.bits, slow.bits, "case {case}");
        }
    }

    #[test]
    fn resolve_is_the_rounded_mean_of_each_pixels_samples() {
        let mut state = 3u64;
        for pattern in [&SAMPLES_1X[..], &SAMPLES_4X[..]] {
            let mut t = MsaaTarget::new(13, 7, pattern);
            for s in &mut t.samples {
                for c in s.iter_mut() { *c = (lcg(&mut state) * 256.0) as u8; }
            }
            let n = pattern.len();
            let out = t.resolve();
            for p in 0..(13 * 7) {
                for c in 0..4 {
                    let sum: u32 = (0..n).map(|s| u32::from(t.samples[p * n + s][c])).sum();
                    assert_eq!(out[p * 4 + c], ((sum + n as u32 / 2) / n as u32) as u8);
                }
            }
        }
    }

    #[test]
    fn blending_by_table_equals_blending_each_sample() {
        let mut state = 11u64;
        for _ in 0..200 {
            let a = lcg(&mut state);
            let color = [lcg(&mut state) * a, lcg(&mut state) * a, lcg(&mut state) * a, a];
            let mut t = MsaaTarget::new(16, 1, &SAMPLES_4X);
            for (i, s) in t.samples.iter_mut().enumerate() { *s = [(i * 13) as u8, (i * 29 + 7) as u8, (i * 61) as u8, (i * 17 + 200) as u8]; }
            let mut expected = t.samples.clone();
            let mut m = t.mask();
            for (i, b) in m.bits.iter_mut().enumerate() { *b = (i as u8 * 7) & 0xF; }
            for (p, bits) in m.bits.iter().enumerate() {
                for s in 0..4 {
                    if bits & (1 << s) == 0 { continue; }
                    let d = &mut expected[p * 4 + s];
                    for c in 0..4 { d[c] = unorm8(color[c] + f32::from(d[c]) / 255.0 * (1.0 - color[3])); }
                }
            }
            t.blend(&m, color);
            assert_eq!(t.samples, expected);
        }
    }

    #[test]
    fn winding_spans_match_point_sampling_with_holes_and_clipped_windows() {
        let contours = vec![
            vec![[-8.375, 2.125], [80.875, 5.375], [34.125, 60.625], [9.625, 45.875]],
            vec![[20.0, 15.0], [39.875, 15.0], [39.875, 41.125], [20.0, 41.125]],
            vec![[20.0, 15.0], [39.875, 15.0], [39.875, 41.125], [20.0, 41.125]],
        ];
        for rule in [Rule::NonZero, Rule::EvenOdd] {
            for pattern in [&SAMPLES_1X[..], &SAMPLES_4X[..]] {
                let mut mask = SampleMask::new(32, 29, pattern);
                mask.origin = [19, 13];
                fill_polygons(&mut mask, &contours, rule);
                for y in 0..mask.height {
                    for x in 0..mask.width {
                        let mut expected = 0;
                        for (s, sample) in pattern.iter().enumerate() {
                            let (sx, sy) = sample_grid(i64::from(x+19), i64::from(y+13), *sample);
                            let mut winding = 0;
                            for c in &contours {
                                for (a,b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
                                    let (ax,ay,bx,by) = (snap(a[0]),snap(a[1]),snap(b[0]),snap(b[1]));
                                    if ay == by { continue; }
                                    let (x0,y0,x1,y1,wd) = if ay<by {(ax,ay,bx,by,1)} else {(bx,by,ax,ay,-1)};
                                    if sy>=y0 && sy<y1 && x0*(y1-y0)+(x1-x0)*(sy-y0)<=sx*(y1-y0) { winding += wd; }
                                }
                            }
                            if match rule {Rule::NonZero=>winding!=0,Rule::EvenOdd=>winding&1!=0} { expected |= 1<<s; }
                        }
                        assert_eq!(mask.bits[(y*mask.width+x) as usize], expected);
                    }
                }
            }
        }
    }

    #[test]
    fn half_pixel_square_covers_two_samples() {
        let mut m = SampleMask::new(4, 4, &SAMPLES_4X);
        // x from 1.5 to 3: pixel 1 has samples at x 0.625 and 0.875 inside.
        fill_polygons(&mut m, &[vec![[1.5, 0.0], [3.0, 0.0], [3.0, 4.0], [1.5, 4.0]]], Rule::NonZero);
        assert_eq!(m.count(1, 1), 2);
        assert_eq!(m.count(2, 1), 4);
        assert_eq!(m.count(3, 1), 0);
    }

    #[test]
    fn resolve_rounds_like_the_gpu() {
        let mut t = MsaaTarget::new(1, 1, &SAMPLES_4X);
        let mut m = SampleMask::new(1, 1, &SAMPLES_4X);
        m.bits[0] = 0b0001;
        t.blend(&m, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(t.resolve()[3], 64);
        m.bits[0] = 0b0111;
        t.blend(&m, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(t.resolve()[3], 191);
    }
}
