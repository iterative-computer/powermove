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
//! Each outline's coverage within its pixel bounds is [`crate::coverage`]'s
//! exact area; the feather is [`super::gaussian`]. The work is bounded by
//! each mask's blurred bounds; the canvas is f32 and is stored as
//! `r16float`.

use kurbo::{Affine, BezPath};
use pm_vector::render::scene::PixelRect;
use pm_vector::{MaskMode, PathMasks};

use super::gaussian::{
    Kernel, Params, Passes, Separable, bind, compute, f32_bytes, groups, inflate, rect4, render,
    scratch, sizes, storage,
};
use crate::coverage::{AreaCoverage, Batch, Item, bounds};
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

/// One mask of a [`PathCoverage`], prepared.
struct Prepared {
    /// The outline in output pixels and its pixel bounds there.
    path: BezPath,
    bounds: PixelRect,
    kernel: Option<Kernel>,
    /// Where the feathered mask can be nonzero.
    out: PixelRect,
    mode: MaskMode,
}

pub struct PathMaskPipelines {
    area: AreaCoverage,
    separable: Separable,
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
                    "{}\n{}\n{}",
                    crate::nearest_half::NEAREST_HALF_WGSL,
                    include_str!("path_masks.wgsl"),
                    include_str!("gaussian.wgsl")
                )
                .into(),
            ),
        });
        PathMaskPipelines {
            area: AreaCoverage::new(device),
            separable: Separable::new(device, &module),
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
        all.extend(self.area.pipelines());
        all.extend([
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
                let path = m.transform_from(cov.world).affine() * &m.path;
                let bounds = bounds(&path, w, h)?;
                let kernel = Kernel::new(m.blur_sigma(world));
                let reach = kernel.as_ref().map_or(0, |k| k.extent);
                Some(Prepared {
                    out: inflate(bounds, reach).intersect(target)?,
                    path,
                    bounds,
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
        let mut weights = Vec::new();
        // The canvas, the coverage, and the blur's scratch.
        let area = |r: PixelRect| u64::from(r.width) * u64::from(r.height) * 4;
        let mut need = [if single { 0 } else { area(reach) }, 0, 0, 0, 0, 0, 0];
        let params: Vec<Params> = masks
            .iter()
            .map(|m| {
                let b = m.bounds;
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
                need[1] = need[1].max(area(b));
                p
            })
            .collect();
        let [acc, accum, h, u, h2, v, ix] = self
            .scratch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .buffers(device, need);
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
            let item = Item {
                path: &m.path,
                bounds: m.bounds,
                at: [0, 0],
            };
            self.area
                .encode(device, encoder, &Batch::new(&[item]), &accum, None);
            let mut pass = begin(encoder);
            let uniform = p.uniform(device);
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
