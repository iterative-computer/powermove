//! Exportable surfaces: IOSurfaces wrapped as Metal textures and adopted by
//! wgpu (contract 3, `export_surface`; plan section 18 item 2).

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceAllocSize, kIOSurfaceBytesPerElement,
    kIOSurfaceBytesPerRow, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};
use pm_protocol::common::Size;
use pm_protocol::interop::{SurfaceFormat, SurfaceRequest};

use super::BridgeOutError;

/// `kCVPixelFormatType_32BGRA` and `kCVPixelFormatType_64RGBAHalf`.
pub const FOURCC_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
pub const FOURCC_RGHA: u32 = u32::from_be_bytes(*b"RGhA");

/// A retained IOSurface. IOSurface references are reference counted
/// atomically and the IOSurface API is thread-safe, so a reference may move
/// between threads.
pub struct SharedIoSurface(pub CFRetained<IOSurfaceRef>);

// SAFETY: see the type comment; pixels are only touched under IOSurface locks
// (tests) or by the GPU.
unsafe impl Send for SharedIoSurface {}
// SAFETY: as above.
unsafe impl Sync for SharedIoSurface {}

impl SharedIoSurface {
    /// A send right for the surface (`IOSurfaceCreateMachPort`), to be moved
    /// to main through the bridge. `None` if the kernel refused.
    pub fn mach_port(&self) -> Option<u32> {
        let port = self.0.create_mach_port();
        (port != 0).then_some(port)
    }

    pub fn bytes_per_row(&self) -> u32 {
        self.0.bytes_per_row() as u32
    }

    pub fn alloc_size(&self) -> u64 {
        self.0.alloc_size() as u64
    }

    /// Reads the surface's bytes under a read-only CPU lock (tests and
    /// diagnostics; the GPU work that wrote it must have completed).
    pub fn read(&self) -> Vec<u8> {
        let io = &self.0;
        let mut seed = 0u32;
        // SAFETY: a read-only lock pairs with the unlock below.
        unsafe { io.lock(IOSurfaceLockOptions::ReadOnly, &mut seed) };
        let len = io.bytes_per_row() * io.height();
        // SAFETY: the locked base address covers bytes_per_row × height bytes.
        let bytes = unsafe {
            std::slice::from_raw_parts(io.base_address().as_ptr() as *const u8, len).to_vec()
        };
        // SAFETY: matches the lock above.
        unsafe { io.unlock(IOSurfaceLockOptions::ReadOnly, &mut seed) };
        bytes
    }
}

/// One exported surface: the IOSurface and the wgpu texture over it.
pub struct ExportedSurface {
    pub iosurface: SharedIoSurface,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: Size,
    pub format: SurfaceFormat,
}

/// The wgpu format of a viewer surface format.
pub fn wgpu_format(format: SurfaceFormat) -> Result<wgpu::TextureFormat, BridgeOutError> {
    match format {
        SurfaceFormat::Bgra8Unorm => Ok(wgpu::TextureFormat::Bgra8Unorm),
        SurfaceFormat::Rgba16Float => Ok(wgpu::TextureFormat::Rgba16Float),
        other => Err(BridgeOutError::Format(other)),
    }
}

fn metal_format(format: SurfaceFormat) -> Result<(MTLPixelFormat, u32), BridgeOutError> {
    match format {
        SurfaceFormat::Bgra8Unorm => Ok((MTLPixelFormat::BGRA8Unorm, FOURCC_BGRA)),
        SurfaceFormat::Rgba16Float => Ok((MTLPixelFormat::RGBA16Float, FOURCC_RGHA)),
        other => Err(BridgeOutError::Format(other)),
    }
}

impl ExportedSurface {
    /// Creates an IOSurface of the request's size and format and adopts it as
    /// a render-target texture on `device` (a Metal device).
    pub fn create(device: &wgpu::Device, request: &SurfaceRequest) -> Result<Self, BridgeOutError> {
        let Size { width, height } = request.size;
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(BridgeOutError::Size(request.size));
        }
        let (mtl_format, fourcc) = metal_format(request.format)?;
        let format = wgpu_format(request.format)?;
        let bpe = request.format.bytes_per_pixel() as usize;
        // SAFETY: the property key statics are exported by IOSurface.framework.
        let (k_w, k_h, k_bpe, k_bpr, k_size, k_fmt) = unsafe {
            (
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfaceBytesPerRow,
                kIOSurfaceAllocSize,
                kIOSurfacePixelFormat,
            )
        };
        let bpr = IOSurfaceRef::align_property(k_bpr, width as usize * bpe);
        let alloc = IOSurfaceRef::align_property(k_size, bpr * height as usize);
        let values = [
            CFNumber::new_i64(i64::from(width)),
            CFNumber::new_i64(i64::from(height)),
            CFNumber::new_i64(bpe as i64),
            CFNumber::new_i64(bpr as i64),
            CFNumber::new_i64(alloc as i64),
            CFNumber::new_i32(fourcc as i32),
        ];
        let keys: [&CFString; 6] = [k_w, k_h, k_bpe, k_bpr, k_size, k_fmt];
        let key_refs: Vec<&CFType> = keys.iter().map(|k| k.as_ref()).collect();
        let value_refs: Vec<&CFType> = values.iter().map(|v| v.as_ref()).collect();
        let props = CFDictionary::<CFType, CFType>::from_slices(&key_refs, &value_refs);
        // SAFETY: documented keys with CFNumber values.
        let iosurface = unsafe { IOSurfaceRef::new(props.as_opaque()) }
            .ok_or(BridgeOutError::Platform("IOSurfaceCreate failed"))?;

        let raw: Retained<ProtocolObject<dyn MTLTexture>> = {
            // SAFETY: the hal device outlives this block; the descriptor is
            // fully set before use.
            let hal = unsafe { device.as_hal::<wgpu_hal::api::Metal>() }
                .ok_or(BridgeOutError::Platform("not a Metal device"))?;
            let mtl = hal.raw_device();
            // SAFETY: a 2D, single-mip descriptor of a valid format and size.
            let desc = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    mtl_format,
                    width as usize,
                    height as usize,
                    false,
                )
            };
            desc.setUsage(
                MTLTextureUsage::RenderTarget
                    | MTLTextureUsage::ShaderRead
                    | MTLTextureUsage::ShaderWrite,
            );
            desc.setStorageMode(MTLStorageMode::Shared);
            mtl.newTextureWithDescriptor_iosurface_plane(&desc, &iosurface, 0)
                .ok_or(BridgeOutError::Platform(
                    "newTextureWithDescriptor:iosurface:plane: failed",
                ))?
        };
        // SAFETY: `raw` is a 2D, single-mip, single-layer texture of `format`.
        let hal_texture = unsafe {
            wgpu_hal::metal::Device::texture_from_raw(
                raw,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu_hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
                None,
            )
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("viewer pool surface"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        // SAFETY: the Metal texture was created with matching usage, format,
        // and size.
        let texture = unsafe {
            device.create_texture_from_hal::<wgpu_hal::api::Metal>(
                hal_texture,
                &descriptor,
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            iosurface: SharedIoSurface(iosurface),
            texture,
            view,
            size: request.size,
            format: request.format,
        })
    }
}
