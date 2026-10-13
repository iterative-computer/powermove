//! Bridge-out: the export half of the interop trait (contract 3,
//! `export_surface`) and the viewer's display transform (plan sections 9
//! and 18, amendment 4).
//!
//! - [`surface`]: IOSurfaces created per pool and adopted by wgpu as render
//!   targets; each is sent to main once through `pm-bridge`.
//! - [`pool`]: the pool generation, its size (ring + jitter buffer + 2), and
//!   the format choice: `rgbaf16` on EDR displays, `bgra` elsewhere.
//! - [`display`]: working values to the pool's format: P3 conversion on a
//!   `display-p3` canvas, extended values kept for `rgbaf16`, and an 8-bit
//!   quantization with a 64 × 64 blue-noise dither for `bgra`.
//! - [`test_source`]: the S1 test pattern and cube field, presented until
//!   pm-render draws real frames.
//!
//! These are standalone types rather than a `GpuInterop` impl: the decode
//! half lives in `decode_in` (media lane), and the trait impl delegates to
//! both once they share one device type (see docs/engine/bridge.md).

pub mod display;
mod noise;
pub mod pool;
pub mod remote_layer;
pub mod surface;
pub mod test_source;

pub use noise::void_and_cluster;

use pm_protocol::common::Size;
use pm_protocol::interop::SurfaceFormat;

#[derive(Debug, thiserror::Error)]
pub enum BridgeOutError {
    #[error("{0:?} is not a viewer surface format")]
    Format(SurfaceFormat),
    #[error("surface size {0:?} is outside 1..16384")]
    Size(Size),
    #[error("a pool has 4 to 32 surfaces, not {0}")]
    PoolSize(u32),
    #[error("{0}")]
    Platform(&'static str),
    #[error("{0}")]
    Device(String),
}

/// A Metal device and queue on the high-performance adapter, for the
/// interactive engine's viewer frames and for tests.
pub fn metal_device(
    label: &str,
) -> Result<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo), BridgeOutError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|e| BridgeOutError::Device(format!("request_adapter: {e}")))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: 16384,
            ..wgpu::Limits::default()
        },
        ..Default::default()
    }))
    .map_err(|e| BridgeOutError::Device(format!("request_device: {e}")))?;
    Ok((device, queue, info))
}
