//! Bezier path masks on the GPU (docs/engine/composite.md "Path masks").
//!
//! When any enabled mask of a layer has a path, every enabled mask
//! (parametric ones too, with no count limit) is a filled outline (nonzero)
//! in output pixels, feathered by the exact Gaussian of
//! σ = max(0, feather) · |world x| / 2 px (transparent outside the target),
//! and drawn in order onto one coverage canvas, `source-over` (add) or
//! `destination-out` (subtract); the canvas starts covered when no mask
//! adds. The layer is multiplied by the canvas, sampled at each output
//! pixel (through the inverse plane for a 3D layer).
//!
//! Each outline is flattened on the CPU to [`FLATTEN`] px, clipped to its
//! pixel bounds within the target and fixed to 1/16384 px; the GPU
//! accumulates each edge's exact signed area per pixel and a row scan turns
//! it into coverage (exact for outlines that do not overlap themselves;
//! where they do, a pixel's winding areas add before the clamp to 1). The
//! work is bounded by each mask's blurred bounds; the canvas is f32 and is
//! stored as `r16float`.

use kurbo::{Affine, BezPath, PathEl, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::{MaskMode, PathMasks};

use super::gaussian::{
    Kernel, Params, Passes, Separable, bind, compute, f32_bytes, groups, inflate, rect4, render,
    scratch, sizes, storage,
};
use crate::pipelines::{LazyComputePipeline, LazyRenderPipeline};

/// A layer's masks on the path route, ready for the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct PathCoverage {
    /// The coverage canvas: the output size (`W × H`).
    pub width: u32,
    pub height: u32,
    /// The layer's `scaledWorld`.
    pub world: [f64; 6],
    pub masks: PathMasks,
}

/// The coverage canvas format.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Flattening tolerance, px: a chord moves a pixel's coverage by at most
/// this much.
pub const FLATTEN: f64 = 1e-4;
/// Fixed-point outline units per pixel (`UNIT` in the shader).
const UNITS: f64 = 16384.0;
/// The longest edge, px, one invocation walks (keeps its arithmetic exact
/// in f32 and the work spread).
const PIECE: f64 = 64.0;

/// One mask's outline on the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// Pixel bounds within the target.
    pub bounds: PixelRect,
    /// Edges `[x0, y0, x1, y1]` in 1/16384 px from the bounds' origin.
    pub edges: Vec<[i32; 4]>,
}

/// The device outline of `path` (pixels) clipped to `width × height`, or
/// `None` when it covers nothing there.
pub fn outline(path: &BezPath, width: u32, height: u32) -> Option<Outline> {
    let lines = flatten(path);
    let (mut lo, mut hi) = (Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN));
    for (a, b) in &lines {
        lo = Point::new(lo.x.min(a.x).min(b.x), lo.y.min(a.y).min(b.y));
        hi = Point::new(hi.x.max(a.x).max(b.x), hi.y.max(a.y).max(b.y));
    }
    let clamp = |v: f64, n: u32| v.clamp(0.0, f64::from(n));
    let (x0, x1) = (clamp(lo.x.floor(), width), clamp(hi.x.ceil(), width));
    let (y0, y1) = (clamp(lo.y.floor(), height), clamp(hi.y.ceil(), height));
    if lines.is_empty() || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut edges = Vec::with_capacity(lines.len());
    let fixed = |p: Point| -> [i32; 2] {
        [
            ((p.x.clamp(x0, x1) - x0) * UNITS).round() as i32,
            ((p.y.clamp(y0, y1) - y0) * UNITS).round() as i32,
        ]
    };
    let mut ts = Vec::with_capacity(4);
    for (a, b) in lines {
        if a.y == b.y {
            continue;
        }
        // The part within the bounds' rows; rows outside never matter.
        let at = |y: f64| (y - a.y) / (b.y - a.y);
        let (t0, t1) = (at(y0), at(y1));
        let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
        if t0 >= t1 {
            continue;
        }
        // Split where it leaves the columns: beyond them it is clamped to
        // the side (left of the target it still covers every pixel to its
        // right).
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
                    edges.push([prev[0], prev[1], next[0], next[1]]);
                }
                prev = next;
            }
        }
    }
    Some(Outline {
        bounds: PixelRect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32),
        edges,
    })
}

/// `path` as line segments, every subpath closed (a fill closes open
/// subpaths), non-finite segments dropped.
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

/// One mask of a [`PathCoverage`], prepared.
struct Prepared {
    outline: Outline,
    kernel: Option<Kernel>,
    /// Where the feathered mask can be nonzero.
    out: PixelRect,
    mode: MaskMode,
}

pub struct PathMaskPipelines {
    separable: Separable,
    raster: LazyComputePipeline,
    scan: LazyComputePipeline,
    fill: LazyComputePipeline,
    composite: LazyComputePipeline,
    store: LazyRenderPipeline,
    single: LazyRenderPipeline,
    scratch: std::sync::Mutex<Scratch>,
}

impl PathMaskPipelines {
    pub fn new(device: &wgpu::Device) -> PathMaskPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render path masks"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("path_masks.wgsl"),
                    include_str!("gaussian.wgsl")
                )
                .into(),
            ),
        });
        PathMaskPipelines {
            separable: Separable::new(device, &module),
            raster: compute(device, &module, "raster"),
            scan: compute(device, &module, "scan"),
            fill: compute(device, &module, "fill"),
            composite: compute(device, &module, "composite"),
            store: render(device, &module, "fs_store", FORMAT),
            single: render(device, &module, "fs_single", FORMAT),
            scratch: Default::default(),
        }
    }

    #[cfg(test)]
    fn pipelines(&self) -> Vec<(&'static str, &LazyComputePipeline)> {
        let mut all = self.separable.pipelines().to_vec();
        all.extend([
            ("raster", &self.raster),
            ("scan", &self.scan),
            ("fill", &self.fill),
            ("composite", &self.composite),
        ]);
        all
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        self.pipelines().iter().filter(|(_, p)| p.is_built()).count()
            + usize::from(self.store.is_built())
            + usize::from(self.single.is_built())
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        let mut times: Vec<(String, std::time::Duration)> = self
            .pipelines()
            .into_iter()
            .map(|(name, p)| {
                let start = std::time::Instant::now();
                p.get();
                (format!("path mask {name}"), start.elapsed())
            })
            .collect();
        for (name, p) in [("store", &self.store), ("single", &self.single)] {
            let start = std::time::Instant::now();
            p.get();
            times.push((format!("path mask {name}"), start.elapsed()));
        }
        times
    }

    /// The coverage canvas of `cov`: an `r16float` texture of the output
    /// size, top-down rows.
    pub fn coverage(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        cov: &PathCoverage,
    ) -> wgpu::Texture {
        let (w, h) = (cov.width.max(1), cov.height.max(1));
        let target = PixelRect::new(0, 0, w, h);
        let world = Affine::new(cov.world);
        let masks: Vec<Prepared> = cov
            .masks
            .masks
            .iter()
            .filter_map(|m| {
                let outline = outline(&(m.transform_from(cov.world).affine() * &m.path), w, h)?;
                let kernel = Kernel::new(m.blur_sigma(world));
                let reach = kernel.as_ref().map_or(0, |k| k.extent);
                Some(Prepared {
                    out: inflate(outline.bounds, reach).intersect(target)?,
                    outline,
                    kernel,
                    mode: m.mode,
                })
            })
            .collect();
        let start = f64::from(u8::from(cov.masks.start_covered));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm path mask coverage"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let Some(reach) = masks.iter().map(|m| m.out).reduce(PixelRect::union) else {
            drop(clear(encoder, &view, start));
            return texture;
        };
        // A layer's only mask is drawn straight onto the start; more go
        // through the f32 canvas over `reach`.
        let single = masks.len() == 1;
        let (mut edges, mut weights) = (Vec::new(), Vec::new());
        // The canvas, the coverage, and the blur's scratch.
        let area = |r: PixelRect| u64::from(r.width) * u64::from(r.height) * 4;
        let mut need = [if single { 0 } else { area(reach) }, 0, 0, 0, 0, 0, 0];
        let params: Vec<Params> = masks
            .iter()
            .map(|m| {
                let b = m.outline.bounds;
                let mut p = match &m.kernel {
                    Some(k) => {
                        let p = Params::blur(k, b, m.out, weights.len() as i32);
                        weights.extend_from_slice(&k.weights);
                        weights.extend_from_slice(&k.coarse);
                        for (n, size) in need[2..].iter_mut().zip(sizes(&p, 4)) {
                            *n = (*n).max(size);
                        }
                        p
                    }
                    // The coverage itself is the grid.
                    None => Params {
                        src: rect4(b),
                        grid: rect4(b),
                        out: rect4(b),
                        step: 1,
                        ..Default::default()
                    },
                };
                p.dst = rect4(reach);
                p.mode = i32::from(m.mode == MaskMode::Subtract);
                p.first = edges.len() as i32;
                p.count = m.outline.edges.len() as i32;
                edges.extend_from_slice(&m.outline.edges);
                need[1] = need[1].max(area(b));
                p
            })
            .collect();
        let [acc, accum, h, u, h2, v, ix] = self
            .scratch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .buffers(device, need);
        // SAFETY: [i32; 4] has no padding.
        let edge_bytes = unsafe {
            std::slice::from_raw_parts(edges.as_ptr().cast::<u8>(), size_of_val(&edges[..]))
        };
        let edges = storage(device, "pm path mask edges", edge_bytes);
        let weights = storage(device, "pm path mask weights", f32_bytes(&weights));
        let canvas = Params {
            dst: rect4(reach),
            mode: i32::from(cov.masks.start_covered),
            ..Default::default()
        }
        .uniform(device);
        let run = |pass: &mut wgpu::ComputePass<'_>,
                       pipeline: &LazyComputePipeline,
                       entries: &[(u32, wgpu::BindingResource<'_>)],
                       size: [u32; 3]| {
            let pipeline = pipeline.get();
            let group = bind(device, &pipeline.get_bind_group_layout(0), entries);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(size[0], size[1], size[2]);
        };
        let begin = |encoder: &mut wgpu::CommandEncoder| {
            encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("pm path mask"),
                    timestamp_writes: None,
                })
                .forget_lifetime()
        };
        if !single {
            run(
                &mut begin(encoder),
                &self.fill,
                &[(0, canvas.as_entire_binding()), (7, acc.as_entire_binding())],
                [groups(reach.width as i32, 16), groups(reach.height as i32, 16), 1],
            );
        }
        let mut last = None;
        for (m, p) in masks.iter().zip(&params) {
            let b = m.outline.bounds;
            // The accumulator starts at zero.
            encoder.clear_buffer(&accum, 0, Some(area(b)));
            let mut pass = begin(encoder);
            let uniform = p.uniform(device);
            run(
                &mut pass,
                &self.raster,
                &[
                    (0, uniform.as_entire_binding()),
                    (5, accum.as_entire_binding()),
                    (6, edges.as_entire_binding()),
                ],
                [groups(p.count, 64), 1, 1],
            );
            run(
                &mut pass,
                &self.scan,
                &[(0, uniform.as_entire_binding()), (11, accum.as_entire_binding())],
                [b.height, 1, 1],
            );
            // `grid_at` reads stage 1's nodes and the interpolated rows; the
            // coverage itself when nothing is feathered.
            let grid = if m.kernel.is_some() {
                let b = Passes {
                    uniform: &uniform,
                    weights: &weights,
                    src: accum.as_entire_binding(),
                    scratch: [&h, &u, &h2, &v, &ix],
                };
                self.separable.encode(device, &mut pass, &b, p);
                [u.clone(), ix.clone()]
            } else {
                [accum.clone(), accum.clone()]
            };
            if single {
                last = Some((uniform, grid));
                continue;
            }
            run(
                &mut pass,
                &self.composite,
                &[
                    (0, uniform.as_entire_binding()),
                    (7, acc.as_entire_binding()),
                    (8, grid[0].as_entire_binding()),
                    (10, grid[1].as_entire_binding()),
                ],
                [groups(p.out[2], 16), groups(p.out[3], 16), 1],
            );
        }
        let mut pass = clear(encoder, &view, start);
        let (pipeline, entries) = match &last {
            Some((uniform, grid)) => (
                self.single.get(),
                [
                    (0, uniform.as_entire_binding()),
                    (8, grid[0].as_entire_binding()),
                    (10, grid[1].as_entire_binding()),
                ]
                .to_vec(),
            ),
            None => (
                self.store.get(),
                vec![(0, canvas.as_entire_binding()), (7, acc.as_entire_binding())],
            ),
        };
        let group = bind(device, &pipeline.get_bind_group_layout(0), &entries);
        pass.set_scissor_rect(reach.x as u32, reach.y as u32, reach.width, reach.height);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        texture
    }
}

/// The store pass onto `view`: cleared to `start`, which the canvas keeps
/// outside every mask's reach.
fn clear<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    start: f64,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm path mask store"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: start,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Scratch buffers kept across frames (a fresh buffer is zero-filled
/// before first use): the f32 canvas, the coverage accumulator, and the
/// Gaussian's five. They grow to the largest use; the GPU orders
/// every use (Metal tracks the hazards between command buffers).
#[derive(Default)]
struct Scratch {
    buffers: [Option<wgpu::Buffer>; 7],
}

impl Scratch {
    fn buffers(&mut self, device: &wgpu::Device, need: [u64; 7]) -> [wgpu::Buffer; 7] {
        std::array::from_fn(|i| {
            let slot = &mut self.buffers[i];
            match slot {
                Some(b) if b.size() >= need[i] => b.clone(),
                _ => {
                    // Whole MiB, so small growth does not reallocate.
                    let b = scratch(device, "pm path mask scratch", need[i].next_multiple_of(1 << 20));
                    *slot = Some(b.clone());
                    b
                }
            }
        })
    }
}

/// The coverage canvas of `cov` read back (top-down, one value per pixel):
/// for gates and diagnostics.
pub fn coverage_values(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cov: &PathCoverage,
) -> Result<Vec<f32>, String> {
    let pipelines = PathMaskPipelines::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let texture = pipelines.coverage(device, &mut encoder, cov);
    queue.submit([encoder.finish()]);
    let raw = pm_gpu::readback::read_raw(device, queue, &texture).map_err(|e| format!("{e:?}"))?;
    Ok(raw
        .chunks_exact(2)
        .map(|c| pm_gpu::readback::half_to_f32(u16::from_le_bytes([c[0], c[1]])))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn area(o: &Outline) -> f64 {
        // Trapezoids against x = 0 (dropped horizontal edges add none).
        o.edges
            .iter()
            .map(|e| f64::from(e[0] + e[2]) / 2.0 * f64::from(e[3] - e[1]))
            .sum::<f64>()
            / (UNITS * UNITS)
    }

    #[test]
    fn outlines_clip_to_the_target_and_keep_their_area() {
        let r = kurbo::Rect::new(10.25, 4.5, 30.75, 20.0).to_path(1e-9);
        let o = outline(&r, 64, 64).unwrap();
        assert_eq!(o.bounds, PixelRect::new(10, 4, 21, 16));
        assert!((area(&o).abs() - 20.5 * 15.5).abs() < 1e-6);
        // Half off the left edge: clamped to x = 0, the area within stays.
        let r = kurbo::Rect::new(-10.0, 0.0, 10.0, 8.0).to_path(1e-9);
        let o = outline(&r, 64, 64).unwrap();
        assert_eq!(o.bounds, PixelRect::new(0, 0, 10, 8));
        assert!((area(&o).abs() - 80.0).abs() < 1e-6);
        // Nothing inside.
        let r = kurbo::Rect::new(-10.0, 0.0, -1.0, 8.0).to_path(1e-9);
        assert!(outline(&r, 64, 64).is_none());
        // Long edges are split for the GPU.
        let r = kurbo::Rect::new(0.0, 0.0, 300.0, 300.0).to_path(1e-9);
        let o = outline(&r, 512, 512).unwrap();
        assert!(o.edges.iter().all(|e| (e[3] - e[1]).abs() <= 64 * 16384));
    }
}
