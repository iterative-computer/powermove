//! The text scene: what a text raster (or a level 2 output region) paints,
//! as an ordered list of items in device pixels.
//!
//! Each item is either *coverage* (glyph outlines filled or stroked, or a
//! filled path such as a caption box), which Vello rasterizes as 8-bit
//! coverage and the paint pass multiplies by the item's color in float, or
//! a color *image* (an emoji bitmap), which the paint pass samples. Items
//! composite in order with premultiplied source-over, exactly as the
//! canvas calls they stand for (`fillText`, `strokeText`, `fill`, each with
//! its `globalAlpha` and `filter: blur()`).

use std::sync::Arc;

use kurbo::{Affine, BezPath, Rect, Shape, Stroke};

use super::bitmap::{Bitmaps, Rgba8, has_bitmaps};
use super::color::Rgba;
use super::outline::{FaceRef, GlyphStyle, Outline, Outlines};
use super::profile::Profile;
use crate::compat::PlacedGlyph;

/// How a coverage path is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum PathStyle {
    Fill,
    /// `strokeText` and `stroke()`: width in the path's space.
    Stroke(Stroke),
}

/// A path placed in device space.
#[derive(Clone, Debug)]
pub struct PlacedPath {
    pub path: Arc<BezPath>,
    pub transform: Affine,
    pub style: PathStyle,
}

/// What an item paints.
#[derive(Clone, Debug)]
pub enum ItemContent {
    Coverage {
        paths: Vec<PlacedPath>,
        /// The coverage curve the paint pass applies (`profile::Curve`).
        curve: super::profile::Curve,
    },
    Image {
        image: Arc<Rgba8>,
        /// Image pixels to device px.
        transform: Affine,
    },
}

/// One canvas draw.
#[derive(Clone, Debug)]
pub struct Item {
    pub content: ItemContent,
    /// Straight E-encoded RGBA (`fillStyle`); images ignore the color but
    /// keep its alpha.
    pub color: Rgba,
    /// `globalAlpha`.
    pub opacity: f64,
    /// `filter: blur(σ px)` in device px (0 for none).
    pub blur: f64,
    /// Device-space bounds of the ink before blur (conservative).
    pub bounds: Rect,
}

impl Item {
    /// The device rect the item can touch, blur included.
    pub fn reach(&self) -> Rect {
        let pad = blur_radius(self.blur) as f64 + 1.0;
        self.bounds.inflate(pad, pad)
    }
}

/// The kernel radius of a Gaussian blur of `sigma` px (3σ, as Skia's
/// Gaussian blur samples it).
pub fn blur_radius(sigma: f64) -> u32 {
    if sigma > 0.0 {
        (3.0 * sigma).ceil() as u32
    } else {
        0
    }
}

/// A scene in a target of `width` × `height` device px.
#[derive(Clone, Debug, Default)]
pub struct TextScene {
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

impl TextScene {
    /// The union of every item's reach, clipped to the target, in whole
    /// pixels (`None` when nothing draws).
    pub fn ink(&self) -> Option<[u32; 4]> {
        let mut r: Option<Rect> = None;
        for item in &self.items {
            let reach = item.reach();
            r = Some(r.map_or(reach, |u| u.union(reach)));
        }
        let r = r?.intersect(Rect::new(
            0.0,
            0.0,
            f64::from(self.width),
            f64::from(self.height),
        ));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return None;
        }
        let x0 = r.x0.floor().max(0.0) as u32;
        let y0 = r.y0.floor().max(0.0) as u32;
        let x1 = (r.x1.ceil() as u32).min(self.width);
        let y1 = (r.y1.ceil() as u32).min(self.height);
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
    }
}

/// One shaped run to draw: a face, its size and location, and glyphs in
/// the run's layout space.
#[derive(Clone, Copy)]
pub struct Run<'a> {
    pub face: FaceRef<'a>,
    pub size: f32,
    pub coords: &'a [(u32, f32)],
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub glyphs: &'a [PlacedGlyph],
    /// For color bitmap faces: the size CoreText draws their glyphs at, for
    /// this run's device size (`FontResolver::em_advance`), when it differs
    /// from the device size.
    pub bitmap_size: Option<f32>,
}

/// What a glyph draw needs: the caches and the profile.
pub struct Painter<'c> {
    pub outlines: &'c mut Outlines,
    pub bitmaps: &'c mut Bitmaps,
    pub profile: Profile,
    /// What the scene could not draw (color vector glyphs): the renderer
    /// refuses such a layer rather than drawing it wrong.
    pub unsupported: Vec<String>,
}

/// A layout-to-device transform split into a uniform scale (the device
/// font size factor) and the residual linear part.
#[derive(Clone, Copy, Debug)]
struct Placement {
    m: Affine,
    scale: f64,
    residual: Affine,
    axis_aligned: bool,
}

impl Placement {
    fn of(m: Affine) -> Placement {
        let [a, b, c, d, _, _] = m.as_coeffs();
        let det = a * d - b * c;
        let scale = det.abs().sqrt();
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 0.0 };
        let residual = if scale > 0.0 {
            Affine::new([a / scale, b / scale, c / scale, d / scale, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };
        let axis_aligned = b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 && (a - d).abs() <= 1e-9 * a.abs();
        Placement {
            m,
            scale,
            residual,
            axis_aligned,
        }
    }
}

fn union(r: &mut Option<Rect>, b: Rect) {
    *r = Some(r.map_or(b, |u| u.union(b)));
}

impl Painter<'_> {
    /// The fake-bold outset per side for a synthesized bold run.
    fn fake_bold(&self, synthetic: bool, device_size: f64) -> (f64, f64) {
        if synthetic {
            let x = self.profile.fake_bold * device_size;
            (x, x * self.profile.fake_bold_y)
        } else {
            (0.0, 0.0)
        }
    }

    /// `fillText` of the runs with `fillStyle = color`, `globalAlpha =
    /// opacity`, `filter: blur(blur px)`, the context transform `m` (layout
    /// px to device px), and the text origin `origin` in layout px. Outline
    /// glyphs form one coverage item; color bitmap glyphs follow as image
    /// items.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_text(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) {
        let place = Placement::of(m);
        if place.scale == 0.0 {
            return;
        }
        // Glyphs drawn from smoothed masks (the profile's expansion, offset,
        // and curve) and glyphs drawn as plain paths (Skia draws a glyph as a
        // path when its size or its bounding box exceeds the atlas limit).
        let mut mask = (Vec::new(), None::<Rect>);
        let mut path = (Vec::new(), None::<Rect>);
        let mut images = Vec::new();
        let mut curve_size = 0.0f64;
        for run in runs {
            let device_size = f64::from(run.size) * place.scale;
            let dsize = device_size as f32;
            let bitmap_face = has_bitmaps(run.face);
            let colr = super::bitmap::has_colr(run.face);
            let large = self.profile.is_large(device_size);
            let (ex, ey) = if large {
                (self.profile.large.expand_x, self.profile.large.expand_y)
            } else {
                self.profile.expansion(device_size, false)
            };
            let bold = self.fake_bold(run.synthetic_bold, device_size);
            let style = GlyphStyle {
                size: dsize,
                synthetic_bold: bold,
                synthetic_italic: run.synthetic_italic,
                expand: (ex, ey),
            };
            let plain = GlyphStyle {
                expand: (0.0, 0.0),
                ..style
            };
            for g in run.glyphs {
                let p = place.m * kurbo::Point::new(origin.0 + g.x, origin.1 + g.y);
                let bsize = run.bitmap_size.unwrap_or(dsize);
                if bitmap_face && let Some(bmp) = self.bitmaps.get(run.face, g.id, dsize, bsize) {
                    let (x, y) = if place.axis_aligned {
                        self.profile.position(p.x, p.y, device_size, true)
                    } else {
                        (p.x, p.y)
                    };
                    let t = Affine::translate((x, y)) * place.residual * bmp.transform;
                    let b = t.transform_rect_bbox(Rect::new(
                        0.0,
                        0.0,
                        f64::from(bmp.image.width),
                        f64::from(bmp.image.height),
                    ));
                    images.push(Item {
                        content: ItemContent::Image {
                            image: bmp.image.clone(),
                            transform: t,
                        },
                        color,
                        opacity,
                        blur,
                        bounds: b,
                    });
                    continue;
                }
                if colr && super::bitmap::is_colr_glyph(run.face, g.id) {
                    let what = "color vector glyphs (COLR)".to_owned();
                    if !self.unsupported.contains(&what) {
                        self.unsupported.push(what);
                    }
                    continue;
                }
                let Some(bare) = self.outlines.get(run.face, run.coords, g.id, plain) else {
                    continue;
                };
                let as_path = self.profile.glyph_as_path(device_size, bare.bounds);
                let dy = if as_path {
                    0.0
                } else if large {
                    self.profile.large.dy
                } else {
                    self.profile.dy(device_size, false)
                };
                let outline = if as_path || (ex == 0.0 && ey == 0.0) {
                    bare
                } else {
                    let Some(o) = self.outlines.get(run.face, run.coords, g.id, style) else {
                        continue;
                    };
                    o
                };
                let (x, y) = if place.axis_aligned && !(large && !as_path) {
                    let (x, y) = self.profile.snap.apply(p.x, p.y);
                    (x, y + dy)
                } else if place.axis_aligned {
                    (p.x, p.y + dy)
                } else {
                    let d = place.residual * kurbo::Point::new(0.0, dy);
                    (p.x + d.x, p.y + d.y)
                };
                let t = Affine::translate((x, y)) * place.residual;
                let target = if as_path || large { &mut path } else { &mut mask };
                union(&mut target.1, t.transform_rect_bbox(outline.bounds));
                target.0.push(placed(&outline, t, PathStyle::Fill));
                if !as_path {
                    curve_size = curve_size.max(device_size);
                }
            }
        }
        if let Some(bounds) = mask.1 {
            out.push(Item {
                content: ItemContent::Coverage {
                    paths: mask.0,
                    curve: self.profile.curve(curve_size, [color[0], color[1], color[2]]),
                },
                color,
                opacity,
                blur,
                bounds,
            });
        }
        if let Some(bounds) = path.1 {
            let mut curve = super::profile::LINEAR;
            let g = self.profile.large.gamma;
            if g != 1.0 {
                for v in &mut curve {
                    *v = super::profile::coverage_curve(f64::from(*v), g) as f32;
                }
            }
            out.push(Item {
                content: ItemContent::Coverage {
                    paths: path.0,
                    curve,
                },
                color,
                opacity,
                blur,
                bounds,
            });
        }
        out.extend(images);
    }

    /// `strokeText` of the runs with `lineWidth = width` (layout px), round
    /// joins, and miter limit 2 (the captions context). Color bitmap glyphs
    /// are not stroked (Skia strokes outlines only).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_text(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        width: f64,
        color: Rgba,
        opacity: f64,
    ) {
        let place = Placement::of(m);
        if place.scale == 0.0 || width <= 0.0 {
            return;
        }
        let device_width = width * place.scale + 2.0 * self.profile.stroke.widen;
        let stroke = Stroke::new(device_width)
            .with_join(kurbo::Join::Round)
            .with_miter_limit(2.0);
        let half = device_width / 2.0 + 1.0;
        let mut paths = Vec::new();
        let mut bounds: Option<Rect> = None;
        for run in runs {
            let device_size = f64::from(run.size) * place.scale;
            let style = GlyphStyle {
                size: device_size as f32,
                synthetic_bold: self.fake_bold(run.synthetic_bold, device_size),
                synthetic_italic: run.synthetic_italic,
                expand: (0.0, 0.0),
            };
            let bitmap_face = has_bitmaps(run.face);
            for g in run.glyphs {
                let bsize = run.bitmap_size.unwrap_or(device_size as f32);
                if bitmap_face && self.bitmaps.get(run.face, g.id, device_size as f32, bsize).is_some() {
                    continue;
                }
                let Some(outline) = self.outlines.get(run.face, run.coords, g.id, style) else {
                    continue;
                };
                let p = place.m * kurbo::Point::new(origin.0 + g.x, origin.1 + g.y);
                let (x, y) = if place.axis_aligned {
                    self.profile.stroke_position(p.x, p.y)
                } else {
                    (p.x, p.y)
                };
                let t = Affine::translate((x, y)) * place.residual;
                union(
                    &mut bounds,
                    t.transform_rect_bbox(outline.bounds).inflate(half, half),
                );
                paths.push(placed(&outline, t, PathStyle::Stroke(stroke.clone())));
            }
        }
        if let Some(bounds) = bounds {
            out.push(Item {
                content: ItemContent::Coverage {
                    paths,
                    curve: stroke_curve(self.profile.stroke.gamma),
                },
                color,
                opacity,
                blur: 0.0,
                bounds,
            });
        }
    }
}

fn stroke_curve(gamma: f64) -> super::profile::Curve {
    let mut c = super::profile::LINEAR;
    for v in &mut c {
        *v = super::profile::coverage_curve(f64::from(*v), gamma) as f32;
    }
    c
}

fn placed(outline: &Outline, transform: Affine, style: PathStyle) -> PlacedPath {
    PlacedPath {
        path: outline.path.clone(),
        transform,
        style,
    }
}

/// A filled path (`fill()` with the nonzero rule) in layout px through `m`.
pub fn fill_path(path: BezPath, m: Affine, color: Rgba, opacity: f64) -> Option<Item> {
    let bounds = m.transform_rect_bbox(path.bounding_box());
    if bounds.width() <= 0.0 && bounds.height() <= 0.0 {
        return None;
    }
    Some(Item {
        content: ItemContent::Coverage {
            paths: vec![PlacedPath {
                path: Arc::new(path),
                transform: m,
                style: PathStyle::Fill,
            }],
            curve: super::profile::LINEAR,
        },
        color,
        opacity,
        blur: 0.0,
        bounds,
    })
}

/// The canvas `roundedRect` of `captions/render.ts`: `moveTo(x + r, y)`
/// and four `arcTo` corners with `r = max(0, min(r, w / 2, h / 2))`.
pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> BezPath {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut p = BezPath::new();
    if r <= 0.0 {
        p.move_to((x, y));
        p.line_to((x + w, y));
        p.line_to((x + w, y + h));
        p.line_to((x, y + h));
        p.close_path();
        return p;
    }
    // Circular quarter arcs as cubics (Skia converts arcTo to conics; the
    // two agree within 0.03% of r).
    const K: f64 = 0.552_284_749_830_793_4;
    let k = r * K;
    p.move_to((x + r, y));
    p.line_to((x + w - r, y));
    p.curve_to((x + w - r + k, y), (x + w, y + r - k), (x + w, y + r));
    p.line_to((x + w, y + h - r));
    p.curve_to((x + w, y + h - r + k), (x + w - r + k, y + h), (x + w - r, y + h));
    p.line_to((x + r, y + h));
    p.curve_to((x + r - k, y + h), (x, y + h - r + k), (x, y + h - r));
    p.line_to((x, y + r));
    p.curve_to((x, y + r - k), (x + r - k, y), (x + r, y));
    p.close_path();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placements_split_scale() {
        let p = Placement::of(Affine::scale(2.0));
        assert_eq!(p.scale, 2.0);
        assert!(p.axis_aligned);
        let p = Placement::of(Affine::rotate(0.3) * Affine::scale(3.0));
        assert!((p.scale - 3.0).abs() < 1e-12);
        assert!(!p.axis_aligned);
    }

    #[test]
    fn the_scene_ink_covers_items_and_their_blur() {
        let mut s = TextScene {
            width: 100,
            height: 50,
            items: vec![],
        };
        assert_eq!(s.ink(), None);
        s.items.push(
            fill_path(
                rounded_rect(10.5, 10.5, 20.0, 10.0, 3.0),
                Affine::IDENTITY,
                [1.0; 4],
                1.0,
            )
            .unwrap(),
        );
        assert_eq!(s.ink(), Some([9, 9, 32, 22]));
        s.items[0].blur = 2.0;
        assert_eq!(s.ink(), Some([3, 3, 38, 28]));
    }
}
