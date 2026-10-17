//! Draws a [`ShaderSource`]: the compositor's `grab(targetW, targetH)`,
//! `clear(0, 0, 0, 0)`, blending off, the quad through `PM.VERT`, and the
//! uniform calls in issue order, in GL row order.

use std::collections::HashMap;
use std::sync::Arc;

use pm_glsl::{Failure, Key};
use pm_gpu::Target;
use pm_protocol::definitions::DefinitionKind;

use super::fallback::{FallbackPixels, FallbackRequest, FallbackStore};
use super::plan::{ProgramRef, ShaderSource};
use crate::effects::program::{Call, GpuProgram};

/// GPU programs of shader-layer code kept by a renderer (by source key),
/// and the fallback runner's state.
#[derive(Default)]
pub struct ShaderState {
    programs: HashMap<Key, (GpuProgram, u64)>,
    tick: u64,
    /// Whether a fallback runner draws programs the native pipeline cannot
    /// (the interactive engine with a viewer); see [`super::fallback`].
    pub fallback_runner: bool,
    /// Pixels the runner uploaded.
    pub fallback: FallbackStore,
}

/// Programs kept before the least recently used is dropped (today's
/// `shaderUniformDefs` bound).
const PROGRAMS: usize = 256;

impl ShaderState {
    /// Programs built.
    pub fn programs(&self) -> usize {
        self.programs.len()
    }

    fn program(
        &mut self,
        device: &wgpu::Device,
        key: Key,
        prepared: &super::translate::Translated,
        format: wgpu::TextureFormat,
    ) -> Result<&mut GpuProgram, Failure> {
        self.tick += 1;
        let tick = self.tick;
        if !self.programs.contains_key(&key) {
            let prepared = prepared.as_ref().as_ref().map_err(|f| f.clone())?;
            let program = prepared
                .finish()
                .and_then(|p| GpuProgram::new(device, p, "pm shader layer"))?;
            if self.programs.len() >= PROGRAMS {
                let oldest = self
                    .programs
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| *k)
                    .expect("not empty");
                self.programs.remove(&oldest);
            }
            self.programs.insert(key, (program, tick));
        }
        let entry = self.programs.get_mut(&key).expect("inserted above");
        entry.1 = tick;
        if let Err(f) = entry.0.pipeline(device, format) {
            self.programs.remove(&key);
            return Err(f);
        }
        Ok(&mut self.programs.get_mut(&key).expect("kept").0)
    }
}

/// Why a source could not be drawn.
pub(crate) enum Missed {
    /// The program WebGL draws failed in the native pipeline at pipeline
    /// creation (the Metal compiler): it needs the fallback runner.
    NeedsFallback(String),
    /// The runner's pixels for this request have not arrived.
    Pending(Arc<FallbackRequest>),
}

/// One draw of a legacy program into `out` (cleared to transparent,
/// blending off) with the compositor's calls; samplers read `units` or the
/// empty unit (0, 0, 0, 1).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_program(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    program: &mut GpuProgram,
    calls: &[(String, Call)],
    units: &[Option<&wgpu::TextureView>],
    empty: &wgpu::TextureView,
    linear: &wgpu::Sampler,
    out: &Target,
) {
    let format = out.format();
    let buffer = |block: &pm_glsl::reflect::Block| {
        let bytes = GpuProgram::block_bytes(block, calls);
        let b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm shader layer uniforms"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&b, 0, &bytes);
        b
    };
    let fragment_buffer = program.fragment_block.as_ref().map(buffer);
    let vertex_buffer = program.vertex_block.as_ref().map(buffer);
    let views: Vec<&wgpu::TextureView> = program
        .sampler_units(calls)
        .into_iter()
        .map(|unit| units.get(unit as usize).copied().flatten().unwrap_or(empty))
        .collect();
    let mut entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.fragment_block, &fragment_buffer) {
        entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    for ((_, binding), view) in program.samplers.iter().zip(&views) {
        entries.push(wgpu::BindGroupEntry {
            binding: *binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: binding + pm_glsl::SAMPLER_BINDING_OFFSET,
            resource: wgpu::BindingResource::Sampler(linear),
        });
    }
    let fragment = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer fragment"),
        layout: &program.fragment_layout,
        entries: &entries,
    });
    let mut vertex_entries = Vec::new();
    if let (Some(block), Some(b)) = (&program.vertex_block, &vertex_buffer) {
        vertex_entries.push(wgpu::BindGroupEntry {
            binding: block.binding,
            resource: b.as_entire_binding(),
        });
    }
    let vertex = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm shader layer vertex"),
        layout: &program.vertex_layout,
        entries: &vertex_entries,
    });
    let pipeline = program
        .pipeline(device, format)
        .expect("built by the caller")
        .clone();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pm shader layer"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &out.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&pipeline);
    pass.set_bind_group(0, &fragment, &[]);
    pass.set_bind_group(1, &vertex, &[]);
    pass.draw(0..4, 0..1);
}

/// Uploads runner pixels (RGBA16F, GL row order) as a sampled texture.
pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &FallbackPixels,
) -> Target {
    let target = Target::new(
        device,
        "pm shader fallback pixels",
        pixels.width,
        pixels.height,
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let bytes: Vec<u8> = pixels
        .rgba16f
        .iter()
        .flat_map(|h| h.to_le_bytes())
        .collect();
    queue.write_texture(
        target.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pixels.width * 8),
            rows_per_image: Some(pixels.height),
        },
        wgpu::Extent3d {
            width: pixels.width,
            height: pixels.height,
            depth_or_array_layers: 1,
        },
    );
    target
}

impl crate::exec::Exec<'_> {
    /// Draws a shader source into a new working target (GL row order,
    /// E-encoded legacy output), and whether the target is the pool's.
    pub(crate) fn shader_source(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &ShaderSource,
    ) -> Result<(Target, bool), Missed> {
        if let ProgramRef::Fallback(request) = &source.program {
            return match self.shaders.fallback.get(&request.key) {
                Some(pixels) if pixels.fits(request) => {
                    Ok((upload(self.device, self.queue, &pixels), false))
                }
                _ => Err(Missed::Pending(request.clone())),
            };
        }
        let out = self
            .pool
            .take_working(self.device, source.width, source.height, self.precision);
        let format = out.format();
        let empty = self.effects.dummy(self.device, self.queue, format);
        let linear = &self.pipelines.linear;
        let device = self.device;
        let queue = self.queue;
        let program = match &source.program {
            ProgramRef::Code { key, prepared } => {
                match self.shaders.program(device, *key, prepared, format) {
                    Ok(p) => p,
                    Err(f) => {
                        super::translate::mark_failed(*key, f);
                        self.pool.give(out);
                        return Err(Missed::NeedsFallback("shader layer".into()));
                    }
                }
            }
            ProgramRef::Definition { id, .. } => {
                match self
                    .effects
                    .program(device, DefinitionKind::Layer, id, format)
                {
                    Ok(p) => p,
                    // The definition is now `Status::NeedsFallback`.
                    Err(_) => {
                        self.pool.give(out);
                        return Err(Missed::NeedsFallback(id.clone()));
                    }
                }
            }
            ProgramRef::Fallback(_) => unreachable!("handled above"),
        };
        draw_program(
            device,
            queue,
            encoder,
            program,
            &source.calls,
            &[],
            &empty.view,
            linear,
            &out,
        );
        Ok((out, true))
    }
}
