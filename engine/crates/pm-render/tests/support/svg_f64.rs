//! Independent f64 SVG paint for the subset whose geometry and paint can be
//! read directly from usvg. No native float scene or SVG painter is used.
//! A bounded fine-cell grid integrates gradients and joint path/clip coverage;
//! each cell's geometric area comes from pm-reference's f64 path integrator.

use kurbo::{Affine, BezPath, Cap, Join, Point, Stroke};
use pm_reference::composite::Rgba;
use pm_reference::coverage::{FillRule, fill};

const RATE: usize = 16;
const TILE: usize = 32;

struct Shape {
    outline: BezPath,
    rule: FillRule,
    paint: Paint,
}

enum Node {
    Group(Group),
    Shape(Vec<Shape>),
}

struct Group {
    children: Vec<Node>,
    opacity: f64,
    clip: Option<Clip>,
}

struct Clip {
    outlines: Vec<(BezPath, FillRule)>,
    parent: Option<Box<Clip>>,
}

enum Paint {
    Solid(Rgba),
    Linear {
        from_device: Affine,
        p0: Point,
        delta: kurbo::Vec2,
        stops: Vec<(f64, Rgba)>,
        spread: usvg::SpreadMethod,
    },
    Radial {
        from_device: Affine,
        focal: Point,
        center: Point,
        fr: f64,
        radius: f64,
        stops: Vec<(f64, Rgba)>,
        spread: usvg::SpreadMethod,
    },
}

fn affine(t: usvg::Transform) -> Affine {
    Affine::new([
        f64::from(t.sx),
        f64::from(t.ky),
        f64::from(t.kx),
        f64::from(t.sy),
        f64::from(t.tx),
        f64::from(t.ty),
    ])
}

fn path(data: &tiny_skia::Path) -> BezPath {
    use tiny_skia::PathSegment;
    let mut out = BezPath::new();
    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(p) => out.move_to((f64::from(p.x), f64::from(p.y))),
            PathSegment::LineTo(p) => out.line_to((f64::from(p.x), f64::from(p.y))),
            PathSegment::QuadTo(c, p) => out.quad_to(
                (f64::from(c.x), f64::from(c.y)),
                (f64::from(p.x), f64::from(p.y)),
            ),
            PathSegment::CubicTo(a, b, p) => out.curve_to(
                (f64::from(a.x), f64::from(a.y)),
                (f64::from(b.x), f64::from(b.y)),
                (f64::from(p.x), f64::from(p.y)),
            ),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

fn rule(rule: usvg::FillRule) -> FillRule {
    match rule {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn stops(gradient: &usvg::BaseGradient, opacity: f64) -> Vec<(f64, Rgba)> {
    gradient
        .stops()
        .iter()
        .map(|stop| {
            let c = stop.color();
            (
                f64::from(stop.offset().get()),
                [
                    f64::from(c.red) / 255.0,
                    f64::from(c.green) / 255.0,
                    f64::from(c.blue) / 255.0,
                    f64::from(stop.opacity().get()) * opacity,
                ],
            )
        })
        .collect()
}

fn resolve_paint(paint: &usvg::Paint, opacity: f64, path_to_device: Affine) -> Option<Paint> {
    let invert = |transform: Affine| {
        (transform.as_coeffs()[0] * transform.as_coeffs()[3]
            - transform.as_coeffs()[1] * transform.as_coeffs()[2])
            .abs()
            .gt(&1e-15)
            .then(|| transform.inverse())
    };
    Some(match paint {
        usvg::Paint::Color(c) => Paint::Solid([
            f64::from(c.red) / 255.0 * opacity,
            f64::from(c.green) / 255.0 * opacity,
            f64::from(c.blue) / 255.0 * opacity,
            opacity,
        ]),
        usvg::Paint::LinearGradient(g) => Paint::Linear {
            from_device: invert(path_to_device * affine(g.transform()))?,
            p0: Point::new(f64::from(g.x1()), f64::from(g.y1())),
            delta: kurbo::Vec2::new(f64::from(g.x2() - g.x1()), f64::from(g.y2() - g.y1())),
            stops: stops(g, opacity),
            spread: g.spread_method(),
        },
        usvg::Paint::RadialGradient(g) => Paint::Radial {
            from_device: invert(path_to_device * affine(g.transform()))?,
            focal: Point::new(f64::from(g.fx()), f64::from(g.fy())),
            center: Point::new(f64::from(g.cx()), f64::from(g.cy())),
            fr: f64::from(g.fr().get()),
            radius: f64::from(g.r().get()),
            stops: stops(g, opacity),
            spread: g.spread_method(),
        },
        usvg::Paint::Pattern(_) => return None,
    })
}

fn shape(path_node: &usvg::Path, svg_to_device: Affine) -> Option<Node> {
    if !path_node.is_visible() {
        return Some(Node::Shape(Vec::new()));
    }
    let local = path(path_node.data());
    let to_device = svg_to_device * affine(path_node.abs_transform());
    let mut shapes = Vec::new();
    let add_fill = || -> Option<Option<Shape>> {
        if let Some(fill_style) = path_node.fill() {
            return Some(Some(Shape {
                outline: to_device * local.clone(),
                rule: rule(fill_style.rule()),
                paint: resolve_paint(
                    fill_style.paint(),
                    f64::from(fill_style.opacity().get()),
                    to_device,
                )?,
            }));
        }
        Some(None)
    };
    let add_stroke = || -> Option<Option<Shape>> {
        if let Some(s) = path_node.stroke() {
            let cap = match s.linecap() {
                usvg::LineCap::Butt => Cap::Butt,
                usvg::LineCap::Round => Cap::Round,
                usvg::LineCap::Square => Cap::Square,
            };
            let join = match s.linejoin() {
                usvg::LineJoin::Miter => Join::Miter,
                usvg::LineJoin::Round => Join::Round,
                usvg::LineJoin::Bevel => Join::Bevel,
                usvg::LineJoin::MiterClip => return None,
            };
            let mut stroke = Stroke::new(f64::from(s.width().get()));
            stroke.start_cap = cap;
            stroke.end_cap = cap;
            stroke.join = join;
            stroke.miter_limit = f64::from(s.miterlimit().get());
            stroke.dash_pattern = s
                .dasharray()
                .unwrap_or(&[])
                .iter()
                .map(|&v| f64::from(v))
                .collect();
            stroke.dash_offset = f64::from(s.dashoffset());
            let region = pm_reference::stroke::outline(&local, &stroke, 1e-5)?;
            return Some(Some(Shape {
                outline: to_device * region.path,
                rule: FillRule::NonZero,
                paint: resolve_paint(s.paint(), f64::from(s.opacity().get()), to_device)?,
            }));
        }
        Some(None)
    };
    match path_node.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            shapes.extend(add_fill()?);
            shapes.extend(add_stroke()?);
        }
        usvg::PaintOrder::StrokeAndFill => {
            shapes.extend(add_stroke()?);
            shapes.extend(add_fill()?);
        }
    }
    Some(Node::Shape(shapes))
}

fn build_clip(clip: &usvg::ClipPath, group_to_device: Affine) -> Option<Clip> {
    let base = group_to_device * affine(clip.transform());
    fn collect(
        group: &usvg::Group,
        base: Affine,
        out: &mut Vec<(BezPath, FillRule)>,
    ) -> Option<()> {
        for child in group.children() {
            match child {
                usvg::Node::Path(p) if p.is_visible() => {
                    let style = p.fill()?;
                    out.push((
                        base * affine(p.abs_transform()) * path(p.data()),
                        rule(style.rule()),
                    ));
                }
                usvg::Node::Group(g) if g.clip_path().is_none() => collect(g, base, out)?,
                _ => return None,
            }
        }
        Some(())
    }
    let mut outlines = Vec::new();
    collect(clip.root(), base, &mut outlines)?;
    Some(Clip {
        outlines,
        parent: clip
            .clip_path()
            .map(|other| build_clip(other, group_to_device))
            .transpose_option()?
            .map(Box::new),
    })
}

trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            Some(Some(value)) => Some(Some(value)),
            Some(None) => None,
            None => Some(None),
        }
    }
}

fn build_group(group: &usvg::Group, svg_to_device: Affine) -> Option<Group> {
    if group.blend_mode() != usvg::BlendMode::Normal
        || group.mask().is_some()
        || !group.filters().is_empty()
    {
        return None;
    }
    let clip = group
        .clip_path()
        .map(|c| build_clip(c, svg_to_device * affine(group.abs_transform())))
        .transpose_option()?;
    let mut children = Vec::new();
    for child in group.children() {
        children.push(match child {
            usvg::Node::Group(g) => Node::Group(build_group(g, svg_to_device)?),
            usvg::Node::Path(p) => shape(p, svg_to_device)?,
            _ => return None,
        });
    }
    Some(Group {
        children,
        opacity: f64::from(group.opacity().get()),
        clip,
    })
}

fn spread(t: f64, mode: usvg::SpreadMethod) -> f64 {
    match mode {
        usvg::SpreadMethod::Pad => t.clamp(0.0, 1.0),
        usvg::SpreadMethod::Repeat => t.rem_euclid(1.0),
        usvg::SpreadMethod::Reflect => {
            let u = t.rem_euclid(2.0);
            if u <= 1.0 { u } else { 2.0 - u }
        }
    }
}

fn interpolate(stops: &[(f64, Rgba)], t: f64, mode: usvg::SpreadMethod) -> Rgba {
    let t = spread(t, mode);
    let first = stops.first().map_or([0.0; 4], |stop| stop.1);
    if t <= stops.first().map_or(0.0, |stop| stop.0) {
        return first;
    }
    for pair in stops.windows(2) {
        if t <= pair[1].0 {
            let u = if pair[1].0 > pair[0].0 {
                (t - pair[0].0) / (pair[1].0 - pair[0].0)
            } else {
                1.0
            };
            return std::array::from_fn(|channel| {
                pair[0].1[channel] * (1.0 - u) + pair[1].1[channel] * u
            });
        }
    }
    stops.last().map_or(first, |stop| stop.1)
}

impl Paint {
    fn at(&self, device: Point) -> Rgba {
        match self {
            Self::Solid(c) => *c,
            Self::Linear {
                from_device,
                p0,
                delta,
                stops,
                spread,
            } => {
                let p = *from_device * device - *p0;
                let t = p.dot(*delta) / delta.hypot2();
                let mut c = interpolate(stops, t, *spread);
                let alpha = c[3];
                for value in &mut c[..3] {
                    *value *= alpha;
                }
                c
            }
            Self::Radial {
                from_device,
                focal,
                center,
                fr,
                radius,
                stops,
                spread,
            } => {
                let p = *from_device * device - *focal;
                let dc = *center - *focal;
                let dr = radius - fr;
                let a = dc.hypot2() - dr * dr;
                let b = -2.0 * (p.dot(dc) + fr * dr);
                let c0 = p.hypot2() - fr * fr;
                let t = if a.abs() < 1e-15 {
                    if b.abs() < 1e-15 { 0.0 } else { -c0 / b }
                } else {
                    let disc = (b * b - 4.0 * a * c0).max(0.0).sqrt();
                    ((-b + disc) / (2.0 * a)).max((-b - disc) / (2.0 * a))
                };
                let mut c = interpolate(stops, t, *spread);
                let alpha = c[3];
                for value in &mut c[..3] {
                    *value *= alpha;
                }
                c
            }
        }
    }
}

fn over(src: Rgba, dst: &mut Rgba) {
    for channel in 0..3 {
        dst[channel] = src[channel] + dst[channel] * (1.0 - src[3]);
    }
    dst[3] = src[3] + dst[3] * (1.0 - src[3]);
}

fn coverage(
    outline: &BezPath,
    rule: FillRule,
    x0: usize,
    y0: usize,
    fw: usize,
    fh: usize,
) -> pm_reference::coverage::Coverage {
    let local = Affine::scale(RATE as f64)
        * Affine::translate((-(x0 as f64), -(y0 as f64)))
        * outline.clone();
    fill(&local, rule, 1, fw, fh)
}

fn clip_mask(clip: &Clip, x0: usize, y0: usize, fw: usize, fh: usize) -> Vec<f64> {
    let mut mask = vec![0.0_f64; fw * fh];
    for (outline, rule) in &clip.outlines {
        let area = coverage(outline, *rule, x0, y0, fw, fh);
        for y in area.y0..area.y0 + area.height {
            for x in area.x0..area.x0 + area.width {
                let i = y * fw + x;
                mask[i] = mask[i].max(area.at(x, y));
            }
        }
    }
    if let Some(parent) = &clip.parent {
        let other = clip_mask(parent, x0, y0, fw, fh);
        for (m, p) in mask.iter_mut().zip(other) {
            *m *= p;
        }
    }
    mask
}

fn draw(group: &Group, x0: usize, y0: usize, fw: usize, fh: usize, dst: &mut [Rgba]) {
    let mut layer = vec![[0.0; 4]; fw * fh];
    for child in &group.children {
        match child {
            Node::Group(child) => draw(child, x0, y0, fw, fh, &mut layer),
            Node::Shape(shapes) => {
                for shape in shapes {
                    let area = coverage(&shape.outline, shape.rule, x0, y0, fw, fh);
                    for y in area.y0..area.y0 + area.height {
                        for x in area.x0..area.x0 + area.width {
                            let cover = area.at(x, y);
                            if cover <= 0.0 {
                                continue;
                            }
                            let device = Point::new(
                                x0 as f64 + (x as f64 + 0.5) / RATE as f64,
                                y0 as f64 + (y as f64 + 0.5) / RATE as f64,
                            );
                            let mut color = shape.paint.at(device);
                            for channel in &mut color {
                                *channel *= cover;
                            }
                            over(color, &mut layer[y * fw + x]);
                        }
                    }
                }
            }
        }
    }
    let mask = group.clip.as_ref().map(|c| clip_mask(c, x0, y0, fw, fh));
    for (i, color) in layer.into_iter().enumerate() {
        let alpha = group.opacity * mask.as_ref().map_or(1.0, |m| m[i]);
        let src = color.map(|value| value * alpha);
        over(src, &mut dst[i]);
    }
}

/// Paint a complete supported SVG tree in f64. Unsupported SVG constructs
/// return None so the caller can report/use its independent resvg fallback.
pub fn paint(
    tree: &usvg::Tree,
    svg_to_device: Affine,
    crop: &[Point; 4],
    bounds: (usize, usize, usize, usize),
    width: usize,
    height: usize,
) -> Option<Vec<(Rgba, bool)>> {
    let scene = build_group(tree.root(), svg_to_device)?;
    let mut out = vec![([0.0; 4], false); width * height];
    let (left, top, right, bottom) = bounds;
    let mut crop_path = BezPath::new();
    crop_path.move_to(crop[0]);
    for point in &crop[1..] {
        crop_path.line_to(*point);
    }
    crop_path.close_path();
    for y0 in (top..bottom).step_by(TILE) {
        for x0 in (left..right).step_by(TILE) {
            let tw = (right - x0).min(TILE);
            let th = (bottom - y0).min(TILE);
            let (fw, fh) = (tw * RATE, th * RATE);
            let mut fine = vec![[0.0; 4]; fw * fh];
            draw(&scene, x0, y0, fw, fh, &mut fine);
            let crop_area = coverage(&crop_path, FillRule::NonZero, x0, y0, fw, fh);
            for y in 0..th {
                for x in 0..tw {
                    let mut sum = [0.0_f64; 4];
                    let mut lo = [f64::INFINITY; 4];
                    let mut hi = [f64::NEG_INFINITY; 4];
                    for sy in 0..RATE {
                        for sx in 0..RATE {
                            let fx = x * RATE + sx;
                            let fy = y * RATE + sy;
                            let mask = crop_area.at(fx, fy);
                            let color = fine[fy * fw + fx].map(|v| v * mask);
                            for channel in 0..4 {
                                sum[channel] += color[channel] / (RATE * RATE) as f64;
                                lo[channel] = lo[channel].min(color[channel]);
                                hi[channel] = hi[channel].max(color[channel]);
                            }
                        }
                    }
                    let edge = (sum[3] > 1e-8 && sum[3] < 1.0 - 1e-8)
                        || (0..4).any(|channel| (hi[channel] - lo[channel]) * 255.0 > 2.0);
                    out[(y0 + y) * width + x0 + x] = (sum, edge);
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn independent_gradient_clip_and_group_opacity() {
        let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
          <defs>
            <linearGradient id="a" x1="0" y1="0" x2="1" y2="0">
              <stop stop-color="#123456"/><stop offset="1" stop-color="#C89650"/>
            </linearGradient>
            <radialGradient id="b" cx="0.5" cy="0.5" r="0.5">
              <stop stop-color="#FFE0A0"/><stop offset="1" stop-color="#A04020" stop-opacity="0"/>
            </radialGradient>
            <clipPath id="c"><rect x="8" y="8" width="48" height="48"/></clipPath>
          </defs>
          <g opacity="0.6" clip-path="url(#c)">
            <rect width="64" height="64" fill="url(#a)"/>
            <circle cx="32" cy="32" r="16" fill="url(#b)"/>
          </g>
        </svg>"##;
        let tree = usvg::Tree::from_data(source, &usvg::Options::default()).unwrap();
        let crop = [
            kurbo::Point::new(0.0, 0.0),
            kurbo::Point::new(64.0, 0.0),
            kurbo::Point::new(64.0, 64.0),
            kurbo::Point::new(0.0, 64.0),
        ];
        let pixels = super::paint(
            &tree,
            kurbo::Affine::IDENTITY,
            &crop,
            (0, 0, 64, 64),
            64,
            64,
        )
        .unwrap();
        assert_eq!(pixels[4 * 64 + 4].0, [0.0; 4], "clip excludes its exterior");
        let actual = pixels[30 * 64 + 30].0;
        let mut expected = [0.0; 4];
        const N: usize = 512;
        for sy in 0..N {
            for sx in 0..N {
                let x = 30.0 + (sx as f64 + 0.5) / N as f64;
                let y = 30.0 + (sy as f64 + 0.5) / N as f64;
                let t = x / 64.0;
                let base = [18.0 + 182.0 * t, 52.0 + 98.0 * t, 86.0 - 6.0 * t];
                let d = (x - 32.0).hypot(y - 32.0) / 16.0;
                let sun = [255.0 - 95.0 * d, 224.0 - 160.0 * d, 160.0 - 128.0 * d];
                let alpha = 1.0 - d;
                for channel in 0..3 {
                    expected[channel] += (sun[channel] * alpha + base[channel] * (1.0 - alpha))
                        * 0.6
                        / (255.0 * (N * N) as f64);
                }
                expected[3] += 0.6 / (N * N) as f64;
            }
        }
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() * 255.0 < 0.01,
                "channel {channel}: actual {:?}, independent 512x {:?}",
                actual,
                expected
            );
        }
    }
}
