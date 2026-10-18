//! The GPU watchdog for effects, transitions, and shader-layer programs
//! (plan section 8, "Device": "Each effect and shader has a GPU timestamp
//! budget; an overrun disables it from the next frame and reports it";
//! issue A2).
//!
//! Two guards, each judged when a frame's commands have completed (nothing
//! waits during rendering: [`Watchdog::collect`] runs at the start of later
//! frames):
//!
//! - **The budget** (interactive renderers): every effect instance,
//!   transition, and shader or extension fragment layer of a timed frame is
//!   timed between two marker dispatches, each a one-workgroup compute pass
//!   whose end-of-pass sample is the timestamp (as `pm-export`'s `GpuSpan`:
//!   Metal's start-of-encoder samples are not reliable, and pass-boundary
//!   samples of render passes came back empty while several devices were
//!   busy): one before the program's first pass reading a texel of its
//!   input, one after its last pass reading a texel of its output. The
//!   timestamps are resolved in a submission of their own once the frame has
//!   completed (resolved in the frame's own command buffer, Metal returned
//!   the previous frame's end sample). A program over the budget is disabled
//!   for every later frame (skipped with its data kept, like a missing
//!   effect) and reported as `WatchdogTimeout` until it is registered again
//!   (a shader layer: until its code changes).
//! - **The hang guard** (every renderer, every frame): a frame whose
//!   commands have not completed within the hang budget of wall time after
//!   submission (a runaway loop: the GPU runs it until the operating system's
//!   GPU timeout aborts the command buffer) disables every program it drew
//!   that is not a built-in, at once, so it is never submitted again, and
//!   reports each. Built-in programs cannot run away and are kept.
//!
//! A timestamp sample is trusted only when it is plausible: written (after
//! the newest trusted sample) and spanning no more than the frame's own
//! submission-to-completion wall time. A sample Metal left unwritten, or a
//! stale or garbage value, is not judged and never becomes the floor later
//! samples are compared with (before 2026-10-18 one garbage end sample
//! raised the floor past every later sample, so the watchdog stopped
//! judging for the rest of the process).
//!
//! **Policies.** An interactive renderer skips a disabled program and keeps
//! drawing ([`Policy::Interactive`]). A job renderer ([`Policy::Job`]: export
//! jobs, agent stills) never skips a program: it applies no per-program
//! budget (today's exporter renders a slow effect slowly) and, after a hang,
//! fails every later frame with a named error until [`Watchdog::reset`]
//! (the job engine resets it per job), so nothing is delivered without the
//! program the project asks for.
//!
//! **The test hook.** [`Watchdog::set_timing_source`] replaces the GPU
//! timestamps and the wall clock with a [`TimingSource`]: every frame is then
//! judged at its completion with the times the source gives, so the budget,
//! the hang guard, and every re-enable rule are tested deterministically on
//! any device, under any load. Production renderers never set one.
//!
//! Devices without `TIMESTAMP_QUERY` apply only the hang guard.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pm_protocol::common::{DefinitionId, EffectInstanceId, LayerId};
use pm_protocol::definitions::DefinitionKind;
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope, Retry};

/// Default budget per program per frame. The slowest built-in at UHD
/// (Gaussian Blur) takes about 15 ms on the reference machine; a quarter
/// second is far past anything playable and still well inside the operating
/// system's GPU timeout.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(250);

/// Default hang budget of an interactive renderer: a frame not done this
/// long after submission is a runaway.
pub const DEFAULT_HANG: Duration = Duration::from_secs(2);

/// Hang budget of a job renderer: Chromium's own GPU watchdog lets a WebGL
/// frame run this long before it loses the context, so an export that
/// finishes today finishes here.
pub const JOB_HANG: Duration = Duration::from_secs(10);

/// Timed programs per frame (two queries each).
pub const CAPACITY: u32 = 64;

/// Frames whose completion the hang guard follows at once; older ones are
/// judged as they complete, in order.
const INFLIGHT: usize = 8;

/// Disabled shader-layer programs remembered (by source key).
const DISABLED_CODE: usize = 1024;

/// How a renderer acts on what the watchdog finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// The viewer: the budget and the hang guard; a disabled program is
    /// skipped with its data kept and reported.
    #[default]
    Interactive,
    /// Export jobs and agent stills: no per-program budget; after a hang
    /// every later frame fails with a named error until [`Watchdog::reset`].
    Job,
}

/// What a timed program is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    /// An effect, transition, or extension fragment layer definition.
    Definition {
        kind: DefinitionKind,
        id: DefinitionId,
    },
    /// A shader layer's code (document content), by its source key.
    Code(pm_glsl::Key),
}

/// One program drawn in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub subject: Subject,
    pub layer: LayerId,
    /// The effect instance (`fx.id`); `None` for a transition or a layer.
    pub instance: Option<EffectInstanceId>,
    /// A built-in program (origin `builtIn`): the hang guard keeps it.
    pub builtin: bool,
}

impl Span {
    /// An effect instance's or a transition's span.
    pub fn definition(
        kind: DefinitionKind,
        definition: &str,
        layer: &str,
        instance: Option<&str>,
        builtin: bool,
    ) -> Span {
        Span {
            subject: Subject::Definition {
                kind,
                id: DefinitionId::from(definition),
            },
            layer: LayerId::from(layer),
            instance: instance.map(EffectInstanceId::from),
            builtin,
        }
    }

    /// A shader layer's span.
    pub fn code(key: pm_glsl::Key, layer: &str) -> Span {
        Span {
            subject: Subject::Code(key),
            layer: LayerId::from(layer),
            instance: None,
            builtin: false,
        }
    }

    fn name(&self) -> String {
        match &self.subject {
            Subject::Definition { id, .. } => id.as_str().to_owned(),
            Subject::Code(_) => format!("The shader of layer {}", self.layer.as_str()),
        }
    }

    fn scope(&self) -> ErrorScope {
        match (&self.subject, &self.instance) {
            (Subject::Definition { id, .. }, Some(effect)) => ErrorScope::Effect {
                layer: self.layer.clone(),
                effect: effect.clone(),
                definition: id.clone(),
            },
            (Subject::Definition { kind, id }, None) if *kind != DefinitionKind::Layer => {
                ErrorScope::Definition {
                    definition: id.clone(),
                }
            }
            _ => ErrorScope::Layer {
                layer: self.layer.clone(),
            },
        }
    }
}

/// The test hook: what a frame took, in place of GPU timestamps and the
/// wall clock ([`Watchdog::set_timing_source`]).
pub trait TimingSource: Send + Sync {
    /// The GPU time `span` took in its frame; `None`: not measured.
    fn span(&self, span: &Span) -> Option<Duration>;
    /// The wall time from the frame's submission to its completion.
    fn frame(&self, spans: &[Span]) -> Duration;
}

/// A [`TimingSource`] from two closures (tests).
pub struct FnTiming<S, F>(pub S, pub F);

impl<S, F> TimingSource for FnTiming<S, F>
where
    S: Fn(&Span) -> Option<Duration> + Send + Sync,
    F: Fn(&[Span]) -> Duration + Send + Sync,
{
    fn span(&self, span: &Span) -> Option<Duration> {
        (self.0)(span)
    }

    fn frame(&self, spans: &[Span]) -> Duration {
        (self.1)(spans)
    }
}

/// A frame the watchdog follows.
struct Inflight {
    spans: Vec<Span>,
    /// Whether its spans carry GPU queries (one frame at a time).
    queried: bool,
    stage: Stage,
    /// When it was submitted.
    submitted: Option<Instant>,
    /// Set by the queue when its commands completed.
    completed: Arc<Mutex<Option<Instant>>>,
    /// The hang guard already acted on it.
    hung: bool,
}

/// Where a frame's timestamps are.
enum Stage {
    /// Encoded; the frame is not submitted yet.
    Encoded,
    /// Submitted; waiting for completion.
    Submitted,
    /// The resolve was submitted after the frame completed; `mapped` is set
    /// when the readback can be read.
    Resolving(Arc<AtomicBool>),
}

/// The query set and buffers, allocated once per renderer: Metal's counter
/// sample buffers are a scarce resource, so one frame is timed at a time
/// (a frame encoded while the previous one's timestamps are unread is not
/// timed, but the hang guard still follows it).
struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
}

/// A frame's spans while it is being encoded.
pub struct FrameTimer {
    pub(crate) spans: Vec<Span>,
    /// The frame's query set and marker, when it is timed on the GPU.
    gpu: Option<(wgpu::QuerySet, Marker)>,
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
    /// Records a program drawn in the frame; the query indices of its
    /// markers when the frame is timed on the GPU and the set has room.
    pub(crate) fn begin(&mut self, span: Span) -> Option<(u32, u32)> {
        let index = self.spans.len() as u32;
        self.spans.push(span);
        if self.gpu.is_none() || index >= CAPACITY {
            return None;
        }
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
        let Some((set, marker)) = &self.gpu else {
            return;
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm effect watchdog marker"),
            layout: &marker.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: marker.sink.as_entire_binding(),
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
                query_set: set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(query),
            }),
        });
        pass.set_pipeline(&marker.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

/// The watchdog state of a renderer.
pub struct Watchdog {
    budget: Duration,
    hang: Duration,
    policy: Policy,
    disabled: BTreeMap<Subject, f64>,
    /// Shader-layer code disabled, oldest first (bounded).
    disabled_code: VecDeque<pm_glsl::Key>,
    inflight: VecDeque<Inflight>,
    reports: Vec<EngineError>,
    /// The longest time measured per program, in milliseconds.
    measured: BTreeMap<Subject, f64>,
    marker: Option<Marker>,
    timing: Option<Timing>,
    /// The newest trusted timestamp: a sample at or before it is a stale
    /// value Metal did not overwrite.
    last_tick: u64,
    /// Job policy: the hang that stops every later frame.
    stopped: Option<String>,
    source: Option<Arc<dyn TimingSource>>,
}

impl Default for Watchdog {
    fn default() -> Self {
        Watchdog {
            budget: DEFAULT_BUDGET,
            hang: DEFAULT_HANG,
            policy: Policy::Interactive,
            disabled: BTreeMap::new(),
            disabled_code: VecDeque::new(),
            inflight: VecDeque::new(),
            reports: Vec::new(),
            measured: BTreeMap::new(),
            marker: None,
            timing: None,
            last_tick: 0,
            stopped: None,
            source: None,
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

    /// The wall time after submission past which a frame is a runaway.
    pub fn set_hang(&mut self, hang: Duration) {
        self.hang = hang;
    }

    pub fn hang(&self) -> Duration {
        self.hang
    }

    /// How the renderer acts on overruns; [`Policy::Job`] also sets the
    /// job hang budget ([`JOB_HANG`]).
    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
        self.hang = match policy {
            Policy::Interactive => DEFAULT_HANG,
            Policy::Job => JOB_HANG,
        };
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// The test hook: times from `source` instead of the GPU and the wall
    /// clock (`None`: the GPU again).
    pub fn set_timing_source(&mut self, source: Option<Arc<dyn TimingSource>>) {
        self.source = source;
    }

    pub fn is_disabled(&self, kind: DefinitionKind, id: &str) -> bool {
        self.disabled.contains_key(&Subject::Definition {
            kind,
            id: DefinitionId::from(id),
        })
    }

    /// Whether a shader layer's code is disabled.
    pub fn is_code_disabled(&self, key: &pm_glsl::Key) -> bool {
        self.disabled.contains_key(&Subject::Code(*key))
    }

    /// A re-registered definition starts over.
    pub fn forget(&mut self, kind: DefinitionKind, id: &DefinitionId) {
        let subject = Subject::Definition {
            kind,
            id: id.clone(),
        };
        self.disabled.remove(&subject);
        self.measured.remove(&subject);
    }

    /// Re-enables everything (the user asked to try again; a job starts).
    pub fn reset(&mut self) {
        self.disabled.clear();
        self.disabled_code.clear();
        self.stopped = None;
    }

    /// Job policy: why every frame fails now (a hang), until
    /// [`Watchdog::reset`].
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The longest GPU time measured for a definition, in milliseconds.
    pub fn measured_ms(&self, kind: DefinitionKind, id: &str) -> Option<f64> {
        self.measured
            .get(&Subject::Definition {
                kind,
                id: DefinitionId::from(id),
            })
            .copied()
    }

    /// The longest GPU time measured for a shader layer's code.
    pub fn measured_code_ms(&self, key: &pm_glsl::Key) -> Option<f64> {
        self.measured.get(&Subject::Code(*key)).copied()
    }

    /// Overruns reported since the last call.
    pub fn take_reports(&mut self) -> Vec<EngineError> {
        std::mem::take(&mut self.reports)
    }

    /// Frames submitted and not yet judged.
    pub fn frames_in_flight(&self) -> usize {
        self.inflight.len()
    }

    /// The timer for a frame: spans are always recorded (the hang guard);
    /// GPU queries are added when the device can time passes, the budget
    /// applies, no test source is set, and no other timed frame is unread.
    pub(crate) fn frame_timer(&mut self, device: &wgpu::Device) -> FrameTimer {
        let queried = self.inflight.iter().any(|f| f.queried);
        let gpu = (self.source.is_none()
            && self.policy == Policy::Interactive
            && !queried
            && device.features().contains(wgpu::Features::TIMESTAMP_QUERY))
        .then(|| {
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
                }
            });
            (timing.set.clone(), marker)
        });
        FrameTimer {
            spans: Vec::new(),
            gpu,
        }
    }

    /// Keeps a frame's spans until its command buffer is submitted
    /// ([`Watchdog::submitted`]).
    pub(crate) fn finish(&mut self, _encoder: &mut wgpu::CommandEncoder, timer: FrameTimer) {
        if timer.spans.is_empty() {
            return;
        }
        while self.inflight.len() >= INFLIGHT {
            // Judged with what is known: the oldest frame has had seven
            // frames' time to complete.
            let oldest = self.inflight.pop_front().expect("not empty");
            self.forget_frame(oldest);
        }
        self.inflight.push_back(Inflight {
            queried: timer.gpu.is_some(),
            spans: timer.spans,
            stage: Stage::Encoded,
            submitted: None,
            completed: Arc::new(Mutex::new(None)),
            hung: false,
        });
    }

    /// A frame dropped unjudged: its hang guard still applies when it was
    /// already past the hang budget.
    fn forget_frame(&mut self, frame: Inflight) {
        let late = frame.submitted.is_some_and(|t| {
            t.elapsed() > self.hang && frame.completed.lock().map_or(true, |c| c.is_none())
        });
        if late && !frame.hung {
            self.runaway(&frame.spans, None);
        }
    }

    /// The frame just submitted: its completion is watched.
    pub(crate) fn submitted(&mut self, queue: &wgpu::Queue) {
        let Some(frame) = self
            .inflight
            .back_mut()
            .filter(|f| matches!(f.stage, Stage::Encoded))
        else {
            return;
        };
        frame.stage = Stage::Submitted;
        frame.submitted = Some(Instant::now());
        let completed = frame.completed.clone();
        queue.on_submitted_work_done(move || {
            if let Ok(mut c) = completed.lock() {
                *c = Some(Instant::now());
            }
        });
    }

    /// Advances the frames followed: a frame past the hang budget disables
    /// its programs; a completed frame is judged against the budget (GPU
    /// timestamps resolved in a submission of their own and read once
    /// mapped, or the test source's times). `wait` blocks until every
    /// submitted frame is judged (tests).
    pub fn collect(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, wait: bool) {
        loop {
            let _ = device.poll(if wait {
                wgpu::PollType::wait_indefinitely()
            } else {
                wgpu::PollType::Poll
            });
            let progressed = self.advance(device, queue);
            if !wait || !progressed || self.inflight.is_empty() {
                return;
            }
        }
    }

    /// One step over the frames in order; whether anything moved.
    fn advance(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let mut progressed = false;
        let mut i = 0;
        while i < self.inflight.len() {
            let frame = &mut self.inflight[i];
            let Some(submitted) = frame.submitted else {
                i += 1;
                continue;
            };
            let completed = frame.completed.lock().ok().and_then(|c| *c);
            // The wall time from submission to completion, or so far.
            let wall = match (&self.source, completed) {
                (Some(source), Some(_)) => source.frame(&frame.spans),
                (Some(_), None) => Duration::ZERO,
                (None, Some(done)) => done.saturating_duration_since(submitted),
                (None, None) => submitted.elapsed(),
            };
            if wall > self.hang && !frame.hung {
                frame.hung = true;
                let spans = frame.spans.clone();
                self.runaway(&spans, Some(wall));
                progressed = true;
            }
            let frame = &mut self.inflight[i];
            if completed.is_none() {
                i += 1;
                continue;
            }
            if let Some(source) = self.source.clone() {
                let frame = self.inflight.remove(i).expect("indexed");
                for span in &frame.spans {
                    if let Some(time) = source.span(span) {
                        self.record(span, time.as_secs_f64() * 1000.0);
                    }
                }
                progressed = true;
                continue;
            }
            if !frame.queried || self.timing.is_none() {
                self.inflight.remove(i);
                progressed = true;
                continue;
            }
            let timing = self.timing.as_ref().expect("checked");
            if matches!(frame.stage, Stage::Submitted) {
                let count = frame.spans.len().min(CAPACITY as usize) as u32 * 2;
                let bytes = u64::from(count) * 8;
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("pm effect watchdog resolve"),
                });
                encoder.resolve_query_set(&timing.set, 0..count, &timing.resolve, 0);
                encoder.copy_buffer_to_buffer(&timing.resolve, 0, &timing.readback, 0, bytes);
                queue.submit([encoder.finish()]);
                let mapped = Arc::new(AtomicBool::new(false));
                let flag = mapped.clone();
                timing
                    .readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        if result.is_ok() {
                            flag.store(true, Ordering::Release);
                        }
                    });
                frame.stage = Stage::Resolving(mapped);
                progressed = true;
            }
            let Stage::Resolving(mapped) = &frame.stage else {
                i += 1;
                continue;
            };
            if !mapped.load(Ordering::Acquire) {
                i += 1;
                continue;
            }
            let frame = self.inflight.remove(i).expect("indexed");
            self.judge_timestamps(&frame, wall, queue);
            progressed = true;
        }
        progressed
    }

    /// Reads a timed frame's mapped timestamps and judges each plausible
    /// span.
    fn judge_timestamps(&mut self, frame: &Inflight, wall: Duration, queue: &wgpu::Queue) {
        let timing = self.timing.as_ref().expect("timed");
        let period = f64::from(queue.get_timestamp_period());
        let timed = frame.spans.len().min(CAPACITY as usize);
        let ticks: Vec<u64> = {
            let data = timing
                .readback
                .slice(..)
                .get_mapped_range()
                .expect("mapped readback");
            data.chunks_exact(8)
                .take(timed * 2)
                .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
                .collect()
        };
        timing.readback.unmap();
        // A span can take no longer than its frame did (plus the poll's
        // granularity); anything longer is not a sample of this frame.
        let ceiling_ms = wall.as_secs_f64() * 1000.0 + 50.0;
        let (times, floor) = plausible_spans(&ticks, self.last_tick, period, ceiling_ms);
        self.last_tick = floor;
        for (span, ms) in frame.spans.iter().zip(times) {
            if let Some(ms) = ms {
                self.record(span, ms);
            }
        }
    }

    fn record(&mut self, span: &Span, ms: f64) {
        let longest = self.measured.entry(span.subject.clone()).or_insert(0.0);
        *longest = longest.max(ms);
        if self.policy == Policy::Job
            || ms <= self.budget.as_secs_f64() * 1000.0
            || self.disabled.contains_key(&span.subject)
        {
            return;
        }
        let message = format!(
            "{} took {ms:.1} ms of GPU time in one frame (budget {} ms) and is turned off until {}.",
            span.name(),
            self.budget.as_millis(),
            match span.subject {
                Subject::Definition { .. } => "it is registered or reloaded again",
                Subject::Code(_) => "its code changes",
            }
        );
        self.disable(span, ms, message);
    }

    /// The hang guard: every program of a runaway frame that is not a
    /// built-in is disabled (job policy: every later frame fails).
    fn runaway(&mut self, spans: &[Span], wall: Option<Duration>) {
        let seconds = wall.unwrap_or(self.hang).as_secs_f64();
        let mut named = Vec::new();
        for span in spans {
            if span.builtin || self.disabled.contains_key(&span.subject) {
                continue;
            }
            let message = format!(
                "{} did not finish: its frame ran {seconds:.1} s on the GPU (limit {} s), so it is turned off until {}.",
                span.name(),
                self.hang.as_secs_f64(),
                match span.subject {
                    Subject::Definition { .. } => "it is registered or reloaded again",
                    Subject::Code(_) => "its code changes",
                }
            );
            named.push(span.name());
            self.disable(span, seconds * 1000.0, message);
        }
        if self.policy == Policy::Job && !named.is_empty() && self.stopped.is_none() {
            self.stopped = Some(format!(
                "The render stopped: {} did not finish within {} s of GPU time.",
                named.join(", "),
                self.hang.as_secs_f64()
            ));
        }
    }

    fn disable(&mut self, span: &Span, ms: f64, message: String) {
        if let Subject::Code(key) = &span.subject {
            if self.disabled_code.len() >= DISABLED_CODE
                && let Some(old) = self.disabled_code.pop_front()
            {
                self.disabled.remove(&Subject::Code(old));
            }
            self.disabled_code.push_back(*key);
        }
        self.disabled.insert(span.subject.clone(), ms);
        self.reports.push(
            EngineError::new(
                ErrorCode::WatchdogTimeout,
                span.scope(),
                crate::effects::truncate(message),
            )
            .with_retry(Retry::AfterUserAction),
        );
    }
}

/// The trusted span times of one frame's resolved timestamps (begin and end
/// per span, in ticks of `period` ns), and the new floor: a span is trusted
/// when both samples are after `floor` (not a stale value Metal left), it
/// ends after it begins, and it is no longer than `ceiling_ms` (the frame's
/// wall time). Only trusted samples raise the floor, so one garbage sample
/// cannot stop later frames from being judged.
pub(crate) fn plausible_spans(
    ticks: &[u64],
    floor: u64,
    period: f64,
    ceiling_ms: f64,
) -> (Vec<Option<f64>>, u64) {
    let mut newest = floor;
    let times = ticks
        .chunks_exact(2)
        .map(|pair| {
            let (begin, end) = (pair[0], pair[1]);
            if begin <= floor || end <= begin {
                return None;
            }
            let ms = (end - begin) as f64 * period / 1.0e6;
            if !ms.is_finite() || ms > ceiling_ms {
                return None;
            }
            newest = newest.max(end);
            Some(ms)
        })
        .collect();
    (times, newest)
}

#[cfg(test)]
mod tests {
    use super::plausible_spans;

    #[test]
    fn written_spans_are_judged_and_raise_the_floor() {
        let (times, floor) = plausible_spans(&[1_000, 3_000, 3_500, 4_000], 0, 1.0, 1.0);
        assert_eq!(times, vec![Some(0.002), Some(0.0005)]);
        assert_eq!(floor, 4_000);
    }

    #[test]
    fn stale_and_unwritten_samples_are_not_judged() {
        // Zeros (unwritten), a begin at the floor (left from an earlier
        // frame), an end before its begin.
        let (times, floor) = plausible_spans(&[0, 0, 4_000, 9_000, 9_000, 8_000], 4_000, 1.0, 1.0);
        assert_eq!(times, vec![None, None, None]);
        assert_eq!(floor, 4_000);
    }

    #[test]
    fn a_garbage_sample_does_not_poison_the_floor() {
        // One frame's end sample is garbage (far in the future): it is not
        // judged and later frames are still judged.
        let garbage = u64::MAX / 2;
        let (times, floor) = plausible_spans(&[1_000_000, garbage], 0, 1.0, 16.0);
        assert_eq!(times, vec![None]);
        assert_eq!(floor, 0, "the floor did not move");
        let (times, floor) = plausible_spans(&[2_000_000, 2_500_000], floor, 1.0, 16.0);
        assert_eq!(times, vec![Some(0.5)]);
        assert_eq!(floor, 2_500_000);
    }

    #[test]
    fn a_span_longer_than_its_frame_is_not_judged() {
        // 300 ms between samples of a frame whose commands took 20 ms of
        // wall time: not this frame's samples.
        let (times, _) = plausible_spans(&[1, 300_000_001], 0, 1.0, 20.0 + 50.0);
        assert_eq!(times, vec![None]);
    }
}
