//! Playback: the render thread, the ring, and the device that drives the
//! master clock (sections 15 and 16, contract 6).
//!
//! ```text
//! control ──commands──▶ render thread ──chunks──▶ ring ──▶ device callback
//!                        (transport, mixer,                (CoreAudio)
//!                         rate conversion)                      │
//! consumers ◀──────────── ClockSlot ◀── clock samples ◀─────────┘
//! ```
//!
//! The render thread keeps the ring [`PlaybackConfig::ahead_ms`] ahead of
//! the device. A play or seek starts a new epoch: queued audio of the old
//! epoch is dropped by the callback, and the thread pre-rolls (decodes the
//! source blocks of the first [`PlaybackConfig::preroll_ms`]) before it
//! queues the first chunk, so the first buffer the device plays is complete
//! and identical to an offline render. The clock line starts when the
//! device presents that buffer.
//!
//! When the device does not run at the project rate, `rubato::Async` (sinc,
//! the section 16 filter) converts the master to the device rate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pm_eval::Evaluator;
use pm_model::Document;
use pm_protocol::clock::{ClockSample, TransportCommand, TransportState};
use pm_protocol::common::{PlaybackRate, TICKS_PER_SECOND, TickRange};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType};

use crate::clock::{ClockCore, ClockSlot, host_now_us};
use crate::device::OutputStream;
use crate::error::{AudioError, Result};
use crate::graph::{AssetResolver, MixGraph};
use crate::kernel;
use crate::meter::Meters;
use crate::mixer::{MixFormat, Mixer, RenderOptions};
use crate::ring::{CHUNK, Consumer, Ring, Tag};
use crate::shuttle::Shuttle;
use crate::source::SourceCache;
use crate::transport::{Effect, Transport};

/// Groups of control points ([`crate::mixer::READ_GROUP`]) of time-mapped
/// voices the look-ahead bounds per turn of the render loop: 4 to 7 ms of
/// evaluation (0.5 to 0.8 ms a group of a variable-speed voice, measured
/// with `mix_bench` under load), well inside the ring's 80 ms.
const LOOKAHEAD_GROUPS: usize = 8;
/// Maximum source wait, measured in callback frames at the device rate,
/// independent of wall-clock scheduling, project rate, and shuttle speed.
pub const SOURCE_WAIT_MS: u64 = 120;

#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    /// Audio queued ahead of the device.
    pub ahead_ms: u32,
    /// Audio decoded before a play or seek starts.
    pub preroll_ms: u32,
    /// The longest a start waits for that pre-roll in wall time. Device
    /// time bounds it to [`SOURCE_WAIT_MS`] as well; zero leaves only that
    /// device-time hold.
    pub preroll_wait_ms: u32,
    /// Source decoded ahead of the playhead.
    pub lookahead_ms: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        PlaybackConfig {
            ahead_ms: 80,
            preroll_ms: 250,
            preroll_wait_ms: 400,
            lookahead_ms: 3000,
        }
    }
}

/// The device side, called from the device's real-time callback.
pub struct DeviceSide {
    consumer: Consumer,
    core: ClockCore,
    shared: Arc<Shared>,
    pending: Option<ClockSample>,
}

impl DeviceSide {
    /// Fills one device buffer (interleaved, `channels` wide) whose first
    /// frame reaches the device output at host time `host_us`.
    pub fn callback(&mut self, out: &mut [f32], channels: usize, host_us: u64) {
        let heard = self.consumer.fill(out, channels);
        if let Some(h) = heard {
            self.shared
                .start_timing
                .heard(h.tag.epoch, host_us, out.len() / channels.max(1));
            let latency = self.shared.latency_us.load(Ordering::Relaxed);
            if self.shared.reanchor.swap(false, Ordering::Relaxed) {
                self.core.invalidate();
            }
            if let Some(s) = self.core.observe(h, host_us + latency as u64, latency) {
                self.pending = Some(s);
            }
        }
        if let Some(s) = self.pending
            && self.shared.clock.try_store(s)
        {
            self.pending = None;
        }
    }

    /// Chunks of [`CHUNK`] frames queued for the device (a chunk being played
    /// counts whole). For hosts that drive a device themselves, such as a
    /// simulated device that must not outrun a starved test machine.
    pub fn queued_chunks(&self) -> usize {
        self.consumer.ring().queued()
    }

    /// Frames of the current epoch queued for the device: what the next
    /// callback can play without an underrun.
    pub fn queued_frames(&self) -> usize {
        self.consumer.queued_frames()
    }
}

/// Reads the output device's id and presentation latency (µs).
type DeviceWatch = Box<dyn FnMut() -> Option<(u32, u32)> + Send>;

enum Control {
    Command(
        TransportCommand,
        std::sync::mpsc::SyncSender<Result<()>>,
        u64,
    ),
    Project(
        Arc<Document>,
        Arc<MixGraph>,
        Option<pm_time::Rate>,
        Option<i64>,
    ),
    Solo(Option<HashSet<String>>),
    /// Polled about once a second: the output device id and its latency.
    Watch(DeviceWatch),
    Evaluator(Box<Evaluator>),
    Shutdown,
}

struct Shared {
    clock: ClockSlot,
    latency_us: AtomicU32,
    reanchor: AtomicBool,
    meters: Mutex<VecDeque<Meters>>,
    errors: Mutex<ErrorLog>,
    starved: AtomicU64,
    starve_retries: AtomicU64,
    dropped_renders: AtomicU64,
    dropped_voice_frames: AtomicU64,
    device_rate: u32,
    prepared: Mutex<Vec<i64>>,
    start_timing: StartTrace,
}

/// Times of a run's first buffer on the shared monotonic host clock.
/// Separates control delivery, render preparation, callback phase and HAL lead.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTiming {
    pub epoch: u32,
    pub requested_us: u64,
    pub applied_us: u64,
    pub queued_us: u64,
    pub callback_us: u64,
    pub device_host_us: u64,
    pub callback_frames: u64,
}

#[derive(Default)]
struct StartTrace {
    epoch: AtomicU32,
    requested: AtomicU64,
    applied: AtomicU64,
    queued: AtomicU64,
    callback: AtomicU64,
    callback_epoch: AtomicU32,
    device_host: AtomicU64,
    callback_frames: AtomicU64,
}

impl StartTrace {
    fn begin(&self, epoch: u32, requested: u64) {
        self.epoch.store(0, Ordering::Release);
        self.requested.store(requested, Ordering::Relaxed);
        self.applied.store(host_now_us(), Ordering::Relaxed);
        self.queued.store(0, Ordering::Relaxed);
        self.epoch.store(epoch, Ordering::Release);
    }

    fn queued(&self) {
        self.queued.store(host_now_us(), Ordering::Release);
    }

    fn heard(&self, epoch: u32, device_host: u64, frames: usize) {
        if self.epoch.load(Ordering::Acquire) == epoch
            && self.callback_epoch.load(Ordering::Relaxed) != epoch
        {
            self.device_host.store(device_host, Ordering::Relaxed);
            self.callback_frames.store(frames as u64, Ordering::Relaxed);
            self.callback.store(host_now_us(), Ordering::Relaxed);
            self.callback_epoch.store(epoch, Ordering::Release);
        }
    }

    fn latest(&self) -> Option<StartTiming> {
        let epoch = self.epoch.load(Ordering::Acquire);
        let heard_epoch = self.callback_epoch.load(Ordering::Acquire);
        let callback = self.callback.load(Ordering::Relaxed);
        let s = StartTiming {
            epoch,
            requested_us: self.requested.load(Ordering::Relaxed),
            applied_us: self.applied.load(Ordering::Relaxed),
            queued_us: self.queued.load(Ordering::Acquire),
            callback_us: callback,
            device_host_us: self.device_host.load(Ordering::Relaxed),
            callback_frames: self.callback_frames.load(Ordering::Relaxed),
        };
        (epoch != 0
            && heard_epoch == epoch
            && epoch == self.epoch.load(Ordering::Acquire)
            && heard_epoch == self.callback_epoch.load(Ordering::Acquire))
        .then_some(s)
    }
}

/// Pending diagnostics, each reported once per episode however often the
/// host drains this log. Per-voice render errors form an episode while they
/// recur on consecutive renders; decode stalls while their voice is missing
/// (see `VoiceStall`). Whole-mix failures (a render, the converter, the 2x
/// shuttle) form an episode within a run: a play or seek starts a new one,
/// and a whole render that fails again after a successful one is reported
/// again.
#[derive(Default)]
struct ErrorLog {
    pending: Vec<String>,
    /// Whole-mix failures already reported in this run.
    episode: HashSet<String>,
    /// Per-voice render errors met by the latest render.
    render_active: HashSet<String>,
}

impl ErrorLog {
    fn push(&mut self, message: String) {
        if self.episode.insert(message.clone()) {
            self.pending.push(message);
        }
    }

    /// A whole-mix failure stopped recurring: a later one is a new episode.
    fn end(&mut self, message: &str) {
        self.episode.remove(message);
    }

    /// A play, seek or stop: every ongoing episode ends.
    fn new_run(&mut self) {
        self.episode.clear();
        self.render_active.clear();
    }

    fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }
}

/// A read-only view of a [`Playback`]'s clock, for another thread.
#[derive(Clone)]
pub struct ClockHandle(Arc<Shared>);

impl ClockHandle {
    /// The latest clock sample.
    pub fn latest(&self) -> Option<ClockSample> {
        self.0.clock.latest()
    }

    /// Bumped whenever a new clock sample is published.
    pub fn version(&self) -> u64 {
        self.0.clock.version()
    }
}

/// Playback of one composition.
pub struct Playback {
    tx: Sender<Control>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    stream: Option<OutputStream>,
    builder: Mutex<Evaluator>,
    cache: Arc<SourceCache>,
    format: MixFormat,
    ring: Arc<Ring>,
}

/// Statistics for diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackStats {
    pub underruns: u64,
    pub played_frames: u64,
    /// Segments a source miss stopped (each counted once, however often
    /// it was retried).
    pub starved_renders: u64,
    /// Renders of a stopped segment that were still missing a source block
    /// (playback retries every turn of its render loop, about 2 ms).
    pub starve_retries: u64,
    /// Committed segments containing a voice with missing input.
    pub dropped_renders: u64,
    /// Project output frames silenced for missing input (summed over voices).
    pub dropped_voice_frames: u64,
}

impl Playback {
    /// Starts playback on the default output device (CoreAudio through
    /// `cpal`), mixing at `format`. The transport starts stopped at 0.
    pub fn start(
        format: MixFormat,
        cache: Arc<SourceCache>,
        config: PlaybackConfig,
    ) -> Result<Playback> {
        let info = crate::device::default_output_info()?;
        let (mut playback, mut side) = Self::start_headless(
            format,
            cache,
            config,
            info.sample_rate,
            info.channels.min(crate::ring::MAX_CHANNELS),
            info.latency_us(),
        )?;
        let stream = OutputStream::open(move |out, channels, host_us| {
            side.callback(out, channels, host_us)
        })?;
        playback
            .shared
            .latency_us
            .store(stream.info.latency_us(), Ordering::Relaxed);
        let rate = stream.info.sample_rate;
        let _ = playback.tx.send(Control::Watch(Box::new(move || {
            crate::device::default_output_state(rate)
        })));
        playback.stream = Some(stream);
        Ok(playback)
    }

    /// Starts playback without a device: the returned [`DeviceSide`] is
    /// driven by the caller (tests, measurements, a host that owns the
    /// device). `device_rate` and `device_channels` describe the device.
    pub fn start_headless(
        format: MixFormat,
        cache: Arc<SourceCache>,
        config: PlaybackConfig,
        device_rate: u32,
        device_channels: usize,
        latency_us: u32,
    ) -> Result<(Playback, DeviceSide)> {
        if !(8_000..=192_000).contains(&device_rate) {
            return Err(AudioError::Device(format!("device rate {device_rate}")));
        }
        cache.ensure_workers(1);
        let ring_channels = device_channels.clamp(1, crate::ring::MAX_CHANNELS);
        let chunk_ms = CHUNK as f64 * 1000.0 / device_rate as f64;
        let chunks = ((config.ahead_ms as f64 * 2.0 / chunk_ms).ceil() as usize).max(8);
        let ring = Ring::new(chunks, ring_channels);
        let seq = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            clock: ClockSlot::default(),
            latency_us: AtomicU32::new(latency_us),
            reanchor: AtomicBool::new(false),
            meters: Mutex::new(VecDeque::new()),
            errors: Mutex::new(ErrorLog::default()),
            starved: AtomicU64::new(0),
            starve_retries: AtomicU64::new(0),
            dropped_renders: AtomicU64::new(0),
            dropped_voice_frames: AtomicU64::new(0),
            device_rate,
            prepared: Mutex::new(Vec::new()),
            start_timing: StartTrace::default(),
        });
        let (tx, rx) = channel();
        let empty = Arc::new(Document::new(None, 0));
        let graph = Arc::new(MixGraph {
            comp_id: String::new(),
            comps: vec![],
            voices: vec![],
            strips: vec![],
            master: 1.0,
            problems: vec![],
        });
        let mixer = Mixer::new(
            format.clone(),
            empty,
            graph,
            Evaluator::new(),
            cache.clone(),
        );
        let worker = RenderThread {
            rx,
            shared: shared.clone(),
            ring: ring.clone(),
            transport: Transport::new(format.sample_rate),
            pending_segment: None,
            stalls: HashMap::new(),
            startup: None,
            render_failure: None,
            mixer,
            config,
            solo: None,
            converter: None,
            shuttle: None,
            device_rate,
            ring_channels,
            stopped_published: false,
            stop_pending: false,
            epoch_start: None,
            mono_fold: (ring_channels == 1 && format.channels() > 1)
                .then(|| crate::layout::matrix(&format.speakers, &crate::layout::mono()).remove(0)),
            core: ClockCore::new(device_rate, seq.clone()),
            watch: None,
            device_state: None,
            last_watch: std::time::Instant::now(),
        };
        let thread = std::thread::Builder::new()
            .name("pm-audio-render".into())
            .spawn(move || worker.run())
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let side = DeviceSide {
            consumer: Consumer::new(ring.clone()),
            core: ClockCore::new(device_rate, seq),
            shared: shared.clone(),
            pending: None,
        };
        Ok((
            Playback {
                tx,
                shared,
                thread: Some(thread),
                stream: None,
                builder: Mutex::new(Evaluator::new()),
                cache,
                format,
                ring,
            },
            side,
        ))
    }

    /// The evaluators for building mix graphs and for the render thread's
    /// automation (`make` is called twice; configure the expression engine
    /// and hooks in it).
    pub fn set_evaluator(&self, make: impl Fn() -> Evaluator) {
        *self.builder.lock().unwrap_or_else(|e| e.into_inner()) = make();
        let _ = self.tx.send(Control::Evaluator(Box::new(make())));
    }

    /// Plays `comp_id` of `doc` from now on (a new revision, or another
    /// composition). Returns the media that could not be opened.
    pub fn set_project(
        &self,
        doc: Arc<Document>,
        comp_id: &str,
        resolver: &dyn AssetResolver,
    ) -> Result<Vec<(String, String, String)>> {
        let graph = {
            let mut ev = self.builder.lock().unwrap_or_else(|e| e.into_inner());
            MixGraph::build(&doc, comp_id, &mut ev, &self.cache, resolver)?
        };
        let problems = graph.problems.clone();
        let comp = pm_eval::Comp::resolve(&doc, comp_id)?;
        let rate = comp.rate().map(|r| r.rate);
        let dur = comp.duration();
        let end = dur
            .is_finite()
            .then(|| (dur * TICKS_PER_SECOND as f64).round() as i64)
            .filter(|&e| e > 0);
        self.tx
            .send(Control::Project(doc, Arc::new(graph), rate, end))
            .map_err(|_| AudioError::Device("playback stopped".into()))?;
        Ok(problems)
    }

    /// Applies a transport command (contract 6). Returns once the render
    /// thread has applied it; the clock samples that follow are the answer.
    pub fn command(&self, command: TransportCommand) -> Result<()> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(Control::Command(command, reply, host_now_us()))
            .map_err(|_| AudioError::Device("playback stopped".into()))?;
        answer
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| AudioError::Device("the render thread did not answer".into()))?
    }

    /// Follows an output device the host owns: `watch` reports its id and
    /// presentation latency (µs) and is polled about once a second. A change
    /// re-anchors the clock on a new epoch from what is heard (playback
    /// continues). [`Playback::start`] watches the default output itself.
    pub fn set_device_watch(&self, watch: impl FnMut() -> Option<(u32, u32)> + Send + 'static) {
        let _ = self.tx.send(Control::Watch(Box::new(watch)));
    }

    /// The mixer's monitor solo (strip ids heard; empty or `None`: all).
    pub fn set_monitor_solo(&self, strips: Option<HashSet<String>>) {
        let _ = self.tx.send(Control::Solo(strips));
    }

    /// The latest clock sample.
    pub fn clock(&self) -> Option<ClockSample> {
        self.shared.clock.latest()
    }

    /// The clock for another thread (the viewer presenter follows it while
    /// a control thread owns the playback).
    pub fn clock_handle(&self) -> ClockHandle {
        ClockHandle(self.shared.clone())
    }

    /// Bumped whenever a new clock sample is published.
    pub fn clock_version(&self) -> u64 {
        self.shared.clock.version()
    }

    /// The meters for what is heard now (the snapshot nearest the clock's
    /// media position).
    pub fn meters(&self) -> Option<Meters> {
        let clock = self.clock()?;
        let now = pm_protocol::common::HostMicros::new(host_now_us()).ok()?;
        let ticks = clock.media_time_at(now).get();
        let at =
            (ticks as i128 * self.format.sample_rate as i128 / TICKS_PER_SECOND as i128) as i64;
        let meters = self.shared.meters.lock().unwrap_or_else(|e| e.into_inner());
        meters
            .iter()
            .min_by_key(|m| (m.position - at).abs())
            .cloned()
    }

    /// Errors met while rendering (decode or evaluation) since the last
    /// call. Ongoing failures are deduplicated per episode; a recovered
    /// voice can report the same problem again in a later episode.
    pub fn take_errors(&self) -> Vec<String> {
        self.shared
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn stats(&self) -> PlaybackStats {
        PlaybackStats {
            underruns: self.ring.underruns(),
            played_frames: self.ring.played_frames(),
            starved_renders: self.shared.starved.load(Ordering::Relaxed),
            starve_retries: self.shared.starve_retries.load(Ordering::Relaxed),
            dropped_renders: self.shared.dropped_renders.load(Ordering::Relaxed),
            dropped_voice_frames: self.shared.dropped_voice_frames.load(Ordering::Relaxed),
        }
    }

    /// Diagnostic timing of the most recently started device epoch.
    pub fn start_timing(&self) -> Option<StartTiming> {
        self.shared.start_timing.latest()
    }

    /// Paused starts whose source and voice DSP pre-roll is ready (diagnostics).
    pub fn prepared_starts(&self) -> Vec<i64> {
        self.shared
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn device(&self) -> Option<&crate::device::DeviceInfo> {
        self.stream.as_ref().map(|s| &s.info)
    }

    /// The output stream, when playback owns a device.
    pub fn stream(&self) -> Option<&OutputStream> {
        self.stream.as_ref()
    }

    /// The mix bus (project rate and speakers).
    pub fn format(&self) -> &MixFormat {
        &self.format
    }

    /// The device's sample rate.
    pub fn device_rate(&self) -> u32 {
        self.shared.device_rate
    }

    /// Stops the render thread and closes the device.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Control::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(s) = self.stream.take() {
            let _ = s.pause();
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One contiguous media run of a converter stream.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// Stream position (project frames since the stream began) of its first
    /// frame.
    offset: u64,
    frames: u64,
    /// Media ticks of its first frame.
    ticks: i64,
    state: TransportState,
    /// The loop its clock line wraps by.
    loop_range: Option<TickRange>,
    /// Media project samples per stream frame (J/K/L: ±1, ±2).
    rate: i64,
}

/// Project-to-device rate conversion with its media bookkeeping.
///
/// One epoch is one continuous stream through the filter: a loop wrap (a
/// jump in media time inside an epoch) continues the stream, so the device
/// hears the band-limited concatenation of the passes, exactly as a device
/// at the project rate would hear the passes back to back, and nothing is
/// cut at the wrap. Device frame `j` of the stream (after the filter's
/// delay is dropped) sits at stream position
/// `(j + delay + 1) × project / device − taps / 2` project frames, rubato's
/// exact phase for its first output (the delay it reports is rounded down
/// to whole frames, so the stream starts a fraction of a project frame in);
/// the run holding that position gives its media time, and chunks are
/// split where runs change so every ring chunk is linear in media time. A new epoch starts a new stream; the end of a run
/// flushes the filter's tail and the partial chunk ([`Converter::flush`]).
struct Converter {
    rs: Async<f32>,
    project: u32,
    device: u32,
    /// One filter input chunk, filled across segments: the filter consumes
    /// whole chunks (a partial one would insert silence into the stream).
    input: Vec<Vec<f32>>,
    input_fill: usize,
    output: Vec<Vec<f32>>,
    /// Device frames still to drop (the filter's delay) at a stream start.
    skip: usize,
    /// The whole delay dropped at a stream start.
    delay: u64,
    /// Converted device frames not yet queued (planar).
    pending: Vec<Vec<f32>>,
    /// Stream device frame of `pending[0]`.
    emitted: u64,
    /// Project frames fed into the stream.
    fed: u64,
    runs: VecDeque<Run>,
    /// The epoch of the stream, `None` before its first segment.
    epoch: Option<u32>,
    /// Project sample the last run continues from.
    next_input: Option<i64>,
}

impl Converter {
    fn new(project: u32, device: u32, channels: usize) -> Result<Converter> {
        let params = SincInterpolationParameters::new(kernel::TAPS, kernel::WINDOW)
            .oversampling_factor(256)
            .interpolation(SincInterpolationType::Cubic);
        let rs = Async::<f32>::new_sinc(
            device as f64 / project as f64,
            1.0,
            &params,
            CHUNK,
            channels,
            FixedAsync::Input,
        )
        .map_err(|e| AudioError::Device(e.to_string()))?;
        let out_max = rs.output_frames_max();
        // The stream phase below follows rubato's sinc: its delay is
        // ⌊taps × ratio / 2⌋ and its first output reads one step past
        // −(taps − 1).
        debug_assert_eq!(
            rs.output_delay() as u64,
            (kernel::TAPS as u64 * device as u64) / (2 * project as u64)
        );
        Ok(Converter {
            skip: rs.output_delay(),
            delay: rs.output_delay() as u64,
            rs,
            project,
            device,
            input: vec![vec![0.0; CHUNK]; channels],
            input_fill: 0,
            output: vec![vec![0.0; out_max]; channels],
            pending: vec![Vec::new(); channels],
            emitted: 0,
            fed: 0,
            runs: VecDeque::new(),
            epoch: None,
            next_input: None,
        })
    }

    /// Drops the stream (a new epoch): the filter starts from silence.
    fn reset(&mut self) {
        self.rs.reset();
        self.skip = self.rs.output_delay();
        for p in &mut self.pending {
            p.clear();
        }
        self.input_fill = 0;
        self.emitted = 0;
        self.fed = 0;
        self.runs.clear();
        self.epoch = None;
        self.next_input = None;
    }

    /// Stream position of stream device frame `j`, in units of 1/device of a
    /// project frame.
    fn position(&self, j: u64) -> i128 {
        (j + self.delay + 1) as i128 * self.project as i128
            - (kernel::TAPS / 2) as i128 * self.device as i128
    }

    /// The first stream device frame at or after stream position `frames`.
    fn device_frame(&self, frames: u64) -> u64 {
        let (p, d) = (self.project as i128, self.device as i128);
        let at = (frames as i128 + (kernel::TAPS / 2) as i128) * d;
        let j = (at + p - 1).div_euclid(p) - self.delay as i128 - 1;
        j.max(0) as u64
    }

    /// Converts one segment (project samples `[start, start + frames)` at
    /// media `ticks`) and appends the chunks it completes to `out`.
    #[allow(clippy::too_many_arguments)]
    fn feed(
        &mut self,
        planar: &[Vec<f32>],
        start: i64,
        frames: usize,
        ticks: i64,
        epoch: u32,
        state: TransportState,
        loop_range: Option<TickRange>,
        rate: i64,
        ring_channels: usize,
        out: &mut Vec<(Tag, Vec<f32>)>,
    ) {
        if self.epoch != Some(epoch) {
            self.reset();
            self.epoch = Some(epoch);
        }
        match self.runs.back_mut() {
            Some(run)
                if self.next_input == Some(start)
                    && run.state == state
                    && run.loop_range == loop_range
                    && run.rate == rate =>
            {
                run.frames += frames as u64;
            }
            _ => self.runs.push_back(Run {
                offset: self.fed,
                frames: frames as u64,
                ticks,
                state,
                loop_range,
                rate,
            }),
        }
        self.next_input = Some(start + frames as i64 * rate);
        let mut done = 0;
        while done < frames {
            let at = self.input_fill;
            let n = (frames - done).min(CHUNK - at);
            for (dst, src) in self.input.iter_mut().zip(planar) {
                dst[at..at + n].copy_from_slice(&src[done..done + n]);
            }
            done += n;
            self.fed += n as u64;
            self.input_fill += n;
            if self.input_fill == CHUNK {
                self.input_fill = 0;
                self.process();
            }
        }
        self.drain(false, ring_channels, out);
    }

    /// Ends the stream (the run ended): pushes the filter's tail through
    /// and queues every frame up to the stream's end, the last chunk
    /// partial.
    fn flush(&mut self, ring_channels: usize, out: &mut Vec<(Tag, Vec<f32>)>) {
        if self.epoch.is_none() || self.fed == 0 {
            self.reset();
            return;
        }
        let target = self.device_frame(self.fed);
        // Silence follows the run's last frame through the filter until its
        // tail is out (about half the filter; bound the loop all the same).
        for _ in 0..64 {
            if self.emitted + self.pending[0].len() as u64 >= target {
                break;
            }
            for dst in self.input.iter_mut() {
                dst[self.input_fill..].fill(0.0);
            }
            self.input_fill = 0;
            if !self.process() {
                break;
            }
        }
        let keep = target.saturating_sub(self.emitted) as usize;
        for p in &mut self.pending {
            p.truncate(keep);
        }
        self.drain(true, ring_channels, out);
        self.reset();
    }

    /// Runs the filter over the input chunk into `pending`.
    fn process(&mut self) -> bool {
        let channels = self.input.len();
        let in_adapter = SequentialSliceOfVecs::new(&self.input, channels, CHUNK).expect("input");
        let out_len = self.output[0].len();
        let mut out_adapter =
            SequentialSliceOfVecs::new_mut(&mut self.output, channels, out_len).expect("output");
        let Ok((_, written)) = self
            .rs
            .process_into_buffer(&in_adapter, &mut out_adapter, None)
        else {
            return false;
        };
        let drop = self.skip.min(written);
        self.skip -= drop;
        for (p, o) in self.pending.iter_mut().zip(&self.output) {
            p.extend_from_slice(&o[drop..written]);
        }
        true
    }

    /// Moves pending frames into ring chunks: whole chunks, split where the
    /// media run changes; with `all`, the partial rest as well.
    fn drain(&mut self, all: bool, ring_channels: usize, out: &mut Vec<(Tag, Vec<f32>)>) {
        let (p, d) = (self.project as i128, self.device as i128);
        let epoch = self.epoch.unwrap_or(0);
        loop {
            // Drop runs the stream has played past.
            while let Some(run) = self.runs.front() {
                if self.runs.len() > 1 && self.emitted >= self.device_frame(run.offset + run.frames)
                {
                    self.runs.pop_front();
                } else {
                    break;
                }
            }
            let Some(&run) = self.runs.front() else { break };
            let avail = self.pending[0].len();
            // The last run may still grow; an earlier one ends where the next
            // begins.
            let to_end = if self.runs.len() > 1 || all {
                self.device_frame(run.offset + run.frames)
                    .saturating_sub(self.emitted) as usize
            } else {
                usize::MAX
            };
            let want = CHUNK.min(to_end);
            let n = if avail >= want {
                want
            } else if all {
                avail
            } else {
                break;
            };
            if n == 0 {
                if self.runs.len() > 1 {
                    self.runs.pop_front();
                    continue;
                }
                break;
            }
            let mut inter = Vec::with_capacity(n * ring_channels);
            for i in 0..n {
                for ch in 0..ring_channels {
                    inter.push(self.pending.get(ch).map_or(0.0, |q| q[i]));
                }
            }
            for q in &mut self.pending {
                q.drain(..n);
            }
            // Device frame j sits `position(j)/d − offset` project frames
            // into its run.
            let into = self.position(self.emitted) - run.offset as i128 * d;
            let media = run.ticks as i128
                + (into * TICKS_PER_SECOND as i128 * run.rate as i128).div_euclid(p * d);
            out.push((
                Tag {
                    media_ticks: media as i64,
                    epoch,
                    state: run.state,
                    frames: n as u32,
                    loop_range: run.loop_range,
                    rate: PlaybackRate {
                        num: run.rate as i32,
                        den: 1,
                    },
                },
                inter,
            ));
            self.emitted += n as u64;
        }
    }
}

/// The ring depth `fill` keeps, in chunks: `ahead` frames, and at least
/// one callback plus half of that beyond the refill reserve (two of the
/// largest observed callbacks). The callback waking the producer has already
/// spent one buffer; even then a newly missing voice keeps a wait budget.
/// The producer grows the ring before computing this bounded target.
fn fill_target(ahead: usize, reserve: usize, capacity: usize) -> usize {
    // The callback that wakes the producer has already spent one buffer.
    // Keep that buffer in addition to the two-buffer refill reserve, so
    // every newly missing voice still gets a wait, regardless of alignment.
    let frames = ahead.max(reserve + reserve / 2 + ahead / 2);
    (frames / CHUNK).max(2).min(capacity - 1)
}

/// One voice's missing-input episode, in device frames
/// ([`Ring::played_frames`]).
struct VoiceStall {
    /// When the episode began: the miss, or the pre-roll of a cold start.
    start: u64,
    /// When the voice drops. Never later than `start` plus
    /// [`SOURCE_WAIT_MS`] (the A3 hold).
    deadline: u64,
    /// Committed as silence: stays dropped until the voice renders whole.
    dropped: bool,
    reported: bool,
}

/// Why a voice's wait ended, for its diagnostic.
fn stall_message(id: &str, by_reserve: bool) -> String {
    if by_reserve {
        format!(
            "Audio decode fell behind the output buffer; missing voice plays as silence so the others keep playing: {id}"
        )
    } else {
        format!("Audio decode exceeded {SOURCE_WAIT_MS} ms; missing voice plays as silence: {id}")
    }
}

struct RenderThread {
    rx: Receiver<Control>,
    shared: Arc<Shared>,
    ring: Arc<Ring>,
    transport: Transport,
    /// An undecoded segment is retried before advancing the transport.
    /// Never feed its zero-filled samples into the shuttle or converter.
    pending_segment: Option<crate::transport::Segment>,
    /// Each voice owns its deadline and notification until it recovers.
    stalls: HashMap<String, VoiceStall>,
    /// The device frame a play or seek began pre-rolling, until its first
    /// device output chunk is queued. Until then, holding for a
    /// cold voice is start latency, not a gap: every missing voice gets the
    /// whole A3 hold from here.
    startup: Option<u64>,
    /// The whole-render failure of the latest render, if it failed.
    render_failure: Option<String>,
    mixer: Mixer,
    config: PlaybackConfig,
    solo: Option<HashSet<String>>,
    converter: Option<Converter>,
    /// The 2× J/K/L stretch, warmed during the first audible pause.
    shuttle: Option<Shuttle>,
    device_rate: u32,
    ring_channels: usize,
    stopped_published: bool,
    /// A run ended; publish the stopped clock once the ring has played out.
    stop_pending: bool,
    /// The epoch of the latest restart and the media sample it starts at:
    /// what is heard until the device presents that epoch's first buffer.
    epoch_start: Option<(u32, i64)>,
    /// A mono device: the bus folded to one channel by the layout rules
    /// (a stereo bus as ½(L + R)), not its left channel alone.
    mono_fold: Option<Vec<f32>>,
    core: ClockCore,
    watch: Option<DeviceWatch>,
    device_state: Option<(u32, u32)>,
    last_watch: std::time::Instant,
}

impl RenderThread {
    fn run(mut self) {
        let project = self.transport.rate();
        if project != self.device_rate {
            match Converter::new(project, self.device_rate, self.mixer.format().channels()) {
                Ok(c) => self.converter = Some(c),
                Err(e) => self.error(e.to_string()),
            }
        }
        self.publish_stopped();
        let mut need_preroll = false;
        let mut last_prefetch: Option<i64> = None;
        let mut lookahead_pending = false;
        loop {
            // Controls.
            let wait = if self.transport.is_running() {
                Duration::from_millis(2)
            } else {
                Duration::from_millis(20)
            };
            match self.rx.recv_timeout(wait) {
                Ok(Control::Shutdown) => return,
                Ok(c) => {
                    if self.control(c) {
                        need_preroll = true;
                    }
                    while let Ok(c) = self.rx.try_recv() {
                        if matches!(c, Control::Shutdown) {
                            return;
                        }
                        if self.control(c) {
                            need_preroll = true;
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            if self.last_watch.elapsed() >= Duration::from_secs(1) && self.device_changed() {
                need_preroll = true;
            }
            if !self.transport.is_running() {
                if self.stop_pending && self.ring.queued() == 0 {
                    self.stop_pending = false;
                    self.publish_stopped();
                }
                self.prepare_paused();
                continue;
            }
            if need_preroll {
                need_preroll = false;
                self.preroll();
                last_prefetch = None;
            }
            self.fill();
            // Keep the sources decoding ahead of the playhead. Bounding a
            // time-mapped voice's reads evaluates its control points, so a
            // long look-ahead is spread over the loop's turns (the ring stays
            // full between them).
            let position = self.transport.position();
            let rate = self.transport.rate() as i64;
            if lookahead_pending
                || last_prefetch.is_none_or(|last| (position - last).abs() > rate / 4)
            {
                // J/K/L reads ahead in its direction, as far as it plays.
                let speed = self.transport.play_rate();
                let ahead = self.config.lookahead_ms as i64 * rate / 1000 * speed.abs().max(1);
                let from = if speed < 0 {
                    position - ahead
                } else {
                    position
                };
                lookahead_pending =
                    !self
                        .mixer
                        .prefetch_within(from, ahead as usize, LOOKAHEAD_GROUPS);
                last_prefetch = Some(position);
            }
        }
    }

    fn error(&self, e: String) {
        self.shared
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(e);
    }

    fn report_render_errors(&self, errors: Vec<(String, String)>) {
        let mut log = self.shared.errors.lock().unwrap_or_else(|e| e.into_inner());
        let active: HashSet<_> = errors
            .into_iter()
            .map(|(id, e)| format!("{id}: {e}"))
            .collect();
        for message in &active {
            if !log.render_active.contains(message) {
                log.pending.push(message.clone());
            }
        }
        log.render_active = active;
    }

    /// The media position heard now, from the published clock. Until the
    /// device presents the current epoch (a seek's pre-roll), the listener
    /// is at the epoch's start, not on the old line.
    fn heard_sample(&self) -> Option<i64> {
        let clock = self.shared.clock.latest()?;
        if clock.epoch != self.transport.epoch() {
            return self
                .epoch_start
                .filter(|(e, _)| *e == self.transport.epoch())
                .map(|(_, at)| at);
        }
        if clock.state == TransportState::Stopped {
            return None;
        }
        let now = pm_protocol::common::HostMicros::new(host_now_us()).ok()?;
        let ticks = clock.media_time_at(now).get();
        Some(crate::offline::sample_of_ticks(
            ticks,
            self.transport.rate(),
        ))
    }

    /// Applies a control; returns whether playback must pre-roll.
    fn control(&mut self, c: Control) -> bool {
        self.shared
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        match c {
            Control::Command(cmd, reply, requested_us) => {
                // A pause, a step, a play around, or a new J/K/L rate while
                // running starts from what is heard.
                if self.transport.is_running()
                    && matches!(
                        cmd,
                        TransportCommand::Pause
                            | TransportCommand::Step { .. }
                            | TransportCommand::PlayAround { .. }
                            | TransportCommand::Play { .. }
                    )
                    && let Some(heard) = self.heard_sample()
                {
                    self.transport.set_position(heard);
                }
                let result = self.transport.apply(&cmd);
                let restart = match &result {
                    Ok(Effect::Restart) => {
                        self.shared
                            .start_timing
                            .begin(self.transport.epoch(), requested_us);
                        self.epoch_start =
                            Some((self.transport.epoch(), self.transport.position()));
                        self.ring.set_epoch(self.transport.epoch());
                        self.stopped_published = false;
                        self.stop_pending = false;
                        true
                    }
                    Ok(Effect::Stopped) => {
                        // A stop drops the run's starved segment: the next
                        // run renders from its own position.
                        self.pending_segment = None;
                        self.stalls.clear();
                        self.startup = None;
                        self.render_failure = None;
                        self.shared
                            .errors
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .new_run();
                        self.mixer.abandon_retries();
                        self.ring.set_epoch(self.transport.epoch());
                        self.stopped_published = false;
                        self.stop_pending = false;
                        self.publish_stopped();
                        false
                    }
                    _ => false,
                };
                let _ = reply.send(result.map(|_| ()));
                restart
            }
            Control::Project(doc, graph, rate, end) => {
                // A live edit glides into the new mix (a retune); paused, the
                // next run starts on it.
                if self.transport.is_running() {
                    self.mixer.glide_to(doc, graph, self.solo.clone());
                } else {
                    self.mixer.set_graph(doc, graph);
                }
                self.transport.set_composition(rate, end);
                false
            }
            Control::Solo(s) => {
                if self.transport.is_running() && s != self.solo {
                    let (doc, graph) = (self.mixer.document().clone(), self.mixer.graph().clone());
                    self.mixer.glide_to(doc, graph, self.solo.clone());
                }
                self.solo = s;
                false
            }
            Control::Evaluator(ev) => {
                self.mixer.set_evaluator(*ev);
                false
            }
            Control::Watch(w) => {
                self.watch = Some(w);
                self.device_state = self.watch.as_mut().and_then(|w| w());
                false
            }
            Control::Shutdown => false,
        }
    }

    /// Follows the output device (contract 6, open question 3: playback
    /// continues on a new device with a new epoch). Returns whether playback
    /// restarted.
    fn device_changed(&mut self) -> bool {
        self.last_watch = std::time::Instant::now();
        let Some(watch) = self.watch.as_mut() else {
            return false;
        };
        let now = watch();
        if now.is_none() || now == self.device_state {
            return false;
        }
        self.device_state = now;
        if let Some((_, latency)) = now {
            self.shared.latency_us.store(latency, Ordering::Relaxed);
        }
        if !self.transport.is_running() {
            self.stopped_published = false;
            self.publish_stopped();
            return false;
        }
        // Re-anchor on a new epoch from what is heard now.
        let at = self.heard_sample().unwrap_or(self.transport.position());
        let ticks =
            pm_protocol::common::Ticks::saturating_from_i128(self.transport.ticks_of(at) as i128);
        if self
            .transport
            .apply(&TransportCommand::Seek {
                time: ticks,
                mode: pm_protocol::clock::SeekMode::Exact,
            })
            .is_ok()
        {
            self.epoch_start = Some((self.transport.epoch(), self.transport.position()));
            self.ring.set_epoch(self.transport.epoch());
            self.shared.reanchor.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// Publishes a stopped line at the transport's position. Every stop
    /// (pause, seek while paused, the end of a run) has moved the transport
    /// to a new epoch first, so a sample of the run the device callback
    /// stores afterwards (a higher `seq` on the older epoch) never replaces
    /// it: the slot orders by `(epoch, seq)`.
    fn publish_stopped(&mut self) {
        if self.stopped_published {
            return;
        }
        let ticks = self.transport.ticks_of(self.transport.position());
        let latency = self.shared.latency_us.load(Ordering::Relaxed);
        let s = self
            .core
            .stopped(self.transport.epoch(), ticks, host_now_us(), latency);
        self.shared.clock.store(s);
        self.stopped_published = true;
    }

    /// Idle work is cancellable between turns: decode around the playhead
    /// first, then both work-area edges. Each target has its own vocoder.
    fn prepare_paused(&mut self) {
        if self.stop_pending {
            return;
        }
        if self.shuttle.is_none() && !self.mixer.graph().voices.is_empty() {
            // Master FFT plans/workspace need no project input. Allocate and
            // exercise them off the Play path without advancing the mix.
            match Shuttle::new(self.mixer.format().channels(), self.transport.rate()) {
                Ok(mut shuttle) => {
                    if let Err(error) = shuttle.warm() {
                        self.error(error.to_string());
                    }
                    self.shuttle = Some(shuttle);
                }
                Err(error) => self.error(error.to_string()),
            }
        }
        let rate = self.transport.rate() as i64;
        let mut starts = vec![self.transport.position()];
        if let Some(range) = self.transport.loop_range() {
            starts.push(crate::offline::sample_of_ticks(
                range.start.get(),
                rate as u32,
            ));
            starts.push(
                crate::offline::sample_of_ticks(range.end.get(), rate as u32)
                    .saturating_sub(CHUNK as i64)
                    .max(0),
            );
        }
        if let Ok(comp) = pm_eval::Comp::resolve(self.mixer.document(), &self.mixer.graph().comp_id)
            && let Some(work) = pm_eval::node::array(comp.member("work"))
        {
            for edge in work.iter().take(2) {
                if let Some(seconds) = pm_eval::node::number(Some(edge)).filter(|t| t.is_finite()) {
                    let at = (seconds * rate as f64).round() as i64;
                    // Bound retained DSP histories to the playhead and
                    // work-area boundaries, regardless of work-area length.
                    if starts.len() < 3 && !starts.contains(&at) {
                        starts.push(at.max(0));
                    }
                }
            }
        }
        starts.dedup();
        self.mixer.retain_prepared(&starts);
        let ahead = (self.config.lookahead_ms.max(self.config.preroll_ms) as i64 * rate / 1000)
            .max(CHUNK as i64);
        let mut ready = Vec::new();
        for start in starts {
            self.mixer
                .prefetch_within(start - 2 * ahead, (2 * ahead) as usize, LOOKAHEAD_GROUPS);
            // Read bounds include stretch's deterministic pre-roll. Do not
            // block commands evaluating a large work area all in one turn.
            if !self
                .mixer
                .prefetch_within(start, ahead as usize, LOOKAHEAD_GROUPS)
            {
                continue;
            }
            let first = (self.config.preroll_ms as usize * rate as usize / 1000).max(CHUNK);
            if self.mixer.is_ready(start, first) && self.mixer.prepare_start(start) {
                ready.push(start);
            }
        }
        *self
            .shared
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = ready;
    }

    /// Decodes what the first moments of playback need before queueing.
    fn preroll(&mut self) {
        self.pending_segment = None;
        self.stalls.clear();
        self.render_failure = None;
        self.shared
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .new_run();
        // A seek or a new run starts on the current mix, and its first
        // render is a new request, not a retry of a starved one.
        self.mixer.end_glides();
        self.mixer.abandon_retries();
        let position = self.transport.position();
        if self.transport.play_rate() == 1 {
            self.mixer.use_prepared(position);
        }
        let rate = self.transport.rate() as i64;
        let speed = self.transport.play_rate();
        let frames = self.config.preroll_ms as i64 * rate / 1000 * speed.abs().max(1);
        let from = if speed < 0 {
            position - frames
        } else {
            position
        };
        let frames = frames as usize;
        self.mixer.prefetch(from, frames);
        // The A3 start hold: wait for every voice's pre-roll, for at most
        // `preroll_wait_ms` of wall time and SOURCE_WAIT_MS of device time.
        // Nothing of this run is queued, so no ready voice goes silent while
        // it waits. `fill` keeps holding a still-missing first segment by
        // device time from the same frame.
        let device_start = self.ring.played_frames();
        self.startup = Some(device_start);
        let deadline =
            std::time::Instant::now() + Duration::from_millis(self.config.preroll_wait_ms as u64);
        while !self.mixer.is_ready(from, frames)
            && std::time::Instant::now() < deadline
            && self.ring.played_frames().saturating_sub(device_start)
                < self.device_rate as u64 * SOURCE_WAIT_MS / 1000
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(c) = &mut self.converter {
            c.reset();
        }
        if let Some(s) = &mut self.shuttle {
            s.reset();
        }
    }

    /// Renders until the ring holds its callback-adapted look-ahead target.
    fn fill(&mut self) {
        let ahead = self.config.ahead_ms as usize * self.device_rate as usize / 1000;
        let reserve = self.ring.refill_reserve() as usize;
        // Callback only observes its size. Allocation and growth stay on the
        // producer, preserving queued audio across first observation/changes.
        self.ring
            .grow((ahead.max(reserve + reserve / 2 + ahead / 2)).div_ceil(CHUNK) + 2);
        let target = fill_target(
            ahead,
            self.ring.refill_reserve() as usize,
            self.ring.capacity(),
        );
        let channels = self.mixer.format().channels();
        let mut planar = vec![Vec::new(); channels];
        while self.ring.queued() < target {
            let retry = self.pending_segment.is_some();
            let Some(seg) = self
                .pending_segment
                .take()
                .or_else(|| self.transport.next(CHUNK))
            else {
                if self.transport.take_stop() == Effect::Stopped {
                    // The shuttle's and the converter's tails belong to the run.
                    self.finish_shuttle();
                    self.flush_converter();
                    // Let what is queued play out; the clock stops at the
                    // run's end position once it has.
                    self.stopped_published = false;
                    self.stop_pending = true;
                }
                return;
            };
            // The samples the segment plays, mixed forward; reverse runs
            // play them backwards. Reverse renders each chunk on its own,
            // so stretched voices resample there (see RenderOptions).
            let (lo, hi) = seg.media_span();
            let now = self.ring.played_frames();
            let mut options = RenderOptions {
                wait: false,
                drop_starved: false,
                drop_voices: self
                    .stalls
                    .iter()
                    .filter(|(_, stall)| stall.dropped || now >= stall.deadline)
                    .map(|(id, _)| id.clone())
                    .collect(),
                monitor_solo: self.solo.clone(),
                meters: true,
                no_stretch: seg.rate < 0,
            };
            let n = (hi - lo) as usize;
            let mut result = self.mixer.render(lo, n, &mut planar, &options);
            if let Ok(report) = &result {
                let now = self.ring.played_frames();
                let hold = self.device_rate as u64 * SOURCE_WAIT_MS / 1000;
                // A6: every mid-run miss is bounded by the ring reserve,
                // including an all-missing mix. Tagged silence advances the
                // device clock and picture; only a cold start keeps its hold.
                let protects = self.startup.is_none();
                let reserve = self
                    .ring
                    .buffered_frames()
                    .saturating_sub(self.ring.refill_reserve());
                self.stalls
                    .retain(|id, _| report.starved_voices.contains(id));
                for id in &report.starved_voices {
                    let start = self.startup.unwrap_or(now);
                    let stall = self.stalls.entry(id.clone()).or_insert(VoiceStall {
                        start,
                        deadline: start.saturating_add(hold),
                        dropped: false,
                        reported: false,
                    });
                    let hard = stall.start.saturating_add(hold);
                    stall.deadline = if protects {
                        // The reserve can shrink faster than first estimated
                        // (large callbacks / downstream DSP latency).
                        stall.deadline.min(now.saturating_add(reserve)).min(hard)
                    } else {
                        hard
                    };
                    if stall.dropped || now >= stall.deadline {
                        options.drop_voices.insert(id.clone());
                    }
                }
                // A newly expired voice must render with sample-validity
                // masking. Do not commit its speculative incomplete output.
                if report
                    .waiting_voices
                    .iter()
                    .any(|id| options.drop_voices.contains(id))
                {
                    result = self.mixer.render(lo, n, &mut planar, &options);
                }
            }
            match result {
                Ok(report) => {
                    if let Some(failure) = self.render_failure.take() {
                        self.shared
                            .errors
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .end(&failure);
                    }
                    self.report_render_errors(report.errors);
                    if report.starved > 0 {
                        let counter = if retry {
                            &self.shared.starve_retries
                        } else {
                            &self.shared.starved
                        };
                        counter.fetch_add(1, Ordering::Relaxed);
                        if !report.waiting_voices.is_empty() {
                            self.pending_segment = Some(seg);
                            return;
                        }
                        let mut notifications = Vec::new();
                        let hold = self.device_rate as u64 * SOURCE_WAIT_MS / 1000;
                        for id in &report.starved_voices {
                            if let Some(stall) = self.stalls.get_mut(id) {
                                stall.dropped = true;
                                if !stall.reported {
                                    stall.reported = true;
                                    let by_reserve =
                                        stall.deadline < stall.start.saturating_add(hold);
                                    notifications.push(stall_message(id, by_reserve));
                                }
                            }
                        }
                        let mut errors =
                            self.shared.errors.lock().unwrap_or_else(|e| e.into_inner());
                        errors.pending.extend(notifications);
                        self.shared.dropped_renders.fetch_add(1, Ordering::Relaxed);
                        self.shared
                            .dropped_voice_frames
                            .fetch_add(report.dropped_voice_frames, Ordering::Relaxed);
                    } else {
                        self.stalls.clear();
                    }
                }
                Err(e) => {
                    self.stalls.clear();
                    let message = e.to_string();
                    self.error(message.clone());
                    self.render_failure = Some(message);
                    for c in &mut planar {
                        c.clear();
                        c.resize(n, 0.0);
                    }
                }
            }
            if seg.rate < 0 {
                for c in &mut planar {
                    c.reverse();
                }
            }
            if let Some(g) = seg.grain {
                for (i, s) in (seg.start..seg.start + seg.frames as i64).enumerate() {
                    let gain = self.transport.grain_gain(g, s);
                    for c in &mut planar {
                        c[i] *= gain;
                    }
                }
            }
            {
                let mut meters = self.shared.meters.lock().unwrap_or_else(|e| e.into_inner());
                let at = if seg.rate < 0 { lo } else { hi };
                meters.push_back(self.mixer.meters(at));
                while meters.len() > 64 {
                    meters.pop_front();
                }
            }
            if seg.rate.abs() == 2 {
                let mut pieces = Vec::new();
                let shuttle = match self.shuttle.as_mut() {
                    Some(s) => s,
                    None => match Shuttle::new(channels, self.transport.rate()) {
                        Ok(s) => self.shuttle.insert(s),
                        Err(e) => {
                            self.error(e.to_string());
                            continue;
                        }
                    },
                };
                if let Err(e) = shuttle.push(
                    &planar,
                    seg.start,
                    seg.rate.signum(),
                    seg.epoch,
                    seg.state,
                    seg.loop_range,
                    &mut pieces,
                ) {
                    self.error(e.to_string());
                }
                for p in pieces {
                    self.emit(p.planar, p.start, p.rate, seg.epoch, p.state, p.loop_range);
                }
            } else {
                let out = std::mem::replace(&mut planar, vec![Vec::new(); channels]);
                self.emit(
                    out,
                    seg.start,
                    seg.rate,
                    seg.epoch,
                    seg.state,
                    seg.loop_range,
                );
            }
        }
    }

    /// Queues output frames (planar, at the project rate) that play media
    /// from sample `start` at `rate` samples a frame: folded for a mono
    /// device, converted to the device rate, in ring chunks.
    fn emit(
        &mut self,
        mut planar: Vec<Vec<f32>>,
        start: i64,
        rate: i64,
        epoch: u32,
        state: TransportState,
        loop_range: Option<TickRange>,
    ) {
        let frames = planar.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if let Some(row) = &self.mono_fold {
            for i in 0..frames {
                let v: f32 = row.iter().zip(&planar).map(|(m, c)| m * c[i]).sum();
                planar[0][i] = v;
            }
        }
        let ticks = self.transport.ticks_of(start);
        let ring_channels = self.ring_channels;
        if let Some(c) = self.converter.as_mut() {
            let mut chunks = Vec::new();
            c.feed(
                &planar,
                start,
                frames,
                ticks,
                epoch,
                state,
                loop_range,
                rate,
                ring_channels,
                &mut chunks,
            );
            self.push_chunks(chunks);
            return;
        }
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            let mut inter = Vec::with_capacity(n * self.ring_channels);
            for i in done..done + n {
                for c in 0..self.ring_channels {
                    inter.push(planar.get(c).map_or(0.0, |p| p[i]));
                }
            }
            let tag = Tag {
                media_ticks: self.transport.ticks_of(start + done as i64 * rate),
                epoch,
                state,
                frames: n as u32,
                loop_range,
                rate: PlaybackRate {
                    num: rate as i32,
                    den: 1,
                },
            };
            if self.startup.is_some() {
                self.shared.start_timing.queued();
            }
            while !self.ring.push(tag, &inter) {
                std::thread::sleep(Duration::from_millis(1));
            }
            // Stateful DSP can consume many input segments before emitting
            // a device frame. Keep the cold-start hold until output exists.
            self.startup = None;
            done += n;
        }
    }

    /// The end of a 2× run: the stretch's tail, on the run's epoch.
    fn finish_shuttle(&mut self) {
        let mut pieces = Vec::new();
        let Some(s) = self.shuttle.as_mut() else {
            return;
        };
        let Some(epoch) = s.epoch() else {
            return;
        };
        if let Err(e) = s.finish(&mut pieces) {
            self.error(e.to_string());
        }
        for p in pieces {
            self.emit(p.planar, p.start, p.rate, epoch, p.state, p.loop_range);
        }
    }

    fn flush_converter(&mut self) {
        let ring_channels = self.ring_channels;
        let mut chunks = Vec::new();
        if let Some(c) = self.converter.as_mut() {
            c.flush(ring_channels, &mut chunks);
        }
        self.push_chunks(chunks);
    }

    fn push_chunks(&mut self, chunks: Vec<(Tag, Vec<f32>)>) {
        for (tag, inter) in chunks {
            if self.startup.is_some() {
                self.shared.start_timing.queued();
            }
            while !self.ring.push(tag, &inter) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.startup = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_buffer_timing_ignores_other_epochs_and_later_callbacks() {
        let trace = StartTrace::default();
        trace.begin(3, host_now_us());
        trace.queued();
        assert!(trace.latest().is_none());
        trace.heard(2, 10, 512);
        assert!(trace.latest().is_none());
        trace.heard(3, 20, 128);
        let first = trace.latest().unwrap();
        trace.heard(3, 30, 256);
        assert_eq!(trace.latest().unwrap().device_host_us, 20);
        assert_eq!(first.callback_frames, 128);
        assert!(first.requested_us <= first.applied_us);
        assert!(first.applied_us <= first.queued_us);
        assert!(first.queued_us <= first.callback_us);
        trace.begin(4, host_now_us());
        trace.heard(3, 40, 512);
        assert!(trace.latest().is_none());
        trace.queued();
        trace.heard(4, 50, 256);
        assert_eq!(trace.latest().unwrap().device_host_us, 50);
    }

    #[test]
    fn whole_mix_failures_are_reported_once_per_episode() {
        let mut log = ErrorLog::default();
        log.push("mix: broken".into());
        log.push("mix: broken".into());
        assert_eq!(log.take(), ["mix: broken"]);
        // Still failing on later renders and drains: nothing new.
        log.push("mix: broken".into());
        assert!(log.take().is_empty());
        // A successful render ends the episode; a later failure reports.
        log.end("mix: broken");
        log.push("mix: broken".into());
        assert_eq!(log.take(), ["mix: broken"]);
        // So does a new run (converter and shuttle failures included).
        log.push("shuttle: x".into());
        log.new_run();
        log.push("shuttle: x".into());
        log.push("mix: broken".into());
        assert_eq!(log.take(), ["shuttle: x", "shuttle: x", "mix: broken"]);
    }

    #[test]
    fn large_callbacks_keep_a_decode_wait_budget() {
        // 48 kHz, 80 ms ahead (3,840 frames), an expanded ring.
        let (ahead, capacity) = (3_840, 128);
        // 512-frame callbacks: the target is unchanged.
        assert_eq!(fill_target(ahead, 1_024, capacity), 15);
        // 2,048-frame callbacks reserve 4,096 frames, more than the 80 ms
        // target: keep the wake-up callback plus 40 ms beyond the reserve.
        let target = fill_target(ahead, 4_096, capacity);
        assert_eq!(target, 31);
        assert!(target * CHUNK - 4_096 >= 1_664);
        // Never past the ring.
        assert_eq!(fill_target(ahead, 16_384, 30), 29);
        assert_eq!(fill_target(100, 0, capacity), 2);
    }
}
