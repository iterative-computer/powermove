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
use super::outline::{Coords, FaceRef, GlyphStyle, Outline, Outlines};
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
    pub coords: Coords<'a>,
    pub synthetic_bold: bool,
    /// Skia's synthetic italic (level 1).
    pub synthetic_italic: bool,
    /// Synthesized oblique angle in degrees (level 2, Parley's), 0 for none.
    pub oblique: f64,
    pub glyphs: &'a [PlacedGlyph],
    /// For color bitmap faces: the size CoreText draws their glyphs at, for
    /// this run's device size (`FontResolver::em_advance`), when it differs
    /// from the device size.
    pub bitmap_size: Option<f32>,
}

/// A glyph's mask as the platform's text rasterizer makes it (CoreGraphics
/// on macOS, `cgmask`), before the paint's preblend.
#[derive(Debug)]
pub struct GlyphMask {
    pub width: u32,
    pub height: u32,
    /// The mask's top-left pixel relative to the glyph's whole-pixel origin
    /// (x right, y down; the origin is the pixel corner left of the pen on
    /// the baseline).
    pub left: i32,
    pub top: i32,
    /// Coverage before the preblend, row by row.
    pub coverage: Vec<u8>,
}

/// How a platform glyph mask is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskStyle {
    /// The glyph's offset right of a whole pixel, in quarter pixels.
    pub quarter: u8,
    /// Skia's synthetic italic: `x' = x + skew × y` with y down (0: none).
    pub skew: f32,
}

/// Where level 1 glyph masks come from when the platform rasterizes them as
/// Canvas2D does (`cgmask` on macOS).
pub trait MaskSource {
    /// The mask of `glyph` of `face` at `size` device px and variation
    /// `coords`, drawn in `style`; `bounds` is the glyph's outline box at
    /// that size (device px, y down).
    fn mask(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        style: MaskStyle,
        bounds: Rect,
    ) -> Option<Arc<GlyphMask>>;
    /// The preblend table for straight E-encoded paint `rgb`.
    fn preblend(&self, rgb: [f64; 3]) -> [u8; 256];
    /// A color glyph (`sbix` emoji) as the platform draws it at `size`
    /// device px, `quarter`/4 px right of a whole pixel: a premultiplied
    /// image and its top-left pixel relative to the whole-pixel origin.
    /// `em` is the size of the glyph's em box (`FontResolver::em_advance`).
    fn color(
        &mut self,
        face: FaceRef<'_>,
        glyph: u16,
        size: f32,
        quarter: u8,
        em: f32,
    ) -> Option<(Arc<Rgba8>, i32, i32)> {
        let _ = (face, glyph, size, quarter, em);
        None
    }
}

/// What a glyph draw needs: the caches and the profile.
pub struct Painter<'c> {
    pub outlines: &'c mut Outlines,
    pub bitmaps: &'c mut Bitmaps,
    /// Level 1 glyph masks from the platform rasterizer, when it has one;
    /// glyphs Canvas2D draws from masks are then drawn from these.
    pub masks: Option<&'c mut dyn MaskSource>,
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
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            0.0
        };
        let residual = if scale > 0.0 {
            Affine::new([a / scale, b / scale, c / scale, d / scale, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };
        let axis_aligned =
            b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 && (a - d).abs() <= 1e-9 * a.abs();
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
        // Platform masks of this call, at whole device pixels.
        let mut platform: Vec<(i64, i64, Arc<GlyphMask>)> = Vec::new();
        let mut curve_size = 0.0f64;
        for run in runs {
            let device_size = f64::from(run.size) * place.scale;
            let dsize = device_size as f32;
            let bitmap_face = has_bitmaps(run.face);
            let color_face = bitmap_face && super::bitmap::has_color_bitmaps(run.face);
            // Canvas2D draws Apple Color Emoji lower than CoreGraphics does,
            // by whole pixels (`apple_emoji_shift`, measured on `fit-emoji`).
            let emoji_shift = if color_face && super::bitmap::is_apple_color_emoji(run.face) {
                Some(
                    super::bitmap::apple_emoji_shift(dsize, run.bitmap_size.unwrap_or(dsize))
                        as i32,
                )
            } else {
                None
            };
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
                skew: skew_of(run),
                expand: (ex, ey),
            };
            let plain = GlyphStyle {
                expand: (0.0, 0.0),
                ..style
            };
            for g in run.glyphs {
                let p = place.m * kurbo::Point::new(origin.0 + g.x, origin.1 + g.y);
                let bsize = run.bitmap_size.unwrap_or(dsize);
                if color_face
                    && place.axis_aligned
                    && let Some(masks) = self.masks.as_deref_mut()
                    && let Some((image, left, top)) = {
                        let (x, y) = self.profile.snap.apply(p.x, p.y);
                        let ix = x.floor();
                        let quarter = (((x - ix) * 4.0).round() as i64).clamp(0, 3) as u8;
                        // Drawn at the em size CoreText uses for the strike.
                        let shift = emoji_shift.unwrap_or(0);
                        masks
                            .color(run.face, g.id as u16, bsize, quarter, bsize)
                            .map(|(i, l, t)| (i, l + ix as i32, t + y as i32 + shift))
                    }
                {
                    let t = Affine::translate((f64::from(left), f64::from(top)));
                    let b = Rect::new(
                        f64::from(left),
                        f64::from(top),
                        f64::from(left) + f64::from(image.width),
                        f64::from(top) + f64::from(image.height),
                    );
                    images.push(Item {
                        content: ItemContent::Image {
                            image,
                            transform: t,
                        },
                        color,
                        opacity,
                        blur,
                        bounds: b,
                    });
                    continue;
                }
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
                // Synthesized bold keeps the outline path: Skia's fake bold
                // (a stroke-and-fill path) is not reproduced exactly yet
                // (docs/engine/text-render.md, "Gaps").
                if !as_path
                    && !large
                    && place.axis_aligned
                    && !run.synthetic_bold
                    && let (Some(masks), Coords::User(coords)) =
                        (self.masks.as_deref_mut(), run.coords)
                {
                    let (x, y) = self.profile.snap.apply(p.x, p.y);
                    let ix = x.floor();
                    let style = MaskStyle {
                        quarter: (((x - ix) * 4.0).round() as i64).clamp(0, 3) as u8,
                        skew: skew_of(run) as f32,
                    };
                    let grow_x = f64::from(style.skew).abs()
                        * bare.bounds.y0.abs().max(bare.bounds.y1.abs());
                    if let Some(m) = masks.mask(
                        run.face,
                        coords,
                        g.id as u16,
                        dsize,
                        style,
                        bare.bounds.inflate(grow_x, 0.0),
                    ) {
                        platform.push((ix as i64, y as i64, m));
                    }
                    continue;
                }
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
                let (x, y) = if place.axis_aligned && (as_path || !large) {
                    let (x, y) = self.profile.snap.apply(p.x, p.y);
                    (x, y + dy)
                } else if place.axis_aligned {
                    (p.x, p.y + dy)
                } else {
                    let d = place.residual * kurbo::Point::new(0.0, dy);
                    (p.x + d.x, p.y + d.y)
                };
                let t = Affine::translate((x, y)) * place.residual;
                let target = if as_path || large {
                    &mut path
                } else {
                    &mut mask
                };
                union(&mut target.1, t.transform_rect_bbox(outline.bounds));
                target.0.push(placed(&outline, t, PathStyle::Fill));
                if let Some(width) = outline.dilate {
                    let stroke = Stroke::new(width).with_join(kurbo::Join::Round);
                    target
                        .0
                        .push(placed(&outline, t, PathStyle::Stroke(stroke)));
                }
                if !as_path {
                    curve_size = curve_size.max(device_size);
                }
            }
        }
        if !platform.is_empty()
            && let Some(masks) = self.masks.as_deref_mut()
        {
            out.extend(mask_image(
                &platform,
                masks.preblend([color[0], color[1], color[2]]),
                color,
                opacity,
                blur,
            ));
        }
        if let Some(bounds) = mask.1 {
            out.push(Item {
                content: ItemContent::Coverage {
                    paths: mask.0,
                    curve: self
                        .profile
                        .curve(curve_size, [color[0], color[1], color[2]]),
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
                skew: skew_of(run),
                expand: (0.0, 0.0),
            };
            let bitmap_face = has_bitmaps(run.face);
            for g in run.glyphs {
                let bsize = run.bitmap_size.unwrap_or(device_size as f32);
                if bitmap_face
                    && self
                        .bitmaps
                        .get(run.face, g.id, device_size as f32, bsize)
                        .is_some()
                {
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

/// One image item holding the platform masks of a `fillText` call, each
/// glyph composited over the earlier ones (Skia draws glyph quads one by
/// one with source-over), in `color` through the preblend `table`.
fn mask_image(
    masks: &[(i64, i64, Arc<GlyphMask>)],
    table: [u8; 256],
    color: Rgba,
    opacity: f64,
    blur: f64,
) -> Option<Item> {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for (x, y, m) in masks {
        x0 = x0.min(x + i64::from(m.left));
        y0 = y0.min(y + i64::from(m.top));
        x1 = x1.max(x + i64::from(m.left) + i64::from(m.width));
        y1 = y1.max(y + i64::from(m.top) + i64::from(m.height));
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    // Coverage composited source-over in float.
    let mut cov = vec![0.0f32; w * h];
    for (x, y, m) in masks {
        let ox = (x + i64::from(m.left) - x0) as usize;
        let oy = (y + i64::from(m.top) - y0) as usize;
        for row in 0..m.height as usize {
            let src = &m.coverage[row * m.width as usize..(row + 1) * m.width as usize];
            let dst = &mut cov[(oy + row) * w + ox..(oy + row) * w + ox + m.width as usize];
            for (d, &c) in dst.iter_mut().zip(src) {
                if c != 0 {
                    let a = f32::from(table[c as usize]) / 255.0;
                    *d = a + *d * (1.0 - a);
                }
            }
        }
    }
    let a = color[3].clamp(0.0, 1.0) as f32;
    let mut data = Vec::with_capacity(w * h * 4);
    for c in cov {
        let alpha = c * a;
        for ch in 0..3 {
            data.push((color[ch].clamp(0.0, 1.0) as f32 * alpha * 255.0).round() as u8);
        }
        data.push((alpha * 255.0).round() as u8);
    }
    let image = Arc::new(Rgba8 {
        width: w as u32,
        height: h as u32,
        data,
    });
    let transform = Affine::translate((x0 as f64, y0 as f64));
    Some(Item {
        content: ItemContent::Image { image, transform },
        // The image holds the color; the item keeps only `globalAlpha`.
        color: [1.0, 1.0, 1.0, 1.0],
        opacity,
        blur,
        bounds: Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64),
    })
}

/// The shear of a run's synthesized italic or oblique.
fn skew_of(run: &Run<'_>) -> f64 {
    if run.synthetic_italic {
        super::outline::SYNTHETIC_ITALIC_SKEW
    } else if run.oblique != 0.0 {
        -run.oblique.to_radians().tan()
    } else {
        0.0
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

/// The sample positions of a 4× multisampled pixel (the standard pattern
/// of Metal, D3D, and Vulkan), in pixel units from its top-left corner.
const MSAA4: [(f64, f64); 4] = [
    (0.375, 0.125),
    (0.875, 0.375),
    (0.125, 0.625),
    (0.625, 0.875),
];

/// `fill()` of a path as Chromium's GPU canvas fills it: 4× multisampled
/// coverage (a quarter per sample inside the path, nonzero winding), as a
/// premultiplied image item in `color` (measured on the caption boxes,
/// whose edge pixels take only the values 0, 1/4, 1/2, 3/4, 1).
pub fn fill_path_msaa(path: BezPath, m: Affine, color: Rgba, opacity: f64) -> Option<Item> {
    let path = m * path;
    msaa_item(
        path.bounding_box(),
        |p| path.winding(p) != 0,
        color,
        opacity,
    )
}

/// [`fill_path_msaa`] of the canvas `roundedRect(x, y, w, h, r)` through
/// `m` (a translation and uniform scale), with exact circular corners
/// (Skia's `arcTo` makes conics, which are exact circles).
pub fn fill_rounded_rect_msaa(
    rect: (f64, f64, f64, f64, f64),
    m: Affine,
    color: Rgba,
    opacity: f64,
) -> Option<Item> {
    let (x, y, w, h, r) = rect;
    let [a, b, c, d, e, f] = m.as_coeffs();
    if b != 0.0 || c != 0.0 || a <= 0.0 || (a - d).abs() > 1e-12 * a {
        return fill_path_msaa(rounded_rect(x, y, w, h, r), m, color, opacity);
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0) * a;
    let (x0, y0, x1, y1) = (x * a + e, y * a + f, (x + w) * a + e, (y + h) * a + f);
    let inside = |p: kurbo::Point| {
        if p.x < x0 || p.x > x1 || p.y < y0 || p.y > y1 {
            return false;
        }
        let cx = p.x.clamp(x0 + r, x1 - r);
        let cy = p.y.clamp(y0 + r, y1 - r);
        let (dx, dy) = (p.x - cx, p.y - cy);
        dx * dx + dy * dy <= r * r
    };
    msaa_item(Rect::new(x0, y0, x1, y1), inside, color, opacity)
}

fn msaa_item(
    b: Rect,
    inside: impl Fn(kurbo::Point) -> bool,
    color: Rgba,
    opacity: f64,
) -> Option<Item> {
    if !(b.width() > 0.0 && b.height() > 0.0) {
        return None;
    }
    let (x0, y0) = (b.x0.floor() as i64, b.y0.floor() as i64);
    let (x1, y1) = (b.x1.ceil() as i64, b.y1.ceil() as i64);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let a = color[3].clamp(0.0, 1.0) as f32;
    let mut data = Vec::with_capacity(w * h * 4);
    for py in 0..h {
        for px in 0..w {
            let n = MSAA4
                .iter()
                .filter(|(sx, sy)| {
                    inside(kurbo::Point::new(
                        (x0 + px as i64) as f64 + sx,
                        (y0 + py as i64) as f64 + sy,
                    ))
                })
                .count();
            let alpha = a * n as f32 / 4.0;
            for ch in 0..3 {
                data.push((color[ch].clamp(0.0, 1.0) as f32 * alpha * 255.0).round() as u8);
            }
            data.push((alpha * 255.0).round() as u8);
        }
    }
    Some(Item {
        content: ItemContent::Image {
            image: Arc::new(Rgba8 {
                width: w as u32,
                height: h as u32,
                data,
            }),
            transform: Affine::translate((x0 as f64, y0 as f64)),
        },
        color: [1.0, 1.0, 1.0, 1.0],
        opacity,
        blur: 0.0,
        bounds: Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64),
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
    p.curve_to(
        (x + w, y + h - r + k),
        (x + w - r + k, y + h),
        (x + w - r, y + h),
    );
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

    /// `font` with a `COLR` (version 0) and `CPAL` table that make `glyph`
    /// a color vector glyph of one red layer (itself).
    fn with_colr(font: &[u8], glyph: u16) -> Vec<u8> {
        let be16 = |v: u16| v.to_be_bytes();
        let be32 = |v: u32| v.to_be_bytes();
        let mut colr = Vec::new();
        colr.extend(be16(0)); // version
        colr.extend(be16(1)); // numBaseGlyphRecords
        colr.extend(be32(14)); // baseGlyphRecordsOffset
        colr.extend(be32(20)); // layerRecordsOffset
        colr.extend(be16(1)); // numLayerRecords
        colr.extend(be16(glyph));
        colr.extend(be16(0)); // firstLayerIndex
        colr.extend(be16(1)); // numLayers
        colr.extend(be16(glyph));
        colr.extend(be16(0)); // paletteIndex
        let mut cpal = Vec::new();
        cpal.extend(be16(0)); // version
        cpal.extend(be16(1)); // numPaletteEntries
        cpal.extend(be16(1)); // numPalettes
        cpal.extend(be16(1)); // numColorRecords
        cpal.extend(be32(14)); // colorRecordsArrayOffset
        cpal.extend(be16(0)); // colorRecordIndices[0]
        cpal.extend([0, 0, 255, 255]); // BGRA red
        let n = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..n)
            .map(|i| {
                let r = &font[12 + 16 * i..28 + 16 * i];
                let off = u32::from_be_bytes([r[8], r[9], r[10], r[11]]) as usize;
                let len = u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize;
                ([r[0], r[1], r[2], r[3]], font[off..off + len].to_vec())
            })
            .collect();
        tables.push((*b"COLR", colr));
        tables.push((*b"CPAL", cpal));
        tables.sort_by_key(|t| t.0);
        let mut out = font[..4].to_vec();
        out.extend(be16(tables.len() as u16));
        out.extend([0; 6]); // searchRange etc. (unused by the reader)
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            out.extend(tag);
            out.extend(be32(0)); // checksum (not verified)
            out.extend(be32(offset as u32));
            out.extend(be32(data.len() as u32));
            let mut padded = data.clone();
            padded.resize(data.len().div_ceil(4) * 4, 0);
            offset += padded.len();
            body.extend(padded);
        }
        out.extend(body);
        out
    }

    #[test]
    fn color_vector_glyphs_are_refused_not_drawn() {
        use skrifa::MetadataProvider;
        let Ok(arial) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            eprintln!("Arial is not installed; skipped");
            return;
        };
        let font = skrifa::FontRef::new(&arial).unwrap();
        let a = font.charmap().map('A').unwrap().to_u32();
        let b = font.charmap().map('B').unwrap().to_u32();
        let colr = with_colr(&arial, a as u16);
        let face = FaceRef {
            data: &colr,
            index: 0,
            id: colr.as_ptr() as usize,
        };
        assert!(super::super::bitmap::has_colr(face));
        assert!(super::super::bitmap::is_colr_glyph(face, a));
        assert!(!super::super::bitmap::is_colr_glyph(face, b));
        let draw = |glyphs: &[PlacedGlyph]| {
            let mut outlines = Outlines::new();
            let mut bitmaps = Bitmaps::new();
            let mut painter = Painter {
                outlines: &mut outlines,
                bitmaps: &mut bitmaps,
                masks: None,
                profile: Profile::CANVAS2D,
                unsupported: Vec::new(),
            };
            let run = Run {
                face,
                size: 40.0,
                coords: Coords::User(&[]),
                synthetic_bold: false,
                synthetic_italic: false,
                oblique: 0.0,
                glyphs,
                bitmap_size: None,
            };
            let mut items = Vec::new();
            painter.fill_text(
                &mut items,
                &[run],
                (0.0, 40.0),
                Affine::IDENTITY,
                [0.0, 0.0, 0.0, 1.0],
                1.0,
                0.0,
            );
            (items.len(), painter.unsupported)
        };
        let glyph = |id| PlacedGlyph { id, x: 0.0, y: 0.0 };
        // A plain glyph of the face draws.
        let (n, unsupported) = draw(&[glyph(b)]);
        assert!(n > 0 && unsupported.is_empty(), "{unsupported:?}");
        // The color vector glyph is refused, not drawn as a monochrome outline.
        let (_, unsupported) = draw(&[glyph(b), glyph(a)]);
        assert_eq!(unsupported, vec!["color vector glyphs (COLR)".to_owned()]);
    }
}
