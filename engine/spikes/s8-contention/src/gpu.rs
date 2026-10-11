//! wgpu device setup, GPU timestamps, and the Metal queue priority probe.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub timestamps: bool,
    /// Nanoseconds per timestamp tick.
    pub ts_period_ns: f64,
    pub adapter: String,
}

/// Create a Metal device on the high-performance adapter. Each process calls this, so
/// A and B own separate `MTLDevice` connections and separate command queues.
pub fn create(label: &str) -> Gpu {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .expect("no Metal adapter");
    let info = adapter.get_info();
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let mut required_features = wgpu::Features::empty();
    if timestamps {
        required_features |= wgpu::Features::TIMESTAMP_QUERY;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features,
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: 8192,
            ..wgpu::Limits::default()
        },
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .expect("request_device failed");
    device.on_uncaptured_error(Arc::new(|e| panic!("wgpu error: {e}")));
    let ts_period_ns = queue.get_timestamp_period() as f64;
    Gpu {
        device,
        queue,
        timestamps,
        ts_period_ns,
        adapter: format!("{} ({:?}, {})", info.name, info.backend, info.driver_info),
    }
}

/// Private Metal API: `-[IOGPUMetalCommandQueue setGPUPriority:]`. Foreground values
/// 0..=2 (default 1; the background default is 2). Metal asserts and aborts on other
/// values, so they are rejected here. Used only as a measured experiment (S8 report);
/// private API use is a product decision, not something the spike adopts.
pub fn set_private_gpu_priority(queue: &wgpu::Queue, priority: usize) -> Result<usize, String> {
    use objc2::{msg_send, sel};
    if priority > 2 {
        return Err(format!("priority {priority} outside 0..=2"));
    }
    // SAFETY: the raw queue outlives this call; selectors are checked before use.
    unsafe {
        let hal = queue
            .as_hal::<wgpu::hal::api::Metal>()
            .ok_or("not a Metal queue")?;
        let raw = hal.as_raw();
        let ok: bool = msg_send![raw, respondsToSelector: sel!(setGPUPriority:)];
        let ok_get: bool = msg_send![raw, respondsToSelector: sel!(getGPUPriority)];
        if !ok || !ok_get {
            return Err("MTLCommandQueue does not respond to setGPUPriority:".into());
        }
        let set: bool = msg_send![raw, setGPUPriority: priority];
        let now: usize = msg_send![raw, getGPUPriority];
        if !set || now != priority {
            return Err(format!("setGPUPriority:{priority} returned {set}, now {now}"));
        }
        Ok(now)
    }
}

pub fn get_private_gpu_priority(queue: &wgpu::Queue) -> Option<usize> {
    use objc2::{msg_send, sel};
    // SAFETY: as above.
    unsafe {
        let hal = queue.as_hal::<wgpu::hal::api::Metal>()?;
        let raw = hal.as_raw();
        let ok: bool = msg_send![raw, respondsToSelector: sel!(getGPUPriority)];
        if !ok {
            return None;
        }
        let p: usize = msg_send![raw, getGPUPriority];
        Some(p)
    }
}

/// Two timestamps (begin, end) per slot, resolved into per-slot readback buffers so a
/// slot can be read while later slots are in flight.
pub struct Timestamps {
    pub set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: Vec<wgpu::Buffer>,
    period_ns: f64,
}

impl Timestamps {
    pub fn new(gpu: &Gpu, slots: u32) -> Option<Self> {
        if !gpu.timestamps {
            return None;
        }
        let set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("ts"),
            ty: wgpu::QueryType::Timestamp,
            count: slots * 2,
        });
        let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-resolve"),
            size: slots as u64 * 256,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = (0..slots)
            .map(|_| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ts-readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Some(Self { set, resolve, readback, period_ns: gpu.ts_period_ns })
    }

    pub fn begin(&self, slot: u32) -> u32 {
        slot * 2
    }

    pub fn end(&self, slot: u32) -> u32 {
        slot * 2 + 1
    }

    pub fn compute_writes(&self, slot: u32, begin: bool, end: bool) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: begin.then(|| self.begin(slot)),
            end_of_pass_write_index: end.then(|| self.end(slot)),
        }
    }

    pub fn render_writes(&self, slot: u32, begin: bool, end: bool) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: begin.then(|| self.begin(slot)),
            end_of_pass_write_index: end.then(|| self.end(slot)),
        }
    }

    pub fn resolve(&self, enc: &mut wgpu::CommandEncoder, slot: u32) {
        let off = slot as u64 * 256;
        enc.resolve_query_set(&self.set, slot * 2..slot * 2 + 2, &self.resolve, off);
        enc.copy_buffer_to_buffer(&self.resolve, off, &self.readback[slot as usize], 0, 16);
    }

    /// Read a completed slot's GPU duration in nanoseconds. The slot's submission must
    /// have completed. Polls the device without blocking on other submissions.
    pub fn read_ns(&self, device: &wgpu::Device, slot: u32) -> Option<f64> {
        let buf = &self.readback[slot as usize];
        let state = Arc::new(AtomicU8::new(0));
        let s2 = state.clone();
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            s2.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release)
        });
        let start = Instant::now();
        while state.load(Ordering::Acquire) == 0 {
            let _ = device.poll(wgpu::PollType::Poll);
            if state.load(Ordering::Acquire) != 0 {
                break;
            }
            if start.elapsed() > Duration::from_secs(2) {
                return None;
            }
            std::thread::sleep(Duration::from_micros(50));
        }
        if state.load(Ordering::Acquire) != 1 {
            return None;
        }
        let v = {
            let Ok(data) = buf.get_mapped_range(..) else {
                buf.unmap();
                return None;
            };
            let b: &[u64] = bytemuck::cast_slice(&data[..16]);
            (b[0], b[1])
        };
        buf.unmap();
        if v.1 < v.0 || v.0 == 0 {
            return None;
        }
        Some((v.1 - v.0) as f64 * self.period_ns)
    }
}
