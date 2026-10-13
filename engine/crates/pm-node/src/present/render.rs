//! The presenter's render thread: GPU device, surface pools, the bridge
//! client, the transport clock, and frame delivery.
//!
//! Commands arrive from the engine's JavaScript thread; descriptors, clock
//! samples, surface pool announcements, and frame responses leave through
//! the sink straight to the viewer's port. A descriptor is sent only after
//! the command buffer that wrote its surface completed, and a surface is
//! never written while the viewer holds it (contract 2's `PoolLedger`).

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pm_bridge::rendezvous::{Client, SendRight};
use pm_bridge::wire::{PixelFormat, SurfaceInfo};
use pm_engine::{Action, LogLevel};
use pm_gpu::bridge_out::display::{DisplayParams, DisplayTransform, WorkingTexture};
use pm_gpu::bridge_out::pool::SurfacePool;
use pm_gpu::bridge_out::test_source::{TestFrame, TestSource};
use pm_protocol::clock::TransportCommand;
use pm_protocol::codec;
use pm_protocol::color::WorkingEncoding;
use pm_protocol::common::{RequestId, Revision, Seq, SessionId, Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::envelope::{Envelope, Event, Message, Response};
use pm_protocol::error::{EngineError, ErrorCode, ErrorScope};
use pm_protocol::frame::{
    FrameDescriptor, FrameQuality, FrameRelease, FrameRequest, PoolLedger, PreviewResolution,
    Representation, ViewerState,
};
use pm_protocol::host::PortId;
use pm_protocol::interop::SurfaceFormat;
use serde::Serialize;

use super::clock::{SAMPLE_INTERVAL_US, TransportClock};
use super::schedule::{Next, Schedule};
use super::view;

/// Where the engine's actions go: posted on the utility process's ports by
/// its JavaScript (pm-node's threadsafe function), or collected by tests.
pub type Sink = Arc<dyn Fn(Vec<Action>) + Send + Sync>;

pub type Key = (PortId, SessionId);

/// A pool the JavaScript side sized and charged to the lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSpec {
    pub generation: u32,
    pub size: Size,
    pub format: SurfaceFormat,
    pub count: u32,
}

pub enum Command {
    Configure {
        key: Key,
        state: Box<ViewerState>,
        revision: Option<Revision>,
        pool: Option<PoolSpec>,
    },
    Release {
        key: Key,
        release: FrameRelease,
    },
    Transport {
        key: Key,
        command: TransportCommand,
        at: i64,
    },
    Render {
        key: Key,
        id: RequestId,
        request: Box<FrameRequest>,
        revision: Option<Revision>,
    },
    Close {
        key: Key,
    },
    ClosePort {
        port: PortId,
    },
    Shutdown,
}

/// How the thread runs.
#[derive(Debug, Clone)]
pub struct ThreadConfig {
    /// The per-launch bootstrap name; `None` when main gave none.
    pub bridge_name: Option<String>,
    /// Frames of render-ahead during playback.
    pub lead: u32,
    /// Test source frame rate, frames per second.
    pub fps: u32,
    /// Extra GPU passes per frame (the S1 harness's engine load).
    pub extra_passes: u32,
    /// Keep a per-frame trace for the S1 harness.
    pub trace: bool,
}

/// One frame's record for the S1 harness.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FrameTrace {
    pub seq: u64,
    pub k: i64,
    pub slot: u32,
    pub generation: u32,
    /// Host µs the frame is shown from (timed frames) or 0.
    pub shown_from: i64,
    /// Host µs it is shown for (timed frames) or 0.
    pub shown_for: i64,
    pub view_seq: u64,
    pub revision: u64,
    pub start: i64,
    pub gpu_done: i64,
    pub sent: i64,
    pub timed: bool,
}

/// Counters the engine reports in its status.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub checked_in: bool,
    pub check_in_us: i64,
    pub bridge_error: Option<String>,
    pub adapter: Option<String>,
    pub pools: u64,
    pub surfaces_sent: u64,
    pub frames: u64,
    pub timed_frames: u64,
    pub immediate_frames: u64,
    pub responses: u64,
    pub late_skipped: u64,
    pub pool_waits: u64,
    pub releases: u64,
    pub release_mismatches: u64,
    pub gpu_us_max: i64,
    pub gpu_us_total: i64,
    pub max_held: u32,
    #[serde(skip)]
    pub trace: Vec<FrameTrace>,
}

const TRACE_CAP: usize = 200_000;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    display: DisplayTransform,
}

struct Live {
    spec: PoolSpec,
    pool: SurfacePool,
    ledger: PoolLedger,
    working: WorkingTexture,
    source: TestSource,
}

struct View {
    key: Key,
    state: ViewerState,
    revision: Revision,
    pool: Option<Live>,
    clock: TransportClock,
    schedule: Schedule,
    last_sample: Option<i64>,
    sample_due: bool,
    next_seq: u64,
    dirty: bool,
    requests: VecDeque<(RequestId, FrameRequest, Option<Revision>)>,
}

struct Thread {
    config: ThreadConfig,
    sink: Sink,
    stats: Arc<Mutex<Stats>>,
    bridge: Option<Client>,
    gpu: Option<Gpu>,
    views: BTreeMap<Key, View>,
    frame_ticks: i64,
}

fn now() -> i64 {
    pm_bridge::now_us()
}

/// The render thread's body.
pub fn run(config: ThreadConfig, rx: Receiver<Command>, sink: Sink, stats: Arc<Mutex<Stats>>) {
    pm_bridge::set_thread_user_interactive();
    let fps = i64::from(config.fps.max(1));
    let mut thread = Thread {
        frame_ticks: TICKS_PER_SECOND / fps,
        config,
        sink,
        stats,
        bridge: None,
        gpu: None,
        views: BTreeMap::new(),
    };
    thread.check_in();
    loop {
        let timeout = thread.next_wake();
        match rx.recv_timeout(timeout) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                if !thread.apply(command) {
                    break;
                }
                while let Ok(command) = rx.try_recv() {
                    if matches!(command, Command::Shutdown) || !thread.apply(command) {
                        thread.shutdown();
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        thread.work();
    }
    thread.shutdown();
}

impl Thread {
    fn log(&self, level: LogLevel, message: String) {
        (self.sink)(vec![Action::Log { level, message }]);
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, Stats> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check_in(&mut self) {
        let Some(name) = self.config.bridge_name.clone() else {
            return;
        };
        let start = now();
        match Client::check_in(&name, Duration::from_secs(10)) {
            Ok(client) => {
                let mut stats = self.stats();
                stats.checked_in = true;
                stats.check_in_us = now() - start;
                drop(stats);
                self.log(
                    LogLevel::Info,
                    format!(
                        "frame bridge: checked in with main (pid {}) in {} µs",
                        client.bridge_peer().pid(),
                        now() - start
                    ),
                );
                self.bridge = Some(client);
            }
            Err(error) => {
                self.stats().bridge_error = Some(error.to_string());
                self.log(
                    LogLevel::Error,
                    format!("frame bridge: check-in failed: {error}"),
                );
            }
        }
    }

    fn gpu(&mut self) -> Option<&Gpu> {
        if self.gpu.is_none() {
            match pm_gpu::bridge_out::metal_device("interactive engine viewer") {
                Ok((device, queue, info)) => {
                    let display = DisplayTransform::new(&device, &queue);
                    self.stats().adapter = Some(info.name.clone());
                    self.gpu = Some(Gpu {
                        device,
                        queue,
                        display,
                    });
                }
                Err(error) => {
                    self.log(
                        LogLevel::Error,
                        format!("frame bridge: no GPU device: {error}"),
                    );
                    return None;
                }
            }
        }
        self.gpu.as_ref()
    }

    /// Sends one envelope on a port.
    fn send(&self, port: PortId, envelope: Envelope) {
        match codec::encode(&envelope) {
            Ok(bytes) => (self.sink)(vec![Action::Send { port, bytes }]),
            Err(error) => self.log(
                LogLevel::Error,
                format!("frame bridge: could not encode a message: {error}"),
            ),
        }
    }

    fn event(&self, key: Key, body: Event, revision: Option<Revision>) {
        let mut envelope = Envelope::new(Message::Event { body }).with_session(key.1);
        envelope.revision = revision;
        self.send(key.0, envelope);
    }

    fn error(&self, key: Key, id: RequestId, code: ErrorCode, message: &str) {
        let error = EngineError::new(code, ErrorScope::Request { id }, message);
        self.send(key.0, Envelope::new(Message::Error { id, error }));
    }

    /// Returns false when the thread must stop.
    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Shutdown => return false,
            Command::Configure {
                key,
                state,
                revision,
                pool,
            } => self.configure(key, *state, revision, pool),
            Command::Release { key, release } => {
                let ok = self
                    .views
                    .get_mut(&key)
                    .and_then(|view| view.pool.as_mut())
                    .is_some_and(|live| live.ledger.release(release).is_ok());
                let mut stats = self.stats();
                if ok {
                    stats.releases += 1;
                } else {
                    // A frame of a retired pool, or a duplicate: ignored.
                    stats.release_mismatches += 1;
                }
            }
            Command::Transport { key, command, at } => {
                if let Some(view) = self.views.get_mut(&key)
                    && view.clock.apply(command, at)
                {
                    view.schedule.reset();
                    view.sample_due = true;
                    if !view.clock.is_playing() {
                        view.dirty = true;
                    }
                }
            }
            Command::Render {
                key,
                id,
                request,
                revision,
            } => match self.views.get_mut(&key) {
                Some(view) => view.requests.push_back((id, *request, revision)),
                None => self.error(
                    key,
                    id,
                    ErrorCode::InvalidMessage,
                    "Send the viewer state before asking for viewer frames.",
                ),
            },
            Command::Close { key } => {
                if let Some(view) = self.views.remove(&key) {
                    self.retire(view);
                }
            }
            Command::ClosePort { port } => {
                let keys: Vec<Key> = self.views.keys().filter(|k| k.0 == port).copied().collect();
                for key in keys {
                    if let Some(view) = self.views.remove(&key) {
                        self.retire(view);
                    }
                }
            }
        }
        true
    }

    fn retire(&mut self, mut view: View) {
        if let Some(live) = view.pool.take() {
            self.retire_pool(view.key, live);
        }
    }

    fn retire_pool(&self, key: Key, live: Live) {
        if let Some(bridge) = &self.bridge
            && let Err(error) = bridge.retire(key.0.0, live.spec.generation)
        {
            self.log(
                LogLevel::Warn,
                format!(
                    "frame bridge: could not retire generation {}: {error}",
                    live.spec.generation
                ),
            );
        }
        // The engine's references go; main and Chromium keep theirs until
        // they release them.
        drop(live);
    }

    fn configure(
        &mut self,
        key: Key,
        state: ViewerState,
        revision: Option<Revision>,
        pool: Option<PoolSpec>,
    ) {
        let frame_ticks = self.frame_ticks;
        let view = self.views.entry(key).or_insert_with(|| View {
            key,
            state: state.clone(),
            revision: Revision::ZERO,
            pool: None,
            clock: TransportClock::new(frame_ticks),
            schedule: Schedule::default(),
            last_sample: None,
            sample_due: true,
            next_seq: 1,
            dirty: true,
            requests: VecDeque::new(),
        });
        view.state = state;
        if let Some(revision) = revision {
            view.revision = view.revision.max(revision);
        }
        view.dirty = true;
        let Some(spec) = pool else {
            return;
        };
        if let Some(old) = self.views.get_mut(&key).and_then(|v| v.pool.take()) {
            self.retire_pool(key, old);
        }
        match self.create_pool(key, spec) {
            Ok(live) => {
                let descriptor = live.pool.descriptor();
                if let Some(view) = self.views.get_mut(&key) {
                    view.pool = Some(live);
                }
                self.event(key, Event::SurfacePool(descriptor), None);
            }
            Err(error) => self.log(
                LogLevel::Error,
                format!(
                    "frame bridge: pool generation {} ({}x{} {:?}) failed: {error}",
                    spec.generation, spec.size.width, spec.size.height, spec.format
                ),
            ),
        }
    }

    fn create_pool(&mut self, key: Key, spec: PoolSpec) -> Result<Live, String> {
        if self.bridge.is_none() {
            return Err("the frame bridge did not check in with main".into());
        }
        let extra_passes = self.config.extra_passes;
        let gpu = self.gpu().ok_or("no GPU device")?;
        let pool = SurfacePool::new(
            &gpu.device,
            spec.generation,
            spec.size,
            spec.format,
            spec.count,
        )
        .map_err(|e| e.to_string())?;
        let working = WorkingTexture::new(&gpu.device, spec.size);
        let source = TestSource::new(&gpu.device, spec.size, extra_passes);
        let bridge = self.bridge.as_ref().ok_or("no bridge")?;
        let format = match spec.format {
            SurfaceFormat::Bgra8Unorm => PixelFormat::Bgra8,
            _ => PixelFormat::Rgba16F,
        };
        for (index, surface) in pool.surfaces.iter().enumerate() {
            let port = surface
                .iosurface
                .mach_port()
                .ok_or("IOSurfaceCreateMachPort failed")?;
            let info = SurfaceInfo {
                client_port: key.0.0,
                generation: spec.generation,
                pool_index: index as u32,
                pool_size: spec.count,
                width: spec.size.width,
                height: spec.size.height,
                format,
                bytes_per_row: surface.iosurface.bytes_per_row(),
            };
            bridge
                .send_surface(SendRight::from_raw(port), &info)
                .map_err(|e| format!("sending surface {index}: {e}"))?;
        }
        let mut stats = self.stats();
        stats.pools += 1;
        stats.surfaces_sent += u64::from(spec.count);
        drop(stats);
        Ok(Live {
            spec,
            ledger: PoolLedger::new(pool.len()),
            pool,
            working,
            source,
        })
    }

    /// How long to wait for a command before there is work.
    fn next_wake(&self) -> Duration {
        let now = now();
        let mut wake = now + 50_000;
        for view in self.views.values() {
            if view.dirty || !view.requests.is_empty() || view.sample_due {
                return Duration::ZERO;
            }
            if view.clock.is_playing() {
                let sample_at = view.last_sample.unwrap_or(now) + SAMPLE_INTERVAL_US;
                wake = wake.min(sample_at);
                let mut schedule = view.schedule.clone();
                match schedule.next(&view.clock, now, self.config.lead) {
                    Next::WaitUntil(at) => wake = wake.min(at.ceil() as i64),
                    Next::Render { .. } | Next::Skip { .. } => {
                        // Waiting for a free slot: a release wakes us; poll
                        // briefly in case it already came.
                        wake = wake.min(now + 1_000);
                    }
                    Next::Idle => {}
                }
            }
        }
        Duration::from_micros((wake - now).max(0) as u64)
    }

    fn work(&mut self) {
        let keys: Vec<Key> = self.views.keys().copied().collect();
        for key in keys {
            self.work_view(key);
        }
    }

    fn work_view(&mut self, key: Key) {
        let lead = self.config.lead;
        // Clock samples.
        if let Some(view) = self.views.get_mut(&key) {
            let t = now();
            let periodic = view.clock.is_playing()
                && view
                    .last_sample
                    .is_none_or(|last| t - last >= SAMPLE_INTERVAL_US);
            if view.sample_due || periodic {
                view.sample_due = false;
                view.last_sample = Some(t);
                let sample = view.clock.sample(t);
                self.event(key, Event::Clock(sample), None);
            }
        }
        // Frame requests (viewer and scrub): shown at the next refresh.
        while let Some((id, request, revision)) = self
            .views
            .get_mut(&key)
            .and_then(|view| view.requests.pop_front())
        {
            self.answer(key, id, &request, revision);
        }
        // A paused frame after a view or transport change.
        let paused_dirty = self
            .views
            .get(&key)
            .is_some_and(|v| v.dirty && v.pool.is_some() && !v.clock.is_playing());
        if paused_dirty {
            let media = self.views[&key].clock.media_at(now());
            if let Some(descriptor) = self.render(key, media, -1, None) {
                if let Some(view) = self.views.get_mut(&key) {
                    view.dirty = false;
                }
                let revision = descriptor.revision;
                self.event(key, Event::Frame(descriptor), Some(revision));
            }
        } else if let Some(view) = self.views.get_mut(&key)
            && view.clock.is_playing()
        {
            view.dirty = false;
        }
        // Playback: render ahead while frames are due and slots are free.
        loop {
            let Some(view) = self.views.get_mut(&key) else {
                return;
            };
            if view.pool.is_none() {
                return;
            }
            let t = now();
            match view.schedule.next(&view.clock, t, lead) {
                Next::Render { k, shown_from } => {
                    if view
                        .pool
                        .as_ref()
                        .is_some_and(|l| l.ledger.free().is_none())
                    {
                        self.stats().pool_waits += 1;
                        return;
                    }
                    let f = view.clock.frame_ticks();
                    let media = view.clock.wrap(k * f);
                    let shown_for = super::schedule::Schedule::frame_us(&view.clock);
                    let rendered = self.render(key, media, k, Some((shown_from, shown_for)));
                    let Some(view) = self.views.get_mut(&key) else {
                        return;
                    };
                    view.schedule.rendered(&view.clock, k);
                    if let Some(descriptor) = rendered {
                        let revision = descriptor.revision;
                        self.event(key, Event::Frame(descriptor), Some(revision));
                    }
                }
                Next::Skip { .. } => {
                    self.stats().late_skipped += 1;
                }
                Next::WaitUntil(_) | Next::Idle => return,
            }
        }
    }

    fn answer(
        &mut self,
        key: Key,
        id: RequestId,
        request: &FrameRequest,
        revision: Option<Revision>,
    ) {
        let Some(view) = self.views.get_mut(&key) else {
            return;
        };
        if let Some(revision) = revision {
            view.revision = view.revision.max(revision);
        }
        let Some(live) = view.pool.as_ref() else {
            self.error(
                key,
                id,
                ErrorCode::Unsupported,
                "The viewer has no surface pool yet.",
            );
            return;
        };
        if request.output_size != live.spec.size {
            self.error(
                key,
                id,
                ErrorCode::InvalidMessage,
                "A viewer frame request must match the viewer state's size.",
            );
            return;
        }
        if live.ledger.free().is_none() {
            self.error(
                key,
                id,
                ErrorCode::BudgetExceeded,
                "Every pool surface is held by the viewer.",
            );
            return;
        }
        let media = request.time.get();
        if let Some(descriptor) = self.render(key, media, -1, None) {
            self.stats().responses += 1;
            let mut envelope = Envelope::new(Message::Response {
                id,
                body: Response::Frame(descriptor.clone()),
            })
            .with_session(key.1);
            envelope.revision = Some(descriptor.revision);
            self.send(key.0, envelope);
        } else {
            self.error(
                key,
                id,
                ErrorCode::Internal,
                "The frame could not be rendered.",
            );
        }
    }

    /// Renders the frame showing `media` into a free surface, waits for the
    /// GPU, and returns its descriptor. `timing` is `(shown_from, shown_for)`
    /// in host µs for timed frames.
    fn render(
        &mut self,
        key: Key,
        media: i64,
        k: i64,
        timing: Option<(f64, f64)>,
    ) -> Option<FrameDescriptor> {
        let trace = self.config.trace;
        let gpu = self.gpu.as_ref()?;
        let view = self.views.get_mut(&key)?;
        let live = view.pool.as_mut()?;
        let index = live.ledger.free()?;
        let seq = view.next_seq;
        let scene = view::scene(&view.state);
        let frame = TestFrame {
            time_s: (media as f64 / TICKS_PER_SECOND as f64) as f32,
            seq,
            view_seq: view.state.view_seq.get(),
            composition: scene.composition,
            marker: scene.marker,
            camera: scene.camera,
        };
        let start = now();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        live.source
            .encode(&gpu.queue, &mut encoder, &live.working, &frame);
        let params = DisplayParams {
            working: WorkingEncoding::Standard,
            canvas: view.state.color_space,
            noise_offset: DisplayParams::noise_offset_for(seq),
        };
        let surface = &live.pool.surfaces[index as usize];
        if let Err(error) = gpu.display.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &live.working,
            surface,
            params,
        ) {
            let message = format!("frame bridge: display transform failed: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message,
            }]);
            return None;
        }
        let submission = gpu.queue.submit([encoder.finish()]);
        let done = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(2)),
        });
        let gpu_done = now();
        if let Err(error) = done {
            let message = format!("frame bridge: the GPU did not finish a frame: {error}");
            (self.sink)(vec![Action::Log {
                level: LogLevel::Error,
                message,
            }]);
            return None;
        }
        let seq_value = Seq::new(seq).ok()?;
        if !live.ledger.present(index, seq_value) {
            return None;
        }
        view.next_seq += 1;
        let duration = timing.map(|_| Ticks::new(view.clock.frame_ticks()).unwrap_or(Ticks::MAX));
        let descriptor = FrameDescriptor {
            pool_index: index,
            pool_generation: Seq::new(u64::from(live.spec.generation)).unwrap_or(Seq::MAX),
            seq: seq_value,
            pts: Ticks::new(media).unwrap_or(Ticks::ZERO),
            revision: view.revision,
            color_space: view.state.color_space,
            format: live.spec.format,
            quality: FrameQuality {
                resolution: match view.state.resolution {
                    PreviewResolution::Automatic => PreviewResolution::Full,
                    other => other,
                },
                representation: Representation::Original,
                draft_3d: false,
                cached: false,
            },
            overlay: Some(scene.overlay),
            depth_id: None,
            view_seq: view.state.view_seq,
            duration,
        };
        let held = live.ledger.held_count() as u32;
        let generation = live.spec.generation;
        let view_seq = view.state.view_seq.get();
        let revision = view.revision.get();
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.frames += 1;
        if timing.is_some() {
            stats.timed_frames += 1;
        } else {
            stats.immediate_frames += 1;
        }
        stats.gpu_us_max = stats.gpu_us_max.max(gpu_done - start);
        stats.gpu_us_total += gpu_done - start;
        stats.max_held = stats.max_held.max(held);
        if trace && stats.trace.len() < TRACE_CAP {
            let (shown_from, shown_for) = timing.unwrap_or((0.0, 0.0));
            stats.trace.push(FrameTrace {
                seq,
                k,
                slot: index,
                generation,
                shown_from: shown_from.round() as i64,
                shown_for: shown_for.round() as i64,
                view_seq,
                revision,
                start,
                gpu_done,
                sent: now(),
                timed: timing.is_some(),
            });
        }
        Some(descriptor)
    }

    fn shutdown(&mut self) {
        let views: Vec<View> = std::mem::take(&mut self.views).into_values().collect();
        for view in views {
            self.retire(view);
        }
        if let Some(bridge) = self.bridge.take() {
            let _ = bridge.goodbye();
        }
    }
}
