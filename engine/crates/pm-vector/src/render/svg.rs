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
//!   output resolution ([`SvgDocument::raster_device`]), clipped as the
//!   image's box and viewport clip it.
//!
//! The production renderer still gates SVG text. The diagnostic
//! [`SvgDocument::raster_compat_with_text`] shapes and paints resolved text
//! spans through the caller's pm-text resolver, never usvg glyph outlines.
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

/// A cached gate verdict: the source size (bits) and the result.
type CompatVerdict = ((u64, u64), Result<(), SvgError>);

/// A parsed SVG.
pub struct SvgDocument {
    tree: usvg::Tree,
    has_text: bool,
    data: Vec<u8>,
    /// [`SvgDocument::compat_supported`] verdicts by source size (the
    /// bucket ladder depends only on it).
    compat: std::sync::Mutex<Vec<CompatVerdict>>,
}

impl core::fmt::Debug for SvgDocument {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (w, h) = self.size();
        f.debug_struct("SvgDocument")
            .field("size", &(w, h))
            .field("has_text", &self.has_text)
            .finish()
    }
}

fn options(text: bool) -> usvg::Options<'static> {
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    if text {
        static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> = std::sync::OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut db = usvg::fontdb::Database::new();
                db.load_system_fonts();
                std::sync::Arc::new(db)
            })
            .clone();
    }
    options
}

/// The deepest element nesting Blink's XML parser accepts
/// (`kMaxXMLTreeDepth`); a deeper document is a broken image in Chromium.
pub const MAX_ELEMENT_DEPTH: usize = 5000;

/// The deepest element nesting in `data`, counted without recursion over
/// comments, CDATA, processing instructions, declarations and quoted
/// attribute values (entity expansions are not counted).
pub fn element_depth(data: &[u8]) -> usize {
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    let skip_past = |i: &mut usize, end: &[u8]| {
        match data[*i..].windows(end.len()).position(|w| w == end) {
            Some(at) => *i += at + end.len(),
            None => *i = data.len(),
        }
    };
    while i < data.len() {
        let Some(lt) = data[i..].iter().position(|&b| b == b'<') else {
            break;
        };
        i += lt;
        let rest = &data[i..];
        if rest.starts_with(b"<!--") {
            i += 4;
            skip_past(&mut i, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            skip_past(&mut i, b"]]>");
        } else if rest.starts_with(b"<?") {
            skip_past(&mut i, b"?>");
        } else if rest.starts_with(b"</") {
            i += 2;
            skip_past(&mut i, b">");
            depth = depth.saturating_sub(1);
        } else if rest.starts_with(b"<!") {
            // DOCTYPE: skip to its end over an internal subset.
            i += 2;
            let mut bracket = 0usize;
            while i < data.len() {
                match data[i] {
                    b'[' => bracket += 1,
                    b']' => bracket = bracket.saturating_sub(1),
                    b'"' | b'\'' => {
                        let q = data[i];
                        i += 1;
                        skip_past(&mut i, &[q]);
                        continue;
                    }
                    b'>' if bracket == 0 => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
        } else {
            i += 1;
            let mut empty = false;
            while i < data.len() {
                match data[i] {
                    b'>' => {
                        i += 1;
                        break;
                    }
                    b'/' if data.get(i + 1) == Some(&b'>') => {
                        empty = true;
                        i += 2;
                        break;
                    }
                    b'"' | b'\'' => {
                        let q = data[i];
                        i += 1;
                        skip_past(&mut i, &[q]);
                    }
                    _ => i += 1,
                }
            }
            max = max.max(depth + 1);
            if !empty {
                depth += 1;
            }
        }
    }
    max
}

/// Runs `f` on a thread whose stack fits a document nested `depth` levels.
/// roxmltree, usvg's converter and the paint-record walk each recurse once
/// per element level; engine worker threads have 2 MiB, so a deep document
/// gets its own stack instead of overflowing the caller's. Documents deeper
/// than [`MAX_ELEMENT_DEPTH`] are rejected before any recursive parse.
fn with_document_stack<T: Send>(data: &[u8], f: impl FnOnce() -> Result<T, SvgError> + Send) -> Result<T, SvgError> {
    with_document_stack_depth(element_depth(data), f)
}

/// Documents nested at most this deep parse on the caller's stack (2 MiB
/// engine workers; `shallow_documents_parse_on_a_small_stack` checks it).
const INLINE_DEPTH: usize = 64;

fn with_document_stack_depth<T: Send>(depth: usize, f: impl FnOnce() -> Result<T, SvgError> + Send) -> Result<T, SvgError> {
    if depth > MAX_ELEMENT_DEPTH {
        return Err(SvgError(format!(
            "elements nest {depth} deep; Chromium's XML parser stops at {MAX_ELEMENT_DEPTH}"
        )));
    }
    if depth <= INLINE_DEPTH {
        return f();
    }
    // 16 MiB plus 64 KiB per level (at most ~330 MiB of reserved, lazily
    // committed address space).
    let stack = (16 << 20) + depth * (64 << 10);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("svg-parse".into())
            .stack_size(stack)
            .spawn_scoped(s, f)
            .map_err(|e| SvgError(format!("could not start the parse thread: {e}")))?
            .join()
            .map_err(|_| SvgError("the parse thread panicked".into()))?
    })
}

impl SvgDocument {
    /// Parses SVG bytes (SVGZ is not read: usvg's compression feature is
    /// off). Deeply nested documents parse on a dedicated stack; deeper than
    /// [`MAX_ELEMENT_DEPTH`] is an error, as Chromium shows a broken image.
    pub fn parse(data: &[u8]) -> Result<Self, SvgError> {
        let (tree, has_text) = with_document_stack(data, || {
            let has_text = contains_text(data);
            let tree = usvg::Tree::from_data(data, &options(has_text)).map_err(|e| SvgError(e.to_string()))?;
            Ok((tree, has_text))
        })?;
        Ok(Self {
            tree,
            has_text,
            data: data.to_vec(),
            compat: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// The SVG's size in user units (`width`/`height`, else the viewBox).
    pub fn size(&self) -> (f64, f64) {
        let s = self.tree.size();
        (f64::from(s.width()), f64::from(s.height()))
    }

    /// Whether the SVG has `<text>` (production text gate remains).
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// `drawImage(img, 0, 0, width, height)` into a cleared canvas: the
    /// SVG's user space stretched over `width` × `height`, premultiplied
    /// RGBA8, top-down.
    pub fn raster(&self, width: u32, height: u32) -> Bitmap {
        let (sw, sh) = self.size();
        let t = Affine::scale_non_uniform(f64::from(width) / sw, f64::from(height) / sh);
        self.raster_device(t, PixelRect::new(0, 0, width, height), None)
    }

    /// The SVG under `transform` (user units to device pixels), drawn by
    /// resvg into the device rectangle `rect`: premultiplied RGBA8,
    /// top-down, `rect.width` × `rect.height`. `clip` (a closed polygon in
    /// device pixels) limits it, antialiased.
    pub fn raster_device(&self, transform: Affine, rect: PixelRect, clip: Option<&[kurbo::Point]>) -> Bitmap {
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let empty = || Bitmap {
            width: w,
            height: h,
            rgba: vec![0; (w * h * 4) as usize],
        };
        let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else {
            return empty();
        };
        let to_rect = Affine::translate((-f64::from(rect.x), -f64::from(rect.y)));
        let c = (to_rect * transform).as_coeffs().map(|v| v as f32);
        resvg::render(
            &self.tree,
            tiny_skia::Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]),
            &mut pixmap.as_mut(),
        );
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
            mask.fill_path(&path, tiny_skia::FillRule::Winding, true, tiny_skia::Transform::identity());
            pixmap.apply_mask(&mask);
        }
        Bitmap {
            width: w,
            height: h,
            rgba: pixmap.take(),
        }
    }

    /// Whether [`SvgDocument::raster_compat`] paints this document as
    /// Chromium does at every bucket `svgRasterDimensions` can choose for an
    /// asset of `source_width` × `source_height` (the asset record's `w`
    /// and `h`; see [`svg_bucket_ladder`]). The error names what is not
    /// emulated yet (docs/engine/vector.md, "What level 1 draws"). The
    /// verdict is computed once per source size and kept with the document.
    pub fn compat_supported(&self, source_width: f64, source_height: f64) -> Result<(), SvgError> {
        let key = (source_width.to_bits(), source_height.to_bits());
        let mut cache = self.compat.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, v)) = cache.iter().find(|(k, _)| *k == key) {
            return v.clone();
        }
        let sizes = svg_bucket_ladder(source_width, source_height, SVG_MAX_DIMENSION);
        let verdict = self.compat_supported_at(&sizes);
        cache.push((key, verdict.clone()));
        verdict
    }

    /// Whether [`SvgDocument::raster_compat`] paints this document as
    /// Chromium does at each of `sizes` (uncached).
    pub fn compat_supported_at(&self, sizes: &[(u32, u32)]) -> Result<(), SvgError> {
        if self.has_text {
            return Err(SvgError("SVG text needs pm-text".into()));
        }
        with_document_stack(&self.data, || {
            let (annotated, elements) = crate::canvas2d::svg::annotate(&self.data)?;
            let tree = usvg::Tree::from_data(&annotated, &options(false)).map_err(|e| SvgError(e.to_string()))?;
            crate::canvas2d::svg::supported(&annotated, &tree, &elements, sizes)
        })
    }

    pub fn raster_compat(&self, width: u32, height: u32) -> Result<Bitmap, SvgError> {
        if self.has_text {
            return Err(SvgError("SVG text needs pm-text".into()));
        }
        with_document_stack(&self.data, || {
            let (annotated, elements) = crate::canvas2d::svg::annotate(&self.data)?;
            let tree = usvg::Tree::from_data(&annotated, &options(false)).map_err(|e| SvgError(e.to_string()))?;
            crate::canvas2d::svg::raster(&annotated, &tree, &elements, width, height, &mut |_, _, _, _| {
                Err(SvgError("SVG text needs pm-text".into()))
            })
        })
    }

    /// Bucket paint with text shaped and rasterized by the caller's pm-text
    /// font resolver. This diagnostic API does not retire the level-1 gate.
    pub fn raster_compat_with_text<R: pm_text::font::FontResolver>(
        &self,
        width: u32,
        height: u32,
        compat: &mut pm_text::compat::Compat<R>,
    ) -> Result<Bitmap, SvgError> {
        // The resolver is used only by the parse thread while this thread
        // blocks on it (a scoped join), so the exclusive borrow moves across
        // threads without a `Send` bound on the resolver type.
        struct Exclusive<T>(*mut T);
        unsafe impl<T> Send for Exclusive<T> {}
        let compat = Exclusive(compat as *mut pm_text::compat::Compat<R>);
        let (data, has_text) = (&self.data, self.has_text);
        with_document_stack(data, move || {
            let compat = compat;
            // SAFETY: the pointer came from an exclusive borrow that outlives
            // the scoped thread, and nothing else touches it meanwhile.
            let compat: &mut pm_text::compat::Compat<R> = unsafe { &mut *compat.0 };
            let (annotated, elements) = crate::canvas2d::svg::annotate(data)?;
            let tree = usvg::Tree::from_data(&annotated, &options(has_text)).map_err(|e| SvgError(e.to_string()))?;
            let mut renderer = crate::canvas2d::svg_text::TextRenderer::new(compat);
            crate::canvas2d::svg::raster(&annotated, &tree, &elements, width, height, &mut move |t, m, w, h| renderer.raster(t, m, w, h))
        })
    }

    /// User-space bounds of everything the SVG draws (strokes and filter
    /// regions included), or `None` when it draws nothing.
    pub fn user_bounds(&self) -> Option<kurbo::Rect> {
        let root = self.tree.root();
        if !root.has_children() {
            return None;
        }
        let b = root.abs_layer_bounding_box();
        Some(kurbo::Rect::new(
            f64::from(b.left()),
            f64::from(b.top()),
            f64::from(b.right()),
            f64::from(b.bottom()),
        ))
    }

    /// The SVG as coverage items under `transform` (user units to device
    /// pixels), or `None` when it draws anything but solid fills and strokes
    /// in plain groups.
    pub fn scene(&self, transform: Affine) -> Option<CoverageScene> {
        let mut scene = CoverageScene::default();
        push_group(self.tree.root(), transform, &mut scene)?;
        Some(scene)
    }
}

/// Whether the document has a `<text>` element.
fn contains_text(data: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(data) else {
        return false;
    };
    let Ok(doc) = usvg::roxmltree::Document::parse_with_options(
        s,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) else {
        return false;
    };
    doc.descendants().any(|n| n.is_element() && n.tag_name().name() == "text")
}

fn affine(t: tiny_skia::Transform) -> Affine {
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
/// The largest bucket dimension the compositor asks for (`compositor.ts`).
pub const SVG_MAX_DIMENSION: f64 = 8192.0;

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
    bucket_at_scale(sw, sh, limit, requested)
}

/// Every bucket [`svg_raster_dimensions`] can return for a source size,
/// whatever the box and matrix: the requested scale is a multiple of 0.25
/// between 0.25 and the bound, so the ladder is finite. Ordered by scale,
/// without repeats.
pub fn svg_bucket_ladder(source_width: f64, source_height: f64, max_dimension: f64) -> Vec<(u32, u32)> {
    use crate::js;
    let num = |v: f64, d: f64| if v.is_nan() || v == 0.0 { d } else { v };
    let sw = js::max(1.0, num(source_width, 1.0));
    let sh = js::max(1.0, num(source_height, 1.0));
    let limit = js::max(1.0, num(max_dimension, 8192.0));
    let max_scale = js::max(0.25, limit / js::max(sw, sh));
    let steps = libm::ceil(js::max(0.25, max_scale) * 4.0 - 1e-9) as u32;
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(steps as usize);
    for k in 1..=steps {
        let (w, h, _) = bucket_at_scale(sw, sh, limit, f64::from(k) / 4.0);
        if out.last() != Some(&(w, h)) {
            out.push((w, h));
        }
    }
    out
}

fn bucket_at_scale(sw: f64, sh: f64, limit: f64, requested: f64) -> (u32, u32, f64) {
    use crate::js;
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
        assert_eq!(
            svg_raster_dimensions(100.0, 50.0, 400.0, 200.0, &[2.0, 0.0, 0.0, 2.0, 0.0, 0.0], 8192.0),
            (800, 400, 8.0)
        );
        assert_eq!(
            svg_raster_dimensions(100.0, 50.0, 400.0, 400.0, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 8192.0),
            (800, 400, 8.0)
        );
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

    /// Every bucket `svgRasterDimensions` returns for a source size is on
    /// that size's ladder, whatever the box and matrix.
    #[test]
    fn the_bucket_ladder_covers_every_requested_bucket() {
        for (sw, sh) in [(100.0, 50.0), (24.0, 24.0), (320.0, 180.0), (1.0, 1.0), (3000.0, 7.0)] {
            let ladder = svg_bucket_ladder(sw, sh, SVG_MAX_DIMENSION);
            assert!(ladder.windows(2).all(|w| w[0] != w[1]));
            let mut seed = 0x9E37_79B9_7F4A_7C15u64;
            let mut rand = || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 11) as f64 / (1u64 << 53) as f64
            };
            for _ in 0..2000 {
                let s = rand() * 300.0;
                let a = rand() * 6.3;
                let m = [s * a.cos(), s * a.sin() * (rand() - 0.5), -s * a.sin(), s * a.cos() * 2.0 * rand(), 0.0, 0.0];
                let (bw, bh) = (rand() * 4000.0, rand() * 4000.0);
                let (w, h, _) = svg_raster_dimensions(sw, sh, bw, bh, &m, SVG_MAX_DIMENSION);
                assert!(ladder.contains(&(w, h)), "{sw}x{sh}: {w}x{h} not on the ladder");
            }
        }
        assert_eq!(svg_bucket_ladder(100.0, 50.0, SVG_MAX_DIMENSION)[0], (25, 13));
        assert!(svg_bucket_ladder(100.0, 50.0, SVG_MAX_DIMENSION).contains(&(800, 400)));
    }

    const NS: &str = r#"xmlns="http://www.w3.org/2000/svg" width="80" height="60" viewBox="0 0 80 60""#;

    fn gate(body: &str) -> Result<(), SvgError> {
        let svg = format!("<svg {NS}>{body}</svg>");
        SvgDocument::parse(svg.as_bytes()).unwrap().compat_supported(80.0, 60.0)
    }

    /// The level 1 gate admits what the canvas references measure inside
    /// the 2D class and names everything else (docs/engine/vector.md, "What
    /// level 1 draws").
    #[test]
    fn the_level1_gate_admits_only_measured_paint() {
        let admitted = [
            // Analytic draws only: rects, a round rect, ovals.
            r##"<rect x="4.5" y="4" width="40" height="30" fill="#A73366"/><rect width="10" height="10" rx="3" fill="#123"/><circle cx="50" cy="30" r="12" fill="#4C8DFF" fill-opacity="0.5"/><ellipse cx="20" cy="40" rx="9" ry="5" fill="red"/>"##,
            // Tessellated draws only, either fill rule.
            r##"<path d="M10 10 L70 20 L40 55 Z" fill="#4C8DFF"/><path fill-rule="evenodd" d="M5 5 H75 V55 H5 Z M20 20 H60 V40 H20 Z" fill="#E8E2CF"/><polygon points="1,1 9,1 5,9" fill="blue"/>"##,
            // Curves are tessellated wedges.
            r##"<path d="M0 50 C 20 10, 40 70, 80 30 S 60 10, 40 5 Q 20 0 0 50 Z" fill="#0C0D12"/>"##,
            // Padded linear and centered radial gradients of up to four stops.
            r##"<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1B2A4A"/><stop offset="60%" stop-color="#4C8DFF"/><stop offset="1" stop-color="#FFD66B"/></linearGradient><radialGradient id="r" href="#g" cx="0.5" cy="0.5" r="0.5"/></defs><rect width="80" height="60" rx="8" fill="url(#g)"/><circle cx="40" cy="30" r="20" fill="url(#r)"/>"##,
            // Group opacity over analytic draws; a circle clip at the
            // source's aspect.
            r##"<g opacity="0.6"><rect x="4" y="4" width="40" height="40" fill="#A73366"/></g><defs><clipPath id="c"><circle cx="40" cy="30" r="25"/></clipPath></defs><g clip-path="url(#c)"><rect width="80" height="30" fill="#3FCF8E"/><rect y="30" width="80" height="30" fill="#1B2A4A"/></g>"##,
            // Metadata, editor namespaces, inline style, display none.
            r##"<title>t</title><desc>d</desc><metadata><x xmlns="urn:x"><y/></x></metadata><g xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" inkscape:label="layer"><rect width="10" height="10" style="fill:#fff; fill-opacity:0.5"/><path d="M0 0 L5 5" display="none"/></g>"##,
            // A stroke property without a stroke paints nothing.
            r##"<circle cx="10" cy="10" r="5" fill="red" stroke-width="3" stroke="none"/>"##,
        ];
        for body in admitted {
            assert_eq!(gate(body), Ok(()), "{body}");
        }
        // The document `apps/desktop/e2e/import-svg.spec.ts` imports.
        let card = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect width="64" height="32" rx="8" fill="#ff5a1f"/><circle cx="48" cy="16" r="8" fill="#ffffff"/></svg>"##;
        assert_eq!(SvgDocument::parse(card).unwrap().compat_supported(64.0, 32.0), Ok(()));
        let routed = [
            (r##"<style>rect{fill:red}</style><rect width="10" height="10"/>"##, "<style>"),
            (r##"<defs><rect id="a" width="5" height="5"/></defs><use href="#a"/>"##, "<use>"),
            (r##"<svg width="10" height="10"><rect width="5" height="5"/></svg>"##, "<svg>"),
            (r##"<path d="M10 10 A 5 5 0 0 1 20 20 Z" fill="red"/>"##, "d="),
            (r##"<rect width="50%" height="10" fill="red"/>"##, "width="),
            (r##"<rect width="10" height="10" fill="red" mask="url(#m)"/>"##, "mask="),
            (r##"<defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter></defs><rect width="10" height="10" filter="url(#f)"/>"##, "filter"),
            (r##"<rect width="10" height="10" fill="red" stroke="blue"/>"##, "strokes"),
            (r##"<rect x="0.5" width="10" height="10" fill="red" shape-rendering="crispEdges"/>"##, "non-antialiased"),
            (r##"<defs><linearGradient id="g" spreadMethod="repeat" x2="0.5"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient></defs><rect width="10" height="10" fill="url(#g)"/>"##, "spread"),
            (r##"<defs><radialGradient id="g" fx="0.3"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient></defs><rect width="10" height="10" fill="url(#g)"/>"##, "focal"),
            (r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="0.25" stop-color="#0f0"/><stop offset="0.5" stop-color="#00f"/><stop offset="0.75" stop-color="#ff0"/><stop offset="1" stop-color="#0ff"/></linearGradient></defs><rect width="10" height="10" fill="url(#g)"/>"##, "four stops"),
            (r##"<defs><clipPath id="c" clipPathUnits="objectBoundingBox"><circle cx="0.5" cy="0.5" r="0.5"/></clipPath></defs><rect width="10" height="10" clip-path="url(#c)"/>"##, "clipPathUnits"),
            // A tessellated path next to an analytic oval in one pass.
            (r##"<path d="M10 10 L70 20 L40 55 Z" fill="#4C8DFF"/><circle cx="56" cy="21" r="11" fill="#FF6B1A"/>"##, "multisampled pass"),
            // An analytic clip over a tessellated path.
            (r##"<defs><clipPath id="c"><circle cx="40" cy="30" r="25"/></clipPath></defs><path clip-path="url(#c)" d="M10 10 L70 20 L40 55 Z" fill="#4C8DFF"/>"##, "multisampled pass"),
            // A path clip is drawn into the depth attachment.
            (r##"<defs><clipPath id="c"><path d="M0 0 L80 0 L40 60 Z"/></clipPath></defs><rect clip-path="url(#c)" width="80" height="60" fill="red"/>"##, "depth attachment"),
            (r##"<text x="1" y="9">a</text>"##, "text"),
        ];
        for (body, why) in routed {
            let e = gate(body).expect_err(body);
            assert!(e.0.contains(why), "{body}: {e}");
        }
    }

    #[test]
    fn the_gate_verdict_is_kept_with_the_document() {
        let svg = format!(r##"<svg {NS}><rect width="10" height="10" fill="red"/></svg>"##);
        let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
        assert_eq!(doc.compat_supported(80.0, 60.0), Ok(()));
        assert_eq!(doc.compat_supported(80.0, 60.0), Ok(()));
        assert_eq!(doc.compat_supported(1.0, 1.0), Ok(()));
        assert_eq!(doc.compat.lock().unwrap().len(), 2);
    }

    /// Documents up to the inline depth parse, judge and raster on a stack
    /// as small as the engine's worker threads.
    #[test]
    fn shallow_documents_parse_on_a_small_stack() {
        let svg = format!(
            r##"<svg {NS}>{}<rect x="1.5" width="10" height="10" fill="red"/>{}</svg>"##,
            "<g>".repeat(INLINE_DEPTH - 2),
            "</g>".repeat(INLINE_DEPTH - 2)
        );
        assert_eq!(element_depth(svg.as_bytes()), INLINE_DEPTH);
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
                assert_eq!(doc.compat_supported(80.0, 60.0), Ok(()));
                let b = doc.raster_compat(80, 60).unwrap();
                assert_eq!(b.rgba[(5 * 80 + 5) * 4 + 3], 255);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
