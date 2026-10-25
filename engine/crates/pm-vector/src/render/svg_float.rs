//! Ordered SVG paint operations for the float GPU painter. Geometry is
//! resolved here, while coverage and colour stay in GPU float storage.

use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Shape, Stroke};

use super::{SvgDocument, affine, bez};
use crate::render::scene::FLATTEN_TOLERANCE;
use crate::render::svg_depth::with_stack;

#[derive(Clone, Debug)]
pub struct FloatStop {
    pub offset: f32,
    /// Straight alpha, sRGB encoded colour. SVG interpolates these before
    /// multiplying by opacity and coverage.
    pub rgba: [f32; 4],
}

#[derive(Clone, Debug)]
pub enum FloatGradient {
    Linear {
        p0: [f32; 2],
        p1: [f32; 2],
        inverse: [f32; 6],
    },
    Radial {
        focus: [f32; 2],
        center: [f32; 2],
        radius: f32,
        focus_radius: f32,
        inverse: [f32; 6],
    },
}

#[derive(Clone, Debug)]
pub enum FloatPaint {
    Solid([f32; 4]),
    Gradient {
        geometry: FloatGradient,
        stops: Vec<FloatStop>,
        spread: usvg::SpreadMethod,
        opacity: f32,
    },
}

#[derive(Clone, Debug)]
pub struct FloatDraw {
    /// Device coordinates, disjoint contours with nonzero winding.
    pub outline: BezPath,
    pub paint: FloatPaint,
}

#[derive(Clone, Debug)]
pub struct FloatGroup {
    pub children: Vec<FloatNode>,
    pub opacity: f32,
    pub blend: usvg::BlendMode,
    pub clip: Option<Vec<FloatNode>>,
    pub mask: Option<Vec<FloatNode>>,
    pub mask_luminance: bool,
}

#[derive(Clone, Debug)]
pub enum FloatNode {
    Draw(FloatDraw),
    Group(FloatGroup),
}

impl SvgDocument {
    /// The paint tree under the authored transform. `None` selects the
    /// established resvg path for features the float painter cannot draw.
    pub fn float_scene(&self, transform: Affine, clip: &[Point]) -> Option<Vec<FloatNode>> {
        if !self.gpu_paintable() || !transform.as_coeffs().iter().all(|v| v.is_finite()) {
            return None;
        }
        let tree: &usvg::Tree = &self.tree;
        with_stack(self.depth, || {
            let mut nodes = children(tree.root(), transform)?;
            if !clip.is_empty() {
                let mut outline = BezPath::new();
                for (i, p) in clip.iter().enumerate() {
                    if i == 0 {
                        outline.move_to(*p);
                    } else {
                        outline.line_to(*p);
                    }
                }
                outline.close_path();
                nodes = vec![FloatNode::Group(FloatGroup {
                    children: nodes,
                    opacity: 1.0,
                    blend: usvg::BlendMode::Normal,
                    clip: Some(vec![FloatNode::Draw(FloatDraw {
                        outline,
                        paint: FloatPaint::Solid([1.0; 4]),
                    })]),
                    mask: None,
                    mask_luminance: false,
                })];
            }
            Some(nodes)
        })
    }
}

fn children(group: &usvg::Group, transform: Affine) -> Option<Vec<FloatNode>> {
    let mut out = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Path(p) => path(p, transform, &mut out)?,
            usvg::Node::Group(g) => out.extend(group_node(g, transform)?),
            usvg::Node::Text(t) => out.extend(group_node(t.flattened(), transform)?),
            usvg::Node::Image(_) => return None,
        }
    }
    Some(out)
}

fn group_node(g: &usvg::Group, parent: Affine) -> Option<Vec<FloatNode>> {
    let t = parent * affine(g.transform());
    let children = children(g, t)?;
    if !g.should_isolate() {
        return Some(children);
    }
    let clip = if let Some(c) = g.clip_path() {
        Some(clip_path(c, t)?)
    } else {
        None
    };
    let (mask, mask_luminance) = if let Some(m) = g.mask() {
        (Some(mask(m, t)?), m.kind() == usvg::MaskType::Luminance)
    } else {
        (None, false)
    };
    Some(vec![FloatNode::Group(FloatGroup {
        children,
        opacity: g.opacity().get(),
        blend: g.blend_mode(),
        clip,
        mask,
        mask_luminance,
    })])
}

fn clip_path(c: &usvg::ClipPath, t: Affine) -> Option<Vec<FloatNode>> {
    // The clip's geometry is painted opaque white. Its own clip path is an
    // intersection, represented by a nested isolated group.
    let parent = t;
    let t = t * affine(c.transform());
    let children = clip_children(c.root(), t)?;
    if let Some(outer) = c.clip_path() {
        Some(vec![FloatNode::Group(FloatGroup {
            children,
            opacity: 1.0,
            blend: usvg::BlendMode::Normal,
            clip: Some(clip_path(outer, parent)?),
            mask: None,
            mask_luminance: false,
        })])
    } else {
        Some(children)
    }
}

fn clip_children(group: &usvg::Group, t: Affine) -> Option<Vec<FloatNode>> {
    let mut out = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Path(p) if p.is_visible() => {
                if let Some(f) = p.fill() {
                    let rule = f.rule();
                    let outline = outline(&bez(p.data()), t, rule);
                    out.push(FloatNode::Draw(FloatDraw {
                        outline,
                        paint: FloatPaint::Solid([1.0; 4]),
                    }));
                }
            }
            usvg::Node::Group(g) => {
                let gt = t * affine(g.transform());
                let mut nested = clip_children(g, gt)?;
                if let Some(c) = g.clip_path() {
                    nested = vec![FloatNode::Group(FloatGroup {
                        children: nested,
                        opacity: 1.0,
                        blend: usvg::BlendMode::Normal,
                        clip: Some(clip_path(c, gt)?),
                        mask: None,
                        mask_luminance: false,
                    })];
                }
                out.extend(nested);
            }
            usvg::Node::Text(text) => out.extend(clip_children(text.flattened(), t)?),
            _ => {}
        }
    }
    Some(out)
}

fn mask(m: &usvg::Mask, t: Affine) -> Option<Vec<FloatNode>> {
    let mut nodes = children(m.root(), t)?;
    let r = m.rect();
    let region = Rect::new(
        f64::from(r.left()),
        f64::from(r.top()),
        f64::from(r.right()),
        f64::from(r.bottom()),
    );
    nodes = vec![FloatNode::Group(FloatGroup {
        children: nodes,
        opacity: 1.0,
        blend: usvg::BlendMode::Normal,
        clip: Some(vec![FloatNode::Draw(FloatDraw {
            outline: outline(
                &region.to_path(FLATTEN_TOLERANCE),
                t,
                usvg::FillRule::NonZero,
            ),
            paint: FloatPaint::Solid([1.0; 4]),
        })]),
        mask: None,
        mask_luminance: false,
    })];
    if let Some(inner) = m.mask() {
        nodes = vec![FloatNode::Group(FloatGroup {
            children: nodes,
            opacity: 1.0,
            blend: usvg::BlendMode::Normal,
            clip: None,
            mask: Some(mask(inner, t)?),
            mask_luminance: inner.kind() == usvg::MaskType::Luminance,
        })];
    }
    Some(nodes)
}

fn path(p: &usvg::Path, t: Affine, out: &mut Vec<FloatNode>) -> Option<()> {
    if !p.is_visible() {
        return Some(());
    }
    let fill = |out: &mut Vec<FloatNode>| -> Option<()> {
        let Some(f) = p.fill() else {
            return Some(());
        };
        let b = p.data().bounds();
        if b.width() == 0.0 || b.height() == 0.0 {
            return Some(());
        }
        let paint = paint(f.paint(), f.opacity().get(), t)?;
        out.push(FloatNode::Draw(FloatDraw {
            outline: outline(&bez(p.data()), t, f.rule()),
            paint,
        }));
        Some(())
    };
    let stroke = |out: &mut Vec<FloatNode>| -> Option<()> {
        let Some(s) = p.stroke() else {
            return Some(());
        };
        let paint = paint(s.paint(), s.opacity().get(), t)?;
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
            style = style.with_dashes(
                f64::from(s.dashoffset()),
                dashes.iter().map(|d| f64::from(*d)),
            );
        }
        let c = t.as_coeffs();
        let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3])).max(1e-9);
        let expanded = kurbo::stroke(
            bez(p.data()).iter(),
            &style,
            &kurbo::StrokeOpts::default(),
            FLATTEN_TOLERANCE / scale / 4.0,
        );
        out.push(FloatNode::Draw(FloatDraw {
            outline: outline(&expanded, t, usvg::FillRule::NonZero),
            paint,
        }));
        Some(())
    };
    match p.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            fill(out)?;
            stroke(out)?;
        }
        usvg::PaintOrder::StrokeAndFill => {
            stroke(out)?;
            fill(out)?;
        }
    }
    Some(())
}

fn paint(p: &usvg::Paint, opacity: f32, t: Affine) -> Option<FloatPaint> {
    let solid = |c: usvg::Color| {
        FloatPaint::Solid([
            f32::from(c.red) / 255.0,
            f32::from(c.green) / 255.0,
            f32::from(c.blue) / 255.0,
            opacity,
        ])
    };
    let gradient = |base: &usvg::BaseGradient, geometry: FloatGradient| FloatPaint::Gradient {
        geometry,
        spread: base.spread_method(),
        opacity,
        stops: base
            .stops()
            .iter()
            .map(|s| FloatStop {
                offset: s.offset().get(),
                rgba: [
                    f32::from(s.color().red) / 255.0,
                    f32::from(s.color().green) / 255.0,
                    f32::from(s.color().blue) / 255.0,
                    s.opacity().get(),
                ],
            })
            .collect(),
    };
    Some(match p {
        usvg::Paint::Color(c) => solid(*c),
        usvg::Paint::LinearGradient(g) => {
            let inv = (t * affine(g.transform())).inverse();
            if !inv.as_coeffs().iter().all(|x| x.is_finite()) {
                return None;
            }
            gradient(
                g,
                FloatGradient::Linear {
                    p0: [g.x1(), g.y1()],
                    p1: [g.x2(), g.y2()],
                    inverse: inv.as_coeffs().map(|x| x as f32),
                },
            )
        }
        usvg::Paint::RadialGradient(g) => {
            let inv = (t * affine(g.transform())).inverse();
            if !inv.as_coeffs().iter().all(|x| x.is_finite()) {
                return None;
            }
            gradient(
                g,
                FloatGradient::Radial {
                    focus: [g.fx(), g.fy()],
                    center: [g.cx(), g.cy()],
                    radius: g.r().get(),
                    focus_radius: g.fr().get(),
                    inverse: inv.as_coeffs().map(|x| x as f32),
                },
            )
        }
        usvg::Paint::Pattern(_) => return None,
    })
}

fn outline(path: &BezPath, transform: Affine, rule: usvg::FillRule) -> BezPath {
    use crate::render::scene::{CoverageItem, FillRule, ItemStyle};
    CoverageItem {
        path: path.clone(),
        transform,
        style: ItemStyle::Fill(match rule {
            usvg::FillRule::NonZero => FillRule::NonZero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        }),
        color: [1.0; 4],
    }
    .device_outline()
}
