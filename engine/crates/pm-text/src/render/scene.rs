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
    /// The mask of a glyph Skia draws from its path (synthesized bold,
    /// `strokeText`): `glyph` of `face` at `size` device px and variation
    /// `coords` in `style` (quarter offset, skew), with `frame` applied in
    /// user space (the device transform is a uniform `scale`). `None` when
    /// the platform does not draw glyphs this way.
    #[cfg(feature = "cg-masks")]
    #[allow(clippy::too_many_arguments)]
    fn path_mask(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        style: MaskStyle,
        scale: f32,
        frame: super::skpath::Frame,
    ) -> Option<Arc<GlyphMask>> {
        let _ = (face, coords, glyph, size, style, scale, frame);
        None
    }
    /// The mask of `glyph` at text size `size` under the device 2×2
    /// `post` (`[scaleX, skewY, skewX, scaleY]`, kept to 1/1024) with the
    /// synthetic skew `skew`, at quarter offsets `sub` in x and y, as the
    /// platform generates glyphs under a rotated, skewed or non-uniform
    /// matrix. `None` when the platform does not draw glyphs this way.
    #[cfg(feature = "cg-masks")]
    #[allow(clippy::too_many_arguments)]
    fn matrix_mask(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        size: f32,
        post: [f32; 4],
        skew: f32,
        sub: (u8, u8),
    ) -> Option<Arc<GlyphMask>> {
        let _ = (face, coords, glyph, size, post, skew, sub);
        None
    }
    /// A glyph's outline as Skia draws text as paths
    /// (`SkStrikeSpec::MakePath`): the platform's outline at the canonical
    /// 64 px (`SkFontPriv::kCanonicalTextSizeForPaths`), y down, no
    /// subpixel offset. `None` when the platform does not provide it.
    #[cfg(feature = "cg-masks")]
    fn canonical_path(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
    ) -> Option<Arc<pm_skia_path::Path>> {
        let _ = (face, coords, glyph);
        None
    }
    /// The distance field of `glyph` as the GPU canvas's distance-field
    /// text generates it (`sdf`): CoreGraphics' smoothed mask at the strike
    /// size `strike` (256 px; 162 at exactly 162) with the synthetic skew
    /// `skew`, linearized, no preblend. `None` when the platform does not
    /// provide it.
    /// `frame` (strike px) is the glyph's path frame when Skia generates
    /// the strike's A8 from the path: synthesized bold, `strokeText`.
    #[cfg(feature = "cg-masks")]
    fn sdf_glyph(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u16,
        strike: f32,
        skew: f32,
        frame: Option<super::skpath::Frame>,
    ) -> Option<Arc<super::sdf::SdfGlyph>> {
        let _ = (face, coords, glyph, strike, skew, frame);
        None
    }
    /// The mask gamma of path-generated masks for paint `rgb` (no
    /// CoreGraphics linearization).
    fn path_preblend(&self, rgb: [f64; 3]) -> [u8; 256] {
        let _ = rgb;
        std::array::from_fn(|i| i as u8)
    }
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
    /// Forget everything drawn so far: the fonts behind the faces' ids
    /// changed (ids are never reused, so this frees memory; it is not
    /// needed for correctness).
    fn reset(&mut self) {}
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
    /// A glyph's device origin as Skia maps it for `drawTextBlob` at
    /// `origin`: the canvas matrix in `f32`, pre-translated by the blob's
    /// origin (`SkMatrix::preTranslate`), then the glyph's blob position
    /// mapped (`SkMatrix::mapPoints`), all in `f32` (Chromium builds Skia
    /// without floating-point contraction).
    fn glyph_origin(&self, origin: (f64, f64), g: (f64, f64)) -> kurbo::Point {
        let [sx, ky, kx, sy, tx, ty] = self.m.as_coeffs().map(|v| v as f32);
        let (ox, oy) = (origin.0 as f32, origin.1 as f32);
        let translate_only = sx == 1.0 && sy == 1.0 && kx == 0.0 && ky == 0.0;
        let (tx, ty) = if translate_only {
            (tx + ox, ty + oy)
        } else {
            (tx + (sx * ox + kx * oy), ty + (ky * ox + sy * oy))
        };
        let (gx, gy) = (g.0 as f32, g.1 as f32);
        kurbo::Point::new(
            f64::from(gx * sx + gy * kx + tx),
            f64::from(gy * sy + gx * ky + ty),
        )
    }

    /// The matrix a glyph's path draws with (`PathOpSubmitter`): the canvas
    /// matrix in `f32` concatenated with a translation to the glyph's
    /// position (blob origin plus glyph offset) and the strike-to-source
    /// scale, as `[a, b, c, d, e, f]`.
    #[cfg(feature = "cg-masks")]
    fn glyph_matrix(&self, origin: (f64, f64), g: (f64, f64), scale: f32) -> [f32; 6] {
        let [a, b, c, d, e, f] = self.m.as_coeffs().map(|v| v as f32);
        let px = g.0 as f32 + origin.0 as f32;
        let py = g.1 as f32 + origin.1 as f32;
        [
            a * scale,
            b * scale,
            c * scale,
            d * scale,
            a * px + c * py + e,
            b * px + d * py + f,
        ]
    }

    /// The device scale a glyph strike is made for: Skia's descriptor
    /// keeps the device matrix to 1/1024 (`sk_relax` in
    /// `SkScalerContext::MakeRecAndEffects`), so glyph masks of a scaled
    /// word are generated at `size × relax(scale)`, not at the exact scale.
    fn strike_scale(&self) -> f32 {
        let x = self.scale as f32;
        (x * 1024.0 + 0.5).floor() / 1024.0
    }

    /// The device 2×2 as the strike descriptor keeps it (each entry to
    /// 1/1024, `sk_relax`), `[scaleX, skewY, skewX, scaleY]`.
    #[cfg(feature = "cg-masks")]
    fn relaxed_post(&self) -> [f32; 4] {
        let [a, b, c, d, _, _] = self.m.as_coeffs();
        let relax = |v: f64| ((v as f32) * 1024.0 + 0.5).floor() / 1024.0;
        [relax(a), relax(b), relax(c), relax(d)]
    }

    /// A distance-field glyph's quad as `prepare_for_SDFT_drawing` and
    /// Graphite's `VertexFiller` place it: the glyph position mapped into
    /// strike space by the creation matrix (`1 / strikeToSourceScale`, in
    /// `f32`) plus the field's bounds inset by 2, drawn through the canvas
    /// matrix pre-translated by the blob origin and post-scaled by the
    /// creation matrix's inverse (`[a, b, c, d, e, f]`).
    #[cfg(feature = "cg-masks")]
    fn sdf_placement(
        &self,
        origin: (f64, f64),
        g: (f64, f64),
        text_size: f32,
        strike: f32,
        field: &super::sdf::SdfGlyph,
    ) -> ((f32, f32), [f32; 6]) {
        let to_source = text_size / strike;
        let creation = 1.0 / to_source;
        let inverse = 1.0 / creation;
        let mapped = (g.0 as f32 * creation, g.1 as f32 * creation);
        let inset = super::sdf::INSET as f32;
        let left_top = (
            (field.left as f32 + inset) + mapped.0,
            (field.top as f32 + inset) + mapped.1,
        );
        let [a, b, c, d, e, f] = self.m.as_coeffs().map(|v| v as f32);
        let (ox, oy) = (origin.0 as f32, origin.1 as f32);
        (
            left_top,
            [
                a * inverse,
                b * inverse,
                c * inverse,
                d * inverse,
                a * ox + c * oy + e,
                b * ox + d * oy + f,
            ],
        )
    }

    /// `SkMatrix::getMaxScale` of the 2×2 (the largest singular value).
    fn max_scale(&self) -> f32 {
        let [a, b, c, d, _, _] = self.m.as_coeffs();
        // Singular values of [[a, c], [b, d]].
        let s1 = a * a + b * b;
        let s2 = c * c + d * d;
        let s12 = a * c + b * d;
        let mean = (s1 + s2) / 2.0;
        let diff = ((s1 - s2) / 2.0).hypot(s12);
        ((mean + diff).max(0.0).sqrt()) as f32
    }

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

/// The GPU canvas's distance-field strike size for text of `size` under
/// the placement's matrix, or `None` outside its range
/// (`SubRunControl::isSDFT`, `getSDFFont` on macOS): from 162 to 256 device
/// px (`SkFontPriv::ApproximateTransformedTextSize`), the 162 px strike at
/// exactly 162 and the 256 px strike above.
fn sdf_strike(size: f32, place: &Placement) -> Option<f32> {
    let approx = size * place.max_scale();
    if !(approx.is_finite() && approx > 0.0) {
        return None;
    }
    if (162.0..=256.0).contains(&approx) {
        Some(if approx <= 162.0 { 162.0 } else { 256.0 })
    } else {
        None
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
        // Platform masks of this call, at whole device pixels; `true` for
        // masks Skia generates from paths (their own gamma table).
        let mut platform: Vec<(i64, i64, Arc<GlyphMask>, bool)> = Vec::new();
        // Glyphs drawn as multisampled paths (sample counts per pixel).
        let mut msaa_glyphs: Vec<(i64, i64, usize, usize, Vec<u8>)> = Vec::new();
        // Glyphs drawn as distance-field text (coverage per pixel).
        let mut sdf_glyphs: Vec<(i64, i64, usize, usize, Vec<u8>)> = Vec::new();
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
            // Distance-field text: the strike size, when this run's device
            // size is in the GPU canvas's SDFT range.
            let sdf_strike = if self.masks.is_some() {
                sdf_strike(run.size, &place)
            } else {
                None
            };
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
                let p = place.glyph_origin(origin, (g.x, g.y));
                let bsize = run.bitmap_size.unwrap_or(dsize);
                if color_face
                    && place.axis_aligned
                    && let Some(item) =
                        self.color_glyph(run, g.id, p, bsize, emoji_shift, color, opacity, blur)
                {
                    images.push(item);
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
                let mut as_path = self.profile.glyph_as_path(device_size, bare.bounds);
                #[cfg(feature = "cg-masks")]
                if let (Some(strike), Coords::User(coords), Some(masks)) =
                    (sdf_strike, run.coords, self.masks.as_deref_mut())
                    && let Some(field) = masks.sdf_glyph(
                        run.face,
                        coords,
                        g.id as u16,
                        strike,
                        skew_of(run) as f32,
                        run.synthetic_bold
                            .then(|| super::skpath::Frame::fake_bold(strike)),
                    )
                {
                    // A field beyond the atlas is drawn as its path
                    // (`SkGlyphDigest::fitsInAtlasDirect`).
                    if field.width.max(field.height) <= 256 {
                        let (left_top, matrix) =
                            place.sdf_placement(origin, (g.x, g.y), run.size, strike, &field);
                        if let Some(cov) =
                            super::sdf::draw(&field, left_top, matrix, super::sdf::APPLE_GPU)
                        {
                            sdf_glyphs.push(cov);
                        }
                        continue;
                    }
                    as_path = true;
                }
                #[cfg(not(feature = "cg-masks"))]
                let _ = sdf_strike;
                // Beyond the atlas, the GPU canvas draws a glyph as its path,
                // 4× multisampled, at its exact position.
                if as_path && self.masks.is_some() && !run.synthetic_bold {
                    // As the GPU canvas tessellates the platform's outline at
                    // the canonical 64 px under the glyph's matrix.
                    #[cfg(feature = "cg-masks")]
                    if let Coords::User(coords) = run.coords
                        && let Some(local) = self
                            .masks
                            .as_deref_mut()
                            .and_then(|masks| masks.canonical_path(run.face, coords, g.id as u16))
                    {
                        let gm = place.glyph_matrix(origin, (g.x, g.y), run.size / 64.0);
                        if let Some(cov) = tessellated_coverage(&local, gm) {
                            msaa_glyphs.push(cov);
                        }
                        continue;
                    }
                    // Flattening tolerance closest to the GPU's tessellation
                    // on the `large` cases (0.03 to 0.2 px tried).
                    let tol = 0.12;
                    let (px, py) = (p.x, p.y);
                    let t = Affine::translate((px, py)) * place.residual;
                    if let Some((x0, y0, w, h, counts)) = msaa_coverage(&(t * &*bare.path), tol) {
                        msaa_glyphs.push((x0, y0, w, h, counts));
                    }
                    continue;
                }
                // Synthesized bold: Skia draws the glyph image from its path
                // with a stroke-and-fill frame (`skpath`).
                #[cfg(feature = "cg-masks")]
                if !as_path
                    && !large
                    && place.axis_aligned
                    && run.synthetic_bold
                    && let Some(masks) = self.masks.as_deref_mut()
                {
                    let (x, y) = self.profile.snap.apply(p.x, p.y);
                    let ix = x.floor();
                    let quarter = (((x - ix) * 4.0).round()).clamp(0.0, 3.0);
                    let skew = skew_of(run);
                    let Coords::User(coords) = run.coords else {
                        continue;
                    };
                    if let Some(m) = masks.path_mask(
                        run.face,
                        coords,
                        g.id as u16,
                        run.size * place.strike_scale(),
                        MaskStyle {
                            quarter: quarter as u8,
                            skew: skew as f32,
                        },
                        place.strike_scale(),
                        super::skpath::Frame::fake_bold(run.size),
                    ) {
                        platform.push((ix as i64, y as i64, m, true));
                    }
                    continue;
                }
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
                        run.size * place.strike_scale(),
                        style,
                        bare.bounds.inflate(grow_x, 0.0),
                    ) {
                        platform.push((ix as i64, y as i64, m, false));
                    }
                    continue;
                }
                // Under a rotated, skewed or non-uniform matrix Skia still
                // draws glyph masks, generated with the whole matrix and
                // positioned to quarter pixels in x and y.
                #[cfg(feature = "cg-masks")]
                if !as_path
                    && !large
                    && !place.axis_aligned
                    && !run.synthetic_bold
                    && let (Some(masks), Coords::User(coords)) =
                        (self.masks.as_deref_mut(), run.coords)
                {
                    let qx = (p.x * 4.0 + 0.5).floor() / 4.0;
                    let qy = (p.y * 4.0 + 0.5).floor() / 4.0;
                    let (ix, iy) = (qx.floor(), qy.floor());
                    let sub = (((qx - ix) * 4.0) as u8, ((qy - iy) * 4.0) as u8);
                    if let Some(m) = masks.matrix_mask(
                        run.face,
                        coords,
                        g.id as u16,
                        run.size,
                        place.relaxed_post(),
                        skew_of(run) as f32,
                        sub,
                    ) {
                        platform.push((ix as i64, iy as i64, m, false));
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
            let rgb = [color[0], color[1], color[2]];
            out.extend(mask_image(
                &platform,
                [&masks.preblend(rgb), &masks.path_preblend(rgb)],
                color,
                opacity,
                blur,
            ));
        }
        if !msaa_glyphs.is_empty() {
            // Coverage quarters as masks (a linear table), composited like
            // the glyph masks.
            let quarters: Vec<(i64, i64, Arc<GlyphMask>, bool)> = msaa_glyphs
                .into_iter()
                .map(|(x0, y0, w, h, counts)| {
                    (
                        x0,
                        y0,
                        Arc::new(GlyphMask {
                            width: w as u32,
                            height: h as u32,
                            left: 0,
                            top: 0,
                            coverage: counts
                                .iter()
                                .map(|&n| (u32::from(n) * 255 / 4) as u8)
                                .collect(),
                        }),
                        false,
                    )
                })
                .collect();
            let linear: [u8; 256] = std::array::from_fn(|i| i as u8);
            out.extend(mask_image(
                &quarters,
                [&linear, &linear],
                color,
                opacity,
                blur,
            ));
        }
        if !sdf_glyphs.is_empty() {
            out.extend(sdf_image(sdf_glyphs, color, opacity, blur));
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
    /// have no outline to stroke: on macOS their image is drawn again.
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
        #[cfg(feature = "cg-masks")]
        if place.axis_aligned
            && self.masks.is_some()
            && self.stroke_masks(out, runs, origin, &place, width, color, opacity)
        {
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
                let p = place.glyph_origin(origin, (g.x, g.y));
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

impl Painter<'_> {
    /// A color bitmap glyph as the platform draws it (CoreGraphics, the
    /// whole-pixel position and quarter offset, Apple Color Emoji's shift),
    /// with the paint's alpha; `None` when the platform does not draw it.
    #[allow(clippy::too_many_arguments)]
    fn color_glyph(
        &mut self,
        run: &Run<'_>,
        glyph: u32,
        p: kurbo::Point,
        bsize: f32,
        emoji_shift: Option<i32>,
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) -> Option<Item> {
        let masks = self.masks.as_deref_mut()?;
        let (x, y) = self.profile.snap.apply(p.x, p.y);
        let ix = x.floor();
        let quarter = (((x - ix) * 4.0).round() as i64).clamp(0, 3) as u8;
        // Drawn at the em size CoreText uses for the strike.
        let shift = emoji_shift.unwrap_or(0);
        let (image, left, top) = masks
            .color(run.face, glyph as u16, bsize, quarter, bsize)
            .map(|(i, l, t)| (i, l + ix as i32, t + y as i32 + shift))?;
        let t = Affine::translate((f64::from(left), f64::from(top)));
        let b = Rect::new(
            f64::from(left),
            f64::from(top),
            f64::from(left) + f64::from(image.width),
            f64::from(top) + f64::from(image.height),
        );
        Some(Item {
            content: ItemContent::Image {
                image,
                transform: t,
            },
            color,
            opacity,
            blur,
            bounds: b,
        })
    }

    /// `strokeText` as Skia draws it on macOS: each glyph's image generated
    /// from its path with a stroke frame (`skpath`), at the glyph's
    /// quarter-pixel position, through the path mask gamma of the stroke
    /// color, composited like `fillText`'s masks. `false` (nothing pushed)
    /// when a glyph is beyond the atlas (Skia strokes such text as paths).
    #[cfg(feature = "cg-masks")]
    #[allow(clippy::too_many_arguments)]
    fn stroke_masks(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        place: &Placement,
        width: f64,
        color: Rgba,
        opacity: f64,
    ) -> bool {
        let frame = super::skpath::Frame {
            width: width as f32,
            fill: false,
            join: pm_skia_path::LineJoin::Round,
            miter_limit: 2.0,
        };
        // `SkStrikeSpec::ShouldDrawAsPath`: the text matrix's scale beyond
        // 256 px draws paths, not glyph images.
        if runs.iter().any(|r| f64::from(r.size) * place.scale > 256.0) {
            return false;
        }
        let mut platform: Vec<(i64, i64, Arc<GlyphMask>, bool)> = Vec::new();
        let mut sdf_quads: Vec<(i64, i64, usize, usize, Vec<u8>)> = Vec::new();
        let mut images = Vec::new();
        for run in runs {
            let device_size = f64::from(run.size) * place.scale;
            let skew = skew_of(run);
            let bitmap_face = has_bitmaps(run.face);
            let color_face = bitmap_face && super::bitmap::has_color_bitmaps(run.face);
            let emoji_shift =
                (color_face && super::bitmap::is_apple_color_emoji(run.face)).then(|| {
                    let dsize = device_size as f32;
                    super::bitmap::apple_emoji_shift(dsize, run.bitmap_size.unwrap_or(dsize)) as i32
                });
            for g in run.glyphs {
                let bsize = run.bitmap_size.unwrap_or(device_size as f32);
                let p = place.glyph_origin(origin, (g.x, g.y));
                if bitmap_face
                    && self
                        .bitmaps
                        .get(run.face, g.id, device_size as f32, bsize)
                        .is_some()
                {
                    // A color glyph has no path to frame: Skia draws its
                    // image in the stroke pass too (with the stroke paint's
                    // alpha), so the fill pass draws it over itself.
                    if color_face {
                        let Some(item) =
                            self.color_glyph(run, g.id, p, bsize, emoji_shift, color, opacity, 0.0)
                        else {
                            return false;
                        };
                        images.push(item);
                    }
                    continue;
                }
                let Coords::User(coords) = run.coords else {
                    return false;
                };
                // A wide stroke from 162 to 256 px is distance-field text
                // too (`SubRunControl::isSDFT`): the strike's A8 is the
                // glyph's path framed with the stroke scaled into strike
                // space (`dfPaint.setStrokeWidth(width / strikeToSourceScale)`).
                if let Some(strike) = sdf_strike(run.size, place) {
                    let to_source = run.size / strike;
                    let mut sdf_frame = super::skpath::Frame {
                        width: frame.width / to_source,
                        ..frame
                    };
                    if run.synthetic_bold {
                        sdf_frame.width += super::skpath::Frame::fake_bold(strike).width;
                    }
                    let Some(masks) = self.masks.as_deref_mut() else {
                        return false;
                    };
                    let Some(field) = masks.sdf_glyph(
                        run.face,
                        coords,
                        g.id as u16,
                        strike,
                        skew as f32,
                        Some(sdf_frame),
                    ) else {
                        continue;
                    };
                    if field.width.max(field.height) > 256 {
                        return false;
                    }
                    let (left_top, matrix) =
                        place.sdf_placement(origin, (g.x, g.y), run.size, strike, &field);
                    if let Some(cov) =
                        super::sdf::draw(&field, left_top, matrix, super::sdf::APPLE_GPU)
                    {
                        sdf_quads.push(cov);
                    }
                    continue;
                }
                let (x, y) = self.profile.snap.apply(p.x, p.y);
                let ix = x.floor();
                let quarter = (((x - ix) * 4.0).round()).clamp(0.0, 3.0);
                // A synthesized bold run strokes its emboldened path: the
                // fake bold frame and the stroke add (Skia adds the widths).
                let frame = if run.synthetic_bold {
                    super::skpath::Frame {
                        width: frame.width + super::skpath::Frame::fake_bold(run.size).width,
                        ..frame
                    }
                } else {
                    frame
                };
                let Some(masks) = self.masks.as_deref_mut() else {
                    return false;
                };
                let Some(m) = masks.path_mask(
                    run.face,
                    coords,
                    g.id as u16,
                    run.size * place.strike_scale(),
                    MaskStyle {
                        quarter: quarter as u8,
                        skew: skew as f32,
                    },
                    place.strike_scale(),
                    frame,
                ) else {
                    continue;
                };
                // Glyph images beyond the atlas are drawn as paths.
                if m.width > 256 || m.height > 256 {
                    return false;
                }
                platform.push((ix as i64, y as i64, m, true));
            }
        }
        if !platform.is_empty()
            && let Some(masks) = self.masks.as_deref_mut()
        {
            let rgb = [color[0], color[1], color[2]];
            out.extend(mask_image(
                &platform,
                [&masks.preblend(rgb), &masks.path_preblend(rgb)],
                color,
                opacity,
                0.0,
            ));
        }
        if !sdf_quads.is_empty() {
            out.extend(sdf_image(sdf_quads, color, opacity, 0.0));
        }
        out.extend(images);
        true
    }
}

/// Distance-field glyph coverage (`sdf::draw`) as one image item, each
/// glyph composited over the earlier ones like the glyph masks.
fn sdf_image(
    quads: Vec<(i64, i64, usize, usize, Vec<u8>)>,
    color: Rgba,
    opacity: f64,
    blur: f64,
) -> Option<Item> {
    let fields: Vec<(i64, i64, Arc<GlyphMask>, bool)> = quads
        .into_iter()
        .map(|(x0, y0, w, h, coverage)| {
            (
                x0,
                y0,
                Arc::new(GlyphMask {
                    width: w as u32,
                    height: h as u32,
                    left: 0,
                    top: 0,
                    coverage,
                }),
                false,
            )
        })
        .collect();
    let linear: [u8; 256] = std::array::from_fn(|i| i as u8);
    mask_image(&fields, [&linear, &linear], color, opacity, blur)
}

/// One image item holding the platform masks of a `fillText` call, each
/// glyph composited over the earlier ones (Skia draws glyph quads one by
/// one with source-over), in `color` through the preblend `table`.
fn mask_image(
    masks: &[(i64, i64, Arc<GlyphMask>, bool)],
    tables: [&[u8; 256]; 2],
    color: Rgba,
    opacity: f64,
    blur: f64,
) -> Option<Item> {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for (x, y, m, _) in masks {
        x0 = x0.min(x + i64::from(m.left));
        y0 = y0.min(y + i64::from(m.top));
        x1 = x1.max(x + i64::from(m.left) + i64::from(m.width));
        y1 = y1.max(y + i64::from(m.top) + i64::from(m.height));
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    // Each glyph quad is blended source-over into the 8-bit canvas with the
    // paint's color and alpha (`globalAlpha` included): overlapping glyphs
    // of a translucent paint build up, and the target is quantized after
    // every glyph, as the GPU canvas blends them.
    let a = (color[3].clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0)) as f32;
    let rgb = [color[0], color[1], color[2]].map(|c| c.clamp(0.0, 1.0) as f32);
    let mut data = vec![0u8; w * h * 4];
    for (x, y, m, path) in masks {
        let table = tables[usize::from(*path)];
        let ox = (x + i64::from(m.left) - x0) as usize;
        let oy = (y + i64::from(m.top) - y0) as usize;
        for row in 0..m.height as usize {
            let src = &m.coverage[row * m.width as usize..(row + 1) * m.width as usize];
            let start = ((oy + row) * w + ox) * 4;
            let dst = &mut data[start..start + m.width as usize * 4];
            for (d, &c) in dst.chunks_exact_mut(4).zip(src) {
                if c == 0 {
                    continue;
                }
                let sa = f32::from(table[c as usize]) / 255.0 * a;
                let keep = 1.0 - sa;
                for k in 0..3 {
                    let v = rgb[k] * sa + f32::from(d[k]) / 255.0 * keep;
                    d[k] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                let v = sa + f32::from(d[3]) / 255.0 * keep;
                d[3] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    let image = Arc::new(Rgba8 {
        width: w as u32,
        height: h as u32,
        data,
    });
    let transform = Affine::translate((x0 as f64, y0 as f64));
    Some(Item {
        content: ItemContent::Image { image, transform },
        // The image holds the color and `globalAlpha`.
        color: [1.0, 1.0, 1.0, 1.0],
        opacity: 1.0,
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
    // `roundedRect` of `captions/render.ts`: `arcTo` corners, each a
    // quarter conic (`SkPath::arcTo`), in user space as Skia gets them.
    let radius = r.min(w / 2.0).min(h / 2.0).max(0.0) as f32;
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let weight = (0.5f32).sqrt();
    let corners = [
        (
            (fx + fw, fy),
            (fx + fw - radius, fy),
            (fx + fw, fy + radius),
        ),
        (
            (fx + fw, fy + fh),
            (fx + fw, fy + fh - radius),
            (fx + fw - radius, fy + fh),
        ),
        (
            (fx, fy + fh),
            (fx + radius, fy + fh),
            (fx, fy + fh - radius),
        ),
        ((fx, fy), (fx, fy + radius), (fx + radius, fy)),
    ];
    let (a32, e32, f32_) = (a as f32, e as f32, f as f32);
    // Graphite tessellates a conic at T = k / 2^L, L from Wang's formula for
    // conics at precision 4 in device space, evaluated in local space.
    let mut poly: Vec<(f64, f64)> = Vec::new();
    for (p1, p0, p2) in corners {
        let dev = |p: (f32, f32)| (p.0 * a32, p.1 * a32);
        let (q0, q1, q2) = (dev(p0), dev(p1), dev(p2));
        let cx = 0.5 * (q0.0.min(q1.0).min(q2.0) + q0.0.max(q1.0).max(q2.0));
        let cy = 0.5 * (q0.1.min(q1.1).min(q2.1) + q0.1.max(q1.1).max(q2.1));
        let c = |q: (f32, f32)| (q.0 - cx, q.1 - cy);
        let (c0, c1, c2) = (c(q0), c(q1), c(q2));
        let d = |q: (f32, f32)| q.0 * q.0 + q.1 * q.1;
        let max_len = d(c0).max(d(c1)).max(d(c2)).sqrt();
        let dp = (
            -2.0 * weight * c1.0 + c0.0 + c2.0,
            -2.0 * weight * c1.1 + c0.1 + c2.1,
        );
        let dw = (-2.0 * weight + 2.0).abs();
        let rp = (max_len * 4.0 - 1.0).max(0.0);
        let numer = d(dp).sqrt() * 4.0 + rp * dw;
        let denom = 4.0 * weight.min(1.0);
        let n2 = numer / denom;
        let level = if n2 <= 1.0 {
            0
        } else {
            ((n2.log2().ceil() as i32) + 1) >> 1
        }
        .clamp(0, 5);
        let segments = 1 << level;
        let p1w = (p1.0 * weight, p1.1 * weight);
        let mix = |a: f32, b: f32, t: f32| (b - a).mul_add(t, a);
        for k in 0..=segments {
            let t = (k << (5 - level)) as f32 / 32.0;
            let (lx, ly) = if k == 0 {
                p0
            } else if k == segments {
                p2
            } else {
                let ab = (mix(p0.0, p1w.0, t), mix(p0.1, p1w.1, t));
                let bc = (mix(p1w.0, p2.0, t), mix(p1w.1, p2.1, t));
                let abc = (mix(ab.0, bc.0, t), mix(ab.1, bc.1, t));
                let u = mix(1.0, weight, t);
                let v = weight + 1.0 - u;
                let uv = mix(u, v, t);
                (abc.0 / uv, abc.1 / uv)
            };
            // The rasterizer snaps vertices to 1/256 px.
            let snap = |v: f32| f64::from((v * 256.0).round() / 256.0);
            poly.push((snap(lx.mul_add(a32, e32)), snap(ly.mul_add(a32, f32_))));
        }
    }
    let (x0, y0, x1, y1) = (x * a + e, y * a + f, (x + w) * a + e, (y + h) * a + f);
    let inside = |p: kurbo::Point| {
        // Nonzero winding of the closed polygon.
        let mut winding = 0i32;
        for i in 0..poly.len() {
            let (ax, ay) = poly[i];
            let (bx, by) = poly[(i + 1) % poly.len()];
            if ay <= p.y {
                if by > p.y && (bx - ax) * (p.y - ay) - (p.x - ax) * (by - ay) > 0.0 {
                    winding += 1;
                }
            } else if by <= p.y && (bx - ax) * (p.y - ay) - (p.x - ax) * (by - ay) < 0.0 {
                winding -= 1;
            }
        }
        winding != 0
    };
    msaa_item(Rect::new(x0, y0, x1, y1), inside, color, opacity)
}

/// A path drawn as the GPU canvas tessellates it (Graphite's wedges): each
/// quad as its cubic, each cubic cut at `T = k / 2^L` (`L` from Wang's
/// formula at precision 4 in device space, at most 5), evaluated in local
/// space by De Casteljau, mapped by `m` (`[a, b, c, d, e, f]`: `x' = a x + c
/// y + e`), and snapped to 1/256 px; then 4 samples per pixel, nonzero
/// winding. Sample counts 0 to 4 per pixel over the whole-pixel bounds.
#[cfg(feature = "cg-masks")]
pub fn tessellated_coverage(
    path: &pm_skia_path::Path,
    m: [f32; 6],
) -> Option<(i64, i64, usize, usize, Vec<u8>)> {
    use pm_skia_path::PathSegment;
    let [a, b, c, d, e, f] = m;
    let dev = |x: f32, y: f32| {
        let snap = |v: f32| f64::from((v * 256.0).round() / 256.0);
        (snap(a * x + c * y + e), snap(b * x + d * y + f))
    };
    let mix = |p: (f32, f32), q: (f32, f32), t: f32| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
    let mut contours: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut last = (0.0f32, 0.0f32);
    let cubic = |out: &mut Vec<(f64, f64)>,
                 p0: (f32, f32),
                 p1: (f32, f32),
                 p2: (f32, f32),
                 p3: (f32, f32)| {
        let fd = |q0: (f32, f32), q1: (f32, f32), q2: (f32, f32)| {
            let v = (
                (-2.0f32).mul_add(q1.0, q2.0) + q0.0,
                (-2.0f32).mul_add(q1.1, q2.1) + q0.1,
            );
            let w = (a * v.0 + c * v.1, b * v.0 + d * v.1);
            w.0 * w.0 + w.1 * w.1
        };
        let m2 = fd(p0, p1, p2).max(fd(p1, p2, p3));
        let level = ((9.0 * m2).max(1.0).log2() * 0.25).ceil().clamp(0.0, 5.0) as i32;
        let n = 1i32 << level;
        for k in 1..n {
            let t = (k << (5 - level)) as f32 / 32.0;
            let ab = mix(p0, p1, t);
            let bc = mix(p1, p2, t);
            let cd = mix(p2, p3, t);
            let abc = mix(ab, bc, t);
            let bcd = mix(bc, cd, t);
            let q = mix(abc, bcd, t);
            out.push(dev(q.0, q.1));
        }
        out.push(dev(p3.0, p3.1));
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                last = (p.x, p.y);
                contours.push(vec![dev(p.x, p.y)]);
            }
            PathSegment::LineTo(p) => {
                last = (p.x, p.y);
                contours.last_mut()?.push(dev(p.x, p.y));
            }
            PathSegment::QuadTo(p1, p2) => {
                let (q1, q2) = ((p1.x, p1.y), (p2.x, p2.y));
                // `writeQuadPatch`: the quad's cubic.
                let c1 = mix(last, q1, 2.0 / 3.0);
                let c2 = mix(q2, q1, 2.0 / 3.0);
                cubic(contours.last_mut()?, last, c1, c2, q2);
                last = q2;
            }
            PathSegment::CubicTo(p1, p2, p3) => {
                let q3 = (p3.x, p3.y);
                cubic(contours.last_mut()?, last, (p1.x, p1.y), (p2.x, p2.y), q3);
                last = q3;
            }
            PathSegment::ConicTo(..) => return None,
            PathSegment::Close => {}
        }
    }
    let edges: Vec<(f64, f64, f64, f64)> = contours
        .iter()
        .flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()])))
        .filter(|(p, q)| p.1 != q.1)
        .map(|(p, q)| (p.0, p.1, q.0, q.1))
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in &contours {
        for p in c {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
    }
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let (ix0, iy0) = (x0.floor() as i64, y0.floor() as i64);
    let (w, h) = (
        (x1.ceil() as i64 - ix0) as usize,
        (y1.ceil() as i64 - iy0) as usize,
    );
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let mut counts = vec![0u8; w * h];
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for py in 0..h {
        for (sx, sy) in MSAA4 {
            let ys = (iy0 + py as i64) as f64 + sy;
            crossings.clear();
            for &(ax, ay, bx, by) in &edges {
                let (lo, hi, dir) = if ay < by { (ay, by, 1) } else { (by, ay, -1) };
                if ys >= lo && ys < hi {
                    let t = (ys - ay) / (by - ay);
                    crossings.push((ax + (bx - ax) * t, dir));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut winding = 0;
            let mut ci = 0;
            for px in 0..w {
                let xs = (ix0 + px as i64) as f64 + sx;
                while ci < crossings.len() && crossings[ci].0 < xs {
                    winding += crossings[ci].1;
                    ci += 1;
                }
                if winding != 0 {
                    counts[py * w + px] += 1;
                }
            }
        }
    }
    Some((ix0, iy0, w, h, counts))
}

/// 4× multisampled coverage of a device-space path (nonzero winding), as
/// sample counts 0 to 4 per pixel over the path's whole-pixel bounds: one
/// scanline per sample row of the standard pattern (each sample has its own
/// row), crossings of the flattened path sorted along it.
pub fn msaa_coverage(path: &BezPath, tolerance: f64) -> Option<(i64, i64, usize, usize, Vec<u8>)> {
    let b = path.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.x0.is_finite() && b.y1.is_finite()) {
        return None;
    }
    let (x0, y0) = (b.x0.floor() as i64, b.y0.floor() as i64);
    let (x1, y1) = (b.x1.ceil() as i64, b.y1.ceil() as i64);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    // Line segments of the flattened path, (x0, y0, x1, y1), closing
    // every subpath.
    let mut lines: Vec<(f64, f64, f64, f64)> = Vec::new();
    let (mut start, mut last) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
    kurbo::flatten(path.iter(), tolerance, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if last != start {
                lines.push((last.x, last.y, start.x, start.y));
            }
            start = p;
            last = p;
        }
        kurbo::PathEl::LineTo(p) => {
            lines.push((last.x, last.y, p.x, p.y));
            last = p;
        }
        kurbo::PathEl::ClosePath => {
            if last != start {
                lines.push((last.x, last.y, start.x, start.y));
            }
            last = start;
        }
        _ => {}
    });
    if last != start {
        lines.push((last.x, last.y, start.x, start.y));
    }
    let mut counts = vec![0u8; w * h];
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for py in 0..h {
        for (sx, sy) in MSAA4 {
            let y = (y0 + py as i64) as f64 + sy;
            crossings.clear();
            for &(ax, ay, bx, by) in &lines {
                // Half-open in y, so a vertex is crossed once.
                let (lo, hi, dir) = if ay < by { (ay, by, 1) } else { (by, ay, -1) };
                if y >= lo && y < hi {
                    let t = (y - ay) / (by - ay);
                    crossings.push((ax + t * (bx - ax), dir));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding == 0 {
                    continue;
                }
                // Samples at x = px + sx inside [pair[0].0, pair[1].0).
                let first = ((pair[0].0 - sx - x0 as f64).ceil()).max(0.0) as i64;
                let last = ((pair[1].0 - sx - x0 as f64).ceil() as i64 - 1).min(w as i64 - 1);
                for px in first..=last {
                    counts[py * w + px as usize] += 1;
                }
            }
        }
    }
    Some((x0, y0, w, h, counts))
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
            for c in &color[..3] {
                data.push((c.clamp(0.0, 1.0) as f32 * alpha * 255.0).round() as u8);
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
            id: 1,
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

    /// Text drawn as paths: a square covers its pixels with all four
    /// samples, a half-pixel edge with two; a quad's tessellation stays
    /// inside its hull; vertices land on the 1/256 px grid.
    #[cfg(feature = "cg-masks")]
    #[test]
    fn tessellated_paths_sample_four_times_per_pixel() {
        let mut b = pm_skia_path::PathBuilder::new();
        b.move_to(0.0, 0.0);
        b.line_to(4.0, 0.0);
        b.line_to(4.0, 4.0);
        b.line_to(0.0, 4.0);
        b.close();
        let square = b.finish().unwrap();
        let (x0, y0, w, h, counts) =
            tessellated_coverage(&square, [1.0, 0.0, 0.0, 1.0, 1.5, 2.0]).unwrap();
        assert_eq!((x0, y0, w, h), (1, 2, 5, 4));
        for row in counts.chunks(5) {
            assert_eq!(row, [2, 4, 4, 4, 2]);
        }
        let mut q = pm_skia_path::PathBuilder::new();
        q.move_to(0.0, 0.0);
        q.quad_to(32.0, 64.0, 64.0, 0.0);
        q.close();
        let quad = q.finish().unwrap();
        let (_, _, _, h, counts) =
            tessellated_coverage(&quad, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
        assert_eq!(h, 32, "the curve reaches y = 32, not its control point");
        assert!(counts.contains(&4) && counts.iter().all(|&c| c <= 4));
    }
}
