//! Glyph masks Skia generates from paths (macOS Canvas2D, level 1).
//!
//! Skia draws a glyph image from its path, not from CoreGraphics, when the
//! glyph has a frame: synthesized bold (`SkFont::setEmbolden`, a
//! stroke-and-fill frame of `size × interp(size, {9, 36}, {1/24, 1/32})`)
//! and `strokeText` (a stroke frame of `lineWidth`). `SkScalerContext::
//! internalGetPath` takes the device path, adds the subpixel offset, maps it
//! back to user space through the inverse of the 2×2 matrix, applies the
//! frame with `SkStroke`, and maps it to device space again;
//! `GenerateImageFromPath` fills it with Skia's supersampling scan converter
//! into an A8 mask and applies the paint's mask gamma
//! ([`skia_mask_gamma`]). `pm-skia-path` is a port of the same stroker and
//! scan converter, in the same `f32` arithmetic.

use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl};

use super::scene::GlyphMask;

/// A glyph frame (`SkScalerContextRec::fFrameWidth` and its stroke
/// parameters), in user space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub width: f32,
    /// Stroke and fill (synthesized bold) rather than stroke only.
    pub fill: bool,
    pub join: pm_skia_path::LineJoin,
    pub miter_limit: f32,
}

impl Frame {
    /// Skia's fake bold for a font of `size` user px: the paint's fill
    /// style becomes stroke-and-fill with this width; the join and miter
    /// limit are the fill paint's (Skia's defaults, miter and 4).
    pub fn fake_bold(size: f32) -> Frame {
        Frame {
            width: size * fake_bold_scale(size),
            fill: true,
            join: pm_skia_path::LineJoin::Miter,
            miter_limit: 4.0,
        }
    }
}

/// `SkScalarInterpFunc(size, {9, 36}, {1/24, 1/32}, 2)` in `f32`.
pub fn fake_bold_scale(size: f32) -> f32 {
    const KEYS: [f32; 2] = [9.0, 36.0];
    const VALUES: [f32; 2] = [1.0 / 24.0, 1.0 / 32.0];
    let mut right = 0;
    while right < 2 && KEYS[right] < size {
        right += 1;
    }
    if right == 2 {
        return VALUES[1];
    }
    if right == 0 {
        return VALUES[0];
    }
    let fract = (size - KEYS[right - 1]) / (KEYS[right] - KEYS[right - 1]);
    VALUES[right - 1] + (VALUES[right] - VALUES[right - 1]) * fract
}

fn to_skia(path: &BezPath, m: Affine) -> Option<pm_skia_path::Path> {
    let mut b = pm_skia_path::PathBuilder::new();
    let p = |q: kurbo::Point| {
        let q = m * q;
        (q.x as f32, q.y as f32)
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(a) => {
                let a = p(a);
                b.move_to(a.0, a.1);
            }
            PathEl::LineTo(a) => {
                let a = p(a);
                b.line_to(a.0, a.1);
            }
            PathEl::QuadTo(c, a) => {
                let (c, a) = (p(c), p(a));
                b.quad_to(c.0, c.1, a.0, a.1);
            }
            PathEl::CurveTo(c0, c1, a) => {
                let (c0, c1, a) = (p(c0), p(c1), p(a));
                b.cubic_to(c0.0, c0.1, c1.0, c1.1, a.0, a.1);
            }
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

/// `SkPathPriv::ComputeFirstDirection`: the direction (true: clockwise,
/// y down) of the contour holding the path's lowest point (greatest y),
/// from the cross product at that point; `None` when unknown.
fn first_direction(path: &pm_skia_path::Path) -> Option<bool> {
    // Contours as point lists, as `ContourIter` walks the point array.
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut contours: Vec<(usize, usize)> = Vec::new();
    for seg in path.segments() {
        let add = |q: pm_skia_path::Point,
                   pts: &mut Vec<(f32, f32)>,
                   contours: &mut Vec<(usize, usize)>| {
            pts.push((q.x, q.y));
            if let Some(c) = contours.last_mut() {
                c.1 += 1;
            }
        };
        match seg {
            pm_skia_path::PathSegment::MoveTo(q) => {
                contours.push((pts.len(), 0));
                add(q, &mut pts, &mut contours);
            }
            pm_skia_path::PathSegment::LineTo(q) => add(q, &mut pts, &mut contours),
            pm_skia_path::PathSegment::QuadTo(c, q)
            | pm_skia_path::PathSegment::ConicTo(c, q, _) => {
                add(c, &mut pts, &mut contours);
                add(q, &mut pts, &mut contours);
            }
            pm_skia_path::PathSegment::CubicTo(c0, c1, q) => {
                add(c0, &mut pts, &mut contours);
                add(c1, &mut pts, &mut contours);
                add(q, &mut pts, &mut contours);
            }
            pm_skia_path::PathSegment::Close => {}
        }
    }
    let cross_prod = |p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)| -> f32 {
        let c = (p1.0 - p0.0) * (p2.1 - p0.1) - (p1.1 - p0.1) * (p2.0 - p0.0);
        if c == 0.0 {
            let (a, b, c2) = (
                (f64::from(p0.0), f64::from(p0.1)),
                (f64::from(p1.0), f64::from(p1.1)),
                (f64::from(p2.0), f64::from(p2.1)),
            );
            ((b.0 - a.0) * (c2.1 - a.1) - (b.1 - a.1) * (c2.0 - a.0)) as f32
        } else {
            c
        }
    };
    let mut ymax = path.bounds().top();
    let mut ymax_cross = 0.0f32;
    for &(start, n) in &contours {
        if n < 3 {
            continue;
        }
        let p = &pts[start..start + n];
        let mut index = 0;
        for (k, q) in p.iter().enumerate().skip(1) {
            if q.1 > p[index].1 {
                index = k;
            }
        }
        if p[index].1 < ymax {
            continue;
        }
        let mut cross;
        let mut crossprod = true;
        if p[(index + 1) % n].1 == p[index].1 {
            // find_min_max_x_at_y
            let y = p[index].1;
            let (mut min, mut max) = (p[index].0, p[index].0);
            let (mut min_i, mut max_i) = (index, index);
            for (k, q) in p.iter().enumerate().skip(index + 1) {
                if q.1 != y {
                    break;
                }
                if q.0 < min {
                    min = q.0;
                    min_i = k;
                } else if q.0 > max {
                    max = q.0;
                    max_i = k;
                }
            }
            if min_i != max_i {
                cross = min_i as f32 - max_i as f32;
                crossprod = false;
            } else {
                cross = 0.0;
            }
        } else {
            cross = 0.0;
        }
        if crossprod {
            let diff = |inc: usize| {
                let mut k = index;
                loop {
                    k = (k + inc) % n;
                    if k == index || p[k] != p[index] {
                        break;
                    }
                }
                k
            };
            let prev = diff(n - 1);
            if prev == index {
                continue;
            }
            let next = diff(1);
            cross = cross_prod(p[prev], p[index], p[next]);
            if cross == 0.0 && p[prev].1 == p[index].1 && p[next].1 == p[index].1 {
                cross = p[index].0 - p[next].0;
            }
        }
        if cross != 0.0 {
            ymax = p[index].1;
            ymax_cross = cross;
        }
    }
    if ymax_cross != 0.0 {
        Some(ymax_cross > 0.0)
    } else {
        None
    }
}

/// The path with its contours reversed (`SkPath::reverseAddPath`).
fn reversed(path: &pm_skia_path::Path) -> Option<pm_skia_path::Path> {
    // Collect contours as point lists with their verbs, then emit each
    // backwards.
    #[derive(Clone, Copy)]
    enum Seg {
        Line(pm_skia_path::Point),
        Quad(pm_skia_path::Point, pm_skia_path::Point),
        Conic(pm_skia_path::Point, pm_skia_path::Point, f32),
        Cubic(
            pm_skia_path::Point,
            pm_skia_path::Point,
            pm_skia_path::Point,
        ),
    }
    /// A contour: its start, its segments (each with its start point), and
    /// whether it is closed.
    type Contour = (pm_skia_path::Point, Vec<(pm_skia_path::Point, Seg)>, bool);
    let mut contours: Vec<Contour> = Vec::new();
    let mut last = pm_skia_path::Point::zero();
    for seg in path.segments() {
        match seg {
            pm_skia_path::PathSegment::MoveTo(p) => {
                contours.push((p, Vec::new(), false));
                last = p;
            }
            pm_skia_path::PathSegment::LineTo(p) => {
                contours.last_mut()?.1.push((last, Seg::Line(p)));
                last = p;
            }
            pm_skia_path::PathSegment::QuadTo(c, p) => {
                contours.last_mut()?.1.push((last, Seg::Quad(c, p)));
                last = p;
            }
            pm_skia_path::PathSegment::ConicTo(c, p, w) => {
                contours.last_mut()?.1.push((last, Seg::Conic(c, p, w)));
                last = p;
            }
            pm_skia_path::PathSegment::CubicTo(c0, c1, p) => {
                contours.last_mut()?.1.push((last, Seg::Cubic(c0, c1, p)));
                last = p;
            }
            pm_skia_path::PathSegment::Close => {
                contours.last_mut()?.2 = true;
            }
        }
    }
    let mut b = pm_skia_path::PathBuilder::new();
    for (start, segs, closed) in &contours {
        let end = segs.last().map_or(*start, |(_, s)| match *s {
            Seg::Line(p) | Seg::Quad(_, p) | Seg::Conic(_, p, _) | Seg::Cubic(_, _, p) => p,
        });
        b.move_to(end.x, end.y);
        for (from, s) in segs.iter().rev() {
            match *s {
                Seg::Line(_) => b.line_to(from.x, from.y),
                Seg::Quad(c, _) => b.quad_to(c.x, c.y, from.x, from.y),
                Seg::Conic(c, _, w) => b.conic_to(c.x, c.y, from.x, from.y, w),
                Seg::Cubic(c0, c1, _) => b.cubic_to(c1.x, c1.y, c0.x, c0.y, from.x, from.y),
            }
        }
        if *closed {
            b.close();
        }
    }
    b.finish()
}

fn append(dst: &mut pm_skia_path::PathBuilder, src: &pm_skia_path::Path) {
    for seg in src.segments() {
        match seg {
            pm_skia_path::PathSegment::MoveTo(p) => dst.move_to(p.x, p.y),
            pm_skia_path::PathSegment::LineTo(p) => dst.line_to(p.x, p.y),
            pm_skia_path::PathSegment::QuadTo(c, p) => dst.quad_to(c.x, c.y, p.x, p.y),
            pm_skia_path::PathSegment::ConicTo(c, p, w) => dst.conic_to(c.x, c.y, p.x, p.y, w),
            pm_skia_path::PathSegment::CubicTo(c0, c1, p) => {
                dst.cubic_to(c0.x, c0.y, c1.x, c1.y, p.x, p.y)
            }
            pm_skia_path::PathSegment::Close => dst.close(),
        }
    }
}

/// `frame` applied to `user` (`SkStrokeRec::applyToPath`, resolution
/// scale 1): the stroke, plus the path itself for stroke-and-fill, added
/// so its first contour winds clockwise.
fn framed(user: &pm_skia_path::Path, frame: Frame) -> Option<pm_skia_path::Path> {
    let stroke = pm_skia_path::Stroke {
        width: frame.width,
        miter_limit: frame.miter_limit,
        line_cap: pm_skia_path::LineCap::Butt,
        line_join: frame.join,
    };
    let stroked = user.stroke(&stroke, 1.0)?;
    if !frame.fill {
        return Some(stroked);
    }
    let mut b = pm_skia_path::PathBuilder::new();
    append(&mut b, &stroked);
    match first_direction(user) {
        Some(false) => append(&mut b, &reversed(user)?),
        _ => append(&mut b, user),
    }
    b.finish()
}

/// The mask of a glyph path given in `f64` (tests): see [`frame_mask`].
pub fn path_mask(device: &BezPath, scale: f32, frame: Option<Frame>) -> Option<Arc<GlyphMask>> {
    frame_mask(&to_skia(device, Affine::IDENTITY)?, scale, frame)
}

/// The mask of a glyph, as Skia generates it from the glyph's path.
///
/// `device` is the glyph's outline in device px relative to its whole-pixel
/// origin, the subpixel offset already added (`SkScalerContext::
/// internalGetPath`); `scale` is the device transform's 2×2 part
/// (`getMatrixFrom2x2`, a uniform scale for level 1 rasters): the frame
/// applies with it removed. Coverage is before the mask gamma.
pub fn frame_mask(
    device: &pm_skia_path::Path,
    scale: f32,
    frame: Option<Frame>,
) -> Option<Arc<GlyphMask>> {
    let path = match frame {
        None => device.clone(),
        Some(frame) => {
            // `SkMatrix::invert` of a scale: `1 / s` in `f32`.
            let inv = 1.0f32 / scale;
            if !inv.is_finite() {
                return None;
            }
            let user = if scale == 1.0 {
                device.clone()
            } else {
                device
                    .clone()
                    .transform(pm_skia_path::Transform::from_scale(inv, inv))?
            };
            let framed = framed(&user, frame)?;
            if scale == 1.0 {
                framed
            } else {
                framed.transform(pm_skia_path::Transform::from_scale(scale, scale))?
            }
        }
    };
    // `GenerateMetricsFromPath`: the bounds rounded out, then one more
    // pixel each side (the mask is generated "from LCD": Chromium asks for
    // subpixel text on a surface of unknown pixel geometry, so Skia sets
    // `kGenA8FromLCD`).
    let bounds = path.bounds();
    let (l, t, r, b) = (
        bounds.left().floor() as i32,
        bounds.top().floor() as i32,
        bounds.right().ceil() as i32,
        bounds.bottom().ceil() as i32,
    );
    if r <= l || b <= t || r - l > 8192 || b - t > 8192 {
        return None;
    }
    let (left, top) = (l - 1, t);
    let (w, h) = ((r - l + 2) as u32, (b - t) as u32);
    // `GenerateImageFromPath` from LCD: the path at 4× horizontally into an
    // A8 of `4 × (w − 2)` samples, then `pack4xHToMask`.
    let samples = 4 * (w - 2);
    let wide = path.transform(pm_skia_path::Transform::from_row(
        4.0,
        0.0,
        0.0,
        1.0,
        -((left + 1) as f32) * 4.0,
        -(top as f32),
    ))?;
    let a8 = super::skaaa::fill_path_a8(&wide, samples, h);
    let coverage = pack_4x_h(&a8, samples as usize, h as usize);
    Some(Arc::new(GlyphMask {
        width: w,
        height: h,
        left,
        top,
        coverage,
    }))
}

/// `pack4xHToMask` to A8: each output pixel is the mean of three subpixel
/// FIRs over the 4× samples (coefficients sum to 0x110: a little ink
/// spread), before the mask gamma.
/// A plain A8 glyph image Skia generates from a path (`GenerateImageFromPath`
/// without `kGenA8FromLCD`, as the distance-field strike's antialiased
/// edging asks): `path` framed by `frame` in its own space, its bounds
/// rounded out, filled by Skia's analytic filler. `(left, top, width,
/// height, coverage)`.
pub fn frame_a8(path: &pm_skia_path::Path, frame: Frame) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    let framed = framed(path, frame)?;
    let bounds = framed.bounds();
    let (l, t, r, b) = (
        bounds.left().floor() as i32,
        bounds.top().floor() as i32,
        bounds.right().ceil() as i32,
        bounds.bottom().ceil() as i32,
    );
    if r <= l || b <= t || r - l > 8192 || b - t > 8192 {
        return None;
    }
    let (w, h) = ((r - l) as u32, (b - t) as u32);
    let moved = framed.transform(pm_skia_path::Transform::from_translate(
        -(l as f32),
        -(t as f32),
    ))?;
    Some((l, t, w, h, super::skaaa::fill_path_a8(&moved, w, h)))
}

fn pack_4x_h(src: &[u8], sample_width: usize, height: usize) -> Vec<u8> {
    const COEFFICIENTS: [[u32; 12]; 3] = [
        [
            0x03, 0x0b, 0x1c, 0x33, 0x40, 0x39, 0x24, 0x10, 0x05, 0x01, 0x00, 0x00,
        ],
        [
            0x00, 0x02, 0x08, 0x16, 0x2b, 0x3d, 0x3d, 0x2b, 0x16, 0x08, 0x02, 0x00,
        ],
        [
            0x00, 0x00, 0x01, 0x05, 0x10, 0x24, 0x39, 0x40, 0x33, 0x1c, 0x0b, 0x03,
        ],
    ];
    let out_w = sample_width / 4 + 2;
    let mut out = Vec::with_capacity(out_w * height);
    let sw = sample_width as i64;
    for y in 0..height {
        let row = &src[y * sample_width..(y + 1) * sample_width];
        let mut sx = -4i64;
        while sx < sw + 4 {
            let mut fir = [0u32; 3];
            let start = (sx - 4).max(0);
            let first = (start - (sx - 4)) as usize;
            for (coeff, si) in (start..(sx + 8).min(sw)).enumerate() {
                let v = u32::from(row[si as usize]);
                for (f, c) in fir.iter_mut().zip(&COEFFICIENTS) {
                    *f += c[first + coeff] * v;
                }
            }
            let fir = fir.map(|f| (f / 0x100).min(255));
            out.push(((fir[0] + fir[1] + fir[2]) / 3) as u8);
            sx += 4;
        }
    }
    out
}

/// Skia's mask gamma table for luminance bucket `bucket` (of 8; the paint
/// luminance's top 3 bits after the Mac scaler's adjustments), with
/// contrast 0 and sRGB luminance for both paint and device, as
/// `SkTMaskGamma_build_correcting_lut` builds it in `f32`.
pub fn skia_mask_gamma(bucket: usize) -> [u8; 256] {
    fn to_luma(v: f32) -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    fn from_luma(v: f32) -> f32 {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }
    let i = (bucket as u32).min(7);
    let src_i = (i << 5) | (i << 2) | (i >> 1);
    let src = src_i as f32 / 255.0;
    let lin_src = to_luma(src);
    let dst = 1.0 - src;
    let lin_dst = to_luma(dst);
    let mut table = [0u8; 256];
    let mut ii = 0.0f32;
    for t in table.iter_mut() {
        let srca = ii / 255.0;
        let v = if (src - dst).abs() < 1.0 / 256.0 {
            srca
        } else {
            let lin_out = lin_src * srca + (1.0 - srca) * lin_dst;
            (from_luma(lin_out) - dst) / (src - dst)
        };
        *t = (255.0 * v + 0.5).floor().clamp(0.0, 255.0) as u8;
        ii += 1.0;
    }
    table
}

/// CoreGraphics' smoothed coverage made linear (`SkScalerContext_Mac`'s
/// `gLinearCoverageFromCGLCDValue`, applied to the black-on-white pixel).
pub fn linear_from_cg(coverage: u8) -> u8 {
    let p = 255 - u32::from(coverage);
    255 - ((p * p + 128) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured Canvas2D tables are Skia's mask gamma applied to
    /// CoreGraphics' coverage made linear: the model holds for every entry,
    /// so path masks (which skip CoreGraphics) take the gamma alone.
    #[test]
    fn measured_tables_are_the_mask_gamma_of_linear_cg_coverage() {
        for (bucket, measured) in super::super::mask_tables::TABLES.iter().enumerate() {
            let g = skia_mask_gamma(bucket);
            for c in 0..=255u8 {
                assert_eq!(
                    g[linear_from_cg(c) as usize],
                    measured[c as usize],
                    "bucket {bucket} coverage {c}"
                );
            }
        }
    }

    #[test]
    fn fake_bold_scale_interpolates_like_skia() {
        assert_eq!(fake_bold_scale(4.0), 1.0 / 24.0);
        assert_eq!(fake_bold_scale(9.0), 1.0 / 24.0);
        assert_eq!(fake_bold_scale(36.0), 1.0 / 32.0);
        assert_eq!(fake_bold_scale(80.0), 1.0 / 32.0);
        let mid = fake_bold_scale(22.5);
        assert!((mid - (1.0 / 24.0 + 1.0 / 32.0) / 2.0).abs() < 1e-7);
    }

    #[test]
    fn a_square_fills_whole_pixels_and_a_frame_grows_it() {
        let mut sq = BezPath::new();
        sq.move_to((1.0, 1.0));
        sq.line_to((5.0, 1.0));
        sq.line_to((5.0, 5.0));
        sq.line_to((1.0, 5.0));
        sq.close_path();
        // The mask is generated from LCD: one more pixel each side, and the
        // subpixel FIRs spread the ink a little.
        let m = path_mask(&sq, 1.0, None).unwrap();
        assert_eq!((m.left, m.top, m.width, m.height), (0, 1, 6, 4));
        for row in m.coverage.chunks(6) {
            assert_eq!(row, [42, 224, 255, 255, 224, 42]);
        }
        let bold = Frame {
            width: 2.0,
            fill: true,
            join: pm_skia_path::LineJoin::Miter,
            miter_limit: 4.0,
        };
        let b = path_mask(&sq, 1.0, Some(bold)).unwrap();
        assert_eq!((b.left, b.top, b.width, b.height), (-1, 0, 8, 6));
        assert!(
            b.coverage
                .chunks(8)
                .all(|r| r[2..6].iter().all(|&c| c == 255))
        );
        // The same frame at 2× in user space is twice as wide in device px.
        let mut sq2 = BezPath::new();
        sq2.move_to((2.0, 2.0));
        sq2.line_to((10.0, 2.0));
        sq2.line_to((10.0, 10.0));
        sq2.line_to((2.0, 10.0));
        sq2.close_path();
        let b2 = path_mask(&sq2, 2.0, Some(bold)).unwrap();
        assert_eq!((b2.left, b2.top, b2.width, b2.height), (-1, 0, 14, 12));
        // Counter-clockwise contours fill too (the fill is added clockwise).
        let mut ccw = BezPath::new();
        ccw.move_to((1.0, 1.0));
        ccw.line_to((1.0, 5.0));
        ccw.line_to((5.0, 5.0));
        ccw.line_to((5.0, 1.0));
        ccw.close_path();
        let c = path_mask(&ccw, 1.0, Some(bold)).unwrap();
        assert_eq!(c.coverage, b.coverage);
    }
}
