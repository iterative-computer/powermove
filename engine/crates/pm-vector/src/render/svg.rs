//! SVG images (plan section 10, `image` (SVG); docs/engine/vector.md,
//! "SVG images").
//!
//! Today an SVG asset is an `<img>` drawn into a canvas per size bucket
//! (`rasterSvgAsset`: `drawImage(img, 0, 0, width, height)`, the bucket
//! chosen by `svgRasterDimensions` from the layer's display transform).
//!
//! - [`svg_raster_dimensions`] is that bucket choice.
//! - [`SvgDocument::raster`] is the bucket raster, drawn by resvg: the
//!   SVG's user space stretched over the bucket as `drawImage` stretches the
//!   image. It is not Chromium's rasterization (Blink paints SVG shapes
//!   through Skia's analytic and multisampled renderers); the measured
//!   difference is in docs/engine/vector.md.
//! - [`SvgDocument::scene`] is the SVG as vectors for `compat` level 2:
//!   solid fills and strokes become coverage items rendered at output
//!   resolution; anything else (gradients, patterns, clips, masks, filters,
//!   group opacity, blend modes, embedded images) is drawn by resvg at
//!   output resolution ([`SvgDocument::raster_device`]).
//!
//! usvg parses without its `text` feature: SVG text is not converted here.
//! [`SvgDocument::has_text`] reports it so the renderer routes such SVGs
//! until text goes through `pm-text`.
//!
//! As in an `<img>`, an SVG image loads no external resources: only
//! `data:` URLs resolve.

use kurbo::{Affine, BezPath, Cap, Join, Stroke};

use crate::canvas2d::shape::Bitmap;
use crate::render::scene::{CoverageItem, CoverageScene, FillRule, ItemStyle, PixelRect};

/// Why an SVG could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgError(pub String);

impl core::fmt::Display for SvgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid SVG: {}", self.0)
    }
}

impl std::error::Error for SvgError {}

/// A parsed SVG.
pub struct SvgDocument {
    tree: usvg::Tree,
    has_text: bool,
}

impl core::fmt::Debug for SvgDocument {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (w, h) = self.size();
        f.debug_struct("SvgDocument").field("size", &(w, h)).field("has_text", &self.has_text).finish()
    }
}

fn options() -> usvg::Options<'static> {
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

impl SvgDocument {
    /// Parses SVG bytes (SVGZ is not read: usvg's compression feature is
    /// off).
    pub fn parse(data: &[u8]) -> Result<Self, SvgError> {
        let tree = usvg::Tree::from_data(data, &options()).map_err(|e| SvgError(e.to_string()))?;
        let has_text = contains_text(data);
        Ok(Self { tree, has_text })
    }

    /// The SVG's size in user units (`width`/`height`, else the viewBox).
    pub fn size(&self) -> (f64, f64) {
        let s = self.tree.size();
        (f64::from(s.width()), f64::from(s.height()))
    }

    /// Whether the SVG has `<text>` (not drawn: usvg's text feature is off).
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// `drawImage(img, 0, 0, width, height)` into a cleared canvas: the
    /// SVG's user space stretched over `width` × `height`, premultiplied
    /// RGBA8, top-down.
    pub fn raster(&self, width: u32, height: u32) -> Bitmap {
        let (sw, sh) = self.size();
        let t = Affine::scale_non_uniform(f64::from(width) / sw, f64::from(height) / sh);
        self.raster_device(t, PixelRect::new(0, 0, width, height))
    }

    /// The SVG under `transform` (user units to device pixels), drawn by
    /// resvg into the device rectangle `rect`: premultiplied RGBA8,
    /// top-down, `rect.width` × `rect.height`.
    pub fn raster_device(&self, transform: Affine, rect: PixelRect) -> Bitmap {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else {
            return Bitmap { width: w, height: h, rgba: vec![0; (w * h * 4) as usize] };
        };
        let t = Affine::translate((-f64::from(rect.x), -f64::from(rect.y))) * transform;
        let c = t.as_coeffs().map(|v| v as f32);
        resvg::render(&self.tree, tiny_skia::Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]), &mut pixmap.as_mut());
        Bitmap { width: w, height: h, rgba: pixmap.take() }
    }

    /// The SVG as coverage items under `transform` (user units to device
    /// pixels), or `None` when it draws anything but solid fills and strokes
    /// in plain groups.
    pub fn scene(&self, transform: Affine) -> Option<CoverageScene> {
        let mut scene = CoverageScene::default();
        push_group(self.tree.root(), transform, &mut scene)?;
        Some(scene)
    }

    /// Device bounds of everything the SVG draws under `transform`, or
    /// `None` when it draws nothing. Filters can draw outside their
    /// element's bounds, so they extend to the filter regions.
    pub fn device_bounds(&self, transform: Affine) -> Option<kurbo::Rect> {
        let root = self.tree.root();
        if !root.has_children() {
            return None;
        }
        let b = root.abs_layer_bounding_box();
        let r = kurbo::Rect::new(f64::from(b.left()), f64::from(b.top()), f64::from(b.right()), f64::from(b.bottom()));
        Some(transform.transform_rect_bbox(r))
    }
}

/// Whether the document has a `<text>` element.
fn contains_text(data: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(data) else {
        return false;
    };
    let Ok(doc) = usvg::roxmltree::Document::parse_with_options(
        s,
        usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    ) else {
        return false;
    };
    doc.descendants().any(|n| n.is_element() && n.tag_name().name() == "text")
}

fn affine(t: tiny_skia::Transform) -> Affine {
    Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)])
}

fn bez(path: &tiny_skia::Path) -> BezPath {
    use tiny_skia::PathSegment;
    let p = |q: tiny_skia::Point| kurbo::Point::new(f64::from(q.x), f64::from(q.y));
    let mut out = BezPath::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(a) => out.move_to(p(a)),
            PathSegment::LineTo(a) => out.line_to(p(a)),
            PathSegment::QuadTo(a, b) => out.quad_to(p(a), p(b)),
            PathSegment::CubicTo(a, b, c) => out.curve_to(p(a), p(b), p(c)),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

fn solid(paint: &usvg::Paint, opacity: f32) -> Option<[f32; 4]> {
    match paint {
        usvg::Paint::Color(c) => Some([
            f32::from(c.red) / 255.0,
            f32::from(c.green) / 255.0,
            f32::from(c.blue) / 255.0,
            opacity,
        ]),
        _ => None,
    }
}

fn push_group(group: &usvg::Group, transform: Affine, scene: &mut CoverageScene) -> Option<()> {
    if group.opacity().get() < 1.0
        || group.clip_path().is_some()
        || group.mask().is_some()
        || !group.filters().is_empty()
        || group.blend_mode() != usvg::BlendMode::Normal
    {
        return None;
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => push_group(g, transform, scene)?,
            usvg::Node::Path(p) => push_path(p, transform, scene)?,
            // Embedded images and (text off) text are not vectors here.
            usvg::Node::Image(_) | usvg::Node::Text(_) => return None,
        }
    }
    Some(())
}

fn push_path(p: &usvg::Path, transform: Affine, scene: &mut CoverageScene) -> Option<()> {
    if !p.is_visible() {
        return Some(());
    }
    // `shape-rendering: crispEdges` turns antialiasing off: resvg draws it.
    if p.rendering_mode() == usvg::ShapeRendering::CrispEdges {
        return None;
    }
    let path = bez(p.data());
    let t = transform * affine(p.abs_transform());
    let fill = match p.fill() {
        Some(f) => Some(CoverageItem {
            path: path.clone(),
            transform: t,
            style: ItemStyle::Fill(match f.rule() {
                usvg::FillRule::NonZero => FillRule::NonZero,
                usvg::FillRule::EvenOdd => FillRule::EvenOdd,
            }),
            color: solid(f.paint(), f.opacity().get())?,
        }),
        None => None,
    };
    let stroke = match p.stroke() {
        Some(s) => {
            let join = match s.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter,
                usvg::LineJoin::Round => Join::Round,
                usvg::LineJoin::Bevel => Join::Bevel,
            };
            let cap = match s.linecap() {
                usvg::LineCap::Butt => Cap::Butt,
                usvg::LineCap::Round => Cap::Round,
                usvg::LineCap::Square => Cap::Square,
            };
            let mut style = Stroke::new(f64::from(s.width().get()))
                .with_join(join)
                .with_caps(cap)
                .with_miter_limit(f64::from(s.miterlimit().get()));
            if let Some(dashes) = s.dasharray() {
                style = style.with_dashes(f64::from(s.dashoffset()), dashes.iter().map(|d| f64::from(*d)));
            }
            Some(CoverageItem {
                path: path.clone(),
                transform: t,
                style: ItemStyle::Stroke(style),
                color: solid(s.paint(), s.opacity().get())?,
            })
        }
        None => None,
    };
    let (first, second) = match p.paint_order() {
        usvg::PaintOrder::FillAndStroke => (fill, stroke),
        usvg::PaintOrder::StrokeAndFill => (stroke, fill),
    };
    scene.items.extend(first);
    scene.items.extend(second);
    Some(())
}

/// `svgRasterDimensions(sourceWidth, sourceHeight, boxWidth, boxHeight,
/// matrix, maxDimension = 8192)` (legacy/gl/compositor.ts): the bucket a
/// layer's SVG is rasterized at, keeping the source aspect ratio. Returns
/// `(width, height, scale)`.
pub fn svg_raster_dimensions(
    source_width: f64,
    source_height: f64,
    box_width: f64,
    box_height: f64,
    matrix: &[f64; 6],
    max_dimension: f64,
) -> (u32, u32, f64) {
    use crate::js;
    let num = |v: f64, d: f64| if v.is_nan() || v == 0.0 { d } else { v };
    let sw = js::max(1.0, num(source_width, 1.0));
    let sh = js::max(1.0, num(source_height, 1.0));
    let bw = js::max(1.0, num(box_width, sw));
    let bh = js::max(1.0, num(box_height, sh));
    let limit = js::max(1.0, num(max_dimension, 8192.0));
    let box_density = js::max(bw / sw, bh / sh);
    let max_scale = js::max(0.25, limit / js::max(sw, sh));
    let requested = continuous_raster_scale(matrix, box_density, max_scale);
    let mut width = js::max(1.0, libm::ceil(sw * requested));
    let mut height = js::max(1.0, libm::ceil(sh * requested));
    if js::max(width, height) > limit {
        let ratio = limit / js::max(width, height);
        width = js::max(1.0, libm::floor(width * ratio));
        height = js::max(1.0, libm::floor(height * ratio));
    }
    (width as u32, height as u32, width / sw)
}

/// `continuousRasterScale(matrix, density, maxScale)`.
fn continuous_raster_scale(m: &[f64; 6], density: f64, max_scale: f64) -> f64 {
    use crate::js;
    let n = |v: f64| if v.is_nan() { 0.0 } else { v };
    let (m0, m1, m2, m3) = (n(m[0]), n(m[1]), n(m[2]), n(m[3]));
    let aa = m0 * m0 + m1 * m1;
    let bb = m0 * m2 + m1 * m3;
    let cc = m2 * m2 + m3 * m3;
    let discriminant = js::max(0.0, (aa - cc) * (aa - cc) + 4.0 * bb * bb).sqrt();
    let largest = js::max(0.0, (aa + cc + discriminant) / 2.0).sqrt();
    let density = if density.is_nan() || density == 0.0 { 1.0 } else { density };
    let requested = largest * js::max(0.01, density);
    let max_scale = if max_scale.is_nan() || max_scale == 0.0 { 32.0 } else { max_scale };
    let bounded = js::max(0.25, js::min(js::max(0.25, max_scale), requested));
    libm::ceil(bounded * 4.0 - 1e-9) / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// compositor.test.ts's expectations.
    #[test]
    fn raster_dimensions_follow_the_compositor() {
        assert_eq!(svg_raster_dimensions(100.0, 50.0, 400.0, 200.0, &[2.0, 0.0, 0.0, 2.0, 0.0, 0.0], 8192.0), (800, 400, 8.0));
        assert_eq!(svg_raster_dimensions(100.0, 50.0, 400.0, 400.0, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 8192.0), (800, 400, 8.0));
        assert_eq!(
            svg_raster_dimensions(100.0, 50.0, 20_000.0, 10_000.0, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 8192.0),
            (8192, 4096, 81.92)
        );
    }

    #[test]
    fn solid_svgs_become_vectors() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">
            <rect x="10" y="10" width="30" height="20" fill="#ff0000" fill-opacity="0.5" stroke="#00ff00" stroke-width="2"/>
            <g transform="translate(50 0)"><circle cx="20" cy="25" r="10" fill="#0000ff"/></g>
        </svg>"##;
        let doc = SvgDocument::parse(svg).unwrap();
        assert_eq!(doc.size(), (100.0, 50.0));
        assert!(!doc.has_text());
        let scene = doc.scene(Affine::scale(2.0)).unwrap();
        assert_eq!(scene.items.len(), 3);
        assert_eq!(scene.items[0].color, [1.0, 0.0, 0.0, 0.5]);
        assert!(matches!(scene.items[1].style, ItemStyle::Stroke(_)));
        let b = scene.items[2].device_bounds().unwrap();
        assert!((b.x0 - 120.0).abs() < 1e-3 && (b.x1 - 160.0).abs() < 1e-3, "{b:?}");
    }

    #[test]
    fn gradients_text_and_external_images_are_reported() {
        let gradient = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient></defs><rect width="10" height="10" fill="url(#g)"/></svg>"##;
        assert!(SvgDocument::parse(gradient).unwrap().scene(Affine::IDENTITY).is_none());
        let text = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text x="1" y="8">a</text></svg>"##;
        assert!(SvgDocument::parse(text).unwrap().has_text());
        // An external file reference resolves to nothing (as in an <img>).
        let external = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10"><image width="10" height="10" xlink:href="/etc/hosts"/></svg>"##;
        let doc = SvgDocument::parse(external).unwrap();
        assert!(doc.raster(10, 10).rgba.iter().all(|&v| v == 0));
    }

    #[test]
    fn rasters_stretch_like_draw_image() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect x="0" y="0" width="5" height="10" fill="#ffffff"/></svg>"##;
        let b = SvgDocument::parse(svg).unwrap().raster(40, 10);
        assert_eq!((b.width, b.height), (40, 10));
        // The left half is covered: the 10 × 10 user space spans 40 × 10.
        assert_eq!(b.rgba[(5 * 40 + 10) * 4 + 3], 255);
        assert_eq!(b.rgba[(5 * 40 + 30) * 4 + 3], 0);
    }
}
