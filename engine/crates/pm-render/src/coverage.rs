//! Exact area coverage of filled outlines on the GPU (`coverage.wgsl`):
//! path masks and vector shapes.
//!
//! Each item is a path in device pixels filled nonzero over a pixel
//! rectangle. On the CPU it is flattened to [`FLATTEN`] px, clipped to the
//! rectangle's rows and clamped to its columns (left of it, an edge still
//! covers every pixel to its right), fixed to 1/16384 px and split into
//! edges of at most 64 px. On the GPU each edge adds its exact signed area
//! per pixel into integer atomics, its shares rounded so that a closed row
//! sums to exactly zero, and a scan per row turns the running sum into
//! `min(1, |area|)`: exact for contours that do not overlap (to the
//! flattening, ≈ 10⁻⁴, and the fixed point, ≈ 3·10⁻⁵); where contours
//! overlap inside one pixel their winding areas add before the clamp.
//! Many items go in one batch, each into its own rows of an f32 buffer
//! and, optionally, its cell of an `r32float` page.

use kurbo::{BezPath, PathEl, Point};
use pm_vector::render::scene::PixelRect;

use crate::composite::gaussian::{bind, compute};
use crate::pipelines::LazyComputePipeline;

/// Flattening tolerance, px: a chord moves a pixel's coverage by at most
/// this much.
pub const FLATTEN: f64 = 1e-4;
/// Fixed-point units per pixel (`UNIT` in the shader).
const UNITS: f64 = 16384.0;
/// The longest edge, px, one invocation walks (keeps its arithmetic exact
/// in f32 and the work spread).
const PIECE: f64 = 64.0;

/// One outline to cover.
#[derive(Debug, Clone, Copy)]
pub struct Item<'a> {
    /// Device pixels, filled nonzero; open contours close.
    pub path: &'a BezPath,
    /// The device pixels covered.
    pub bounds: PixelRect,
    /// Where `bounds`' top-left pixel lands in the page.
    pub at: [u32; 2],
}

/// Items on the CPU, ready for [`AreaCoverage::encode`].
#[derive(Debug, Clone, Default)]
pub struct Batch {
    /// `Item` in the shader.
    items: Vec<[u32; 8]>,
    edges: Vec<[i32; 4]>,
    edge_items: Vec<u32>,
    /// (item, row) per scan workgroup.
    rows: Vec<[u32; 2]>,
    /// Values in the f32 buffer.
    len: u64,
}

impl Batch {
    pub fn new(items: &[Item<'_>]) -> Batch {
        let mut b = Batch::default();
        for (i, item) in items.iter().enumerate() {
            let r = item.bounds;
            b.items.push([b.len as u32, r.width, r.height, item.at[0], item.at[1], 0, 0, 0]);
            edges(&flatten(item.path), r, &mut b.edges);
            b.edge_items.resize(b.edges.len(), i as u32);
            b.rows.extend((0..r.height).map(|y| [i as u32, y]));
            b.len += u64::from(r.width) * u64::from(r.height);
        }
        b
    }

    /// Where item `i`'s rows start in the f32 buffer.
    pub fn offset(&self, i: usize) -> u64 {
        u64::from(self.items[i][0])
    }

    /// Bytes of the f32 buffer.
    pub fn bytes(&self) -> u64 {
        self.len * 4
    }

    /// Edges of item `i` (tests).
    #[cfg(test)]
    fn edges_of(&self, i: usize) -> impl Iterator<Item = &[i32; 4]> {
        self.edges.iter().zip(&self.edge_items).filter(move |(_, k)| **k as usize == i).map(|(e, _)| e)
    }
}

/// The pixel bounds of `path` within `width × height`, or `None` when it
/// covers nothing there.
pub fn bounds(path: &BezPath, width: u32, height: u32) -> Option<PixelRect> {
    use kurbo::Shape;
    let r = path.bounding_box();
    let clamp = |v: f64, n: u32| if v.is_nan() { 0.0 } else { v.clamp(0.0, f64::from(n)) };
    let (x0, x1) = (clamp(r.x0.floor(), width), clamp(r.x1.ceil(), width));
    let (y0, y1) = (clamp(r.y0.floor(), height), clamp(r.y1.ceil(), height));
    (x1 > x0 && y1 > y0)
        .then(|| PixelRect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// `path` as line segments, every subpath closed, non-finite segments
/// dropped.
fn flatten(path: &BezPath) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let close = |out: &mut Vec<(Point, Point)>, last: Point, start: Point| {
        if last != start {
            out.push((last, start));
        }
    };
    kurbo::flatten(path.iter(), FLATTEN, |el| match el {
        PathEl::MoveTo(p) => {
            close(&mut out, last, start);
            (start, last) = (p, p);
        }
        PathEl::LineTo(p) => {
            out.push((last, p));
            last = p;
        }
        PathEl::ClosePath => {
            close(&mut out, last, start);
            last = start;
        }
        _ => {}
    });
    close(&mut out, last, start);
    out.retain(|(a, b)| a.is_finite() && b.is_finite());
    out
}

/// `lines` clipped to `r`'s rows, clamped to its columns, fixed point from
/// its origin, at most [`PIECE`] px each, appended to `out`.
fn edges(lines: &[(Point, Point)], r: PixelRect, out: &mut Vec<[i32; 4]>) {
    let (x0, x1) = (f64::from(r.x), f64::from(r.x) + f64::from(r.width));
    let (y0, y1) = (f64::from(r.y), f64::from(r.y) + f64::from(r.height));
    let fixed = |p: Point| -> [i32; 2] {
        [
            ((p.x.clamp(x0, x1) - x0) * UNITS).round() as i32,
            ((p.y.clamp(y0, y1) - y0) * UNITS).round() as i32,
        ]
    };
    let mut ts = Vec::with_capacity(4);
    for &(a, b) in lines {
        if a.y == b.y {
            continue;
        }
        // The part within the rows; rows outside never matter.
        let at = |y: f64| (y - a.y) / (b.y - a.y);
        let (t0, t1) = (at(y0), at(y1));
        let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
        if t0 >= t1 {
            continue;
        }
        // Split where it leaves the columns; beyond them it is clamped to
        // the side.
        ts.clear();
        ts.extend([t0, t1]);
        if a.x != b.x {
            for x in [x0, x1] {
                let t = (x - a.x) / (b.x - a.x);
                if t > t0 && t < t1 {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        for pair in ts.windows(2) {
            let (p, q) = (a.lerp(b, pair[0]), a.lerp(b, pair[1]));
            let n = ((q - p).x.abs().max((q - p).y.abs()) / PIECE).ceil().max(1.0);
            let mut prev = fixed(p);
            for i in 1..=n as usize {
                let next = fixed(p.lerp(q, i as f64 / n));
                if next[1] != prev[1] {
                    out.push([prev[0], prev[1], next[0], next[1]]);
                }
                prev = next;
            }
        }
    }
}

/// `v` as bytes: arrays of 32-bit words (no padding).
fn words<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: callers pass u32/i32 values or arrays of them, which have no
    // padding, and every byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), size_of_val(v)) }
}

/// Counts and four read-only arrays share one upload and tracked GPU resource.
/// Each binding starts at the device's required storage offset alignment.
fn input_buffer(device: &wgpu::Device, batch: &Batch) -> (wgpu::Buffer, [(u64, u64); 5]) {
    let counts = [[batch.edges.len() as u32, batch.rows.len() as u32, 0, 0]];
    let inputs = [
        words(&counts),
        words(&batch.items),
        words(&batch.edges),
        words(&batch.edge_items),
        words(&batch.rows),
    ];
    let align = u64::from(device.limits().min_storage_buffer_offset_alignment.max(4));
    let mut end = 0u64;
    let mut ranges = [(0, 0); 5];
    for (i, bytes) in inputs.iter().enumerate() {
        let start = end.div_ceil(align) * align;
        let size = bytes.len().max(16) as u64;
        ranges[i] = (start, size);
        end = start + size;
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm coverage inputs"),
        size: end.div_ceil(4) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: true,
    });
    {
        let mut mapped = buffer.slice(..).get_mapped_range_mut().expect("buffer mapped at creation");
        for (bytes, &(start, _)) in inputs.iter().zip(&ranges) {
            mapped.slice(start as usize..start as usize + bytes.len()).copy_from_slice(bytes);
        }
    }
    buffer.unmap();
    (buffer, ranges)
}

/// The coverage pipelines.
pub struct AreaCoverage {
    raster: LazyComputePipeline,
    scan: LazyComputePipeline,
    scan_page: LazyComputePipeline,
}

/// A page `width × height`: `r32float`, written by [`AreaCoverage::encode`]
/// and read with `textureLoad`.
pub fn page_descriptor(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("pm coverage page"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }
}

/// An f32 buffer for `batch` ([`Batch::bytes`]).
pub fn buffer(device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm coverage"),
        size: bytes.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

impl AreaCoverage {
    pub fn new(device: &wgpu::Device) -> AreaCoverage {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render coverage"),
            source: wgpu::ShaderSource::Wgsl(include_str!("coverage.wgsl").into()),
        });
        AreaCoverage {
            raster: compute(device, &module, "raster"),
            scan: compute(device, &module, "scan"),
            scan_page: compute(device, &module, "scan_page"),
        }
    }

    /// Records the coverage of every item of `batch` into `encoder` (no
    /// submit): f32 per pixel in `values` (item `i`'s rows from
    /// [`Batch::offset`], at least [`Batch::bytes`]), and into each item's
    /// cell of `page` ([`page_descriptor`]) when given. Pixels of `page`
    /// outside the cells are left as they are.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        batch: &Batch,
        values: &wgpu::Buffer,
        page: Option<&wgpu::TextureView>,
    ) {
        if batch.len == 0 {
            return;
        }
        encoder.clear_buffer(values, 0, Some(batch.bytes()));
        let (inputs, ranges) = input_buffer(device, batch);
        let read = |index: usize| {
            let (offset, size) = ranges[index];
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &inputs,
                offset,
                size: std::num::NonZeroU64::new(size),
            })
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm coverage"),
            timestamp_writes: None,
        });
        let raster = self.raster.get();
        let group = bind(
            device,
            &raster.get_bind_group_layout(0),
            &[
                (0, read(0)),
                (1, read(1)),
                (2, read(2)),
                (3, read(3)),
                (5, values.as_entire_binding()),
            ],
        );
        pass.set_pipeline(raster);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((batch.edges.len() as u32).div_ceil(64), 1, 1);
        let mut entries = vec![
            (0, read(0)),
            (1, read(1)),
            (4, read(4)),
            (6, values.as_entire_binding()),
        ];
        let scan = match page {
            Some(view) => {
                entries.push((7, wgpu::BindingResource::TextureView(view)));
                self.scan_page.get()
            }
            None => self.scan.get(),
        };
        let group = bind(device, &scan.get_bind_group_layout(0), &entries);
        pass.set_pipeline(scan);
        pass.set_bind_group(0, &group, &[]);
        let n = batch.rows.len() as u32;
        pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
    }

    #[cfg(test)]
    pub(crate) fn pipelines(&self) -> [(&'static str, &LazyComputePipeline); 3] {
        [("raster", &self.raster), ("scan", &self.scan), ("scan page", &self.scan_page)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Affine, Circle, Rect, Shape};
    use pm_reference::coverage::{FillRule, fill};

    fn read(gpu: &pm_gpu::Gpu, texture: &wgpu::Texture) -> Vec<f32> {
        pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, texture).unwrap()
    }

    /// A page of `w × h` with `items` encoded into it.
    fn page(gpu: &pm_gpu::Gpu, items: &[Item<'_>], w: u32, h: u32) -> Vec<f32> {
        let (device, queue) = (&gpu.device, &gpu.queue);
        let coverage = AreaCoverage::new(device);
        let texture = device.create_texture(&page_descriptor(w, h));
        let view = texture.create_view(&Default::default());
        let batch = Batch::new(items);
        let values = buffer(device, batch.bytes());
        let mut encoder = device.create_command_encoder(&Default::default());
        coverage.encode(device, &mut encoder, &batch, &values, Some(&view));
        queue.submit([encoder.finish()]);
        read(gpu, &texture)
    }

    fn shapes() -> Vec<BezPath> {
        vec![
            Circle::new((20.37, 17.81), 13.2).to_path(1e-9),
            Affine::translate((24.6, 21.3)) * Affine::rotate(0.61) * Rect::new(-15.0, -6.5, 15.0, 6.5).to_path(1e-9),
            // Beyond the bounds on every side.
            Rect::new(-3.25, -2.5, 52.75, 45.5).to_path(1e-9),
        ]
    }

    #[test]
    fn one_item_is_the_exact_area() {
        let gpu = pm_gpu::Gpu::new("coverage test").unwrap();
        let (w, h) = (48u32, 40u32);
        for path in shapes() {
            let r = PixelRect::new(0, 0, w, h);
            let ours = page(&gpu, &[Item { path: &path, bounds: r, at: [0, 0] }], w, h);
            let exact = fill(&path, FillRule::NonZero, 16, w as usize, h as usize);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let (a, b) = (f64::from(ours[y * w as usize + x]), exact.at(x, y));
                    // Flattening, fixed point and rounded shares.
                    assert!((a - b).abs() < 2.5e-4, "({x}, {y}): {a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn a_batch_keeps_each_item_in_its_cell() {
        let gpu = pm_gpu::Gpu::new("coverage test").unwrap();
        let paths = shapes();
        // Overlapping device bounds, one cell each across the page.
        let bounds = [
            PixelRect::new(4, 2, 34, 31),
            PixelRect::new(6, 5, 38, 33),
            PixelRect::new(0, 0, 48, 40),
        ];
        let items: Vec<Item<'_>> = paths
            .iter()
            .zip(bounds)
            .enumerate()
            .map(|(i, (path, bounds))| Item { path, bounds, at: [i as u32 * 50 + 1, 3] })
            .collect();
        let (w, h) = (150u32, 46u32);
        let ours = page(&gpu, &items, w, h);
        for item in &items {
            let exact = fill(item.path, FillRule::NonZero, 16, 64, 64);
            let b = item.bounds;
            for y in 0..b.height as usize {
                for x in 0..b.width as usize {
                    let v = f64::from(ours[(item.at[1] as usize + y) * w as usize + item.at[0] as usize + x]);
                    let e = exact.at(b.x as usize + x, b.y as usize + y);
                    assert!((v - e).abs() < 2.5e-4, "{b:?} ({x}, {y}): {v} vs {e}");
                }
            }
        }
        // Nothing outside the cells.
        let inside = |x: u32, y: u32| {
            items.iter().any(|i| {
                x >= i.at[0] && x < i.at[0] + i.bounds.width && y >= i.at[1] && y < i.at[1] + i.bounds.height
            })
        };
        for y in 0..h {
            for x in 0..w {
                if !inside(x, y) {
                    assert_eq!(ours[(y * w + x) as usize], 0.0);
                }
            }
        }
    }

    #[test]
    fn edges_clip_to_the_bounds_and_keep_their_area() {
        // Trapezoids against x = 0 (dropped horizontal edges add none).
        let area = |b: &Batch| -> f64 {
            b.edges_of(0).map(|e| f64::from(e[0] + e[2]) / 2.0 * f64::from(e[3] - e[1])).sum::<f64>()
                / (UNITS * UNITS)
        };
        let r = Rect::new(10.25, 4.5, 30.75, 20.0).to_path(1e-9);
        let b = bounds(&r, 64, 64).unwrap();
        assert_eq!(b, PixelRect::new(10, 4, 21, 16));
        let batch = Batch::new(&[Item { path: &r, bounds: b, at: [0, 0] }]);
        assert!((area(&batch).abs() - 20.5 * 15.5).abs() < 1e-6);
        // Half off the left edge: clamped to x = 0, the area within stays.
        let r = Rect::new(-10.0, 0.0, 10.0, 8.0).to_path(1e-9);
        let b = bounds(&r, 64, 64).unwrap();
        assert_eq!(b, PixelRect::new(0, 0, 10, 8));
        assert!((area(&Batch::new(&[Item { path: &r, bounds: b, at: [0, 0] }])).abs() - 80.0).abs() < 1e-6);
        assert!(bounds(&Rect::new(-10.0, 0.0, -1.0, 8.0).to_path(1e-9), 64, 64).is_none());
        // Long edges are split for the GPU.
        let r = Rect::new(0.0, 0.0, 300.0, 300.0).to_path(1e-9);
        let b = bounds(&r, 512, 512).unwrap();
        let batch = Batch::new(&[Item { path: &r, bounds: b, at: [0, 0] }]);
        assert!(batch.edges.iter().all(|e| (e[3] - e[1]).abs() <= 64 * 16384));
    }
}
