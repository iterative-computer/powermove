//! IOSurface-backed render targets wrapped as wgpu textures (plan section 18, step 2).

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_surface::{
    IOSurfaceRef, kIOSurfaceAllocSize, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceHeight,
    kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage};
use s1_mach::rendezvous::SendRight;
use s1_mach::wire::{PixelFormat, SurfaceInfo};

/// `kCVPixelFormatType_32BGRA` and `kCVPixelFormatType_64RGBAHalf`.
const FOURCC_BGRA: i32 = i32::from_be_bytes(*b"BGRA");
const FOURCC_RGHA: i32 = i32::from_be_bytes(*b"RGhA");

/// IOSurface references are reference counted atomically and the IOSurface API is
/// thread-safe, so a retained reference may move between threads.
pub struct SharedIOSurface(pub CFRetained<IOSurfaceRef>);
// SAFETY: see the type comment; we never touch the surface's CPU memory.
unsafe impl Send for SharedIOSurface {}
// SAFETY: as above.
unsafe impl Sync for SharedIOSurface {}

pub struct Surface {
    pub iosurface: SharedIOSurface,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub info: SurfaceInfo,
}

impl Surface {
    pub fn create(device: &wgpu::Device, index: u32, width: u32, height: u32, format: PixelFormat) -> Result<Self, String> {
        let bpe = format.bytes_per_pixel() as usize;
        // SAFETY: the property key statics are exported by IOSurface.framework.
        let (k_w, k_h, k_bpe, k_bpr, k_size, k_fmt) = unsafe {
            (kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, kIOSurfaceAllocSize, kIOSurfacePixelFormat)
        };
        let bpr = IOSurfaceRef::align_property(k_bpr, width as usize * bpe);
        let size = IOSurfaceRef::align_property(k_size, bpr * height as usize);
        let fourcc = match format {
            PixelFormat::Bgra8 => FOURCC_BGRA,
            PixelFormat::Rgba16F => FOURCC_RGHA,
        };
        let values = [
            CFNumber::new_i64(width as i64),
            CFNumber::new_i64(height as i64),
            CFNumber::new_i64(bpe as i64),
            CFNumber::new_i64(bpr as i64),
            CFNumber::new_i64(size as i64),
            CFNumber::new_i32(fourcc),
        ];
        let keys: [&CFString; 6] = [k_w, k_h, k_bpe, k_bpr, k_size, k_fmt];
        let value_refs: Vec<&CFType> = values.iter().map(|v| v.as_ref()).collect();
        let key_refs: Vec<&CFType> = keys.iter().map(|k| k.as_ref()).collect();
        let props = CFDictionary::<CFType, CFType>::from_slices(&key_refs, &value_refs);
        // SAFETY: properties dictionary uses the documented keys and CFNumber values.
        let iosurface = unsafe { IOSurfaceRef::new(props.as_opaque()) }.ok_or("IOSurfaceCreate failed")?;

        let (mtl_format, wgpu_format) = match format {
            PixelFormat::Bgra8 => (MTLPixelFormat::BGRA8Unorm, wgpu::TextureFormat::Bgra8Unorm),
            PixelFormat::Rgba16F => (MTLPixelFormat::RGBA16Float, wgpu::TextureFormat::Rgba16Float),
        };
        let raw: Retained<ProtocolObject<dyn MTLTexture>> = {
            // SAFETY: the hal device outlives this call; the descriptor is fully set.
            let hal = unsafe { device.as_hal::<wgpu_hal::api::Metal>() }.ok_or("not a Metal device")?;
            let mtl = hal.raw_device();
            let desc = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    mtl_format,
                    width as usize,
                    height as usize,
                    false,
                )
            };
            desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
            desc.setStorageMode(MTLStorageMode::Shared);
            mtl.newTextureWithDescriptor_iosurface_plane(&desc, &iosurface, 0)
                .ok_or("newTextureWithDescriptor:iosurface:plane: failed")?
        };
        // SAFETY: raw is a 2D, single-mip, single-layer texture of the stated format.
        let hal_tex = unsafe {
            wgpu_hal::metal::Device::texture_from_raw(
                raw,
                wgpu_format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu_hal::CopyExtent { width, height, depth: 1 },
                None,
            )
        };
        let desc = wgpu::TextureDescriptor {
            label: Some("s1 pool surface"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        // SAFETY: the texture was created with matching usage, format, and size.
        let texture = unsafe { device.create_texture_from_hal::<wgpu_hal::api::Metal>(hal_tex, &desc, wgpu::TextureUses::UNINITIALIZED) };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let info = SurfaceInfo { pool_index: index, generation: 1, width, height, format, bytes_per_row: bpr as u32 };
        Ok(Self { iosurface: SharedIOSurface(iosurface), texture, view, info })
    }

    /// A send right for the IOSurface, to be moved to main.
    pub fn mach_port(&self) -> Result<SendRight, String> {
        let port = self.iosurface.0.create_mach_port();
        if port == 0 {
            return Err("IOSurfaceCreateMachPort returned MACH_PORT_NULL".into());
        }
        Ok(SendRight::from_raw(port))
    }
}
