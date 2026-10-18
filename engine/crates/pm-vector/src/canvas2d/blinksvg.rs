//! Blink's side of an SVG `<img>` drawn into a Canvas2D (Chromium
//! 152.0.7977.54): how `drawImage(img, 0, 0, w, h)` sizes the SVG document
//! (`SVGImageForContainer::ConcreteObjectSize`, `LayoutSVGRoot` in
//! `LayoutUnit`s, `SVGRootPainter::TransformToPixelSnappedBorderBox`),
//! the `SkM44` chain `SVGImage::DrawInternal` builds, the element transforms
//! (`TransformHelper::ComputeTransform` on `gfx::Transform` in double), the
//! paint property tree (`PaintPropertyTreeBuilder`), and the cc paint ops
//! `PaintChunksToCcLayer` emits for the chunks (`SaveOp`, `ConcatOp`,
//! `TranslateOp`, `ClipRectOp`, `ClipRRectOp`, `ClipPathOp`,
//! `SaveLayerAlphaOp`, `SaveLayerOp` with a filter, the draws).
//!
//! The result is the op list `cc::PaintOpBuffer::Playback` hands to
//! `SkCanvas` (replayed by `svgrender`).

use super::m44::M44;
use super::msaa::Rule;
use super::skpath::{Mat, SkPath};
use super::skrrect::SkRRect;
use crate::render::svg::SvgError;
use std::collections::HashMap;

/// `gfx::ClampFloatGeometry` / `ClampTo<float>` of a double.
#[inline]
pub fn clamp_f32(v: f64) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
    }
}

/// Blink's `AffineTransform` (`[a b c d e f]`, double).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    pub fn is_identity_or_translation(&self) -> bool {
        self.0[0] == 1.0 && self.0[1] == 0.0 && self.0[2] == 0.0 && self.0[3] == 1.0
    }

    /// `DoMultiply(t1, t2)`: `t1 * t2` (apply `t2` first).
    pub fn multiply(t1: &Affine, t2: &Affine) -> Affine {
        let (a, b) = (&t1.0, &t2.0);
        if t1.is_identity_or_translation() {
            return Affine([b[0], b[1], b[2], b[3], a[4] + b[4], a[5] + b[5]]);
        }
        Affine([
            a[0] * b[0] + a[2] * b[1],
            a[1] * b[0] + a[3] * b[1],
            a[0] * b[2] + a[2] * b[3],
            a[1] * b[2] + a[3] * b[3],
            a[0] * b[4] + a[2] * b[5] + a[4],
            a[1] * b[4] + a[3] * b[5] + a[5],
        ])
    }

    /// `PreConcat(other)`: `*this = *this * other`.
    pub fn pre_concat(&mut self, other: &Affine) {
        *self = Affine::multiply(self, other);
    }

    /// `PostConcat(other)`: `*this = other * *this`.
    pub fn post_concat(&mut self, other: &Affine) {
        *self = Affine::multiply(other, self);
    }

    /// `ScaleNonUniform(sx, sy)` / `Scale(sx, sy)`.
    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.0[0] *= sx;
        self.0[1] *= sx;
        self.0[2] *= sy;
        self.0[3] *= sy;
    }

    /// `Translate(tx, ty)`.
    pub fn translate(&mut self, tx: f64, ty: f64) {
        self.0[4] += tx * self.0[0] + ty * self.0[2];
        self.0[5] += tx * self.0[1] + ty * self.0[3];
    }

    /// The `SkM44` `gfx::TransformToSkM44(affine.ToTransform())` produces:
    /// each entry rounded to float.
    pub fn to_m44(&self) -> M44 {
        let m = self.0.map(|v| v as f32);
        M44::affine(m[0], m[1], m[2], m[3], m[4], m[5])
    }

    /// `AffineTransform::ToSkMatrix()`.
    pub fn to_mat(&self) -> Mat {
        let m = self.0.map(|v| v as f32);
        Mat { sx: m[0], ky: m[1], kx: m[2], sy: m[3], tx: m[4], ty: m[5] }
    }

    pub fn map_point(&self, p: [f64; 2]) -> [f64; 2] {
        let m = &self.0;
        [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
    }

    pub fn is_invertible(&self) -> bool {
        let m = &self.0;
        let det = m[0] * m[3] - m[1] * m[2];
        det != 0.0 && det.is_finite()
    }

    pub fn x_scale_squared(&self) -> f64 {
        self.0[0] * self.0[0] + self.0[1] * self.0[1]
    }

    pub fn y_scale_squared(&self) -> f64 {
        self.0[2] * self.0[2] + self.0[3] * self.0[3]
    }
}

/// `gfx::SinCosDegrees`: exact values at multiples of 90 degrees.
fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let r = degrees.rem_euclid(360.0);
    if r == 0.0 {
        (0.0, 1.0)
    } else if r == 90.0 {
        (1.0, 0.0)
    } else if r == 180.0 {
        (0.0, -1.0)
    } else if r == 270.0 {
        (-1.0, 0.0)
    } else {
        let rad = degrees * (core::f64::consts::PI / 180.0);
        (rad.sin(), rad.cos())
    }
}

/// The CSS transform operations Blink derives from an SVG `transform`
/// attribute (`SVGTransformList` → `TransformOperations`), applied to a
/// `gfx::Transform` in double with float-rounded arguments
/// (`TranslateTransformOperation::Apply` and friends).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformOp {
    Translate(f32, f32),
    Scale(f32, f32),
    Rotate(f64),
    RotateAround(f64, f32, f32),
    SkewX(f64),
    SkewY(f64),
    Matrix([f32; 6]),
}

/// Blink's SVG number parser (`ParseNumber`): float values.
pub fn parse_numbers(s: &str) -> Option<Vec<f32>> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        if bytes[i] == b'+' || bytes[i] == b'-' {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            let save = i;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            if i < bytes.len() && bytes[i].is_ascii_digit() {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            } else {
                i = save;
            }
        }
        if i == start {
            return None;
        }
        out.push(s[start..i].parse::<f32>().ok()?);
    }
    Some(out)
}

/// Parses an SVG transform list into operations.
pub fn parse_transform_list(s: &str) -> Result<Vec<TransformOp>, SvgError> {
    let mut ops = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(').ok_or_else(|| SvgError(format!("transform list {s:?}")))?;
        let close = rest.find(')').ok_or_else(|| SvgError(format!("transform list {s:?}")))?;
        let name = rest[..open].trim();
        let args = parse_numbers(&rest[open + 1..close]).ok_or_else(|| SvgError(format!("transform list {s:?}")))?;
        let op = match (name, args.len()) {
            ("translate", 1) => TransformOp::Translate(args[0], 0.0),
            ("translate", 2) => TransformOp::Translate(args[0], args[1]),
            ("scale", 1) => TransformOp::Scale(args[0], args[0]),
            ("scale", 2) => TransformOp::Scale(args[0], args[1]),
            ("rotate", 1) => TransformOp::Rotate(f64::from(args[0])),
            ("rotate", 3) => TransformOp::RotateAround(f64::from(args[0]), args[1], args[2]),
            ("skewX", 1) => TransformOp::SkewX(f64::from(args[0])),
            ("skewY", 1) => TransformOp::SkewY(f64::from(args[0])),
            ("matrix", 6) => TransformOp::Matrix([args[0], args[1], args[2], args[3], args[4], args[5]]),
            _ => return Err(SvgError(format!("transform list {s:?}"))),
        };
        ops.push(op);
        rest = rest[close + 1..].trim_start_matches([' ', '\t', '\r', '\n', ',']);
    }
    Ok(ops)
}

/// `TransformOperations::Apply` on a `gfx::Transform` (double 4×4, the 2D
/// part), the SVG attribute's transform as `TransformHelper::ComputeTransform`
/// returns it (`transform-origin` is the viewport origin for SVG elements).
pub fn apply_transform_ops(ops: &[TransformOp]) -> Affine {
    let mut t = Affine::IDENTITY;
    for op in ops {
        match *op {
            TransformOp::Translate(x, y) => t.translate(f64::from(x), f64::from(y)),
            TransformOp::Scale(x, y) => t.scale(f64::from(x), f64::from(y)),
            TransformOp::Rotate(deg) => rotate(&mut t, deg),
            TransformOp::RotateAround(deg, cx, cy) => {
                t.translate(f64::from(cx), f64::from(cy));
                rotate(&mut t, deg);
                t.translate(-f64::from(cx), -f64::from(cy));
            }
            TransformOp::SkewX(deg) => {
                let s = (deg * (core::f64::consts::PI / 180.0)).tan();
                t.pre_concat(&Affine([1.0, 0.0, s, 1.0, 0.0, 0.0]));
            }
            TransformOp::SkewY(deg) => {
                let s = (deg * (core::f64::consts::PI / 180.0)).tan();
                t.pre_concat(&Affine([1.0, s, 0.0, 1.0, 0.0, 0.0]));
            }
            TransformOp::Matrix(m) => t.pre_concat(&Affine(m.map(f64::from))),
        }
    }
    t
}

fn rotate(t: &mut Affine, deg: f64) {
    let (s, c) = sin_cos_degrees(deg);
    if s == 0.0 && c == 1.0 {
        return;
    }
    t.pre_concat(&Affine([c, s, -s, c, 0.0, 0.0]));
}

/// The element transform from a raw `transform` attribute.
pub fn element_transform(attr: Option<&str>) -> Result<Affine, SvgError> {
    match attr {
        None => Ok(Affine::IDENTITY),
        Some(s) if s.trim().is_empty() || s.trim() == "none" => Ok(Affine::IDENTITY),
        Some(s) => Ok(apply_transform_ops(&parse_transform_list(s)?)),
    }
}

// ---------------------------------------------------------------------------
// Sizing

/// `LayoutUnit` (1/64 px, `FromFloatFloor`).
#[inline]
fn layout_unit_floor(v: f32) -> f64 {
    libm::floorf(v * 64.0) as f64 / 64.0
}

/// `LayoutUnit::Round()` for a non-negative value.
#[inline]
fn layout_unit_round(v: f64) -> i32 {
    let raw = (v * 64.0) as i64;
    ((raw + 32) >> 6) as i32
}

/// `gfx::ToRoundedInt`: `std::round` (half away from zero).
#[inline]
fn rounded_int(v: f32) -> i32 {
    libm::roundf(v) as i32
}

/// What `SVGImage::DrawInternal` and the paint record settle for one
/// `drawImage(img, 0, 0, canvas_w, canvas_h)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDraw {
    /// The concrete object size (`container_size_`).
    pub container: [f32; 2],
    /// `ToRoundedSize(container)`: the frame view's size.
    pub rounded: [i32; 2],
    /// `SkM44::RectToRect(unzoomed_src, dst)`.
    pub src_to_dst: M44,
    /// `ToEnclosingRect(dst_rect)`: the non-AA clip around the picture.
    pub dst_clip: [i32; 4],
    /// The root's replaced-content transform (`None` when identity).
    pub root_transform: Option<Affine>,
    /// The non-scaling-stroke container scale (`SetContainerScale`).
    pub container_scale: [f32; 2],
}

/// The natural sizing of the document: absolute `width`/`height` and the
/// viewBox (or synthesized viewBox) aspect ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NaturalSizing {
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// `viewBox` as written, or `None`.
    pub view_box: Option<[f32; 4]>,
    pub preserve_aspect_ratio: PreserveAspectRatio,
}

/// `SVGPreserveAspectRatio` (align, meet-or-slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreserveAspectRatio {
    /// 0 none, 1 xMinYMin, 2 xMidYMin, 3 xMaxYMin, 4 xMinYMid, 5 xMidYMid,
    /// 6 xMaxYMid, 7 xMinYMax, 8 xMidYMax, 9 xMaxYMax.
    pub align: u8,
    pub slice: bool,
}

impl PreserveAspectRatio {
    pub const DEFAULT: PreserveAspectRatio = PreserveAspectRatio { align: 5, slice: false };

    pub fn parse(s: Option<&str>) -> PreserveAspectRatio {
        let Some(s) = s else { return Self::DEFAULT };
        let mut it = s.split_whitespace();
        let mut out = Self::DEFAULT;
        let mut first = it.next();
        if first == Some("defer") {
            first = it.next();
        }
        if let Some(a) = first {
            out.align = match a {
                "none" => 0,
                "xMinYMin" => 1,
                "xMidYMin" => 2,
                "xMaxYMin" => 3,
                "xMinYMid" => 4,
                "xMidYMid" => 5,
                "xMaxYMid" => 6,
                "xMinYMax" => 7,
                "xMidYMax" => 8,
                "xMaxYMax" => 9,
                _ => return Self::DEFAULT,
            };
        }
        if let Some(m) = it.next() {
            out.slice = m == "slice";
        }
        out
    }

    /// `SVGPreserveAspectRatio::ComputeTransform` (double).
    pub fn compute_transform(&self, view_box: [f32; 4], viewport: [f32; 2]) -> Affine {
        let (lx, ly, lw, lh) = (f64::from(view_box[0]), f64::from(view_box[1]), f64::from(view_box[2]), f64::from(view_box[3]));
        let (pw, ph) = (f64::from(viewport[0]), f64::from(viewport[1]));
        let mut t = Affine::IDENTITY;
        if self.align == 0 {
            t.scale(pw / lw, ph / lh);
            t.translate(-lx, -ly);
            return t;
        }
        let logical_ratio = lw / lh;
        let physical_ratio = pw / ph;
        let x_min = matches!(self.align, 1 | 4 | 7);
        let x_mid = matches!(self.align, 2 | 5 | 8);
        let y_min = matches!(self.align, 1..=3);
        let y_mid = matches!(self.align, 4..=6);
        if (logical_ratio < physical_ratio && !self.slice) || (logical_ratio >= physical_ratio && self.slice) {
            t.scale(ph / lh, ph / lh);
            if x_min {
                t.translate(-lx, -ly);
            } else if x_mid {
                t.translate(-lx - (lw - pw * lh / ph) / 2.0, -ly);
            } else {
                t.translate(-lx - (lw - pw * lh / ph), -ly);
            }
            return t;
        }
        t.scale(pw / lw, pw / lw);
        if y_min {
            t.translate(-lx, -ly);
        } else if y_mid {
            t.translate(-lx, -ly - (lh - ph * lw / pw) / 2.0);
        } else {
            t.translate(-lx, -ly - (lh - ph * lw / pw));
        }
        t
    }
}

/// `ConcreteObjectSize` of an `SVGImage` against a default object size
/// (the canvas), then everything `SVGImage::DrawInternal` and the root's
/// layout decide.
pub fn image_draw(sizing: &NaturalSizing, canvas: [u32; 2]) -> ImageDraw {
    let default_size = [canvas[0] as f32, canvas[1] as f32];
    // LayoutSVGRoot::UnscaledNaturalSizingInfo.
    let size = [sizing.width.unwrap_or(0.0), sizing.height.unwrap_or(0.0)];
    let has_w = sizing.width.is_some();
    let has_h = sizing.height.is_some();
    let aspect = if size[0] > 0.0 && size[1] > 0.0 {
        Some(size)
    } else {
        match sizing.view_box {
            Some(vb) if vb[2] > 0.0 && vb[3] > 0.0 => Some([vb[2], vb[3]]),
            _ => None,
        }
    };
    // SvgSizingWithPreserveAspectRatioNone is experimental: with
    // preserveAspectRatio="none" and a missing dimension the natural sizing is
    // cleared.
    let (has_w, has_h, aspect) = if (!has_w || !has_h) && sizing.preserve_aspect_ratio.align == 0 {
        (false, false, None)
    } else {
        (has_w, has_h, aspect)
    };
    // ConcreteObjectSizeImpl (float).
    let container = if has_w && has_h {
        size
    } else if has_w {
        match aspect {
            None => [size[0], default_size[1]],
            Some(r) => [size[0], size[0] * r[1] / r[0]],
        }
    } else if has_h {
        match aspect {
            None => [default_size[0], size[1]],
            Some(r) => [size[1] * r[0] / r[1], size[1]],
        }
    } else if let Some(r) = aspect {
        let solution_width = default_size[1] * r[0] / r[1];
        if solution_width <= default_size[0] {
            [solution_width, default_size[1]]
        } else {
            [default_size[0], default_size[0] * r[1] / r[0]]
        }
    } else {
        default_size
    };
    let rounded = [rounded_int(container[0]), rounded_int(container[1])];
    // DrawForContainer: unzoomed_src scaled by the residual.
    let residual = [rounded[0] as f32 / container[0], rounded[1] as f32 / container[1]];
    let unzoomed_src = [0.0, 0.0, container[0] * residual[0], container[1] * residual[1]];
    let dst = [0.0, 0.0, default_size[0], default_size[1]];
    let src_to_dst = M44::rect_to_rect(unzoomed_src, dst);
    let container_scale = [src_to_dst.rc(0, 0) * residual[0], src_to_dst.rc(1, 1) * residual[1]];
    // LayoutSVGRoot: the box is the container size in LayoutUnits (floor).
    let box_size = [layout_unit_floor(container[0]), layout_unit_floor(container[1])];
    // SVGRootPainter::TransformToPixelSnappedBorderBox with paint offset 0.
    let snapped = [layout_unit_round(box_size[0]), layout_unit_round(box_size[1])];
    let mut paint_offset_to_border_box = Affine::IDENTITY;
    if box_size[0] > 0.0 && box_size[1] > 0.0 {
        // IsDocumentElement: scale by snapped / size (float division).
        let sx = snapped[0] as f32 / box_size[0] as f32;
        let sy = snapped[1] as f32 / box_size[1] as f32;
        paint_offset_to_border_box.scale(f64::from(sx), f64::from(sy));
    }
    // LayoutSVGRoot::BuildLocalToBorderBoxTransform.
    let content_size = [box_size[0] as f32, box_size[1] as f32];
    let view_box = sizing.view_box.filter(|vb| vb[2] > 0.0 && vb[3] > 0.0).or({
        // ShouldSynthesizeViewBox: non-relative width/height in an SVGImage.
        if has_w && has_h && size[0] > 0.0 && size[1] > 0.0 {
            Some([0.0, 0.0, size[0], size[1]])
        } else {
            None
        }
    });
    let mut local_to_border_box = match view_box {
        Some(vb) if content_size[0] > 0.0 && content_size[1] > 0.0 => {
            sizing.preserve_aspect_ratio.compute_transform(vb, content_size)
        }
        _ => Affine::IDENTITY,
    };
    // view_to_border_box: translate(content offset 0, 0) scaled by currentScale 1.
    local_to_border_box.post_concat(&Affine::IDENTITY);
    paint_offset_to_border_box.pre_concat(&local_to_border_box);
    let root_transform = if paint_offset_to_border_box.is_identity() {
        None
    } else {
        Some(paint_offset_to_border_box)
    };
    let enclosing = |v: f32, up: bool| if up { libm::ceilf(v) as i32 } else { libm::floorf(v) as i32 };
    ImageDraw {
        container,
        rounded,
        src_to_dst,
        dst_clip: [enclosing(dst[0], false), enclosing(dst[1], false), enclosing(dst[2], true), enclosing(dst[3], true)],
        root_transform,
        container_scale,
    }
}

// ---------------------------------------------------------------------------
// Paint flags and ops

/// A gradient as Blink's `Gradient::CreateShaderInternal` hands it to Skia:
/// stops padded to 0 and 1, colors as `SkColor4f` (sRGB, unpremultiplied),
/// the local matrix (the user-space transform) as an `SkMatrix`.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub colors: Vec<[f32; 4]>,
    pub positions: Vec<f32>,
    /// 0 clamp (pad), 1 repeat, 2 mirror (reflect).
    pub tile: u8,
    pub local_matrix: Mat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientKind {
    Linear { p0: [f32; 2], p1: [f32; 2] },
    /// `MakeTwoPointConical(p0 = focal, r0 = focal radius, p1 = center, r1 = radius)`.
    Conical { p0: [f32; 2], r0: f32, p1: [f32; 2], r1: f32 },
}

/// `cc::PaintFlags` as the SVG painters fill it.
#[derive(Debug, Clone, PartialEq)]
pub struct Flags {
    /// `SkColor4f` (unpremultiplied, sRGB).
    pub color: [f32; 4],
    pub shader: Option<Gradient>,
    pub anti_alias: bool,
    pub dither: bool,
    pub stroke: Option<StrokeFlags>,
}

/// `StrokeData::SetupPaint`.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeFlags {
    pub width: f32,
    /// 0 butt, 1 round, 2 square.
    pub cap: u8,
    /// 0 miter, 1 round, 2 bevel.
    pub join: u8,
    pub miter: f32,
    /// `SkDashPathEffect::Make(intervals, phase)`.
    pub dash: Option<(Vec<f32>, f32)>,
}

/// An image filter chain the effect node carries (`SaveLayerOp` flags).
#[derive(Debug, Clone, PartialEq)]
pub enum ImageFilter {
    /// `ColorFilter(LinearToSRGB, Crop(decal, Blur(sigma, decal,
    /// ColorFilter(SRGBToLinear, source))))`: Blink's `feGaussianBlur` with
    /// `color-interpolation-filters: linearRGB`. `crop` is the primitive
    /// subregion ∩ filter region in the layer's parameter space.
    GaussianBlur { sigma: [f32; 2], crop: [f32; 4], linear: bool },
}

/// A text run to draw (shaped and rasterized by the caller through pm-text).
#[derive(Debug, Clone)]
pub struct TextItem {
    pub node: usvg::Text,
}

/// One cc paint op.
#[derive(Debug, Clone)]
pub enum Op {
    Save,
    Restore,
    Concat(M44),
    Translate(f32, f32),
    ClipRect { rect: [f32; 4], aa: bool },
    ClipRRect { rrect: SkRRect, aa: bool },
    ClipPath { path: SkPath, rule: Rule, aa: bool },
    SaveLayerAlpha(f32),
    SaveLayer { filter: ImageFilter },
    DrawRect { rect: [f32; 4], flags: Flags },
    DrawOval { rect: [f32; 4], flags: Flags },
    DrawPath { path: SkPath, rule: Rule, flags: Flags },
    DrawText(Box<TextItem>),
}

// ---------------------------------------------------------------------------
// Property tree

#[derive(Debug, Clone)]
struct TransformNode {
    parent: Option<usize>,
    matrix: Affine,
}

#[derive(Debug, Clone)]
struct ClipNode {
    parent: Option<usize>,
    transform: usize,
    /// `PaintClipRect()`: a rounded rect, or a plain rect.
    paint_rect: ClipRectKind,
    clip_path: Option<(SkPath, Rule)>,
}

#[derive(Debug, Clone, PartialEq)]
enum ClipRectKind {
    Infinite,
    Rect([f32; 4]),
    RRect(SkRRect),
}

#[derive(Debug, Clone)]
struct EffectNode {
    parent: Option<usize>,
    transform: usize,
    output_clip: Option<usize>,
    kind: EffectKind,
}

#[derive(Debug, Clone)]
enum EffectKind {
    Root,
    Opacity(f32),
    Filter(ImageFilter),
}

/// The property tree state of a display item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    transform: usize,
    clip: usize,
    effect: usize,
}

#[derive(Debug, Default)]
struct Tree {
    transforms: Vec<TransformNode>,
    clips: Vec<ClipNode>,
    effects: Vec<EffectNode>,
}

impl Tree {
    fn ancestors<'a>(&'a self, nodes: &'a [Option<usize>], mut i: usize) -> impl Iterator<Item = usize> + 'a {
        let mut done = false;
        core::iter::from_fn(move || {
            if done {
                return None;
            }
            let cur = i;
            match nodes[i] {
                Some(p) => i = p,
                None => done = true,
            }
            Some(cur)
        })
    }

    fn transform_parents(&self) -> Vec<Option<usize>> {
        self.transforms.iter().map(|t| t.parent).collect()
    }

    fn clip_parents(&self) -> Vec<Option<usize>> {
        self.clips.iter().map(|t| t.parent).collect()
    }

    fn effect_parents(&self) -> Vec<Option<usize>> {
        self.effects.iter().map(|t| t.parent).collect()
    }

    /// `GeometryMapper::SourceToDestinationProjection(target, current)` when
    /// `current` is an ancestor of `target`: the product of the local
    /// matrices in double.
    fn projection(&self, target: usize, current: usize) -> Option<Affine> {
        let parents = self.transform_parents();
        let mut chain = Vec::new();
        let mut found = false;
        for n in self.ancestors(&parents, target) {
            if n == current {
                found = true;
                break;
            }
            chain.push(n);
        }
        if !found {
            return None;
        }
        let mut m = Affine::IDENTITY;
        for n in chain.iter().rev() {
            m.pre_concat(&self.transforms[*n].matrix);
        }
        Some(m)
    }
}

// ---------------------------------------------------------------------------
// Display items

#[derive(Debug, Clone)]
struct Item {
    state: State,
    ops: Vec<Op>,
}

/// `PaintChunksToCcLayer::ConversionContext`.
struct Conversion<'a> {
    tree: &'a Tree,
    ops: Vec<Op>,
    current_transform: usize,
    current_clip: usize,
    current_effect: usize,
    previous_transform: Option<usize>,
    stack: Vec<StateEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Clip,
    ClipOmitted,
    Effect,
}

#[derive(Debug, Clone, Copy)]
struct StateEntry {
    kind: EntryKind,
    transform: usize,
    clip: usize,
    effect: usize,
    previous_transform: Option<usize>,
}

impl<'a> Conversion<'a> {
    fn new(tree: &'a Tree) -> Self {
        Self {
            tree,
            ops: Vec::new(),
            current_transform: 0,
            current_clip: 0,
            current_effect: 0,
            previous_transform: None,
            stack: Vec::new(),
        }
    }

    fn is_ancestor(parents: &[Option<usize>], ancestor: usize, node: usize) -> bool {
        let mut i = node;
        loop {
            if i == ancestor {
                return true;
            }
            match parents[i] {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    fn push_state(&mut self, kind: EntryKind) {
        self.stack.push(StateEntry {
            kind,
            transform: self.current_transform,
            clip: self.current_clip,
            effect: self.current_effect,
            previous_transform: self.previous_transform,
        });
        self.previous_transform = None;
    }

    fn pop_state(&mut self) {
        let e = self.stack.pop().expect("state");
        if e.kind != EntryKind::ClipOmitted {
            self.ops.push(Op::Restore);
        }
        self.current_transform = e.transform;
        self.previous_transform = e.previous_transform;
        self.current_clip = e.clip;
        self.current_effect = e.effect;
    }

    fn end_transform(&mut self) {
        if let Some(prev) = self.previous_transform.take() {
            self.ops.push(Op::Restore);
            self.current_transform = prev;
        }
    }

    /// `ApplyTransform`: concatenates the projection from `target` to the
    /// current transform without changing the current transform.
    fn apply_transform(&mut self, target: usize) {
        if target == self.current_transform {
            return;
        }
        let projection = self.tree.projection(target, self.current_transform).unwrap_or(Affine::IDENTITY);
        push_projection(&mut self.ops, &projection);
    }

    fn switch_to_transform(&mut self, target: usize) {
        if target == self.current_transform {
            return;
        }
        self.end_transform();
        if target == self.current_transform {
            return;
        }
        let projection = self.tree.projection(target, self.current_transform).unwrap_or(Affine::IDENTITY);
        if projection.is_identity() {
            return;
        }
        self.ops.push(Op::Save);
        push_projection(&mut self.ops, &projection);
        self.previous_transform = Some(self.current_transform);
        self.current_transform = target;
    }

    fn end_clip(&mut self) {
        self.end_transform();
        self.pop_state();
    }

    fn end_clips(&mut self) {
        while matches!(self.stack.last(), Some(e) if e.kind != EntryKind::Effect) {
            self.end_clip();
        }
    }

    fn end_effect(&mut self) {
        self.end_transform();
        self.pop_state();
    }

    fn start_clip(&mut self, combined: &ClipRectKind, lowest: usize) {
        if *combined == ClipRectKind::Infinite {
            self.push_state(EntryKind::ClipOmitted);
            return;
        }
        let local_transform = self.tree.clips[lowest].transform;
        if local_transform != self.current_transform {
            self.end_transform();
        }
        self.ops.push(Op::Save);
        self.apply_transform(local_transform);
        match combined {
            ClipRectKind::RRect(rr) => self.ops.push(Op::ClipRRect { rrect: *rr, aa: true }),
            ClipRectKind::Rect(r) => self.ops.push(Op::ClipRect { rect: *r, aa: true }),
            ClipRectKind::Infinite => {}
        }
        if let Some((path, rule)) = &self.tree.clips[lowest].clip_path {
            self.ops.push(Op::ClipPath { path: path.clone(), rule: *rule, aa: true });
        }
        self.push_state(EntryKind::Clip);
        self.current_transform = local_transform;
    }

    fn switch_to_clip(&mut self, target: usize) {
        if target == self.current_clip {
            return;
        }
        let parents = self.tree.clip_parents();
        // Step 1: exit clips until the current clip is an ancestor of the target.
        while !Self::is_ancestor(&parents, self.current_clip, target) {
            match self.stack.last() {
                Some(e) if e.kind != EntryKind::Effect => self.end_clip(),
                _ => break,
            }
        }
        if target == self.current_clip {
            return;
        }
        // Step 2: the clips between target and current, top-down.
        let mut pending = Vec::new();
        let mut c = target;
        while c != self.current_clip {
            pending.push(c);
            match parents[c] {
                Some(p) => c = p,
                None => break,
            }
        }
        // Step 3: apply, combining consecutive plain rects in one transform space.
        let mut combined = self.tree.clips[*pending.last().unwrap()].paint_rect.clone();
        let mut lowest = *pending.last().unwrap();
        for &sub in pending.iter().rev().skip(1) {
            if let Some(c) = combine_clip(self.tree, &combined, lowest, sub) {
                combined = c;
                lowest = sub;
            } else {
                self.start_clip(&combined, lowest);
                combined = self.tree.clips[sub].paint_rect.clone();
                lowest = sub;
            }
        }
        self.start_clip(&combined, lowest);
        self.current_clip = target;
    }

    fn start_effect(&mut self, e: usize) {
        let effect = &self.tree.effects[e];
        if let Some(oc) = effect.output_clip {
            self.switch_to_clip(oc);
            self.switch_to_transform(effect.transform);
        } else {
            self.end_clips();
        }
        match &effect.kind {
            EffectKind::Opacity(a) => self.ops.push(Op::SaveLayerAlpha(*a)),
            EffectKind::Filter(f) => self.ops.push(Op::SaveLayer { filter: f.clone() }),
            EffectKind::Root => {}
        }
        let input_clip = self.current_clip;
        self.push_state(EntryKind::Effect);
        self.current_clip = input_clip;
        self.current_effect = e;
    }

    fn switch_to_effect(&mut self, target: usize) {
        if target == self.current_effect {
            return;
        }
        let parents = self.tree.effect_parents();
        while !Self::is_ancestor(&parents, self.current_effect, target) {
            self.end_clips();
            self.end_effect();
        }
        let mut pending = Vec::new();
        let mut e = target;
        while e != self.current_effect {
            pending.push(e);
            match parents[e] {
                Some(p) => e = p,
                None => break,
            }
        }
        for &e in pending.iter().rev() {
            self.start_effect(e);
        }
    }

    fn convert(&mut self, item: &Item) {
        self.switch_to_effect(item.state.effect);
        self.switch_to_clip(item.state.clip);
        self.switch_to_transform(item.state.transform);
        self.ops.extend(item.ops.iter().cloned());
    }

    fn finish(mut self) -> Vec<Op> {
        while let Some(e) = self.stack.last() {
            match e.kind {
                EntryKind::Effect => self.end_effect(),
                _ => self.end_clip(),
            }
        }
        self.end_transform();
        self.ops
    }
}

/// `CombineClip`: consecutive plain rectangle clips in the same transform
/// space combine by intersection.
fn combine_clip(tree: &Tree, combined: &ClipRectKind, lowest: usize, sub: usize) -> Option<ClipRectKind> {
    let (a, b) = (&tree.clips[lowest], &tree.clips[sub]);
    if a.transform != b.transform || a.clip_path.is_some() || b.clip_path.is_some() {
        return None;
    }
    match (combined, &b.paint_rect) {
        (ClipRectKind::Infinite, other) => Some(other.clone()),
        (other, ClipRectKind::Infinite) => Some(other.clone()),
        (ClipRectKind::Rect(x), ClipRectKind::Rect(y)) => {
            let r = [x[0].max(y[0]), x[1].max(y[1]), x[2].min(y[2]), x[3].min(y[3])];
            Some(ClipRectKind::Rect([r[0], r[1], r[2].max(r[0]), r[3].max(r[1])]))
        }
        _ => None,
    }
}

/// `ApplyTransform`'s op: a `TranslateOp` for an identity-or-translation
/// (`gfx::Transform::IsIdentityOr2dTranslation`), else a `ConcatOp`.
fn push_projection(ops: &mut Vec<Op>, p: &Affine) {
    if p.is_identity_or_translation() {
        let (tx, ty) = (clamp_f32(p.0[4]), clamp_f32(p.0[5]));
        if tx != 0.0 || ty != 0.0 {
            ops.push(Op::Translate(tx, ty));
        }
    } else {
        ops.push(Op::Concat(p.to_m44()));
    }
}

// ---------------------------------------------------------------------------
// Building the tree from the document

/// Raw element attributes by id (the ids `annotate` injected).
pub struct RawElements<'a> {
    pub doc: &'a usvg::roxmltree::Document<'a>,
    pub by_id: HashMap<String, usvg::roxmltree::NodeId>,
}

impl<'a> RawElements<'a> {
    pub fn new(doc: &'a usvg::roxmltree::Document<'a>) -> Self {
        let mut by_id = HashMap::new();
        for n in doc.descendants().filter(|n| n.is_element()) {
            if let Some(id) = n.attribute("id") {
                by_id.insert(id.to_string(), n.id());
            }
        }
        Self { doc, by_id }
    }

    pub fn get(&self, id: &str) -> Option<usvg::roxmltree::Node<'a, 'a>> {
        self.by_id.get(id).map(|n| self.doc.get_node(*n).unwrap())
    }
}

/// The document's natural sizing from the root `<svg>`.
pub fn natural_sizing(doc: &usvg::roxmltree::Document<'_>) -> NaturalSizing {
    let root = doc.root_element();
    let len = |name: &str| -> Option<f32> {
        let v = root.attribute(name)?.trim();
        if v.ends_with('%') || v == "auto" || v.is_empty() {
            return None;
        }
        let num = v.trim_end_matches("px");
        let n: f32 = num.parse().ok()?;
        (n >= 0.0).then_some(n)
    };
    let view_box = root.attribute("viewBox").and_then(|v| {
        let n = parse_numbers(v)?;
        (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
    });
    NaturalSizing {
        width: len("width"),
        height: len("height"),
        view_box,
        preserve_aspect_ratio: PreserveAspectRatio::parse(root.attribute("preserveAspectRatio")),
    }
}

/// A shape's geometry as Blink's `LayoutSVGShape` classifies it.
#[derive(Debug, Clone)]
pub enum Geometry {
    /// `<rect>` without radii: `DrawRect(ObjectBoundingBox())`.
    Rect([f32; 4]),
    /// `<circle>`/`<ellipse>`: `DrawOval(ObjectBoundingBox())`.
    Oval([f32; 4]),
    /// Everything else: `DrawPath`.
    Path(SkPath),
}

/// A shape's resolved style the record needs.
pub struct ShapeStyle<'a> {
    pub fill: Option<&'a usvg::Fill>,
    pub stroke: Option<&'a usvg::Stroke>,
    pub paint_order: usvg::PaintOrder,
    pub anti_alias: bool,
}

/// How the record builder resolves paint servers for a shape.
pub trait PaintResolver {
    /// The gradient for a usvg paint, in the shape's user space.
    fn gradient(&self, paint: &usvg::Paint, bbox: Option<[f32; 4]>) -> Result<Option<Gradient>, SvgError>;
}

/// `SVGObjectPainter::PreparePaint` for a color or paint server.
fn prepare_flags(
    paint: &usvg::Paint,
    opacity: f32,
    bbox: Option<[f32; 4]>,
    resolver: &dyn PaintResolver,
    anti_alias: bool,
) -> Result<Option<Flags>, SvgError> {
    match paint {
        usvg::Paint::Color(c) => {
            // Color::SetAlpha(Alpha() * opacity) then toSkColor4f.
            let a = 1.0f32 * opacity;
            Ok(Some(Flags {
                color: [f32::from(c.red) / 255.0, f32::from(c.green) / 255.0, f32::from(c.blue) / 255.0, a],
                shader: None,
                anti_alias,
                dither: false,
                stroke: None,
            }))
        }
        other => match resolver.gradient(other, bbox)? {
            Some(g) => Ok(Some(Flags {
                color: [0.0, 0.0, 0.0, libm::roundf(opacity * 255.0).clamp(0.0, 255.0) / 255.0],
                shader: Some(g),
                anti_alias,
                // Gradient::ApplyToFlags: gradients are always dithered.
                dither: true,
                stroke: None,
            })),
            None => Ok(None),
        },
    }
}

/// `StrokeData` from a usvg stroke (`SVGLayoutSupport::ApplyStrokeStyleToStrokeData`).
fn stroke_flags(s: &usvg::Stroke) -> StrokeFlags {
    let dash = s.dasharray().map(|d| {
        let n = d.len();
        let count = if n % 2 == 0 { n } else { n * 2 };
        let intervals: Vec<f32> = (0..count).map(|i| d[i % n]).collect();
        (intervals, s.dashoffset())
    });
    StrokeFlags {
        width: s.width().get(),
        cap: match s.linecap() {
            usvg::LineCap::Butt => 0,
            usvg::LineCap::Round => 1,
            usvg::LineCap::Square => 2,
        },
        join: match s.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => 0,
            usvg::LineJoin::Round => 1,
            usvg::LineJoin::Bevel => 2,
        },
        miter: s.miterlimit().get(),
        dash,
    }
}

/// The draw ops of one shape (`SVGShapePainter::PaintShape`).
pub fn shape_ops(
    geometry: &Geometry,
    style: &ShapeStyle<'_>,
    fill_rule: Rule,
    bbox: Option<[f32; 4]>,
    resolver: &dyn PaintResolver,
) -> Result<Vec<Op>, SvgError> {
    let mut ops = Vec::new();
    let draw = |flags: Flags, ops: &mut Vec<Op>| match geometry {
        Geometry::Rect(r) => ops.push(Op::DrawRect { rect: *r, flags }),
        Geometry::Oval(r) => ops.push(Op::DrawOval { rect: *r, flags }),
        Geometry::Path(p) => ops.push(Op::DrawPath { path: p.clone(), rule: fill_rule, flags }),
    };
    let order: [bool; 2] = match style.paint_order {
        usvg::PaintOrder::FillAndStroke => [true, false],
        usvg::PaintOrder::StrokeAndFill => [false, true],
    };
    for is_fill in order {
        if is_fill {
            if let Some(f) = style.fill
                && let Some(flags) = prepare_flags(f.paint(), f.opacity().get(), bbox, resolver, style.anti_alias)?
            {
                draw(flags, &mut ops);
            }
        } else if let Some(s) = style.stroke
            && s.width().get() > 0.0
            && let Some(mut flags) = prepare_flags(s.paint(), s.opacity().get(), bbox, resolver, style.anti_alias)?
        {
            flags.stroke = Some(stroke_flags(s));
            draw(flags, &mut ops);
        }
    }
    Ok(ops)
}

/// The record builder: walks the usvg tree with the raw document, builds the
/// property tree and display items, and converts them to ops.
pub struct RecordBuilder<'a> {
    pub raw: &'a RawElements<'a>,
    tree: Tree,
    items: Vec<Item>,
}

impl<'a> RecordBuilder<'a> {
    /// A builder whose root transform node is the replaced-content transform
    /// (or identity) and whose root clip is the frame view's rect.
    pub fn new(raw: &'a RawElements<'a>, draw: &ImageDraw) -> Self {
        let mut tree = Tree::default();
        // Transform 0: Root; 1: the replaced-content transform (or Root again).
        tree.transforms.push(TransformNode { parent: None, matrix: Affine::IDENTITY });
        let root_t = match draw.root_transform {
            Some(m) => {
                tree.transforms.push(TransformNode { parent: Some(0), matrix: m });
                1
            }
            None => 0,
        };
        // Clip 0: Root (infinite); 1: the LayoutView's overflow clip.
        tree.clips.push(ClipNode { parent: None, transform: 0, paint_rect: ClipRectKind::Infinite, clip_path: None });
        tree.clips.push(ClipNode {
            parent: Some(0),
            transform: 0,
            paint_rect: ClipRectKind::Rect([0.0, 0.0, draw.rounded[0] as f32, draw.rounded[1] as f32]),
            clip_path: None,
        });
        tree.effects.push(EffectNode { parent: None, transform: 0, output_clip: None, kind: EffectKind::Root });
        let _ = root_t;
        Self { raw, tree, items: Vec::new() }
    }

    /// The initial state for the document's content.
    pub fn root_state(&self) -> ItemState {
        ItemState { transform: if self.tree.transforms.len() > 1 { 1 } else { 0 }, clip: 1, effect: 0 }
    }

    /// A transform node for an element's own transform (`None` when identity).
    pub fn push_transform(&mut self, parent: usize, m: Affine) -> usize {
        if m.is_identity() {
            return parent;
        }
        self.tree.transforms.push(TransformNode { parent: Some(parent), matrix: m });
        self.tree.transforms.len() - 1
    }

    /// The `clip-path` clip node of an element (`UpdateClipPathClip`): the
    /// path (in the element's user space); a rect/rrect/oval path becomes
    /// a rounded-rect clip, anything else an enclosing rect plus the path.
    pub fn push_clip_path(&mut self, parent: usize, transform: usize, path: &SkPath, rule: Rule) -> usize {
        let paint_rect;
        let clip_path;
        if let Some(rr) = path_to_rrect(path) {
            paint_rect = ClipRectKind::RRect(rr);
            clip_path = None;
        } else {
            let b = path.bounds().unwrap_or([0.0; 4]);
            let enclosing = [libm::floorf(b[0]), libm::floorf(b[1]), libm::ceilf(b[2]), libm::ceilf(b[3])];
            paint_rect = ClipRectKind::Rect(enclosing);
            clip_path = Some((path.clone(), rule));
        }
        self.tree.clips.push(ClipNode { parent: Some(parent), transform, paint_rect, clip_path });
        self.tree.clips.len() - 1
    }

    pub fn push_effect(&mut self, parent: usize, transform: usize, output_clip: usize, kind_opacity: Option<f32>, filter: Option<ImageFilter>) -> usize {
        let kind = match (kind_opacity, filter) {
            (_, Some(f)) => EffectKind::Filter(f),
            (Some(a), None) => EffectKind::Opacity(a),
            (None, None) => EffectKind::Root,
        };
        self.tree.effects.push(EffectNode { parent: Some(parent), transform, output_clip: Some(output_clip), kind });
        self.tree.effects.len() - 1
    }

    pub fn push_item(&mut self, state: ItemState, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        self.items.push(Item { state: State { transform: state.transform, clip: state.clip, effect: state.effect }, ops });
    }

    /// `PaintChunksToCcLayer::Convert` over the items.
    pub fn finish(self) -> Vec<Op> {
        let mut conv = Conversion::new(&self.tree);
        for item in &self.items {
            conv.convert(item);
        }
        conv.finish()
    }
}

/// A display item's property state (public mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemState {
    pub transform: usize,
    pub clip: usize,
    pub effect: usize,
}

/// `PathToRRect`: a rect, rrect, or oval path as a rounded rect.
fn path_to_rrect(path: &SkPath) -> Option<SkRRect> {
    if let Some((r, _)) = path.is_rect() {
        return Some(SkRRect::make_rect(r));
    }
    if let Some(rr) = path.is_rrect() {
        return Some(rr);
    }
    if let Some(r) = path.is_oval() {
        return Some(SkRRect::make_oval(r));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_object_size_contains_the_aspect_ratio() {
        let s = NaturalSizing { width: None, height: None, view_box: Some([0.0, 0.0, 100.0, 60.0]), preserve_aspect_ratio: PreserveAspectRatio::DEFAULT };
        let d = image_draw(&s, [64, 64]);
        assert_eq!(d.container, [64.0, 38.4]);
        assert_eq!(d.rounded, [64, 38]);
        // The root box is 38.390625 tall; the snapped box is 38.
        let m = d.root_transform.unwrap();
        assert!((m.0[0] - 0.63984375).abs() < 1e-12, "{m:?}");
        assert!((m.0[4] - 0.0078125).abs() < 1e-12, "{m:?}");
        assert_eq!(d.src_to_dst.rc(0, 0), 1.0);
    }

    #[test]
    fn explicit_size_needs_no_root_transform() {
        let s = NaturalSizing { width: Some(120.0), height: Some(80.0), view_box: Some([0.0, 0.0, 120.0, 80.0]), preserve_aspect_ratio: PreserveAspectRatio::DEFAULT };
        let d = image_draw(&s, [64, 64]);
        assert_eq!(d.container, [120.0, 80.0]);
        assert!(d.root_transform.is_none());
        assert_eq!(d.src_to_dst.rc(0, 0), 64.0 / 120.0);
        assert_eq!(d.src_to_dst.rc(1, 1), 0.8);
    }

    #[test]
    fn rotate_about_a_point_matches_blink() {
        let ops = parse_transform_list("rotate(20 50 50) scale(0.9)").unwrap();
        let m = apply_transform_ops(&ops);
        let (s, c) = (20f64.to_radians().sin(), 20f64.to_radians().cos());
        assert!((m.0[0] - c * f64::from(0.9f32)).abs() < 1e-12);
        assert!((m.0[1] - s * f64::from(0.9f32)).abs() < 1e-12);
        assert!((m.0[4] - (50.0 - 50.0 * c + 50.0 * s)).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn transform_ops_parse_commas_and_exponents() {
        let ops = parse_transform_list("translate(1e1,-2.5) , matrix(1 0 0 1 3 4)").unwrap();
        assert_eq!(ops, vec![TransformOp::Translate(10.0, -2.5), TransformOp::Matrix([1.0, 0.0, 0.0, 1.0, 3.0, 4.0])]);
    }
}
