//! Exact pixel coverage of filled outlines (paths, shapes, glyph outlines).
//!
//! Pixel `(x, y)` is the square `[x, x+1) × [y, y+1)` (y down). The outline
//! is flattened in f64 to [`TOLERANCE`] and split into `n × n` cells per
//! pixel. Each cell's integral of the winding number is exact for the
//! flattened outline (signed area accumulation, the polygon-area method of
//! scanline font rasterizers); the fill rule then applies per cell. For an
//! outline whose contours do not overlap the result is the exact area at
//! any `n`; where overlapping contours change the winding number inside one
//! cell, the error is bounded by that cell's area, `1/n²` of the pixel.
//! Gates use `n = 16` for paths and `n = 32` for glyph outlines.

use kurbo::{BezPath, PathEl, Point};

/// How the winding number decides inside (SVG/Canvas `fill-rule`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Flattening tolerance in pixels. Moving an outline by `d` changes a
/// pixel's coverage by at most `d`, so this is far below any gate's bound.
pub const TOLERANCE: f64 = 1e-5;

/// Coverage of a pixel window `[x0, x0 + width) × [y0, y0 + height)`;
/// zero outside it.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub x0: usize,
    pub y0: usize,
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<f64>,
}

impl Coverage {
    pub fn at(&self, x: usize, y: usize) -> f64 {
        if x < self.x0 || y < self.y0 || x >= self.x0 + self.width || y >= self.y0 + self.height {
            return 0.0;
        }
        self.alpha[(y - self.y0) * self.width + (x - self.x0)]
    }
}

/// Coverage of `path` (pixel coordinates) filled with `rule`, inside a
/// `width × height` frame, with `n × n` cells per pixel.
pub fn fill(path: &BezPath, rule: FillRule, n: usize, width: usize, height: usize) -> Coverage {
    assert!(n > 0);
    let segments = segments(path, n as f64);
    let (mut lo, mut hi) = (Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN));
    for (a, b) in &segments {
        lo = Point::new(lo.x.min(a.x).min(b.x), lo.y.min(a.y).min(b.y));
        hi = Point::new(hi.x.max(a.x).max(b.x), hi.y.max(a.y).max(b.y));
    }
    let nf = n as f64;
    let window = |l: f64, h: f64, size: usize| -> (usize, usize) {
        let a = (l / nf).floor().clamp(0.0, size as f64) as usize;
        let b = (h / nf).ceil().clamp(0.0, size as f64) as usize;
        (a, b.max(a))
    };
    let (x0, x1) = window(lo.x, hi.x, width);
    let (y0, y1) = window(lo.y, hi.y, height);
    let mut out = Coverage {
        x0,
        y0,
        width: x1 - x0,
        height: y1 - y0,
        alpha: vec![0.0; (x1 - x0) * (y1 - y0)],
    };
    if segments.is_empty() || out.width == 0 || out.height == 0 {
        return out;
    }
    let cells = out.width * n;
    let left = (x0 * n) as f64;
    let mut acc = vec![0.0f64; cells + 2];
    let mut row = vec![0.0f64; out.width];
    for py in y0..y1 {
        let (top, bottom) = ((py * n) as f64, ((py + 1) * n) as f64);
        let active: Vec<&(Point, Point)> = segments
            .iter()
            .filter(|(a, b)| a.y.min(b.y) < bottom && a.y.max(b.y) > top)
            .collect();
        row.iter_mut().for_each(|v| *v = 0.0);
        for k in 0..n {
            let strip = top + k as f64;
            acc.iter_mut().for_each(|v| *v = 0.0);
            for (a, b) in &active {
                let (ylo, yhi) = (a.y.min(b.y).max(strip), a.y.max(b.y).min(strip + 1.0));
                if ylo >= yhi {
                    continue;
                }
                let x_at = |y: f64| a.x + (b.x - a.x) * ((y - a.y) / (b.y - a.y)) - left;
                let dy = if b.y > a.y { yhi - ylo } else { ylo - yhi };
                accumulate(&mut acc, x_at(ylo), x_at(yhi), dy, cells);
            }
            let mut winding = 0.0;
            for (c, delta) in acc[..cells].iter().enumerate() {
                winding += delta;
                let w = winding.abs();
                row[c / n] += match rule {
                    FillRule::NonZero => w.min(1.0),
                    FillRule::EvenOdd => {
                        let m = w % 2.0;
                        if m > 1.0 { 2.0 - m } else { m }
                    }
                };
            }
        }
        let base = (py - y0) * out.width;
        let cell_area = 1.0 / (nf * nf);
        for (x, v) in row.iter().enumerate() {
            out.alpha[base + x] = v * cell_area;
        }
    }
    out
}

/// The flattened outline as line segments in cell units, every subpath
/// closed (a fill closes open subpaths).
fn segments(path: &BezPath, n: f64) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let close = |out: &mut Vec<(Point, Point)>, last: Point, start: Point| {
        if last != start {
            out.push((last, start));
        }
    };
    kurbo::flatten(path.iter(), TOLERANCE, |el| match el {
        PathEl::MoveTo(p) => {
            close(&mut out, last, start);
            start = Point::new(p.x * n, p.y * n);
            last = start;
        }
        PathEl::LineTo(p) => {
            let p = Point::new(p.x * n, p.y * n);
            out.push((last, p));
            last = p;
        }
        PathEl::ClosePath => {
            close(&mut out, last, start);
            last = start;
        }
        // `flatten` emits only lines.
        _ => {}
    });
    close(&mut out, last, start);
    out.retain(|(a, b)| a.y != b.y && a.is_finite() && b.is_finite());
    out
}

/// Adds one segment piece inside a one-cell-high strip, running from
/// `xa` to `xb` (window cell units) with signed height `dy`: to each cell it
/// crosses the area to its right, and to every later cell (through the
/// running sum) the full `dy`.
fn accumulate(acc: &mut [f64], xa: f64, xb: f64, dy: f64, cells: usize) {
    let (lo, hi) = (xa.min(xb), xa.max(xb));
    let span = hi - lo;
    let share = |u: f64, v: f64| if span > 0.0 { dy * (v - u) / span } else { dy };
    let mut u = lo;
    // Left of the window: the whole piece of the cell row is to the right.
    if u < 0.0 {
        let v = hi.min(0.0);
        acc[0] += share(u, v);
        u = v;
        if span == 0.0 || u >= hi {
            return;
        }
    }
    let end = hi.min(cells as f64);
    loop {
        let v = (u.floor() + 1.0).min(end);
        let mid = 0.5 * (u + v);
        let c = mid.floor() as usize;
        if c >= cells {
            break;
        }
        let f = mid - c as f64;
        let d = share(u, v);
        acc[c] += d * (1.0 - f);
        acc[c + 1] += d * f;
        if v >= end {
            break;
        }
        u = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Affine, Circle, Rect, Shape};

    fn total(c: &Coverage) -> f64 {
        c.alpha.iter().sum()
    }

    #[test]
    fn rectangles_are_exact() {
        let r = Rect::new(2.25, 3.5, 7.75, 6.125).to_path(1e-9);
        let c = fill(&r, FillRule::NonZero, 1, 10, 10);
        assert!((total(&c) - 5.5 * 2.625).abs() < 1e-12);
        assert!((c.at(2, 4) - 0.75).abs() < 1e-12);
        assert!((c.at(2, 3) - 0.375).abs() < 1e-12);
        assert_eq!(c.at(4, 4), 1.0);
        assert_eq!(c.at(9, 9), 0.0);
    }

    #[test]
    fn rotated_square_area_and_centroid() {
        let path = Affine::translate((16.3, 15.9))
            * Affine::rotate(0.37)
            * Rect::new(-6.0, -4.0, 6.0, 4.0).to_path(1e-9);
        let c = fill(&path, FillRule::NonZero, 4, 32, 32);
        assert!((total(&c) - 96.0).abs() < 1e-9, "{}", total(&c));
        let (mut sx, mut sy) = (0.0, 0.0);
        for y in 0..32 {
            for x in 0..32 {
                let a = c.at(x, y);
                sx += a * (x as f64 + 0.5);
                sy += a * (y as f64 + 0.5);
            }
        }
        // The pixel-center centroid of exact coverage meets the gates'
        // 1e-3 px geometry bound.
        let (cx, cy) = (sx / 96.0 - 16.3, sy / 96.0 - 15.9);
        assert!(cx.abs() < 1e-3 && cy.abs() < 1e-3, "{cx} {cy}");
    }

    #[test]
    fn circle_area_and_clipping() {
        let path = Circle::new((5.0, 5.0), 4.0).to_path(1e-9);
        let c = fill(&path, FillRule::NonZero, 16, 20, 20);
        let want = std::f64::consts::PI * 16.0;
        assert!((total(&c) - want).abs() < 1e-3, "{}", total(&c));
        // Without overlapping contours the area is exact at any `n`.
        let coarse = fill(&path, FillRule::NonZero, 1, 20, 20);
        let worst = (c.alpha.iter().zip(&coarse.alpha)).map(|(a, b)| (a - b).abs());
        assert!(worst.fold(0.0, f64::max) < 1e-9);
        // Half the circle outside the frame on the left.
        let shifted = Circle::new((0.0, 5.0), 4.0).to_path(1e-9);
        let c = fill(&shifted, FillRule::NonZero, 16, 20, 20);
        assert!((total(&c) - want / 2.0).abs() < 1e-3);
    }

    #[test]
    fn fill_rules_on_overlapping_contours() {
        let mut p = Rect::new(1.0, 1.0, 9.0, 9.0).to_path(1e-9);
        p.extend(Rect::new(3.0, 3.0, 7.0, 7.0).to_path(1e-9));
        let nz = fill(&p, FillRule::NonZero, 16, 10, 10);
        let eo = fill(&p, FillRule::EvenOdd, 16, 10, 10);
        assert!((total(&nz) - 64.0).abs() < 1e-12);
        assert!((total(&eo) - 48.0).abs() < 1e-12);
        assert_eq!(eo.at(5, 5), 0.0);
    }

    #[test]
    fn open_subpaths_close_for_fill() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((4.0, 0.0));
        p.line_to((4.0, 4.0));
        let c = fill(&p, FillRule::NonZero, 1, 4, 4);
        assert!((total(&c) - 8.0).abs() < 1e-12);
    }
}
