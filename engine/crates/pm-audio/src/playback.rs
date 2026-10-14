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

use crate::clock::{ClockCore, ClockSlot, host_now_us};
use crate::device::OutputStream;
use crate::error::{AudioError, Result};
use crate::graph::{AssetResolver, MixGraph};
use crate::kernel;
use crate::meter::Meters;
use crate::mixer::{MixFormat, Mixer, RenderOptions};
use crate::ring::{CHUNK, Consumer, Ring, Tag};
use crate::source::SourceCache;
use crate::transport::{Effect, Transport};

/// Groups of control points ([`crate::mixer::READ_GROUP`]) of time-mapped
/// voices the look-ahead bounds per turn of the render loop: about 6 ms of
/// evaluation (0.8 ms a group, measured with `mix_bench`), well inside the
/// ring's 80 ms.
const LOOKAHEAD_GROUPS: usize = 8;

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

    /// Chunks of [`CHUNK`] frames queued for the device (a chunk being played
    /// counts whole). For hosts that drive a device themselves, such as a
    /// simulated device that must not outrun a starved test machine.
    pub fn queued_chunks(&self) -> usize {
        self.consumer.ring().queued()
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
    Evaluator(Box<Evaluator>),
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
        ring_channels: usize,
        out: &mut Vec<(Tag, Vec<f32>)>,
    ) {
        if self.epoch != Some(epoch) {
            self.reset();
            self.epoch = Some(epoch);
        }
        match self.runs.back_mut() {
            Some(run) if self.next_input == Some(start) && run.state == state => {
                run.frames += frames as u64;
            }
            _ => self.runs.push_back(Run {
                offset: self.fed,
                frames: frames as u64,
                ticks,
                state,
            }),
        }
        self.next_input = Some(start + frames as i64);
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
            let media = run.ticks as i128 + into * TICKS_PER_SECOND as i128 / (p * d);
            out.push((
                Tag {
                    media_ticks: media as i64,
                    epoch,
                    state: run.state,
                    frames: n as u32,
                },
                inter,
            ));
            self.emitted += n as u64;
        }
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
                lookahead_pending = !self.mixer.prefetch_within(
                    position,
                    (self.config.lookahead_ms as i64 * rate / 1000) as usize,
                    LOOKAHEAD_GROUPS,
                );
                last_prefetch = Some(position);
            }
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
        // A seek or a new run starts on the current mix.
        self.mixer.end_glides();
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
            c.reset();
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
                    // The converter's tail and partial chunk belong to the run.
                    self.flush_converter();
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

    #[allow(clippy::too_many_arguments)]
    fn push_converted(
        &mut self,
        planar: &[Vec<f32>],
        start: i64,
        frames: usize,
        ticks: i64,
        epoch: u32,
        state: TransportState,
    ) {
        let ring_channels = self.ring_channels;
        let mut chunks = Vec::new();
        let c = self.converter.as_mut().expect("converter");
        c.feed(
            planar,
            start,
            frames,
            ticks,
            epoch,
            state,
            ring_channels,
            &mut chunks,
        );
        self.push_chunks(chunks);
    }

    fn flush_converter(&mut self) {
        let ring_channels = self.ring_channels;
        let mut chunks = Vec::new();
        if let Some(c) = self.converter.as_mut() {
            c.flush(ring_channels, &mut chunks);
        }
        self.push_chunks(chunks);
    }

    fn push_chunks(&self, chunks: Vec<(Tag, Vec<f32>)>) {
        for (tag, inter) in chunks {
            while !self.ring.push(tag, &inter) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
