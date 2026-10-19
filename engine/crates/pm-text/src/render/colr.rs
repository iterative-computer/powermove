//! Fontations COLR paint graphs, rasterized into the ARGB glyph image.
//!
//! Matches SkTypeface_fontations::generateImage/drawCOLRGlyph: unhinted
//! outlines in font units, no font smoothing or mask gamma, the union of
//! every paint layer's bounds, and whole-pixel glyph placement. This is
//! independent of CoreGraphics' color-font implementation.

use std::sync::Arc;

use kurbo::{Affine, Point, Rect};
use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend, Transform};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

use super::bitmap::{BitmapGlyph, Rgba8};
use super::color::Rgba;
use super::outline::{Coords, FaceRef};

#[derive(PartialEq, Eq, Hash)]
struct Key {
    face: u64,
    index: u32,
    glyph: u32,
    size: u32,
    skew: u32,
    coords: Vec<(u32, u32)>,
    foreground: [u64; 4],
}

#[derive(Default)]
pub struct Cache {
    entries: std::collections::HashMap<Key, Result<Option<Arc<BitmapGlyph>>, String>>,
    bytes: usize,
}

impl Cache {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    pub fn get(
        &mut self,
        face: FaceRef<'_>,
        coords: Coords<'_>,
        id: u32,
        size: f32,
        skew: f32,
        foreground: Rgba,
    ) -> Result<Option<Arc<BitmapGlyph>>, String> {
        let key = Key {
            face: face.id,
            index: face.index,
            glyph: id,
            size: size.to_bits(),
            skew: skew.to_bits(),
            foreground: foreground.map(f64::to_bits),
            coords: match coords {
                Coords::User(c) => c.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
                Coords::Normalized(c) => c.iter().map(|v| (u32::MAX, *v as u16 as u32)).collect(),
            },
        };
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        let result = glyph(face, coords, id, size, skew, foreground).map(|g| g.map(Arc::new));
        if self.bytes > 64 * 1024 * 1024 || self.entries.len() >= 4096 {
            self.clear();
        }
        self.bytes += result
            .as_ref()
            .ok()
            .and_then(|g| g.as_ref())
            .map_or(0, |g| g.image.data.len());
        self.entries.insert(key, result.clone());
        result
    }
}

#[derive(Clone)]
enum Fill {
    Solid([f32; 4]),
    Linear(Point, Point, Vec<(f32, [f32; 4])>, Extend),
    Radial(Point, f32, Point, f32, Vec<(f32, [f32; 4])>, Extend),
    Sweep(Point, f32, f32, Vec<(f32, [f32; 4])>, Extend),
}

enum Op {
    Clip(Option<pm_skia_path::Path>),
    Unclip,
    Fill(Fill, Affine),
    Layer(CompositeMode),
    Unlayer,
}

struct Pen(pm_skia_path::PathBuilder);
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0.cubic_to(a, b, c, d, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

struct Recorder<'a> {
    font: skrifa::FontRef<'a>,
    coords: Vec<NormalizedCoord>,
    matrix: Affine,
    matrices: Vec<Affine>,
    colors: Vec<[f32; 4]>,
    foreground: [f32; 4],
    ops: Vec<Op>,
    bounds: Option<Rect>,
}

fn affine(t: Transform) -> Affine {
    Affine::new([
        t.xx as f64,
        t.yx as f64,
        t.xy as f64,
        t.yy as f64,
        t.dx as f64,
        t.dy as f64,
    ])
}

fn path_transform(m: Affine) -> pm_skia_path::Transform {
    let [a, b, c, d, e, f] = m.as_coeffs().map(|v| v as f32);
    pm_skia_path::Transform::from_row(a, b, c, d, e, f)
}

impl Recorder<'_> {
    fn glyph(&self, id: GlyphId) -> Option<pm_skia_path::Path> {
        let outline = self.font.outline_glyphs().get(id)?;
        let mut pen = Pen(pm_skia_path::PathBuilder::new());
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&self.coords)),
                &mut pen,
            )
            .ok()?;
        pen.0.finish()?.transform(path_transform(self.matrix))
    }

    fn clip(&mut self, path: pm_skia_path::Path) {
        let b = path.bounds();
        let rect = Rect::new(
            b.left() as f64,
            b.top() as f64,
            b.right() as f64,
            b.bottom() as f64,
        );
        self.bounds = Some(self.bounds.map_or(rect, |old| old.union(rect)));
        self.ops.push(Op::Clip(Some(path)));
    }

    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let mut c = if index == 0xffff {
            self.foreground
        } else {
            self.colors
                .get(index as usize)
                .copied()
                .unwrap_or([1., 0., 1., 1.])
        };
        c[3] *= alpha;
        c
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<(f32, [f32; 4])> {
        stops
            .iter()
            .map(|s| (s.offset, self.color(s.palette_index, s.alpha)))
            .collect()
    }
}

impl ColorPainter for Recorder<'_> {
    fn push_transform(&mut self, t: Transform) {
        self.matrices.push(self.matrix);
        self.matrix *= affine(t);
    }
    fn pop_transform(&mut self) {
        self.matrix = self.matrices.pop().expect("balanced COLR transforms");
    }
    fn push_clip_glyph(&mut self, id: GlyphId) {
        if let Some(path) = self.glyph(id) {
            self.clip(path);
        } else {
            self.ops.push(Op::Clip(None));
        }
    }
    fn push_clip_box(&mut self, b: skrifa::raw::types::BoundingBox<f32>) {
        let path =
            pm_skia_path::Rect::from_ltrb(b.x_min, b.y_min, b.x_max, b.y_max).and_then(|rect| {
                pm_skia_path::PathBuilder::from_rect(rect).transform(path_transform(self.matrix))
            });
        if let Some(path) = path {
            self.clip(path);
        } else {
            self.ops.push(Op::Clip(None));
        }
    }
    fn pop_clip(&mut self) {
        self.ops.push(Op::Unclip);
    }
    fn fill(&mut self, brush: Brush<'_>) {
        let p = |v: skrifa::raw::types::Point<f32>| Point::new(v.x as f64, v.y as f64);
        let fill = match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => Fill::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => Fill::Linear(p(p0), p(p1), self.stops(color_stops), extend),
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend,
            } => Fill::Radial(p(c0), r0, p(c1), r1, self.stops(color_stops), extend),
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend,
            } => Fill::Sweep(
                p(c0),
                start_angle,
                end_angle,
                self.stops(color_stops),
                extend,
            ),
        };
        self.ops.push(Op::Fill(fill, self.matrix.inverse()));
    }
    fn push_layer(&mut self, mode: CompositeMode) {
        self.ops.push(Op::Layer(mode));
    }
    fn pop_layer(&mut self) {
        self.ops.push(Op::Unlayer);
    }
}

fn gradient(stops: &[(f32, [f32; 4])], t: f32, extend: Extend) -> [f32; 4] {
    let t = match extend {
        Extend::Pad => t.clamp(0., 1.),
        Extend::Repeat => t.rem_euclid(1.),
        Extend::Reflect => {
            let v = t.rem_euclid(2.);
            if v > 1. { 2. - v } else { v }
        }
        _ => return [0.; 4],
    };
    let Some(&(first, color)) = stops.first() else {
        return [0.; 4];
    };
    if t < first {
        return color;
    }
    for pair in stops.windows(2) {
        let [(a, c0), (b, c1)] = pair else {
            unreachable!()
        };
        if t < *b {
            let u = (t - a) / (b - a);
            return std::array::from_fn(|i| c0[i] + (c1[i] - c0[i]) * u);
        }
    }
    stops.last().unwrap().1
}

impl Fill {
    fn at(&self, p: Point) -> [f32; 4] {
        match self {
            Self::Solid(c) => *c,
            Self::Linear(p0, p1, stops, extend) => {
                let d = *p1 - *p0;
                let t = ((p - *p0).dot(d) / d.hypot2()) as f32;
                gradient(stops, t, *extend)
            }
            Self::Radial(c0, r0, c1, r1, stops, extend) => {
                let q = p - *c0;
                let d = *c1 - *c0;
                let dr = (*r1 - *r0) as f64;
                let a = d.hypot2() - dr * dr;
                let b = -2. * (q.dot(d) + *r0 as f64 * dr);
                let c = q.hypot2() - (*r0 as f64).powi(2);
                let t = if a.abs() < 1e-12 {
                    -c / b
                } else {
                    let disc = b * b - 4. * a * c;
                    if disc < 0. {
                        return [0.; 4];
                    }
                    let roots = [(-b - disc.sqrt()) / (2. * a), (-b + disc.sqrt()) / (2. * a)];
                    roots
                        .into_iter()
                        .filter(|t| *r0 as f64 + t * dr >= 0.)
                        .reduce(f64::max)
                        .unwrap_or(f64::NAN)
                };
                if !t.is_finite() {
                    return [0.; 4];
                }
                gradient(stops, t as f32, *extend)
            }
            Self::Sweep(c, a, b, stops, extend) => {
                let q = p - *c;
                let angle = (q.y.atan2(q.x).to_degrees() as f32).rem_euclid(360.);
                gradient(stops, (angle - a) / (b - a), *extend)
            }
        }
    }
}

/// Premultiplied source-over in the raster glyph's 8-bit target.
fn over(dst: &mut [u8], color: [f32; 4], coverage: u8) {
    let alpha = color[3] * (coverage as f32 / 255.);
    for c in 0..4 {
        let src = if c == 3 { alpha } else { color[c] * alpha };
        dst[c] = (src * 255. + dst[c] as f32 * (1. - alpha))
            .round()
            .clamp(0., 255.) as u8;
    }
}

/// SkARGB32_Blitter::blitAntiH: premultiply to N32 first, scale the
/// packed source by coverage, then SkPMSrcOver (integer truncation).
fn solid_over(dst: &mut [u8], color: [f32; 4], coverage: u8) {
    let alpha = (color[3] * 255.).round().clamp(0., 255.) as u32;
    let mut src = [0u32; 4];
    for c in 0..4 {
        let channel = if c == 3 {
            255
        } else {
            (color[c] * 255.).round().clamp(0., 255.) as u32
        };
        let product = channel * alpha + 128;
        let premul = (product + (product >> 8)) >> 8;
        src[c] = (premul * (coverage as u32 + 1)) >> 8;
    }
    for c in 0..4 {
        dst[c] = (src[c] + ((dst[c] as u32 * (256 - src[3])) >> 8)).min(255) as u8;
    }
}

fn composite(dst: &mut [u8], src: &[u8], mode: CompositeMode) -> Result<(), String> {
    use CompositeMode::*;
    let s: [f32; 4] = std::array::from_fn(|c| src[c] as f32 / 255.);
    let d: [f32; 4] = std::array::from_fn(|c| dst[c] as f32 / 255.);
    let (sa, da) = (s[3], d[3]);
    let factors = match mode {
        Clear => Some((0., 0.)),
        Src => Some((1., 0.)),
        Dest => Some((0., 1.)),
        SrcOver => Some((1., 1. - sa)),
        DestOver => Some((1. - da, 1.)),
        SrcIn => Some((da, 0.)),
        DestIn => Some((0., sa)),
        SrcOut => Some((1. - da, 0.)),
        DestOut => Some((0., 1. - sa)),
        SrcAtop => Some((da, 1. - sa)),
        DestAtop => Some((1. - da, sa)),
        Xor => Some((1. - da, 1. - sa)),
        Plus => Some((1., 1.)),
        Unknown => return Err("unknown COLR composite mode".into()),
        _ => None,
    };
    let out: [f32; 4] = if let Some((fs, fd)) = factors {
        std::array::from_fn(|c| s[c] * fs + d[c] * fd)
    } else {
        let cs: [f32; 3] = std::array::from_fn(|c| if sa > 0. { s[c] / sa } else { 0. });
        let cb: [f32; 3] = std::array::from_fn(|c| if da > 0. { d[c] / da } else { 0. });
        let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        let sat = |c: [f32; 3]| {
            c.into_iter().reduce(f32::max).unwrap() - c.into_iter().reduce(f32::min).unwrap()
        };
        let set_lum = |mut c: [f32; 3], l: f32| {
            let delta = l - lum(c);
            c = c.map(|v| v + delta);
            let n = c.into_iter().reduce(f32::min).unwrap();
            let x = c.into_iter().reduce(f32::max).unwrap();
            if n < 0. {
                c = c.map(|v| l + (v - l) * l / (l - n));
            }
            if x > 1. {
                c = c.map(|v| l + (v - l) * (1. - l) / (x - l));
            }
            c
        };
        let set_sat = |c: [f32; 3], value: f32| {
            let min = c.into_iter().reduce(f32::min).unwrap();
            let range = sat(c);
            c.map(|v| {
                if range > 0. {
                    (v - min) * value / range
                } else {
                    0.
                }
            })
        };
        let blend = match mode {
            HslHue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
            HslSaturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
            HslColor => set_lum(cs, lum(cb)),
            HslLuminosity => set_lum(cb, lum(cs)),
            _ => std::array::from_fn(|c| {
                let (a, b) = (cs[c], cb[c]);
                let hard = |a: f32, b: f32| {
                    if a <= 0.5 {
                        2. * a * b
                    } else {
                        1. - 2. * (1. - a) * (1. - b)
                    }
                };
                match mode {
                    Screen => a + b - a * b,
                    Overlay => hard(b, a),
                    HardLight => hard(a, b),
                    Darken => a.min(b),
                    Lighten => a.max(b),
                    Multiply => a * b,
                    ColorDodge => {
                        if b == 0. {
                            0.
                        } else if a == 1. {
                            1.
                        } else {
                            (b / (1. - a)).min(1.)
                        }
                    }
                    ColorBurn => {
                        if b == 1. {
                            1.
                        } else if a == 0. {
                            0.
                        } else {
                            1. - ((1. - b) / a).min(1.)
                        }
                    }
                    SoftLight => {
                        if a <= 0.5 {
                            b - (1. - 2. * a) * b * (1. - b)
                        } else {
                            let g = if b <= 0.25 {
                                ((16. * b - 12.) * b + 4.) * b
                            } else {
                                b.sqrt()
                            };
                            b + (2. * a - 1.) * (g - b)
                        }
                    }
                    Difference => (a - b).abs(),
                    Exclusion => a + b - 2. * a * b,
                    _ => unreachable!("Porter-Duff and HSL modes handled above"),
                }
            }),
        };
        [
            s[0] * (1. - da) + d[0] * (1. - sa) + sa * da * blend[0],
            s[1] * (1. - da) + d[1] * (1. - sa) + sa * da * blend[1],
            s[2] * (1. - da) + d[2] * (1. - sa) + sa * da * blend[2],
            sa + da - sa * da,
        ]
    };
    for c in 0..4 {
        dst[c] = (out[c] * 255.).round().clamp(0., 255.) as u8;
    }
    Ok(())
}

pub fn glyph(
    face: FaceRef<'_>,
    coords: Coords<'_>,
    id: u32,
    size: f32,
    skew: f32,
    foreground: Rgba,
) -> Result<Option<BitmapGlyph>, String> {
    let font = face.font().ok_or("invalid COLR font")?;
    let Some(glyph) = font.color_glyphs().get(GlyphId::new(id)) else {
        return Ok(None);
    };
    let upem = font
        .metrics(Size::unscaled(), LocationRef::default())
        .units_per_em as f32;
    if upem == 0. {
        return Err("COLR font has zero units per em".into());
    }
    let coords = match coords {
        Coords::User(c) => font
            .axes()
            .location(c.iter().map(|(t, v)| (skrifa::Tag::from_u32(*t), *v)))
            .coords()
            .to_vec(),
        Coords::Normalized(c) => c.iter().map(|v| NormalizedCoord::from_bits(*v)).collect(),
    };
    let scale = size / upem;
    let mut painter = Recorder {
        font: font.clone(),
        coords: coords.clone(),
        matrix: Affine::new([
            scale as f64,
            0.,
            (-skew * scale) as f64,
            -scale as f64,
            0.,
            0.,
        ]),
        matrices: Vec::new(),
        colors: font
            .color_palettes()
            .get(0)
            .map(|p| {
                p.colors()
                    .iter()
                    .map(|c| {
                        [
                            c.red() as f32 / 255.,
                            c.green() as f32 / 255.,
                            c.blue() as f32 / 255.,
                            c.alpha() as f32 / 255.,
                        ]
                    })
                    .collect()
            })
            .unwrap_or_default(),
        foreground: foreground.map(|v| v as f32),
        ops: Vec::new(),
        bounds: None,
    };
    glyph
        .paint(LocationRef::new(&coords), &mut painter)
        .map_err(|e| e.to_string())?;
    let Some(b) = painter.bounds else {
        return Ok(None);
    };
    let (left, top) = (b.x0.floor() as i32, b.y0.floor() as i32);
    let (width, height) = (
        (b.x1.ceil() as i32 - left) as u32,
        (b.y1.ceil() as i32 - top) as u32,
    );
    if width == 0 || height == 0 {
        return Ok(None);
    }
    if width > 8192 || height > 8192 {
        return Err("COLR glyph exceeds raster limit".into());
    }
    let mut image = vec![0u8; (width * height * 4) as usize];
    let mut clips: Vec<Vec<u8>> = Vec::new();
    let mut layers: Vec<(Vec<u8>, CompositeMode)> = Vec::new();
    for op in painter.ops {
        match op {
            Op::Clip(path) => {
                let path = path.and_then(|p| {
                    p.transform(pm_skia_path::Transform::from_translate(
                        -left as f32,
                        -top as f32,
                    ))
                });
                let mut mask = if let Some(path) = path {
                    let moved = path;
                    super::skaaa::fill_path_a8(&moved, width, height)
                } else {
                    vec![0; (width * height) as usize]
                };
                if let Some(previous) = clips.last() {
                    for (a, b) in mask.iter_mut().zip(previous) {
                        *a = ((*a as u32 * *b as u32 + 127) / 255) as u8;
                    }
                }
                clips.push(mask);
            }
            Op::Unclip => {
                clips.pop();
            }
            Op::Fill(fill, inverse) => {
                for (i, pixel) in image.chunks_exact_mut(4).enumerate() {
                    let coverage = clips.last().map_or(255, |m| m[i]);
                    if coverage == 0 {
                        continue;
                    }
                    let point = inverse
                        * Point::new(
                            left as f64 + (i % width as usize) as f64 + 0.5,
                            top as f64 + (i / width as usize) as f64 + 0.5,
                        );
                    match &fill {
                        Fill::Solid(color) => solid_over(pixel, *color, coverage),
                        _ => over(pixel, fill.at(point), coverage),
                    }
                }
            }
            Op::Layer(mode) => {
                layers.push((
                    std::mem::replace(&mut image, vec![0; (width * height * 4) as usize]),
                    mode,
                ));
            }
            Op::Unlayer => {
                let (mut previous, mode) = layers.pop().ok_or("unbalanced COLR layers")?;
                for (d, s) in previous.chunks_exact_mut(4).zip(image.chunks_exact(4)) {
                    composite(d, s, mode)?;
                }
                image = previous;
            }
        }
    }
    let image = Arc::new(Rgba8 {
        width,
        height,
        data: image,
    });
    Ok(Some(BitmapGlyph {
        image,
        transform: Affine::translate((left as f64, top as f64)),
        bounds: Rect::new(
            left as f64,
            top as f64,
            left as f64 + width as f64,
            top as f64 + height as f64,
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_colr_composite_modes_preserve_premultiplication() {
        for mode in 0..28 {
            for src in [[0, 0, 0, 0], [255, 0, 0, 255], [19, 38, 76, 128]] {
                for mut dst in [[0, 0, 0, 0], [0, 255, 0, 255], [80, 40, 20, 100]] {
                    composite(&mut dst, &src, CompositeMode::new(mode)).unwrap();
                    assert!(
                        dst[..3].iter().all(|c| *c <= dst[3]),
                        "mode {mode}: {dst:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn porter_duff_layers_have_the_expected_alpha_and_color() {
        let mut dst = [0, 128, 0, 128];
        composite(&mut dst, &[128, 0, 0, 128], CompositeMode::SrcIn).unwrap();
        assert_eq!(dst, [64, 0, 0, 64]);
        let mut dst = [0, 128, 0, 128];
        composite(&mut dst, &[128, 0, 0, 128], CompositeMode::DestOut).unwrap();
        assert_eq!(dst, [0, 64, 0, 64]);
    }
}
