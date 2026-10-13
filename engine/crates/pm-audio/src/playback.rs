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

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pm_eval::Evaluator;
use pm_model::Document;
use pm_protocol::clock::{ClockSample, TransportCommand, TransportState};
use pm_protocol::common::TICKS_PER_SECOND;
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType};

use crate::clock::{ClockCore, ClockSlot, host_now_us, ticks_per_frame};
use crate::device::OutputStream;
use crate::error::{AudioError, Result};
use crate::graph::{AssetResolver, MixGraph};
use crate::kernel;
use crate::meter::Meters;
use crate::mixer::{MixFormat, Mixer, RenderOptions};
use crate::ring::{CHUNK, Consumer, Ring, Tag};
use crate::source::SourceCache;
use crate::transport::{Effect, Transport};

#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    /// Audio queued ahead of the device.
    pub ahead_ms: u32,
    /// Audio decoded before a play or seek starts.
    pub preroll_ms: u32,
    /// The longest a start waits for that pre-roll.
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
            let latency = self.shared.latency_us.load(Ordering::Relaxed);
            if self.shared.reanchor.swap(false, Ordering::Relaxed) {
                self.core.invalidate();
            }
            self.core.loop_range = self.shared.loop_range();
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
}

/// Reads the output device's id and presentation latency (µs).
type DeviceWatch = Box<dyn FnMut() -> Option<(u32, u32)> + Send>;

enum Control {
    Command(TransportCommand, std::sync::mpsc::SyncSender<Result<()>>),
    Project(
        Arc<Document>,
        Arc<MixGraph>,
        Option<pm_time::Rate>,
        Option<i64>,
    ),
    Solo(Option<HashSet<String>>),
    /// Polled about once a second: the output device id and its latency.
    Watch(DeviceWatch),
    Shutdown,
}

struct Shared {
    clock: ClockSlot,
    latency_us: AtomicU32,
    reanchor: AtomicBool,
    loop_start: AtomicU64,
    loop_end: AtomicU64,
    meters: Mutex<VecDeque<Meters>>,
    errors: Mutex<Vec<String>>,
    starved: AtomicU64,
    device_rate: u32,
}

impl Shared {
    fn loop_range(&self) -> Option<pm_protocol::common::TickRange> {
        let (a, b) = (
            self.loop_start.load(Ordering::Relaxed),
            self.loop_end.load(Ordering::Relaxed),
        );
        (b > a).then(|| pm_protocol::common::TickRange {
            start: pm_protocol::common::Ticks::saturating_from_i128(a as i64 as i128),
            end: pm_protocol::common::Ticks::saturating_from_i128(b as i64 as i128),
        })
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
    pub starved_renders: u64,
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
        let ring_channels = device_channels.clamp(1, crate::ring::MAX_CHANNELS);
        let chunk_ms = CHUNK as f64 * 1000.0 / device_rate as f64;
        let chunks = ((config.ahead_ms as f64 * 2.0 / chunk_ms).ceil() as usize).max(8);
        let ring = Ring::new(chunks, ring_channels);
        let seq = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared {
            clock: ClockSlot::default(),
            latency_us: AtomicU32::new(latency_us),
            reanchor: AtomicBool::new(false),
            loop_start: AtomicU64::new(0),
            loop_end: AtomicU64::new(0),
            meters: Mutex::new(VecDeque::new()),
            errors: Mutex::new(Vec::new()),
            starved: AtomicU64::new(0),
            device_rate,
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
            mixer,
            config,
            solo: None,
            converter: None,
            device_rate,
            ring_channels,
            stopped_published: false,
            stop_pending: false,
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

    /// The evaluator used to build mix graphs (configure its expression
    /// engine here).
    pub fn set_evaluator(&self, ev: Evaluator) {
        *self.builder.lock().unwrap_or_else(|e| e.into_inner()) = ev;
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
            .send(Control::Command(command, reply))
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

    /// Errors met while rendering (decode or evaluation), each once.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.shared.errors.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn stats(&self) -> PlaybackStats {
        PlaybackStats {
            underruns: self.ring.underruns(),
            played_frames: self.ring.played_frames(),
            starved_renders: self.shared.starved.load(Ordering::Relaxed),
        }
    }

    pub fn device(&self) -> Option<&crate::device::DeviceInfo> {
        self.stream.as_ref().map(|s| &s.info)
    }

    /// The output stream, when playback owns a device.
    pub fn stream(&self) -> Option<&OutputStream> {
        self.stream.as_ref()
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

/// Project-to-device rate conversion with its media bookkeeping.
struct Converter {
    rs: Async<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    /// Device frames still to drop (the filter's delay) after a reset.
    skip: usize,
    /// Pending device-rate frames (planar) and the media ticks of the first.
    pending: Vec<Vec<f32>>,
    pending_ticks: i64,
    /// Project sample the converter continues from (contiguity check).
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
        Ok(Converter {
            skip: rs.output_delay(),
            rs,
            input: vec![vec![0.0; CHUNK]; channels],
            output: vec![vec![0.0; out_max]; channels],
            pending: vec![Vec::new(); channels],
            pending_ticks: 0,
            next_input: None,
        })
    }

    fn reset(&mut self, start_ticks: i64) {
        self.rs.reset();
        self.skip = self.rs.output_delay();
        for p in &mut self.pending {
            p.clear();
        }
        self.pending_ticks = start_ticks;
        self.next_input = None;
    }
}

struct RenderThread {
    rx: Receiver<Control>,
    shared: Arc<Shared>,
    ring: Arc<Ring>,
    transport: Transport,
    mixer: Mixer,
    config: PlaybackConfig,
    solo: Option<HashSet<String>>,
    converter: Option<Converter>,
    device_rate: u32,
    ring_channels: usize,
    stopped_published: bool,
    /// A run ended; publish the stopped clock once the ring has played out.
    stop_pending: bool,
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
                continue;
            }
            if need_preroll {
                need_preroll = false;
                self.preroll();
                last_prefetch = None;
            }
            // Keep the sources decoding ahead of the playhead.
            let position = self.transport.position();
            let rate = self.transport.rate() as i64;
            if last_prefetch.is_none_or(|last| (position - last).abs() > rate / 4) {
                self.mixer.prefetch(
                    position,
                    (self.config.lookahead_ms as i64 * rate / 1000) as usize,
                );
                last_prefetch = Some(position);
            }
            self.fill();
        }
    }

    fn error(&self, e: String) {
        let mut errors = self.shared.errors.lock().unwrap_or_else(|e| e.into_inner());
        if !errors.contains(&e) {
            errors.push(e);
        }
    }

    /// The media position heard now, from the published clock.
    fn heard_sample(&self) -> Option<i64> {
        let clock = self.shared.clock.latest()?;
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
        match c {
            Control::Command(cmd, reply) => {
                if self.transport.is_running()
                    && matches!(
                        cmd,
                        TransportCommand::Pause
                            | TransportCommand::Step { .. }
                            | TransportCommand::PlayAround { .. }
                    )
                    && let Some(heard) = self.heard_sample()
                {
                    self.transport.set_position(heard);
                }
                let result = self.transport.apply(&cmd);
                let restart = match &result {
                    Ok(Effect::Restart) => {
                        self.ring.set_epoch(self.transport.epoch());
                        self.stopped_published = false;
                        self.stop_pending = false;
                        true
                    }
                    Ok(Effect::Stopped) => {
                        self.ring.set_epoch(self.transport.epoch());
                        self.stopped_published = false;
                        self.stop_pending = false;
                        self.publish_stopped();
                        false
                    }
                    _ => false,
                };
                if let TransportCommand::SetLoop { .. } = cmd {
                    let r = self.transport.loop_range();
                    self.shared
                        .loop_start
                        .store(r.map_or(0, |r| r.start.get() as u64), Ordering::Relaxed);
                    self.shared
                        .loop_end
                        .store(r.map_or(0, |r| r.end.get() as u64), Ordering::Relaxed);
                }
                let _ = reply.send(result.map(|_| ()));
                restart
            }
            Control::Project(doc, graph, rate, end) => {
                self.mixer.set_graph(doc, graph);
                self.transport.set_composition(rate, end);
                false
            }
            Control::Solo(s) => {
                self.solo = s;
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
            self.ring.set_epoch(self.transport.epoch());
            self.shared.reanchor.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    fn publish_stopped(&mut self) {
        if self.stopped_published {
            return;
        }
        let ticks = self.transport.ticks_of(self.transport.position());
        let latency = self.shared.latency_us.load(Ordering::Relaxed);
        self.core.loop_range = self.shared.loop_range();
        let s = self
            .core
            .stopped(self.transport.epoch(), ticks, host_now_us(), latency);
        self.shared.clock.store(s);
        self.stopped_published = true;
    }

    /// Decodes what the first moments of playback need before queueing.
    fn preroll(&mut self) {
        let position = self.transport.position();
        let rate = self.transport.rate() as i64;
        let frames = (self.config.preroll_ms as i64 * rate / 1000) as usize;
        self.mixer.prefetch(position, frames);
        let deadline =
            std::time::Instant::now() + Duration::from_millis(self.config.preroll_wait_ms as u64);
        while !self.mixer.is_ready(position, frames) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(c) = &mut self.converter {
            c.reset(self.transport.ticks_of(position));
        }
    }

    /// Renders until the ring holds `ahead_ms`.
    fn fill(&mut self) {
        let target =
            ((self.config.ahead_ms as usize * self.device_rate as usize / 1000) / CHUNK).max(2);
        let target = target.min(self.ring.capacity() - 1);
        let channels = self.mixer.format().channels();
        let mut planar = vec![Vec::new(); channels];
        while self.ring.queued() < target {
            let Some(seg) = self.transport.next(CHUNK) else {
                if self.transport.take_stop() == Effect::Stopped {
                    // Let what is queued play out; the clock stops at the
                    // run's end position once it has.
                    self.stopped_published = false;
                    self.stop_pending = true;
                }
                return;
            };
            let options = RenderOptions {
                wait: !self.mixer.cache().has_workers(),
                monitor_solo: self.solo.clone(),
                meters: true,
            };
            match self
                .mixer
                .render(seg.start, seg.frames, &mut planar, &options)
            {
                Ok(report) => {
                    if report.starved > 0 {
                        self.shared.starved.fetch_add(1, Ordering::Relaxed);
                    }
                    for (id, e) in report.errors {
                        self.error(format!("{id}: {e}"));
                    }
                }
                Err(e) => {
                    self.error(e.to_string());
                    for c in &mut planar {
                        c.clear();
                        c.resize(seg.frames, 0.0);
                    }
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
                meters.push_back(self.mixer.meters(seg.start + seg.frames as i64));
                while meters.len() > 64 {
                    meters.pop_front();
                }
            }
            let ticks = self.transport.ticks_of(seg.start);
            if self.converter.is_some() {
                self.push_converted(&planar, seg.start, seg.frames, ticks, seg.epoch, seg.state);
            } else {
                let mut inter = Vec::with_capacity(seg.frames * self.ring_channels);
                for i in 0..seg.frames {
                    for c in 0..self.ring_channels {
                        inter.push(planar.get(c).map_or(0.0, |p| p[i]));
                    }
                }
                let tag = Tag {
                    media_ticks: ticks,
                    epoch: seg.epoch,
                    state: seg.state,
                    frames: seg.frames as u32,
                };
                while !self.ring.push(tag, &inter) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    fn push_converted(
        &mut self,
        planar: &[Vec<f32>],
        start: i64,
        frames: usize,
        ticks: i64,
        epoch: u32,
        state: TransportState,
    ) {
        let device_rate = self.device_rate;
        let ring_channels = self.ring_channels;
        let c = self.converter.as_mut().expect("converter");
        if c.next_input != Some(start) {
            c.reset(ticks);
        }
        c.next_input = Some(start + frames as i64);
        let channels = planar.len();
        let mut fed = 0;
        let mut out_chunks: Vec<(Tag, Vec<f32>)> = Vec::new();
        while fed < frames {
            let n = (frames - fed).min(CHUNK);
            for (dst, src) in c.input.iter_mut().zip(planar) {
                dst[..n].copy_from_slice(&src[fed..fed + n]);
                dst[n..].fill(0.0);
            }
            let indexing = rubato::Indexing::new().partial_len(n);
            let in_adapter = SequentialSliceOfVecs::new(&c.input, channels, CHUNK).expect("input");
            let out_len = c.output[0].len();
            let mut out_adapter =
                SequentialSliceOfVecs::new_mut(&mut c.output, channels, out_len).expect("output");
            let Ok((_, written)) =
                c.rs.process_into_buffer(&in_adapter, &mut out_adapter, Some(&indexing))
            else {
                break;
            };
            fed += n;
            let drop = c.skip.min(written);
            c.skip -= drop;
            for (p, o) in c.pending.iter_mut().zip(&c.output) {
                p.extend_from_slice(&o[drop..written]);
            }
            while c.pending[0].len() >= CHUNK {
                let mut inter = Vec::with_capacity(CHUNK * ring_channels);
                for i in 0..CHUNK {
                    for ch in 0..ring_channels {
                        inter.push(c.pending.get(ch).map_or(0.0, |p| p[i]));
                    }
                }
                for p in &mut c.pending {
                    p.drain(..CHUNK);
                }
                let tag = Tag {
                    media_ticks: c.pending_ticks,
                    epoch,
                    state,
                    frames: CHUNK as u32,
                };
                c.pending_ticks += CHUNK as i64 * ticks_per_frame(device_rate);
                out_chunks.push((tag, inter));
            }
        }
        for (tag, inter) in out_chunks {
            while !self.ring.push(tag, &inter) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
