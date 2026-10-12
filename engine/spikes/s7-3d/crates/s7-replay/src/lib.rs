//! Replays recorded three.js WebGL2 command streams on wgpu. Every program goes through
//! the S3-style GLSL pipeline (`s7-glsl`); GL state, objects, and draw semantics are
//! emulated for the subset three.js r180 uses (asserted, never silently ignored).

pub mod compare;
pub mod consts;
pub mod gl;
pub mod hand;
pub mod image;
pub mod trace;

pub use gl::{Frame, Replayer, ReplayStats};
pub use trace::Trace;

/// Creates a Metal device with the features the replay needs.
pub fn device() -> anyhow::Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))?;
    let limits = adapter.limits();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("s7"),
        required_features: wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::FLOAT32_BLENDABLE,
        required_limits: limits,
        ..Default::default()
    }))?;
    Ok((adapter, device, queue))
}
