use crate::{Color, Error, Result, Source, path::finite};
use kurbo::{Affine, Point};
use std::sync::{Arc, RwLock};

/// Invalid CSS retains the previous paint. Unsupported color-space conversion is
/// explicit so a host can promote before drawing, rather than silently drop it.
pub(crate) fn parse_color(s: &str) -> Result<Option<Color>> {
    if let Some(c) = pm_vector::render::color::parse(s) {
        return Ok(Some(c));
    }
    use pm_text::render::color::{Rejected, classify};
    if s.trim().to_ascii_lowercase().starts_with("light-dark(") {
        return Err(Error::NeedsPromotion("style-source CSS color"));
    }
    match classify(s) {
        Ok(c) => {
            if !s.contains('(') {
                return Err(Error::NeedsPromotion("style-source CSS color"));
            }
            if c.iter().any(|v| !v.is_finite())
                || c[..3].iter().any(|v| *v < -1e-5 || *v > 1. + 1e-5)
            {
                return Err(Error::NeedsPromotion("wide-gamut color mapping"));
            }
            Ok(Some(Color {
                r: c[0].clamp(0., 1.) as f32,
                g: c[1].clamp(0., 1.) as f32,
                b: c[2].clamp(0., 1.) as f32,
                a: c[3].clamp(0., 1.) as f32,
            }))
        }
        Err(Rejected::Invalid) => Ok(None),
        Err(Rejected::Unsupported) => Err(Error::NeedsPromotion("CSS color")),
    }
}
#[derive(Debug, Clone)]
pub enum GradientKind {
    Linear(Point, Point),
    Radial {
        start: Point,
        r0: f64,
        end: Point,
        r1: f64,
    },
    Conic {
        angle: f64,
        center: Point,
    },
}
#[derive(Debug, Clone)]
struct GradientData {
    kind: GradientKind,
    stops: Vec<(f64, Color)>,
}
#[derive(Debug, Clone)]
pub struct Gradient(Arc<RwLock<GradientData>>);
impl Gradient {
    pub fn new(kind: GradientKind) -> Result<Self> {
        let valid = match kind {
            GradientKind::Linear(a, b) => a.is_finite() && b.is_finite(),
            GradientKind::Radial { start, r0, end, r1 } => {
                start.is_finite() && end.is_finite() && finite(&[r0, r1])
            }
            GradientKind::Conic { angle, center } => angle.is_finite() && center.is_finite(),
        };
        if !valid {
            return Err(Error::Type);
        }
        if matches!(kind,GradientKind::Radial{r0,..} if r0<0.)
            || matches!(kind,GradientKind::Radial{r1,..} if r1<0.)
        {
            return Err(Error::IndexSize);
        }
        Ok(Self(Arc::new(RwLock::new(GradientData {
            kind,
            stops: Vec::new(),
        }))))
    }
    pub fn add_color_stop(&self, offset: f64, color: &str) -> Result<()> {
        if !offset.is_finite() {
            return Err(Error::Type);
        }
        if !(0. ..=1.).contains(&offset) {
            return Err(Error::IndexSize);
        }
        let color = parse_color(color)?.ok_or(Error::Syntax)?;
        let mut data = self.0.write().map_err(|_| Error::Poisoned)?;
        // Stable sorting keeps equal-offset discontinuities in insertion order.
        data.stops.push((offset, color));
        data.stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repetition {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}
#[derive(Debug, Clone)]
struct PatternData {
    source: Source,
    transform: Affine,
    repetition: Repetition,
}
#[derive(Debug, Clone)]
pub struct Pattern(Arc<RwLock<PatternData>>);
impl Pattern {
    pub fn new(source: Source, repetition: Repetition) -> Self {
        Self(Arc::new(RwLock::new(PatternData {
            source,
            transform: Affine::IDENTITY,
            repetition,
        })))
    }
    pub fn set_transform(&self, m: Affine) -> Result<()> {
        if m.as_coeffs().iter().all(|v| v.is_finite()) {
            self.0.write().map_err(|_| Error::Poisoned)?.transform = m;
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub enum Paint {
    Solid(Color),
    Gradient(Gradient),
    Pattern(Pattern),
}
impl Default for Paint {
    fn default() -> Self {
        Self::Solid(Color::BLACK)
    }
}
#[derive(Debug, Clone)]
pub(crate) enum FrozenPaint {
    Solid([f32; 4]),
    Gradient {
        kind: GradientKind,
        stops: Vec<(f64, Color)>,
        transform: Affine,
    },
    Image {
        source: Source,
        to_source: Affine,
        repetition: Repetition,
        smooth: bool,
        extend: bool,
        crop: kurbo::Rect,
    },
}
impl Paint {
    pub(crate) fn freeze(&self, transform: Affine, smooth: bool) -> Result<FrozenPaint> {
        Ok(match self {
            Self::Solid(c) => FrozenPaint::Solid(c.premultiplied()),
            Self::Gradient(g) => {
                let g = g.0.read().map_err(|_| Error::Poisoned)?;
                FrozenPaint::Gradient {
                    kind: g.kind.clone(),
                    stops: g.stops.clone(),
                    transform,
                }
            }
            Self::Pattern(p) => {
                let p = p.0.read().map_err(|_| Error::Poisoned)?;
                let m = transform * p.transform;
                if m.determinant() == 0. {
                    FrozenPaint::Solid([0.; 4])
                } else {
                    FrozenPaint::Image {
                        source: p.source.clone(),
                        to_source: m.inverse(),
                        repetition: p.repetition,
                        smooth,
                        extend: false,
                        crop: kurbo::Rect::new(
                            0.,
                            0.,
                            p.source.pixels.width() as f64,
                            p.source.pixels.height() as f64,
                        ),
                    }
                }
            }
        })
    }
}
impl FrozenPaint {
    pub(crate) fn support(&self, w: u32, h: u32) -> Option<kurbo::BezPath> {
        use kurbo::Shape;
        let Self::Image {
            to_source,
            repetition,
            extend: false,
            crop,
            ..
        } = self
        else {
            return None;
        };
        if *repetition == Repetition::Repeat {
            return None;
        }
        let corners = [
            Point::ZERO,
            Point::new(w as f64, 0.),
            Point::new(0., h as f64),
            Point::new(w as f64, h as f64),
        ]
        .map(|p| *to_source * p);
        if corners.iter().any(|p| !p.is_finite()) {
            return Some(kurbo::BezPath::new());
        }
        let mut bounds = kurbo::Rect::from_points(corners[0], corners[0]);
        for p in corners {
            bounds = bounds.union_pt(p);
        }
        let rx = matches!(repetition, Repetition::RepeatX);
        let ry = matches!(repetition, Repetition::RepeatY);
        let r = kurbo::Rect::new(
            if rx { bounds.x0 - 1. } else { crop.x0 },
            if ry { bounds.y0 - 1. } else { crop.y0 },
            if rx { bounds.x1 + 1. } else { crop.x1 },
            if ry { bounds.y1 + 1. } else { crop.y1 },
        );
        Some(to_source.inverse() * r.to_path(1e-4))
    }
    pub(crate) fn sample(&self, p: Point) -> [f32; 4] {
        match self {
            Self::Solid(c) => *c,
            Self::Image {
                source,
                to_source,
                repetition,
                smooth,
                extend,
                crop,
            } => {
                let p = *to_source * p;
                let (w, h) = (source.pixels.width() as f64, source.pixels.height() as f64);
                let (rx, ry) = (
                    matches!(repetition, Repetition::Repeat | Repetition::RepeatX),
                    matches!(repetition, Repetition::Repeat | Repetition::RepeatY),
                );
                let texel = |x: i64, y: i64| {
                    // Reconstruction clamps to the source crop; patterns wrap their filter taps.
                    let x = if !extend || rx {
                        x.rem_euclid(w as i64)
                    } else {
                        (x as f64).clamp(crop.x0.floor(), crop.x1.ceil() - 1.) as i64
                    };
                    let y = if !extend || ry {
                        y.rem_euclid(h as i64)
                    } else {
                        (y as f64).clamp(crop.y0.floor(), crop.y1.ceil() - 1.) as i64
                    };
                    source.pixels.pixel(x, y)
                };
                if !smooth {
                    return texel(libm::floor(p.x) as i64, libm::floor(p.y) as i64);
                }
                let (x, y) = (p.x - 0.5, p.y - 0.5);
                let (ix, iy) = (libm::floor(x) as i64, libm::floor(y) as i64);
                let (fx, fy) = ((x - libm::floor(x)) as f32, (y - libm::floor(y)) as f32);
                let (a, b, c, d) = (
                    texel(ix, iy),
                    texel(ix + 1, iy),
                    texel(ix, iy + 1),
                    texel(ix + 1, iy + 1),
                );
                std::array::from_fn(|i| {
                    (a[i] * (1. - fx) + b[i] * fx) * (1. - fy) + (c[i] * (1. - fx) + d[i] * fx) * fy
                })
            }
            Self::Gradient {
                kind,
                stops,
                transform,
            } => {
                let local = if transform.determinant() != 0. {
                    transform.inverse() * p
                } else {
                    p
                };
                if !local.is_finite() {
                    return [0.; 4];
                }
                let t = match *kind {
                    GradientKind::Linear(a, b) => {
                        let (a, b, q) = if transform.determinant() == 0. {
                            (*transform * a, *transform * b, p)
                        } else {
                            (a, b, local)
                        };
                        let d = b - a;
                        let length = d.hypot2();
                        if length == 0. {
                            return [0.; 4];
                        }
                        (q - a).dot(d) / length
                    }
                    GradientKind::Radial { start, r0, end, r1 } => {
                        if transform.determinant() == 0. || (start == end && r0 == r1) {
                            return [0.; 4];
                        }
                        let q = local - start;
                        let d = end - start;
                        let dr = r1 - r0;
                        let (a, b, c) = (
                            d.hypot2() - dr * dr,
                            -2. * (q.dot(d) + r0 * dr),
                            q.hypot2() - r0 * r0,
                        );
                        let roots = if a.abs() < 1e-14 {
                            if b == 0. {
                                return [0.; 4];
                            }
                            [-c / b, f64::NEG_INFINITY]
                        } else {
                            let disc = b * b - 4. * a * c;
                            if disc < 0. {
                                return [0.; 4];
                            }
                            let s = libm::sqrt(disc);
                            [(-b - s) / (2. * a), (-b + s) / (2. * a)]
                        };
                        let t = roots
                            .into_iter()
                            .filter(|t| t.is_finite() && r0 + *t * dr >= 0.)
                            .max_by(f64::total_cmp);
                        let Some(t) = t else {
                            return [0.; 4];
                        };
                        t
                    }
                    GradientKind::Conic { angle, center } => {
                        if transform.determinant() == 0. {
                            return [0.; 4];
                        }
                        (libm::atan2(local.y - center.y, local.x - center.x) - angle)
                            .rem_euclid(std::f64::consts::TAU)
                            / std::f64::consts::TAU
                    }
                };
                interpolate(stops, t)
            }
        }
    }
}
fn interpolate(stops: &[(f64, Color)], t: f64) -> [f32; 4] {
    if stops.is_empty() {
        return [0.; 4];
    }
    if t < stops[0].0 {
        return stops[0].1.premultiplied();
    }
    let after = stops.partition_point(|s| s.0 <= t);
    if after == stops.len() {
        return stops[after - 1].1.premultiplied();
    }
    let (ta, a) = stops[after - 1];
    let (tb, b) = stops[after];
    let u = ((t - ta) / (tb - ta)) as f32;
    // HTML requires interpolation of color AND alpha without premultiplying.
    let c = Color {
        r: a.r + (b.r - a.r) * u,
        g: a.g + (b.g - a.g) * u,
        b: a.b + (b.b - a.b) * u,
        a: a.a + (b.a - a.a) * u,
    };
    c.premultiplied()
}
