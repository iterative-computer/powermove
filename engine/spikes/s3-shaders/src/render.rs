//! Offscreen wgpu renderer that replays a corpus case exactly like the legacy
//! compositor: RGBA16F targets cleared to transparent, blending off, linear
//! filtering with clamp-to-edge, a full-target quad, GL uniform semantics.
//!
//! Images and targets are kept in GL row order (row 0 is the bottom), which is
//! what WebGL's `texImage2D` and `readPixels` use, so outputs compare byte for
//! byte with the WebGL reference. See S3.md, "Origin".

use std::borrow::Cow;
use std::collections::HashMap;

use half::f16;

use crate::case::{Call, Corpus, ImageSpec, Pass, Variant};
use crate::pipeline::{Failure, Path, Translated};
use crate::reflect::{Block, Scalar, UniformType};
use crate::rewrite::SAMPLER_BINDING_OFFSET;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    pub passthrough: bool,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub view: wgpu::TextureView,
}

impl Gpu {
    pub fn new() -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))?;
        let passthrough = adapter.features().contains(wgpu::Features::PASSTHROUGH_SHADERS);
        let mut required_features = wgpu::Features::empty();
        if passthrough {
            required_features |= wgpu::Features::PASSTHROUGH_SHADERS;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s3"),
            required_features,
            required_limits: adapter.limits(),
            // SAFETY: passthrough shaders are an experimental feature; this spike
            // only feeds them SPIRV-Cross output for the comparison path.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gl-linear-clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        // WebGL2 samples an unbound unit as (0, 0, 0, 1).
        let dummy = upload(&device, &queue, 1, 1, &[f16::ZERO, f16::ZERO, f16::ZERO, f16::ONE], "unbound-unit");
        Ok(Gpu { adapter_info: adapter.get_info(), device, queue, passthrough, sampler, dummy })
    }

    pub fn load_images(&self, corpus: &Corpus, base: &std::path::Path) -> anyhow::Result<HashMap<String, Image>> {
        let mut out = HashMap::new();
        for spec in &corpus.images {
            out.insert(spec.name.clone(), self.load_image(spec, base)?);
        }
        Ok(out)
    }

    fn load_image(&self, spec: &ImageSpec, base: &std::path::Path) -> anyhow::Result<Image> {
        let bytes = std::fs::read(base.join(&spec.file))?;
        anyhow::ensure!(bytes.len() == (spec.width * spec.height * 8) as usize, "image {} has {} bytes", spec.name, bytes.len());
        let halfs: Vec<f16> = bytes.chunks_exact(2).map(|c| f16::from_le_bytes([c[0], c[1]])).collect();
        Ok(Image { width: spec.width, height: spec.height, view: upload(&self.device, &self.queue, spec.width, spec.height, &halfs, &spec.name) })
    }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, data: &[f16], label: &str) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = data.iter().flat_map(|h| h.to_le_bytes()).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 8), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    texture.create_view(&Default::default())
}

/// Apply one `setU`/`setI` call with GL's type rules (`glUniform*` on a
/// mismatched type is `INVALID_OPERATION` and changes nothing). Returns
/// whether the call wrote anything.
pub fn apply_call(block: &Block, bytes: &mut [u8], call: &Call) -> bool {
    let name = match &call.alt {
        Some(alt) if !block.members.iter().any(|m| m.name == call.name && m.active) => alt,
        _ => &call.name,
    };
    let Some(member) = block.members.iter().find(|m| &m.name == name) else { return false };
    let at = member.offset as usize;
    let mut put = |index: usize, word: [u8; 4]| bytes[at + index * 4..at + index * 4 + 4].copy_from_slice(&word);
    let f = |v: f64| (v as f32).to_le_bytes();
    let b = |v: f64| u32::from(v != 0.0).to_le_bytes();
    match (call.func.as_str(), &member.ty) {
        ("1f", UniformType::Scalar(Scalar::Float)) => put(0, f(call.v[0])),
        ("1f" | "1i", UniformType::Scalar(Scalar::Bool)) => put(0, b(call.v[0])),
        ("1i", UniformType::Scalar(Scalar::Int)) => put(0, (call.v[0] as i32).to_le_bytes()),
        ("2f" | "3f" | "4f", UniformType::Vector(scalar, n)) if call.func.starts_with(char::from_digit(*n, 10).unwrap()) => match scalar {
            Scalar::Float => (0..*n as usize).for_each(|i| put(i, f(call.v[i]))),
            Scalar::Bool => (0..*n as usize).for_each(|i| put(i, b(call.v[i]))),
            _ => return false,
        },
        ("m3", UniformType::Matrix { columns: 3, rows: 3, stride }) => {
            for column in 0..3 {
                for row in 0..3 {
                    let offset = at + (column * *stride as usize) + row * 4;
                    bytes[offset..offset + 4].copy_from_slice(&f(call.v[column * 3 + row]));
                }
            }
        }
        _ => return false,
    }
    true
}

pub struct Program {
    pipeline: wgpu::RenderPipeline,
    fragment_layout: wgpu::BindGroupLayout,
    vertex_layout: wgpu::BindGroupLayout,
    fragment_block: Option<Block>,
    vertex_block: Option<Block>,
    /// Sampler name and SPIR-V image binding, in binding order.
    samplers: Vec<(String, u32)>,
}

fn layout_entries(block: Option<&Block>, samplers: &[(String, u32)], visibility: wgpu::ShaderStages) -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = Vec::new();
    if let Some(block) = block {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: block.binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        });
    }
    for (_, binding) in samplers {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: *binding,
            visibility,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        });
    }
    for (_, binding) in samplers {
        entries.push(wgpu::BindGroupLayoutEntry { binding: binding + SAMPLER_BINDING_OFFSET, visibility, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
    }
    entries.sort_by_key(|e| e.binding);
    entries
}

/// Validation, internal (shader translation and Metal compilation), and
/// out-of-memory scopes, so one bad shader fails its case instead of the run.
struct Scopes([wgpu::ErrorScopeGuard; 3]);

fn push_scopes(device: &wgpu::Device) -> Scopes {
    Scopes([
        device.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
        device.push_error_scope(wgpu::ErrorFilter::Internal),
        device.push_error_scope(wgpu::ErrorFilter::Validation),
    ])
}

fn pop_error(device: &wgpu::Device, scopes: Scopes) -> Option<String> {
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let [oom, internal, validation] = scopes.0;
    let mut errors = Vec::new();
    for guard in [validation, internal, oom] {
        if let Some(error) = pollster::block_on(guard.pop()) {
            errors.push(error.to_string());
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

impl Gpu {
    pub fn build(&self, translated: Translated, path: Path, label: &str) -> Result<Program, Failure> {
        let device = &self.device;
        let fragment_block = translated.fragment.reflection.block.clone();
        let vertex_block = translated.vertex.reflection.block.clone();
        let mut samplers: Vec<(String, u32)> = translated.fragment.reflection.samplers.iter().map(|s| (s.name.clone(), s.binding)).collect();
        samplers.sort_by_key(|s| s.1);
        let guard = push_scopes(device);
        let (vertex_module, fragment_module) = match path {
            Path::Naga => {
                let v = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("vertex"), source: wgpu::ShaderSource::Naga(Cow::Owned(translated.vertex.naga.unwrap())) });
                let f = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Naga(Cow::Owned(translated.fragment.naga.unwrap())) });
                (v, f)
            }
            Path::SpirvCross => {
                if !self.passthrough {
                    return Err(Failure::new("metal", "adapter lacks PASSTHROUGH_SHADERS"));
                }
                let make = |msl: &str, entry: &str, label: &str| {
                    let entry_points = [wgpu::PassthroughShaderEntryPoint { name: Cow::Borrowed(entry), workgroup_size: (0, 0, 0) }];
                    // SAFETY: the MSL comes from SPIRV-Cross with resource indices
                    // pinned to wgpu-hal's Metal slot assignment for this layout.
                    unsafe {
                        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                            label: Some(label),
                            entry_points: Cow::Borrowed(&entry_points),
                            msl: Some(Cow::Borrowed(msl)),
                            ..Default::default()
                        })
                    }
                };
                let v = make(translated.vertex.msl.as_deref().unwrap(), &translated.vertex.entry_point, "vertex");
                let f = make(translated.fragment.msl.as_deref().unwrap(), &translated.fragment.entry_point, label);
                (v, f)
            }
        };
        if let Some(error) = pop_error(device, guard) {
            return Err(Failure::new("metal", error));
        }
        let guard = push_scopes(device);
        let fragment_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fragment"),
            entries: &layout_entries(fragment_block.as_ref(), &samplers, wgpu::ShaderStages::FRAGMENT),
        });
        let vertex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("vertex"), entries: &layout_entries(vertex_block.as_ref(), &[], wgpu::ShaderStages::VERTEX) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&fragment_layout), Some(&vertex_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vertex_module, entry_point: Some(&translated.vertex.entry_point), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment_module,
                entry_point: Some(&translated.fragment.entry_point),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = pop_error(device, guard) {
            return Err(Failure::new("metal", error));
        }
        Ok(Program { pipeline, fragment_layout, vertex_layout, fragment_block, vertex_block, samplers })
    }

    /// Render every pass of a variant; returns the final target as f16 RGBA in
    /// GL row order.
    pub fn render(&self, program: &Program, variant: &Variant, images: &HashMap<String, Image>) -> Result<Vec<f16>, Failure> {
        let device = &self.device;
        let guard = push_scopes(device);
        let mut outputs: Vec<wgpu::Texture> = Vec::new();
        for (index, pass) in variant.passes.iter().enumerate() {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pass"),
                size: wgpu::Extent3d { width: variant.width, height: variant.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let bind_groups = self.bind_groups(program, pass, images, &outputs)?;
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let view = target.create_view(&Default::default());
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&program.pipeline);
                rp.set_bind_group(0, &bind_groups.0, &[]);
                rp.set_bind_group(1, &bind_groups.1, &[]);
                rp.draw(0..4, 0..1);
            }
            self.queue.submit([encoder.finish()]);
            let _ = index;
            outputs.push(target);
        }
        let pixels = self.read_back(outputs.last().unwrap(), variant.width, variant.height);
        if let Some(error) = pop_error(device, guard) {
            return Err(Failure::new("render", error));
        }
        Ok(pixels)
    }

    fn bind_groups(&self, program: &Program, pass: &Pass, images: &HashMap<String, Image>, outputs: &[wgpu::Texture]) -> Result<(wgpu::BindGroup, wgpu::BindGroup), Failure> {
        let device = &self.device;
        let block_buffer = |block: &Option<Block>| -> Option<wgpu::Buffer> {
            let block = block.as_ref()?;
            let mut bytes = vec![0u8; block.size.max(16) as usize];
            for call in &pass.calls {
                apply_call(block, &mut bytes, call);
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("uniforms"), size: bytes.len() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.queue.write_buffer(&buffer, 0, &bytes);
            Some(buffer)
        };
        let fragment_buffer = block_buffer(&program.fragment_block);
        let vertex_buffer = block_buffer(&program.vertex_block);
        // Texture units: which view sits on each unit for this pass.
        let mut unit_views: HashMap<u32, wgpu::TextureView> = HashMap::new();
        for unit in &pass.units {
            let view = if let Some(k) = unit.src.strip_prefix("pass:") {
                let k: usize = k.parse().map_err(|_| Failure::new("render", format!("bad unit source {}", unit.src)))?;
                outputs.get(k).ok_or_else(|| Failure::new("render", format!("pass {k} has no output yet")))?.create_view(&Default::default())
            } else {
                images.get(&unit.src).ok_or_else(|| Failure::new("render", format!("unknown image {}", unit.src)))?.view.clone()
            };
            unit_views.insert(unit.unit, view);
        }
        // Sampler uniforms default to unit 0; `setI(name, unit)` moves them.
        let unit_of = |name: &str| -> u32 { pass.calls.iter().rev().find(|c| c.name == name && c.func == "1i").map_or(0, |c| c.v[0] as u32) };
        let mut entries: Vec<wgpu::BindGroupEntry> = Vec::new();
        let views: Vec<wgpu::TextureView> = program.samplers.iter().map(|(name, _)| unit_views.get(&unit_of(name)).cloned().unwrap_or_else(|| self.dummy.clone())).collect();
        if let (Some(block), Some(buffer)) = (&program.fragment_block, &fragment_buffer) {
            entries.push(wgpu::BindGroupEntry { binding: block.binding, resource: buffer.as_entire_binding() });
        }
        for ((_, binding), view) in program.samplers.iter().zip(&views) {
            entries.push(wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) });
            entries.push(wgpu::BindGroupEntry { binding: binding + SAMPLER_BINDING_OFFSET, resource: wgpu::BindingResource::Sampler(&self.sampler) });
        }
        let fragment = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fragment"), layout: &program.fragment_layout, entries: &entries });
        let mut vertex_entries = Vec::new();
        if let (Some(block), Some(buffer)) = (&program.vertex_block, &vertex_buffer) {
            vertex_entries.push(wgpu::BindGroupEntry { binding: block.binding, resource: buffer.as_entire_binding() });
        }
        let vertex = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vertex"), layout: &program.vertex_layout, entries: &vertex_entries });
        Ok((fragment, vertex))
    }

    fn read_back(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<f16> {
        let row = width * 8;
        let padded = row.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (padded * height) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("mapped readback");
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let start = (y * padded) as usize;
            for chunk in data[start..start + row as usize].chunks_exact(2) {
                out.push(f16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Member;

    fn block() -> Block {
        let m = |name: &str, offset, ty| Member { name: name.into(), offset, ty, active: true };
        Block {
            set: 0,
            binding: 0,
            size: 64,
            members: vec![
                m("f", 0, UniformType::Scalar(Scalar::Float)),
                m("i", 4, UniformType::Scalar(Scalar::Int)),
                m("b", 8, UniformType::Scalar(Scalar::Bool)),
                m("v2", 16, UniformType::Vector(Scalar::Float, 2)),
                m("v3", 32, UniformType::Vector(Scalar::Float, 3)),
            ],
        }
    }

    fn call(func: &str, name: &str, v: &[f64]) -> Call {
        Call { func: func.into(), name: name.into(), alt: None, v: v.to_vec() }
    }

    #[test]
    fn follows_gl_uniform_type_rules() {
        let block = block();
        let mut bytes = vec![0u8; 64];
        assert!(apply_call(&block, &mut bytes, &call("1f", "f", &[2.5])));
        assert!(!apply_call(&block, &mut bytes, &call("1f", "i", &[3.0])), "glUniform1f on int is INVALID_OPERATION");
        assert!(apply_call(&block, &mut bytes, &call("1i", "b", &[1.0])), "bool accepts glUniform1i");
        assert!(apply_call(&block, &mut bytes, &call("1f", "b", &[1.0])), "bool accepts glUniform1f");
        assert!(!apply_call(&block, &mut bytes, &call("1f", "v2", &[1.0])), "a single float leaves vec2 at 0 (compositor.ts:1024-1030)");
        assert!(!apply_call(&block, &mut bytes, &call("3f", "v2", &[1.0, 2.0, 3.0])));
        assert!(apply_call(&block, &mut bytes, &call("3f", "v3", &[1.0, 2.0, 3.0])));
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 2.5);
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(&bytes[16..24], &[0; 8]);
        assert_eq!(f32::from_le_bytes(bytes[40..44].try_into().unwrap()), 3.0);
    }

    #[test]
    fn named_params_fall_back_to_positional_when_inactive() {
        let mut block = block();
        block.members.push(Member { name: "u_amount".into(), offset: 48, ty: UniformType::Scalar(Scalar::Float), active: false });
        block.members.push(Member { name: "u_p0".into(), offset: 52, ty: UniformType::Scalar(Scalar::Float), active: true });
        let mut bytes = vec![0u8; 64];
        let mut c = call("1f", "u_amount", &[7.0]);
        c.alt = Some("u_p0".into());
        assert!(apply_call(&block, &mut bytes, &c));
        assert_eq!(f32::from_le_bytes(bytes[52..56].try_into().unwrap()), 7.0);
        assert_eq!(&bytes[48..52], &[0; 4]);
    }
}
