//! Ordered exact-outline coverage and color glyph draws in device space.

use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use kurbo::{Affine, BezPath, PathEl, Rect, Shape, Stroke, StrokeOpts};

use super::bitmap::{Bitmaps, Rgba8, has_bitmaps};
use super::color::Rgba;
use super::colr::{Program, RgbaF32};
use super::outline::{Coords, FaceRef, GlyphStyle, Outlines, synthetic_bold_width};
use crate::compat::PlacedGlyph;

#[derive(Clone, Debug, PartialEq)]
pub enum PathStyle {
    Fill,
    Stroke(Stroke),
}

#[derive(Clone, Debug)]
pub struct PlacedPath {
    pub path: Arc<BezPath>,
    pub transform: Affine,
    pub style: PathStyle,
}

/// Maximum curve-to-line deviation in device pixels before area integration.
/// The CPU comparison uses the same cached geometry.
pub const COVERAGE_TOLERANCE: f64 = 1e-4;

pub(crate) fn flatten_device_path(path: &BezPath) -> BezPath {
    let mut lines = BezPath::new();
    kurbo::flatten(path.iter(), COVERAGE_TOLERANCE, |el| lines.push(el));
    lines
}

fn local_coverage_tolerance(transform: Affine) -> f64 {
    let scale = transform.spectral_norm();
    if scale.is_finite() && scale > 0.0 {
        COVERAGE_TOLERANCE / scale
    } else {
        COVERAGE_TOLERANCE
    }
}

/// Return a line-only fill path in page coordinates for one coverage draw.
/// Strokes expand in local space before the full affine is applied, so
/// nonuniform transforms preserve their authored stroke shape.
pub fn coverage_path(placed: &PlacedPath, to_page: Affine) -> BezPath {
    let transform = to_page * placed.transform;
    let mut lines = BezPath::new();
    match &placed.style {
        PathStyle::Fill => kurbo::flatten(
            placed.path.iter().map(|el| transform * el),
            COVERAGE_TOLERANCE,
            |el| lines.push(el),
        ),
        PathStyle::Stroke(stroke) => {
            let path = kurbo::stroke(
                placed.path.iter(),
                stroke,
                &StrokeOpts::default(),
                local_coverage_tolerance(transform),
            );
            kurbo::flatten(
                path.iter().map(|el| transform * el),
                COVERAGE_TOLERANCE,
                |el| lines.push(el),
            );
        }
    }
    lines
}

/// Resolve the nonzero winding of a line-only path into disjoint filled
/// contours. The area rasterizer integrates signed edges, so retaining two
/// overlapping component contours would count their shared area twice.
pub(crate) fn resolve_nonzero(lines: &BezPath) -> BezPath {
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut contour = Vec::new();
    let finish = |contours: &mut Vec<Vec<[f64; 2]>>, contour: &mut Vec<[f64; 2]>| {
        if contour.len() >= 3 {
            if contour.first() == contour.last() {
                contour.pop();
            }
            if contour.len() >= 3 {
                contours.push(std::mem::take(contour));
                return;
            }
        }
        contour.clear();
    };
    for el in lines.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut contours, &mut contour);
                contour.push([p.x, p.y]);
            }
            PathEl::LineTo(p) => {
                let point = [p.x, p.y];
                if contour.last() != Some(&point) {
                    contour.push(point);
                }
            }
            PathEl::ClosePath => finish(&mut contours, &mut contour),
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
                unreachable!("coverage paths were flattened before nonzero resolution")
            }
        }
    }
    finish(&mut contours, &mut contour);
    if contours.is_empty() {
        return BezPath::new();
    }
    let shapes = contours.simplify_shape_as::<i64>(FillRule::NonZero);
    let mut resolved = BezPath::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        if let Some(first) = contour.first() {
            resolved.move_to((first[0], first[1]));
            for point in &contour[1..] {
                resolved.line_to((point[0], point[1]));
            }
            resolved.close_path();
        }
    }
    resolved
}

/// Maximum retained path payload in the default cross-frame cache.
pub const COVERAGE_PATH_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CoverageKey {
    source: usize,
    linear: [u64; 4],
}

struct CoverageEntry {
    // Retaining the source makes its pointer identity safe as a cache key.
    source: Arc<BezPath>,
    style: PathStyle,
    lines: Arc<BezPath>,
    bytes: usize,
    used: u64,
}

/// Bounded LRU of line-only coverage geometry after the placed path's
/// linear transform. Glyph and atlas translations are applied after lookup.
pub struct CoveragePathCache {
    entries: HashMap<CoverageKey, Vec<CoverageEntry>>,
    bytes: usize,
    limit: usize,
    tick: u64,
}

impl Default for CoveragePathCache {
    fn default() -> Self {
        Self::new(COVERAGE_PATH_CACHE_BYTES)
    }
}

impl CoveragePathCache {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            limit,
            tick: 0,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.tick = 0;
    }

    fn oldest(&self) -> Option<(CoverageKey, usize)> {
        self.entries
            .iter()
            .flat_map(|(key, bucket)| {
                bucket
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (*key, index, entry.used))
            })
            .min_by_key(|(_, _, used)| *used)
            .map(|(key, index, _)| (key, index))
    }

    fn evict_oldest(&mut self) {
        let Some((key, index)) = self.oldest() else {
            return;
        };
        let bucket = self.entries.get_mut(&key).expect("oldest entry exists");
        let entry = bucket.swap_remove(index);
        let empty = bucket.is_empty();
        self.bytes -= entry.bytes;
        if empty {
            self.entries.remove(&key);
        }
    }

    /// Reuse line-only geometry for the same retained source, linear affine,
    /// and stroke style. Apply `placed.transform`'s translation separately
    /// when drawing the returned Arc. It remains valid after eviction.
    pub fn get(&mut self, placed: &PlacedPath) -> Arc<BezPath> {
        self.tick = self.tick.wrapping_add(1);
        let [a, b, c, d, _, _] = placed.transform.as_coeffs();
        let key = CoverageKey {
            source: Arc::as_ptr(&placed.path) as usize,
            linear: [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()],
        };
        if let Some(hit) = self.entries.get_mut(&key).and_then(|bucket| {
            bucket.iter_mut().find(|entry| {
                Arc::ptr_eq(&entry.source, &placed.path) && entry.style == placed.style
            })
        }) {
            hit.used = self.tick;
            return hit.lines.clone();
        }
        let linear = PlacedPath {
            transform: Affine::new([a, b, c, d, 0.0, 0.0]),
            ..placed.clone()
        };
        let raw = coverage_path(&linear, Affine::IDENTITY);
        let elements = resolve_nonzero(&raw).into_elements();
        let line_capacity = elements.capacity();
        let lines = Arc::new(BezPath::from_vec(elements));
        let bytes = (placed.path.elements().len() + line_capacity)
            .saturating_mul(size_of::<kurbo::PathEl>())
            .saturating_add(size_of::<CoverageEntry>() + size_of::<CoverageKey>());
        if bytes <= self.limit {
            while self.bytes > self.limit - bytes {
                self.evict_oldest();
            }
            self.entries.entry(key).or_default().push(CoverageEntry {
                source: placed.path.clone(),
                style: placed.style.clone(),
                lines: lines.clone(),
                bytes,
                used: self.tick,
            });
            self.bytes += bytes;
        }
        lines
    }
}

#[derive(Clone, Debug)]
pub enum ImagePixels {
    Rgba8(Arc<Rgba8>),
    WorkingF32(Arc<RgbaF32>),
    Colr(Arc<Program>),
}

impl ImagePixels {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Rgba8(i) => (i.width, i.height),
            Self::WorkingF32(i) => (i.width, i.height),
            Self::Colr(i) => (i.width, i.height),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ItemContent {
    Coverage {
        paths: Vec<PlacedPath>,
    },
    Image {
        image: ImagePixels,
        transform: Affine,
    },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub content: ItemContent,
    /// Straight E-encoded paint color; image draws use its alpha.
    pub color: Rgba,
    pub opacity: f64,
    /// Gaussian sigma in device pixels.
    pub blur: f64,
    /// Conservative pre-blur device-space ink box.
    pub bounds: Rect,
}

impl Item {
    pub fn reach(&self) -> Rect {
        let pad = blur_radius(self.blur) as f64 + 1.0;
        self.bounds.inflate(pad, pad)
    }
}

pub fn blur_radius(sigma: f64) -> u32 {
    if sigma > 0.0 && sigma.is_finite() {
        (8.0 * sigma).ceil().min(u32::MAX as f64) as u32
    } else {
        0
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextScene {
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

impl TextScene {
    pub fn ink(&self) -> Option<[u32; 4]> {
        let mut r: Option<Rect> = None;
        for item in &self.items {
            let b = item.reach();
            r = Some(r.map_or(b, |old| old.union(b)));
        }
        let r = r?.intersect(Rect::new(0.0, 0.0, self.width as f64, self.height as f64));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return None;
        }
        Some([
            r.x0.floor().max(0.0) as u32,
            r.y0.floor().max(0.0) as u32,
            r.x1.ceil().min(self.width as f64) as u32,
            r.y1.ceil().min(self.height as f64) as u32,
        ])
    }
}

#[derive(Clone, Copy)]
pub struct Run<'a> {
    pub face: FaceRef<'a>,
    pub size: f32,
    pub coords: Coords<'a>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub oblique: f64,
    pub glyphs: &'a [PlacedGlyph],
    pub bitmap_size: Option<f32>,
}

pub struct Painter<'a> {
    pub outlines: &'a mut Outlines,
    pub bitmaps: &'a mut Bitmaps,
    pub unsupported: Vec<String>,
}

fn skew(run: &Run<'_>) -> f64 {
    if run.synthetic_italic {
        super::outline::SYNTHETIC_ITALIC_SKEW
    } else if run.oblique != 0.0 {
        -run.oblique.to_radians().tan()
    } else {
        0.0
    }
}

fn device_scale(m: Affine) -> f64 {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let scale = (a * d - b * c).abs().sqrt();
    if scale.is_finite() { scale } else { 0.0 }
}

fn device_residual(m: Affine, scale: f64) -> Affine {
    let [a, b, c, d, _, _] = m.as_coeffs();
    Affine::new([a / scale, b / scale, c / scale, d / scale, 0.0, 0.0])
}

impl Painter<'_> {
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        stroke: Option<f64>,
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) {
        let scale = device_scale(m);
        if scale <= 0.0 || !scale.is_finite() {
            return;
        }
        let residual = device_residual(m, scale);
        for run in runs {
            let device_size = f64::from(run.size) * scale;
            let style = GlyphStyle {
                size: run.size,
                synthetic_bold: if run.synthetic_bold {
                    let outset = synthetic_bold_width(f64::from(run.size)) / 2.0;
                    (outset, outset)
                } else {
                    (0.0, 0.0)
                },
                skew: skew(run),
                expand_tolerance: if run.synthetic_bold {
                    local_coverage_tolerance(m * Affine::skew(skew(run), 0.0))
                } else {
                    0.0
                },
            };
            for glyph in run.glyphs {
                let glyph_origin = m * kurbo::Point::new(origin.0 + glyph.x, origin.1 + glyph.y);
                let bitmap_size = run.bitmap_size.unwrap_or(device_size as f32);
                if has_bitmaps(run.face)
                    && let Some(bitmap) =
                        self.bitmaps
                            .get(run.face, glyph.id, device_size as f32, bitmap_size)
                {
                    let transform = Affine::translate((glyph_origin.x, glyph_origin.y))
                        * residual
                        * bitmap.transform;
                    let image = ImagePixels::Rgba8(bitmap.image.clone());
                    let (w, h) = image.dimensions();
                    out.push(Item {
                        bounds: transform
                            .transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64)),
                        content: ItemContent::Image { image, transform },
                        color,
                        opacity,
                        blur,
                    });
                    continue;
                }
                if super::bitmap::is_colr_glyph(run.face, glyph.id) {
                    let [a, b, c, d, _, _] = m.as_coeffs();
                    let linear = Affine::new([a, b, c, d, 0.0, 0.0]);
                    match self.bitmaps.colr.get(super::colr::ColrRequest {
                        face: run.face,
                        coords: run.coords,
                        id: glyph.id,
                        size: run.size,
                        skew: skew(run) as f32,
                        linear,
                        origin: (glyph_origin.x, glyph_origin.y),
                        // Fill-style alpha is applied once to the completed
                        // color glyph, including palette-only paints.
                        foreground: [color[0], color[1], color[2], 1.0],
                    }) {
                        Ok(Some(program)) => {
                            let transform =
                                Affine::translate((glyph_origin.x.floor(), glyph_origin.y.floor()))
                                    * program.transform;
                            let image = ImagePixels::Colr(program);
                            let (w, h) = image.dimensions();
                            out.push(Item {
                                bounds: transform
                                    .transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64)),
                                content: ItemContent::Image { image, transform },
                                // Canvas fill-style alpha applies to the whole
                                // color glyph, including palette-only paints.
                                color,
                                opacity,
                                blur,
                            });
                        }
                        Ok(None) => {}
                        Err(e) => self.unsupported.push(format!("COLR: {e}")),
                    }
                    continue;
                }
                let Some(outline) = self.outlines.get(run.face, run.coords, glyph.id, style) else {
                    continue;
                };
                let transform = m * Affine::translate((origin.0 + glyph.x, origin.1 + glyph.y));
                let path_style = if let Some(width) = stroke {
                    PathStyle::Stroke(
                        Stroke::new(width)
                            .with_join(kurbo::Join::Round)
                            .with_miter_limit(2.0),
                    )
                } else {
                    PathStyle::Fill
                };
                let mut b = transform.transform_rect_bbox(outline.bounds);
                if let Some(width) = stroke {
                    b = b.inflate(width * scale / 2.0, width * scale / 2.0);
                }
                out.push(Item {
                    content: ItemContent::Coverage {
                        paths: vec![PlacedPath {
                            path: outline.path.clone(),
                            transform,
                            style: path_style,
                        }],
                    },
                    color,
                    opacity,
                    blur,
                    bounds: b,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_text(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) {
        self.draw(out, runs, origin, m, None, color, opacity, blur);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_text(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        width: f64,
        color: Rgba,
        opacity: f64,
    ) {
        if width > 0.0 {
            self.draw(out, runs, origin, m, Some(width), color, opacity, 0.0);
        }
    }
}

pub fn fill_path(path: BezPath, m: Affine, color: Rgba, opacity: f64) -> Option<Item> {
    let bounds = m.transform_rect_bbox(path.bounding_box());
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    Some(Item {
        content: ItemContent::Coverage {
            paths: vec![PlacedPath {
                path: Arc::new(path),
                transform: m,
                style: PathStyle::Fill,
            }],
        },
        color,
        opacity,
        blur: 0.0,
        bounds,
    })
}

/// Analytic outline of the authored rounded caption rectangle.
pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> BezPath {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut p = BezPath::new();
    if r <= 0.0 {
        p.move_to((x, y));
        p.line_to((x + w, y));
        p.line_to((x + w, y + h));
        p.line_to((x, y + h));
        p.close_path();
        return p;
    }
    const K: f64 = 0.552_284_749_830_793_4;
    let k = r * K;
    p.move_to((x + r, y));
    p.line_to((x + w - r, y));
    p.curve_to((x + w - r + k, y), (x + w, y + r - k), (x + w, y + r));
    p.line_to((x + w, y + h - r));
    p.curve_to(
        (x + w, y + h - r + k),
        (x + w - r + k, y + h),
        (x + w - r, y + h),
    );
    p.line_to((x + r, y + h));
    p.curve_to((x + r - k, y + h), (x, y + h - r + k), (x, y + h - r));
    p.line_to((x, y + r));
    p.curve_to((x, y + r - k), (x + r - k, y), (x + r, y));
    p.close_path();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fractional_placement_is_retained() {
        let path = rounded_rect(0.0, 0.0, 1.0, 1.0, 0.0);
        let item = fill_path(path, Affine::translate((0.125, 0.375)), [1.0; 4], 1.0).unwrap();
        assert_eq!(item.bounds.x0, 0.125);
        assert_eq!(item.bounds.y0, 0.375);
    }

    #[test]
    fn blur_reach_uses_eight_sigma() {
        assert_eq!(blur_radius(2.25), 18);
    }

    #[test]
    fn stroked_path_expands_before_nonuniform_world_transform() {
        let mut line = BezPath::new();
        line.move_to((1.0, 2.0));
        line.line_to((3.0, 2.0));
        let placed = PlacedPath {
            path: Arc::new(line),
            transform: Affine::scale_non_uniform(10.0, 2.0),
            style: PathStyle::Stroke(Stroke::new(2.0)),
        };
        let path = coverage_path(&placed, Affine::translate((5.0, 7.0)));
        assert!(path.elements().iter().all(|el| matches!(
            el,
            kurbo::PathEl::MoveTo(_) | kurbo::PathEl::LineTo(_) | kurbo::PathEl::ClosePath
        )));
        let b = path.bounding_box();
        // Kurbo's default round caps extend one local pixel past each end.
        assert!((b.x0 - 5.0).abs() < 1e-8 && (b.x1 - 45.0).abs() < 1e-8);
        assert!((b.y0 - 9.0).abs() < 1e-8 && (b.y1 - 13.0).abs() < 1e-8);
        assert!(
            (local_coverage_tolerance(placed.transform) - COVERAGE_TOLERANCE / 10.0).abs() < 1e-12
        );
    }

    #[test]
    fn flattened_coverage_cache_reuses_and_bounds_retained_paths() {
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.quad_to((4.0, 8.0), (8.0, 0.0));
        curve.close_path();
        let source = Arc::new(curve);
        let first = PlacedPath {
            path: source.clone(),
            transform: Affine::IDENTITY,
            style: PathStyle::Fill,
        };
        let shifted = PlacedPath {
            transform: Affine::translate((0.25, 0.0)),
            ..first.clone()
        };
        let scaled = PlacedPath {
            transform: Affine::scale_non_uniform(1.1, 1.0),
            ..first.clone()
        };
        let stroked = PlacedPath {
            style: PathStyle::Stroke(Stroke::new(1.0)),
            ..first.clone()
        };
        let mut unlimited = CoveragePathCache::new(usize::MAX);
        let lines = unlimited.get(&first);
        assert!(Arc::ptr_eq(&lines, &unlimited.get(&first)));
        assert!(Arc::ptr_eq(&lines, &unlimited.get(&shifted)));
        assert!(!Arc::ptr_eq(&lines, &unlimited.get(&scaled)));
        assert!(!Arc::ptr_eq(&lines, &unlimited.get(&stroked)));

        let mut probe = CoveragePathCache::new(usize::MAX);
        probe.get(&first);
        let one_entry_bytes = probe.bytes();
        let mut limited = CoveragePathCache::new(one_entry_bytes);
        let old = limited.get(&first);
        assert!(Arc::ptr_eq(&old, &limited.get(&first)));
        let same_shape_new_source = PlacedPath {
            path: Arc::new((*source).clone()),
            ..first.clone()
        };
        limited.get(&same_shape_new_source);
        assert!(limited.bytes() <= one_entry_bytes);
        assert!(!Arc::ptr_eq(&old, &limited.get(&first)));
    }

    #[test]
    fn cached_nonzero_contours_cover_overlap_once() {
        let mut components = BezPath::new();
        components.move_to((0.0, 0.0));
        components.line_to((2.0, 0.0));
        components.line_to((2.0, 2.0));
        components.line_to((0.0, 2.0));
        components.close_path();
        components.move_to((1.0, 0.0));
        components.line_to((3.0, 0.0));
        components.line_to((3.0, 2.0));
        components.line_to((1.0, 2.0));
        components.close_path();
        let placed = PlacedPath {
            path: Arc::new(components),
            transform: Affine::IDENTITY,
            style: PathStyle::Fill,
        };
        let raw = coverage_path(&placed, Affine::IDENTITY);
        assert_eq!(raw.winding(kurbo::Point::new(1.5, 1.0)), 2);
        let mut cache = CoveragePathCache::default();
        let resolved = cache.get(&placed);
        assert_eq!(resolved.winding(kurbo::Point::new(1.5, 1.0)), 1);
        assert!((resolved.area().abs() - 6.0).abs() < 1e-9);
        assert_eq!(resolved.winding(kurbo::Point::new(3.5, 1.0)), 0);
        let shifted = PlacedPath {
            transform: Affine::translate((0.375, 0.125)),
            ..placed
        };
        assert!(Arc::ptr_eq(&resolved, &cache.get(&shifted)));
    }

    #[test]
    fn cached_nonzero_stroke_resolves_crossing_segments() {
        let mut crossing = BezPath::new();
        crossing.move_to((0.0, 1.0));
        crossing.line_to((4.0, 1.0));
        crossing.move_to((2.0, -1.0));
        crossing.line_to((2.0, 3.0));
        let placed = PlacedPath {
            path: Arc::new(crossing),
            transform: Affine::IDENTITY,
            style: PathStyle::Stroke(Stroke::new(1.0)),
        };
        let raw = coverage_path(&placed, Affine::IDENTITY);
        assert_eq!(raw.winding(kurbo::Point::new(2.0, 1.0)).abs(), 2);
        let resolved = CoveragePathCache::default().get(&placed);
        assert_eq!(resolved.winding(kurbo::Point::new(2.0, 1.0)).abs(), 1);
    }
}
