//! Decoded frames into wgpu without copies (contract 3 `import_decoded`,
//! section 13 "Metal import"), and the section 13 normalization pass.
//!
//! * **CoreVideo buffers** (VideoToolbox, `v216`, `y416`, `v0a8`, biplanar
//!   formats) come in per plane through `CVMetalTextureCache`: `R8`/`RG8`
//!   or `R16Unorm`/`RG16Unorm` planes, `y416` and `v216` as one
//!   `RGBA16Unorm` texture. Every texture is checked to be the buffer's
//!   IOSurface (and plane).
//! * **Pool frames** (FFmpeg, dav1d, image readers): each pool slot is
//!   wrapped once in a shared-storage `MTLBuffer` over the decoder's memory
//!   (`newBufferWithBytesNoCopy`, cached on the slot), and each plane is a
//!   linear texture at the exact address the decoder wrote it.
//!
//! Textures are wrapped through `wgpu_hal::metal::Device::texture_from_raw`
//! and `create_texture_from_hal`; each texture's drop callback keeps the
//! decoded frame (and its CoreVideo texture) alive until wgpu is done.

pub mod deinterlace;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLBuffer, MTLDevice, MTLPixelFormat, MTLResourceOptions, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use pm_media::apple::cf::*;
use pm_media::frame::{DecodedFrame, Layout, PixelFormat, Pixels, PlaneFormat};
use pm_protocol::color::ColorDescription;
use pm_protocol::common::{PixelRect, Size};
use pm_protocol::interop::DecodedFrameDesc;
use std::ptr::NonNull;
use std::sync::Arc;

const SHADER: &str = include_str!("normalize.wgsl");

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a Metal device")]
    NotMetal,
    #[error("the device lacks TEXTURE_FORMAT_16BIT_NORM (R16Unorm, RG16Unorm, RGBA16Unorm)")]
    No16BitNorm,
    #[error("CoreVideo {what}: {status}")]
    CoreVideo { what: String, status: i32 },
    #[error("Metal {0}")]
    Metal(String),
    #[error("{0}")]
    Media(#[from] pm_media::MediaError),
    #[error("plane {plane}: offset {offset} or stride {stride} is not {align}-byte aligned")]
    Alignment {
        plane: usize,
        offset: usize,
        stride: usize,
        align: usize,
    },
    #[error("{0}")]
    Unsupported(String),
}

pub type Result<T, E = ImportError> = std::result::Result<T, E>;

/// Features a device needs for decode import.
pub fn required_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}

fn plane_formats(f: PlaneFormat) -> (MTLPixelFormat, wgpu::TextureFormat) {
    match f {
        PlaneFormat::R8 => (MTLPixelFormat::R8Unorm, wgpu::TextureFormat::R8Unorm),
        PlaneFormat::Rg8 => (MTLPixelFormat::RG8Unorm, wgpu::TextureFormat::Rg8Unorm),
        PlaneFormat::R16 => (MTLPixelFormat::R16Unorm, wgpu::TextureFormat::R16Unorm),
        PlaneFormat::Rg16 => (MTLPixelFormat::RG16Unorm, wgpu::TextureFormat::Rg16Unorm),
        PlaneFormat::Rgba8 => (MTLPixelFormat::RGBA8Unorm, wgpu::TextureFormat::Rgba8Unorm),
        PlaneFormat::Rgba16 => (
            MTLPixelFormat::RGBA16Unorm,
            wgpu::TextureFormat::Rgba16Unorm,
        ),
        PlaneFormat::Rgba16Float => (
            MTLPixelFormat::RGBA16Float,
            wgpu::TextureFormat::Rgba16Float,
        ),
        PlaneFormat::Rgba32Float => (
            MTLPixelFormat::RGBA32Float,
            wgpu::TextureFormat::Rgba32Float,
        ),
    }
}

/// A decoded frame's planes as wgpu textures over the decoder's memory.
pub struct ImportedFrame {
    pub textures: Vec<wgpu::Texture>,
    pub format: Arc<PixelFormat>,
    /// Every plane was checked to alias the decoder's output (the
    /// CVPixelBuffer's IOSurface and plane, or the pool memory at the address
    /// the decoder reported). False for frames copied on decode.
    pub aliased: bool,
    /// The contract 3 descriptor, when the layout has one and the frame
    /// carries its color.
    pub desc: Option<DecodedFrameDesc>,
}

/// The normalization output.
pub enum Normalized {
    /// (Y′, 0, 0, A) at luma resolution and (Cb, Cr) at chroma resolution.
    YCbCr {
        luma: wgpu::Texture,
        chroma: wgpu::Texture,
    },
    /// (R, G, B, A).
    Rgb { rgba: wgpu::Texture },
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    kind: u32,
    alpha: u32,
    sx: u32,
    sy: u32,
    code_max: f32,
    a_code_max: f32,
    y_off: f32,
    y_div: f32,
    c_off: f32,
    c_div: f32,
    a_div: f32,
    _pad: f32,
}

/// One per wgpu device.
pub struct DecodeImporter {
    device: wgpu::Device,
    mtl: Retained<ProtocolObject<dyn MTLDevice>>,
    /// Identifies the device for slot wrapper caching.
    device_id: u64,
    cache: Cf,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    dummy: wgpu::TextureView,
    dummy_out: wgpu::TextureView,
}

/// An `MTLBuffer` over one pool slot, cached on the slot.
struct SlotBuffer(Retained<ProtocolObject<dyn MTLBuffer>>);

// Metal buffers are thread-safe objects.
unsafe impl Send for SlotBuffer {}
unsafe impl Sync for SlotBuffer {}

impl DecodeImporter {
    pub fn new(device: &wgpu::Device) -> Result<DecodeImporter> {
        if !device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        {
            return Err(ImportError::No16BitNorm);
        }
        let mtl = unsafe {
            let hal = device
                .as_hal::<wgpu_hal::api::Metal>()
                .ok_or(ImportError::NotMetal)?;
            hal.raw_device().clone()
        };
        let device_id = mtl.registryID();
        let mut cache: CVMetalTextureCacheRef = std::ptr::null_mut();
        let st = unsafe {
            CVMetalTextureCacheCreate(
                kCFAllocatorDefault,
                std::ptr::null(),
                Retained::as_ptr(&mtl) as *mut std::ffi::c_void,
                std::ptr::null(),
                &mut cache,
            )
        };
        if st != 0 {
            return Err(ImportError::CoreVideo {
                what: "CVMetalTextureCacheCreate".into(),
                status: st,
            });
        }
        let cache = unsafe { Cf::from_create(cache as CFTypeRef) }.unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-gpu normalize"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |b| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let out = |b, format| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm-gpu normalize"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                out(5, wgpu::TextureFormat::Rgba32Float),
                out(6, wgpu::TextureFormat::Rg32Float),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm-gpu normalize"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pm-gpu normalize"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let small = |format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("pm-gpu placeholder"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Ok(DecodeImporter {
            device: device.clone(),
            mtl,
            device_id,
            cache,
            pipeline,
            layout,
            dummy: small(
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            dummy_out: small(
                wgpu::TextureFormat::Rg32Float,
                wgpu::TextureUsages::STORAGE_BINDING,
            ),
        })
    }

    fn wrap(
        &self,
        raw: Retained<ProtocolObject<dyn MTLTexture>>,
        format: wgpu::TextureFormat,
        w: u32,
        h: u32,
        keep: Box<dyn FnOnce() + Send + Sync>,
    ) -> wgpu::Texture {
        unsafe {
            let hal = wgpu_hal::metal::Device::texture_from_raw(
                raw,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu_hal::CopyExtent {
                    width: w,
                    height: h,
                    depth: 1,
                },
                Some(keep),
            );
            self.device.create_texture_from_hal::<wgpu_hal::api::Metal>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("decoded plane"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE,
            )
        }
    }

    /// Imports every plane of a decoded frame (contract 3 invariant 1: one
    /// texture per plane, in order, no pixel copies).
    pub fn import(&self, frame: &DecodedFrame) -> Result<ImportedFrame> {
        let f = frame.format.clone();
        let mut textures = Vec::with_capacity(f.planes.len());
        let mut aliased = !frame.copied();
        match &frame.pixels {
            Pixels::CoreVideo(pb) => {
                let planar = unsafe { CVPixelBufferIsPlanar(pb.as_mut_ptr()) } != 0;
                let surface = unsafe { CVPixelBufferGetIOSurface(pb.as_mut_ptr()) }
                    as *const std::ffi::c_void;
                for (i, p) in f.planes.iter().enumerate() {
                    let (mf, wf) = plane_formats(p.format);
                    let mut t: CVMetalTextureRef = std::ptr::null_mut();
                    let st = unsafe {
                        CVMetalTextureCacheCreateTextureFromImage(
                            kCFAllocatorDefault,
                            self.cache.as_mut_ptr(),
                            pb.as_mut_ptr(),
                            std::ptr::null(),
                            mf.0,
                            p.width,
                            p.height,
                            if planar { i } else { 0 },
                            &mut t,
                        )
                    };
                    if st != 0 || t.is_null() {
                        return Err(ImportError::CoreVideo {
                            what: format!("texture from image, plane {i}"),
                            status: st,
                        });
                    }
                    let cvt = unsafe { Cf::from_create(t as CFTypeRef) }.unwrap();
                    let raw = unsafe {
                        let mt = CVMetalTextureGetTexture(cvt.as_mut_ptr());
                        Retained::retain(mt as *mut ProtocolObject<dyn MTLTexture>).ok_or_else(
                            || ImportError::Metal("CVMetalTextureGetTexture returned null".into()),
                        )?
                    };
                    let tex_surface = raw
                        .iosurface()
                        .map(|s| Retained::as_ptr(&s) as *const std::ffi::c_void);
                    let plane_ok = !planar || raw.iosurfacePlane() == i;
                    aliased &= !surface.is_null() && tex_surface == Some(surface) && plane_ok;
                    let keep_frame = frame.clone();
                    let keep = Box::new(move || {
                        drop(cvt);
                        drop(keep_frame);
                    });
                    textures.push(self.wrap(raw, wf, p.width as u32, p.height as u32, keep));
                }
            }
            Pixels::Pool(pf) => {
                for (i, (p, pl)) in f.planes.iter().zip(&pf.planes).enumerate() {
                    let (mf, wf) = plane_formats(p.format);
                    let handle = pl.slot.gpu_handle(self.device_id, || {
                        let ptr = NonNull::new(pl.slot.ptr() as *mut std::ffi::c_void).unwrap();
                        let buf = unsafe {
                            self.mtl
                                .newBufferWithBytesNoCopy_length_options_deallocator(
                                    ptr,
                                    pl.slot.len(),
                                    MTLResourceOptions::StorageModeShared,
                                    None,
                                )
                        }
                        .ok_or_else(|| {
                            pm_media::MediaError::invalid("newBufferWithBytesNoCopy failed")
                        })?;
                        Ok(Arc::new(SlotBuffer(buf)))
                    })?;
                    let buf = handle
                        .downcast_ref::<SlotBuffer>()
                        .ok_or_else(|| ImportError::Metal("slot handle type".into()))?;
                    let align = self.mtl.minimumLinearTextureAlignmentForPixelFormat(mf);
                    if pl.offset % align != 0 || pl.stride % align != 0 {
                        return Err(ImportError::Alignment {
                            plane: i,
                            offset: pl.offset,
                            stride: pl.stride,
                            align,
                        });
                    }
                    let raw = unsafe {
                        let td = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(mf, p.width, p.height, false);
                        td.setStorageMode(MTLStorageMode::Shared);
                        td.setUsage(MTLTextureUsage::ShaderRead);
                        buf.0
                            .newTextureWithDescriptor_offset_bytesPerRow(&td, pl.offset, pl.stride)
                            .ok_or_else(|| {
                                ImportError::Metal(format!("linear texture for plane {i}"))
                            })?
                    };
                    let tex_ptr = raw
                        .buffer()
                        .map(|b| b.contents().as_ptr() as usize + raw.bufferOffset())
                        .unwrap_or(0);
                    aliased &= tex_ptr == pl.decoder_ptr;
                    let keep_frame = frame.clone();
                    let keep_handle = handle.clone();
                    let keep = Box::new(move || {
                        drop(keep_handle);
                        drop(keep_frame);
                    });
                    textures.push(self.wrap(raw, wf, p.width as u32, p.height as u32, keep));
                }
            }
        }
        let desc = match (f.decoded_layout(), frame.color) {
            (Some(layout), Some(color)) => Some(descriptor(&f, layout, color)),
            _ => None,
        };
        Ok(ImportedFrame {
            textures,
            format: f,
            aliased,
            desc,
        })
    }

    /// Records the section 13 normalization of an imported frame into
    /// `encoder` and returns its output textures (rgba32float and rg32float,
    /// `STORAGE_BINDING | TEXTURE_BINDING | COPY_SRC`).
    pub fn normalize(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        imp: &ImportedFrame,
    ) -> Result<Normalized> {
        let f = &imp.format;
        let n = f.norm();
        let unorm_max = |pf: PlaneFormat| match pf {
            PlaneFormat::R8 | PlaneFormat::Rg8 | PlaneFormat::Rgba8 => 255.0,
            PlaneFormat::R16 | PlaneFormat::Rg16 | PlaneFormat::Rgba16 => 65535.0,
            PlaneFormat::Rgba16Float | PlaneFormat::Rgba32Float => 0.0,
        };
        let (kind, alpha) = match f.layout {
            Layout::Biplanar => (0, 0),
            Layout::BiplanarAlpha => (0, 1),
            Layout::Planar => (1, if f.has_alpha() { 3 } else { 0 }),
            Layout::Ayuv16 => (2, 2),
            Layout::Cbycry16 => (3, 0),
            Layout::PlanarRgb => (4, if f.has_alpha() { 3 } else { 0 }),
            Layout::Rgba => (5, 2),
        };
        let alpha_plane = match alpha {
            1 => Some(2),
            3 => Some(3),
            2 => Some(0),
            _ => None,
        };
        let params = Params {
            kind,
            alpha,
            sx: f.log2_chroma_w as u32,
            sy: f.log2_chroma_h as u32,
            code_max: unorm_max(f.planes[0].format),
            a_code_max: alpha_plane
                .map(|p| unorm_max(f.planes[p].format))
                .unwrap_or(0.0),
            y_off: n.y_off as f32,
            y_div: n.y_div as f32,
            c_off: n.c_off as f32,
            c_div: n.c_div as f32,
            a_div: n.a_div as f32,
            _pad: 0.0,
        };
        let (w, h) = (f.width as u32, f.height as u32);
        let cw = (w + (1 << f.log2_chroma_w) - 1) >> f.log2_chroma_w;
        let ch = (h + (1 << f.log2_chroma_h) - 1) >> f.log2_chroma_h;
        let mk = |w, h, format, label| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let rgb = kind >= 4;
        let out0 = mk(
            w,
            h,
            wgpu::TextureFormat::Rgba32Float,
            "normalized luma/rgba",
        );
        let out1 = (!rgb).then(|| mk(cw, ch, wgpu::TextureFormat::Rg32Float, "normalized chroma"));
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("normalize params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, unsafe {
            std::slice::from_raw_parts(
                &params as *const Params as *const u8,
                std::mem::size_of::<Params>(),
            )
        });
        let views: Vec<wgpu::TextureView> = imp
            .textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let v = |i: usize| views.get(i).unwrap_or(&self.dummy);
        let o0 = out0.create_view(&Default::default());
        let o1 = out1.as_ref().map(|t| t.create_view(&Default::default()));
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("normalize"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(v(0)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(v(1)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(v(2)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(v(3)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&o0),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(
                        o1.as_ref().unwrap_or(&self.dummy_out),
                    ),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("normalize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        Ok(match out1 {
            Some(chroma) => Normalized::YCbCr { luma: out0, chroma },
            None => Normalized::Rgb { rgba: out0 },
        })
    }

    /// Releases CoreVideo's texture cache entries no longer in use.
    pub fn flush_cache(&self) {
        unsafe { CVMetalTextureCacheFlush(self.cache.as_mut_ptr(), 0) }
    }
}

/// The contract 3 descriptor for a frame.
pub fn descriptor(
    f: &PixelFormat,
    layout: pm_protocol::interop::DecodedLayout,
    color: ColorDescription,
) -> DecodedFrameDesc {
    DecodedFrameDesc {
        size: Size {
            width: f.width as u32,
            height: f.height as u32,
        },
        layout,
        // The frame's own range and depth override the source's description
        // where they are facts of the pixel format.
        color: ColorDescription {
            range: f.range,
            bit_depth: f.bits,
            subsampling: f.subsampling(),
            ..color
        },
        clean_aperture: None::<PixelRect>,
    }
}
