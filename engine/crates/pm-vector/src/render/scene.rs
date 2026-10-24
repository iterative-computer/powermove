//! Coverage scenes: what a vector layer draws at output resolution
//! (`compat` level 2, plan section 8 "Sampling and antialiasing").
//!
//! A [`CoverageScene`] is an ordered list of [`CoverageItem`]s, each one
//! fill or stroke of a path in device pixels with one paint. The engine
//! covers every item exactly into its own cell of an `r32float` atlas
//! ([`AtlasLayout`], `pm_render::coverage`) and composites `paint ×
//! coverage` per item in order with premultiplied source-over at the
//! working precision.

use kurbo::{Affine, BezPath, Cap, Join, Rect, Shape, Stroke};

use crate::path::PathLayer;
use crate::render::color::{CanvasColor, StyleSlot};
use crate::shape::{ShapeGeometry, StrokePaint};

/// Largest distance, in device pixels, between an item's curves and the
/// line segments the coverage backends rasterize.
pub const FLATTEN_TOLERANCE: f64 = 1e-3;

/// Fill rule of a filled item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// How an item covers pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemStyle {
    Fill(FillRule),
    /// A stroke in the item's local space (transformed with the path, as
    /// Canvas2D strokes under the current transform).
    Stroke(Stroke),
}

/// One fill or stroke with a solid paint.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageItem {
    /// The path in local coordinates.
    pub path: BezPath,
    /// Local coordinates to device pixels.
    pub transform: Affine,
    pub style: ItemStyle,
    /// Straight RGBA in the working encoding's code values; `a` includes any
    /// `globalAlpha`.
    pub color: [f32; 4],
}

impl CoverageItem {
    /// Device-pixel bounds of everything the item can cover (conservative:
    /// strokes include the miter limit and caps), or `None` when empty or
    /// not finite.
    pub fn device_bounds(&self) -> Option<Rect> {
        if self.path.elements().is_empty() {
            return None;
        }
        let base = (self.transform * &self.path).bounding_box();
        let r = match &self.style {
            ItemStyle::Fill(_) => base,
            ItemStyle::Stroke(s) => {
                let c = self.transform.as_coeffs();
                let scale = libm::hypot(c[0], c[1]).max(libm::hypot(c[2], c[3]));
                let half = s.width / 2.0;
                let joins = match s.join {
                    Join::Miter => s.miter_limit.max(1.0),
                    _ => 1.0,
                };
                let caps = match (s.start_cap, s.end_cap) {
                    (Cap::Square, _) | (_, Cap::Square) => core::f64::consts::SQRT_2,
                    _ => 1.0,
                };
                base.inflate(half * joins.max(caps) * scale, half * joins.max(caps) * scale)
            }
        };
        let finite = r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite();
        (finite && r.width() >= 0.0 && r.height() >= 0.0).then_some(r)
    }

    /// The area the item covers as disjoint contours in device pixels:
    /// fills are transformed and flattened, strokes expanded in local space
    /// (Canvas2D strokes under the current transform), then transformed and
    /// flattened, all within [`FLATTEN_TOLERANCE`]; then the fill rule is
    /// resolved into contours that do not overlap.
    ///
    /// Flattening within 10⁻³ px moves an edge by less than that (chords
    /// lie inside convex curves: a coarse tolerance erodes curved edges).
    /// Area coverage sums the winding number over a pixel before the fill
    /// rule, so where overlapping pieces (a stroke's inner join through its
    /// pivot, a self-intersecting fill) meet an edge inside one pixel it
    /// covers too much: the inner corner of a stroked rectangle drew 1
    /// where it is 3/4. Resolving the rule first leaves nothing to sum.
    pub fn device_outline(&self) -> BezPath {
        let (flat, rule) = self.flattened();
        resolve(&flat, rule)
    }

    fn flattened(&self) -> (BezPath, FillRule) {
        let (local, rule) = match &self.style {
            ItemStyle::Fill(rule) => (self.path.clone(), *rule),
            ItemStyle::Stroke(stroke) => {
                // Expand finely enough that the transformed outline stays
                // within the tolerance.
                let c = self.transform.as_coeffs();
                let scale = libm::hypot(c[0], c[1]).max(libm::hypot(c[2], c[3])).max(1e-9);
                let outline = kurbo::stroke(
                    self.path.iter(),
                    stroke,
                    &kurbo::StrokeOpts::default(),
                    FLATTEN_TOLERANCE / scale / 4.0,
                );
                (outline, FillRule::NonZero)
            }
        };
        let device = self.transform * local;
        let mut out = BezPath::new();
        kurbo::flatten(device.iter(), FLATTEN_TOLERANCE, |el| out.push(el));
        (out, rule)
    }

    /// Premultiplied paint.
    pub fn premultiplied(&self) -> [f32; 4] {
        let [r, g, b, a] = self.color;
        [r * a, g * a, b * a, a]
    }
}

/// `flat` (line segments only) filled with `rule`, as disjoint contours:
/// outer boundaries counterclockwise, holes clockwise, so the nonzero and
/// even-odd fills of the result agree and no pixel sums overlapping area.
fn resolve(flat: &BezPath, rule: FillRule) -> BezPath {
    use i_overlay::core::fill_rule::FillRule as Rule;
    use i_overlay::float::simplify::SimplifyShape;
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    for el in flat.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => contours.push(vec![[p.x, p.y]]),
            kurbo::PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push([p.x, p.y]);
                }
            }
            _ => {}
        }
    }
    contours.retain(|c| c.len() > 2 && c.iter().flatten().all(|v| v.is_finite()));
    let shapes = contours.simplify_shape(match rule {
        FillRule::NonZero => Rule::NonZero,
        FillRule::EvenOdd => Rule::EvenOdd,
    });
    let mut out = BezPath::new();
    for contour in shapes.iter().flatten() {
        let mut points = contour.iter();
        let Some(first) = points.next() else { continue };
        out.move_to((first[0], first[1]));
        for p in points {
            out.line_to((p[0], p[1]));
        }
        out.close_path();
    }
    out
}

/// An ordered list of items.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoverageScene {
    pub items: Vec<CoverageItem>,
}

impl CoverageScene {
    /// Device bounds of the whole scene, rounded out to pixels and clipped to
    /// `clip` (the target), or `None` when nothing can be drawn.
    pub fn pixel_bounds(&self, clip: PixelRect) -> Option<PixelRect> {
        self.items
            .iter()
            .filter_map(|i| i.device_bounds())
            .map(PixelRect::round_out)
            .reduce(PixelRect::union)
            .and_then(|r| r.intersect(clip))
    }

    /// Appends a shape's fill and stroke (`rasterShape` order) under
    /// `transform` (shape space to device pixels). `fill_color` and
    /// `stroke_color` are the content strings; `opacity` scales both alphas.
    pub fn push_shape(
        &mut self,
        geometry: &ShapeGeometry,
        transform: Affine,
        fill_color: &str,
        stroke_color: &str,
    ) {
        let mut fill = StyleSlot::default();
        fill.set(fill_color);
        // `c.strokeStyle = d.strokeColor || '#fff'`.
        let mut stroke = StyleSlot::default();
        stroke.set(if stroke_color.is_empty() { "#fff" } else { stroke_color });
        if geometry.fill {
            self.items.push(CoverageItem {
                path: geometry.path.clone(),
                transform,
                style: ItemStyle::Fill(FillRule::NonZero),
                color: rgba(fill.0),
            });
        }
        if let Some(s) = &geometry.stroke {
            let paint = match s.paint {
                StrokePaint::StrokeColor => stroke.0,
                StrokePaint::Color => fill.0,
            };
            self.items.push(CoverageItem {
                path: geometry.path.clone(),
                transform,
                style: ItemStyle::Stroke(s.style.clone()),
                color: rgba(paint),
            });
        }
    }

    /// Appends a layer's editable paths (`rasterPaths` order: per path and
    /// copy, fill then stroke) under `layer_to_device` (layer pixels to
    /// device pixels). `fills` and `strokes` are each path's color strings.
    pub fn push_paths(
        &mut self,
        layer: &PathLayer,
        layer_to_device: Affine,
        fills: &[String],
        strokes: &[String],
    ) {
        for draw in &layer.draws {
            let path = &layer.traced[draw.path];
            // Each copy runs between save() and restore(): the styles start
            // from the defaults and an invalid string keeps black.
            let mut fill = StyleSlot::default();
            fill.set(fills.get(draw.path).map_or("", String::as_str));
            let mut stroke = StyleSlot::default();
            stroke.set(strokes.get(draw.path).map_or("", String::as_str));
            let transform = layer_to_device * draw.transform;
            if let Some(alpha) = draw.fill {
                self.items.push(CoverageItem {
                    path: path.clone(),
                    transform,
                    style: ItemStyle::Fill(FillRule::NonZero),
                    color: rgba(fill.0.with_alpha_scale(alpha as f32)),
                });
            }
            if let Some(s) = &draw.stroke {
                self.items.push(CoverageItem {
                    path: path.clone(),
                    transform,
                    style: ItemStyle::Stroke(s.style.clone()),
                    color: rgba(stroke.0.with_alpha_scale(s.alpha as f32)),
                });
            }
        }
    }
}

fn rgba(c: CanvasColor) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// An integer pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The smallest pixel rectangle containing `r`.
    pub fn round_out(r: Rect) -> Self {
        let clamp = |v: f64| v.clamp(-(1 << 24) as f64, (1 << 24) as f64);
        let x0 = libm::floor(clamp(r.x0)) as i32;
        let y0 = libm::floor(clamp(r.y0)) as i32;
        let x1 = libm::ceil(clamp(r.x1)) as i32;
        let y1 = libm::ceil(clamp(r.y1)) as i32;
        Self::new(x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn union(self, o: Self) -> Self {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Self::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    pub fn intersect(self, o: Self) -> Option<Self> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        (x1 > x0 && y1 > y0).then(|| Self::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Where each item's coverage lives in an atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Index into the scene's items.
    pub item: usize,
    /// Atlas page.
    pub page: usize,
    /// Position in the page.
    pub x: u32,
    pub y: u32,
    /// The device pixels the cell covers (already clipped to the target).
    pub device: PixelRect,
}

/// Cells of every drawable item, packed into pages of at most
/// `max_side` × `max_side` pixels with a one-pixel gutter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasLayout {
    pub cells: Vec<Cell>,
    /// Page sizes.
    pub pages: Vec<(u32, u32)>,
}

/// Gutter between cells, so a stroke's coverage cannot reach a neighbor.
const GUTTER: u32 = 1;

impl AtlasLayout {
    /// Packs every item that intersects `clip` with shelf packing, tallest
    /// first. Items larger than a page are clipped to the page size around
    /// the visible region (they can only be that large when `clip` is).
    pub fn pack(scene: &CoverageScene, clip: PixelRect, max_side: u32) -> Self {
        let max_side = max_side.max(16);
        let mut wanted: Vec<(usize, PixelRect)> = scene
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let r = PixelRect::round_out(item.device_bounds()?).intersect(clip)?;
                Some((i, r))
            })
            .collect();
        // Cap each cell at the page size (the visible part of a huge item).
        for (_, r) in &mut wanted {
            r.width = r.width.min(max_side);
            r.height = r.height.min(max_side);
        }
        let mut order: Vec<usize> = (0..wanted.len()).collect();
        order.sort_by(|&a, &b| {
            wanted[b]
                .1
                .height
                .cmp(&wanted[a].1.height)
                .then(wanted[a].0.cmp(&wanted[b].0))
        });
        let mut cells = Vec::with_capacity(wanted.len());
        let mut pages: Vec<(u32, u32)> = Vec::new();
        // Shelf state of the current page.
        let (mut x, mut y, mut shelf, mut page_w) = (0u32, 0u32, 0u32, 0u32);
        for idx in order {
            let (item, device) = wanted[idx];
            let (w, h) = (device.width + GUTTER, device.height + GUTTER);
            if pages.is_empty() {
                pages.push((0, 0));
            }
            if x + w > max_side {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            if y + h > max_side {
                let last = pages.len() - 1;
                pages[last] = (page_w, y + shelf);
                pages.push((0, 0));
                (x, y, shelf, page_w) = (0, 0, 0, 0);
            }
            cells.push(Cell {
                item,
                page: pages.len() - 1,
                x,
                y,
                device,
            });
            x += w;
            shelf = shelf.max(h);
            page_w = page_w.max(x);
        }
        if let Some(last) = pages.last_mut() {
            *last = (page_w.max(1), (y + shelf).max(1));
        }
        cells.sort_by_key(|c| c.item);
        Self { cells, pages }
    }

    /// Transform from an item's device pixels to its cell in the page.
    pub fn cell_transform(cell: &Cell) -> Affine {
        Affine::translate((
            f64::from(cell.x) - f64::from(cell.device.x),
            f64::from(cell.y) - f64::from(cell.device.y),
        ))
    }

    /// Bytes of the `r32float` coverage pages.
    pub fn bytes(&self) -> u64 {
        self.pages
            .iter()
            .map(|&(w, h)| u64::from(w) * u64::from(h) * 4)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{ShapeContent, ShapeKind, shape_geometry};

    #[test]
    fn shape_items_follow_raster_shape() {
        let g = shape_geometry(&ShapeContent {
            shape: ShapeKind::Ellipse,
            w: 100.0,
            h: 50.0,
            stroke: 4.0,
            ..ShapeContent::default()
        })
        .unwrap();
        let mut scene = CoverageScene::default();
        scene.push_shape(&g, Affine::IDENTITY, "#ff0000", "");
        assert_eq!(scene.items.len(), 2);
        assert_eq!(scene.items[0].color, [1.0, 0.0, 0.0, 1.0]);
        // `d.strokeColor || '#fff'`.
        assert_eq!(scene.items[1].color, [1.0, 1.0, 1.0, 1.0]);
        let b = scene.items[1].device_bounds().unwrap();
        assert!(b.x0 <= -2.0 && b.x1 >= 102.0);
        // An invalid fill keeps Canvas2D's default black.
        let mut s2 = CoverageScene::default();
        s2.push_shape(&g, Affine::IDENTITY, "nope", "#00ff0080");
        assert_eq!(s2.items[0].color, [0.0, 0.0, 0.0, 1.0]);
        assert!((s2.items[1].color[3] - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn atlas_packs_without_overlap() {
        let mut scene = CoverageScene::default();
        for i in 0..40 {
            let r = Rect::new(0.0, 0.0, 30.0 + f64::from(i), 20.0 + f64::from(i % 7) * 9.0);
            scene.items.push(CoverageItem {
                path: r.to_path(0.1),
                transform: Affine::translate((f64::from(i) * 13.0, 5.0)),
                style: ItemStyle::Fill(FillRule::NonZero),
                color: [1.0; 4],
            });
        }
        let layout = AtlasLayout::pack(&scene, PixelRect::new(0, 0, 4000, 4000), 256);
        assert_eq!(layout.cells.len(), 40);
        for (i, a) in layout.cells.iter().enumerate() {
            let (pw, ph) = layout.pages[a.page];
            assert!(a.x + a.device.width <= pw && a.y + a.device.height <= ph);
            for b in &layout.cells[i + 1..] {
                if a.page != b.page {
                    continue;
                }
                let disjoint = a.x + a.device.width + GUTTER <= b.x
                    || b.x + b.device.width + GUTTER <= a.x
                    || a.y + a.device.height + GUTTER <= b.y
                    || b.y + b.device.height + GUTTER <= a.y;
                assert!(disjoint, "{a:?} {b:?}");
            }
        }
        assert!(layout.pages.len() > 1);
    }

    #[test]
    fn clipped_items_are_skipped() {
        let mut scene = CoverageScene::default();
        scene.items.push(CoverageItem {
            path: Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            transform: Affine::translate((-100.0, 0.0)),
            style: ItemStyle::Fill(FillRule::NonZero),
            color: [1.0; 4],
        });
        let layout = AtlasLayout::pack(&scene, PixelRect::new(0, 0, 50, 50), 1024);
        assert!(layout.cells.is_empty());
        assert_eq!(scene.pixel_bounds(PixelRect::new(0, 0, 50, 50)), None);
    }
}
