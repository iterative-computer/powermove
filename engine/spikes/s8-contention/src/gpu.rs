//! wgpu device setup, GPU timestamps, and the Metal queue priority probe.

use std::collections::VecDeque;
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
        let hal = queue.as_hal::<wgpu::hal::api::Metal>().ok_or("not a Metal queue")?;
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

/// A log of GPU timestamp pairs (begin, end), one entry per command buffer or frame.
///
/// Entries are written by pass timestamp writes and resolved in batches by separate
/// small command buffers, never inside the measured command buffer: on Apple GPUs an
/// end-of-render-pass sample resolved in the same command buffer reads a stale value,
/// and a per-frame resolve-and-wait would itself perturb the measurement. On Apple
/// Silicon the values are host time in ns (the `mach_absolute_time` domain), so they
/// compare directly with [`crate::clock::now_ns`].
///
/// The log is a ring of `capacity` entries; an entry may be reused only after it has
/// been collected.
pub struct TsLog {
    sets: Vec<wgpu::QuerySet>,
    capacity: usize,
    period_ns: f64,
    /// Entries `< flushed` have a resolve submitted.
    flushed: usize,
    pending: VecDeque<Pending>,
    spare: Vec<wgpu::Buffer>,
    resolve: wgpu::Buffer,
    results: Vec<(usize, Option<(f64, f64)>)>,
}

struct Pending {
    first: usize,
    count: usize,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

const PER_SET: usize = (wgpu::QUERY_SET_MAX_QUERIES / 2) as usize;
const BATCH: usize = 256;

impl TsLog {
    pub fn new(gpu: &Gpu, capacity: usize) -> Option<Self> {
        if !gpu.timestamps {
            return None;
        }
        let capacity = capacity.div_ceil(PER_SET) * PER_SET;
        let sets = (0..capacity / PER_SET)
            .map(|_| {
                gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("ts"),
                    ty: wgpu::QueryType::Timestamp,
                    count: (PER_SET * 2) as u32,
                })
            })
            .collect();
        let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-resolve"),
            size: (BATCH * 16) as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Some(Self {
            sets,
            capacity,
            period_ns: gpu.ts_period_ns,
            flushed: 0,
            pending: VecDeque::new(),
            spare: Vec::new(),
            resolve,
            results: Vec::new(),
        })
    }

    fn locate(&self, entry: usize) -> (&wgpu::QuerySet, u32) {
        let e = entry % self.capacity;
        (&self.sets[e / PER_SET], ((e % PER_SET) * 2) as u32)
    }

    pub fn compute_writes(&self, entry: usize) -> wgpu::ComputePassTimestampWrites<'_> {
        let (set, q) = self.locate(entry);
        wgpu::ComputePassTimestampWrites {
            query_set: set,
            beginning_of_pass_write_index: Some(q),
            end_of_pass_write_index: Some(q + 1),
        }
    }

    /// Timestamp writes for a frame spanning several render passes: begin on the first
    /// pass, end on the last.
    pub fn render_writes(&self, entry: usize, begin: bool, end: bool) -> wgpu::RenderPassTimestampWrites<'_> {
        let (set, q) = self.locate(entry);
        wgpu::RenderPassTimestampWrites {
            query_set: set,
            beginning_of_pass_write_index: begin.then_some(q),
            end_of_pass_write_index: end.then_some(q + 1),
        }
    }

    /// Submit resolves for every entry below `upto` (whose command buffers must already
    /// be submitted; the resolves queue behind them).
    pub fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, upto: usize) {
        while self.flushed < upto {
            let first = self.flushed;
            let in_set = PER_SET - (first % self.capacity) % PER_SET;
            let count = (upto - first).min(BATCH).min(in_set);
            let buffer = self.spare.pop().unwrap_or_else(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ts-readback"),
                    size: (BATCH * 16) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            let (set, q) = self.locate(first);
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ts-resolve"),
            });
            enc.resolve_query_set(set, q..q + (count * 2) as u32, &self.resolve, 0);
            enc.copy_buffer_to_buffer(&self.resolve, 0, &buffer, 0, (count * 16) as u64);
            queue.submit([enc.finish()]);
            let state = Arc::new(AtomicU8::new(0));
            let s2 = state.clone();
            buffer.map_async(wgpu::MapMode::Read, ..(count * 16) as u64, move |r| {
                s2.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release)
            });
            self.pending.push_back(Pending {
                first,
                count,
                buffer,
                state,
            });
            self.flushed += count;
        }
    }

    /// Harvest finished resolves without blocking. Returns newly available entries.
    pub fn collect(&mut self, device: &wgpu::Device) -> Vec<(usize, Option<(f64, f64)>)> {
        let _ = device.poll(wgpu::PollType::Poll);
        let mut out = Vec::new();
        while let Some(p) = self.pending.front() {
            match p.state.load(Ordering::Acquire) {
                0 => break,
                st => {
                    let p = self.pending.pop_front().unwrap();
                    if st == 1 {
                        {
                            let data = p.buffer.get_mapped_range(..(p.count * 16) as u64).expect("mapped");
                            let v: &[u64] = bytemuck::cast_slice(&data);
                            for i in 0..p.count {
                                let (b, e) = (v[i * 2], v[i * 2 + 1]);
                                let ok = b != 0 && e >= b;
                                out.push((
                                    p.first + i,
                                    ok.then_some((b as f64 * self.period_ns, e as f64 * self.period_ns)),
                                ));
                            }
                        }
                        p.buffer.unmap();
                    } else {
                        for i in 0..p.count {
                            out.push((p.first + i, None));
                        }
                    }
                    self.spare.push(p.buffer);
                }
            }
        }
        self.results.extend(out.iter().copied());
        out
    }

    /// Flush everything below `upto` and block until all of it is collected.
    pub fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        upto: usize,
    ) -> Vec<(usize, Option<(f64, f64)>)> {
        self.flush(device, queue, upto);
        let start = Instant::now();
        while !self.pending.is_empty() {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            self.collect(device);
            if start.elapsed() > Duration::from_secs(30) {
                break;
            }
        }
        std::mem::take(&mut self.results)
    }
}
