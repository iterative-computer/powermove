//! Fontations COLR paint graphs with unhinted outlines and float compositing.
//! Paint transforms, clips and all composite modes retain their authored order.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
use pm_color::contract::WorkingEncoding;
use pm_color::transfer::display_decode_f32;
use skrifa::color::{Brush, ColorPainter, ColorStop, Transform};
pub use skrifa::color::{CompositeMode, Extend};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

use super::color::Rgba;
use super::outline::{Coords, FaceRef};

/// Premultiplied float pixels in the requested working encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaF32 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// Recorded COLR paint commands. Geometry and brush transforms are in device
/// coordinates, including the fractional glyph phase. The integer placement
/// remains in `transform` so the same program can serve translated glyphs.
#[derive(Debug)]
pub struct Program {
    pub width: u32,
    pub height: u32,
    pub transform: Affine,
    pub bounds: Rect,
    phase: (f64, f64),
    foreground: [f32; 4],
    ops: Vec<Op>,
}

impl Program {
    /// Authored COLR graph commands, in paint order. Consumers may interpret
    /// these directly without using this rasterizer's pixels or coverage.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Content identity for renderer caches. This includes every recorded
    /// geometry, brush and graph parameter, but excludes working encoding;
    /// consumers must key that separately when storing rasterized pixels.
    pub fn content_key(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.width.hash(&mut hash);
        self.height.hash(&mut hash);
        [self.phase.0.to_bits(), self.phase.1.to_bits()].hash(&mut hash);
        hash_color(self.foreground, &mut hash);
        hash_affine(self.transform, &mut hash);
        [
            self.bounds.x0.to_bits(),
            self.bounds.y0.to_bits(),
            self.bounds.x1.to_bits(),
            self.bounds.y1.to_bits(),
        ]
        .hash(&mut hash);
        self.ops.len().hash(&mut hash);
        for op in &self.ops {
            match op {
                Op::Clip(path) => {
                    0u8.hash(&mut hash);
                    path.is_some().hash(&mut hash);
                    if let Some(path) = path {
                        hash_path(path, &mut hash);
                    }
                }
                Op::Unclip => 1u8.hash(&mut hash),
                Op::Fill(fill, inverse) => {
                    2u8.hash(&mut hash);
                    hash_fill(fill, &mut hash);
                    hash_affine(*inverse, &mut hash);
                }
                Op::SolidGlyph(path, color) => {
                    3u8.hash(&mut hash);
                    hash_path(path, &mut hash);
                    hash_color(*color, &mut hash);
                }
                Op::Layer(mode) => {
                    4u8.hash(&mut hash);
                    std::mem::discriminant(mode).hash(&mut hash);
                }
                Op::Unlayer => 5u8.hash(&mut hash),
            }
        }
        hash.finish()
    }
}

fn hash_point(point: Point, hash: &mut impl Hasher) {
    [point.x.to_bits(), point.y.to_bits()].hash(hash);
}

fn hash_affine(affine: Affine, hash: &mut impl Hasher) {
    affine.as_coeffs().map(f64::to_bits).hash(hash);
}

fn hash_color(color: [f32; 4], hash: &mut impl Hasher) {
    color.map(f32::to_bits).hash(hash);
}

fn hash_path(path: &BezPath, hash: &mut impl Hasher) {
    path.elements().len().hash(hash);
    for element in path.iter() {
        match element {
            PathEl::MoveTo(point) => {
                0u8.hash(hash);
                hash_point(point, hash);
            }
            PathEl::LineTo(point) => {
                1u8.hash(hash);
                hash_point(point, hash);
            }
            PathEl::QuadTo(control, point) => {
                2u8.hash(hash);
                hash_point(control, hash);
                hash_point(point, hash);
            }
            PathEl::CurveTo(first, second, point) => {
                3u8.hash(hash);
                hash_point(first, hash);
                hash_point(second, hash);
                hash_point(point, hash);
            }
            PathEl::ClosePath => 4u8.hash(hash),
        }
    }
}

fn hash_stops(stops: &[(f32, [f32; 4])], hash: &mut impl Hasher) {
    stops.len().hash(hash);
    for (offset, color) in stops {
        offset.to_bits().hash(hash);
        hash_color(*color, hash);
    }
}

fn hash_fill(fill: &Fill, hash: &mut impl Hasher) {
    match fill {
        Fill::Solid(color) => {
            0u8.hash(hash);
            hash_color(*color, hash);
        }
        Fill::Linear(p0, p1, stops, extend) => {
            1u8.hash(hash);
            hash_point(*p0, hash);
            hash_point(*p1, hash);
            hash_stops(stops, hash);
            std::mem::discriminant(extend).hash(hash);
        }
        Fill::Radial(c0, r0, c1, r1, stops, extend) => {
            2u8.hash(hash);
            hash_point(*c0, hash);
            r0.to_bits().hash(hash);
            hash_point(*c1, hash);
            r1.to_bits().hash(hash);
            hash_stops(stops, hash);
            std::mem::discriminant(extend).hash(hash);
        }
        Fill::Sweep(center, start, end, stops, extend) => {
            3u8.hash(hash);
            hash_point(*center, hash);
            start.to_bits().hash(hash);
            end.to_bits().hash(hash);
            hash_stops(stops, hash);
            std::mem::discriminant(extend).hash(hash);
        }
    }
}

#[derive(Clone, Copy)]
pub struct ColrRequest<'a> {
    pub face: FaceRef<'a>,
    pub coords: Coords<'a>,
    pub id: u32,
    pub size: f32,
    pub skew: f32,
    /// Text layout-to-device linear transform; translation is in `origin`.
    pub linear: Affine,
    pub origin: (f64, f64),
    pub foreground: Rgba,
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    face: u64,
    index: u32,
    glyph: u32,
    size: u32,
    skew: u32,
    coords: Vec<(u32, u32)>,
    foreground: [u64; 4],
    linear: [u64; 4],
    phase: [u64; 2],
}

#[derive(Default)]
pub struct Cache {
    entries: std::collections::HashMap<Key, Result<Option<Arc<Program>>, String>>,
    bytes: usize,
}

impl Cache {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    pub fn get(&mut self, request: ColrRequest<'_>) -> Result<Option<Arc<Program>>, String> {
        let ColrRequest {
            face,
            coords,
            id,
            size,
            skew,
            linear,
            origin,
            foreground,
        } = request;
        let [a, b, c, d, _, _] = linear.as_coeffs();
        let phase = (origin.0 - origin.0.floor(), origin.1 - origin.1.floor());
        let key = Key {
            face: face.id,
            index: face.index,
            glyph: id,
            size: size.to_bits(),
            skew: skew.to_bits(),
            foreground: foreground.map(f64::to_bits),
            linear: [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()],
            phase: [phase.0.to_bits(), phase.1.to_bits()],
            coords: match coords {
                Coords::User(c) => c.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
                Coords::Normalized(c) => c.iter().map(|v| (u32::MAX, *v as u16 as u32)).collect(),
            },
        };
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        let result = program(ColrRequest {
            origin: phase,
            ..request
        })
        .map(|g| g.map(Arc::new));
        if self.bytes > 64 * 1024 * 1024 || self.entries.len() >= 4096 {
            self.clear();
        }
        self.bytes += result
            .as_ref()
            .ok()
            .and_then(|g| g.as_ref())
            .map_or(0, |g| g.memory_bytes());
        self.entries.insert(key, result.clone());
        result
    }
}

/// A COLR brush. Colors and gradient stops are straight, display-encoded RGBA.
#[derive(Clone, Debug)]
pub enum Fill {
    Solid([f32; 4]),
    Linear(Point, Point, Vec<(f32, [f32; 4])>, Extend),
    Radial(Point, f32, Point, f32, Vec<(f32, [f32; 4])>, Extend),
    Sweep(Point, f32, f32, Vec<(f32, [f32; 4])>, Extend),
}

/// One authored COLR graph command. Clip and layer stacks follow paint order.
#[derive(Debug)]
pub enum Op {
    /// Push the intersection of the current clip with a device-space path.
    /// `None` is an empty clip for a missing source outline.
    Clip(Option<BezPath>),
    Unclip,
    /// Brush and inverse device-to-brush transform.
    Fill(Fill, Affine),
    /// Device-space outline and straight display-encoded color.
    SolidGlyph(BezPath, [f32; 4]),
    Layer(CompositeMode),
    Unlayer,
}

struct Pen(BezPath);
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to((cx as f64, cy as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0.curve_to(
            (a as f64, b as f64),
            (c as f64, d as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
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

impl Recorder<'_> {
    fn glyph(&self, id: GlyphId) -> Option<BezPath> {
        let outline = self.font.outline_glyphs().get(id)?;
        let mut pen = Pen(BezPath::new());
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&self.coords)),
                &mut pen,
            )
            .ok()?;
        Some(self.matrix * pen.0)
    }

    fn clip(&mut self, path: BezPath) {
        let rect = path.bounding_box();
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
        let rect = Rect::new(
            b.x_min as f64,
            b.y_min as f64,
            b.x_max as f64,
            b.y_max as f64,
        );
        self.clip(self.matrix * rect.to_path(0.1));
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
    fn fill_glyph(&mut self, id: GlyphId, transform: Option<Transform>, brush: Brush<'_>) {
        if let Brush::Solid {
            palette_index,
            alpha,
        } = brush
        {
            if let Some(path) = self.glyph(id) {
                let r = path.bounding_box();
                self.bounds = Some(self.bounds.map_or(r, |old| old.union(r)));
                self.ops
                    .push(Op::SolidGlyph(path, self.color(palette_index, alpha)));
            }
        } else {
            self.push_clip_glyph(id);
            if let Some(t) = transform {
                self.push_transform(t);
                self.fill(brush);
                self.pop_transform();
            } else {
                self.fill(brush);
            }
            self.pop_clip();
        }
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
    fn in_working(&self, working: WorkingEncoding) -> Self {
        let stops = |stops: &[(f32, [f32; 4])]| {
            stops
                .iter()
                .map(|(offset, color)| (*offset, color_to_working(*color, working)))
                .collect()
        };
        match self {
            Self::Solid(color) => Self::Solid(color_to_working(*color, working)),
            Self::Linear(p0, p1, colors, extend) => Self::Linear(*p0, *p1, stops(colors), *extend),
            Self::Radial(c0, r0, c1, r1, colors, extend) => {
                Self::Radial(*c0, *r0, *c1, *r1, stops(colors), *extend)
            }
            Self::Sweep(c, a, b, colors, extend) => Self::Sweep(*c, *a, *b, stops(colors), *extend),
        }
    }

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

/// Deposit a directed edge's signed area into each pixel row it crosses.
/// Splitting at integer x boundaries makes the trapezoid shares exact for a
/// linear edge: the left cell gets its area to the edge's right, and the next
/// cell gets the remainder. A row prefix sum then recovers filled area.
fn deposit_edge(
    from: Point,
    to: Point,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    differences: &mut [f64],
    breaks: &mut Vec<f64>,
) {
    if !from.is_finite() || !to.is_finite() || from.y == to.y {
        return;
    }
    let (a, b, sign) = if from.y < to.y {
        (from, to, 1.0)
    } else {
        (to, from, -1.0)
    };
    let row_start = (a.y.floor() as i64).max(i64::from(top));
    let row_end = (b.y.ceil() as i64).min(i64::from(top) + i64::from(height));
    let (left_x, right_x) = (f64::from(left), f64::from(left) + f64::from(width));
    for row in row_start..row_end {
        let y0 = a.y.max(row as f64);
        let y1 = b.y.min(row as f64 + 1.0);
        if y1 <= y0 {
            continue;
        }
        let x_at = |y: f64| a.x + (b.x - a.x) * ((y - a.y) / (b.y - a.y));
        let x0 = x_at(y0);
        let x1 = x_at(y1);
        breaks.clear();
        breaks.extend([0.0, 1.0]);
        if x0 != x1 {
            let first = (x0.min(x1).ceil() as i64).max(i64::from(left));
            let last = (x0.max(x1).floor() as i64).min(i64::from(left) + i64::from(width));
            for boundary in first..=last {
                let t = (boundary as f64 - x0) / (x1 - x0);
                if t > 0.0 && t < 1.0 {
                    breaks.push(t);
                }
            }
            breaks.sort_by(f64::total_cmp);
        }
        let row_offset = (row - i64::from(top)) as usize * (width as usize + 1);
        for pair in breaks.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let area = sign * (y1 - y0) * (t1 - t0);
            let mid_x = x0 + (x1 - x0) * (0.5 * (t0 + t1));
            if mid_x <= left_x {
                differences[row_offset] += area;
            } else if mid_x < right_x {
                let column = (mid_x.floor() as i64 - i64::from(left)) as usize;
                let fraction = (mid_x - mid_x.floor()).clamp(0.0, 1.0);
                differences[row_offset + column] += area * (1.0 - fraction);
                differences[row_offset + column + 1] += area * fraction;
            }
        }
    }
}

/// Float area scan conversion of one transformed COLR path. Resolve nonzero
/// winding before integrating edges so overlapping source contours do not
/// count their common area twice.
fn coverage(path: &BezPath, width: u32, height: u32, left: i32, top: i32) -> Vec<f32> {
    let lines = super::scene::flatten_device_path(path);
    let resolved = super::scene::resolve_nonzero(&lines);
    let mut differences = vec![0.0f64; (width as usize + 1) * height as usize];
    let mut breaks = Vec::new();
    let (mut start, mut last, mut open) = (Point::ZERO, Point::ZERO, false);
    for element in resolved.elements() {
        match *element {
            PathEl::MoveTo(point) => {
                if open {
                    deposit_edge(
                        last,
                        start,
                        width,
                        height,
                        left,
                        top,
                        &mut differences,
                        &mut breaks,
                    );
                }
                (start, last, open) = (point, point, true);
            }
            PathEl::LineTo(point) => {
                if open {
                    deposit_edge(
                        last,
                        point,
                        width,
                        height,
                        left,
                        top,
                        &mut differences,
                        &mut breaks,
                    );
                    last = point;
                }
            }
            PathEl::ClosePath => {
                if open {
                    deposit_edge(
                        last,
                        start,
                        width,
                        height,
                        left,
                        top,
                        &mut differences,
                        &mut breaks,
                    );
                    open = false;
                }
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
                unreachable!("COLR coverage paths were flattened before nonzero resolution")
            }
        }
    }
    if open {
        deposit_edge(
            last,
            start,
            width,
            height,
            left,
            top,
            &mut differences,
            &mut breaks,
        );
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for row in differences.chunks_exact(width as usize + 1) {
        let mut winding_area = 0.0f64;
        for share in &row[..width as usize] {
            winding_area += share;
            pixels.push(winding_area.abs().clamp(0.0, 1.0) as f32);
        }
    }
    pixels
}

fn over(dst: &mut [f32], color: [f32; 4], coverage: f32) {
    let a = (color[3] * coverage).clamp(0.0, 1.0);
    for c in 0..3 {
        dst[c] = color[c] * a + dst[c] * (1.0 - a);
    }
    dst[3] = a + dst[3] * (1.0 - a);
}

fn composite(dst: &mut [f32], src: &[f32], mode: CompositeMode) -> Result<(), String> {
    use CompositeMode::*;
    let s: [f32; 4] = src.try_into().expect("RGBA source");
    let d: [f32; 4] = dst.try_into().expect("RGBA destination");
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
        dst[c] = out[c].clamp(0.0, 1.0);
    }
    Ok(())
}

fn color_to_working(mut color: [f32; 4], working: WorkingEncoding) -> [f32; 4] {
    if working == WorkingEncoding::LinearLight {
        for channel in &mut color[..3] {
            *channel = display_decode_f32(*channel);
        }
    }
    color
}

pub fn program(request: ColrRequest<'_>) -> Result<Option<Program>, String> {
    let ColrRequest {
        face,
        coords,
        id,
        size,
        skew,
        linear,
        origin: phase,
        foreground,
    } = request;
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
        matrix: Affine::translate(phase)
            * linear
            * Affine::new([
                scale as f64,
                0.0,
                (-skew * scale) as f64,
                -scale as f64,
                0.0,
                0.0,
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
    let (mut clip_depth, mut layer_depth) = (0usize, 0usize);
    for op in &painter.ops {
        match op {
            Op::Clip(_) => clip_depth += 1,
            Op::Unclip if clip_depth > 0 => clip_depth -= 1,
            Op::Unclip => return Err("unbalanced COLR clip stack".into()),
            Op::Layer(CompositeMode::Unknown) => {
                return Err("unknown COLR composite mode".into());
            }
            Op::Layer(_) => layer_depth += 1,
            Op::Unlayer if layer_depth > 0 => layer_depth -= 1,
            Op::Unlayer => return Err("unbalanced COLR layer stack".into()),
            _ => {}
        }
    }
    if clip_depth != 0 || layer_depth != 0 {
        return Err("unbalanced COLR paint graph".into());
    }
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
    Ok(Some(Program {
        width,
        height,
        phase,
        foreground: foreground.map(|v| v as f32),
        transform: Affine::translate((left as f64, top as f64)),
        bounds: Rect::new(
            left as f64,
            top as f64,
            left as f64 + width as f64,
            top as f64 + height as f64,
        ),
        ops: painter.ops,
    }))
}

impl Program {
    fn memory_bytes(&self) -> usize {
        let mut bytes =
            std::mem::size_of::<Self>() + self.ops.capacity() * std::mem::size_of::<Op>();
        for op in &self.ops {
            bytes += match op {
                Op::Clip(Some(path)) | Op::SolidGlyph(path, _) => {
                    std::mem::size_of_val(path.elements())
                }
                Op::Fill(
                    Fill::Linear(_, _, stops, _)
                    | Fill::Radial(_, _, _, _, stops, _)
                    | Fill::Sweep(_, _, _, stops, _),
                    _,
                ) => stops.capacity() * std::mem::size_of::<(f32, [f32; 4])>(),
                _ => 0,
            };
        }
        bytes
    }

    /// Renders the graph as premultiplied float pixels in `working`. Palette
    /// and foreground channels are decoded before gradient and layer blends.
    pub fn rasterize(&self, working: WorkingEncoding) -> RgbaF32 {
        let (width, height) = (self.width, self.height);
        let (left, top) = (self.bounds.x0 as i32, self.bounds.y0 as i32);
        let mut image = vec![0f32; (width * height * 4) as usize];
        let mut clips: Vec<Vec<f32>> = Vec::new();
        let mut layers: Vec<(Vec<f32>, CompositeMode)> = Vec::new();
        for op in &self.ops {
            match op {
                Op::Clip(path) => {
                    let mut mask = if let Some(path) = path {
                        coverage(path, width, height, left, top)
                    } else {
                        vec![0.0; width as usize * height as usize]
                    };
                    if let Some(previous) = clips.last() {
                        for (a, b) in mask.iter_mut().zip(previous) {
                            *a *= *b;
                        }
                    }
                    clips.push(mask);
                }
                Op::Unclip => {
                    clips.pop();
                }
                Op::Fill(fill, inverse) => {
                    let fill = fill.in_working(working);
                    for (i, pixel) in image.chunks_exact_mut(4).enumerate() {
                        let coverage = clips.last().map_or(1.0, |m| m[i]);
                        if coverage == 0.0 {
                            continue;
                        }
                        let point = *inverse
                            * Point::new(
                                left as f64 + (i % width as usize) as f64 + 0.5,
                                top as f64 + (i / width as usize) as f64 + 0.5,
                            );
                        over(pixel, fill.at(point), coverage);
                    }
                }
                Op::SolidGlyph(path, color) => {
                    let mask = coverage(path, width, height, left, top);
                    let color = color_to_working(*color, working);
                    for (i, pixel) in image.chunks_exact_mut(4).enumerate() {
                        let clip = clips.last().map_or(1.0, |m| m[i]);
                        let cov = mask[i] * clip;
                        if cov > 0.0 {
                            over(pixel, color, cov);
                        }
                    }
                }
                Op::Layer(mode) => {
                    layers.push((
                        std::mem::replace(&mut image, vec![0.0; (width * height * 4) as usize]),
                        *mode,
                    ));
                }
                Op::Unlayer => {
                    let (mut previous, mode) = layers.pop().expect("validated COLR layer stack");
                    for (d, s) in previous.chunks_exact_mut(4).zip(image.chunks_exact(4)) {
                        composite(d, s, mode).expect("validated COLR composite mode");
                    }
                    image = previous;
                }
            }
        }
        RgbaF32 {
            width,
            height,
            data: image,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_rect_area(rect: Rect, x: usize, y: usize) -> f64 {
        let x = x as f64;
        let y = y as f64;
        (rect.x1.min(x + 1.0) - rect.x0.max(x)).max(0.0)
            * (rect.y1.min(y + 1.0) - rect.y0.max(y)).max(0.0)
    }

    #[test]
    fn fractional_rectangle_has_sub_byte_area_and_centroid_accuracy() {
        let rect = Rect::new(1.123, 0.234, 6.777, 7.456);
        let coverage = coverage(&rect.to_path(0.1), 8, 9, 0, 0);
        let (mut actual_area, mut actual_x, mut actual_y) = (0.0, 0.0, 0.0);
        let (mut expected_area, mut expected_x, mut expected_y) = (0.0, 0.0, 0.0);
        for y in 0..9 {
            for x in 0..8 {
                let expected = pixel_rect_area(rect, x, y);
                let actual = f64::from(coverage[y * 8 + x]);
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "{x},{y}: {actual} vs {expected}"
                );
                actual_area += actual;
                actual_x += actual * (x as f64 + 0.5);
                actual_y += actual * (y as f64 + 0.5);
                expected_area += expected;
                expected_x += expected * (x as f64 + 0.5);
                expected_y += expected * (y as f64 + 0.5);
            }
        }
        assert!((actual_area - expected_area).abs() < 1e-5);
        assert!((actual_x / actual_area - expected_x / expected_area).abs() < 1e-5);
        assert!((actual_y / actual_area - expected_y / expected_area).abs() < 1e-5);
    }

    #[test]
    fn overlapping_component_contours_cover_their_union_once() {
        let a = Rect::new(1.2, 1.1, 5.6, 6.9);
        let b = Rect::new(3.4, 0.8, 7.2, 4.7);
        let intersection = Rect::new(
            a.x0.max(b.x0),
            a.y0.max(b.y0),
            a.x1.min(b.x1),
            a.y1.min(b.y1),
        );
        let mut path = BezPath::new();
        for rect in [a, b] {
            path.move_to((rect.x0, rect.y0));
            path.line_to((rect.x1, rect.y0));
            path.line_to((rect.x1, rect.y1));
            path.line_to((rect.x0, rect.y1));
            path.close_path();
        }
        let pixels = coverage(&path, 9, 8, 0, 0);
        for y in 0..8 {
            for x in 0..9 {
                let expected = pixel_rect_area(a, x, y) + pixel_rect_area(b, x, y)
                    - pixel_rect_area(intersection, x, y);
                let actual = f64::from(pixels[y * 9 + x]);
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "{x},{y}: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn device_path_coverage_matches_independent_supersample() {
        let mut path = BezPath::new();
        path.move_to((1.1, 1.2));
        path.line_to((7.7, 2.4));
        path.line_to((3.8, 8.1));
        path.close_path();
        let path = Affine::rotate(0.21) * path;
        let pixels = coverage(&path, 12, 12, 0, 0);
        let reference = |x: usize, y: usize| {
            let mut n = 0u32;
            for sy in 0..32 {
                for sx in 0..32 {
                    let p = Point::new(
                        x as f64 + (sx as f64 + 0.5) / 32.0,
                        y as f64 + (sy as f64 + 0.5) / 32.0,
                    );
                    n += u32::from(path.winding(p) != 0);
                }
            }
            n as f32 / 1024.0
        };
        for y in 0..12 {
            for x in 0..12 {
                assert!(
                    (pixels[y * 12 + x] - reference(x, y)).abs() < 0.035,
                    "{x},{y}"
                );
            }
        }
    }

    #[test]
    fn working_domain_decodes_before_colr_blend() {
        let encoded = [0.5, 0.25, 0.0, 1.0];
        let standard = color_to_working(encoded, WorkingEncoding::Standard);
        let linear = color_to_working(encoded, WorkingEncoding::LinearLight);
        assert_eq!(standard, encoded);
        assert!(linear[0] < standard[0] && linear[1] < standard[1]);
        let mut dst = [0.0; 4];
        over(&mut dst, linear, 0.5);
        assert!((dst[0] - display_decode_f32(0.5) * 0.5).abs() < 1e-7);
    }

    #[test]
    fn source_program_keeps_encoded_colors_and_rasterizes_in_requested_domain() {
        let path = Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.1);
        let program = Program {
            width: 4,
            height: 4,
            phase: (0.0, 0.0),
            foreground: [0.0, 0.0, 0.0, 1.0],
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 4.0, 4.0),
            ops: vec![Op::SolidGlyph(path, [0.5, 0.25, 0.0, 0.5])],
        };
        let Op::SolidGlyph(_, encoded) = &program.ops()[0] else {
            panic!("expected authored solid glyph")
        };
        assert_eq!(*encoded, [0.5, 0.25, 0.0, 0.5]);
        let standard = program.rasterize(WorkingEncoding::Standard);
        let linear = program.rasterize(WorkingEncoding::LinearLight);
        let center = 20;
        assert!((standard.data[center] - 0.25).abs() < 1e-7);
        assert!((linear.data[center] - display_decode_f32(0.5) * 0.5).abs() < 1e-7);
        assert_eq!(standard.data[center + 3], 0.5);
        assert_eq!(linear.data[center + 3], 0.5);
        assert_eq!(*encoded, [0.5, 0.25, 0.0, 0.5]);
    }

    #[test]
    fn content_key_tracks_authored_geometry_paint_and_graph() {
        let path = Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.1);
        let mut program = Program {
            width: 4,
            height: 4,
            phase: (0.0, 0.0),
            foreground: [0.0, 0.0, 0.0, 1.0],
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 4.0, 4.0),
            ops: vec![
                Op::Clip(Some(path.clone())),
                Op::Fill(
                    Fill::Linear(
                        Point::ZERO,
                        Point::new(4.0, 0.0),
                        vec![(0.0, [0.25, 0.0, 0.0, 1.0]), (1.0, [0.75, 0.0, 0.0, 1.0])],
                        Extend::Pad,
                    ),
                    Affine::IDENTITY,
                ),
                Op::Unclip,
                Op::Layer(CompositeMode::SrcOver),
                Op::SolidGlyph(path, [0.5, 0.25, 0.0, 1.0]),
                Op::Unlayer,
            ],
        };
        let original = program.content_key();
        assert_eq!(program.content_key(), original);
        program.phase.0 = 0.25;
        assert_ne!(program.content_key(), original);
        program.phase.0 = 0.0;
        program.foreground[0] = 0.5;
        assert_ne!(program.content_key(), original);
        program.foreground[0] = 0.0;
        program.transform = Affine::translate((1.0, 0.0));
        assert_ne!(program.content_key(), original);
        program.transform = Affine::IDENTITY;
        if let Op::Fill(Fill::Linear(_, _, stops, _), _) = &mut program.ops[1] {
            stops[0].1[0] = 0.5;
        } else {
            panic!("expected gradient")
        }
        assert_ne!(program.content_key(), original);
        if let Op::Fill(Fill::Linear(_, _, stops, _), _) = &mut program.ops[1] {
            stops[0].1[0] = 0.25;
        }
        if let Op::SolidGlyph(path, _) = &mut program.ops[4] {
            path.line_to((5.0, 5.0));
        }
        assert_ne!(program.content_key(), original);
    }

    #[test]
    fn gradient_stops_decode_before_working_space_interpolation() {
        let brush = Fill::Linear(
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            vec![(0.0, [0.25, 0.0, 0.0, 1.0]), (1.0, [0.75, 0.0, 0.0, 1.0])],
            Extend::Pad,
        );
        let sample = Point::new(1.0, 0.0);
        let encoded = brush.in_working(WorkingEncoding::Standard).at(sample);
        let linear = brush.in_working(WorkingEncoding::LinearLight).at(sample);
        assert!((encoded[0] - 0.5).abs() < 1e-7);
        let expected = (display_decode_f32(0.25) + display_decode_f32(0.75)) * 0.5;
        assert!((linear[0] - expected).abs() < 1e-7);
        assert!((linear[0] - display_decode_f32(0.5)).abs() > 1e-3);
    }

    #[test]
    fn float_colr_compositing_keeps_sub_byte_precision() {
        let mut dst = [0.0, 0.25, 0.0, 0.5];
        composite(&mut dst, &[0.125, 0.0, 0.0, 0.25], CompositeMode::SrcOver).unwrap();
        assert!((dst[0] - 0.125).abs() < 1e-7);
        assert!((dst[1] - 0.1875).abs() < 1e-7);
        assert!((dst[3] - 0.625).abs() < 1e-7);
        assert_ne!(dst[0], (dst[0] * 255.0).round() / 255.0);
    }

    #[test]
    fn modes_preserve_premultiplication() {
        for mode in 0..28 {
            let mut dst = [0.1, 0.2, 0.05, 0.5];
            composite(&mut dst, &[0.3, 0.1, 0.2, 0.5], CompositeMode::new(mode)).unwrap();
            assert!(
                dst[..3].iter().all(|c| *c <= dst[3] + 1e-6),
                "mode {mode}: {dst:?}"
            );
        }
    }
}
