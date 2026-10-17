//! The GPU watchdog for effects and transitions (plan section 8, "Device":
//! "Each effect and shader has a GPU timestamp budget; an overrun disables
//! it from the next frame and reports it").
//!
//! Every effect instance and transition of a frame is timed between two
//! marker dispatches, each a one-workgroup compute pass whose end-of-pass
//! sample is the timestamp (as `pm-export`'s `GpuSpan`: Metal's
//! start-of-encoder samples are not reliable, and pass-boundary samples of
//! render passes came back empty while several devices were busy): one
//! before the effect's first pass that reads a texel of its input, one after
//! its last pass that reads a texel of its output, so neither runs out of
//! order with the effect. The timestamps are resolved into a buffer in the frame's own command
//! buffer and read back without waiting ([`Watchdog::collect`], at the start
//! of the next frame); a definition whose time is over the budget is
//! disabled for every later frame (skipped with its data kept, like a
//! missing effect) and reported as `WatchdogTimeout` until it is registered
//! again. A true GPU hang is device loss, which the device's owner recovers
//! from; the watchdog is for the slow-but-finishing case.
//!
//! Devices without `TIMESTAMP_QUERY` time nothing.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pm_protocol::common::{DefinitionId, EffectInstanceId, LayerId};
use pm_protocol::definitions::DefinitionKind;
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope, Retry};

/// Default budget per effect instance per frame. The slowest built-in at
/// UHD (Glow, 400 px radius) takes a few milliseconds on the reference
/// machine; a quarter second is far past anything playable and still well
/// inside the operating system's GPU timeout.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(250);

/// Timed instances per frame (two queries each).
pub const CAPACITY: u32 = 64;

/// What one timed span belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub kind: DefinitionKind,
    pub definition: DefinitionId,
    pub layer: LayerId,
    /// The effect instance (`fx.id`); `None` for a transition.
    pub instance: Option<EffectInstanceId>,
}

/// One frame's timestamps in flight.
struct Pending {
    spans: Vec<Span>,
    /// `map_async` was called (after the frame was submitted).
    requested: bool,
    mapped: Arc<AtomicBool>,
}

/// The query set and buffers, allocated once per renderer: Metal's counter
/// sample buffers are a scarce resource, so one frame is timed at a time
/// (a frame encoded while the previous one's timestamps are unread is not
/// timed).
struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    marker: Marker,
}

/// A frame's query set while it is being encoded.
pub struct FrameTimer {
    pub(crate) set: wgpu::QuerySet,
    pub(crate) spans: Vec<Span>,
    marker: Marker,
}

/// The marker dispatch: reads one texel of a texture into a sink.
#[derive(Clone)]
struct Marker {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    sink: wgpu::Buffer,
}

const MARKER: &str = "
@group(0) @binding(0) var<storage, read_write> sink: array<f32>;
@group(0) @binding(1) var after: texture_2d<f32>;
@compute @workgroup_size(1)
fn mark() { sink[0] = sink[0] + textureLoad(after, vec2<u32>(0u, 0u), 0).a; }
";

impl Marker {
    fn new(device: &wgpu::Device) -> Marker {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm effect watchdog marker"),
            source: wgpu::ShaderSource::Wgsl(MARKER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm effect watchdog marker"),
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
            label: Some("pm effect watchdog marker"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pm effect watchdog marker"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("mark"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sink = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm effect watchdog sink"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Marker {
            pipeline,
            layout,
            sink,
        }
    }
}

impl FrameTimer {
    /// Query indices for the next span, or `None` when the set is full.
    pub(crate) fn begin(&mut self, span: Span) -> Option<(u32, u32)> {
        let index = self.spans.len() as u32;
        if index >= CAPACITY {
            return None;
        }
        self.spans.push(span);
        Some((index * 2, index * 2 + 1))
    }

    /// A marker dispatch reading `texture`, its end sampled into `query`.
    pub(crate) fn mark(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::TextureView,
        query: u32,
    ) {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm effect watchdog marker"),
            layout: &self.marker.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.marker.sink.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pm effect watchdog marker"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(query),
            }),
        });
        pass.set_pipeline(&self.marker.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

/// The watchdog state of a renderer.
pub struct Watchdog {
    budget: Duration,
    disabled: BTreeMap<(DefinitionKind, DefinitionId), f64>,
    pending: Option<Pending>,
    reports: Vec<EngineError>,
    /// The longest time measured per definition, in milliseconds.
    measured: BTreeMap<(DefinitionKind, DefinitionId), f64>,
    marker: Option<Marker>,
    timing: Option<Timing>,
    /// The newest valid timestamp read: a sample at or before it is a
    /// stale value Metal did not overwrite (unwritten samples happen while
    /// several devices are busy in one process).
    last_tick: u64,
}

impl Default for Watchdog {
    fn default() -> Self {
        Watchdog {
            budget: DEFAULT_BUDGET,
            disabled: BTreeMap::new(),
            pending: None,
            reports: Vec::new(),
            measured: BTreeMap::new(),
            marker: None,
            timing: None,
            last_tick: 0,
        }
    }
}

impl Watchdog {
    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    pub fn budget(&self) -> Duration {
        self.budget
    }

    pub fn is_disabled(&self, kind: DefinitionKind, id: &str) -> bool {
        self.disabled.contains_key(&(kind, DefinitionId::from(id)))
    }

    /// A re-registered definition starts over.
    pub fn forget(&mut self, kind: DefinitionKind, id: &DefinitionId) {
        self.disabled.remove(&(kind, id.clone()));
        self.measured.remove(&(kind, id.clone()));
    }

    /// Re-enables everything (the user asked to try again).
    pub fn reset(&mut self) {
        self.disabled.clear();
    }

    /// The longest GPU time measured for a definition, in milliseconds.
    pub fn measured_ms(&self, kind: DefinitionKind, id: &str) -> Option<f64> {
        self.measured.get(&(kind, DefinitionId::from(id))).copied()
    }

    /// Overruns reported since the last call.
    pub fn take_reports(&mut self) -> Vec<EngineError> {
        std::mem::take(&mut self.reports)
    }

    /// The timer for a frame, when the device can time passes and the
    /// previous timed frame has been read.
    pub(crate) fn frame_timer(&mut self, device: &wgpu::Device) -> Option<FrameTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) || self.pending.is_some() {
            return None;
        }
        let marker = self
            .marker
            .get_or_insert_with(|| Marker::new(device))
            .clone();
        let timing = self.timing.get_or_insert_with(|| {
            let bytes = u64::from(CAPACITY) * 2 * 8;
            Timing {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("pm effect watchdog"),
                    ty: wgpu::QueryType::Timestamp,
                    count: CAPACITY * 2,
                }),
                resolve: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pm effect watchdog resolve"),
                    size: bytes,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pm effect watchdog readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                marker: marker.clone(),
            }
        });
        Some(FrameTimer {
            set: timing.set.clone(),
            spans: Vec::new(),
            marker: timing.marker.clone(),
        })
    }

    /// Resolves a frame's timestamps in its command buffer; the readback is
    /// mapped after [`Watchdog::submitted`].
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder, timer: FrameTimer) {
        let Some(timing) = &self.timing else { return };
        if timer.spans.is_empty() {
            return;
        }
        let count = timer.spans.len() as u32 * 2;
        let bytes = u64::from(count) * 8;
        encoder.resolve_query_set(&timing.set, 0..count, &timing.resolve, 0);
        encoder.copy_buffer_to_buffer(&timing.resolve, 0, &timing.readback, 0, bytes);
        self.pending = Some(Pending {
            spans: timer.spans,
            requested: false,
            mapped: Arc::new(AtomicBool::new(false)),
        });
    }

    /// Starts mapping the readback of the frame just submitted.
    pub(crate) fn submitted(&mut self) {
        let (Some(pending), Some(timing)) = (&mut self.pending, &self.timing) else {
            return;
        };
        if pending.requested {
            return;
        }
        pending.requested = true;
        let done = pending.mapped.clone();
        timing
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    done.store(true, Ordering::Release);
                }
            });
    }

    /// Reads the timed frame's timestamps when they are ready; `wait`
    /// blocks until they are (tests). Overruns disable their definitions
    /// and are reported.
    pub fn collect(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, wait: bool) {
        if self.pending.as_ref().is_none_or(|p| !p.requested) {
            return;
        }
        let poll = if wait {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        let _ = device.poll(poll);
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.mapped.load(Ordering::Acquire))
        {
            return;
        }
        let pending = self.pending.take().expect("checked");
        let timing = self.timing.as_ref().expect("a pending frame has timing");
        let period = f64::from(queue.get_timestamp_period());
        let ticks: Vec<u64> = {
            let data = timing
                .readback
                .slice(..)
                .get_mapped_range()
                .expect("mapped readback");
            data.chunks_exact(8)
                .take(pending.spans.len() * 2)
                .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
                .collect()
        };
        timing.readback.unmap();
        let floor = self.last_tick;
        for (i, span) in pending.spans.iter().enumerate() {
            let (begin, end) = (ticks[i * 2], ticks[i * 2 + 1]);
            if begin <= floor || end <= begin {
                // Not written (or left from an earlier frame): Metal left a
                // sample empty. The frame is not judged; a slow effect is
                // caught on its next measured frame.
                continue;
            }
            self.last_tick = self.last_tick.max(end);
            let ms = (end - begin) as f64 * period / 1.0e6;
            self.record(span, ms);
        }
    }

    fn record(&mut self, span: &Span, ms: f64) {
        let key = (span.kind, span.definition.clone());
        let longest = self.measured.entry(key.clone()).or_insert(0.0);
        *longest = longest.max(ms);
        if ms <= self.budget.as_secs_f64() * 1000.0 || self.disabled.contains_key(&key) {
            return;
        }
        self.disabled.insert(key, ms);
        let scope = match &span.instance {
            Some(effect) => ErrorScope::Effect {
                layer: span.layer.clone(),
                effect: effect.clone(),
                definition: span.definition.clone(),
            },
            None => ErrorScope::Definition {
                definition: span.definition.clone(),
            },
        };
        let message = format!(
            "{} took {ms:.1} ms of GPU time in one frame (budget {} ms) and is turned off until it is registered or reloaded again.",
            span.definition.as_str(),
            self.budget.as_millis()
        );
        self.reports.push(
            EngineError::new(ErrorCode::WatchdogTimeout, scope, message)
                .with_retry(Retry::AfterUserAction),
        );
    }
}
