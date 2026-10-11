//! wgpu device on Metal, CVPixelBuffer import through CVMetalTextureCache, and the
//! prototype conversion pipeline (pipeline.wgsl).

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CFRetained;
use objc2_core_video::{CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, kCVMetalTextureUsage};
use objc2_foundation::NSNumber;
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTexture, MTLTextureType, MTLTextureUsage};

use crate::cf::{self, Dict, Result, err};
use crate::pixbuf::{self, PixelBuffer};

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    cache: CFRetained<CVMetalTextureCache>,
    pipeline_module: wgpu::ShaderModule,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    dummy: wgpu::TextureView,
}

struct SendCv(#[allow(dead_code)] CFRetained<CVMetalTexture>);
unsafe impl Send for SendCv {}
unsafe impl Sync for SendCv {}

/// Working storage precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Precision {
    F16,
    F32,
}

impl Precision {
    pub fn format(self) -> wgpu::TextureFormat {
        match self {
            Precision::F16 => wgpu::TextureFormat::Rgba16Float,
            Precision::F32 => wgpu::TextureFormat::Rgba32Float,
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct PipeParams {
    pub precision: Precision,
    pub linear: bool,
    pub premultiply: bool,
    pub clip: bool,
}

impl PipeParams {
    pub fn standard(precision: Precision) -> Self {
        Self { precision, linear: false, premultiply: true, clip: true }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ParamsUniform {
    linear: u32,
    premultiply: u32,
    clip: u32,
    alpha_guard: f32,
}

pub const WANTED_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
    .union(wgpu::Features::FLOAT32_FILTERABLE)
    .union(wgpu::Features::FLOAT32_BLENDABLE)
    .union(wgpu::Features::PASSTHROUGH_SHADERS)
    .union(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);

impl Gpu {
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| cf::Error(format!("request_adapter: {e}")))?;
        let available = adapter.features();
        let features = WANTED_FEATURES & available;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("s2"),
            required_features: features,
            required_limits: adapter.limits(),
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }))
        .map_err(|e| cf::Error(format!("request_device: {e}")))?;

        let mtl: Retained<ProtocolObject<dyn MTLDevice>> = unsafe {
            let hal = device.as_hal::<wgpu::hal::api::Metal>().ok_or_else(|| cf::Error("not Metal".into()))?;
            hal.raw_device().clone()
        };
        let mut cache_ptr: *mut CVMetalTextureCache = std::ptr::null_mut();
        let r = unsafe { CVMetalTextureCache::create(None, None, &mtl, None, NonNull::from(&mut cache_ptr)) };
        if r != 0 || cache_ptr.is_null() {
            return err(format!("CVMetalTextureCacheCreate: {r}"));
        }
        let cache = unsafe { CFRetained::from_raw(NonNull::new_unchecked(cache_ptr)) };

        let pipeline_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pipeline.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pipeline.wgsl").into()),
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pipeline bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Ok(Self { adapter, device, queue, cache, pipeline_module, bgl, layout, dummy })
    }

    pub fn mtl_device(&self) -> Retained<ProtocolObject<dyn MTLDevice>> {
        unsafe { self.device.as_hal::<wgpu::hal::api::Metal>().unwrap().raw_device().clone() }
    }

    /// Import one plane of an IOSurface-backed pixel buffer as a wgpu texture, without copies.
    pub fn import_plane(
        &self,
        pb: &PixelBuffer,
        plane: usize,
        mtl_format: MTLPixelFormat,
        format: wgpu::TextureFormat,
    ) -> Result<wgpu::Texture> {
        let planar = pb.plane_count() > 0;
        let (w, h) = if planar {
            (
                objc2_core_video::CVPixelBufferGetWidthOfPlane(pb.as_cf(), plane),
                objc2_core_video::CVPixelBufferGetHeightOfPlane(pb.as_cf(), plane),
            )
        } else {
            (pb.width(), pb.height())
        };
        let usage = MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderWrite;
        let attrs = Dict::new()
            .cfkey(unsafe { kCVMetalTextureUsage }, &NSNumber::new_usize(usage.0))
            .build();
        let mut out: *mut CVMetalTexture = std::ptr::null_mut();
        let r = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                &self.cache,
                pb.as_cf(),
                Some(cf::cf_dict(&attrs)),
                mtl_format,
                w,
                h,
                plane,
                NonNull::from(&mut out),
            )
        };
        if r != 0 || out.is_null() {
            return err(format!(
                "CVMetalTextureCacheCreateTextureFromImage plane {plane} of '{}' as {:?}: {r}",
                cf::fourcc_str(pb.format()),
                mtl_format
            ));
        }
        let cvtex = unsafe { CFRetained::from_raw(NonNull::new_unchecked(out)) };
        let mtl_tex: Retained<ProtocolObject<dyn MTLTexture>> =
            CVMetalTextureGetTexture(&cvtex).ok_or_else(|| cf::Error("CVMetalTextureGetTexture null".into()))?;
        let keep = SendCv(cvtex);
        let size = wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 };
        let hal_tex = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                mtl_tex,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent { width: w as u32, height: h as u32, depth: 1 },
                Some(Box::new(move || drop(keep))),
            )
        };
        let tex = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal_tex,
                &wgpu::TextureDescriptor {
                    label: Some("cv import"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        Ok(tex)
    }

    pub fn working_texture(&self, w: u32, h: u32, precision: Precision) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("working"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: precision.format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn params_buffer(&self, p: &PipeParams) -> wgpu::Buffer {
        let u = ParamsUniform {
            linear: p.linear as u32,
            premultiply: p.premultiply as u32,
            clip: p.clip as u32,
            // Section 9: unpremultiply guarded for α < 1/4096 at half float.
            alpha_guard: match p.precision {
                Precision::F16 => 1.0 / 4096.0,
                Precision::F32 => f32::MIN_POSITIVE,
            },
        };
        let bytes: [u8; 16] = unsafe { std::mem::transmute(u) };
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf, 0, &bytes);
        buf
    }

    /// One fullscreen fragment pass from up to two sources into a target.
    pub fn pass(
        &self,
        entry: &str,
        src0: &wgpu::TextureView,
        src1: Option<&wgpu::TextureView>,
        params: &PipeParams,
        target: &wgpu::Texture,
    ) {
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.pipeline_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.pipeline_module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let ub = self.params_buffer(params);
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(entry),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src0) },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src1.unwrap_or(&self.dummy)),
                },
                wgpu::BindGroupEntry { binding: 2, resource: ub.as_entire_binding() },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(entry),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&pipeline);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
    }

    pub fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }

    /// Engine path for a y416 source: decode buffer → working texture → y416 sink buffer.
    pub fn run_y416(&self, src: &PixelBuffer, sink: &PixelBuffer, p: &PipeParams) -> Result<()> {
        let s = self.import_plane(src, 0, MTLPixelFormat::RGBA16Unorm, wgpu::TextureFormat::Rgba16Unorm)?;
        let t = self.import_plane(sink, 0, MTLPixelFormat::RGBA16Unorm, wgpu::TextureFormat::Rgba16Unorm)?;
        let work = self.working_texture(src.width() as u32, src.height() as u32, p.precision);
        self.pass("fs_from_y416", &s.create_view(&Default::default()), None, p, &work);
        self.pass("fs_to_y416", &work.create_view(&Default::default()), None, p, &t);
        self.wait();
        Ok(())
    }

    /// Engine path for an x422 source: decode buffer → working texture → x422 sink buffer.
    pub fn run_x422(&self, src: &PixelBuffer, sink: &PixelBuffer, p: &PipeParams) -> Result<()> {
        let sy = self.import_plane(src, 0, MTLPixelFormat::R16Unorm, wgpu::TextureFormat::R16Unorm)?;
        let sc = self.import_plane(src, 1, MTLPixelFormat::RG16Unorm, wgpu::TextureFormat::Rg16Unorm)?;
        let ty = self.import_plane(sink, 0, MTLPixelFormat::R16Unorm, wgpu::TextureFormat::R16Unorm)?;
        let tc = self.import_plane(sink, 1, MTLPixelFormat::RG16Unorm, wgpu::TextureFormat::Rg16Unorm)?;
        let work = self.working_texture(src.width() as u32, src.height() as u32, p.precision);
        self.pass(
            "fs_from_x422",
            &sy.create_view(&Default::default()),
            Some(&sc.create_view(&Default::default())),
            p,
            &work,
        );
        let wv = work.create_view(&Default::default());
        self.pass("fs_to_x422_luma", &wv, None, p, &ty);
        self.pass("fs_to_x422_chroma", &wv, None, p, &tc);
        self.wait();
        Ok(())
    }

    /// Engine path for 16-bit straight RGBA: 'l64r' buffer → working texture → 'l64r' sink.
    pub fn run_rgba16(&self, src: &PixelBuffer, sink: &PixelBuffer, p: &PipeParams) -> Result<()> {
        let s = self.import_plane(src, 0, MTLPixelFormat::RGBA16Unorm, wgpu::TextureFormat::Rgba16Unorm)?;
        let t = self.import_plane(sink, 0, MTLPixelFormat::RGBA16Unorm, wgpu::TextureFormat::Rgba16Unorm)?;
        let work = self.working_texture(src.width() as u32, src.height() as u32, p.precision);
        self.pass("fs_from_rgba16", &s.create_view(&Default::default()), None, p, &work);
        self.pass("fs_to_rgba16", &work.create_view(&Default::default()), None, p, &t);
        self.wait();
        Ok(())
    }

    /// Run the engine path for a decoded buffer into a freshly allocated sink of the same format.
    pub fn engine_copy(&self, src: &PixelBuffer, p: &PipeParams) -> Result<PixelBuffer> {
        let sink = PixelBuffer::new(src.width(), src.height(), src.format())?;
        match src.format() {
            pixbuf::Y416 => self.run_y416(src, &sink, p)?,
            pixbuf::X422 => self.run_x422(src, &sink, p)?,
            pixbuf::RGBA64 => self.run_rgba16(src, &sink, p)?,
            f => return err(format!("engine_copy: unsupported format '{}'", cf::fourcc_str(f))),
        }
        Ok(sink)
    }
}
