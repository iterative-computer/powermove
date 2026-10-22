//! GPU time of a sampled viewer render, read without blocking the presenter.
//!
//! The job engine measured each admitted submission from `queue.submit` to
//! completion. While the interactive engine plays, that wall time includes
//! the wait behind its frames, so a 0.3 ms band could measure 4 ms and be
//! taken for an over-bound pass. [`GpuSpan`] brackets a frame's commands,
//! inside the frame's own command buffer (`pm_render::CommandBracket`), with
//! two one-workgroup compute dispatches whose passes write timestamps
//! (Metal samples at encoder boundaries and may skip an empty encoder, hence
//! the dispatch). Both take end-of-encoder samples (Metal's start-of-encoder
//! samples were not reliable): the first as the GPU starts the frame; the
//! second dispatch reads one texel of the frame's output, so it cannot run
//! before the frame has written it, and its end is sampled after. The span
//! leaves out the queue wait and, being in the frame's command buffer, the
//! time the CPU spent encoding the frame. Other processes' work that the GPU interleaves inside
//! the span still counts; the span is an upper bound of the work's own
//! cost.
//!
//! Devices without `TIMESTAMP_QUERY` get no timer. No wall-time surrogate is
//! fed to the machine profile.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

type Sink = Arc<dyn Fn(Duration) + Send + Sync>;
const SAMPLE_EVERY: u64 = 16;
const SLOTS: usize = 3;

/// Three reusable slots, each with its own query and readback buffer. A slot
/// remains busy until its device-poll callback has unmapped the buffer.
pub(super) struct GpuMeter {
    slots: [Arc<GpuSpan>; SLOTS],
    busy: Arc<AtomicU8>,
    sink: Sink,
    frames: u64,
    interval: u64,
}

pub(super) struct Sample {
    slot: usize,
    span: Arc<GpuSpan>,
}

impl Sample {
    pub(super) fn bracket(&self) -> Arc<dyn pm_render::CommandBracket> {
        self.span.clone()
    }
}

impl GpuMeter {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, sink: Sink) -> Option<Self> {
        Some(Self {
            slots: [
                Arc::new(GpuSpan::new(device, queue)?),
                Arc::new(GpuSpan::new(device, queue)?),
                Arc::new(GpuSpan::new(device, queue)?),
            ],
            busy: Arc::new(AtomicU8::new(0)),
            sink,
            frames: 0,
            interval: SAMPLE_EVERY,
        })
    }

    pub(super) fn begin(&mut self) -> Option<Sample> {
        self.frames = self.frames.wrapping_add(1);
        if !self.frames.is_multiple_of(self.interval) {
            return None;
        }
        for slot in 0..SLOTS {
            let bit = 1 << slot;
            if self
                .busy
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |busy| {
                    (busy & bit == 0).then_some(busy | bit)
                })
                .is_ok()
            {
                return Some(Sample {
                    slot,
                    span: self.slots[slot].clone(),
                });
            }
        }
        None
    }

    pub(super) fn end(&self, queue: &wgpu::Queue, sample: Sample) {
        sample.span.resolve_async(
            queue,
            self.sink.clone(),
            self.busy.clone(),
            1 << sample.slot,
        );
    }

    pub(super) fn abandon(&self, sample: Sample) {
        self.busy.fetch_and(!(1 << sample.slot), Ordering::AcqRel);
    }
}

/// Two timestamps around a frame's commands.
struct GpuSpan {
    device: wgpu::Device,
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    begin: wgpu::ComputePipeline,
    end: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    sink: wgpu::Buffer,
    /// A texture for `begin`'s bind group (it reads nothing).
    blank: wgpu::TextureView,
    period_ns: f64,
}

const SHADER: &str = "
@group(0) @binding(0) var<storage, read_write> sink: array<f32>;
@group(0) @binding(1) var after: texture_2d<f32>;
@compute @workgroup_size(1)
fn begin() { sink[0] = sink[0] + 1.0; }
@compute @workgroup_size(1)
fn end() { sink[1] = textureLoad(after, vec2<u32>(0u, 0u), 0).a; }
";

impl GpuSpan {
    /// `None` when the device cannot write pass timestamps.
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuSpan> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let period_ns = f64::from(queue.get_timestamp_period());
        if !(period_ns.is_finite() && period_ns > 0.0) {
            return None;
        }
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pm viewer gpu span"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm viewer gpu span resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm viewer gpu span read"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sink = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm viewer gpu span sink"),
            size: 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm viewer gpu span"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm viewer gpu span"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm viewer gpu span"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm viewer gpu span"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("pm viewer gpu span blank"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Some(GpuSpan {
            device: device.clone(),
            set,
            resolve,
            read,
            begin: pipeline("begin"),
            end: pipeline("end"),
            layout,
            sink,
            blank,
            period_ns,
        })
    }

    fn mark(&self, encoder: &mut wgpu::CommandEncoder, after: Option<&wgpu::TextureView>) {
        let end = after.is_some();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm viewer gpu span"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.sink.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(after.unwrap_or(&self.blank)),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm viewer gpu span"),
            // End-of-encoder samples only: Metal's start-of-encoder samples
            // were not reliable.
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(u32::from(end)),
            }),
        });
        pass.set_pipeline(if end { &self.end } else { &self.begin });
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// The renderer has submitted its command buffer. Resolve and map in a
    /// later submission; the existing completion-thread device poll invokes
    /// the callback after the GPU writes the timestamps.
    fn resolve_async(
        self: &Arc<Self>,
        queue: &wgpu::Queue,
        sink: Sink,
        busy: Arc<AtomicU8>,
        bit: u8,
    ) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm viewer gpu span resolve"),
            });
        encoder.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, 16);
        queue.submit([encoder.finish()]);
        let span = self.clone();
        self.read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    if let Ok(data) = span.read.slice(..).get_mapped_range() {
                        let t = |i: usize| {
                            u64::from_le_bytes(
                                data[i * 8..i * 8 + 8].try_into().expect("timestamp size"),
                            )
                        };
                        let (t0, t1) = (t(0), t(1));
                        drop(data);
                        span.read.unmap();
                        if t1 > t0 && t0 > 0 {
                            let seconds = (t1 - t0) as f64 * span.period_ns * 1e-9;
                            if seconds.is_finite() && seconds > 0.0 && seconds < 2.0 {
                                sink(Duration::from_secs_f64(seconds));
                            }
                        }
                    } else {
                        span.read.unmap();
                    }
                }
                busy.fetch_and(!bit, Ordering::AcqRel);
            });
    }
}

impl pm_render::CommandBracket for GpuSpan {
    fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        self.mark(encoder, None);
    }

    fn end(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
        self.mark(encoder, Some(output));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn real_gpu_timestamp_is_positive_when_supported() {
        let gpu = match pm_gpu::Gpu::new("pm viewer gpu timestamp test") {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("GPU timestamp test unavailable: {error}");
                return;
            }
        };
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let observed = observed.clone();
            Arc::new(move |duration| observed.lock().unwrap().push(duration))
        };
        let Some(mut meter) = GpuMeter::new(&gpu.device, &gpu.queue, sink) else {
            eprintln!("GPU timestamp feature unavailable");
            return;
        };
        meter.interval = 1;
        let sample = meter.begin().expect("free sample slot");
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm viewer gpu timestamp test output"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm viewer gpu timestamp test"),
            });
        let bracket = sample.bracket();
        bracket.begin(&mut encoder);
        bracket.end(&mut encoder, &view);
        gpu.queue.submit([encoder.finish()]);
        meter.end(&gpu.queue, sample);
        gpu.wait_idle();
        let durations = observed.lock().unwrap();
        assert_eq!(durations.len(), 1);
        assert!(durations[0] > Duration::ZERO);
        eprintln!("GPU timestamp sample: {:?}", durations[0]);
        assert_eq!(meter.busy.load(Ordering::Acquire), 0);
    }
}
