//! Ordered exact-outline coverage and color glyph draws in device space.

use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::overlay::FloatOverlay;
use kurbo::{Affine, BezPath, PathEl, Rect, Shape, Stroke, StrokeOpts};

use super::bitmap::{Bitmaps, Rgba8, has_bitmaps};
use super::color::Rgba;
use super::colr::{Program, RgbaF32};
use super::outline::{Coords, FaceRef, GlyphStyle, Outlines, synthetic_bold_width};
use crate::compat::PlacedGlyph;

#[derive(Clone, Debug, PartialEq)]
pub enum PathStyle {
    Fill,
    Stroke(Stroke),
}

#[derive(Clone, Debug)]
pub struct PlacedPath {
    pub path: Arc<BezPath>,
    pub transform: Affine,
    pub style: PathStyle,
}

/// Maximum curve-to-line deviation in device pixels before area integration.
/// The CPU comparison uses the same cached geometry.
pub const COVERAGE_TOLERANCE: f64 = 1e-4;
const SMALL_COVERAGE_TOLERANCE: f64 = 1e-5;
const SMALL_COVERAGE_EXTENT: f64 = 32.0;

pub(crate) fn flatten_device_path(path: &BezPath) -> BezPath {
    let mut lines = BezPath::new();
    kurbo::flatten(path.iter(), COVERAGE_TOLERANCE, |el| lines.push(el));
    lines
}

fn device_coverage_tolerance(extent: f64) -> f64 {
    if extent.is_finite() && extent <= SMALL_COVERAGE_EXTENT {
        SMALL_COVERAGE_TOLERANCE
    } else {
        COVERAGE_TOLERANCE
    }
}

fn path_coverage_tolerance(path: &BezPath, transform: Affine) -> f64 {
    let bounds = transform.transform_rect_bbox(path.bounding_box());
    device_coverage_tolerance(bounds.width().max(bounds.height()))
}

fn local_coverage_tolerance(transform: Affine, device_tolerance: f64) -> f64 {
    let scale = transform.spectral_norm();
    if scale.is_finite() && scale > 0.0 {
        device_tolerance / scale
    } else {
        device_tolerance
    }
}

/// Return a line-only fill path in page coordinates for one coverage draw.
/// Strokes expand in local space before the full affine is applied, so
/// nonuniform transforms preserve their authored stroke shape.
pub fn coverage_path(placed: &PlacedPath, to_page: Affine) -> BezPath {
    let transform = to_page * placed.transform;
    let tolerance = path_coverage_tolerance(&placed.path, transform);
    match &placed.style {
        PathStyle::Fill => flatten_transformed(&placed.path, transform, tolerance),
        PathStyle::Stroke(stroke) => {
            let path = kurbo::stroke(
                placed.path.iter(),
                stroke,
                &StrokeOpts::default(),
                local_coverage_tolerance(transform, tolerance),
            );
            flatten_transformed(&path, transform, tolerance)
        }
    }
}

fn flatten_transformed(path: &BezPath, transform: Affine, tolerance: f64) -> BezPath {
    let mut count = 0usize;
    kurbo::flatten(path.iter().map(|el| transform * el), tolerance, |_| {
        count += 1
    });
    let mut lines = BezPath::from_vec(Vec::with_capacity(count));
    kurbo::flatten(path.iter().map(|el| transform * el), tolerance, |el| {
        lines.push(el)
    });
    lines
}

/// Reusable nonzero resolver for line-only device paths. The area rasterizers
/// need nonoverlapping filled boundaries: overlapping source contours would
/// count their shared pixels twice.
pub(crate) struct NonzeroResolver {
    contours: Vec<Vec<[f64; 2]>>,
    edges: Vec<ContourEdge>,
    active_edges: Vec<usize>,
    overlay: FloatOverlay<[f64; 2], i64>,
    peak_estimate: usize,
}

#[derive(Clone, Copy)]
struct ContourEdge {
    a: [f64; 2],
    b: [f64; 2],
    contour: usize,
    index: usize,
}

impl ContourEdge {
    fn new(a: [f64; 2], b: [f64; 2], contour: usize, index: usize) -> Self {
        Self {
            a,
            b,
            contour,
            index,
        }
    }

    fn min_x(self) -> f64 {
        self.a[0].min(self.b[0])
    }

    fn max_x(self) -> f64 {
        self.a[0].max(self.b[0])
    }

    fn min_y(self) -> f64 {
        self.a[1].min(self.b[1])
    }

    fn max_y(self) -> f64 {
        self.a[1].max(self.b[1])
    }
}

/// A strict sign is a proof; zero, overflow, and cancellation are unknown.
/// The margin is wider than the standard floating-point orientation bound.
fn orientation_sign(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<i32> {
    let (ax, ay) = (a[0] - c[0], a[1] - c[1]);
    let (bx, by) = (b[0] - c[0], b[1] - c[1]);
    let (left, right) = (ax * by, ay * bx);
    let det = left - right;
    let sum = left.abs() + right.abs();
    if !det.is_finite() || !sum.is_finite() {
        return None;
    }
    let margin = (64.0 * f64::EPSILON * sum).max(f64::MIN_POSITIVE);
    (det.abs() > margin).then_some(if det > 0.0 { 1 } else { -1 })
}

/// A forward continuation meets the preceding edge only at their shared
/// endpoint, including when their turn is too small to certify separately.
fn certified_forward(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let (ux, uy) = (b[0] - a[0], b[1] - a[1]);
    let (vx, vy) = (c[0] - b[0], c[1] - b[1]);
    let (horizontal, vertical) = (ux * vx, uy * vy);
    let dot = horizontal + vertical;
    let sum = horizontal.abs() + vertical.abs();
    if !dot.is_finite() || !sum.is_finite() {
        return false;
    }
    let margin = (64.0 * f64::EPSILON * sum).max(f64::MIN_POSITIVE);
    dot > margin
}

fn certified_separate(a: ContourEdge, b: ContourEdge) -> bool {
    let a0 = orientation_sign(a.a, a.b, b.a);
    let a1 = orientation_sign(a.a, a.b, b.b);
    if a0.is_some() && a0 == a1 {
        return true;
    }
    let b0 = orientation_sign(b.a, b.b, a.a);
    let b1 = orientation_sign(b.a, b.b, a.b);
    b0.is_some() && b0 == b1
}

fn certified_area_sign(contour: &[[f64; 2]]) -> Option<i32> {
    let origin = contour[0];
    let mut sum = 0.0;
    let mut magnitude = 0.0;
    for pair in contour[1..].windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let left = (a[0] - origin[0]) * (b[1] - origin[1]);
        let right = (a[1] - origin[1]) * (b[0] - origin[0]);
        let term = left - right;
        if !term.is_finite() || !left.is_finite() || !right.is_finite() {
            return None;
        }
        sum += term;
        magnitude += left.abs() + right.abs();
    }
    if !sum.is_finite() || !magnitude.is_finite() {
        return None;
    }
    let margin = (64.0 * contour.len() as f64 * f64::EPSILON * magnitude).max(f64::MIN_POSITIVE);
    (sum.abs() > margin).then_some(if sum > 0.0 { 1 } else { -1 })
}

/// Winding of a simple contour at a point certified to be off its boundary.
fn certified_winding(contour: &[[f64; 2]], point: [f64; 2]) -> Option<i32> {
    let mut winding = 0i32;
    for index in 0..contour.len() {
        let a = contour[index];
        let b = contour[(index + 1) % contour.len()];
        if a[1] <= point[1] && point[1] < b[1] {
            if orientation_sign(a, b, point)? > 0 {
                winding = winding.checked_add(1)?;
            }
        } else if b[1] <= point[1] && point[1] < a[1] {
            if orientation_sign(a, b, point)? < 0 {
                winding = winding.checked_sub(1)?;
            }
        }
    }
    Some(winding)
}

impl Default for NonzeroResolver {
    fn default() -> Self {
        Self {
            contours: Vec::new(),
            edges: Vec::new(),
            active_edges: Vec::new(),
            overlay: FloatOverlay::new_empty(Default::default(), Default::default(), 0),
            peak_estimate: 0,
        }
    }
}

impl NonzeroResolver {
    fn start_contour(&mut self, index: usize, point: [f64; 2], point_count: usize) {
        if index == self.contours.len() {
            self.contours.push(Vec::new());
        }
        let contour = &mut self.contours[index];
        contour.clear();
        if contour.capacity() < point_count {
            contour.reserve_exact(point_count);
        }
        contour.push(point);
    }

    fn finish_contour(&mut self, index: &mut usize, open: &mut bool) {
        if !*open {
            return;
        }
        let contour = &mut self.contours[*index];
        if contour.first() == contour.last() {
            contour.pop();
        }
        if contour.len() >= 3 {
            *index += 1;
        }
        *open = false;
    }

    fn parse(&mut self, lines: &BezPath) -> (usize, bool) {
        let (mut active, mut open) = (0usize, false);
        let mut explicit_simple = true;
        let elements = lines.elements();
        for (offset, el) in elements.iter().enumerate() {
            match *el {
                PathEl::MoveTo(p) => {
                    explicit_simple &= !open;
                    self.finish_contour(&mut active, &mut open);
                    let mut point_count = 1;
                    for next in &elements[offset + 1..] {
                        match *next {
                            PathEl::LineTo(_) => point_count += 1,
                            PathEl::MoveTo(_) | PathEl::ClosePath => break,
                            _ => {}
                        }
                    }
                    self.start_contour(active, [p.x, p.y], point_count);
                    open = true;
                }
                PathEl::LineTo(p) if open => {
                    let point = [p.x, p.y];
                    let contour = &mut self.contours[active];
                    if contour.last() != Some(&point) {
                        contour.push(point);
                    }
                }
                PathEl::LineTo(_) => explicit_simple = false,
                PathEl::ClosePath => {
                    explicit_simple &= open && self.contours[active].len() >= 3;
                    self.finish_contour(&mut active, &mut open);
                }
                PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
                    unreachable!("coverage paths were flattened before nonzero resolution")
                }
            }
        }
        explicit_simple &= !open;
        self.finish_contour(&mut active, &mut open);
        (active, explicit_simple && active != 0)
    }

    pub(crate) fn resolve(&mut self, lines: &BezPath) -> BezPath {
        let (active, _) = self.parse(lines);
        self.overlay_active(active)
    }

    /// Preserve the owned path only when its raw contours already describe
    /// the exact nonzero filled boundary. Any uncertified case uses i_overlay.
    pub(crate) fn resolve_owned(&mut self, lines: BezPath) -> BezPath {
        let (active, explicit_simple) = self.parse(&lines);
        if explicit_simple && self.certifies_raw_boundary(active) {
            return lines;
        }
        self.overlay_active(active)
    }

    fn overlay_active(&mut self, active: usize) -> BezPath {
        if active == 0 {
            return BezPath::new();
        }
        self.overlay.reinit_with_subj(&self.contours[..active]);
        let shapes = self
            .overlay
            .overlay(OverlayRule::Subject, FillRule::NonZero);
        let mut resolved = BezPath::new();
        for contour in shapes.iter().flat_map(|shape| shape.iter()) {
            if let Some(first) = contour.first() {
                resolved.move_to((first[0], first[1]));
                for point in &contour[1..] {
                    resolved.line_to((point[0], point[1]));
                }
                resolved.close_path();
            }
        }
        resolved
    }

    /// Certify simple, nonintersecting contours whose winding is either zero
    /// or one common sign everywhere. This last condition matters for area
    /// integration: adjacent opposite-winding outer contours can cancel in a
    /// pixel even though their boundaries never intersect.
    fn certifies_raw_boundary(&mut self, count: usize) -> bool {
        self.edges.clear();
        self.active_edges.clear();
        let edge_count: usize = self.contours[..count].iter().map(Vec::len).sum();
        if self.edges.capacity() < edge_count {
            self.edges.reserve_exact(edge_count);
        }
        for (contour_index, contour) in self.contours[..count].iter().enumerate() {
            if contour.len() < 3 || contour.iter().flatten().any(|v| !v.is_finite()) {
                return false;
            }
            for index in 0..contour.len() {
                let a = contour[index];
                let b = contour[(index + 1) % contour.len()];
                let c = contour[(index + 2) % contour.len()];
                if a == b || (orientation_sign(a, b, c).is_none() && !certified_forward(a, b, c)) {
                    return false;
                }
                self.edges
                    .push(ContourEdge::new(a, b, contour_index, index));
            }
        }
        self.edges
            .sort_unstable_by(|a, b| a.min_x().total_cmp(&b.min_x()));
        for index in 0..self.edges.len() {
            let edge = self.edges[index];
            self.active_edges
                .retain(|&other| self.edges[other].max_x() >= edge.min_x());
            for &other_index in &self.active_edges {
                let other = self.edges[other_index];
                if edge.max_y() < other.min_y() || other.max_y() < edge.min_y() {
                    continue;
                }
                if edge.contour == other.contour {
                    let length = self.contours[edge.contour].len();
                    if (edge.index + 1) % length == other.index
                        || (other.index + 1) % length == edge.index
                    {
                        // Their sole shared endpoint and strict turn or
                        // forward continuation were certified above.
                        continue;
                    }
                }
                if !certified_separate(edge, other) {
                    return false;
                }
            }
            self.active_edges.push(index);
        }

        let mut common_sign = 0i32;
        for (index, contour) in self.contours[..count].iter().enumerate() {
            let sign = match certified_area_sign(contour) {
                Some(sign) => sign,
                None => return false,
            };
            let probe = contour[0];
            let mut outside = 0i32;
            for (other_index, other) in self.contours[..count].iter().enumerate() {
                if index == other_index {
                    continue;
                }
                let winding = match certified_winding(other, probe) {
                    Some(winding) => winding,
                    None => return false,
                };
                outside = match outside.checked_add(winding) {
                    Some(winding) => winding,
                    None => return false,
                };
            }
            let inside = match outside.checked_add(sign) {
                Some(winding) => winding,
                None => return false,
            };
            if (outside == 0) == (inside == 0) {
                return false;
            }
            let filled_sign = if outside == 0 { inside } else { outside };
            if common_sign == 0 {
                common_sign = filled_sign;
            } else if common_sign != filled_sign {
                return false;
            }
        }
        true
    }

    /// i_overlay does not expose its internal capacities. Retain only one
    /// resolver, and discard it after unusually large input/output jobs. The
    /// estimate covers reusable contours plus a conservative allowance for
    /// i_overlay's segment, split, graph, and extraction buffers.
    pub(crate) fn trim_after(
        &mut self,
        input_elements: usize,
        output_elements: usize,
        limit: usize,
    ) {
        let contour_bytes = self.contours.capacity() * size_of::<Vec<[f64; 2]>>()
            + self
                .contours
                .iter()
                .map(|contour| contour.capacity() * size_of::<[f64; 2]>())
                .sum::<usize>();
        let certificate_bytes = self.edges.capacity() * size_of::<ContourEdge>()
            + self.active_edges.capacity() * size_of::<usize>();
        let overlay_allowance = input_elements
            .max(output_elements)
            .saturating_mul(16 * size_of::<[f64; 2]>());
        self.peak_estimate = self.peak_estimate.max(
            contour_bytes
                .saturating_add(certificate_bytes)
                .saturating_add(overlay_allowance),
        );
        if self.peak_estimate > limit {
            *self = Self::default();
        }
    }
}

/// Maximum retained path payload in the default cross-frame cache.
pub const COVERAGE_PATH_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CoverageKey {
    source: usize,
    linear: [u64; 4],
}

struct CoverageEntry {
    // Retaining the source makes its pointer identity safe as a cache key.
    source: Arc<BezPath>,
    style: PathStyle,
    lines: Arc<BezPath>,
    bytes: usize,
    used: u64,
}

/// Bounded LRU of line-only coverage geometry after the placed path's
/// linear transform. Glyph and atlas translations are applied after lookup.
pub struct CoveragePathCache {
    entries: HashMap<CoverageKey, Vec<CoverageEntry>>,
    resolver: NonzeroResolver,
    bytes: usize,
    limit: usize,
    tick: u64,
}

impl Default for CoveragePathCache {
    fn default() -> Self {
        Self::new(COVERAGE_PATH_CACHE_BYTES)
    }
}

impl CoveragePathCache {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            resolver: NonzeroResolver::default(),
            bytes: 0,
            limit,
            tick: 0,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.resolver = NonzeroResolver::default();
        self.bytes = 0;
        self.tick = 0;
    }

    fn oldest(&self) -> Option<(CoverageKey, usize)> {
        self.entries
            .iter()
            .flat_map(|(key, bucket)| {
                bucket
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (*key, index, entry.used))
            })
            .min_by_key(|(_, _, used)| *used)
            .map(|(key, index, _)| (key, index))
    }

    fn evict_oldest(&mut self) {
        let Some((key, index)) = self.oldest() else {
            return;
        };
        let bucket = self.entries.get_mut(&key).expect("oldest entry exists");
        let entry = bucket.swap_remove(index);
        let empty = bucket.is_empty();
        self.bytes -= entry.bytes;
        if empty {
            self.entries.remove(&key);
        }
    }

    /// Reuse line-only geometry for the same retained source, linear affine,
    /// and stroke style. Apply `placed.transform`'s translation separately
    /// when drawing the returned Arc. It remains valid after eviction.
    pub fn get(&mut self, placed: &PlacedPath) -> Arc<BezPath> {
        self.tick = self.tick.wrapping_add(1);
        let [a, b, c, d, _, _] = placed.transform.as_coeffs();
        let key = CoverageKey {
            source: Arc::as_ptr(&placed.path) as usize,
            linear: [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()],
        };
        if let Some(hit) = self.entries.get_mut(&key).and_then(|bucket| {
            bucket.iter_mut().find(|entry| {
                Arc::ptr_eq(&entry.source, &placed.path) && entry.style == placed.style
            })
        }) {
            hit.used = self.tick;
            return hit.lines.clone();
        }
        let linear = PlacedPath {
            transform: Affine::new([a, b, c, d, 0.0, 0.0]),
            ..placed.clone()
        };
        let raw = coverage_path(&linear, Affine::IDENTITY);
        let input_elements = raw.elements().len();
        let elements = self.resolver.resolve_owned(raw).into_elements();
        self.resolver.trim_after(
            input_elements,
            elements.len(),
            self.limit.min(COVERAGE_PATH_CACHE_BYTES),
        );
        let line_capacity = elements.capacity();
        let lines = Arc::new(BezPath::from_vec(elements));
        let bytes = (placed.path.elements().len() + line_capacity)
            .saturating_mul(size_of::<kurbo::PathEl>())
            .saturating_add(size_of::<CoverageEntry>() + size_of::<CoverageKey>());
        if bytes <= self.limit {
            while self.bytes > self.limit - bytes {
                self.evict_oldest();
            }
            self.entries.entry(key).or_default().push(CoverageEntry {
                source: placed.path.clone(),
                style: placed.style.clone(),
                lines: lines.clone(),
                bytes,
                used: self.tick,
            });
            self.bytes += bytes;
        }
        lines
    }
}

#[derive(Clone, Debug)]
pub enum ImagePixels {
    Rgba8(Arc<Rgba8>),
    WorkingF32(Arc<RgbaF32>),
    Colr(Arc<Program>),
}

impl ImagePixels {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Rgba8(i) => (i.width, i.height),
            Self::WorkingF32(i) => (i.width, i.height),
            Self::Colr(i) => (i.width, i.height),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ItemContent {
    Coverage {
        paths: Vec<PlacedPath>,
    },
    Image {
        image: ImagePixels,
        transform: Affine,
    },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub content: ItemContent,
    /// Straight E-encoded paint color; image draws use its alpha.
    pub color: Rgba,
    pub opacity: f64,
    /// Gaussian sigma in device pixels.
    pub blur: f64,
    /// Conservative pre-blur device-space ink box.
    pub bounds: Rect,
}

impl Item {
    pub fn reach(&self) -> Rect {
        let pad = blur_radius(self.blur) as f64 + 1.0;
        self.bounds.inflate(pad, pad)
    }
}

pub fn blur_radius(sigma: f64) -> u32 {
    if sigma > 0.0 && sigma.is_finite() {
        (8.0 * sigma).ceil().min(u32::MAX as f64) as u32
    } else {
        0
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextScene {
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

impl TextScene {
    pub fn ink(&self) -> Option<[u32; 4]> {
        let mut r: Option<Rect> = None;
        for item in &self.items {
            let b = item.reach();
            r = Some(r.map_or(b, |old| old.union(b)));
        }
        let r = r?.intersect(Rect::new(0.0, 0.0, self.width as f64, self.height as f64));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return None;
        }
        Some([
            r.x0.floor().max(0.0) as u32,
            r.y0.floor().max(0.0) as u32,
            r.x1.ceil().min(self.width as f64) as u32,
            r.y1.ceil().min(self.height as f64) as u32,
        ])
    }
}

#[derive(Clone, Copy)]
pub struct Run<'a> {
    pub face: FaceRef<'a>,
    pub size: f32,
    pub coords: Coords<'a>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub oblique: f64,
    pub glyphs: &'a [PlacedGlyph],
    pub bitmap_size: Option<f32>,
}

pub struct Painter<'a> {
    pub outlines: &'a mut Outlines,
    pub bitmaps: &'a mut Bitmaps,
    pub unsupported: Vec<String>,
}

fn skew(run: &Run<'_>) -> f64 {
    if run.synthetic_italic {
        super::outline::SYNTHETIC_ITALIC_SKEW
    } else if run.oblique != 0.0 {
        -run.oblique.to_radians().tan()
    } else {
        0.0
    }
}

fn device_scale(m: Affine) -> f64 {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let scale = (a * d - b * c).abs().sqrt();
    if scale.is_finite() { scale } else { 0.0 }
}

fn device_residual(m: Affine, scale: f64) -> Affine {
    let [a, b, c, d, _, _] = m.as_coeffs();
    Affine::new([a / scale, b / scale, c / scale, d / scale, 0.0, 0.0])
}

impl Painter<'_> {
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        out: &mut Vec<Item>,
        runs: &[Run<'_>],
        origin: (f64, f64),
        m: Affine,
        stroke: Option<f64>,
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) {
        let scale = device_scale(m);
        if scale <= 0.0 || !scale.is_finite() {
            return;
        }
        let residual = device_residual(m, scale);
        for run in runs {
            let device_size = f64::from(run.size) * scale;
            let style = GlyphStyle {
                size: run.size,
                synthetic_bold: if run.synthetic_bold {
                    let outset = synthetic_bold_width(f64::from(run.size)) / 2.0;
                    (outset, outset)
                } else {
                    (0.0, 0.0)
                },
                skew: skew(run),
                expand_tolerance: if run.synthetic_bold {
                    local_coverage_tolerance(
                        m * Affine::skew(skew(run), 0.0),
                        device_coverage_tolerance(device_size),
                    )
                } else {
                    0.0
                },
            };
            for glyph in run.glyphs {
                let glyph_origin = m * kurbo::Point::new(origin.0 + glyph.x, origin.1 + glyph.y);
                let bitmap_size = run.bitmap_size.unwrap_or(device_size as f32);
                if has_bitmaps(run.face)
                    && let Some(bitmap) =
                        self.bitmaps
                            .get(run.face, glyph.id, device_size as f32, bitmap_size)
                {
                    let transform = Affine::translate((glyph_origin.x, glyph_origin.y))
                        * residual
                        * bitmap.transform;
                    let image = ImagePixels::Rgba8(bitmap.image.clone());
                    let (w, h) = image.dimensions();
                    out.push(Item {
                        bounds: transform
                            .transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64)),
                        content: ItemContent::Image { image, transform },
                        color,
                        opacity,
                        blur,
                    });
                    continue;
                }
                if super::bitmap::is_colr_glyph(run.face, glyph.id) {
                    let [a, b, c, d, _, _] = m.as_coeffs();
                    let linear = Affine::new([a, b, c, d, 0.0, 0.0]);
                    match self.bitmaps.colr.get(super::colr::ColrRequest {
                        face: run.face,
                        coords: run.coords,
                        id: glyph.id,
                        size: run.size,
                        skew: skew(run) as f32,
                        linear,
                        origin: (glyph_origin.x, glyph_origin.y),
                        // Fill-style alpha is applied once to the completed
                        // color glyph, including palette-only paints.
                        foreground: [color[0], color[1], color[2], 1.0],
                    }) {
                        Ok(Some(program)) => {
                            let transform =
                                Affine::translate((glyph_origin.x.floor(), glyph_origin.y.floor()))
                                    * program.transform;
                            let image = ImagePixels::Colr(program);
                            let (w, h) = image.dimensions();
                            out.push(Item {
                                bounds: transform
                                    .transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64)),
                                content: ItemContent::Image { image, transform },
                                // Canvas fill-style alpha applies to the whole
                                // color glyph, including palette-only paints.
                                color,
                                opacity,
                                blur,
                            });
                        }
                        Ok(None) => {}
                        Err(e) => self.unsupported.push(format!("COLR: {e}")),
                    }
                    continue;
                }
                let Some(outline) = self.outlines.get(run.face, run.coords, glyph.id, style) else {
                    continue;
                };
                let transform = m * Affine::translate((origin.0 + glyph.x, origin.1 + glyph.y));
                let path_style = if let Some(width) = stroke {
                    PathStyle::Stroke(
                        Stroke::new(width)
                            .with_join(kurbo::Join::Round)
                            .with_miter_limit(2.0),
                    )
                } else {
                    PathStyle::Fill
                };
                let mut b = transform.transform_rect_bbox(outline.bounds);
                if let Some(width) = stroke {
                    b = b.inflate(width * scale / 2.0, width * scale / 2.0);
                }
                out.push(Item {
                    content: ItemContent::Coverage {
                        paths: vec![PlacedPath {
                            path: outline.path.clone(),
                            transform,
                            style: path_style,
                        }],
                    },
                    color,
                    opacity,
                    blur,
                    bounds: b,
                });
            }
        }
    }

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
        self.draw(out, runs, origin, m, None, color, opacity, blur);
    }

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
        if width > 0.0 {
            self.draw(out, runs, origin, m, Some(width), color, opacity, 0.0);
        }
    }
}

pub fn fill_path(path: BezPath, m: Affine, color: Rgba, opacity: f64) -> Option<Item> {
    let bounds = m.transform_rect_bbox(path.bounding_box());
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    Some(Item {
        content: ItemContent::Coverage {
            paths: vec![PlacedPath {
                path: Arc::new(path),
                transform: m,
                style: PathStyle::Fill,
            }],
        },
        color,
        opacity,
        blur: 0.0,
        bounds,
    })
}

/// Analytic outline of the authored rounded caption rectangle.
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
    use i_overlay::float::simplify::SimplifyShape;

    fn fresh_union(contours: &[Vec<[f64; 2]>]) -> BezPath {
        let shapes = contours.simplify_shape_as::<i64>(FillRule::NonZero);
        let mut path = BezPath::new();
        for contour in shapes.iter().flat_map(|shape| shape.iter()) {
            if let Some(first) = contour.first() {
                path.move_to((first[0], first[1]));
                for point in &contour[1..] {
                    path.line_to((point[0], point[1]));
                }
                path.close_path();
            }
        }
        path
    }

    fn line_contours(contours: &[Vec<[f64; 2]>]) -> BezPath {
        let mut path = BezPath::new();
        for contour in contours {
            path.move_to((contour[0][0], contour[0][1]));
            for point in &contour[1..] {
                path.line_to((point[0], point[1]));
            }
            path.close_path();
        }
        path
    }

    fn assert_union_semantics(contours: &[Vec<[f64; 2]>], certified: bool) {
        let raw = line_contours(contours);
        let mut resolver = NonzeroResolver::default();
        let (count, clean) = resolver.parse(&raw);
        assert!(clean);
        assert_eq!(resolver.certifies_raw_boundary(count), certified);
        let resolved = resolver.resolve_owned(raw);
        let fresh = fresh_union(contours);
        for y in -2..13 {
            for x in -2..17 {
                let point = kurbo::Point::new(x as f64 * 0.25 + 0.125, y as f64 * 0.25 + 0.125);
                assert_eq!(resolved.winding(point).abs(), fresh.winding(point).abs());
            }
        }
        assert!((resolved.area().abs() - fresh.area().abs()).abs() < 1e-9);
    }

    #[test]
    fn certified_nonzero_boundary_handles_separate_and_nested_contours() {
        let outer = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let separate = vec![[2.25, 0.125], [3.25, 0.125], [3.25, 1.125], [2.25, 1.125]];
        assert_union_semantics(&[outer.clone(), separate.clone()], true);
        let raw = line_contours(&[outer.clone(), separate.clone()]);
        let raw_buffer = raw.elements().as_ptr();
        let preserved = NonzeroResolver::default().resolve_owned(raw);
        assert_eq!(preserved.elements().as_ptr(), raw_buffer);
        let opposite: Vec<_> = separate.into_iter().rev().collect();
        // Separate opposite-winding contours would cancel in a shared pixel.
        assert_union_semantics(&[outer.clone(), opposite], false);

        let hole = vec![[0.5, 0.5], [0.5, 1.5], [1.5, 1.5], [1.5, 0.5]];
        assert_union_semantics(&[outer.clone(), hole.clone()], true);
        assert_union_semantics(&[outer, hole.into_iter().rev().collect()], false);
    }

    #[test]
    fn certified_nonzero_boundary_rejects_crossings_and_touches() {
        let rect = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let crossing = vec![[1.0, -0.5], [3.0, -0.5], [3.0, 1.5], [1.0, 1.5]];
        assert_union_semantics(&[rect.clone(), crossing], false);
        let touching = vec![[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0]];
        assert_union_semantics(&[rect, touching], false);
        let bowtie = vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]];
        assert_union_semantics(&[bowtie], false);
        let backtrack = vec![[0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]];
        assert_union_semantics(&[backtrack], false);
    }

    #[test]
    fn certified_forward_continuations_preserve_simple_boundaries() {
        let forward = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        assert_union_semantics(&[forward], true);

        let a = [0.0, 0.0];
        let b = [1.0, 1.0];
        let near_collinear = [2.0, 2.0 + 1e-15];
        assert!(orientation_sign(a, b, near_collinear).is_none());
        assert!(certified_forward(a, b, near_collinear));
        assert!(!certified_forward([0.0, 0.0], [2.0, 0.0], [1.0, 0.0]));
    }

    #[test]
    fn duplicate_endpoints_do_not_prevent_boundary_certification() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((2.0, 0.0));
        path.line_to((2.0, 0.0));
        path.line_to((2.0, 2.0));
        path.line_to((0.0, 2.0));
        path.line_to((0.0, 0.0));
        path.close_path();
        let buffer = path.elements().as_ptr();
        let resolved = NonzeroResolver::default().resolve_owned(path);
        assert_eq!(resolved.elements().as_ptr(), buffer);
        assert_eq!(resolved.area(), 4.0);
        assert_eq!(resolved.winding(kurbo::Point::new(1.0, 1.0)), 1);
    }

    #[test]
    fn exact_fractional_placement_is_retained() {
        let path = rounded_rect(0.0, 0.0, 1.0, 1.0, 0.0);
        let item = fill_path(path, Affine::translate((0.125, 0.375)), [1.0; 4], 1.0).unwrap();
        assert_eq!(item.bounds.x0, 0.125);
        assert_eq!(item.bounds.y0, 0.375);
    }

    #[test]
    fn coverage_flatten_reserves_exact_line_count() {
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.curve_to((0.25, 2.0), (1.75, -2.0), (2.0, 0.0));
        curve.close_path();
        let placed = PlacedPath {
            path: Arc::new(curve),
            transform: Affine::translate((0.125, 0.375)),
            style: PathStyle::Fill,
        };
        let elements = coverage_path(&placed, Affine::IDENTITY).into_elements();
        assert_eq!(elements.len(), elements.capacity());
    }

    #[test]
    fn small_device_outlines_use_tighter_curve_tolerance() {
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.curve_to((3.0, 14.0), (11.0, -14.0), (14.0, 0.0));
        curve.close_path();
        let source = Arc::new(curve);
        let small = PlacedPath {
            path: source.clone(),
            transform: Affine::IDENTITY,
            style: PathStyle::Fill,
        };
        let large = PlacedPath {
            path: source,
            transform: Affine::scale(7.0),
            style: PathStyle::Fill,
        };
        assert_eq!(path_coverage_tolerance(&small.path, small.transform), 1e-5);
        assert_eq!(path_coverage_tolerance(&large.path, large.transform), 1e-4);
        let small_lines = coverage_path(&small, Affine::IDENTITY);
        let mut expected = BezPath::new();
        kurbo::flatten(small.path.iter(), 1e-5, |el| expected.push(el));
        assert_eq!(small_lines.elements(), expected.elements());
        assert!(small_lines.elements().len() > 3);
    }

    #[test]
    fn blur_reach_uses_eight_sigma() {
        assert_eq!(blur_radius(2.25), 18);
    }

    #[test]
    fn stroked_path_expands_before_nonuniform_world_transform() {
        let mut line = BezPath::new();
        line.move_to((1.0, 2.0));
        line.line_to((3.0, 2.0));
        let placed = PlacedPath {
            path: Arc::new(line),
            transform: Affine::scale_non_uniform(10.0, 2.0),
            style: PathStyle::Stroke(Stroke::new(2.0)),
        };
        let path = coverage_path(&placed, Affine::translate((5.0, 7.0)));
        assert!(path.elements().iter().all(|el| matches!(
            el,
            kurbo::PathEl::MoveTo(_) | kurbo::PathEl::LineTo(_) | kurbo::PathEl::ClosePath
        )));
        let b = path.bounding_box();
        // Kurbo's default round caps extend one local pixel past each end.
        assert!((b.x0 - 5.0).abs() < 1e-8 && (b.x1 - 45.0).abs() < 1e-8);
        assert!((b.y0 - 9.0).abs() < 1e-8 && (b.y1 - 13.0).abs() < 1e-8);
        assert!(
            (local_coverage_tolerance(placed.transform, SMALL_COVERAGE_TOLERANCE)
                - SMALL_COVERAGE_TOLERANCE / 10.0)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn flattened_coverage_cache_reuses_and_bounds_retained_paths() {
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.quad_to((4.0, 8.0), (8.0, 0.0));
        curve.close_path();
        let source = Arc::new(curve);
        let first = PlacedPath {
            path: source.clone(),
            transform: Affine::IDENTITY,
            style: PathStyle::Fill,
        };
        let shifted = PlacedPath {
            transform: Affine::translate((0.25, 0.0)),
            ..first.clone()
        };
        let scaled = PlacedPath {
            transform: Affine::scale_non_uniform(1.1, 1.0),
            ..first.clone()
        };
        let stroked = PlacedPath {
            style: PathStyle::Stroke(Stroke::new(1.0)),
            ..first.clone()
        };
        let mut unlimited = CoveragePathCache::new(usize::MAX);
        let lines = unlimited.get(&first);
        assert!(Arc::ptr_eq(&lines, &unlimited.get(&first)));
        assert!(Arc::ptr_eq(&lines, &unlimited.get(&shifted)));
        assert!(!Arc::ptr_eq(&lines, &unlimited.get(&scaled)));
        assert!(!Arc::ptr_eq(&lines, &unlimited.get(&stroked)));

        let mut probe = CoveragePathCache::new(usize::MAX);
        probe.get(&first);
        let one_entry_bytes = probe.bytes();
        let mut limited = CoveragePathCache::new(one_entry_bytes);
        let old = limited.get(&first);
        assert!(Arc::ptr_eq(&old, &limited.get(&first)));
        let same_shape_new_source = PlacedPath {
            path: Arc::new((*source).clone()),
            ..first.clone()
        };
        limited.get(&same_shape_new_source);
        assert!(limited.bytes() <= one_entry_bytes);
        assert!(!Arc::ptr_eq(&old, &limited.get(&first)));
    }

    #[test]
    fn cached_nonzero_contours_cover_overlap_once() {
        let mut components = BezPath::new();
        components.move_to((0.0, 0.0));
        components.line_to((2.0, 0.0));
        components.line_to((2.0, 2.0));
        components.line_to((0.0, 2.0));
        components.close_path();
        components.move_to((1.0, 0.0));
        components.line_to((3.0, 0.0));
        components.line_to((3.0, 2.0));
        components.line_to((1.0, 2.0));
        components.close_path();
        let placed = PlacedPath {
            path: Arc::new(components),
            transform: Affine::IDENTITY,
            style: PathStyle::Fill,
        };
        let raw = coverage_path(&placed, Affine::IDENTITY);
        assert_eq!(raw.winding(kurbo::Point::new(1.5, 1.0)), 2);
        let mut cache = CoveragePathCache::default();
        let resolved = cache.get(&placed);
        assert_eq!(resolved.winding(kurbo::Point::new(1.5, 1.0)), 1);
        assert!((resolved.area().abs() - 6.0).abs() < 1e-9);
        assert_eq!(resolved.winding(kurbo::Point::new(3.5, 1.0)), 0);
        let shifted = PlacedPath {
            transform: Affine::translate((0.375, 0.125)),
            ..placed
        };
        assert!(Arc::ptr_eq(&resolved, &cache.get(&shifted)));
    }

    #[test]
    fn cached_nonzero_stroke_resolves_crossing_segments() {
        let mut crossing = BezPath::new();
        crossing.move_to((0.0, 1.0));
        crossing.line_to((4.0, 1.0));
        crossing.move_to((2.0, -1.0));
        crossing.line_to((2.0, 3.0));
        let placed = PlacedPath {
            path: Arc::new(crossing),
            transform: Affine::IDENTITY,
            style: PathStyle::Stroke(Stroke::new(1.0)),
        };
        let raw = coverage_path(&placed, Affine::IDENTITY);
        assert_eq!(raw.winding(kurbo::Point::new(2.0, 1.0)).abs(), 2);
        let resolved = CoveragePathCache::default().get(&placed);
        assert_eq!(resolved.winding(kurbo::Point::new(2.0, 1.0)).abs(), 1);
    }

    #[test]
    fn nonzero_resolver_reuse_matches_fresh_for_compound_and_self_intersecting_paths() {
        let mut compound = BezPath::new();
        for (x0, x1) in [(0.0, 2.0), (1.0, 3.0)] {
            compound.move_to((x0, 0.0));
            compound.line_to((x1, 0.0));
            compound.line_to((x1, 2.0));
            compound.line_to((x0, 2.0));
            compound.close_path();
        }
        let mut crossing = BezPath::new();
        crossing.move_to((0.0, 0.0));
        crossing.line_to((3.0, 3.0));
        crossing.line_to((0.0, 3.0));
        crossing.line_to((3.0, 0.0));
        crossing.close_path();

        let expected_compound = fresh_union(&[
            vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
            vec![[1.0, 0.0], [3.0, 0.0], [3.0, 2.0], [1.0, 2.0]],
        ]);
        let expected_crossing =
            fresh_union(&[vec![[0.0, 0.0], [3.0, 3.0], [0.0, 3.0], [3.0, 0.0]]]);

        let mut reused = NonzeroResolver::default();
        for (path, fresh) in [
            (&compound, &expected_compound),
            (&crossing, &expected_crossing),
            (&compound, &expected_compound),
            (&crossing, &expected_crossing),
        ] {
            let actual = reused.resolve(path);
            assert_eq!(actual.elements(), fresh.elements());
            for y in 0..7 {
                for x in 0..7 {
                    let point = kurbo::Point::new(x as f64 * 0.5 + 0.25, y as f64 * 0.5 + 0.25);
                    assert_eq!(actual.winding(point), fresh.winding(point));
                }
            }
        }
        let resolved_compound = reused.resolve(&compound);
        assert!((resolved_compound.area().abs() - 6.0).abs() < 1e-9);
        let resolved_crossing = reused.resolve(&crossing);
        assert_eq!(
            resolved_crossing.winding(kurbo::Point::new(1.5, 0.5)).abs(),
            1
        );
        assert_eq!(
            resolved_crossing.winding(kurbo::Point::new(1.5, 2.5)).abs(),
            1
        );
        assert_eq!(resolved_crossing.winding(kurbo::Point::new(0.5, 1.5)), 0);

        reused.trim_after(1, 1, 0);
        assert!(reused.contours.is_empty());
    }
}
