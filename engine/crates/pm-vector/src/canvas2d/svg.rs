//! SVG paint records replayed by the Chromium/Graphite Canvas2D emulation:
//! the usvg tree gives the render list and resolved styles, the raw document
//! gives Blink's double-precision transforms and the original shapes
//! (`blinksvg` builds the paint record, `svgrender` replays it).
use super::blinksvg::{self, Affine, Geometry, Gradient, GradientKind, ImageDraw, ImageFilter, ItemState, Op, PaintResolver, RawElements, RecordBuilder, ShapeStyle, TextItem};
use super::msaa::Rule;
use super::skpath::{Mat, SkPath};
use super::skrrect::{SkRRect, oval_path, rect_path};
use super::svgrender::Canvas;
use crate::{canvas2d::shape::Bitmap, render::svg::SvgError};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Element {
    pub kind: String,
    pub attrs: BTreeMap<String, String>,
}
pub type Elements = BTreeMap<String, Element>;

/// Injects ids on every element that lacks one so usvg's resolved nodes can
/// be matched with the raw elements, and records the raw attributes.
pub fn annotate(data: &[u8]) -> Result<(Vec<u8>, Elements), SvgError> {
    let text = std::str::from_utf8(data).map_err(|e| SvgError(e.to_string()))?;
    let doc = usvg::roxmltree::Document::parse(text).map_err(|e| SvgError(e.to_string()))?;
    if doc
        .descendants()
        .filter(|n| n.has_tag_name("style"))
        .any(|n| n.text().is_some_and(|css| css.contains("[id")))
    {
        return Err(SvgError(
            "SVG id-attribute selectors need a geometry association without injected IDs".into(),
        ));
    }
    let mut elements = Elements::new();
    let mut insert = Vec::new();
    let mut prefix = "__pm_svg_".to_string();
    while text.contains(&prefix) {
        prefix.push('_');
    }
    for (i, n) in doc.descendants().filter(|n| n.is_element()).enumerate() {
        let kind = n.tag_name().name();
        let id = n.attribute("id").map(str::to_string).unwrap_or_else(|| format!("{prefix}{i}"));
        if n.attribute("id").is_none() {
            let start = n.range().start;
            let end = text[start..]
                .find([' ', '\t', '\r', '\n', '/', '>'])
                .ok_or_else(|| SvgError("element has no end".into()))?
                + start;
            insert.push((end, format!(" id=\"{id}\"")));
        }
        elements.insert(
            id,
            Element {
                kind: kind.into(),
                attrs: n.attributes().map(|a| (a.name().into(), a.value().into())).collect(),
            },
        );
    }
    let mut result = text.to_string();
    for (at, id) in insert.into_iter().rev() {
        result.insert_str(at, &id)
    }
    Ok((result.into_bytes(), elements))
}


fn rule(r: usvg::FillRule) -> Rule {
    match r {
        usvg::FillRule::NonZero => Rule::NonZero,
        usvg::FillRule::EvenOdd => Rule::EvenOdd,
    }
}

/// A length attribute (user units; `px` allowed).
fn number(e: &Element, name: &str) -> Option<f32> {
    e.attrs.get(name).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
}

/// Blink's geometry for a usvg path with its raw element: rects and
/// ellipses keep their identity (`LayoutSVGRect`, `LayoutSVGEllipse`),
/// rounded rects become `addRRect` paths, `<path>` data goes through
/// Blink's SVG path parser, polylines/polygons/lines are line paths.
fn geometry(p: &usvg::Path, elements: &Elements) -> Geometry {
    let source = elements.get(p.id());
    if let Some(e) = source {
        match e.kind.as_str() {
            "path" => {
                if let Some(d) = e.attrs.get("d") {
                    return Geometry::Path(super::blink::path_from_svg(d));
                }
            }
            "circle" | "ellipse" => {
                let (cx, cy) = (number(e, "cx").unwrap_or(0.0), number(e, "cy").unwrap_or(0.0));
                let (rx, ry) = if e.kind == "circle" {
                    let r = number(e, "r").unwrap_or(0.0);
                    (r, r)
                } else {
                    let rx = number(e, "rx");
                    let ry = number(e, "ry");
                    (rx.or(ry).unwrap_or(0.0), ry.or(rx).unwrap_or(0.0))
                };
                if rx > 0.0 && ry > 0.0 {
                    return Geometry::Oval([cx - rx, cy - ry, cx + rx, cy + ry]);
                }
            }
            "rect" => {
                let (x, y) = (number(e, "x").unwrap_or(0.0), number(e, "y").unwrap_or(0.0));
                let (w, h) = (number(e, "width").unwrap_or(0.0), number(e, "height").unwrap_or(0.0));
                if w > 0.0 && h > 0.0 {
                    let rect = [x, y, x + w, y + h];
                    let rx_attr = number(e, "rx");
                    let ry_attr = number(e, "ry");
                    // SVGRectElement::AsMutablePath radii rules.
                    let mut rx = rx_attr.unwrap_or(-1.0);
                    let mut ry = ry_attr.unwrap_or(-1.0);
                    if rx_attr.is_none() || rx < 0.0 {
                        rx = ry.max(0.0);
                    }
                    if ry_attr.is_none() || ry < 0.0 {
                        ry = rx;
                    }
                    if rx > 0.0 || ry > 0.0 {
                        rx = rx.min(w * 0.5);
                        ry = ry.min(h * 0.5);
                        let rr = SkRRect::make_rect_xy(rect, rx, ry);
                        return Geometry::Path(rr.to_path(true, 0));
                    }
                    return Geometry::Rect(rect);
                }
            }
            "line" => {
                let mut path = SkPath::new();
                path.move_to([number(e, "x1").unwrap_or(0.0), number(e, "y1").unwrap_or(0.0)]);
                path.line_to([number(e, "x2").unwrap_or(0.0), number(e, "y2").unwrap_or(0.0)]);
                return Geometry::Path(path);
            }
            "polyline" | "polygon" => {
                if let Some(pts) = e.attrs.get("points") {
                    let nums: Vec<f32> = pts
                        .split(|c: char| c.is_whitespace() || c == ',')
                        .filter(|s| !s.is_empty())
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    let mut path = SkPath::new();
                    for (i, xy) in nums.chunks_exact(2).enumerate() {
                        if i == 0 {
                            path.move_to([xy[0], xy[1]]);
                        } else {
                            path.line_to([xy[0], xy[1]]);
                        }
                    }
                    if e.kind == "polygon" && nums.len() >= 2 {
                        path.close();
                    }
                    return Geometry::Path(path);
                }
            }
            _ => {}
        }
    }
    // Anything else: usvg's resolved path data.
    use usvg::tiny_skia_path::PathSegment;
    let mut out = SkPath::new();
    for s in p.data().segments() {
        match s {
            PathSegment::MoveTo(a) => out.move_to([a.x, a.y]),
            PathSegment::LineTo(a) => out.line_to([a.x, a.y]),
            PathSegment::QuadTo(a, b) => out.quad_to([a.x, a.y], [b.x, b.y]),
            PathSegment::CubicTo(a, b, c) => out.cubic_to([a.x, a.y], [b.x, b.y], [c.x, c.y]),
            PathSegment::Close => out.close(),
        }
    }
    Geometry::Path(out)
}

/// The object bounding box Blink computes for a shape (`ObjectBoundingBox`).
fn object_bbox(g: &Geometry) -> Option<[f32; 4]> {
    match g {
        Geometry::Rect(r) | Geometry::Oval(r) => Some(*r),
        Geometry::Path(p) => p.bounds(),
    }
}

/// Paint servers resolved the way `LayoutSVGResourceGradient::BuildGradientData`
/// and Blink's `Gradient` do.
struct Resolver<'a> {
    elements: &'a Elements,
    /// The root viewport (user units) for percentage lengths.
    viewport: [f32; 2],
}

/// A gradient attribute resolved through the `href` chain
/// (`SVGGradientElement::CollectCommonAttributes`).
fn gradient_attr<'a>(elements: &'a Elements, id: &str, name: &str) -> Option<&'a str> {
    let mut cur = id.to_string();
    for _ in 0..32 {
        let e = elements.get(&cur)?;
        if let Some(v) = e.attrs.get(name) {
            return Some(v.as_str());
        }
        let href = e.attrs.get("href").or_else(|| e.attrs.get("xlink:href"))?;
        cur = href.trim_start_matches('#').to_string();
    }
    None
}

/// An SVG length for gradient geometry: `SVGLengthContext` resolution
/// against the unit box (objectBoundingBox) or the viewport.
fn gradient_length(v: Option<&str>, default: f32, bbox_units: bool, viewport: f32) -> f32 {
    let Some(v) = v else { return default };
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%') {
        let n: f32 = p.trim().parse().unwrap_or(0.0);
        return n / 100.0 * if bbox_units { 1.0 } else { viewport };
    }
    v.trim_end_matches("px").parse().unwrap_or(default)
}

impl PaintResolver for Resolver<'_> {
    fn gradient(&self, paint: &usvg::Paint, bbox: Option<[f32; 4]>) -> Result<Option<Gradient>, SvgError> {
        let base: &usvg::BaseGradient = match paint {
            usvg::Paint::Color(_) => return Ok(None),
            usvg::Paint::LinearGradient(g) => g,
            usvg::Paint::RadialGradient(g) => g,
            usvg::Paint::Pattern(_) => return Err(SvgError("SVG pattern paint server is not implemented by the compatibility canvas".into())),
        };
        let id = base.id();
        let attr = |name: &str| gradient_attr(self.elements, id, name);
        let bbox_units = attr("gradientUnits").is_none_or(|u| u.trim() != "userSpaceOnUse");
        let [vw, vh] = self.viewport;
        let vdiag = ((vw * vw + vh * vh) / 2.0).sqrt();
        let kind = match paint {
            usvg::Paint::LinearGradient(_) => GradientKind::Linear {
                p0: [gradient_length(attr("x1"), 0.0, bbox_units, vw), gradient_length(attr("y1"), 0.0, bbox_units, vh)],
                p1: [gradient_length(attr("x2"), if bbox_units { 1.0 } else { vw }, bbox_units, vw), gradient_length(attr("y2"), 0.0, bbox_units, vh)],
            },
            _ => {
                let cx = gradient_length(attr("cx"), if bbox_units { 0.5 } else { vw * 0.5 }, bbox_units, vw);
                let cy = gradient_length(attr("cy"), if bbox_units { 0.5 } else { vh * 0.5 }, bbox_units, vh);
                let r = gradient_length(attr("r"), if bbox_units { 0.5 } else { vdiag * 0.5 }, bbox_units, vdiag);
                let fx = gradient_length(attr("fx"), cx, bbox_units, vw);
                let fy = gradient_length(attr("fy"), cy, bbox_units, vh);
                let fr = gradient_length(attr("fr"), 0.0, bbox_units, vdiag);
                GradientKind::Conical { p0: [fx, fy], r0: fr, p1: [cx, cy], r1: r }
            }
        };
        // userspace_transform: the object bounding box for objectBoundingBox
        // units, then the gradientTransform (double).
        let mut userspace = Affine::IDENTITY;
        if bbox_units {
            let Some(b) = bbox else { return Ok(None) };
            if b[2] - b[0] <= 0.0 || b[3] - b[1] <= 0.0 {
                return Ok(None);
            }
            userspace.translate(f64::from(b[0]), f64::from(b[1]));
            userspace.scale(f64::from(b[2] - b[0]), f64::from(b[3] - b[1]));
        }
        let gradient_transform = blinksvg::element_transform(attr("gradientTransform"))?;
        if !gradient_transform.is_invertible() {
            return Ok(None);
        }
        userspace.pre_concat(&gradient_transform);
        // Gradient::FillSkiaStops: pad to 0 and 1, colors as SkColor4f.
        let mut colors = Vec::new();
        let mut positions = Vec::new();
        let stops = base.stops();
        let color = |s: &usvg::Stop| [f32::from(s.color().red) / 255.0, f32::from(s.color().green) / 255.0, f32::from(s.color().blue) / 255.0, s.opacity().get()];
        if stops.is_empty() {
            positions.push(0.0);
            colors.push([0.0, 0.0, 0.0, 0.0]);
        } else if stops[0].offset().get() > 0.0 {
            positions.push(0.0);
            colors.push(color(&stops[0]));
        }
        for s in stops {
            positions.push(s.offset().get());
            colors.push(color(s));
        }
        if positions.last().is_some_and(|p| *p < 1.0) {
            positions.push(1.0);
            colors.push(*colors.last().unwrap());
        }
        let tile = match base.spread_method() {
            usvg::SpreadMethod::Pad => 0,
            usvg::SpreadMethod::Repeat => 1,
            usvg::SpreadMethod::Reflect => 2,
        };
        Ok(Some(Gradient { kind, colors, positions, tile, local_matrix: userspace.to_mat() }))
    }
}

/// The raw transform attribute of a usvg node's element, as Blink computes it.
fn raw_transform(elements: &Elements, id: &str, fallback: usvg::Transform) -> Result<Affine, SvgError> {
    match elements.get(id) {
        Some(e) => blinksvg::element_transform(e.attrs.get("transform").map(String::as_str)),
        None => {
            let t = fallback;
            Ok(Affine([t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f64::from)))
        }
    }
}

struct Walk<'a> {
    builder: RecordBuilder<'a>,
    elements: &'a Elements,
    resolver: Resolver<'a>,
}

impl Walk<'_> {
    fn group(&mut self, g: &usvg::Group, parent: ItemState, is_root: bool) -> Result<(), SvgError> {
        if g.mask().is_some() {
            return Err(SvgError("SVG mask paint record needs compatibility support".into()));
        }
        if g.blend_mode() != usvg::BlendMode::Normal {
            return Err(SvgError("SVG blend mode paint record needs compatibility support".into()));
        }
        let mut state = parent;
        if !is_root {
            // The element's own transform node (UpdateTransformForSVGChild).
            let m = raw_transform(self.elements, g.id(), g.transform())?;
            state.transform = self.builder.push_transform(parent.transform, m);
            // clip-path (UpdateClipPathClip): the clip's path in the element's
            // user space.
            if let Some(clip) = g.clip_path() {
                let path = self.clip_path(clip)?;
                state.clip = self.builder.push_clip_path(state.clip, state.transform, &path.0, path.1);
            }
            // opacity (UpdateEffect), then filter (UpdateFilter) inside it.
            let opacity = g.opacity().get();
            if opacity != 1.0 {
                state.effect = self.builder.push_effect(state.effect, state.transform, state.clip, Some(opacity), None);
            }
            for f in g.filters() {
                let filter = self.filter(f, g)?;
                state.effect = self.builder.push_effect(state.effect, state.transform, state.clip, None, Some(filter));
            }
        }
        for n in g.children() {
            match n {
                usvg::Node::Group(c) => self.group(c, state, false)?,
                usvg::Node::Path(p) => self.path(p, state)?,
                usvg::Node::Text(t) => {
                    let ops = vec![Op::DrawText(Box::new(TextItem { node: (**t).clone() }))];
                    self.builder.push_item(state, ops);
                }
                usvg::Node::Image(_) => return Err(SvgError("embedded SVG image needs a decoded bitmap".into())),
            }
        }
        Ok(())
    }

    fn path(&mut self, p: &usvg::Path, parent: ItemState) -> Result<(), SvgError> {
        if !p.is_visible() {
            return Ok(());
        }
        let mut state = parent;
        // usvg folds a path's own `transform` into its abs_transform; Blink
        // gives it its own transform node.
        let own = match self.elements.get(p.id()) {
            Some(e) => blinksvg::element_transform(e.attrs.get("transform").map(String::as_str))?,
            None => Affine::IDENTITY,
        };
        state.transform = self.builder.push_transform(parent.transform, own);
        let geometry = geometry(p, self.elements);
        let bbox = object_bbox(&geometry);
        let style = ShapeStyle {
            fill: p.fill(),
            stroke: p.stroke(),
            paint_order: p.paint_order(),
            anti_alias: !matches!(p.rendering_mode(), usvg::ShapeRendering::CrispEdges | usvg::ShapeRendering::OptimizeSpeed),
        };
        let fill_rule = p.fill().map_or(Rule::NonZero, |f| rule(f.rule()));
        let ops = blinksvg::shape_ops(&geometry, &style, fill_rule, bbox, &self.resolver)?;
        self.builder.push_item(state, ops);
        Ok(())
    }

    /// `LayoutSVGResourceClipper::AsPath` for a clip path of shapes
    /// (`ToClipPath`: each shape's path transformed by its own transform
    /// and the clip's `transform`/`clipPathUnits`), unioned when several.
    fn clip_path(&mut self, clip: &usvg::ClipPath) -> Result<(SkPath, Rule), SvgError> {
        if clip.clip_path().is_some() {
            return Err(SvgError("nested SVG clip-path needs compatibility support".into()));
        }
        let mut shapes: Vec<(SkPath, Rule)> = Vec::new();
        for n in clip.root().children() {
            match n {
                usvg::Node::Path(p) => {
                    if !p.is_visible() {
                        continue;
                    }
                    let own = match self.elements.get(p.id()) {
                        Some(e) => blinksvg::element_transform(e.attrs.get("transform").map(String::as_str))?,
                        None => Affine::IDENTITY,
                    };
                    let g = geometry(p, self.elements);
                    let mut path = match g {
                        Geometry::Rect(r) => rect_path(r, true, 0),
                        Geometry::Oval(r) => oval_path(r, true, 1),
                        Geometry::Path(p) => p,
                    };
                    let clip_transform = match self.elements.get(clip.id()).and_then(|e| e.attrs.get("transform")) {
                        Some(t) => blinksvg::element_transform(Some(t))?,
                        None => {
                            let t = clip.transform();
                            Affine([t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f64::from))
                        }
                    };
                    let mut m = own;
                    if !clip_transform.is_identity() {
                        m.post_concat(&clip_transform);
                    }
                    if !m.is_identity() {
                        path = path.transform(&m.to_mat());
                    }
                    let r = p.fill().map_or(Rule::NonZero, |f| rule(f.rule()));
                    shapes.push((path, r));
                }
                _ => return Err(SvgError("nested SVG clip record".into())),
            }
        }
        match shapes.len() {
            0 => Ok((SkPath::new(), Rule::NonZero)),
            1 => Ok(shapes.pop().unwrap()),
            _ => Err(SvgError("SVG clip-path with several shapes needs path ops".into())),
        }
    }

    /// Blink's filter chain for a usvg filter: only a lone Gaussian blur.
    fn filter(&self, f: &usvg::filter::Filter, _g: &usvg::Group) -> Result<ImageFilter, SvgError> {
        let prims = f.primitives();
        if prims.len() != 1 {
            return Err(SvgError("SVG filter chain needs compatibility support".into()));
        }
        let p = &prims[0];
        match p.kind() {
            usvg::filter::Kind::GaussianBlur(b) => {
                let region = f.rect();
                let sub = p.rect();
                let crop = [
                    sub.x().max(region.x()),
                    sub.y().max(region.y()),
                    sub.right().min(region.right()),
                    sub.bottom().min(region.bottom()),
                ];
                Ok(ImageFilter::GaussianBlur {
                    sigma: [b.std_dev_x().get(), b.std_dev_y().get()],
                    crop,
                    linear: p.color_interpolation() == usvg::filter::ColorInterpolation::LinearRGB,
                })
            }
            _ => Err(SvgError("SVG filter is not a Gaussian blur".into())),
        }
    }
}

pub type TextDraw<'a> = dyn FnMut(&usvg::Text, Mat, u32, u32) -> Result<Bitmap, SvgError> + 'a;

/// Blink's paint record of the document laid out for `draw` (the content
/// `PaintRecordForCurrentFrame` returns; it depends on the bucket only
/// through the layout container and root transform).
fn content_record(raw: &RawElements, sizing: &blinksvg::NaturalSizing, draw: &ImageDraw, tree: &usvg::Tree, elements: &Elements) -> Result<Vec<Op>, SvgError> {
    let builder = RecordBuilder::new(raw, draw);
    let root_state = builder.root_state();
    let viewport = match sizing.view_box {
        Some(vb) => [vb[2], vb[3]],
        None => [sizing.width.unwrap_or(draw.container[0]), sizing.height.unwrap_or(draw.container[1])],
    };
    let mut walk = Walk { builder, elements, resolver: Resolver { elements, viewport } };
    walk.group(tree.root(), root_state, true)?;
    Ok(walk.builder.finish())
}

/// `SVGImage::DrawInternal` around the record: save, clip to the enclosing
/// destination rect, concat the source-to-destination matrix (the record
/// in between), restore.
fn draw_internal_prefix(draw: &ImageDraw) -> [Op; 3] {
    let c = draw.dst_clip;
    [
        Op::Save,
        Op::ClipRect { rect: [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32], aa: false },
        Op::Concat(draw.src_to_dst),
    ]
}

/// Blink's paint record for `drawImage(img, 0, 0, width, height)`.
fn image_record(doc: &usvg::roxmltree::Document<'_>, tree: &usvg::Tree, elements: &Elements, width: u32, height: u32) -> Result<Vec<Op>, SvgError> {
    let raw = RawElements::new(doc);
    let sizing = blinksvg::natural_sizing(doc);
    let draw: ImageDraw = blinksvg::image_draw(&sizing, [width, height]);
    let record = content_record(&raw, &sizing, &draw, tree, elements)?;
    let mut ops = Vec::with_capacity(record.len() + 4);
    ops.extend(draw_internal_prefix(&draw));
    ops.extend(record);
    ops.push(Op::Restore);
    Ok(ops)
}

/// Whether the replay paints this document as Chromium does at every
/// bucket in `sizes` (the sizes `svgRasterDimensions` can choose for it).
///
/// Only what the canvas references measure inside the 2D class is
/// accepted (docs/engine/vector.md, "What level 1 draws"):
///
/// - the source uses only the elements and attributes of [`check_source`];
/// - the record has no text, strokes, filters, non-antialiased shapes, or
///   gradients other than padded linear and centered radial gradients of
///   at most four stops ([`check_record`]);
/// - at every bucket, every draw pass is one the emulation reproduces
///   (`svgrender::pass_verdict`): analytic draws never share a
///   multisampled pass with tessellated draws, and no clip is drawn into
///   the depth attachment. Which renderer a draw or clip gets depends on
///   the bucket (pixel alignment, the destination's aspect), so each
///   bucket is classified with the replay's own renderer choice.
pub fn supported(data: &[u8], tree: &usvg::Tree, elements: &Elements, sizes: &[(u32, u32)]) -> Result<(), SvgError> {
    let text_doc = std::str::from_utf8(data).map_err(|e| SvgError(e.to_string()))?;
    let doc = usvg::roxmltree::Document::parse(text_doc).map_err(|e| SvgError(e.to_string()))?;
    check_source(&doc)?;
    let raw = RawElements::new(&doc);
    let sizing = blinksvg::natural_sizing(&doc);
    // The content record is rebuilt only when the layout changes (a
    // document without a natural size is laid out in the bucket).
    let mut content: Option<(LayoutKey, Vec<Op>)> = None;
    for &(width, height) in sizes {
        let draw = blinksvg::image_draw(&sizing, [width, height]);
        let key = (draw.container.map(f32::to_bits), draw.rounded, draw.root_transform);
        if content.as_ref().is_none_or(|(k, _)| *k != key) {
            let record = content_record(&raw, &sizing, &draw, tree, elements)?;
            check_record(&record)?;
            content = Some((key, record));
        }
        let record = &content.as_ref().unwrap().1;
        let mut canvas = Canvas::classifier(width, height);
        canvas.replay(&draw_internal_prefix(&draw))?;
        canvas.replay(record)?;
        canvas.replay(&[Op::Restore])?;
        canvas.finish();
        canvas.verdict().map_err(|e| SvgError(format!("{} (at a {width} × {height} bucket)", e.0)))?;
    }
    Ok(())
}

/// What the content record depends on in an `ImageDraw`: the container
/// (bits), its rounded size and the root transform.
type LayoutKey = ([u32; 2], [i32; 2], Option<Affine>);

/// The SVG namespace.
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Elements whose painting the canvas references cover (or that paint
/// nothing).
const ELEMENTS: &[&str] = &[
    "svg", "g", "defs", "path", "rect", "circle", "ellipse", "polygon", "polyline", "line",
    "linearGradient", "radialGradient", "stop", "clipPath", "title", "desc", "metadata",
];

/// Presentation properties (attributes and `style` declarations) whose
/// painting the references cover. Stroke properties are inert without a
/// stroke; a stroke itself is refused by [`check_record`].
const PROPERTIES: &[&str] = &[
    "fill", "fill-opacity", "fill-rule", "opacity", "clip-path", "clip-rule", "color", "display",
    "shape-rendering", "stop-color", "stop-opacity", "stroke", "stroke-width", "stroke-opacity",
    "stroke-linecap", "stroke-linejoin", "stroke-miterlimit", "stroke-dasharray", "stroke-dashoffset",
    // Illustrator's root `enable-background` is ignored by Blink and usvg.
    "enable-background",
];

/// Non-presentation attributes, with what their values may be.
fn attribute_ok(element: &str, name: &str, value: &str) -> Result<bool, SvgError> {
    let v = value.trim();
    let number = |v: &str| v.trim_end_matches("px").trim().parse::<f64>().is_ok_and(f64::is_finite);
    let gradient = matches!(element, "linearGradient" | "radialGradient");
    Ok(match name {
        "id" | "class" | "version" | "baseProfile" | "viewBox" | "preserveAspectRatio" | "transform" | "style" => true,
        // The root's size: numbers, px, percentages or auto
        // (`natural_sizing`).
        "width" | "height" if element == "svg" => v.is_empty() || v == "auto" || number(v) || v.strip_suffix('%').is_some_and(|p| p.trim().parse::<f64>().is_ok()),
        "x" | "y" if element == "svg" => true,
        "x" | "y" | "width" | "height" | "rx" | "ry" | "cx" | "cy" | "r" | "x1" | "y1" | "x2" | "y2" if !gradient => number(v) || v == "auto" && matches!(name, "rx" | "ry"),
        "x1" | "y1" | "x2" | "y2" | "cx" | "cy" | "r" | "fx" | "fy" | "fr" if gradient => {
            number(v) || v.strip_suffix('%').is_some_and(|p| p.trim().parse::<f64>().is_ok())
        }
        "gradientUnits" | "gradientTransform" | "spreadMethod" | "href" if gradient => true,
        "offset" if element == "stop" => number(v) || v.strip_suffix('%').is_some_and(|p| p.trim().parse::<f64>().is_ok()),
        "clipPathUnits" if element == "clipPath" => v == "userSpaceOnUse",
        "d" if element == "path" => !v.contains(['A', 'a']),
        "points" if matches!(element, "polygon" | "polyline") => true,
        _ => false,
    })
}

/// The document's elements and attributes, against what the references
/// cover. Elements outside the SVG namespace (editor metadata) are not
/// rendered by Blink or usvg and are skipped with their content.
pub fn check_source(doc: &usvg::roxmltree::Document<'_>) -> Result<(), SvgError> {
    let root = doc.root_element();
    if root.tag_name().namespace() != Some(SVG_NS) || root.tag_name().name() != "svg" {
        return Err(SvgError("the root element is not an SVG <svg>".into()));
    }
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let name = n.tag_name().name();
        if n.tag_name().namespace() != Some(SVG_NS) {
            continue;
        }
        if !ELEMENTS.contains(&name) || (name == "svg" && n != root) {
            return Err(SvgError(format!("SVG <{name}> needs compatibility support")));
        }
        if matches!(name, "title" | "desc" | "metadata") {
            continue;
        }
        for a in n.attributes() {
            let ok = match a.namespace() {
                None => PROPERTIES.contains(&a.name()) || attribute_ok(name, a.name(), a.value())?,
                Some(XML_NS) => true,
                Some(XLINK_NS) => a.name() == "href" && matches!(name, "linearGradient" | "radialGradient"),
                // Editor namespaces (Inkscape, Sodipodi, Illustrator) are
                // not SVG properties.
                Some(_) => true,
            };
            if !ok {
                return Err(SvgError(format!("SVG <{name} {}=\"{}\"> needs compatibility support", a.name(), a.value())));
            }
            if a.namespace().is_none() && a.name() == "style" {
                for decl in a.value().split(';') {
                    let Some((prop, _)) = decl.split_once(':') else {
                        if decl.trim().is_empty() {
                            continue;
                        }
                        return Err(SvgError(format!("SVG style declaration {decl:?} needs compatibility support")));
                    };
                    let prop = prop.trim();
                    if !PROPERTIES.contains(&prop) {
                        return Err(SvgError(format!("SVG style property {prop} needs compatibility support")));
                    }
                }
            }
        }
        stack.extend(n.children().filter(|c| c.is_element()));
    }
    Ok(())
}

/// The record's paint, against what the references cover.
pub fn check_record(ops: &[Op]) -> Result<(), SvgError> {
    for op in ops {
        let flags = match op {
            Op::DrawText(_) => return Err(SvgError("SVG text needs pm-text".into())),
            Op::SaveLayer { .. } => return Err(SvgError("SVG filters need compatibility support".into())),
            Op::DrawRect { flags, .. } | Op::DrawOval { flags, .. } | Op::DrawPath { flags, .. } => flags,
            _ => continue,
        };
        if flags.stroke.is_some() {
            return Err(SvgError("SVG strokes need compatibility support".into()));
        }
        if !flags.anti_alias {
            return Err(SvgError("non-antialiased SVG shapes (shape-rendering) need compatibility support".into()));
        }
        if let Some(g) = &flags.shader {
            check_gradient(g)?;
        }
    }
    Ok(())
}

fn check_gradient(g: &Gradient) -> Result<(), SvgError> {
    if g.tile != 0 {
        return Err(SvgError("SVG gradients with repeat or reflect spread need compatibility support".into()));
    }
    if let GradientKind::Conical { p0, r0, p1, .. } = g.kind
        && (p0 != p1 || r0 != 0.0)
    {
        return Err(SvgError("focal SVG radial gradients need compatibility support".into()));
    }
    let shader = super::gradient::Shader::from_blink(g);
    if shader.solid.is_some() {
        return Err(SvgError("degenerate SVG gradients need compatibility support".into()));
    }
    if shader.num_stops > 4 {
        return Err(SvgError("SVG gradients of more than four stops need compatibility support".into()));
    }
    Ok(())
}

pub fn raster(data: &[u8], tree: &usvg::Tree, elements: &Elements, width: u32, height: u32, text: &mut TextDraw<'_>) -> Result<Bitmap, SvgError> {
    let text_doc = std::str::from_utf8(data).map_err(|e| SvgError(e.to_string()))?;
    let doc = usvg::roxmltree::Document::parse(text_doc).map_err(|e| SvgError(e.to_string()))?;
    let ops = image_record(&doc, tree, elements, width, height)?;
    if super::diagnostic("PM_SVG_DEBUG").is_some() {
        for op in &ops {
            match op {
                Op::DrawPath { path, rule, flags } => eprintln!("DrawPath verbs={} rule={rule:?} stroke={:?} shader={}", path.verbs.len(), flags.stroke.as_ref().map(|s| s.width), flags.shader.is_some()),
                Op::DrawText(_) => eprintln!("DrawText"),
                other => eprintln!("{other:?}"),
            }
        }
    }
    let mut canvas = Canvas::new(width, height);
    canvas.text = Some(Box::new(|t: &usvg::Text, m: Mat, w: u32, h: u32| text(t, m, w, h).map(|b| b.rgba)));
    canvas.replay(&ops)?;
    Ok(Bitmap { width, height, rgba: canvas.finish() })
}

