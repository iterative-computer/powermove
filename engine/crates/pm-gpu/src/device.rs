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
/// and blending (32-bit precision), and passthrough shaders (Metal's safe
/// math mode for color-critical programs, `crate::msl`).
pub fn optional_features() -> wgpu::Features {
    wgpu::Features::FLOAT32_FILTERABLE
        | wgpu::Features::FLOAT32_BLENDABLE
        | wgpu::Features::PASSTHROUGH_SHADERS
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
            Precision::Float16 => true,
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
