//! SVG images (plan section 10, `image` (SVG); docs/engine/vector.md,
//! "SVG images").
//!
//! An SVG image is parsed once by usvg (with `<text>` laid out against the
//! system fonts, loaded once per process and only for documents with text)
//! and drawn through the native float painter when its features and GPU
//! limits allow:
//!
//! - [`SvgDocument::gpu_paintable`] documents (paths with solid or gradient
//!   paint, groups with opacity and blend modes, clip paths, masks, text,
//!   Gaussian blur and offset filters) are
//!   extracted as ordered float paint operations ([`SvgDocument::float_scene`]);
//! - unsupported filters, patterns, embedded images and `crispEdges` use resvg
//!   ([`SvgDocument::raster_device`]).
//!
//! As in an `<img>`, an SVG image loads no external resources: only
//! `data:` URLs resolve.

use kurbo::{Affine, BezPath};
use std::collections::HashMap;

use super::svg_depth::{MAX_ELEMENT_DEPTH, element_depth, with_stack};
use crate::render::scene::PixelRect;

#[path = "svg_float.rs"]
mod svg_float;
pub use svg_float::{
    FloatDraw, FloatFilter, FloatGradient, FloatGroup, FloatNode, FloatPaint, FloatStop,
};
pub use usvg::NonZeroRect as SvgRect;
pub use usvg::filter as usvg_filter;
pub use usvg::{BlendMode, SpreadMethod};

/// Why an SVG could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgError(pub String);

impl core::fmt::Display for SvgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid SVG: {}", self.0)
    }
}

impl std::error::Error for SvgError {}

/// Premultiplied RGBA8 pixels, top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A parsed SVG.
pub struct SvgDocument {
    /// Dropped on a stack sized to `depth` (see `Drop`).
    pub(crate) tree: core::mem::ManuallyDrop<usvg::Tree>,
    /// The element nesting ([`super::svg_depth::element_depth`]): recursive
    /// work over the tree runs on a stack sized to it.
    pub(crate) depth: usize,
    gpu: bool,
    has_text: bool,
    has_mask_or_filter: bool,
    pub(crate) filter_res: HashMap<String, Option<[u32; 2]>>,
}

impl Drop for SvgDocument {
    fn drop(&mut self) {
        // SAFETY: the tree is taken once, here, and never used again.
        let tree = unsafe { core::mem::ManuallyDrop::take(&mut self.tree) };
        // usvg's groups drop recursively.
        with_stack(self.depth, move || drop(tree));
    }
}

impl core::fmt::Debug for SvgDocument {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SvgDocument")
            .field("size", &self.size())
            .field("gpu", &self.gpu)
            .finish()
    }
}

/// The system font database, loaded on first use (the first document with
/// `<text>`), once per process, with Chrome's macOS generic families (CSS
/// Fonts 4 §2 leaves them to the user agent). Text is laid out by usvg
/// (fontdb matching, rustybuzz shaping) and drawn as its outlines.
fn fonts() -> std::sync::Arc<usvg::fontdb::Database> {
    static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.set_serif_family("Times");
            db.set_sans_serif_family("Helvetica");
            db.set_monospace_family("Courier");
            db.set_cursive_family("Apple Chancery");
            db.set_fantasy_family("Papyrus");
            std::sync::Arc::new(db)
        })
        .clone()
}

fn options(text: bool) -> usvg::Options<'static> {
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    // The initial CSS font size affects `em`/`ex` lengths even in a document
    // without a text element. Loading font faces remains lazy.
    options.font_family = "Times".into();
    options.font_size = 16.0;
    if text {
        options.fontdb = fonts();
    }
    options
}

impl SvgDocument {
    /// Parses SVG bytes (UTF-8; SVGZ is not read), reading the XML as
    /// Chromium's parser does where roxmltree is stricter
    /// ([`super::svg_xml`]). The nesting is scanned first without
    /// recursion: deeper than [`MAX_ELEMENT_DEPTH`] is an error (Chromium
    /// shows a broken image), and a deep document parses on a stack sized
    /// to its nesting.
    pub fn parse(data: &[u8]) -> Result<Self, SvgError> {
        let depth = element_depth(data);
        if depth > MAX_ELEMENT_DEPTH {
            return Err(SvgError(format!(
                "elements nest {depth} deep; Chromium's XML parser stops at {MAX_ELEMENT_DEPTH}"
            )));
        }
        if data.starts_with(&[0x1f, 0x8b]) {
            return Err(SvgError("SVGZ (gzip) is not read".into()));
        }
        let text = std::str::from_utf8(data).map_err(|_| SvgError("not UTF-8".into()))?;
        let (tree, gpu, has_text, has_mask_or_filter, filter_res) = with_stack(depth, || {
            let xml =
                |e: usvg::roxmltree::Error| SvgError(format!("SVG data parsing failed cause {e}"));
            let text = super::svg_xml::libxml2_text(text).map_err(xml)?;
            let doc = super::svg_xml::parse(&text).map_err(xml)?;
            let filter_res = filter_resolutions(&doc)?;
            let unsupported_edge_mode = doc.descendants().any(|n| {
                n.is_element()
                    && n.tag_name().name() == "feGaussianBlur"
                    && n.attribute("edgeMode").is_some_and(|v| v != "none")
            });
            let text_nodes = doc
                .descendants()
                .any(|n| n.is_element() && n.tag_name().name() == "text");
            let has_mask_or_filter = doc.descendants().any(|n| {
                n.is_element() && matches!(n.tag_name().name(), "mask" | "filter")
            });
            let tree = usvg::Tree::from_xmltree(&doc, &options(text_nodes))
                .map_err(|e| SvgError(e.to_string()))?;
            let gpu = paintable_group(tree.root()) && !unsupported_edge_mode;
            Ok::<_, SvgError>((tree, gpu, text_nodes, has_mask_or_filter, filter_res))
        })?;
        Ok(Self {
            tree: core::mem::ManuallyDrop::new(tree),
            depth,
            gpu,
            has_text,
            has_mask_or_filter,
            filter_res,
        })
    }

    /// The element nesting the document was parsed with.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The SVG's size in user units (`width`/`height`, else the viewBox).
    pub fn size(&self) -> (f64, f64) {
        let s = self.tree.size();
        (f64::from(s.width()), f64::from(s.height()))
    }

    /// Raw glyph outlines after usvg shapes text, before the renderer's
    /// flattening and disjoint-contour resolution. This gives the SVG gate
    /// independent geometry for exact f64 coverage of the authored text.
    pub fn raw_text_fill_paths(&self) -> Vec<(kurbo::BezPath, bool)> {
        fn visit(
            group: &usvg::Group,
            parent: kurbo::Affine,
            in_text: bool,
            out: &mut Vec<(kurbo::BezPath, bool)>,
        ) {
            let t = parent * affine(group.transform());
            for node in group.children() {
                match node {
                    usvg::Node::Group(g) => visit(g, t, in_text, out),
                    usvg::Node::Text(text) => visit(text.flattened(), t, true, out),
                    usvg::Node::Path(path) if in_text => {
                        if let Some(fill) = path.fill() {
                            out.push((
                                t * bez(path.data()),
                                fill.rule() == usvg::FillRule::EvenOdd,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        with_stack(self.depth, || {
            let mut paths = Vec::new();
            visit(self.tree.root(), kurbo::Affine::IDENTITY, false, &mut paths);
            paths
        })
    }

    /// Whether the float painter can draw every part of the document. Gaussian
    /// blur and offset filters are supported; patterns, embedded images,
    /// unsupported filters and `crispEdges` shapes are not.
    pub fn gpu_paintable(&self) -> bool {
        self.gpu
    }

    /// Whether the parsed SVG has text glyphs. Small glyph edges need a
    /// denser canonical source when a float raster is reused under motion.
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// Whether the SVG declares a mask or filter, whose high-density source
    /// can require joint fine-grid work during a cache fill.
    pub fn has_mask_or_filter(&self) -> bool {
        self.has_mask_or_filter
    }

    /// The SVG's user space stretched over `width` × `height` (an `<img>`
    /// drawn at that size), by resvg.
    pub fn raster(&self, width: u32, height: u32) -> SvgRaster {
        let (sw, sh) = self.size();
        let t = Affine::scale_non_uniform(f64::from(width) / sw, f64::from(height) / sh);
        self.raster_device(t, PixelRect::new(0, 0, width, height), None)
    }

    /// The SVG under `transform` (user units to device pixels), drawn by
    /// resvg into the device rectangle `rect`. `clip` (a closed polygon in
    /// device pixels) limits it, antialiased.
    pub fn raster_device(
        &self,
        transform: Affine,
        rect: PixelRect,
        clip: Option<&[kurbo::Point]>,
    ) -> SvgRaster {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let empty = || SvgRaster {
            width: w,
            height: h,
            rgba: vec![0; (w * h * 4) as usize],
        };
        let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else {
            return empty();
        };
        let to_rect = Affine::translate((-f64::from(rect.x), -f64::from(rect.y)));
        let c = (to_rect * transform).as_coeffs().map(|v| v as f32);
        let tree: &usvg::Tree = &self.tree;
        let target = &mut pixmap;
        // resvg renders groups recursively.
        with_stack(self.depth, move || {
            resvg::render(
                tree,
                tiny_skia::Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]),
                &mut target.as_mut(),
            )
        });
        if let Some(points) = clip {
            let mut pb = tiny_skia::PathBuilder::new();
            for (i, p) in points.iter().enumerate() {
                let q = to_rect * *p;
                if i == 0 {
                    pb.move_to(q.x as f32, q.y as f32);
                } else {
                    pb.line_to(q.x as f32, q.y as f32);
                }
            }
            pb.close();
            let (Some(path), Some(mut mask)) = (pb.finish(), tiny_skia::Mask::new(w, h)) else {
                return empty();
            };
            mask.fill_path(
                &path,
                tiny_skia::FillRule::Winding,
                true,
                tiny_skia::Transform::identity(),
            );
            pixmap.apply_mask(&mask);
        }
        SvgRaster {
            width: w,
            height: h,
            rgba: pixmap.take(),
        }
    }

    /// User-space bounds of everything the SVG draws (strokes and filter
    /// regions included), or `None` when it draws nothing.
    pub fn user_bounds(&self) -> Option<kurbo::Rect> {
        let root = self.tree.root();
        if !root.has_children() {
            return None;
        }
        // Bounds are computed at parse time; reading them does not recurse.
        let b = root.abs_layer_bounding_box();
        Some(kurbo::Rect::new(
            f64::from(b.left()),
            f64::from(b.top()),
            f64::from(b.right()),
            f64::from(b.bottom()),
        ))
    }
}

fn paintable_group(g: &usvg::Group) -> bool {
    g.filters().iter().all(|f| {
        f.primitives().iter().all(|p| {
            matches!(
                p.kind(),
                usvg::filter::Kind::GaussianBlur(_) | usvg::filter::Kind::Offset(_)
            )
        })
    }) && g.clip_path().is_none_or(paintable_clip)
        && g.mask().is_none_or(paintable_mask)
        && g.children().iter().all(|node| match node {
            usvg::Node::Group(g) => paintable_group(g),
            usvg::Node::Path(p) => {
                p.rendering_mode().use_shape_antialiasing()
                    && p.fill()
                        .is_none_or(|f| !matches!(f.paint(), usvg::Paint::Pattern(_)))
                    && p.stroke()
                        .is_none_or(|s| !matches!(s.paint(), usvg::Paint::Pattern(_)))
            }
            usvg::Node::Image(_) => false,
            usvg::Node::Text(t) => paintable_group(t.flattened()),
        })
}

/// `usvg` discards SVG 1.1 filterRes. Retain it after XML normalization,
/// including href inheritance, so the renderer can use the authored grid.
fn filter_resolutions(
    doc: &usvg::roxmltree::Document<'_>,
) -> Result<HashMap<String, Option<[u32; 2]>>, SvgError> {
    let filters: HashMap<_, _> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "filter")
        .filter_map(|n| n.attribute("id").map(|id| (id.to_owned(), n)))
        .collect();
    let mut result = HashMap::new();
    for (id, node) in &filters {
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(*node);
        while let Some(n) = current {
            let Some(key) = n.attribute("id") else { break };
            if !seen.insert(key) {
                break;
            }
            if let Some(raw) = n.attribute("filterRes") {
                let values: Vec<_> = raw
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(str::parse::<f64>)
                    .collect();
                if values
                    .iter()
                    .any(|value| value.as_ref().is_ok_and(|v| *v < 0.0))
                {
                    return Err(SvgError(format!("negative SVG filterRes on filter {id}")));
                }
                let parsed = match values.as_slice() {
                    [Ok(x)] => Some((*x, *x)),
                    [Ok(x), Ok(y)] => Some((*x, *y)),
                    _ => None,
                }
                .and_then(|(x, y)| {
                    (x.is_finite()
                        && y.is_finite()
                        && x > 0.0
                        && y > 0.0
                        && x < u32::MAX as f64
                        && y < u32::MAX as f64)
                        .then_some([x as u32, y as u32])
                });
                result.insert(id.clone(), parsed.filter(|v| v[0] > 0 && v[1] > 0));
                break;
            }
            current = n
                .attributes()
                .find(|a| a.name() == "href")
                .and_then(|a| a.value().strip_prefix('#'))
                .and_then(|target| filters.get(target).copied());
        }
    }
    Ok(result)
}

fn paintable_clip(c: &usvg::ClipPath) -> bool {
    paintable_group(c.root()) && c.clip_path().is_none_or(paintable_clip)
}

fn paintable_mask(m: &usvg::Mask) -> bool {
    paintable_group(m.root()) && m.mask().is_none_or(paintable_mask)
}

pub(crate) fn affine(t: tiny_skia::Transform) -> Affine {
    Affine::new([
        f64::from(t.sx),
        f64::from(t.ky),
        f64::from(t.kx),
        f64::from(t.sy),
        f64::from(t.tx),
        f64::from(t.ty),
    ])
}

pub(crate) fn bez(path: &tiny_skia::Path) -> BezPath {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_float_can_paint() {
        let gpu = |svg: &str| SvgDocument::parse(svg.as_bytes()).unwrap().gpu_paintable();
        assert!(gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient><clipPath id="c"><circle cx="5" cy="5" r="4"/></clipPath><mask id="m"><rect width="10" height="5" fill="#fff"/></mask></defs><g opacity="0.5" clip-path="url(#c)" mask="url(#m)" style="mix-blend-mode:multiply"><rect width="10" height="10" fill="url(#g)" stroke="#f00" stroke-dasharray="1 1"/></g></svg>"##
        ));
        assert!(gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><filter id="f"><feGaussianBlur stdDeviation="1"/></filter><rect width="10" height="10" filter="url(#f)"/></svg>"##
        ));
        assert!(!gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><filter id="f"><feGaussianBlur stdDeviation="1" edgeMode="wrap"/></filter><rect width="10" height="10" filter="url(#f)"/></svg>"##
        ));
        assert!(!gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><filter id="f"><feColorMatrix type="saturate" values="0.5"/></filter><rect width="10" height="10" filter="url(#f)"/></svg>"##
        ));
        assert!(!gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><pattern id="p" width="2" height="2" patternUnits="userSpaceOnUse"><rect width="1" height="1"/></pattern><rect width="10" height="10" fill="url(#p)"/></svg>"##
        ));
        assert!(!gpu(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" shape-rendering="crispEdges"/></svg>"##
        ));
    }

    #[test]
    fn external_images_resolve_to_nothing() {
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

    #[test]
    fn initial_font_size_applies_to_geometry_without_text() {
        let lengths = SvgDocument::parse(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1ex"><rect width="16" height="8"/></svg>"#,
        ).unwrap();
        assert_eq!(lengths.size(), (16.0, 8.0));
    }

    fn nested(levels: usize) -> String {
        // `levels` elements in all: the root and levels − 2 groups around a rect.
        let groups = levels - 2;
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">{}<rect width="10" height="10" fill="#4C8DFF"/>{}</svg>"##,
            "<g>".repeat(groups),
            "</g>".repeat(groups)
        )
    }

    /// Engine worker threads have 2 MiB. A document 1,000 levels deep
    /// parses, draws, and drops there; one at Chromium's limit (its plain
    /// groups read away past usvg's 1,024 levels) draws as Chromium does,
    /// and one past it is refused before anything recursive runs.
    #[test]
    fn deep_documents_are_safe_on_a_small_stack() {
        let deep = nested(1000);
        assert_eq!(element_depth(deep.as_bytes()), 1000);
        let inline = nested(super::super::svg_depth::INLINE_DEPTH);
        let limit = nested(MAX_ELEMENT_DEPTH);
        let too_deep = nested(MAX_ELEMENT_DEPTH + 1);
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let doc = SvgDocument::parse(deep.as_bytes()).unwrap();
                assert_eq!(doc.depth(), 1000);
                assert!(doc.gpu_paintable());
                let b = doc.raster(8, 8);
                assert_eq!(&b.rgba[..4], &[0x4C, 0x8D, 0xFF, 255]);
                assert!(doc.user_bounds().is_some());
                assert!(doc.float_scene(Affine::IDENTITY, &[]).is_some());
                drop(doc);
                // Shallow enough to run on this thread itself.
                let doc = SvgDocument::parse(inline.as_bytes()).unwrap();
                assert_eq!(doc.raster(4, 4).rgba[3], 255);
                drop(doc);
                let doc = SvgDocument::parse(limit.as_bytes()).unwrap();
                assert_eq!(&doc.raster(4, 4).rgba[..4], &[0x4C, 0x8D, 0xFF, 255]);
                drop(doc);
                let e = SvgDocument::parse(too_deep.as_bytes()).unwrap_err();
                assert!(e.0.contains("nest 5001 deep"), "{e}");
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
