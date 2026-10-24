//! An SVG document as a Vello scene (feature `gpu`; docs/engine/vector.md,
//! "SVG images"): the usvg tree painted as resvg paints it, at output
//! resolution with analytic area coverage.
//!
//! - Paths: fill then stroke (or `paint-order`), solid colors and linear or
//!   radial gradients (`spreadMethod` as the extend mode, the gradient
//!   transform as the brush transform), interpolated in sRGB with straight
//!   alpha as Skia and resvg interpolate SVG gradients.
//! - Groups that isolate (opacity, blend mode, clip path, mask,
//!   `isolation`) are layers over their layer bounds; clip paths and masks
//!   are luminance masks applied to the group's content before its opacity
//!   and blend (clip children are opaque white fills with their clip rule,
//!   an alpha mask is its content painted white).
//! - Text is its usvg outlines.
//!
//! Documents with anything else are not [`SvgDocument::gpu_paintable`].

use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Stroke};
use vello::Scene;
use vello::peniko::color::{AlphaColor, Srgb};
use vello::peniko::{
    BlendMode, Brush, ColorStop, Compose, Extend, Fill, Gradient, InterpolationAlphaSpace, Mix,
};

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

    /// The brush of `paint` at `opacity`, and its transform (user space of
    /// the path to the gradient's space). `None` for patterns.
    fn brush(&self, paint: &usvg::Paint, opacity: f32) -> Option<(Brush, Option<Affine>)> {
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
                Some(affine(g.transform())),
            )
        };
        Some(match paint {
            usvg::Paint::Color(c) => (Brush::Solid(self.color(*c, opacity)), None),
            usvg::Paint::LinearGradient(g) => base(
                g,
                Gradient::new_linear(
                    (f64::from(g.x1()), f64::from(g.y1())),
                    (f64::from(g.x2()), f64::from(g.y2())),
                ),
            ),
            usvg::Paint::RadialGradient(g) => base(
                g,
                Gradient::new_two_point_radial(
                    (f64::from(g.fx()), f64::from(g.fy())),
                    g.fr().get(),
                    (f64::from(g.cx()), f64::from(g.cy())),
                    g.r().get(),
                ),
            ),
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
        self.scene.push_layer(
            Fill::NonZero,
            BlendMode::new(mix(g.blend_mode()), Compose::SrcOver),
            g.opacity().get(),
            Affine::IDENTITY,
            &bounds,
        );
        self.children(g, t);
        if let Some(c) = g.clip_path() {
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
        let Some((brush, brush_transform)) = self.brush(f.paint(), f.opacity().get()) else {
            return;
        };
        let rule = match f.rule() {
            usvg::FillRule::NonZero => Fill::NonZero,
            usvg::FillRule::EvenOdd => Fill::EvenOdd,
        };
        self.scene
            .fill(rule, t, &brush, brush_transform, &bez(p.data()));
    }

    fn stroke(&mut self, p: &usvg::Path, t: Affine) {
        let Some(s) = p.stroke() else { return };
        let Some((brush, brush_transform)) = self.brush(s.paint(), s.opacity().get()) else {
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
        self.scene
            .stroke(&style, t, &brush, brush_transform, &bez(p.data()));
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
