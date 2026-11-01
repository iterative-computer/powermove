//! The viewer display transform: working texture to pool surface
//! (`display.wgsl`; plan sections 9 and 18, amendment 4).

use std::sync::OnceLock;

use pm_protocol::color::{ViewerColorSpace, WorkingEncoding};
use pm_protocol::common::Size;
use pm_protocol::interop::SurfaceFormat;

use super::BridgeOutError;
use super::surface::{ExportedSurface, wgpu_format};

/// The format frames are rendered in before the display transform.
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Side of the blue-noise tile.
pub const NOISE_SIZE: u32 = 64;

/// The 64 × 64 void-and-cluster thresholds, `(rank + 0.5) / 4096`, computed
/// once per process.
pub fn blue_noise() -> &'static [f32] {
    static NOISE: OnceLock<Vec<f32>> = OnceLock::new();
    NOISE.get_or_init(|| {
        let n = NOISE_SIZE as usize;
        super::noise::void_and_cluster(n)
            .iter()
            .map(|&r| (r as f32 + 0.5) / (n * n) as f32)
            .collect()
    })
}

/// A render target frames are drawn into before the display transform.
pub struct WorkingTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: Size,
}

impl WorkingTexture {
    pub fn new(device: &wgpu::Device, size: Size) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewer working frame"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: WORKING_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
        }
    }
}

/// How one frame is transformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayParams {
    pub working: WorkingEncoding,
    pub canvas: ViewerColorSpace,
    /// Offset into the noise tile; vary it per frame so the dither is not a
    /// fixed pattern.
    pub noise_offset: [u32; 2],
}

impl DisplayParams {
    /// The offset sequence the engine uses: a different tile phase per frame.
    pub fn noise_offset_for(seq: u64) -> [u32; 2] {
        [
            ((seq * 17) % u64::from(NOISE_SIZE)) as u32,
            ((seq * 29) % u64::from(NOISE_SIZE)) as u32,
        ]
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    flags: u32,
    _pad: u32,
    noise_offset: [u32; 2],
}

fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: T is a repr(C) plain-old-data struct without padding reads.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// The compiled display transform for both pool formats.
pub struct DisplayTransform {
    layout: wgpu::BindGroupLayout,
    bgra: wgpu::RenderPipeline,
    f16: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    noise: wgpu::TextureView,
}

impl DisplayTransform {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("display transform"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    pm_color::wgsl::CSS_GAMUT,
                    include_str!("display.wgsl")
                )
                .into(),
            ),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display transform"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("display transform"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("display transform"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display transform"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let extent = wgpu::Extent3d {
            width: NOISE_SIZE,
            height: NOISE_SIZE,
            depth_or_array_layers: 1,
        };
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blue noise"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texels: Vec<u8> = blue_noise().iter().flat_map(|v| v.to_ne_bytes()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &noise,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(NOISE_SIZE * 4),
                rows_per_image: Some(NOISE_SIZE),
            },
            extent,
        );
        Self {
            bgra: pipeline(wgpu::TextureFormat::Bgra8Unorm),
            f16: pipeline(wgpu::TextureFormat::Rgba16Float),
            layout,
            uniforms,
            noise: noise.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// Records the transform of `source` into `target`. The uniforms are
    /// written now, so record at most one transform per submission.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &WorkingTexture,
        target: &ExportedSurface,
        params: DisplayParams,
    ) -> Result<(), BridgeOutError> {
        if source.size != target.size {
            return Err(BridgeOutError::Size(source.size));
        }
        let eight_bit = match target.format {
            SurfaceFormat::Bgra8Unorm => true,
            SurfaceFormat::Rgba16Float => false,
            other => return Err(BridgeOutError::Format(other)),
        };
        wgpu_format(target.format)?;
        let mut flags = 0;
        if eight_bit {
            flags |= 1;
        }
        if params.working == WorkingEncoding::LinearLight {
            flags |= 2;
        }
        if params.canvas == ViewerColorSpace::DisplayP3 {
            flags |= 4;
        }
        let uniforms = Uniforms {
            flags,
            _pad: 0,
            noise_offset: params.noise_offset,
        };
        queue.write_buffer(&self.uniforms, 0, bytes_of(&uniforms));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display transform"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.noise),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("display transform"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(if eight_bit { &self.bgra } else { &self.f16 });
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
