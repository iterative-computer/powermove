//! Editable vector paths: cubic segments from per-vertex tangents, trim
//! start/end/offset, group parenting, and the repeater (up to 256 copies).
//!
//! A port of `legacy/core/vector-paths.ts`: [`path_points`] is `pathPoints`,
//! [`trace_path`] is `tracePath`, [`group_matrix`] is `groupMatrix`, and
//! [`PathLayer`] is the geometry `rasterPaths` and `rasterPathsToViewport`
//! draw, with the bounds and bitmap size `rasterPaths` computes. Values are
//! already evaluated (`pathValues`); paths are in layer pixels.

use kurbo::{Affine, BezPath, Join, Point, Stroke};

use crate::canvas::CanvasPath;
use crate::js;
use crate::shape::{CANVAS_MITER_LIMIT, MAX_RASTER_SIZE};

/// Most repeater copies drawn per path.
pub const MAX_COPIES: f64 = 256.0;

/// Line segments per cubic when a path is flattened for trim and bounds.
pub const SAMPLES_PER_SEGMENT: u32 = 32;

/// One evaluated vertex: position and its in and out tangents, relative to
/// the position (`Number(value) || 0` already applied, as `pathValues` does).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    pub in_x: f64,
    pub in_y: f64,
    pub out_x: f64,
    pub out_y: f64,
}

impl Vertex {
    /// A corner vertex (zero tangents).
    pub fn corner(x: f64, y: f64) -> Self {
        Self { x, y, ..Self::default() }
    }
}

/// The evaluated values of one path (`pathValues`). Numbers are as the
/// evaluator returns them (NaN where JavaScript would see a non-number).
#[derive(Debug, Clone, PartialEq)]
pub struct PathValues {
    pub x: f64,
    pub y: f64,
    /// Degrees.
    pub rotation: f64,
    /// Percent.
    pub scale_x: f64,
    pub scale_y: f64,
    pub closed: bool,
    pub fill_enabled: bool,
    /// Percent.
    pub fill_opacity: f64,
    pub stroke_width: f64,
    /// Percent.
    pub stroke_opacity: f64,
    /// Percent of length.
    pub trim_start: f64,
    pub trim_end: f64,
    /// Degrees (360 is one full length).
    pub trim_offset: f64,
    pub copies: f64,
    pub repeat_x: f64,
    pub repeat_y: f64,
    /// Degrees per copy.
    pub repeat_rotation: f64,
    pub vertices: Vec<Vertex>,
}

impl Default for PathValues {
    /// `makeVectorPath`'s defaults.
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            closed: false,
            fill_enabled: true,
            fill_opacity: 100.0,
            stroke_width: 2.0,
            stroke_opacity: 100.0,
            trim_start: 0.0,
            trim_end: 100.0,
            trim_offset: 0.0,
            copies: 1.0,
            repeat_x: 30.0,
            repeat_y: 0.0,
            repeat_rotation: 0.0,
            vertices: Vec::new(),
        }
    }
}

impl PathValues {
    /// Number of cubic segments: one per vertex when closed, else one fewer.
    pub fn segment_count(&self) -> usize {
        let n = self.vertices.len();
        if self.closed { n } else { n.saturating_sub(1) }
    }

    /// The cubic from vertex `i` to the next, as (start, control 1, control 2, end).
    fn segment(&self, i: usize) -> (Point, Point, Point, Point) {
        let vs = &self.vertices;
        let (a, b) = (vs[i], vs[(i + 1) % vs.len()]);
        (
            Point::new(a.x, a.y),
            Point::new(a.x + a.out_x, a.y + a.out_y),
            Point::new(b.x + b.in_x, b.y + b.in_y),
            Point::new(b.x, b.y),
        )
    }

    /// Repeater copies drawn: `min(256, max(1, round(copies)))`, and none
    /// when that is NaN.
    pub fn copy_count(&self) -> u32 {
        let n = js::min(MAX_COPIES, js::max(1.0, js::round(self.copies)));
        if n.is_nan() { 0 } else { n as u32 }
    }

    /// `closed && fillEnabled && trimEnd - trimStart >= 99.999`.
    pub fn fills(&self) -> bool {
        self.closed && self.fill_enabled && self.trim_end - self.trim_start >= 99.999
    }

    /// Alpha for fills: `clamp(fillOpacity, 0, 100) / 100`, or 1 when not finite.
    pub fn fill_alpha(&self) -> f64 {
        opacity(self.fill_opacity)
    }

    /// Alpha for strokes.
    pub fn stroke_alpha(&self) -> f64 {
        opacity(self.stroke_opacity)
    }

    /// The path's own transform: rotation, scale, then position.
    pub fn local_matrix(&self) -> Affine {
        let r = js::radians(self.rotation);
        let (c, s) = (js::cos(r), js::sin(r));
        Affine::new([
            c * self.scale_x / 100.0,
            s * self.scale_x / 100.0,
            -s * self.scale_y / 100.0,
            c * self.scale_y / 100.0,
            self.x,
            self.y,
        ])
    }
}

fn opacity(percent: f64) -> f64 {
    if percent.is_finite() { js::clamp(percent, 0.0, 100.0) / 100.0 } else { 1.0 }
}

/// `PM.mul`: the 2×3 product `m · n` with the legacy evaluation order.
pub fn mul(m: Affine, n: Affine) -> Affine {
    let m = m.as_coeffs();
    let n = n.as_coeffs();
    Affine::new([
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ])
}

/// A rotation by `radians` (clockwise in y-down space), `[c, s, -s, c, 0, 0]`,
/// with `libm` trigonometry so every target agrees.
pub fn rotate(radians: f64) -> Affine {
    let (c, s) = (js::cos(radians), js::sin(radians));
    Affine::new([c, s, -s, c, 0.0, 0.0])
}

/// `pathPoints`: the first vertex, then 32 samples per cubic segment.
pub fn path_points(v: &PathValues) -> Vec<Point> {
    let mut points = Vec::new();
    let Some(first) = v.vertices.first() else {
        return points;
    };
    points.reserve(1 + v.segment_count() * SAMPLES_PER_SEGMENT as usize);
    points.push(Point::new(first.x, first.y));
    for i in 0..v.segment_count() {
        let (a, a_out, b_in, b) = v.segment(i);
        for j in 1..=SAMPLES_PER_SEGMENT {
            let t = f64::from(j) / f64::from(SAMPLES_PER_SEGMENT);
            let u = 1.0 - t;
            points.push(Point::new(
                u * u * u * a.x + 3.0 * u * u * t * a_out.x + 3.0 * u * t * t * b_in.x + t * t * t * b.x,
                u * u * u * a.y + 3.0 * u * u * t * a_out.y + 3.0 * u * t * t * b_in.y + t * t * t * b.y,
            ));
        }
    }
    points
}

/// `tracePath`: the full cubic path when untrimmed, otherwise the trimmed
/// portion of its 32-sample flattening as line segments (in one or two runs
/// when the offset wraps past the end).
pub fn trace_path(v: &PathValues) -> BezPath {
    let mut ctx = CanvasPath::new();
    let start = js::max(0.0, js::min(100.0, js::or0(v.trim_start))) / 100.0;
    let end = js::max(start, js::min(100.0, js::or0(v.trim_end)) / 100.0);
    let span = end - start;
    let vs = &v.vertices;
    if vs.is_empty() || span <= 0.0 || span.is_nan() {
        return ctx.finish();
    }
    if span >= 0.999999 {
        ctx.move_to(vs[0].x, vs[0].y);
        for i in 0..v.segment_count() {
            let (_, c1, c2, b) = v.segment(i);
            ctx.bezier_curve_to(c1.x, c1.y, c2.x, c2.y, b.x, b.y);
        }
        if v.closed {
            ctx.close_path();
        }
        return ctx.finish();
    }
    let points = path_points(v);
    let mut lengths = Vec::with_capacity(points.len());
    lengths.push(0.0);
    for i in 1..points.len() {
        let (a, b) = (points[i - 1], points[i]);
        lengths.push(lengths[i - 1] + js::hypot(b.x - a.x, b.y - a.y));
    }
    let total = js::or0(*lengths.last().unwrap_or(&0.0));
    if total == 0.0 {
        return ctx.finish();
    }
    let offset = ((js::or0(v.trim_offset) / 360.0) % 1.0 + 1.0) % 1.0;
    let lo = (start + offset) % 1.0;
    let hi = lo + span;
    let ranges: &[(f64, f64)] = if hi <= 1.0 { &[(lo, hi)] } else { &[(lo, 1.0), (0.0, hi - 1.0)] };
    for &(from, to) in ranges {
        let mut drawing = false;
        for i in 1..points.len() {
            let (a, b) = (points[i - 1], points[i]);
            let (l0, l1) = (lengths[i - 1], lengths[i]);
            let low = js::max(l0, from * total);
            let high = js::min(l1, to * total);
            // `if (high <= low || l1 <= l0) continue;`
            if high <= low || l1 <= l0 {
                continue;
            }
            let point = |at: f64| {
                Point::new(
                    a.x + (b.x - a.x) * (at - l0) / (l1 - l0),
                    a.y + (b.y - a.y) * (at - l0) / (l1 - l0),
                )
            };
            let (p, q) = (point(low), point(high));
            if !drawing {
                ctx.move_to(p.x, p.y);
            }
            ctx.line_to(q.x, q.y);
            drawing = true;
        }
    }
    ctx.finish()
}

/// One path of a layer with its identity and group parent (`d.paths[i]`).
#[derive(Debug, Clone, PartialEq)]
pub struct PathNode {
    pub id: String,
    pub parent: Option<String>,
    pub values: PathValues,
}

/// `groupMatrix`: the path's transform composed with its group parents'.
/// The first path with the parent's id is the parent; a cycle contributes the
/// identity where it closes, as today.
pub fn group_matrix(paths: &[PathNode], index: usize) -> Affine {
    let mut seen: Vec<&str> = Vec::new();
    group_matrix_inner(paths, index, &mut seen)
}

fn group_matrix_inner<'a>(paths: &'a [PathNode], index: usize, seen: &mut Vec<&'a str>) -> Affine {
    let path = &paths[index];
    if seen.contains(&path.id.as_str()) {
        return Affine::IDENTITY;
    }
    seen.push(&path.id);
    let local = path.values.local_matrix();
    let parent = path
        .parent
        .as_deref()
        .and_then(|parent| paths.iter().position(|p| p.id == parent));
    match parent {
        Some(parent) => mul(group_matrix_inner(paths, parent, seen), local),
        None => local,
    }
}

/// The repeater transform of copy `i`: translate by `i · repeat`, then rotate
/// by `i · repeatRotation` degrees, as `[c, s, -s, c, i·repeatX, i·repeatY]`.
pub fn copy_matrix(v: &PathValues, i: u32) -> Affine {
    let fi = f64::from(i);
    let r = fi * v.repeat_rotation * core::f64::consts::PI / 180.0;
    let (c, s) = (js::cos(r), js::sin(r));
    Affine::new([c, s, -s, c, fi * v.repeat_x, fi * v.repeat_y])
}

/// The stroke a path draws, if any: `lineWidth = max(0, strokeWidth)` with
/// Canvas2D's default miter joins and butt caps, when `strokeWidth > 0`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathStroke {
    pub style: Stroke,
    pub alpha: f64,
}

/// One draw: path `path` (an index into the layer's paths and
/// [`PathLayer::traced`]) as repeater copy `copy`, with its transform into
/// layer pixels. Fill (with `d.paths[path].fill`) comes before stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct PathDraw {
    pub path: usize,
    pub copy: u32,
    pub transform: Affine,
    /// Fill alpha when the copy is filled.
    pub fill: Option<f64>,
    pub stroke: Option<PathStroke>,
}

/// `rasterPaths` bounds in layer pixels, including stroke and a 4 px margin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathBounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// `max(1, x1 - x0)`.
    pub w: f64,
    /// `max(1, y1 - y0)`.
    pub h: f64,
}

/// The bitmap `rasterPaths` allocates for a render scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathRaster {
    pub density: f64,
    /// Pixels.
    pub width: f64,
    pub height: f64,
    /// `-x0`, `-y0`: where the layer origin sits in the bitmap, in layer pixels.
    pub anchor_x: f64,
    pub anchor_y: f64,
}

/// Geometry of a layer's editable paths: traced paths, draws in order, and
/// today's raster bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLayer {
    /// `tracePath` output per path, in the path's own space.
    pub traced: Vec<BezPath>,
    pub draws: Vec<PathDraw>,
    pub bounds: PathBounds,
}

impl PathLayer {
    pub fn new(paths: &[PathNode]) -> Self {
        let matrices: Vec<Affine> = (0..paths.len()).map(|i| group_matrix(paths, i)).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut draws = Vec::new();
        let mut traced = Vec::with_capacity(paths.len());
        for (index, node) in paths.iter().enumerate() {
            let v = &node.values;
            let points = path_points(v);
            traced.push(trace_path(v));
            for i in 0..v.copy_count() {
                let m = mul(matrices[index], copy_matrix(v, i));
                let c = m.as_coeffs();
                let pad = v.stroke_width * js::max(js::hypot(c[0], c[1]), js::hypot(c[2], c[3])) + 4.0;
                for p in &points {
                    let x = c[0] * p.x + c[2] * p.y + c[4];
                    let y = c[1] * p.x + c[3] * p.y + c[5];
                    x0 = js::min(x0, x - pad);
                    y0 = js::min(y0, y - pad);
                    x1 = js::max(x1, x + pad);
                    y1 = js::max(y1, y + pad);
                }
                draws.push(PathDraw {
                    path: index,
                    copy: i,
                    transform: m,
                    fill: v.fills().then(|| v.fill_alpha()),
                    stroke: (v.stroke_width > 0.0).then(|| PathStroke {
                        style: Stroke::new(js::max(0.0, v.stroke_width))
                            .with_join(Join::Miter)
                            .with_miter_limit(CANVAS_MITER_LIMIT),
                        alpha: v.stroke_alpha(),
                    }),
                });
            }
        }
        if !x0.is_finite() {
            (x0, y0, x1, y1) = (0.0, 0.0, 1.0, 1.0);
        }
        let bounds = PathBounds { x0, y0, x1, y1, w: js::max(1.0, x1 - x0), h: js::max(1.0, y1 - y0) };
        Self { traced, draws, bounds }
    }

    /// The bitmap size for a render scale (`density = min(scale, 8192 / w, 8192 / h)`).
    pub fn raster(&self, scale: f64) -> PathRaster {
        let PathBounds { x0, y0, w, h, .. } = self.bounds;
        let density = js::min(js::min(scale, MAX_RASTER_SIZE / w), MAX_RASTER_SIZE / h);
        PathRaster {
            density,
            width: libm::ceil(w * density),
            height: libm::ceil(h * density),
            anchor_x: -x0,
            anchor_y: -y0,
        }
    }

    /// Layer pixels to bitmap pixels for a raster (`scale(density)`, then
    /// `translate(-x0, -y0)`).
    pub fn layer_to_pixels(&self, raster: &PathRaster) -> Affine {
        let d = raster.density;
        Affine::new([d, 0.0, 0.0, d, -self.bounds.x0 * d, -self.bounds.y0 * d])
    }

    /// Layer pixels to presented layer pixels as today's compositor draws the
    /// bitmap: `width` × `height` pixels stretched over `w` × `h` units from
    /// (x0, y0), so content shrinks slightly when the pixel size rounds up.
    pub fn presented(&self, raster: &PathRaster) -> Affine {
        let (sx, sy) = (self.bounds.w / raster.width, self.bounds.h / raster.height);
        Affine::new([sx, 0.0, 0.0, sy, self.bounds.x0, self.bounds.y0]) * self.layer_to_pixels(raster)
    }

    /// Geometric bounds of every draw (fill and stroke outline excluded), for
    /// level 2 rendering at output resolution.
    pub fn geometric_bounds(&self) -> Option<kurbo::Rect> {
        use kurbo::Shape;
        self.draws
            .iter()
            .filter(|d| !self.traced[d.path].elements().is_empty())
            .map(|d| (d.transform * &self.traced[d.path]).bounding_box())
            .reduce(|a, b| a.union(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::PathEl;

    fn square(closed: bool) -> PathValues {
        PathValues {
            closed,
            vertices: vec![
                Vertex::corner(0.0, 0.0),
                Vertex::corner(100.0, 0.0),
                Vertex::corner(100.0, 100.0),
                Vertex::corner(0.0, 100.0),
            ],
            ..PathValues::default()
        }
    }

    #[test]
    fn points_sample_each_segment() {
        assert_eq!(path_points(&square(false)).len(), 1 + 3 * 32);
        assert_eq!(path_points(&square(true)).len(), 1 + 4 * 32);
        assert!(path_points(&PathValues::default()).is_empty());
    }

    #[test]
    fn untrimmed_closed_path_is_cubic_and_closed() {
        let path = trace_path(&square(true));
        let els = path.elements();
        assert_eq!(els.len(), 6);
        assert!(matches!(els[1], PathEl::CurveTo(..)));
        assert_eq!(els[5], PathEl::ClosePath);
    }

    #[test]
    fn trim_halves_the_outline() {
        let v = PathValues { trim_end: 50.0, ..square(true) };
        let path = trace_path(&v);
        let els = path.elements();
        assert_eq!(els[0], PathEl::MoveTo(Point::new(0.0, 0.0)));
        assert_eq!(*els.last().unwrap(), PathEl::LineTo(Point::new(100.0, 100.0)));
    }

    #[test]
    fn trim_offset_wraps_into_two_runs() {
        let v = PathValues { trim_end: 50.0, trim_offset: 270.0, ..square(true) };
        let path = trace_path(&v);
        let moves = path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count();
        assert_eq!(moves, 2);
    }

    #[test]
    fn copies_round_and_clamp() {
        let count = |copies| PathValues { copies, ..PathValues::default() }.copy_count();
        assert_eq!(count(2.5), 3);
        assert_eq!(count(0.0), 1);
        assert_eq!(count(1000.0), 256);
        assert_eq!(count(f64::NAN), 0);
    }

    #[test]
    fn group_parents_compose_and_cycles_stop() {
        let node = |id: &str, parent: Option<&str>, x: f64| PathNode {
            id: id.into(),
            parent: parent.map(Into::into),
            values: PathValues { x, ..PathValues::default() },
        };
        let paths = [node("a", Some("b"), 1.0), node("b", None, 10.0)];
        assert_eq!(group_matrix(&paths, 0).as_coeffs()[4], 11.0);
        let cycle = [node("a", Some("b"), 1.0), node("b", Some("a"), 10.0)];
        // a → b → a (identity where the cycle closes).
        assert_eq!(group_matrix(&cycle, 0).as_coeffs()[4], 11.0);
    }

    #[test]
    fn bounds_include_stroke_and_margin() {
        let layer = PathLayer::new(&[PathNode { id: "p".into(), parent: None, values: square(true) }]);
        let b = layer.bounds;
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (-6.0, -6.0, 106.0, 106.0));
        let raster = layer.raster(2.0);
        assert_eq!((raster.width, raster.anchor_x), (224.0, 6.0));
        assert_eq!(layer.draws.len(), 1);
        assert_eq!(layer.draws[0].fill, Some(1.0));
    }

    #[test]
    fn empty_layer_gets_unit_bounds() {
        let layer = PathLayer::new(&[]);
        assert_eq!((layer.bounds.w, layer.bounds.h), (1.0, 1.0));
    }
}
