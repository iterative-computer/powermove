//! SVG filter graphs in float on a filter grid (docs/engine/vector.md,
//! "SVG filters").
//!
//! A filtered group's source is painted at device resolution and its
//! filter runs there: the grid is the device grid (rate 1), a 4× grid for
//! small Gaussians whose axes a rotation or skew turned away from the
//! device axes, or an authored SVG 1.1 `filterRes` grid. Every image of the
//! graph is RGBA32F, premultiplied, sRGB-encoded between primitives, with
//! its region (`svg_filter_common.wgsl`); each primitive converts its
//! inputs into its `color-interpolation-filters` space.
//!
//! Each image covers only its own rectangle: where its content can be
//! (forward from the source's bounds, each primitive's reach and its
//! subregion) and where the output reads it (backward from the output's
//! rectangle). Memory follows the content, not the blur's reach.
//!
//! Gaussians whose covariance is diagonal on the grid (any axis-aligned
//! placement, and isotropic blurs under rotation) use the exact separable
//! Gaussian shared with path masks (`composite::gaussian`): any σ at O(1)
//! taps per pixel, within 2⁻¹² of the box-reconstruction Gaussian.

use pm_vector::kurbo::{Affine, Rect, Vec2};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::FloatFilter;
use pm_vector::render::svg::usvg_filter as filter;

use super::svg_float::{
    BLEND_WGSL, ScratchPool, bind, blend_code, buffer_entry, pipeline, render_pass, tex_entry,
    uniform,
};
use crate::composite::gaussian::{self, Kernel, Params, Passes, Separable};
use crate::pipelines::LazyRenderPipeline;

/// Direct Gaussian passes (grids not aligned with the blur's axes) extend
/// 6σ: the tail beyond (2e-9 of the mass) is below f32 resolution.
const DIRECT_SUPPORT: f64 = 6.0;
/// A direct Gaussian below this many grid pixels runs on a 4× grid, where
/// its bilinear taps add negligible variance.
const DIRECT_FINE_SIGMA: f64 = 3.0;
/// The largest direct Gaussian (taps × pixels) a filter may encode before
/// the image takes the declared resvg fallback instead of stalling the GPU.
const MAX_DIRECT_TAPS: u64 = 4_000_000_000;
/// The region of an image nothing clipped (a filter's source).
const UNBOUNDED: [f32; 4] = [-1e30, -1e30, 1e30, 1e30];
const NOWHERE: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
/// Where an absent input image is: far from every grid pixel.
const ABSENT: [f32; 2] = [-1e9, -1e9];

const OP_IDENTITY: u32 = 1;
const OP_FLOOD: u32 = 2;
const OP_COMPOSITE: u32 = 3;
const OP_BLEND: u32 = 4;
const OP_MATRIX: u32 = 5;
const OP_TRANSFER: u32 = 6;
const OP_TINT: u32 = 7;

/// One image of the graph: its pixels over `rect` (grid pixels) and the
/// user-space region they are clipped to. An image without a texture is
/// transparent.
#[derive(Clone)]
pub(super) struct Image {
    pub texture: Option<wgpu::Texture>,
    pub rect: PixelRect,
    region: [f32; 4],
}

impl Image {
    /// The filter's source: unclipped.
    pub(super) fn source(texture: wgpu::Texture, rect: PixelRect) -> Image {
        Image {
            texture: Some(texture),
            rect,
            region: UNBOUNDED,
        }
    }

    fn empty() -> Image {
        Image {
            texture: None,
            rect: PixelRect::new(0, 0, 0, 0),
            region: NOWHERE,
        }
    }

    fn origin(&self) -> [f32; 2] {
        if self.texture.is_some() {
            [self.rect.x as f32, self.rect.y as f32]
        } else {
            ABSENT
        }
    }

    pub(super) fn release(self, pool: &mut ScratchPool) {
        if let Some(t) = self.texture {
            pool.release(&t);
        }
    }
}

/// The `Fe` uniform of `svg_filter_common.wgsl`.
#[derive(Clone, Copy)]
struct Fe([f32; 68]);

impl Fe {
    fn new(transform: Affine, output: [f32; 4], linear: bool) -> Fe {
        let inv = transform.inverse().as_coeffs();
        let c = transform.as_coeffs();
        let aligned = c[1] == 0.0 && c[2] == 0.0;
        let mut v = [0.0f32; 68];
        v[0..4].copy_from_slice(&[
            inv[0] as f32,
            inv[2] as f32,
            inv[4] as f32,
            f32::from(aligned),
        ]);
        v[4..8].copy_from_slice(&[inv[1] as f32, inv[3] as f32, inv[5] as f32, 0.0]);
        v[24..28].copy_from_slice(&output);
        let mut fe = Fe(v);
        fe.set_mode([0, if linear { 4 } else { 0 }, 0, 0]);
        fe
    }
    /// The output image's rectangle and the inputs (origins, regions).
    fn images(mut self, out: PixelRect, inputs: [Option<&Image>; 2]) -> Fe {
        self.0[8..10].copy_from_slice(&[out.x as f32, out.y as f32]);
        let [a, b] = inputs.map(|i| i.map_or(ABSENT, Image::origin));
        self.0[12..16].copy_from_slice(&[a[0], a[1], b[0], b[1]]);
        let [qa, qb] = inputs.map(|i| i.map_or(NOWHERE, |i| i.region));
        self.0[16..20].copy_from_slice(&qa);
        self.0[20..24].copy_from_slice(&qb);
        self
    }
    /// The first input's region, as given.
    fn region1(mut self, q: [f32; 4]) -> Fe {
        self.0[16..20].copy_from_slice(&q);
        self
    }
    fn k(mut self, k: [f32; 4]) -> Fe {
        self.0[28..32].copy_from_slice(&k);
        self
    }
    fn mode(&self) -> [u32; 4] {
        std::array::from_fn(|i| self.0[32 + i].to_bits())
    }
    fn set_mode(&mut self, mode: [u32; 4]) {
        for (i, m) in mode.iter().enumerate() {
            self.0[32 + i] = f32::from_bits(*m);
        }
    }
    fn linear(&self) -> bool {
        self.mode()[1] & 4 != 0
    }
    /// Operator, sub-operator and alpha-only inputs (the colour space flag
    /// stays).
    fn op(mut self, op: u32, sub: u32, alpha: [bool; 2]) -> Fe {
        let [_, flags, _, radius] = self.mode();
        let flags = (flags & 4) | u32::from(alpha[0]) | (u32::from(alpha[1]) << 1);
        self.set_mode([op, flags, sub, radius]);
        self
    }
    fn radius(mut self, radius: u32) -> Fe {
        let [op, flags, sub, _] = self.mode();
        self.set_mode([op, flags, sub, radius]);
        self
    }
    fn m(mut self, m: &[f32]) -> Fe {
        self.0[36..36 + m.len()].copy_from_slice(m);
        self
    }
}

fn isect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

fn shift(r: [f32; 4], d: [f32; 2]) -> [f32; 4] {
    if r == UNBOUNDED {
        return r;
    }
    [r[0] + d[0], r[1] + d[1], r[2] + d[0], r[3] + d[1]]
}

fn rect4(r: pm_vector::render::svg::SvgRect) -> [f32; 4] {
    [r.left(), r.top(), r.right(), r.bottom()]
}

/// `r` grown by `d` grid pixels per axis.
pub(super) fn grow(r: PixelRect, d: [u32; 2]) -> PixelRect {
    PixelRect::new(
        r.x.saturating_sub(d[0] as i32),
        r.y.saturating_sub(d[1] as i32),
        r.width.saturating_add(2 * d[0]),
        r.height.saturating_add(2 * d[1]),
    )
}

fn union(a: Option<PixelRect>, b: Option<PixelRect>) -> Option<PixelRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

fn meet(a: Option<PixelRect>, b: Option<PixelRect>) -> Option<PixelRect> {
    a.zip(b)
        .and_then(|(a, b)| a.intersect(b))
        .filter(|r| r.width > 0 && r.height > 0)
}

fn area(r: Option<PixelRect>) -> u64 {
    r.map_or(0, |r| u64::from(r.width) * u64::from(r.height))
}

/// A Gaussian's standard deviations on `transform`'s grid: `Ok` for one
/// along each grid axis (its covariance is diagonal there), `Err` for one
/// along each of the transform's columns.
fn sigmas(std_dev: [f64; 2], transform: Affine) -> Result<[f64; 2], [(Vec2, f64); 2]> {
    let c = transform.as_coeffs();
    let (a0, a1) = (Vec2::new(c[0], c[1]), Vec2::new(c[2], c[3]));
    let (sx, sy) = (std_dev[0] * std_dev[0], std_dev[1] * std_dev[1]);
    let xx = sx * a0.x * a0.x + sy * a1.x * a1.x;
    let yy = sx * a0.y * a0.y + sy * a1.y * a1.y;
    let xy = sx * a0.x * a0.y + sy * a1.x * a1.y;
    if xy.abs() <= 1e-9 * (xx * yy).sqrt().max(1e-300) {
        return Ok([xx.sqrt(), yy.sqrt()]);
    }
    let axis = |a: Vec2, s: f64| {
        let l = a.hypot();
        (if l > 0.0 { a / l } else { Vec2::new(1.0, 0.0) }, s * l)
    };
    Err([axis(a0, std_dev[0]), axis(a1, std_dev[1])])
}

fn std_devs(kind: &filter::Kind) -> Option<[f64; 2]> {
    match kind {
        filter::Kind::GaussianBlur(b) => Some([b.std_dev_x().get(), b.std_dev_y().get()]),
        filter::Kind::DropShadow(d) => Some([d.std_dev_x().get(), d.std_dev_y().get()]),
        _ => None,
    }
    .map(|s| s.map(f64::from))
}

fn offset_of(kind: &filter::Kind) -> Option<[f32; 2]> {
    match kind {
        filter::Kind::Offset(o) => Some([o.dx(), o.dy()]),
        filter::Kind::DropShadow(d) => Some([d.dx(), d.dy()]),
        _ => None,
    }
}

/// How far a Gaussian of `std_dev` user units reads, in grid pixels.
fn blur_reach(std_dev: [f64; 2], transform: Affine) -> [f64; 2] {
    match sigmas(std_dev, transform) {
        Ok(sigma) => sigma.map(|s| Kernel::new(s).map_or(0, |k| k.extent) as f64),
        Err(axes) => {
            let mut r = [1.0; 2];
            for (a, s) in axes {
                r[0] += DIRECT_SUPPORT * s * a.x.abs();
                r[1] += DIRECT_SUPPORT * s * a.y.abs();
            }
            r
        }
    }
}

/// How far a shift of `d` user units reads, in grid pixels.
fn shift_reach(d: [f32; 2], transform: Affine) -> [f64; 2] {
    let c = transform.as_coeffs();
    let (dx, dy) = (f64::from(d[0]), f64::from(d[1]));
    [
        (c[0] * dx + c[2] * dy).abs() + 1.0,
        (c[1] * dx + c[3] * dy).abs() + 1.0,
    ]
}

/// How far one primitive reads its inputs, in grid pixels.
fn reach(kind: &filter::Kind, transform: Affine) -> [u32; 2] {
    let mut r = [0.0f64; 2];
    if let Some(s) = std_devs(kind) {
        let b = blur_reach(s, transform);
        r = [r[0] + b[0], r[1] + b[1]];
    }
    if let Some(d) = offset_of(kind) {
        let o = shift_reach(d, transform);
        r = [r[0] + o[0], r[1] + o[1]];
    }
    r.map(|v| v.min(f64::from(i32::MAX / 8)).ceil() as u32)
}

/// The grid a filter runs on, as a multiple of the device grid: 4 when a
/// small Gaussian's axes are not the device axes (`DIRECT_FINE_SIGMA`).
pub(super) fn grid_rate(f: &FloatFilter) -> u32 {
    if f.resolution.is_some() {
        return 1;
    }
    let fine = f.filter.primitives().iter().any(|p| {
        std_devs(p.kind()).is_some_and(|s| match sigmas(s, f.transform) {
            Ok(_) => false,
            Err(axes) => axes.iter().any(|(_, s)| *s > 0.0 && *s < DIRECT_FINE_SIGMA),
        })
    });
    if fine { 4 } else { 1 }
}

/// How far, in device pixels, a filter's output reads its source per axis.
pub(super) fn halo(f: &FloatFilter) -> Result<[f64; 2], String> {
    let c = f.transform.as_coeffs();
    let mut halo = [0.0f64; 2];
    if let Some(Some([rw, rh])) = f.resolution {
        let r = f.filter.rect();
        let cell = [
            f64::from(r.width()) / f64::from(rw),
            f64::from(r.height()) / f64::from(rh),
        ];
        for axis in 0..2 {
            halo[axis] += 2.0 * (c[axis].abs() * cell[0] + c[axis + 2].abs() * cell[1]);
        }
    }
    for p in f.filter.primitives() {
        let r = reach(p.kind(), f.transform);
        halo = [halo[0] + f64::from(r[0]), halo[1] + f64::from(r[1])];
    }
    if !halo
        .iter()
        .all(|v| v.is_finite() && *v < f64::from(i32::MAX) / 8.0)
    {
        return Err("SVG filter reach exceeds device coordinates".into());
    }
    Ok(halo)
}

/// The producer of each primitive's inputs, by the result name in force.
fn producers(primitives: &[filter::Primitive]) -> Vec<Vec<Option<usize>>> {
    let mut named = std::collections::HashMap::<&str, usize>::new();
    let mut out = Vec::with_capacity(primitives.len());
    for (i, p) in primitives.iter().enumerate() {
        out.push(
            inputs(p.kind())
                .iter()
                .map(|input| match input {
                    filter::Input::Reference(name) => named.get(name.as_str()).copied(),
                    _ => None,
                })
                .collect(),
        );
        named.insert(p.result(), i);
    }
    out
}

fn inputs(kind: &filter::Kind) -> Vec<&filter::Input> {
    match kind {
        filter::Kind::Blend(b) => vec![b.input1(), b.input2()],
        filter::Kind::Composite(c) => vec![c.input1(), c.input2()],
        filter::Kind::ColorMatrix(c) => vec![c.input()],
        filter::Kind::ComponentTransfer(c) => vec![c.input()],
        filter::Kind::DropShadow(d) => vec![d.input()],
        filter::Kind::GaussianBlur(b) => vec![b.input()],
        filter::Kind::Offset(o) => vec![o.input()],
        filter::Kind::Merge(m) => m.inputs().iter().collect(),
        _ => Vec::new(),
    }
}

/// Whether a primitive paints where its inputs are transparent.
fn makes_content(kind: &filter::Kind) -> bool {
    match kind {
        filter::Kind::Flood(f) => f.opacity().get() > 0.0,
        filter::Kind::Composite(c) => matches!(
            c.operator(),
            filter::CompositeOperator::Arithmetic { k4, .. } if k4 > 0.0
        ),
        filter::Kind::ColorMatrix(c) => color_matrix(c.kind())[19] > 0.0,
        filter::Kind::ComponentTransfer(c) => match c.func_a() {
            filter::TransferFunction::Identity => false,
            filter::TransferFunction::Table(v) | filter::TransferFunction::Discrete(v) => {
                v.first().is_some_and(|v| *v > 0.0)
            }
            filter::TransferFunction::Linear { intercept, .. } => *intercept > 0.0,
            filter::TransferFunction::Gamma { offset, .. } => *offset > 0.0,
        },
        _ => false,
    }
}

/// Every image's rectangle in one filter, in grid pixels.
#[derive(Debug)]
pub(super) struct Plan {
    /// Per primitive; `None` is never read or always transparent.
    rects: Vec<Option<PixelRect>>,
    /// Where the output reads the source.
    pub source: Option<PixelRect>,
    /// Where the output can have content.
    pub output: Option<PixelRect>,
    /// Bytes of every image at once (an upper bound of the live set).
    pub bytes: u64,
}

impl Plan {
    /// Plans `f` under `transform` (user to grid pixels): its source has
    /// content within `source`, and its output is read over `need`.
    pub(super) fn new(
        f: &FloatFilter,
        transform: Affine,
        source: Option<PixelRect>,
        need: PixelRect,
    ) -> Plan {
        let primitives = f.filter.primitives();
        let producers = producers(primitives);
        let region = f.filter.rect();
        let region_rect = Rect::new(
            f64::from(region.left()),
            f64::from(region.top()),
            f64::from(region.right()),
            f64::from(region.bottom()),
        );
        if f.resolution.is_some() {
            // An authored grid spreads content over its cells: the output
            // is the region, read from the source within the filter's halo.
            let output = meet(
                Some(need),
                Some(PixelRect::round_out(
                    transform.transform_rect_bbox(region_rect),
                )),
            );
            let halo = halo(f).unwrap_or([f64::from(i32::MAX / 8); 2]);
            let read = output.map(|o| grow(o, halo.map(|v| v.ceil() as u32 + 1)));
            let source = meet(read, source);
            return Plan {
                rects: vec![output; primitives.len()],
                source,
                output,
                bytes: (area(output) + area(source)).saturating_mul(16),
            };
        }
        let subregions: Vec<Option<PixelRect>> = primitives
            .iter()
            .map(|p| {
                let r = isect(rect4(region), rect4(p.rect()));
                (r[0] < r[2] && r[1] < r[3]).then(|| {
                    PixelRect::round_out(transform.transform_rect_bbox(Rect::new(
                        f64::from(r[0]),
                        f64::from(r[1]),
                        f64::from(r[2]),
                        f64::from(r[3]),
                    )))
                })
            })
            .collect();
        let reaches: Vec<[u32; 2]> = primitives
            .iter()
            .map(|p| reach(p.kind(), transform))
            .collect();
        // Forward: where each result can have content.
        let mut support: Vec<Option<PixelRect>> = Vec::with_capacity(primitives.len());
        for (i, p) in primitives.iter().enumerate() {
            let base = if makes_content(p.kind()) {
                subregions[i]
            } else {
                inputs(p.kind())
                    .iter()
                    .zip(&producers[i])
                    .map(|(input, j)| match (input, j) {
                        (filter::Input::Reference(_), Some(j)) => support[*j],
                        (filter::Input::Reference(_), None) => None,
                        _ => source,
                    })
                    .fold(None, union)
            };
            support.push(meet(base.map(|b| grow(b, reaches[i])), subregions[i]));
        }
        // Backward: where each result is read.
        let mut needed: Vec<Option<PixelRect>> = vec![None; primitives.len()];
        let mut source_need = None;
        if let Some(last) = needed.last_mut() {
            *last = Some(need);
        }
        for i in (0..primitives.len()).rev() {
            let Some(n) = meet(needed[i], support[i]) else {
                continue;
            };
            let read = Some(grow(n, reaches[i]));
            for (input, j) in inputs(primitives[i].kind()).iter().zip(&producers[i]) {
                match (input, j) {
                    (filter::Input::Reference(_), Some(j)) => needed[*j] = union(needed[*j], read),
                    (filter::Input::Reference(_), None) => {}
                    _ => source_need = union(source_need, read),
                }
            }
        }
        let rects: Vec<Option<PixelRect>> = needed
            .iter()
            .zip(&support)
            .map(|(n, s)| meet(*n, *s))
            .collect();
        let source = meet(source_need, source);
        // A Gaussian's own node buffers are at most two of its images.
        let blurs: u64 = primitives
            .iter()
            .zip(&rects)
            .filter(|(p, _)| std_devs(p.kind()).is_some())
            .map(|(_, r)| 3 * area(*r))
            .max()
            .unwrap_or(0);
        let bytes =
            (rects.iter().map(|r| area(*r)).sum::<u64>() + area(source) + blurs).saturating_mul(16);
        Plan {
            output: rects.last().copied().flatten(),
            rects,
            source,
            bytes,
        }
    }
}

/// Filter Effects §15.10's matrices for every `feColorMatrix` type (4 × 5,
/// row-major, on straight colour).
fn color_matrix(kind: &filter::ColorMatrixKind) -> [f32; 20] {
    let rgb = |m: [f32; 9]| {
        [
            m[0], m[1], m[2], 0.0, 0.0, m[3], m[4], m[5], 0.0, 0.0, m[6], m[7], m[8], 0.0, 0.0,
            0.0, 0.0, 0.0, 1.0, 0.0,
        ]
    };
    match kind {
        filter::ColorMatrixKind::Matrix(m) => std::array::from_fn(|i| m[i]),
        filter::ColorMatrixKind::Saturate(s) => {
            let s = s.get();
            rgb([
                0.213 + 0.787 * s,
                0.715 - 0.715 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 + 0.285 * s,
                0.072 - 0.072 * s,
                0.213 - 0.213 * s,
                0.715 - 0.715 * s,
                0.072 + 0.928 * s,
            ])
        }
        filter::ColorMatrixKind::HueRotate(degrees) => {
            let (sin, cos) = degrees.to_radians().sin_cos();
            rgb([
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ])
        }
        filter::ColorMatrixKind::LuminanceToAlpha => {
            let mut m = [0.0; 20];
            m[15..18].copy_from_slice(&[0.2125, 0.7154, 0.0721]);
            m
        }
    }
}

/// `feComponentTransfer`'s four functions: per channel (kind, count,
/// table start) and (slope or amplitude, intercept or exponent, offset),
/// with every table's values.
fn transfer(ct: &filter::ComponentTransfer) -> ([f32; 32], Vec<f32>) {
    let mut m = [0.0f32; 32];
    let mut table = Vec::new();
    for (i, f) in [ct.func_r(), ct.func_g(), ct.func_b(), ct.func_a()]
        .into_iter()
        .enumerate()
    {
        let start = table.len() as f32;
        let (kind, n, q) = match f {
            filter::TransferFunction::Identity => (0.0, 0, [0.0; 3]),
            filter::TransferFunction::Table(v) => {
                table.extend_from_slice(v);
                (1.0, v.len(), [0.0; 3])
            }
            filter::TransferFunction::Discrete(v) => {
                table.extend_from_slice(v);
                (2.0, v.len(), [0.0; 3])
            }
            filter::TransferFunction::Linear { slope, intercept } => {
                (3.0, 0, [*slope, *intercept, 0.0])
            }
            filter::TransferFunction::Gamma {
                amplitude,
                exponent,
                offset,
            } => (4.0, 0, [*amplitude, *exponent, *offset]),
        };
        m[i * 4..i * 4 + 4].copy_from_slice(&[kind, n as f32, start, 0.0]);
        m[16 + i * 4..16 + i * 4 + 3].copy_from_slice(&q);
    }
    (m, table)
}

/// Erf-integrated Gaussian weights (the box reconstruction), 6σ each way.
fn direct_weights(sigma: f64) -> Result<Vec<f32>, String> {
    if !sigma.is_finite() || sigma < 0.0 {
        return Err(format!("invalid SVG Gaussian standard deviation {sigma}"));
    }
    if sigma == 0.0 {
        return Ok(vec![1.0]);
    }
    let radius = (DIRECT_SUPPORT * sigma).ceil() as i32;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    Ok((-radius..=radius)
        .map(|k| {
            let k = f64::from(k);
            (phi((k + 0.5) / sigma) - phi((k - 0.5) / sigma)) as f32
        })
        .collect())
}

pub(super) struct Filters {
    layout: wgpu::BindGroupLayout,
    point: wgpu::RenderPipeline,
    offset: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    replicate: wgpu::RenderPipeline,
    resample: wgpu::RenderPipeline,
    resample_layout: wgpu::BindGroupLayout,
    separable: Separable,
    gauss: LazyRenderPipeline,
    /// Bound where a pass has no second input or table.
    blank: wgpu::Texture,
    empty: wgpu::Buffer,
}

/// One pass: a new image over `rect` from up to two inputs.
struct Pass<'a> {
    pipeline: &'a wgpu::RenderPipeline,
    inputs: [Option<&'a Image>; 2],
    fe: Fe,
    table: Option<&'a [f32]>,
    rect: PixelRect,
}

impl Filters {
    pub(super) fn new(device: &wgpu::Device) -> Filters {
        let common = include_str!("svg_filter_common.wgsl");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg filter"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{common}\n{BLEND_WGSL}\n{}",
                    include_str!("svg_filter.wgsl")
                )
                .into(),
            ),
        });
        let gauss_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg filter gaussian"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{common}\n{}\n{}",
                    include_str!("svg_filter_gauss.wgsl"),
                    include_str!("../composite/gaussian.wgsl")
                )
                .into(),
            ),
        });
        let resample_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg filterRes resample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svg_filter_resample.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg filter"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                buffer_entry(2, false),
                buffer_entry(3, true),
            ],
        });
        let resample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg filterRes resample"),
            entries: &[tex_entry(0), buffer_entry(1, false)],
        });
        let f32x4 = wgpu::TextureFormat::Rgba32Float;
        let make = |entry| pipeline(device, (&module, entry), &layout, f32x4, None);
        Filters {
            point: make("fs_point"),
            offset: make("fs_offset"),
            blur: make("fs_blur"),
            replicate: make("fs_replicate"),
            resample: pipeline(
                device,
                (&resample_module, "fs"),
                &resample_layout,
                f32x4,
                None,
            ),
            resample_layout,
            layout,
            separable: Separable::new(device, &gauss_module),
            gauss: gaussian::render(device, &gauss_module, "fs_gauss", f32x4),
            blank: super::svg_float::float_target(device, 1, 1),
            empty: gaussian::storage(device, "pm svg filter table", &[0; 16]),
        }
    }

    /// A per-pixel primitive's pass over `rect`.
    fn point<'a>(
        &'a self,
        fe: Fe,
        inputs: [Option<&'a Image>; 2],
        table: Option<&'a [f32]>,
        rect: PixelRect,
    ) -> Pass<'a> {
        Pass {
            pipeline: &self.point,
            inputs,
            fe,
            table,
            rect,
        }
    }

    fn pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pass: Pass<'_>,
        region: [f32; 4],
        pool: &mut ScratchPool,
    ) -> Image {
        let fe = pass.fe.images(pass.rect, pass.inputs);
        self.encode(device, encoder, pass, fe, region, pool)
    }

    /// [`Filters::pass`] with its uniform as given.
    fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pass: Pass<'_>,
        fe: Fe,
        region: [f32; 4],
        pool: &mut ScratchPool,
    ) -> Image {
        let ub = uniform(device, &fe.0);
        let tb = pass
            .table
            .filter(|t| !t.is_empty())
            .map(|t| gaussian::storage(device, "pm svg filter table", gaussian::f32_bytes(t)));
        let views = pass.inputs.map(|i| {
            i.and_then(|i| i.texture.as_ref())
                .unwrap_or(&self.blank)
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let bg = bind(
            device,
            &self.layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: tb.as_ref().unwrap_or(&self.empty).as_entire_binding(),
                },
            ],
        );
        let out = pool.target(device, pass.rect.width, pass.rect.height);
        render_pass(encoder, &out, true, pass.pipeline, &bg);
        Image {
            texture: Some(out),
            rect: pass.rect,
            region,
        }
    }

    /// `source` (device pixels) repeated onto the `rate`× grid over the
    /// same area.
    pub(super) fn replicate(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Image,
        rate: u32,
        pool: &mut ScratchPool,
    ) -> Image {
        let r = source.rect;
        let rect = PixelRect::new(
            r.x * rate as i32,
            r.y * rate as i32,
            r.width * rate,
            r.height * rate,
        );
        let mut fe = Fe::new(Affine::IDENTITY, NOWHERE, false);
        fe.set_mode([0, 0, rate, 0]);
        let pass = Pass {
            pipeline: &self.replicate,
            inputs: [Some(source), None],
            fe,
            table: None,
            rect,
        };
        self.pass(device, encoder, pass, UNBOUNDED, pool)
    }

    /// Runs `f` (planned by `plan`) under `transform` (user to grid
    /// pixels) on `source`, and returns its output over `plan.output`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        f: &FloatFilter,
        plan: &Plan,
        transform: Affine,
        source: &Image,
        pool: &mut ScratchPool,
    ) -> Result<Image, String> {
        if !transform
            .inverse()
            .as_coeffs()
            .iter()
            .all(|v| v.is_finite())
        {
            return Err("singular SVG filter transform".into());
        }
        let Some(output) = plan.output else {
            return Ok(Image::empty());
        };
        match f.resolution {
            // SVG 1.1: an invalid or zero filterRes disables the element.
            Some(None) => Ok(Image::empty()),
            Some(Some([w, h])) => {
                let limits = device.limits();
                let r = f.filter.rect();
                let to_grid = Affine::scale_non_uniform(
                    f64::from(w) / f64::from(r.width()),
                    f64::from(h) / f64::from(r.height()),
                ) * Affine::translate((-f64::from(r.left()), -f64::from(r.top())));
                let grid = PixelRect::new(0, 0, w, h);
                let grid_plan = Plan::new(f, to_grid, Some(grid), grid);
                if w > limits.max_texture_dimension_2d
                    || h > limits.max_texture_dimension_2d
                    || grid_plan.bytes > super::svg_float::MAX_FILTER_BYTES
                {
                    return Err("SVG filterRes grid exceeds device or filter memory budget".into());
                }
                let region = [r.left(), r.top(), r.width(), r.height()];
                let on_grid = self.resample(
                    device,
                    encoder,
                    source,
                    grid,
                    [w, h],
                    region,
                    transform,
                    false,
                    pool,
                )?;
                let filtered =
                    self.graph(device, encoder, f, &grid_plan, to_grid, &on_grid, pool)?;
                on_grid.release(pool);
                let out = self.resample(
                    device,
                    encoder,
                    &filtered,
                    output,
                    [w, h],
                    region,
                    transform.inverse(),
                    true,
                    pool,
                )?;
                filtered.release(pool);
                Ok(out)
            }
            None => self.graph(device, encoder, f, plan, transform, source, pool),
        }
    }

    /// `source` resampled onto `rect`: from the device grid onto a
    /// filterRes grid (`project` false), or back (`project` true).
    #[allow(clippy::too_many_arguments)]
    fn resample(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Image,
        rect: PixelRect,
        grid: [u32; 2],
        region: [f32; 4],
        transform: Affine,
        project: bool,
        pool: &mut ScratchPool,
    ) -> Result<Image, String> {
        let Some(texture) = &source.texture else {
            return Ok(Image::empty());
        };
        let c = transform.as_coeffs();
        if !c.iter().all(|v| v.is_finite()) {
            return Err("singular SVG filterRes transform".into());
        }
        let p = [
            region[0],
            region[1],
            region[2],
            region[3],
            c[0] as f32,
            c[2] as f32,
            c[4] as f32,
            0.0,
            c[1] as f32,
            c[3] as f32,
            c[5] as f32,
            0.0,
            source.rect.x as f32,
            source.rect.y as f32,
            rect.x as f32,
            rect.y as f32,
            grid[0] as f32,
            grid[1] as f32,
            0.0,
            0.0,
            f32::from(project),
            1.0,
            0.0,
            0.0,
        ];
        let ub = uniform(device, &p);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = bind(
            device,
            &self.resample_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ub.as_entire_binding(),
                },
            ],
        );
        let out = pool.target(device, rect.width, rect.height);
        render_pass(encoder, &out, true, &self.resample, &bg);
        Ok(Image {
            texture: Some(out),
            rect,
            region: UNBOUNDED,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn graph(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        f: &FloatFilter,
        plan: &Plan,
        transform: Affine,
        source: &Image,
        pool: &mut ScratchPool,
    ) -> Result<Image, String> {
        let primitives = f.filter.primitives();
        let region = rect4(f.filter.rect());
        let producers = producers(primitives);
        let mut last = vec![usize::MAX; primitives.len()];
        for (i, sources) in producers.iter().enumerate() {
            for j in sources.iter().flatten() {
                last[*j] = i;
            }
        }
        let mut results: Vec<Option<Image>> = vec![None; primitives.len()];
        for (i, p) in primitives.iter().enumerate() {
            let Some(rect) = plan.rects[i] else {
                results[i] = Some(Image::empty());
                continue;
            };
            let s = isect(region, rect4(p.rect()));
            let linear = p.color_interpolation() == filter::ColorInterpolation::LinearRGB;
            let fe = Fe::new(transform, s, linear);
            let sources = inputs(p.kind());
            let input = |k: usize| -> Result<(Image, bool), String> {
                match (sources[k], producers[i][k]) {
                    (filter::Input::SourceGraphic, _) => Ok((source.clone(), false)),
                    (filter::Input::SourceAlpha, _) => Ok((source.clone(), true)),
                    (filter::Input::Reference(_), Some(j)) => results[j]
                        .clone()
                        .map(|image| (image, false))
                        .ok_or_else(|| "SVG filter input was released".to_owned()),
                    (filter::Input::Reference(_), None) => Ok((Image::empty(), false)),
                }
            };
            let out = match p.kind() {
                filter::Kind::Flood(fl) => {
                    let c = fl.color();
                    let a = fl.opacity().get();
                    let k = [
                        f32::from(c.red) / 255.0 * a,
                        f32::from(c.green) / 255.0 * a,
                        f32::from(c.blue) / 255.0 * a,
                        a,
                    ];
                    let fe = fe.op(OP_FLOOD, 0, [false; 2]).k(k);
                    self.pass(
                        device,
                        encoder,
                        self.point(fe, [None, None], None, rect),
                        s,
                        pool,
                    )
                }
                filter::Kind::Composite(c) => {
                    let ((a, aa), (b, ba)) = (input(0)?, input(1)?);
                    let (sub, k) = match c.operator() {
                        filter::CompositeOperator::Over => (0, [0.0; 4]),
                        filter::CompositeOperator::In => (1, [0.0; 4]),
                        filter::CompositeOperator::Out => (2, [0.0; 4]),
                        filter::CompositeOperator::Atop => (3, [0.0; 4]),
                        filter::CompositeOperator::Xor => (4, [0.0; 4]),
                        filter::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                            (5, [k1, k2, k3, k4])
                        }
                    };
                    let fe = fe.op(OP_COMPOSITE, sub, [aa, ba]).k(k);
                    self.pass(
                        device,
                        encoder,
                        self.point(fe, [Some(&a), Some(&b)], None, rect),
                        s,
                        pool,
                    )
                }
                filter::Kind::Blend(bl) => {
                    let ((a, aa), (b, ba)) = (input(0)?, input(1)?);
                    let fe = fe.op(OP_BLEND, blend_code(bl.mode()), [aa, ba]);
                    self.pass(
                        device,
                        encoder,
                        self.point(fe, [Some(&a), Some(&b)], None, rect),
                        s,
                        pool,
                    )
                }
                filter::Kind::ColorMatrix(c) => {
                    let (a, aa) = input(0)?;
                    let fe = fe.op(OP_MATRIX, 0, [aa, false]).m(&color_matrix(c.kind()));
                    self.pass(
                        device,
                        encoder,
                        self.point(fe, [Some(&a), None], None, rect),
                        s,
                        pool,
                    )
                }
                filter::Kind::ComponentTransfer(ct) => {
                    let (a, aa) = input(0)?;
                    let (m, table) = transfer(ct);
                    let fe = fe.op(OP_TRANSFER, 0, [aa, false]).m(&m);
                    let pass = self.point(fe, [Some(&a), None], Some(&table), rect);
                    self.pass(device, encoder, pass, s, pool)
                }
                filter::Kind::Merge(m) => {
                    let mut acc: Option<Image> = None;
                    for k in 0..m.inputs().len() {
                        let (next, alpha) = input(k)?;
                        let pass = match &acc {
                            None => self.point(
                                fe.op(OP_IDENTITY, 0, [alpha, false]),
                                [Some(&next), None],
                                None,
                                rect,
                            ),
                            Some(prev) => self.point(
                                fe.op(OP_COMPOSITE, 0, [alpha, false]),
                                [Some(&next), Some(prev)],
                                None,
                                rect,
                            ),
                        };
                        let step = self.pass(device, encoder, pass, s, pool);
                        if let Some(prev) = acc.replace(step) {
                            prev.release(pool);
                        }
                    }
                    acc.unwrap_or_else(Image::empty)
                }
                filter::Kind::Offset(o) => {
                    let (a, aa) = input(0)?;
                    let d = [o.dx(), o.dy()];
                    self.offset(device, encoder, transform, fe, (&a, aa), d, rect, s, pool)
                }
                filter::Kind::GaussianBlur(b) => {
                    let (a, aa) = input(0)?;
                    let sigma = [
                        f64::from(b.std_dev_x().get()),
                        f64::from(b.std_dev_y().get()),
                    ];
                    self.gaussian(
                        device,
                        encoder,
                        transform,
                        fe,
                        (&a, aa),
                        sigma,
                        rect,
                        s,
                        pool,
                    )?
                }
                filter::Kind::DropShadow(d) => {
                    let (a, aa) = input(0)?;
                    self.drop_shadow(device, encoder, transform, fe, (&a, aa), d, rect, s, pool)?
                }
                other => {
                    return Err(format!(
                        "{} is not a float SVG filter primitive",
                        pm_vector::render::svg::primitive_name(other)
                    ));
                }
            };
            results[i] = Some(out);
            for (j, result) in results.iter_mut().enumerate().take(i) {
                if last[j] == i
                    && let Some(done) = result.take()
                {
                    done.release(pool);
                }
            }
        }
        // The last primitive's result; other unread results are released.
        let mut out = None;
        for image in results.into_iter().flatten() {
            if let Some(prev) = out.replace(image) {
                prev.release(pool);
            }
        }
        Ok(out.unwrap_or_else(Image::empty))
    }

    /// `input` shifted by `d` user units, clipped to `s`, over `rect`.
    #[allow(clippy::too_many_arguments)]
    fn offset(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        transform: Affine,
        fe: Fe,
        (input, alpha): (&Image, bool),
        d: [f32; 2],
        rect: PixelRect,
        s: [f32; 4],
        pool: &mut ScratchPool,
    ) -> Image {
        if input.texture.is_none() {
            return Image::empty();
        }
        let c = transform.as_coeffs();
        let (dx, dy) = (f64::from(d[0]), f64::from(d[1]));
        let k = [
            (c[0] * dx + c[2] * dy) as f32,
            (c[1] * dx + c[3] * dy) as f32,
            0.0,
            0.0,
        ];
        let moved = shift(input.region, d);
        let pass = Pass {
            pipeline: &self.offset,
            inputs: [Some(input), None],
            fe: fe.op(0, 0, [alpha, false]).k(k),
            table: None,
            rect,
        };
        // The shader clips by the shifted input's region.
        let fe = pass.fe.images(rect, pass.inputs).region1(moved);
        self.encode(device, encoder, pass, fe, isect(moved, s), pool)
    }

    /// The Gaussian of `std_dev` user units, clipped to `s`, over `rect`.
    #[allow(clippy::too_many_arguments)]
    fn gaussian(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        transform: Affine,
        fe: Fe,
        (input, alpha): (&Image, bool),
        std_dev: [f64; 2],
        rect: PixelRect,
        s: [f32; 4],
        pool: &mut ScratchPool,
    ) -> Result<Image, String> {
        let Some(texture) = &input.texture else {
            return Ok(Image::empty());
        };
        match sigmas(std_dev, transform) {
            Ok([0.0, 0.0]) => {
                let pass = Pass {
                    pipeline: &self.point,
                    inputs: [Some(input), None],
                    fe: fe.op(OP_IDENTITY, 0, [alpha, false]),
                    table: None,
                    rect,
                };
                Ok(self.pass(device, encoder, pass, s, pool))
            }
            Ok(sigma) => {
                let kx = Kernel::new(sigma[0]).unwrap_or_else(Kernel::identity);
                let ky = Kernel::new(sigma[1]).unwrap_or_else(Kernel::identity);
                let mut p = Params::blur_xy(&kx, &ky, input.rect, rect, 0);
                p.mode = i32::from(alpha) | (i32::from(fe.linear()) << 1);
                let weights = gaussian::storage(
                    device,
                    "pm svg filter gaussian",
                    gaussian::f32_bytes(&Params::weights(&kx, &ky)),
                );
                let [h, u, h2, v, ix] = gaussian::sizes(&p, 16)
                    .map(|bytes| gaussian::scratch(device, "pm svg filter gaussian", bytes));
                let uniform_p = p.uniform(device);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                {
                    let mut pass = encoder
                        .begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("pm svg filter gaussian"),
                            timestamp_writes: None,
                        })
                        .forget_lifetime();
                    self.separable.encode(
                        device,
                        &mut pass,
                        &Passes {
                            uniform: &uniform_p,
                            weights: &weights,
                            src: wgpu::BindingResource::TextureView(&view),
                            scratch: [&h, &u, &h2, &v, &ix],
                        },
                        &p,
                    );
                }
                let fe = fe.images(rect, [None, None]);
                let fu = uniform(device, &fe.0);
                let pipeline = self.gauss.get();
                let bg = gaussian::bind(
                    device,
                    &pipeline.get_bind_group_layout(0),
                    &[
                        (0, uniform_p.as_entire_binding()),
                        (8, u.as_entire_binding()),
                        (10, ix.as_entire_binding()),
                        (11, fu.as_entire_binding()),
                    ],
                );
                let out = pool.target(device, rect.width, rect.height);
                render_pass(encoder, &out, true, pipeline, &bg);
                Ok(Image {
                    texture: Some(out),
                    rect,
                    region: s,
                })
            }
            Err(axes) => {
                // The first pass covers what the second reads.
                let second = blur_reach([0.0, std_dev[1]], transform);
                let first_rect = grow(rect, second.map(|v| v.ceil() as u32));
                let mut current: Option<Image> = None;
                for (pass, (axis, sigma)) in axes.iter().enumerate() {
                    let target = if pass == 0 { first_rect } else { rect };
                    let taps =
                        (2.0 * (DIRECT_SUPPORT * sigma).ceil() + 1.0) * area(Some(target)) as f64;
                    if taps.is_nan() || taps > MAX_DIRECT_TAPS as f64 {
                        return Err(format!(
                            "SVG Gaussian along a rotated axis exceeds the direct budget (σ {sigma:.1} px)"
                        ));
                    }
                    let weights = direct_weights(*sigma)?;
                    let fe = fe
                        .op(0, pass as u32, [alpha && pass == 0, false])
                        .k([axis.x as f32, axis.y as f32, 0.0, 0.0])
                        .radius((weights.len() / 2) as u32);
                    let read = current.as_ref().unwrap_or(input);
                    let next = self.pass(
                        device,
                        encoder,
                        Pass {
                            pipeline: &self.blur,
                            inputs: [Some(read), None],
                            fe,
                            table: Some(&weights),
                            rect: target,
                        },
                        s,
                        pool,
                    );
                    if let Some(prev) = current.replace(next) {
                        prev.release(pool);
                    }
                }
                Ok(current.expect("two passes"))
            }
        }
    }

    /// Filter Effects §15.8: the input's alpha blurred, offset, in the
    /// shadow colour, under the input.
    #[allow(clippy::too_many_arguments)]
    fn drop_shadow(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        transform: Affine,
        fe: Fe,
        (input, alpha): (&Image, bool),
        d: &filter::DropShadow,
        rect: PixelRect,
        s: [f32; 4],
        pool: &mut ScratchPool,
    ) -> Result<Image, String> {
        let std_dev = [
            f64::from(d.std_dev_x().get()),
            f64::from(d.std_dev_y().get()),
        ];
        let reach = shift_reach([d.dx(), d.dy()], transform).map(|v| v.ceil() as u32);
        let blurred = self.gaussian(
            device,
            encoder,
            transform,
            fe,
            (input, true),
            std_dev,
            grow(rect, reach),
            s,
            pool,
        )?;
        let moved = self.offset(
            device,
            encoder,
            transform,
            fe,
            (&blurred, false),
            [d.dx(), d.dy()],
            rect,
            s,
            pool,
        );
        blurred.release(pool);
        let c = d.color();
        let a = d.opacity().get();
        let tint = [
            f32::from(c.red) / 255.0 * a,
            f32::from(c.green) / 255.0 * a,
            f32::from(c.blue) / 255.0 * a,
            a,
        ];
        let tinted = self.point(
            fe.op(OP_TINT, 0, [false; 2]).k(tint),
            [Some(&moved), None],
            None,
            rect,
        );
        let shadow = self.pass(device, encoder, tinted, s, pool);
        moved.release(pool);
        let over = self.point(
            fe.op(OP_COMPOSITE, 0, [alpha, false]),
            [Some(input), Some(&shadow)],
            None,
            rect,
        );
        let out = self.pass(device, encoder, over, s, pool);
        shadow.release(pool);
        Ok(out)
    }
}
