//! The GPU watchdog for effects, transitions, and shader-layer programs
//! (plan section 8, "Device": "Each effect and shader has a GPU timestamp
//! budget; an overrun disables it from the next frame and reports it";
//! issue A2).
//!
//! Two guards, each judged from evidence about a frame, never from when
//! someone happened to look:
//!
//! - **The budget** (interactive renderers): every effect instance,
//!   transition, and shader or extension fragment layer is timed between two
//!   marker dispatches, each a one-workgroup compute pass whose end-of-pass
//!   sample is the timestamp (as `pm-export`'s `GpuSpan`: Metal's
//!   start-of-encoder samples are not reliable, and pass-boundary samples of
//!   render passes came back empty while several devices were busy): one
//!   before the program's first pass reading a texel of its input, one after
//!   its last pass reading a texel of its output. The timestamps are resolved
//!   in a submission of their own once the frame has completed (resolved in
//!   the frame's own command buffer, Metal returned the previous frame's end
//!   sample). A program over the budget is disabled for every later frame
//!   (skipped with its data kept, like a missing effect) and reported as
//!   `WatchdogTimeout` until it is registered again (a shader layer: until
//!   its code changes).
//! - **The hang guard** (every renderer): a program that does not finish (a
//!   runaway loop the GPU runs until the operating system's GPU timeout
//!   aborts the command buffer) is disabled and reported; a job stops. It
//!   works in two steps:
//!   1. *Suspicion.* A frame is suspected only when a poll made after the
//!      hang budget had passed saw it still running, counting from when it
//!      could start (its submission, or the completion of the frame before
//!      it, which the GPU runs first). A frame that completed while nobody
//!      polled is not suspected: the completion callback fires at the next
//!      poll, so its time says when someone looked, not when the GPU
//!      finished. While a suspected frame runs, the interactive renderer
//!      holds every frame that would draw one of its user programs
//!      ([`crate::RenderError::WatchdogPending`]: nothing is drawn wrong and
//!      a runaway is not submitted again).
//!   2. *Attribution*, when the suspected frame has completed (or the system
//!      aborted it): a program is guilty when its own timestamps show it ran
//!      past the hang budget, or began and never ended in a frame whose wall
//!      time passed the hang budget. Programs whose timestamps show they
//!      finished are cleared: GPU contention from another process, an
//!      export on the same GPU, or a heavy frame of built-ins turns nothing
//!      off. Without timestamps (a device without timestamp queries, or both
//!      query slots busy) a program is guilty only after two such hangs.
//!   Built-in programs cannot run away and are never suspected.
//!
//! A timestamp sample is trusted only when it is plausible: written (after
//! the newest trusted sample) and spanning no more than the frame's own
//! submission-to-completion wall time. A stale or garbage value is not
//! judged and never becomes the floor later samples are compared with.
//!
//! **Policies.** An interactive renderer skips a disabled program and keeps
//! drawing ([`Policy::Interactive`]). A job renderer ([`Policy::Job`]: export
//! jobs, agent stills) never skips a program: it applies no per-program
//! budget (today's exporter renders a slow effect slowly), never holds a
//! frame, and after a runaway fails every later frame with a named error
//! until [`Watchdog::reset`] (the job engine resets it per job), so nothing
//! is delivered without the program the project asks for.
//!
//! **The test hooks.** [`Watchdog::set_timing_source`] replaces the GPU
//! timestamps and the frame's wall time with a [`TimingSource`]: every
//! frame is judged at its completion with the times the source gives, so the
//! budget, both steps of the hang guard, and every re-enable rule are tested
//! deterministically on any device, under any load.
//! [`Watchdog::set_clock`] replaces the wall clock alone, so the real
//! completion path (polls, callbacks, timestamps) is tested with time
//! passing at will. Production renderers set neither.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

/// Default hang budget of an interactive renderer: a frame seen still
/// running this long after it could start is suspected.
pub const DEFAULT_HANG: Duration = Duration::from_secs(2);

/// Hang budget of a job renderer: Chromium's own GPU watchdog lets a WebGL
/// frame run this long before it loses the context, so an export that
/// finishes today finishes here.
pub const JOB_HANG: Duration = Duration::from_secs(10);

/// Timed programs per frame (two queries each).
pub const CAPACITY: u32 = 64;

/// Frames timed at once (query sets; Metal's counter sample buffers are a
/// scarce resource).
const SLOTS: usize = 2;

/// Frames followed at once; past it the oldest is dropped unjudged (nothing
/// is turned off without evidence).
const INFLIGHT: usize = 64;

/// Disabled shader-layer programs remembered (by source key).
const DISABLED_CODE: usize = 1024;

/// Unexplained hangs (no timestamps) after which a suspect is guilty.
const STRIKES: u32 = 2;

/// How a renderer acts on what the watchdog finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// The viewer: the budget and the hang guard; a disabled program is
    /// skipped with its data kept and reported.
    #[default]
    Interactive,
    /// Export jobs and agent stills: no per-program budget; after a runaway
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
    /// A built-in program (origin `builtIn`): never suspected.
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

    fn until(&self) -> &'static str {
        match self.subject {
            Subject::Definition { .. } => "it is registered or reloaded again",
            Subject::Code(_) => "its code changes",
        }
    }
}

/// What a frame's timestamps say about one program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpanTime {
    /// It ran this long.
    Measured(Duration),
    /// It began and its end was never written: the frame stopped inside it
    /// (or a sample was lost; never guilty alone).
    Unfinished,
    /// Not timed, or no trusted sample.
    Unknown,
}

impl From<Option<Duration>> for SpanTime {
    fn from(time: Option<Duration>) -> SpanTime {
        time.map_or(SpanTime::Unknown, SpanTime::Measured)
    }
}

impl From<Duration> for SpanTime {
    fn from(time: Duration) -> SpanTime {
        SpanTime::Measured(time)
    }
}

/// The test hook: what a frame took, in place of GPU timestamps and the
/// frame's wall time ([`Watchdog::set_timing_source`]).
pub trait TimingSource: Send + Sync {
    /// What the timestamps of `span` say in its frame.
    fn span(&self, span: &Span) -> SpanTime;
    /// The wall time from the frame's submission (or the moment it could
    /// start) to its completion; while [`TimingSource::running`], how long
    /// it has run so far.
    fn frame(&self, spans: &[Span]) -> Duration;
    /// Whether the frame is still running when it is looked at (default:
    /// it completed).
    fn running(&self, _spans: &[Span]) -> bool {
        false
    }
}

/// A [`TimingSource`] from two closures (tests); the first returns an
/// `Option<Duration>`, a `Duration`, or a [`SpanTime`].
pub struct FnTiming<S, F>(pub S, pub F);

impl<S, F, T> TimingSource for FnTiming<S, F>
where
    S: Fn(&Span) -> T + Send + Sync,
    T: Into<SpanTime>,
    F: Fn(&[Span]) -> Duration + Send + Sync,
{
    fn span(&self, span: &Span) -> SpanTime {
        (self.0)(span).into()
    }

    fn frame(&self, spans: &[Span]) -> Duration {
        (self.1)(spans)
    }
}

/// The wall clock (a test replaces it: [`Watchdog::set_clock`]).
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A frame the watchdog follows (every frame, with or without programs:
/// a frame cannot start before the one submitted before it completes).
struct Inflight {
    spans: Vec<Span>,
    /// The query slot holding its timestamps, when timed.
    slot: Option<usize>,
    stage: Stage,
    /// When it was submitted.
    submitted: Option<Instant>,
    /// Set by the queue when its completion was seen (at a poll or a
    /// submission: an upper bound of when the GPU finished).
    completed: Arc<Mutex<Option<Instant>>>,
    /// A poll after the hang budget saw it running.
    suspect: bool,
}

impl Inflight {
    fn done(&self) -> Option<Instant> {
        self.completed.lock().ok().and_then(|c| *c)
    }
}

/// Where a frame's timestamps are.
enum Stage {
    /// Encoded; the frame is not submitted yet.
    Encoded,
    /// Submitted; waiting for completion.
    Submitted,
    /// The resolve was submitted after the frame completed; set when the
    /// readback can be read.
    Resolving(Arc<AtomicBool>),
}

/// One query slot: a query set and its buffers, allocated once.
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
    slot: Option<usize>,
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
    /// Unexplained hangs per suspect (no timestamps for it).
    strikes: BTreeMap<Subject, u32>,
    marker: Option<Marker>,
    slots: Vec<Timing>,
    /// The newest trusted timestamp: a sample at or before it is a stale
    /// value Metal did not overwrite.
    last_tick: u64,
    /// Job policy: the runaway that stops every later frame.
    stopped: Option<String>,
    source: Option<Arc<dyn TimingSource>>,
    clock: Option<Clock>,
    /// The newest completion of a frame no longer followed (frames
    /// complete in submission order: the next one started after it).
    last_done: Option<Instant>,
    /// The frame being encoded left a held program out
    /// ([`Watchdog::hold`]).
    held_frame: bool,
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
            strikes: BTreeMap::new(),
            marker: None,
            slots: Vec::new(),
            last_tick: 0,
            stopped: None,
            source: None,
            clock: None,
            last_done: None,
            held_frame: false,
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

    /// The running time past which a frame seen still running is suspected.
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

    /// The test hook: times from `source` instead of the GPU timestamps and
    /// the frame's wall time (`None`: the GPU again).
    pub fn set_timing_source(&mut self, source: Option<Arc<dyn TimingSource>>) {
        self.source = source;
    }

    /// The test hook for the wall clock alone (`None`: `Instant::now`).
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.clock = clock;
    }

    fn now(&self) -> Instant {
        self.clock.as_ref().map_or_else(Instant::now, |c| c())
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

    /// Whether a frame drawing `subject` must wait (interactive policy): a
    /// frame that drew it is suspected of running away and has not ended,
    /// so it is not submitted again until the watchdog has judged it.
    pub fn is_held(&self, subject: &Subject) -> bool {
        self.policy == Policy::Interactive
            && self
                .inflight
                .iter()
                .any(|f| f.suspect && f.spans.iter().any(|s| !s.builtin && &s.subject == subject))
    }

    /// [`Watchdog::is_held`] for a program the frame being encoded would
    /// draw: when held, the frame leaves it out and is not presented
    /// ([`Watchdog::take_held`]).
    pub(crate) fn hold(&mut self, subject: &Subject) -> bool {
        let held = self.is_held(subject);
        self.held_frame |= held;
        held
    }

    /// Whether the frame just encoded left a held program out (and clears
    /// it): the renderer fails it with `RenderError::WatchdogPending`.
    pub(crate) fn take_held(&mut self) -> bool {
        std::mem::take(&mut self.held_frame)
    }

    /// Whether any frame is suspected and not judged yet (a held frame
    /// renders again once this is false).
    pub fn holding(&self) -> bool {
        self.policy == Policy::Interactive && self.inflight.iter().any(|f| f.suspect)
    }

    /// A re-registered definition starts over.
    pub fn forget(&mut self, kind: DefinitionKind, id: &DefinitionId) {
        let subject = Subject::Definition {
            kind,
            id: id.clone(),
        };
        self.disabled.remove(&subject);
        self.measured.remove(&subject);
        self.strikes.remove(&subject);
    }

    /// Re-enables everything (the user asked to try again; a job starts).
    pub fn reset(&mut self) {
        self.disabled.clear();
        self.disabled_code.clear();
        self.strikes.clear();
        self.stopped = None;
    }

    /// Job policy: why every frame fails now (a runaway), until
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
    /// GPU queries are added when the device can time passes, no test
    /// source is set, and a query slot is free.
    pub(crate) fn frame_timer(&mut self, device: &wgpu::Device) -> FrameTimer {
        let timestamps =
            self.source.is_none() && device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let busy: BTreeSet<usize> = self.inflight.iter().filter_map(|f| f.slot).collect();
        let slot = timestamps
            .then(|| (0..SLOTS).find(|s| !busy.contains(s)))
            .flatten();
        let gpu = slot.map(|slot| {
            let marker = self
                .marker
                .get_or_insert_with(|| Marker::new(device))
                .clone();
            while self.slots.len() <= slot {
                let bytes = u64::from(CAPACITY) * 2 * 8;
                self.slots.push(Timing {
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
                });
            }
            (self.slots[slot].set.clone(), marker)
        });
        FrameTimer {
            spans: Vec::new(),
            slot: gpu.as_ref().and(slot),
            gpu,
        }
    }

    /// Keeps a frame until its command buffer is submitted
    /// ([`Watchdog::submitted`]). Every frame is followed, with or without
    /// programs: the next frame cannot start before it completes.
    pub(crate) fn finish(&mut self, _encoder: &mut wgpu::CommandEncoder, timer: FrameTimer) {
        while self.inflight.len() >= INFLIGHT {
            // Dropped unjudged: nothing is turned off without evidence.
            self.inflight.pop_front();
        }
        self.inflight.push_back(Inflight {
            slot: timer.slot.filter(|_| !timer.spans.is_empty()),
            spans: timer.spans,
            stage: Stage::Encoded,
            submitted: None,
            completed: Arc::new(Mutex::new(None)),
            suspect: false,
        });
    }

    /// The frame just submitted: its completion is watched.
    pub(crate) fn submitted(&mut self, queue: &wgpu::Queue) {
        let now = self.now();
        let clock = self.clock.clone();
        let Some(frame) = self
            .inflight
            .back_mut()
            .filter(|f| matches!(f.stage, Stage::Encoded))
        else {
            return;
        };
        frame.stage = Stage::Submitted;
        frame.submitted = Some(now);
        let completed = frame.completed.clone();
        queue.on_submitted_work_done(move || {
            if let Ok(mut c) = completed.lock() {
                *c = Some(clock.as_ref().map_or_else(Instant::now, |c| c()));
            }
        });
    }

    /// Advances the frames followed without waiting: completions the device
    /// has reached are seen now, frames seen still running past the hang
    /// budget are suspected, and completed frames are judged (GPU
    /// timestamps resolved in a submission of their own and read once
    /// mapped, or the test source's times). `wait` blocks until every
    /// submitted frame is judged (tests, and job renderers, whose frames
    /// have already completed).
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
        let now = self.now();
        // When the frame before the one looked at completed (`None`: it has
        // not, so this one has not started).
        let mut previous_done: Option<Option<Instant>> = self.last_done.map(Some);
        let mut i = 0;
        while i < self.inflight.len() {
            let frame = &self.inflight[i];
            let Some(submitted) = frame.submitted else {
                break;
            };
            let real_done = frame.done();
            // It could start at its submission, or once the frame before it
            // completed, whichever is later.
            let start = match previous_done {
                None => Some(submitted),
                Some(Some(done)) => Some(submitted.max(done)),
                Some(None) => None,
            };
            previous_done = Some(real_done);
            let source = self.source.clone();
            // Still running, and for how long it has certainly run.
            let running = match &source {
                Some(source) if source.running(&frame.spans) => Some(source.frame(&frame.spans)),
                Some(_) => real_done.is_none().then_some(Duration::ZERO),
                None => real_done
                    .is_none()
                    .then(|| start.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))),
            };
            if let Some(ran) = running {
                if ran > self.hang && !frame.spans.is_empty() && !self.inflight[i].suspect {
                    self.inflight[i].suspect = true;
                    progressed = true;
                }
                // A frame that has not completed holds back the ones after
                // it only for their start; they are looked at as well.
                i += 1;
                continue;
            }
            // Completed: an upper bound of its running time (the completion
            // is seen at a poll), used only to bound its timestamps.
            let wall = match &source {
                Some(source) => source.frame(&frame.spans),
                None => match (start, real_done) {
                    (Some(s), Some(d)) => d.saturating_duration_since(s),
                    _ => Duration::ZERO,
                },
            };
            if let Some(source) = source {
                let frame = self.remove(i);
                let suspect = frame.suspect || wall > self.hang;
                let times: Vec<SpanTime> = frame.spans.iter().map(|s| source.span(s)).collect();
                self.judge(&frame.spans, &times, suspect);
                progressed = true;
                continue;
            }
            if frame.spans.is_empty() {
                self.remove(i);
                progressed = true;
                continue;
            }
            let Some(slot) = frame.slot.filter(|s| *s < self.slots.len()) else {
                let frame = self.remove(i);
                let times = vec![SpanTime::Unknown; frame.spans.len()];
                self.judge(&frame.spans, &times, frame.suspect);
                progressed = true;
                continue;
            };
            let timing = &self.slots[slot];
            let frame = &mut self.inflight[i];
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
            let frame = self.remove(i);
            let times = self.read_timestamps(&frame, slot, wall, queue);
            self.judge(&frame.spans, &times, frame.suspect);
            progressed = true;
        }
        progressed
    }

    /// Stops following the frame at `i` (it completed).
    fn remove(&mut self, i: usize) -> Inflight {
        let frame = self.inflight.remove(i).expect("indexed");
        if let Some(done) = frame.done() {
            self.last_done = Some(self.last_done.map_or(done, |d| d.max(done)));
        }
        frame
    }

    /// Reads a timed frame's mapped timestamps: what each span's samples
    /// say.
    fn read_timestamps(
        &mut self,
        frame: &Inflight,
        slot: usize,
        wall: Duration,
        queue: &wgpu::Queue,
    ) -> Vec<SpanTime> {
        let timing = &self.slots[slot];
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
        let (mut times, floor) = plausible_spans(&ticks, self.last_tick, period, ceiling_ms);
        self.last_tick = floor;
        times.resize(frame.spans.len(), SpanTime::Unknown);
        times
    }

    /// Judges a completed frame: the budget for every measured program and,
    /// when the frame was suspected, the hang guard's attribution.
    fn judge(&mut self, spans: &[Span], times: &[SpanTime], suspect: bool) {
        for (span, time) in spans.iter().zip(times) {
            if let SpanTime::Measured(t) = time {
                // It finished: earlier unexplained hangs are forgiven.
                if *t <= self.hang {
                    self.strikes.remove(&span.subject);
                }
                self.record(span, t.as_secs_f64() * 1000.0);
            }
        }
        if suspect {
            self.attribute(spans, times);
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
            span.until(),
        );
        self.disable(span, ms, message);
    }

    /// The hang guard's second step, for a frame seen running past the hang
    /// budget that has now ended: a user program is guilty when its
    /// timestamps show it ran past the budget, or that the frame stopped
    /// inside it; with no timestamps for it, after [`STRIKES`] such frames.
    /// A program whose timestamps show it finished in time is cleared.
    fn attribute(&mut self, spans: &[Span], times: &[SpanTime]) {
        let hang_ms = self.hang.as_secs_f64() * 1000.0;
        let mut guilty: Vec<(Span, String)> = Vec::new();
        for (span, time) in spans.iter().zip(times) {
            if span.builtin
                || self.disabled.contains_key(&span.subject)
                || guilty.iter().any(|(g, _)| g.subject == span.subject)
            {
                continue;
            }
            let why = match time {
                SpanTime::Measured(t) if t.as_secs_f64() * 1000.0 > hang_ms => {
                    format!("ran {:.1} s of GPU time in one frame", t.as_secs_f64())
                }
                SpanTime::Measured(_) => continue,
                SpanTime::Unfinished => "did not finish: its frame stopped inside it".to_owned(),
                SpanTime::Unknown => {
                    let strikes = self.strikes.entry(span.subject.clone()).or_insert(0);
                    *strikes += 1;
                    if *strikes < STRIKES {
                        continue;
                    }
                    format!("was in {STRIKES} frames that did not finish")
                }
            };
            guilty.push((span.clone(), why));
        }
        let mut named = Vec::new();
        for (span, why) in guilty {
            let message = format!(
                "{} {why} (limit {} s), so it is turned off until {}.",
                span.name(),
                self.hang.as_secs_f64(),
                span.until(),
            );
            named.push(span.name());
            self.strikes.remove(&span.subject);
            self.disable(&span, hang_ms, message);
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

/// What one frame's resolved timestamps say about each span (begin and end
/// per span, in ticks of `period` ns), and the new floor:
///
/// - **Measured** when both samples are after `floor` (not a stale value
///   Metal left), it ends after it begins, and it is no longer than
///   `ceiling_ms` (the frame's wall time).
/// - **Unfinished** when its begin was written and its end was not, and no
///   later span of the frame has a sample written: the frame stopped inside
///   it (a command buffer the system aborted writes nothing after the point
///   it stopped).
/// - **Unknown** otherwise.
///
/// Only measured samples raise the floor, so one garbage sample cannot stop
/// later frames from being judged.
pub(crate) fn plausible_spans(
    ticks: &[u64],
    floor: u64,
    period: f64,
    ceiling_ms: f64,
) -> (Vec<SpanTime>, u64) {
    let pairs: Vec<(u64, u64)> = ticks.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    let written = |t: u64| t > floor;
    let mut newest = floor;
    let times = pairs
        .iter()
        .enumerate()
        .map(|(i, &(begin, end))| {
            if !written(begin) {
                return SpanTime::Unknown;
            }
            if !written(end) {
                let later_silent = pairs[i + 1..]
                    .iter()
                    .all(|&(b, e)| !written(b) && !written(e));
                return if later_silent {
                    SpanTime::Unfinished
                } else {
                    SpanTime::Unknown
                };
            }
            if end <= begin {
                return SpanTime::Unknown;
            }
            let ms = (end - begin) as f64 * period / 1.0e6;
            if !ms.is_finite() || ms > ceiling_ms {
                return SpanTime::Unknown;
            }
            newest = newest.max(end);
            SpanTime::Measured(Duration::from_secs_f64(ms / 1000.0))
        })
        .collect();
    (times, newest)
}

#[cfg(test)]
mod tests {
    use super::{SpanTime, plausible_spans};
    use std::time::Duration;

    fn ms(v: f64) -> SpanTime {
        SpanTime::Measured(Duration::from_secs_f64(v / 1000.0))
    }

    #[test]
    fn written_spans_are_judged_and_raise_the_floor() {
        let (times, floor) = plausible_spans(&[1_000, 3_000, 3_500, 4_000], 0, 1.0, 1.0);
        assert_eq!(times, vec![ms(0.002), ms(0.0005)]);
        assert_eq!(floor, 4_000);
    }

    #[test]
    fn stale_and_unwritten_samples_are_not_judged() {
        // Zeros (unwritten), a begin at the floor (left from an earlier
        // frame), an end before its begin.
        let (times, floor) = plausible_spans(&[0, 0, 4_000, 9_000, 9_000, 8_000], 4_000, 1.0, 1.0);
        assert_eq!(times, vec![SpanTime::Unknown; 3]);
        assert_eq!(floor, 4_000);
    }

    #[test]
    fn a_garbage_sample_does_not_poison_the_floor() {
        // One frame's end sample is garbage (far in the future): it is not
        // judged and later frames are still judged.
        let garbage = u64::MAX / 2;
        let (times, floor) = plausible_spans(&[1_000_000, garbage], 0, 1.0, 16.0);
        assert_eq!(times, vec![SpanTime::Unknown]);
        assert_eq!(floor, 0, "the floor did not move");
        let (times, floor) = plausible_spans(&[2_000_000, 2_500_000], floor, 1.0, 16.0);
        assert_eq!(times, vec![ms(0.5)]);
        assert_eq!(floor, 2_500_000);
    }

    #[test]
    fn a_span_longer_than_its_frame_is_not_judged() {
        // 300 ms between samples of a frame whose commands took 20 ms of
        // wall time: not this frame's samples.
        let (times, _) = plausible_spans(&[1, 300_000_001], 0, 1.0, 20.0 + 50.0);
        assert_eq!(times, vec![SpanTime::Unknown]);
    }

    #[test]
    fn a_frame_that_stopped_inside_a_program_leaves_it_unfinished() {
        // The first program finished; the second began and never ended,
        // and nothing after it was written (the system aborted the command
        // buffer there).
        let (times, floor) = plausible_spans(&[1_000, 2_000, 3_000, 0, 0, 0], 0, 1.0, 10_000.0);
        assert_eq!(
            times,
            vec![ms(0.001), SpanTime::Unfinished, SpanTime::Unknown]
        );
        assert_eq!(floor, 2_000);
    }

    #[test]
    fn a_lost_end_sample_with_later_samples_written_is_not_unfinished() {
        // Metal left one end sample empty under load, but the frame went
        // on: not a frame that stopped there.
        let (times, _) = plausible_spans(&[1_000, 0, 3_000, 4_000], 0, 1.0, 10_000.0);
        assert_eq!(times, vec![SpanTime::Unknown, ms(0.001)]);
    }
}
