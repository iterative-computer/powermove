//! The engine's wgpu device (plan section 8, "Device").
//!
//! One [`Gpu`] per engine process: a Metal device on the high-performance
//! adapter with the features the render graph needs. `rgba32float`
//! filtering and blending (32-bit precision) are requested when the adapter
//! has them (S2: every reference machine reports both); without them
//! 32-bit precision is unavailable and [`Gpu::supports_precision`] says so.

use pm_protocol::color::Precision;
use pm_protocol::interop::AdapterIdentity;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("no Metal adapter: {0}")]
    Adapter(String),
    #[error("device request failed: {0}")]
    Device(String),
    #[error("the adapter lacks {0}")]
    Missing(&'static str),
}

/// A wgpu device and queue with what the engine learned about them.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// The Metal `registryID`, used to match Chromium's GPU process.
    pub registry_id: Option<u64>,
    pub limits: wgpu::Limits,
}

/// Features the render graph needs on every device.
pub fn required_features() -> wgpu::Features {
    // R16/RG16/RGBA16 unorm planes from decoders (decode-in).
    wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}

/// Features the render graph uses when present: `rgba32float` filtering
/// and blending (32-bit precision), passthrough shaders (Metal's safe
/// math mode for color-critical programs, `crate::msl`), and pass
/// timestamps (the job engine's GPU time per submission, contract 13).
pub fn optional_features() -> wgpu::Features {
    wgpu::Features::FLOAT32_FILTERABLE
        | wgpu::Features::FLOAT32_BLENDABLE
        | wgpu::Features::PASSTHROUGH_SHADERS
        | wgpu::Features::TIMESTAMP_QUERY
}

impl Gpu {
    /// A device on the high-performance Metal adapter.
    pub fn new(label: &str) -> Result<Gpu, DeviceError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| DeviceError::Adapter(e.to_string()))?;
        Self::from_adapter(&adapter, label)
    }

    /// A device on a given adapter.
    pub fn from_adapter(adapter: &wgpu::Adapter, label: &str) -> Result<Gpu, DeviceError> {
        let available = adapter.features();
        if !available.contains(required_features()) {
            return Err(DeviceError::Missing("TEXTURE_FORMAT_16BIT_NORM"));
        }
        let features = required_features() | (available & optional_features());
        let adapter_limits = adapter.limits();
        let limits = wgpu::Limits {
            max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d.min(16384),
            // Studio's path tracer (pm-studio) binds 10 storage buffers per
            // stage and its scene buffers can be large.
            max_storage_buffers_per_shader_stage: adapter_limits.max_storage_buffers_per_shader_stage.min(12),
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            max_buffer_size: adapter_limits.max_buffer_size,
            max_color_attachment_bytes_per_sample: adapter_limits
                .max_color_attachment_bytes_per_sample,
            ..wgpu::Limits::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: features,
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .map_err(|e| DeviceError::Device(e.to_string()))?;
        let registry_id = metal_registry_id(&device);
        Ok(Gpu {
            device,
            queue,
            info: adapter.get_info(),
            registry_id,
            limits,
        })
    }

    /// Whether intermediates can be stored at `precision`: `rgba32float`
    /// needs filtering (resampling layers) and blending (compositing).
    pub fn supports_precision(&self, precision: Precision) -> bool {
        match precision {
            Precision::Unorm8 | Precision::Float16 => true,
            Precision::Float32 => self
                .device
                .features()
                .contains(wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::FLOAT32_BLENDABLE),
        }
    }

    /// The adapter identity of contract 3.
    pub fn adapter_identity(&self) -> AdapterIdentity {
        match self.registry_id {
            Some(id) => AdapterIdentity::Metal {
                registry_id: id.to_string(),
                name: self.info.name.clone(),
            },
            None => AdapterIdentity::Software,
        }
    }

    /// Waits until every submitted command buffer has completed.
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

#[cfg(target_os = "macos")]
fn metal_registry_id(device: &wgpu::Device) -> Option<u64> {
    use objc2_metal::MTLDevice;
    // SAFETY: the raw device is only read for its registry id.
    unsafe {
        device
            .as_hal::<wgpu_hal::api::Metal>()
            .map(|hal| hal.raw_device().registryID())
    }
}

#[cfg(not(target_os = "macos"))]
fn metal_registry_id(_device: &wgpu::Device) -> Option<u64> {
    None
}

/// The bytes the Metal device has allocated for this process's buffers,
/// textures and heaps (`MTLDevice.currentAllocatedSize`): the driver
/// memory that `footprint` charges as owned graphics. Imported IOSurfaces
/// count where they are allocated. `None` off Metal.
#[cfg(target_os = "macos")]
pub fn allocated_bytes(device: &wgpu::Device) -> Option<u64> {
    use objc2_metal::MTLDevice;
    // SAFETY: a read-only property of the raw device.
    unsafe {
        device
            .as_hal::<wgpu_hal::api::Metal>()
            .map(|hal| hal.raw_device().currentAllocatedSize() as u64)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn allocated_bytes(_device: &wgpu::Device) -> Option<u64> {
    None
}

/// Public Metal device counters. `current_allocated_bytes` is the driver's
/// allocation total for this device, not the process's resident footprint.
/// The other values describe the adapter and are not usage counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetalDeviceAllocation {
    pub current_allocated_bytes: u64,
    pub recommended_max_working_set_bytes: Option<u64>,
    pub unified_memory: Option<bool>,
    pub max_transfer_bytes_per_second: Option<u64>,
}

#[cfg(target_os = "macos")]
pub fn metal_device_allocation(device: &wgpu::Device) -> Option<MetalDeviceAllocation> {
    use objc2::runtime::NSObjectProtocol;
    use objc2_metal::MTLDevice;

    // SAFETY: the HAL device is only queried through public, read-only Metal
    // selectors. Optional selectors are checked for older macOS releases.
    unsafe {
        device.as_hal::<wgpu_hal::api::Metal>().map(|hal| {
            let metal = hal.raw_device();
            MetalDeviceAllocation {
                current_allocated_bytes: metal.currentAllocatedSize() as u64,
                recommended_max_working_set_bytes: metal
                    .respondsToSelector(objc2::sel!(recommendedMaxWorkingSetSize))
                    .then(|| metal.recommendedMaxWorkingSetSize()),
                unified_memory: metal
                    .respondsToSelector(objc2::sel!(hasUnifiedMemory))
                    .then(|| metal.hasUnifiedMemory()),
                max_transfer_bytes_per_second: metal
                    .respondsToSelector(objc2::sel!(maxTransferRate))
                    .then(|| metal.maxTransferRate()),
            }
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn metal_device_allocation(_device: &wgpu::Device) -> Option<MetalDeviceAllocation> {
    None
}

/// The size of one surveyed Metal texture. Heap-backed resource sizes may
/// overlap a heap's allocation; never sum them with device or IOSurface bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetalTextureAllocation {
    pub allocated_bytes: u64,
    pub heap_backed: bool,
}

#[cfg(target_os = "macos")]
pub fn metal_texture_allocation(texture: &wgpu::Texture) -> Option<MetalTextureAllocation> {
    use objc2_metal::MTLResource;

    // SAFETY: this live wgpu texture is only inspected through public,
    // read-only properties of its Metal resource.
    unsafe {
        texture
            .as_hal::<wgpu_hal::api::Metal>()
            .map(|hal| MetalTextureAllocation {
                allocated_bytes: hal.raw_handle().allocatedSize() as u64,
                heap_backed: hal.raw_handle().heap().is_some(),
            })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn metal_texture_allocation(_texture: &wgpu::Texture) -> Option<MetalTextureAllocation> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn metal_inventory_reads_live_device_and_texture_resources() {
        let gpu = Gpu::new("metal inventory test").unwrap();
        let device = metal_device_allocation(&gpu.device).unwrap();
        assert_eq!(
            device.current_allocated_bytes,
            allocated_bytes(&gpu.device).unwrap()
        );
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metal inventory texture"),
            size: wgpu::Extent3d {
                width: 17,
                height: 19,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let resource = metal_texture_allocation(&texture).unwrap();
        assert!(resource.allocated_bytes >= 17 * 19 * 4);
    }
}
