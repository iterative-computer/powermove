//! Today's preview crop of a layer raster (`legacy/gl/shape-raster-window.ts`,
//! `compositor.ts` `nativeContentQuad`), for the `raster` override
//! (`crate::compat`; docs/engine/extensions-compat.md, "Crop").
//!
//! In a viewer frame of the open root composition, today's compositor asks
//! `PM.raster` for only the visible window of a large text raster or of a
//! partially visible shape (the fifth `crop` argument), skips a shape the
//! viewport misses, and fills the viewport with a rectangle's color when the
//! viewport lies inside its plain fill (the raster is not asked at all).
//! A wrapper sees those calls, so the engine plans the same decision for
//! the layers a producer draws: the window travels with the need
//! (`CompatNeed::crop`), and a different window is a different answer.
//! Exports and snapshots never crop (today's `opt.exporting` turns source
//! clipping off).

use pm_protocol::common::PixelRect;

/// The viewer's visible window of the frame being planned, in output pixels
/// (today's canvas: `GL.canvas.width` × `GL.canvas.height` at the viewport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewWindow {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What today's compositor does with a layer's raster in a viewer frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CropPlan {
    /// The whole raster.
    Full,
    /// Nothing is drawn: the viewport misses the layer.
    Outside,
    /// The viewport is inside a plain rectangle's fill: today draws the
    /// fill color without asking the raster.
    Solid,
    /// Only this window of the raster (its pixels at the raster density).
    Crop(PixelRect),
}

/// Today's bucket of a crop window edge.
const STEP: f64 = 256.0;

/// `[a, b, c, d, tx, ty]` mapped back from the viewport corners (with
/// today's two-pixel margin) into layer coordinates, or `None` for a
/// matrix today treats as unusable.
fn corners(m: &[f64; 6], width: f64, height: f64) -> Option<[(f64, f64); 4]> {
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    let [a, b, c, d, tx, ty] = *m;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return None;
    }
    let map = |x: f64, y: f64| {
        let (dx, dy) = (x - tx, y - ty);
        ((d * dx - c * dy) / det, (-b * dx + a * dy) / det)
    };
    Some([
        map(-2.0, -2.0),
        map(width + 2.0, -2.0),
        map(-2.0, height + 2.0),
        map(width + 2.0, height + 2.0),
    ])
}

fn bounds(points: &[(f64, f64); 4]) -> (f64, f64, f64, f64) {
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &(x, y) in points {
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
    b
}

/// A text raster's geometry (`textRasterGeometry`): its box in layer
/// pixels, its anchor in the box, and its size in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextGeometry {
    pub w: f64,
    pub h: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub width: f64,
    pub height: f64,
}

impl TextGeometry {
    /// `textRasterGeometry(d, scale)` from the placed box: the density is
    /// `min(scale, 8192 / w, 8192 / h)` and the size `max(1, ceil(w × density))`.
    pub fn at(w: f64, h: f64, anchor_x: f64, anchor_y: f64, scale: f64) -> TextGeometry {
        let density = crate::js::min(crate::js::min(scale, 8192.0 / w), 8192.0 / h);
        TextGeometry {
            w,
            h,
            anchor_x,
            anchor_y,
            width: crate::js::max(1.0, (w * density).ceil()),
            height: crate::js::max(1.0, (h * density).ceil()),
        }
    }
}

/// `rasterIntersectsViewport` then `previewTextRaster`.
pub fn text_plan(g: &TextGeometry, m: &[f64; 6], width: f64, height: f64) -> CropPlan {
    if !intersects(g, m, width, height) {
        return CropPlan::Outside;
    }
    text_window(g, m, width, height).map_or(CropPlan::Full, CropPlan::Crop)
}

/// `rasterIntersectsViewport`: conservative visibility of the whole bitmap.
fn intersects(g: &TextGeometry, m: &[f64; 6], width: f64, height: f64) -> bool {
    if !(g.w > 0.0 && g.h > 0.0) {
        return true;
    }
    let Some(points) = corners(m, width, height) else {
        return true;
    };
    let shifted = points.map(|(x, y)| (x + g.anchor_x, y + g.anchor_y));
    let (x0, y0, x1, y1) = bounds(&shifted);
    !(x1 < 0.0 || y1 < 0.0 || x0 > g.w || y0 > g.h)
}

/// `previewTextRaster`: the visible window of a large, axis-aligned text
/// raster, in 256-pixel buckets; `None` keeps the whole raster.
fn text_window(g: &TextGeometry, m: &[f64; 6], width: f64, height: f64) -> Option<PixelRect> {
    if g.width * g.height < 4.0 * 1024.0 * 1024.0 || !(g.w > 0.0 && g.h > 0.0) {
        return None;
    }
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    let [a, b, c, d, _, _] = *m;
    // Rotation or skew keeps the whole raster (filtering after a crop).
    if !((b.abs() < 1e-9 && c.abs() < 1e-9) || (a.abs() < 1e-9 && d.abs() < 1e-9)) {
        return None;
    }
    let points = corners(m, width, height)?;
    let device = points.map(|(x, y)| {
        (
            (x + g.anchor_x) * g.width / g.w,
            (y + g.anchor_y) * g.height / g.h,
        )
    });
    let (bx0, by0, bx1, by1) = bounds(&device);
    let x = crate::js::max(0.0, ((bx0 - 2.0) / STEP).floor() * STEP);
    let y = crate::js::max(0.0, ((by0 - 2.0) / STEP).floor() * STEP);
    let right = crate::js::min(g.width, ((bx1 + 2.0) / STEP).ceil() * STEP);
    let bottom = crate::js::min(g.height, ((by1 + 2.0) / STEP).ceil() * STEP);
    if right <= x || bottom <= y || (right - x) * (bottom - y) > g.width * g.height / 2.0 {
        return None;
    }
    window(x, y, right - x, bottom - y)
}

/// A shape's content as `previewShapeRaster` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeFacts {
    pub w: f64,
    pub h: f64,
    /// `d.stroke` as a number (NaN when absent).
    pub stroke: f64,
    /// `d.shape === 'rect'` and `d.color` is `#rrggbb`.
    pub plain_rect: bool,
    /// `cornerInset(d)`.
    pub corner_inset: f64,
}

/// `previewShapeRaster` at raster density `scale`.
pub fn shape_plan(d: &ShapeFacts, scale: f64, m: &[f64; 6], width: f64, height: f64) -> CropPlan {
    if !(d.w > 0.0 && d.h > 0.0) {
        return CropPlan::Full;
    }
    let Some(points) = corners(m, width, height) else {
        return CropPlan::Full;
    };
    // `shapeRasterGeometry`.
    // `d.stroke || 0`, then `Number(d.stroke) || 0`: NaN reads as 0.
    let stroke_number = if d.stroke.is_nan() { 0.0 } else { d.stroke };
    let pad = (stroke_number / 2.0).ceil() + 4.0;
    let (gw, gh) = (d.w + pad * 2.0, d.h + pad * 2.0);
    let density = crate::js::min(crate::js::min(scale, 8192.0 / gw), 8192.0 / gh);
    let (pw, ph) = (
        crate::js::max(1.0, (gw * density).ceil()),
        crate::js::max(1.0, (gh * density).ceil()),
    );
    let (x0, y0, x1, y1) = bounds(&points);
    if x1 < -gw / 2.0 || x0 > gw / 2.0 || y1 < -gh / 2.0 || y0 > gh / 2.0 {
        return CropPlan::Outside;
    }
    let half_stroke = crate::js::max(0.0, stroke_number) / 2.0;
    let inset = d.corner_inset + half_stroke + 8.0 / density;
    if d.plain_rect
        && x0 > -d.w / 2.0 + inset
        && x1 < d.w / 2.0 - inset
        && y0 > -d.h / 2.0 + inset
        && y1 < d.h / 2.0 - inset
    {
        return CropPlan::Solid;
    }
    let left = crate::js::max(
        0.0,
        (((x0 + gw / 2.0) * pw / gw - 2.0) / STEP).floor() * STEP,
    );
    let top = crate::js::max(
        0.0,
        (((y0 + gh / 2.0) * ph / gh - 2.0) / STEP).floor() * STEP,
    );
    let right = crate::js::min(pw, (((x1 + gw / 2.0) * pw / gw + 2.0) / STEP).ceil() * STEP);
    let bottom = crate::js::min(ph, (((y1 + gh / 2.0) * ph / gh + 2.0) / STEP).ceil() * STEP);
    if right <= left || bottom <= top {
        return CropPlan::Outside;
    }
    if left == 0.0 && top == 0.0 && right == pw && bottom == ph {
        return CropPlan::Full;
    }
    window(left, top, right - left, bottom - top).map_or(CropPlan::Full, CropPlan::Crop)
}

/// The window of a paths raster a `width` × `height` viewport samples
/// (engine-internal: today draws a paths layer's whole raster, whose pixels
/// a window keeps wherever the frame samples it). The quad covers layer
/// units `(x0, y0)` to `(x0 + w, y0 + h)` with the `bw` × `bh` bitmap
/// stretched over it; `m` maps layer units to viewport pixels. Today's two
/// pixel margin and bucket steps keep sampling inside the window.
#[allow(clippy::too_many_arguments)]
pub fn paths_plan(
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    bw: f64,
    bh: f64,
    m: &[f64; 6],
    width: f64,
    height: f64,
) -> CropPlan {
    if !(w > 0.0 && h > 0.0 && bw >= 1.0 && bh >= 1.0) {
        return CropPlan::Full;
    }
    let Some(points) = corners(m, width, height) else {
        return CropPlan::Full;
    };
    let (sx, sy) = (bw / w, bh / h);
    let texel = points.map(|(x, y)| ((x - x0) * sx, (y - y0) * sy));
    let (tx0, ty0, tx1, ty1) = bounds(&texel);
    if tx1 < 0.0 || ty1 < 0.0 || tx0 > bw || ty0 > bh {
        return CropPlan::Outside;
    }
    let left = crate::js::max(0.0, ((tx0 - 2.0) / STEP).floor() * STEP);
    let top = crate::js::max(0.0, ((ty0 - 2.0) / STEP).floor() * STEP);
    let right = crate::js::min(bw, ((tx1 + 2.0) / STEP).ceil() * STEP);
    let bottom = crate::js::min(bh, ((ty1 + 2.0) / STEP).ceil() * STEP);
    if right <= left || bottom <= top {
        return CropPlan::Outside;
    }
    if left == 0.0 && top == 0.0 && right == bw && bottom == bh {
        return CropPlan::Full;
    }
    window(left, top, right - left, bottom - top).map_or(CropPlan::Full, CropPlan::Crop)
}

fn window(x: f64, y: f64, width: f64, height: f64) -> Option<PixelRect> {
    let ok = |v: f64| v.is_finite() && v >= 0.0 && v <= f64::from(u32::MAX);
    if !(ok(x) && ok(y) && ok(width) && ok(height)) || width < 1.0 || height < 1.0 {
        return None;
    }
    Some(PixelRect {
        x: x as u32,
        y: y as u32,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> CropPlan {
        CropPlan::Crop(PixelRect {
            x,
            y,
            width,
            height,
        })
    }

    fn shape(w: f64, h: f64) -> ShapeFacts {
        ShapeFacts {
            w,
            h,
            stroke: 0.0,
            plain_rect: true,
            corner_inset: 0.0,
        }
    }

    // Expected values are today's `previewShapeRaster` /
    // `previewTextRaster` outputs for the same inputs (computed with the
    // legacy functions, shape-raster-window.test.ts conventions).
    #[test]
    fn a_shape_the_viewport_contains_is_drawn_whole() {
        // A 200 × 100 rect at the centre of a 1920 × 1080 viewport, scale 1.
        let m = [1.0, 0.0, 0.0, 1.0, 960.0, 540.0];
        assert_eq!(
            shape_plan(&shape(200.0, 100.0), 1.0, &m, 1920.0, 1080.0),
            CropPlan::Full
        );
    }

    #[test]
    fn a_shape_the_viewport_misses_is_not_drawn() {
        let m = [1.0, 0.0, 0.0, 1.0, -500.0, 540.0];
        assert_eq!(
            shape_plan(&shape(200.0, 100.0), 1.0, &m, 1920.0, 1080.0),
            CropPlan::Outside
        );
    }

    #[test]
    fn a_viewport_inside_a_plain_rect_fill_is_solid() {
        // 8× zoom into the middle of a 400 × 400 rect.
        let m = [8.0, 0.0, 0.0, 8.0, 100.0, 100.0];
        assert_eq!(
            shape_plan(&shape(400.0, 400.0), 8.0, &m, 200.0, 200.0),
            CropPlan::Solid
        );
        let mut stroked = shape(400.0, 400.0);
        stroked.plain_rect = false;
        // Not a plain rect: the visible window of the raster instead.
        let plan = shape_plan(&stroked, 8.0, &m, 200.0, 200.0);
        // Raster: (400 + 8) × 8 = 3264 px; the viewport maps to layer
        // x ∈ [-12.75, 12.75] → device [1530, 1734] → buckets [1280, 1792].
        assert_eq!(plan, rect(1280, 1280, 512, 512));
    }

    #[test]
    fn a_partly_visible_shape_is_cropped_to_256_pixel_buckets() {
        // A 1000 × 200 rect whose right half leaves a 1000-pixel viewport.
        let m = [1.0, 0.0, 0.0, 1.0, 900.0, 300.0];
        let mut d = shape(1000.0, 200.0);
        d.plain_rect = false;
        // Raster 1008 × 208; viewport x ∈ [-902, 102] → device [0, 610] → [0, 768∧1008].
        assert_eq!(shape_plan(&d, 1.0, &m, 1000.0, 600.0), rect(0, 0, 768, 208));
    }

    #[test]
    fn small_or_rotated_text_keeps_its_whole_raster() {
        let g = TextGeometry::at(400.0, 100.0, 200.0, 50.0, 4.0);
        let m = [4.0, 0.0, 0.0, 4.0, 100.0, 100.0];
        assert_eq!(text_plan(&g, &m, 1920.0, 1080.0), CropPlan::Full);
        let big = TextGeometry::at(2000.0, 400.0, 1000.0, 200.0, 8.0);
        let (s, c) = (0.3f64.sin() * 8.0, 0.3f64.cos() * 8.0);
        assert_eq!(
            text_plan(&big, &[c, s, -s, c, 960.0, 540.0], 1920.0, 1080.0),
            CropPlan::Full
        );
    }

    #[test]
    fn a_large_text_raster_shows_its_visible_window() {
        // 2000 × 400 box at density min(8, 8192/2000) = 4.096 → 8192 × 1639.
        let g = TextGeometry::at(2000.0, 400.0, 1000.0, 200.0, 8.0);
        assert_eq!((g.width, g.height), (8192.0, 1639.0));
        // Zoomed 8×, the viewport sees layer x ∈ [-120.25, 120.25].
        let m = [8.0, 0.0, 0.0, 8.0, 960.0, 540.0];
        // Device x ∈ [3603.4, 4588.6] → [3584, 4608]; y ∈ [598.0, 1041.0] → [512, 1280].
        assert_eq!(
            text_plan(&g, &m, 1920.0, 1080.0),
            rect(3584, 512, 1024, 768)
        );
        // Panned off the layer: not drawn.
        let away = [8.0, 0.0, 0.0, 8.0, 960.0 + 20000.0, 540.0];
        assert_eq!(text_plan(&g, &away, 1920.0, 1080.0), CropPlan::Outside);
    }
}
