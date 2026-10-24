//! An SVG document as a Vello scene (feature `gpu`; docs/engine/vector.md,
//! "SVG images"): the usvg tree painted as resvg paints it, at output
//! resolution with analytic area coverage.
//!
//! - Paths: fill then stroke (or `paint-order`), solid colors and linear or
//!   radial gradients (`spreadMethod` as the extend mode, the gradient
//!   transform as the brush transform), interpolated in sRGB with straight
//!   alpha as Skia and resvg interpolate SVG gradients.
//! - Groups that isolate (opacity, blend mode, clip path, mask,
//!   `isolation`) are layers over their layer bounds; a clip path of one
//!   plain path is the layer's clip shape, other clip paths and masks are
//!   luminance masks applied to the group's content before its opacity and
//!   blend (clip children are opaque white fills with their clip rule, an
//!   alpha mask is its content painted white).
//! - Text is its usvg outlines.
//!
//! Documents with anything else are not [`SvgDocument::gpu_paintable`].

use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Stroke};
use vello::Scene;
use vello::peniko::color::{AlphaColor, Srgb};
use vello::peniko::{
    BlendMode, Brush, ColorStop, Compose, Extend, Fill, Gradient, InterpolationAlphaSpace, Mix,
};

use super::scene::FLATTEN_TOLERANCE;
use super::svg::{SvgDocument, affine, bez};
use super::svg_depth::with_stack;

impl SvgDocument {
    /// Appends the document under `transform` (user units to device
    /// pixels), clipped to the device polygon `clip` (none when empty), to
    /// `scene`. Only meaningful for [`Self::gpu_paintable`] documents: other
    /// nodes are skipped.
    pub fn encode(&self, scene: &mut Scene, transform: Affine, clip: &[Point]) {
        let tree: &usvg::Tree = &self.tree;
        with_stack(self.depth, move || {
            let mut e = Encoder { scene, white: false };
            if clip.is_empty() {
                e.children(tree.root(), transform);
                return;
            }
            let mut outline = BezPath::new();
            for (i, p) in clip.iter().enumerate() {
                if i == 0 {
                    outline.move_to(*p);
                } else {
                    outline.line_to(*p);
                }
            }
            outline.close_path();
            e.scene
                .push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &outline);
            e.children(tree.root(), transform);
            e.scene.pop_layer();
        });
    }
}

/// The convex polygon `poly` clipped to `rect` (Sutherland–Hodgman).
pub(crate) fn clip_to_rect(poly: &[Point], rect: Rect) -> Vec<Point> {
    let mut out: Vec<Point> = poly.to_vec();
    let edges: [(fn(Point, Rect) -> f64, fn(Point, Point, Rect) -> Point); 4] = [
        (|p, r| p.x - r.x0, |a, b, r| lerp(a, b, (r.x0 - a.x) / (b.x - a.x))),
        (|p, r| r.x1 - p.x, |a, b, r| lerp(a, b, (r.x1 - a.x) / (b.x - a.x))),
        (|p, r| p.y - r.y0, |a, b, r| lerp(a, b, (r.y0 - a.y) / (b.y - a.y))),
        (|p, r| r.y1 - p.y, |a, b, r| lerp(a, b, (r.y1 - a.y) / (b.y - a.y))),
    ];
    fn lerp(a: Point, b: Point, t: f64) -> Point {
        a + (b - a) * t
    }
    for (inside, cross) in edges {
        let input = core::mem::take(&mut out);
        for (i, &b) in input.iter().enumerate() {
            let a = input[(i + input.len() - 1) % input.len()];
            let (ia, ib) = (inside(a, rect) >= 0.0, inside(b, rect) >= 0.0);
            if ib {
                if !ia {
                    out.push(cross(a, b, rect));
                }
                out.push(b);
            } else if ia {
                out.push(cross(a, b, rect));
            }
        }
    }
    out
}

/// `path` under `t`, flattened to [`FLATTEN_TOLERANCE`] in device pixels:
/// Vello flattens curves to a quarter pixel itself, which erodes curved
/// edges (`crate::render::scene::CoverageItem::device_outline`).
fn device(path: &BezPath, t: Affine) -> BezPath {
    let mut out = BezPath::new();
    kurbo::flatten((t * path.clone()).iter(), FLATTEN_TOLERANCE, |el| out.push(el));
    out
}

/// `flat` (line segments) filled with `rule`, as disjoint contours: area
/// coverage sums the winding over a pixel before the fill rule, so where
/// overlapping pieces (a stroke's inner join, crossing strokes, a
/// self-intersecting fill) meet an edge inside one pixel it covers too
/// much. The result fills the same with either rule.
fn resolve(flat: &BezPath, rule: Fill) -> BezPath {
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
        Fill::NonZero => Rule::NonZero,
        Fill::EvenOdd => Rule::EvenOdd,
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

struct Encoder<'a> {
    scene: &'a mut Scene,
    /// Painting the content of a clip path or an alpha mask: every paint is
    /// white with its own alpha, so the luminance mask is the coverage.
    white: bool,
}

fn rect(r: usvg::NonZeroRect) -> Rect {
    Rect::new(
        f64::from(r.left()),
        f64::from(r.top()),
        f64::from(r.right()),
        f64::from(r.bottom()),
    )
}

fn mix(mode: usvg::BlendMode) -> Mix {
    use usvg::BlendMode as B;
    match mode {
        B::Normal => Mix::Normal,
        B::Multiply => Mix::Multiply,
        B::Screen => Mix::Screen,
        B::Overlay => Mix::Overlay,
        B::Darken => Mix::Darken,
        B::Lighten => Mix::Lighten,
        B::ColorDodge => Mix::ColorDodge,
        B::ColorBurn => Mix::ColorBurn,
        B::HardLight => Mix::HardLight,
        B::SoftLight => Mix::SoftLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
    }
}

/// The one fill path `c` clips to, with its clip rule, when that is all it
/// is: no clip path of its own, one visible filled path and nothing else
/// (resvg draws exactly that path's fill as the clip's coverage).
fn simple_clip(c: &usvg::ClipPath) -> Option<(&usvg::Path, Fill)> {
    if c.clip_path().is_some() {
        return None;
    }
    let [usvg::Node::Path(p)] = c.root().children() else {
        return None;
    };
    let f = p.fill()?;
    let b = p.data().bounds();
    if !p.is_visible() || b.width() == 0.0 || b.height() == 0.0 {
        return None;
    }
    Some((
        p,
        match f.rule() {
            usvg::FillRule::NonZero => Fill::NonZero,
            usvg::FillRule::EvenOdd => Fill::EvenOdd,
        },
    ))
}

impl Encoder<'_> {
    fn color(&self, c: usvg::Color, alpha: f32) -> AlphaColor<Srgb> {
        if self.white {
            return AlphaColor::new([1.0, 1.0, 1.0, alpha]);
        }
        AlphaColor::new([
            f32::from(c.red) / 255.0,
            f32::from(c.green) / 255.0,
            f32::from(c.blue) / 255.0,
            alpha,
        ])
    }

    /// The brush of `paint` at `opacity` for a path drawn under `t`, and
    /// its transform (device pixels to the gradient's space; paths are
    /// encoded in device pixels). `None` for patterns, or a singular
    /// transform.
    fn brush(&self, paint: &usvg::Paint, opacity: f32, t: Affine) -> Option<(Brush, Option<Affine>)> {
        let base = |g: &usvg::BaseGradient, gradient: Gradient| {
            let stops: Vec<ColorStop> = g
                .stops()
                .iter()
                .map(|s| ColorStop::from((s.offset().get(), self.color(s.color(), s.opacity().get() * opacity))))
                .collect();
            let extend = match g.spread_method() {
                usvg::SpreadMethod::Pad => Extend::Pad,
                usvg::SpreadMethod::Reflect => Extend::Reflect,
                usvg::SpreadMethod::Repeat => Extend::Repeat,
            };
            (
                Brush::Gradient(
                    gradient
                        .with_extend(extend)
                        .with_interpolation_alpha_space(InterpolationAlphaSpace::Unpremultiplied)
                        .with_stops(stops.as_slice()),
                ),
                Some(t * affine(g.transform())),
            )
        };
        Some(match paint {
            usvg::Paint::Color(c) => (Brush::Solid(self.color(*c, opacity)), None),
            usvg::Paint::LinearGradient(g) => {
                // Vello maps a linear gradient's end points through the
                // transform, which keeps the gradient's lines perpendicular
                // to it: wrong under a non-uniform scale or a skew (an
                // `objectBoundingBox` gradient, a stretched image). The
                // gradient is placed in device space instead: t ∘ G maps
                // gradient space to device pixels, the position along the
                // gradient is affine in the device point, and the line it
                // varies along is G's direction under the inverse transpose.
                let f = t * affine(g.transform());
                let (p1, p2) = (
                    kurbo::Point::new(f64::from(g.x1()), f64::from(g.y1())),
                    kurbo::Point::new(f64::from(g.x2()), f64::from(g.y2())),
                );
                let d = p2 - p1;
                let [a, b, c, e, _, _] = f.as_coeffs();
                let det = a * e - b * c;
                if det.abs() < 1e-12 || d.hypot2() == 0.0 {
                    return None;
                }
                // (F⁻¹)ᵀ d / |d|²: the device gradient of the position.
                let grad = kurbo::Vec2::new(e * d.x - b * d.y, -c * d.x + a * d.y) / (det * d.hypot2());
                // Vello evaluates gradients at pixel corners: half a pixel
                // back, they are evaluated at pixel centers.
                let q1 = f * p1 - kurbo::Vec2::new(0.5, 0.5);
                let q2 = q1 + grad / grad.hypot2();
                let (brush, _) = base(g, Gradient::new_linear(q1, q2));
                (brush, None)
            }
            usvg::Paint::RadialGradient(g) => {
                // Vello solves the two-point conical gradient in f32 and
                // nudges coincident centers by 1/4096 of a gradient unit:
                // in `objectBoundingBox` units (radius 0.5) that collapses
                // the middle of a plain radial gradient to its first stop.
                // The gradient is scaled to device pixels first.
                let f = t * affine(g.transform());
                let [a, b, c, d, _, _] = f.as_coeffs();
                let k = (a * d - b * c).abs().sqrt();
                if k < 1e-12 {
                    return None;
                }
                let (brush, _) = base(
                    g,
                    Gradient::new_two_point_radial(
                        (f64::from(g.fx()) * k, f64::from(g.fy()) * k),
                        g.fr().get() * k as f32,
                        (f64::from(g.cx()) * k, f64::from(g.cy()) * k),
                        g.r().get() * k as f32,
                    ),
                );
                // Evaluated at pixel centers (see the linear gradient).
                (brush, Some(Affine::translate((-0.5, -0.5)) * f * Affine::scale(1.0 / k)))
            }
            usvg::Paint::Pattern(_) => return None,
        })
    }

    fn children(&mut self, g: &usvg::Group, t: Affine) {
        for node in g.children() {
            match node {
                usvg::Node::Group(g) => self.group(g, t),
                usvg::Node::Path(p) => self.path(p, t),
                usvg::Node::Text(text) => self.group(text.flattened(), t),
                usvg::Node::Image(_) => {}
            }
        }
    }

    /// The device rectangle a layer of `g` (under `t`, its own transform
    /// included) needs: its layer bounds, grown by 2 px for antialiasing.
    fn layer_rect(g: &usvg::Group, t: Affine) -> Rect {
        t.transform_rect_bbox(rect(g.layer_bounding_box())).inflate(2.0, 2.0)
    }

    fn group(&mut self, g: &usvg::Group, t: Affine) {
        let t = t * affine(g.transform());
        if !g.should_isolate() {
            self.children(g, t);
            return;
        }
        let bounds = Self::layer_rect(g, t);
        let blend = BlendMode::new(mix(g.blend_mode()), Compose::SrcOver);
        // A clip path of one plain path is the layer's own clip shape (no
        // mask layer); any other is a luminance mask.
        let shape_clip = g.clip_path().and_then(|c| simple_clip(c).map(|(p, rule)| (c, p, rule)));
        match &shape_clip {
            Some((c, p, rule)) => self.scene.push_layer(
                Fill::NonZero,
                blend,
                g.opacity().get(),
                Affine::IDENTITY,
                &resolve(&device(&bez(p.data()), t * affine(c.transform())), *rule),
            ),
            None => self.scene.push_layer(
                Fill::NonZero,
                blend,
                g.opacity().get(),
                Affine::IDENTITY,
                &bounds,
            ),
        }
        self.children(g, t);
        if let (Some(c), None) = (g.clip_path(), &shape_clip) {
            self.clip(c, t, bounds);
        }
        if let Some(m) = g.mask() {
            self.mask(m, t, bounds);
        }
        self.scene.pop_layer();
    }

    /// Multiplies the open layer by the coverage of `c` (and of the clip
    /// paths it is clipped by). The mask layer spans the whole of `bounds`
    /// so every tile of the content is masked.
    fn clip(&mut self, c: &usvg::ClipPath, t: Affine, bounds: Rect) {
        self.scene
            .push_luminance_mask_layer(Fill::NonZero, 1.0, Affine::IDENTITY, &bounds);
        let white = core::mem::replace(&mut self.white, true);
        self.clip_children(c.root(), t * affine(c.transform()), bounds);
        self.white = white;
        self.scene.pop_layer();
        if let Some(c) = c.clip_path() {
            self.clip(c, t, bounds);
        }
    }

    /// A clip path's geometry: visible paths' fills with their clip rule;
    /// groups only contribute their transform and their own clip path.
    fn clip_children(&mut self, g: &usvg::Group, t: Affine, bounds: Rect) {
        for node in g.children() {
            match node {
                usvg::Node::Path(p) => {
                    if p.is_visible() {
                        self.fill(p, t);
                    }
                }
                usvg::Node::Text(text) => self.clip_children(text.flattened(), t, bounds),
                usvg::Node::Group(g) => {
                    let t = t * affine(g.transform());
                    if let Some(c) = g.clip_path() {
                        self.scene
                            .push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &bounds);
                        self.clip_children(g, t, bounds);
                        self.clip(c, t, bounds);
                        self.scene.pop_layer();
                    } else {
                        self.clip_children(g, t, bounds);
                    }
                }
                usvg::Node::Image(_) => {}
            }
        }
    }

    /// Multiplies the open layer by the mask `m` (luminance, or alpha for
    /// `mask-type: alpha`) inside its region, and by the masks it is masked
    /// by.
    fn mask(&mut self, m: &usvg::Mask, t: Affine, bounds: Rect) {
        if let Some(inner) = m.mask() {
            self.mask(inner, t, bounds);
        }
        self.scene
            .push_luminance_mask_layer(Fill::NonZero, 1.0, Affine::IDENTITY, &bounds);
        self.scene
            .push_layer(Fill::NonZero, Mix::Normal, 1.0, t, &rect(m.rect()));
        let alpha = self.white || m.kind() == usvg::MaskType::Alpha;
        let white = core::mem::replace(&mut self.white, alpha);
        self.children(m.root(), t);
        self.white = white;
        self.scene.pop_layer();
        self.scene.pop_layer();
    }

    fn fill(&mut self, p: &usvg::Path, t: Affine) {
        let Some(f) = p.fill() else { return };
        // Horizontal and vertical lines cannot be filled.
        let b = p.data().bounds();
        if b.width() == 0.0 || b.height() == 0.0 {
            return;
        }
        let Some((brush, brush_transform)) = self.brush(f.paint(), f.opacity().get(), t) else {
            return;
        };
        let rule = match f.rule() {
            usvg::FillRule::NonZero => Fill::NonZero,
            usvg::FillRule::EvenOdd => Fill::EvenOdd,
        };
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &brush,
            brush_transform,
            &resolve(&device(&bez(p.data()), t), rule),
        );
    }

    fn stroke(&mut self, p: &usvg::Path, t: Affine) {
        let Some(s) = p.stroke() else { return };
        let Some((brush, brush_transform)) = self.brush(s.paint(), s.opacity().get(), t) else {
            return;
        };
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
        // The outline is expanded here (kurbo, the stroke in user space)
        // finely enough to stay within the tolerance once transformed.
        let c = t.as_coeffs();
        let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3])).max(1e-9);
        let outline = kurbo::stroke(
            bez(p.data()).iter(),
            &style,
            &kurbo::StrokeOpts::default(),
            FLATTEN_TOLERANCE / scale / 4.0,
        );
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &brush,
            brush_transform,
            &resolve(&device(&outline, t), Fill::NonZero),
        );
    }

    fn path(&mut self, p: &usvg::Path, t: Affine) {
        if !p.is_visible() {
            return;
        }
        match p.paint_order() {
            usvg::PaintOrder::FillAndStroke => {
                self.fill(p, t);
                self.stroke(p, t);
            }
            usvg::PaintOrder::StrokeAndFill => {
                self.stroke(p, t);
                self.fill(p, t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rotated_clip_is_cut_to_its_cell() {
        let square = [
            Point::new(5.0, -5.0),
            Point::new(15.0, 5.0),
            Point::new(5.0, 15.0),
            Point::new(-5.0, 5.0),
        ];
        let cut = clip_to_rect(&square, Rect::new(0.0, 0.0, 10.0, 10.0));
        // The diamond within the cell: an octagon, every point inside.
        assert_eq!(cut.len(), 8);
        assert!(cut.iter().all(|p| (0.0..=10.0).contains(&p.x) && (0.0..=10.0).contains(&p.y)));
        let inside = clip_to_rect(&square, Rect::new(-10.0, -10.0, 20.0, 20.0));
        assert_eq!(inside, square.to_vec());
        assert!(clip_to_rect(&square, Rect::new(30.0, 30.0, 40.0, 40.0)).is_empty());
    }
}
