//! SVG paint records replayed by the Chromium/Graphite Canvas2D emulation.
//! usvg resolves styles and paint servers; original element geometry is kept
//! because Skia recognizes ovals/rrects and preserves SVG path conics.
use super::{
    analytic::Geometry,
    context::{Context, LineCap, LineJoin},
    msaa::Rule,
    paint::{Paint, inverse},
    skpath::{Mat, SkPath},
};
use crate::{canvas2d::shape::Bitmap, render::svg::SvgError};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Element {
    pub kind: String,
    pub attrs: BTreeMap<String, String>,
}
pub type Elements = BTreeMap<String, Element>;

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
        if !matches!(kind, "path" | "rect" | "circle" | "ellipse" | "line" | "polygon" | "polyline") {
            continue;
        }
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
fn mat(t: usvg::Transform) -> Mat {
    Mat {
        sx: t.sx,
        kx: t.kx,
        ky: t.ky,
        sy: t.sy,
        tx: t.tx,
        ty: t.ty,
    }
}
fn coeff(m: Mat) -> [f64; 6] {
    [m.sx, m.ky, m.kx, m.sy, m.tx, m.ty].map(f64::from)
}
fn rule(r: usvg::FillRule) -> Rule {
    match r {
        usvg::FillRule::NonZero => Rule::NonZero,
        usvg::FillRule::EvenOdd => Rule::EvenOdd,
    }
}
fn path(p: &usvg::Path, elements: &Elements) -> (SkPath, Option<Geometry>) {
    let source = elements.get(p.id());
    let bounds = p.data().bounds();
    let r = [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()];
    let mut out = SkPath::new();
    if let Some(e) = source {
        let number = |name: &str| e.attrs.get(name).and_then(|v| v.trim_end_matches("px").parse::<f32>().ok());
        match e.kind.as_str() {
            "path" => {
                if let Some(d) = e.attrs.get("d") {
                    return (super::blink::path_from_svg(d), None);
                }
            }
            "circle" | "ellipse" => {
                out.arc_to_oval(r, 0.0, 180.0, true);
                out.arc_to_oval(r, 180.0, 180.0, false);
                out.close();
                return (
                    out,
                    Some(Geometry::FillRRect {
                        rect: r,
                        radii: [(r[2] - r[0]) * 0.5, (r[3] - r[1]) * 0.5],
                    }),
                );
            }
            "rect" => {
                let rx = number("rx").or_else(|| number("ry")).unwrap_or(0.0).min((r[2] - r[0]) * 0.5);
                let ry = number("ry").or_else(|| number("rx")).unwrap_or(0.0).min((r[3] - r[1]) * 0.5);
                if rx > 0.0 && ry > 0.0 {
                    out.move_to([r[0] + rx, r[1]]);
                    out.line_to([r[2] - rx, r[1]]);
                    out.arc_to_oval([r[2] - 2.0 * rx, r[1], r[2], r[1] + 2.0 * ry], 270.0, 90.0, false);
                    out.line_to([r[2], r[3] - ry]);
                    out.arc_to_oval([r[2] - 2.0 * rx, r[3] - 2.0 * ry, r[2], r[3]], 0.0, 90.0, false);
                    out.line_to([r[0] + rx, r[3]]);
                    out.arc_to_oval([r[0], r[3] - 2.0 * ry, r[0] + 2.0 * rx, r[3]], 90.0, 90.0, false);
                    out.line_to([r[0], r[1] + ry]);
                    out.arc_to_oval([r[0], r[1], r[0] + 2.0 * rx, r[1] + 2.0 * ry], 180.0, 90.0, false);
                    out.close();
                    return (out, Some(Geometry::FillRRect { rect: r, radii: [rx, ry] }));
                }
            }
            _ => {}
        }
    }
    use usvg::tiny_skia_path::PathSegment;
    for s in p.data().segments() {
        match s {
            PathSegment::MoveTo(a) => out.move_to([a.x, a.y]),
            PathSegment::LineTo(a) => out.line_to([a.x, a.y]),
            PathSegment::QuadTo(a, b) => out.quad_to([a.x, a.y], [b.x, b.y]),
            PathSegment::CubicTo(a, b, c) => out.cubic_to([a.x, a.y], [b.x, b.y], [c.x, c.y]),
            PathSegment::Close => out.close(),
        }
    }
    (out, None)
}
fn shader(p: &usvg::Paint, opacity: f32, m: Mat) -> Result<Paint, SvgError> {
    let color = |c: usvg::Color, a: f32| [f32::from(c.red) / 255.0, f32::from(c.green) / 255.0, f32::from(c.blue) / 255.0, a];
    let stops = |g: &usvg::BaseGradient| {
        g.stops()
            .iter()
            .map(|s| (s.offset().get(), color(s.color(), s.opacity().get())))
            .collect()
    };
    let spread = |g: &usvg::BaseGradient| match g.spread_method() {
        usvg::SpreadMethod::Pad => 0,
        usvg::SpreadMethod::Repeat => 1,
        usvg::SpreadMethod::Reflect => 2,
    };
    let inv = |t| inverse(Mat::concat(&m, &mat(t))).ok_or_else(|| SvgError("singular SVG paint transform".into()));
    Ok(match p {
        usvg::Paint::Color(c) => Paint::Solid(color(*c, opacity).map_with_alpha()),
        usvg::Paint::LinearGradient(g) => Paint::Linear {
            inverse: inv(g.transform())?,
            start: [g.x1(), g.y1()],
            end: [g.x2(), g.y2()],
            stops: stops(g),
            spread: spread(g),
            alpha: opacity,
        },
        usvg::Paint::RadialGradient(g) if g.fr().get() == 0.0 => Paint::Radial {
            inverse: inv(g.transform())?,
            center: [g.cx(), g.cy()],
            focal: [g.fx(), g.fy()],
            radius: g.r().get(),
            stops: stops(g),
            spread: spread(g),
            alpha: opacity,
        },
        _ => return Err(SvgError("SVG paint server is not implemented by the compatibility canvas".into())),
    })
}
trait Premul {
    fn map_with_alpha(self) -> Self;
}
impl Premul for [f32; 4] {
    fn map_with_alpha(mut self) -> Self {
        for i in 0..3 {
            self[i] *= self[3]
        }
        self
    }
}
fn group(ctx: &mut Context, g: &usvg::Group, base: Mat, elements: &Elements, text: &mut TextDraw<'_>) -> Result<(), SvgError> {
    if g.mask().is_some() || g.blend_mode() != usvg::BlendMode::Normal {
        return Err(SvgError("SVG mask/blend paint record needs compatibility support".into()));
    }
    ctx.save();
    if let Some(clip) = g.clip_path() {
        ctx.save();
        ctx.transform(coeff(Mat::concat(&mat(g.abs_transform()), &mat(clip.transform()))));
        let mut shapes = Vec::new();
        for n in clip.root().children() {
            if let usvg::Node::Path(p) = n {
                let (pth, geometry) = path(p, elements);
                shapes.push((
                    pth,
                    Mat::concat(&ctx.matrix(), &mat(p.abs_transform())),
                    p.fill().map_or(Rule::NonZero, |f| rule(f.rule())),
                    geometry,
                ));
            } else {
                return Err(SvgError("nested SVG clip record".into()));
            }
        }
        ctx.clip_shapes(shapes);
        // Keep clip state while restoring the matrix to the bucket transform.
        ctx.set_transform(coeff(base));
    }
    let mut blur = [0.0; 2];
    let mut linear = false;
    for f in g.filters() {
        for p in f.primitives() {
            match p.kind() {
                usvg::filter::Kind::GaussianBlur(b) => {
                    linear = p.color_interpolation() == usvg::filter::ColorInterpolation::LinearRGB;
                    let m = Mat::concat(&base, &mat(g.abs_transform()));
                    blur = [
                        f64::from(b.std_dev_x().get()) * f64::from(m.sx.hypot(m.ky)),
                        f64::from(b.std_dev_y().get()) * f64::from(m.kx.hypot(m.sy)),
                    ]
                }
                _ => return Err(SvgError("SVG filter is not a Gaussian blur".into())),
            }
        }
    }
    let layer = g.opacity().get() != 1.0 || blur != [0.0; 2];
    if layer {
        ctx.begin_layer()
    }
    for n in g.children() {
        match n {
            usvg::Node::Group(g) => group(ctx, g, base, elements, text)?,
            usvg::Node::Path(p) if p.is_visible() => {
                let (pth, geometry) = path(p, elements);
                ctx.save();
                ctx.transform(coeff(mat(p.abs_transform())));
                let fill = |ctx: &mut Context| -> Result<(), SvgError> {
                    if let Some(f) = p.fill() {
                        ctx.set_fill_paint(shader(f.paint(), f.opacity().get(), ctx.matrix())?);
                        if let Some(g) = geometry {
                            ctx.fill_geometry(g)
                        } else {
                            ctx.fill_path_rule(&pth, rule(f.rule())).map_err(|e| SvgError(e.to_string()))?;
                        }
                    }
                    Ok(())
                };
                let stroke = |ctx: &mut Context| -> Result<(), SvgError> {
                    if let Some(s) = p.stroke() {
                        ctx.set_stroke_paint(shader(s.paint(), s.opacity().get(), ctx.matrix())?);
                        ctx.set_line_width(f64::from(s.width().get()));
                        ctx.set_miter_limit(s.miterlimit().get());
                        ctx.set_line_cap(match s.linecap() {
                            usvg::LineCap::Butt => LineCap::Butt,
                            usvg::LineCap::Round => LineCap::Round,
                            usvg::LineCap::Square => LineCap::Square,
                        });
                        ctx.set_line_join(match s.linejoin() {
                            usvg::LineJoin::Round => LineJoin::Round,
                            usvg::LineJoin::Bevel => LineJoin::Bevel,
                            _ => LineJoin::Miter,
                        });
                        let dashed;
                        let draw = if let Some(d) = s.dasharray() {
                            let b = crate::render::svg::bez(p.data());
                            let ds: Vec<_> = d.iter().copied().map(f64::from).collect();
                            let b: kurbo::BezPath = kurbo::dash(b.elements().iter().copied(), f64::from(s.dashoffset()), &ds).collect();
                            dashed = super::blink::path_from_svg(&b.to_svg());
                            &dashed
                        } else {
                            &pth
                        };
                        if s.dasharray().is_none()
                            && let Some(Geometry::FillRRect { rect, radii }) = geometry
                            && radii[0] == radii[1]
                        {
                            ctx.stroke_geometry(Geometry::StrokeRRect {
                                rect,
                                corner_radius: radii[0],
                                radius: s.width().get() * 0.5,
                                join: match s.linejoin() {
                                    usvg::LineJoin::Round => -1.0,
                                    usvg::LineJoin::Bevel => 0.0,
                                    _ => s.miterlimit().get(),
                                },
                            });
                        } else {
                            ctx.stroke_path(draw).map_err(|e| SvgError(e.to_string()))?;
                        }
                    }
                    Ok(())
                };
                match p.paint_order() {
                    usvg::PaintOrder::FillAndStroke => {
                        fill(ctx)?;
                        stroke(ctx)?
                    }
                    usvg::PaintOrder::StrokeAndFill => {
                        stroke(ctx)?;
                        fill(ctx)?
                    }
                }
                ctx.restore();
            }
            usvg::Node::Path(_) => {}
            usvg::Node::Text(t) => {
                let m = Mat::concat(&base, &mat(t.abs_transform()));
                let bitmap = text(t, m, ctx.width(), ctx.height())?;
                ctx.draw_bitmap(bitmap.rgba);
            }
            usvg::Node::Image(_) => return Err(SvgError("embedded SVG image needs a decoded bitmap".into())),
        }
    }
    if layer {
        ctx.end_layer(g.opacity().get(), blur, linear)
    }
    if g.clip_path().is_some() {
        ctx.restore()
    }
    ctx.restore();
    Ok(())
}
pub type TextDraw<'a> = dyn FnMut(&usvg::Text, Mat, u32, u32) -> Result<Bitmap, SvgError> + 'a;

pub fn raster(tree: &usvg::Tree, elements: &Elements, width: u32, height: u32, text: &mut TextDraw<'_>) -> Result<Bitmap, SvgError> {
    let mut ctx = Context::new(width, height);
    ctx.scale(
        f64::from(width) / f64::from(tree.size().width()),
        f64::from(height) / f64::from(tree.size().height()),
    );
    let base = ctx.matrix();
    group(&mut ctx, tree.root(), base, elements, text)?;
    Ok(Bitmap {
        width,
        height,
        rgba: ctx.finish(),
    })
}
