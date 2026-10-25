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
//! epoch is dropped by the callback. Detached workers prime independent
//! voices ahead of Play. Ordinary starts wait for every voice and publish
//! complete audio together. Only a decoder still missing after the bounded
//! A3 device-time hold can omit a counted prefix. The clock line starts
//! when the device presents the first buffer.
//!
//! When the device does not run at the project rate, `rubato::Async` (sinc,
//! the section 16 filter) converts the master to the device rate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
/// Process-wide diagnostics for the shared preparation pool.
pub use crate::preparation::{IdleLeaseState, IdleStats};
#[doc(hidden)]
pub use crate::priority::StartPriority;
pub use crate::ring::Start;
use crate::ring::{CHUNK, Consumer, Ring, Tag};
use crate::shuttle::Shuttle;
use crate::source::SourceCache;
use crate::transport::{Effect, Transport};

/// Groups of control points ([`crate::mixer::READ_GROUP`]) of time-mapped
/// voices the look-ahead bounds per turn of the render loop: 4 to 7 ms of
/// evaluation (0.5 to 0.8 ms a group of a variable-speed voice, measured
/// with `mix_bench` under load), well inside the ring's 80 ms.
const LOOKAHEAD_GROUPS: usize = 8;
/// How often fully prepared paused starts are checked again.
const PAUSED_REFRESH: Duration = Duration::from_secs(1);
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
        let before = self.consumer.ring().underruns();
        let latency = self.shared.latency_us.load(Ordering::Relaxed);
        let rate = self.shared.device_rate;
        let heard = self
            .consumer
            .fill_from(out, channels, host_us + u64::from(latency), rate);
        if let Some(h) = heard {
            // Where the heard frame is in this buffer (a scheduled start
            // begins partway through it).
            let host_us = host_us + (h.at_frame as f64 * 1e6 / f64::from(rate)).round() as u64;
            self.shared
                .start_timing
                .heard(h.tag.epoch, host_us, out.len() / channels.max(1));
            if self.shared.reanchor.swap(false, Ordering::Relaxed) {
                self.core.invalidate();
            }
            if let Some(s) = self.core.observe(h, host_us + latency as u64, latency) {
                self.pending = Some(s);
            }
        }
        let missed = self.consumer.ring().underruns().saturating_sub(before);
        let trace = &self.shared.start_timing;
        let epoch = self.consumer.ring().epoch();
        if missed > 0
            && trace.running.load(Ordering::Relaxed)
            && trace.epoch.load(Ordering::Acquire) == epoch
            && trace.callback_epoch.load(Ordering::Acquire) == epoch
        {
            trace.program_underruns.fetch_add(missed, Ordering::Relaxed);
        }
        if let Some(s) = self.pending
            && self.shared.clock.try_store(s)
        {
            self.pending = None;
        }
    }

    /// Producer publication and observer upper bounds for this epoch's PCM.
    pub fn publication_for_queued_frames(&self, frames: usize) -> Option<crate::ring::Publication> {
        self.consumer.publication_for_queued_frames(frames)
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
    /// Commands, their reply, when they were requested, and when a Play
    /// among them presents its first sample.
    Commands(
        Vec<TransportCommand>,
        std::sync::mpsc::SyncSender<Result<()>>,
        u64,
        Start,
    ),
    Command(
        TransportCommand,
        std::sync::mpsc::SyncSender<Result<()>>,
        u64,
        Start,
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
    Schedule(crate::realtime::AudioSchedule),
    Evaluator(Box<Evaluator>, crate::preparation::Evaluators, bool),
    Shutdown,
}

/// A ticket is published before a control enters the FIFO and remains live
/// while that control is queued or deferred. Dropping a failed send or a
/// receiver with queued controls releases its tickets as well.
struct PendingControl(Arc<AtomicU64>);

impl PendingControl {
    fn new(count: Arc<AtomicU64>) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count)
    }
}

impl Drop for PendingControl {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ControlEnvelope {
    control: Control,
    pending: PendingControl,
}

impl ControlEnvelope {
    /// Called only after the producer has established the scheduling policy
    /// for this control; DSP workers may then resume as it is dispatched.
    fn into_dispatch(self) -> Control {
        let Self { control, pending } = self;
        drop(pending);
        control
    }
}

#[derive(Clone)]
struct ControlSender {
    tx: Sender<ControlEnvelope>,
    pending: Arc<AtomicU64>,
}

impl ControlSender {
    fn send(
        &self,
        control: Control,
    ) -> std::result::Result<(), std::sync::mpsc::SendError<ControlEnvelope>> {
        self.tx.send(ControlEnvelope {
            control,
            pending: PendingControl::new(self.pending.clone()),
        })
    }
}

fn control_label(control: &Control, nested: bool) -> &'static str {
    match control {
        Control::Commands(..) => "commands",
        Control::Command(command, ..) => match (nested, command) {
            (false, TransportCommand::Play { .. }) => "play",
            (true, TransportCommand::Play { .. }) => "nested-play",
            (false, TransportCommand::Pause) => "pause",
            (true, TransportCommand::Pause) => "nested-pause",
            (false, TransportCommand::Seek { .. }) => "seek",
            (true, TransportCommand::Seek { .. }) => "nested-seek",
            (_, TransportCommand::SetLoop { .. }) => "set-loop",
            (_, TransportCommand::Step { .. }) => "step",
            (_, TransportCommand::PlayAround { .. }) => "play-around",
            (_, TransportCommand::AudioScrub { .. }) => "audio-scrub",
        },
        Control::Project(..) => "project",
        Control::Solo(..) => "solo",
        Control::Watch(..) => "watch",
        Control::Schedule(..) => "schedule",
        Control::Evaluator(..) => "evaluator",
        Control::Shutdown => "shutdown",
    }
}

struct Shared {
    #[cfg(test)]
    render_turns: AtomicU64,
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
    scheduling: Arc<crate::preparation::SchedulingCounters>,
    schedule: Mutex<crate::realtime::AudioSchedule>,
    idle_workers: crate::preparation::IdleLeaseHandle,
    idle_startup_arm: Mutex<crate::preparation::IdleArmOutcome>,
}

/// Times of a run's first buffer on the shared monotonic host clock.
/// Separates control delivery, render preparation, callback phase and HAL lead.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTiming {
    pub epoch: u32,
    pub requested_us: u64,
    pub applied_us: u64,
    pub queued_us: u64,
    pub ready_us: u64,
    pub prefill_ready_us: u64,
    pub start_spread_frames: u64,
    pub omitted_prefix_frames: u64,
    pub safety_net_fires: u64,
    pub program_underruns: u64,
    pub voice_start_samples: Vec<(String, i64)>,
    pub callback_us: u64,
    pub device_host_us: u64,
    pub callback_frames: u64,
    pub preparation: crate::profile::Profile,
    pub late_voice_frames: Vec<(String, u64)>,
    /// The run's first ring publications: frames published so far (at the
    /// ring's rate) and the host time (µs) each push completed, recorded by
    /// the producer itself. This trace is diagnostic; deadline checks use
    /// the ring's atomic publication and observer bounds.
    pub publications: Vec<(u64, u64)>,
    /// Wall time (µs) the producer held a segment because an installed head
    /// ended before a worker finished that voice's history, summed over the
    /// run, and the number of such holds.
    pub history_wait_us: u64,
    pub history_waits: u64,
    /// Legacy diagnostic, now zero: independent histories are worker-only.
    pub history_claims: u64,
    /// The longest single segment render (µs) of the run.
    pub max_render_us: u64,
    /// The fewest frames queued in the ring when the producer came back to
    /// fill it, after the run's first publication (its low-water mark).
    pub min_queued_frames: u64,
    /// Actual scheduler selected on the producer; counters are cumulative
    /// all preparation entries for this Playback, including speculative edits.
    pub producer_scheduling: String,
    pub worker_workgroup_joins: u64,
    pub worker_mach_joins: u64,
    pub worker_scheduling_failures: u64,
    pub producer_workgroup_error: Option<i32>,
    pub producer_mach_error: Option<i32>,
    pub producer_hint_error: Option<i32>,
    pub worker_workgroup_error: i32,
    pub worker_mach_error: i32,
    pub worker_hint_error: i32,
    pub scheduling_max_unit_ns: u64,
    pub scheduling_max_dsp_unit_ns: u64,
    /// DSP thread-CPU maxima by worker/producer and stable phase ID; the
    /// phase labels are `realtime::REALTIME_DSP_PHASE_LABELS`.
    pub scheduling_max_dsp_unit_by_role_phase_ns:
        [[u64; crate::realtime::REALTIME_DSP_PHASE_LABELS.len()]; 2],
    pub scheduling_max_dsp_unit_fraction_ppm: u64,
    pub scheduling_dsp_unit_observations: u64,
    pub scheduling_max_period_cpu_ns: u64,
    pub scheduling_computation_overruns: u64,
    pub scheduling_first_computation_overrun: Option<crate::realtime::ComputationOverrun>,
    pub scheduling_constraint_overruns: u64,
    /// Throttling since this epoch's Play; preparation before Play remains
    /// included in the lifetime maxima and entry/failure counters above.
    pub scheduling_throttle_sleeps: u64,
    pub scheduling_total_sleep_ns: u64,
    pub scheduling_policy_transition_waits: u64,
    pub scheduling_policy_transition_wait_ns: u64,
    pub scheduling_context_policy_transition_waits: u64,
    pub scheduling_context_policy_transition_wait_ns: u64,
    pub scheduling_restore_failures: u64,
    pub scheduling_hint_failures: u64,
    pub scheduling_constraint_readback_failures: u64,
    /// Controls yielding since this Play, alongside current-context totals.
    pub scheduling_control_yields: u64,
    pub scheduling_control_wait_ns: u64,
    pub scheduling_context_control_yields: u64,
    pub scheduling_context_control_wait_ns: u64,
    pub scheduling_interval_starts: u64,
    pub scheduling_interval_finishes: u64,
    pub scheduling_interval_expired: u64,
    pub scheduling_interval_late_finishes: u64,
    pub scheduling_interval_failures: u64,
    pub scheduling_interval_auto_finishes: u64,
    pub scheduling_interval_error: i32,
    pub scheduling_interval_last_requested_us: u64,
    pub scheduling_interval_last_deadline_us: u64,
    pub scheduling_interval_last_reanchor_us: u64,
    pub scheduling_interval_last_anchor_tick: u64,
    pub scheduling_interval_last_start_tick: u64,
    pub scheduling_interval_last_deadline_tick: u64,
    /// First audible callback's declared deadline, retained after later
    /// callback cycles replace the context's latest-interval metrics.
    pub scheduling_first_interval_requested_us: u64,
    pub scheduling_first_interval_deadline_us: u64,
    pub scheduling_first_interval_anchor_tick: u64,
    pub scheduling_first_interval_start_tick: u64,
    pub scheduling_first_interval_deadline_tick: u64,
    pub scheduling_first_buffer_last_deadline_tick: u64,
    pub scheduling_first_buffer_reanchors: u64,
    pub scheduling_first_buffer_last_reanchor_us: u64,
    pub scheduling_workgroup_recommended_threads: u32,
    pub profile_worker_log_drops: u64,
    pub profile_worker_flush_failures: u64,
    /// Shared-pool totals; concurrent viewers contribute to these counters.
    pub idle_pool: IdleStats,
    /// Counter deltas since Play; peak fields remain process-lifetime maxima.
    pub idle_pool_since_play: IdleStats,
    pub idle_lease: Option<IdleLeaseState>,
    /// Last construction-time arm result, before the handle exposed Play.
    pub idle_startup_arm: String,
}

/// Publications recorded per run, enough for several device callbacks.
const PUBLICATION_LOG: usize = 16;

#[derive(Default)]
struct StartTrace {
    epoch: AtomicU32,
    requested: AtomicU64,
    applied: AtomicU64,
    queued: AtomicU64,
    ready: AtomicU64,
    prefill_ready: AtomicU64,
    safety_net: AtomicU64,
    running: AtomicBool,
    program_underruns: AtomicU64,
    voices: Mutex<HashMap<String, i64>>,
    callback: AtomicU64,
    callback_epoch: AtomicU32,
    device_host: AtomicU64,
    callback_frames: AtomicU64,
    preparation: Mutex<crate::profile::Profile>,
    late: Mutex<HashMap<String, u64>>,
    published_frames: AtomicU64,
    publications: Mutex<Vec<(u64, u64)>>,
    history_wait: AtomicU64,
    history_waits: AtomicU64,
    history_claims: AtomicU64,
    max_render: AtomicU64,
    min_queued: AtomicU64,
    scheduling: Mutex<String>,
    scheduling_errors: Mutex<(Option<i32>, Option<i32>, Option<i32>)>,
    scheduling_baseline: Mutex<crate::realtime::RealtimeMetrics>,
    idle_baseline: Mutex<IdleStats>,
    first_interval_requested: AtomicU64,
    first_interval_deadline: AtomicU64,
    first_interval_anchor_tick: AtomicU64,
    first_interval_start_tick: AtomicU64,
    first_interval_deadline_tick: AtomicU64,
    first_buffer_last_deadline_tick: AtomicU64,
    first_buffer_reanchors: AtomicU64,
    first_buffer_last_reanchor: AtomicU64,
}

impl StartTrace {
    fn begin(&self, epoch: u32, requested: u64) {
        crate::profile::reset();
        self.late.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.publications
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.published_frames.store(0, Ordering::Relaxed);
        self.first_interval_requested.store(0, Ordering::Relaxed);
        self.first_interval_deadline.store(0, Ordering::Relaxed);
        self.first_interval_anchor_tick.store(0, Ordering::Relaxed);
        self.first_interval_start_tick.store(0, Ordering::Relaxed);
        self.first_interval_deadline_tick
            .store(0, Ordering::Relaxed);
        self.first_buffer_last_deadline_tick
            .store(0, Ordering::Relaxed);
        self.first_buffer_reanchors.store(0, Ordering::Relaxed);
        self.first_buffer_last_reanchor.store(0, Ordering::Relaxed);
        self.history_wait.store(0, Ordering::Relaxed);
        self.history_waits.store(0, Ordering::Relaxed);
        self.history_claims.store(0, Ordering::Relaxed);
        self.max_render.store(0, Ordering::Relaxed);
        self.min_queued.store(u64::MAX, Ordering::Relaxed);
        self.voices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.ready.store(0, Ordering::Relaxed);
        self.prefill_ready.store(0, Ordering::Relaxed);
        self.safety_net.store(0, Ordering::Relaxed);
        self.program_underruns.store(0, Ordering::Relaxed);
        self.running.store(true, Ordering::Relaxed);
        self.epoch.store(0, Ordering::Release);
        self.requested.store(requested, Ordering::Relaxed);
        self.applied.store(host_now_us(), Ordering::Relaxed);
        self.queued.store(0, Ordering::Relaxed);
        self.epoch.store(epoch, Ordering::Release);
    }

    fn queued(&self) {
        *self.preparation.lock().unwrap_or_else(|e| e.into_inner()) = crate::profile::snapshot();
        self.queued.store(host_now_us(), Ordering::Release);
    }

    /// A push of `frames` to the ring completed (producer thread).
    fn published(&self, frames: u64) {
        let before = self.published_frames.load(Ordering::Relaxed);
        if before == u64::MAX {
            return;
        }
        let at = host_now_us();
        let mut log = self.publications.lock().unwrap_or_else(|e| e.into_inner());
        if log.len() < PUBLICATION_LOG {
            log.push((before + frames, at));
            self.published_frames
                .store(before + frames, Ordering::Relaxed);
        } else {
            // The log is full: stop taking the lock for the rest of the run.
            self.published_frames.store(u64::MAX, Ordering::Relaxed);
        }
    }

    /// The producer came back to a ring holding `frames` (producer thread).
    fn refill(&self, frames: u64) {
        if self.published_frames.load(Ordering::Relaxed) != 0 {
            self.min_queued.fetch_min(frames, Ordering::Relaxed);
        }
    }

    fn history_wait(&self, us: u64) {
        self.history_wait.fetch_add(us, Ordering::Relaxed);
        self.history_waits.fetch_add(1, Ordering::Relaxed);
    }

    fn rendered(&self, us: u64) {
        self.max_render.fetch_max(us, Ordering::Relaxed);
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
        let voice_start_samples: Vec<_> = self
            .voices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(id, sample)| (id.clone(), *sample))
            .collect();
        let start_spread_frames = voice_start_samples
            .iter()
            .map(|(_, at)| *at)
            .max()
            .unwrap_or(0)
            .saturating_sub(
                voice_start_samples
                    .iter()
                    .map(|(_, at)| *at)
                    .min()
                    .unwrap_or(0),
            ) as u64;
        let omitted_prefix_frames = self
            .late
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .copied()
            .sum();
        let (producer_workgroup_error, producer_mach_error, producer_hint_error) = *self
            .scheduling_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = StartTiming {
            epoch,
            producer_scheduling: self
                .scheduling
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            worker_workgroup_joins: 0,
            worker_mach_joins: 0,
            worker_scheduling_failures: 0,
            producer_workgroup_error,
            producer_mach_error,
            producer_hint_error,
            worker_workgroup_error: 0,
            worker_mach_error: 0,
            worker_hint_error: 0,
            scheduling_max_unit_ns: 0,
            scheduling_max_dsp_unit_ns: 0,
            scheduling_max_dsp_unit_by_role_phase_ns: [[0;
                crate::realtime::REALTIME_DSP_PHASE_LABELS.len()];
                2],
            scheduling_max_dsp_unit_fraction_ppm: 0,
            scheduling_dsp_unit_observations: 0,
            scheduling_max_period_cpu_ns: 0,
            scheduling_computation_overruns: 0,
            scheduling_first_computation_overrun: None,
            scheduling_constraint_overruns: 0,
            scheduling_throttle_sleeps: 0,
            scheduling_total_sleep_ns: 0,
            scheduling_policy_transition_waits: 0,
            scheduling_policy_transition_wait_ns: 0,
            scheduling_context_policy_transition_waits: 0,
            scheduling_context_policy_transition_wait_ns: 0,
            scheduling_restore_failures: 0,
            scheduling_hint_failures: 0,
            scheduling_constraint_readback_failures: 0,
            scheduling_control_yields: 0,
            scheduling_control_wait_ns: 0,
            scheduling_context_control_yields: 0,
            scheduling_context_control_wait_ns: 0,
            scheduling_interval_starts: 0,
            scheduling_interval_finishes: 0,
            scheduling_interval_expired: 0,
            scheduling_interval_late_finishes: 0,
            scheduling_interval_failures: 0,
            scheduling_interval_auto_finishes: 0,
            scheduling_interval_error: 0,
            scheduling_interval_last_requested_us: 0,
            scheduling_interval_last_deadline_us: 0,
            scheduling_interval_last_reanchor_us: 0,
            scheduling_interval_last_anchor_tick: 0,
            scheduling_interval_last_start_tick: 0,
            scheduling_interval_last_deadline_tick: 0,
            scheduling_first_interval_requested_us: self
                .first_interval_requested
                .load(Ordering::Relaxed),
            scheduling_first_interval_deadline_us: self
                .first_interval_deadline
                .load(Ordering::Relaxed),
            scheduling_first_interval_anchor_tick: self
                .first_interval_anchor_tick
                .load(Ordering::Relaxed),
            scheduling_first_interval_start_tick: self
                .first_interval_start_tick
                .load(Ordering::Relaxed),
            scheduling_first_interval_deadline_tick: self
                .first_interval_deadline_tick
                .load(Ordering::Relaxed),
            scheduling_first_buffer_last_deadline_tick: self
                .first_buffer_last_deadline_tick
                .load(Ordering::Relaxed),
            scheduling_first_buffer_reanchors: self.first_buffer_reanchors.load(Ordering::Relaxed),
            scheduling_first_buffer_last_reanchor_us: self
                .first_buffer_last_reanchor
                .load(Ordering::Relaxed),
            scheduling_workgroup_recommended_threads: 0,
            profile_worker_log_drops: 0,
            profile_worker_flush_failures: 0,
            idle_pool: IdleStats::default(),
            idle_pool_since_play: IdleStats::default(),
            idle_lease: None,
            idle_startup_arm: String::new(),
            requested_us: self.requested.load(Ordering::Relaxed),
            applied_us: self.applied.load(Ordering::Relaxed),
            queued_us: self.queued.load(Ordering::Acquire),
            ready_us: self.ready.load(Ordering::Acquire),
            prefill_ready_us: self.prefill_ready.load(Ordering::Acquire),
            start_spread_frames,
            omitted_prefix_frames,
            safety_net_fires: self.safety_net.load(Ordering::Relaxed),
            program_underruns: self.program_underruns.load(Ordering::Relaxed),
            voice_start_samples,
            callback_us: callback,
            device_host_us: self.device_host.load(Ordering::Relaxed),
            callback_frames: self.callback_frames.load(Ordering::Relaxed),
            preparation: *self.preparation.lock().unwrap_or_else(|e| e.into_inner()),
            late_voice_frames: self
                .late
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(id, n)| (id.clone(), *n))
                .collect(),
            publications: self
                .publications
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            history_wait_us: self.history_wait.load(Ordering::Relaxed),
            history_waits: self.history_waits.load(Ordering::Relaxed),
            history_claims: self.history_claims.load(Ordering::Relaxed),
            max_render_us: self.max_render.load(Ordering::Relaxed),
            min_queued_frames: match self.min_queued.load(Ordering::Relaxed) {
                u64::MAX => 0,
                n => n,
            },
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
pub struct ClockHandle(Arc<Shared>, Arc<Ring>);

impl ClockHandle {
    /// The latest clock sample.
    pub fn latest(&self) -> Option<ClockSample> {
        self.0.clock.latest()
    }

    /// How far ahead of now a Play's first frame can be presented: the
    /// device's presentation latency and one of its buffers (the callback
    /// that plays it must find the run's audio queued).
    pub fn start_lead_us(&self) -> u64 {
        let callback = self.1.callback_frames() as f64 * 1e6 / f64::from(self.0.device_rate.max(1));
        u64::from(self.0.latency_us.load(Ordering::Relaxed)) + callback.ceil() as u64
    }

    /// Bumped whenever a new clock sample is published.
    pub fn version(&self) -> u64 {
        self.0.clock.version()
    }
}

/// Playback of one composition.
pub struct Playback {
    tx: ControlSender,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    stream: Option<OutputStream>,
    builder: Mutex<Evaluator>,
    cache: Arc<SourceCache>,
    format: MixFormat,
    ring: Arc<Ring>,
}

/// The result of an ordered transport command after the render thread applies it.
#[doc(hidden)]
pub struct CommandReceipt {
    answer: std::sync::mpsc::Receiver<Result<()>>,
    deadline: Instant,
}

impl CommandReceipt {
    /// Waits for the render thread's answer, with the same bound as `command`.
    pub fn wait(self) -> Result<()> {
        if let Some(result) = self.try_result() {
            return result;
        }
        self.answer
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| AudioError::Device("the render thread did not answer".into()))?
    }

    /// Takes a ready answer first; an expired or disconnected receipt is an error.
    pub fn try_result(&self) -> Option<Result<()>> {
        match self.answer.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < self.deadline => None,
            Err(
                std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected,
            ) => Some(Err(AudioError::Device(
                "the render thread did not answer".into(),
            ))),
        }
    }
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
    /// Output device I/O cycles that missed their real-time deadline
    /// (CoreAudio processor overloads; 0 for a headless side).
    pub device_overloads: u64,
    /// Output device callbacks so far (0 for a headless side).
    pub device_callbacks: u64,
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
        // The real stream gets its actual device schedule after open.
        // Its temporary context is Mach-only, never a headless interval.
        let initial_schedule =
            crate::realtime::AudioSchedule::new(info.sample_rate, info.buffer_frames, None);
        let (mut playback, mut side) = Self::start_with_schedule(
            format,
            cache,
            config,
            info.sample_rate,
            info.channels.min(crate::ring::MAX_CHANNELS),
            info.latency_us(),
            initial_schedule,
        )?;
        let stream = OutputStream::open(move |out, channels, host_us| {
            side.callback(out, channels, host_us)
        })?;
        playback
            .shared
            .latency_us
            .store(stream.info.latency_us(), Ordering::Relaxed);
        let rate = stream.info.sample_rate;
        let device = crate::device::default_output_state(rate).map(|(id, _)| id);
        let schedule = crate::realtime::AudioSchedule::new(rate, stream.info.buffer_frames, device);
        // Arm the actual callback period on this constructor thread, before
        // exposing Play. No queued transport control waits for this barrier.
        playback.shared.idle_workers.set_schedule(&schedule);
        let idle_arm = playback.shared.idle_workers.arm(Duration::from_millis(100));
        *playback
            .shared
            .idle_startup_arm
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = idle_arm;
        let _ = playback.tx.send(Control::Schedule(schedule));
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
        let schedule = crate::realtime::AudioSchedule::headless(device_rate, 512);
        Self::start_with_schedule(
            format,
            cache,
            config,
            device_rate,
            device_channels,
            latency_us,
            schedule,
        )
    }

    fn start_with_schedule(
        format: MixFormat,
        cache: Arc<SourceCache>,
        config: PlaybackConfig,
        device_rate: u32,
        device_channels: usize,
        latency_us: u32,
        schedule: crate::realtime::AudioSchedule,
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
        let mut preparation = crate::preparation::Preparation::new();
        preparation.set_schedule(schedule.clone());
        let idle_startup_arm = preparation.arm_idle_workers(Duration::from_millis(100));
        let idle_workers = preparation.idle_handle();
        let pending_controls = Arc::new(AtomicU64::new(0));
        preparation.set_pending_controls(pending_controls.clone());
        let shared = Arc::new(Shared {
            #[cfg(test)]
            render_turns: AtomicU64::new(0),
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
            scheduling: preparation.scheduling.clone(),
            schedule: Mutex::new(schedule.clone()),
            idle_workers,
            idle_startup_arm: Mutex::new(idle_startup_arm),
        });
        let (raw_tx, rx) = channel();
        let tx = ControlSender {
            tx: raw_tx,
            pending: pending_controls,
        };
        let empty = Arc::new(Document::new(None, 0));
        let graph = Arc::new(MixGraph {
            comp_id: String::new(),
            comps: vec![],
            voices: vec![],
            strips: vec![],
            master: 1.0,
            problems: vec![],
        });
        let mut mixer = Mixer::new(
            format.clone(),
            empty,
            graph,
            Evaluator::new(),
            cache.clone(),
        );
        mixer.standard_content(true);
        // Replaced documents, graphs and voices are freed on their own
        // thread, never on the rendering one; it ends with the mixer.
        let (retire, retired) = std::sync::mpsc::channel::<crate::mixer::Retired>();
        std::thread::Builder::new()
            .name("pm-audio-retire".into())
            .spawn(move || while retired.recv().is_ok() {})
            .map_err(|e| AudioError::Device(format!("cannot start the retire thread: {e}")))?;
        mixer.set_retire(retire);
        let worker = RenderThread {
            rx,
            deferred: None,
            batching: false,
            pending_prepare: false,
            boundary_due: true,
            settled: None,
            shared: shared.clone(),
            ring: ring.clone(),
            transport: Transport::new(format.sample_rate),
            pending_segment: None,
            last_segment_end: None,
            today_index: None,
            late_start: None,
            history_wait: None,
            stalls: HashMap::new(),
            startup: None,
            preroll_hold: None,
            interval_output_frames: 0,
            interval_start_epoch: None,
            interval_failed_epoch: None,
            render_failure: None,
            mixer,
            preparation,
            schedule,
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
    /// automation and preparation workers. Configure the expression engine
    /// and hooks in each evaluator the factory returns.
    pub fn set_evaluator(&self, make: impl Fn() -> Evaluator + Send + Sync + 'static) {
        self.evaluators(make, false);
    }

    /// Standard channel evaluator, with no custom EvalHooks. Constant source
    /// maps may be compiled once for each detached preparation job. Expression
    /// maps still use the full configured evaluator at every control point.
    pub fn set_standard_evaluator(&self, make: impl Fn() -> Evaluator + Send + Sync + 'static) {
        self.evaluators(make, true);
    }

    fn evaluators(
        &self,
        make: impl Fn() -> Evaluator + Send + Sync + 'static,
        static_content: bool,
    ) {
        *self.builder.lock().unwrap_or_else(|e| e.into_inner()) = make();
        let ev = make();
        let _ = self.tx.send(Control::Evaluator(
            Box::new(ev),
            Arc::new(make),
            static_content,
        ));
    }

    /// Release unused decoded audio without interrupting the current render.
    pub fn memory_pressure(&self) {
        self.cache.playback_memory_pressure();
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
        self.enqueue_command(command)?.wait()
    }

    /// Queues one command in order and returns its eventual individual answer.
    #[doc(hidden)]
    pub fn enqueue_command(&self, command: TransportCommand) -> Result<CommandReceipt> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        self.tx
            .send(Control::Command(
                command,
                reply,
                host_now_us(),
                Start::Ready,
            ))
            .map_err(|_| AudioError::Device("playback stopped".into()))?;
        Ok(CommandReceipt { answer, deadline })
    }

    /// Applies related controls in order, without idle preparation between
    /// them. Earlier controls remain applied if a later one is invalid.
    pub fn commands(&self, commands: impl IntoIterator<Item = TransportCommand>) -> Result<()> {
        self.commands_at(commands, Start::Ready)
    }

    /// [`Self::commands`] whose Play presents its first sample at `start`:
    /// the picture's line and the audio share one start
    /// ([`crate::ring::Consumer::fill_from`]).
    pub fn commands_at(
        &self,
        commands: impl IntoIterator<Item = TransportCommand>,
        start: Start,
    ) -> Result<()> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        let commands = commands.into_iter().collect();
        let deadline = Instant::now() + Duration::from_secs(5);
        self.tx
            .send(Control::Commands(commands, reply, host_now_us(), start))
            .map_err(|_| AudioError::Device("playback stopped".into()))?;
        CommandReceipt { answer, deadline }.wait()
    }

    /// Times a run held by [`Start::Held`]: its first sample is presented
    /// at host µs `at` (0: as soon as it is queued). No effect once the run
    /// started or when nothing is held. Call it after the Play's answer.
    pub fn time_held_start(&self, at: u64) {
        self.ring.time_held_start(at);
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
        ClockHandle(self.shared.clone(), self.ring.clone())
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
            device_overloads: self.stream.as_ref().map_or(0, OutputStream::overloads),
            device_callbacks: self.stream.as_ref().map_or(0, OutputStream::callbacks),
        }
    }

    /// Diagnostic timing of the most recently started device epoch.
    pub fn start_timing(&self) -> Option<StartTiming> {
        self.shared.start_timing.latest().map(|mut timing| {
            timing.worker_workgroup_joins =
                self.shared.scheduling.workgroup.load(Ordering::Relaxed);
            timing.worker_mach_joins = self.shared.scheduling.mach.load(Ordering::Relaxed);
            timing.worker_scheduling_failures =
                self.shared.scheduling.failed.load(Ordering::Relaxed);
            timing.worker_workgroup_error = self
                .shared
                .scheduling
                .workgroup_error
                .load(Ordering::Relaxed);
            timing.worker_mach_error = self.shared.scheduling.mach_error.load(Ordering::Relaxed);
            timing.worker_hint_error = self.shared.scheduling.hint_error.load(Ordering::Relaxed);
            let schedule = self
                .shared
                .schedule
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let metrics = schedule.metrics();
            timing.scheduling_workgroup_recommended_threads = schedule.max_parallel_threads();
            drop(schedule);
            timing.idle_pool = self.shared.idle_workers.stats();
            timing.idle_pool_since_play = timing.idle_pool.counter_delta(
                *self
                    .shared
                    .start_timing
                    .idle_baseline
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            );
            timing.idle_lease = Some(self.shared.idle_workers.state());
            timing.idle_startup_arm = format!(
                "{:?}",
                *self
                    .shared
                    .idle_startup_arm
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
            );
            let profile_stats = crate::profile::worker_event_stats();
            timing.profile_worker_log_drops = profile_stats.dropped;
            timing.profile_worker_flush_failures = profile_stats.flush_failures;
            timing.scheduling_max_unit_ns = metrics.max_unit_ns;
            timing.scheduling_max_dsp_unit_ns = metrics.max_dsp_unit_ns;
            timing.scheduling_max_dsp_unit_by_role_phase_ns = metrics.max_dsp_unit_by_role_phase_ns;
            timing.scheduling_max_dsp_unit_fraction_ppm = metrics.max_dsp_unit_fraction_ppm;
            timing.scheduling_dsp_unit_observations = metrics.dsp_unit_observations;
            timing.scheduling_max_period_cpu_ns = metrics.max_period_cpu_ns;
            timing.scheduling_computation_overruns = metrics.computation_overruns;
            timing.scheduling_first_computation_overrun = metrics.first_computation_overrun;
            timing.scheduling_constraint_overruns = metrics.constraint_overruns;
            let baseline = *self
                .shared
                .start_timing
                .scheduling_baseline
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            timing.scheduling_throttle_sleeps = metrics
                .throttle_sleeps
                .saturating_sub(baseline.throttle_sleeps);
            timing.scheduling_total_sleep_ns = metrics
                .total_sleep_ns
                .saturating_sub(baseline.total_sleep_ns);
            timing.scheduling_policy_transition_waits = metrics
                .policy_transition_waits
                .saturating_sub(baseline.policy_transition_waits);
            timing.scheduling_policy_transition_wait_ns = metrics
                .policy_transition_wait_ns
                .saturating_sub(baseline.policy_transition_wait_ns);
            timing.scheduling_context_policy_transition_waits = metrics.policy_transition_waits;
            timing.scheduling_context_policy_transition_wait_ns = metrics.policy_transition_wait_ns;
            timing.scheduling_restore_failures = metrics.restore_failures;
            timing.scheduling_hint_failures = metrics.hint_failures;
            timing.scheduling_constraint_readback_failures = metrics.constraint_readback_failures;
            timing.scheduling_control_yields = metrics
                .control_yields
                .saturating_sub(baseline.control_yields);
            timing.scheduling_control_wait_ns = metrics
                .control_wait_ns
                .saturating_sub(baseline.control_wait_ns);
            timing.scheduling_context_control_yields = metrics.control_yields;
            timing.scheduling_context_control_wait_ns = metrics.control_wait_ns;
            timing.scheduling_interval_starts = metrics.interval_starts;
            timing.scheduling_interval_finishes = metrics.interval_finishes;
            timing.scheduling_interval_expired = metrics.interval_expired;
            timing.scheduling_interval_late_finishes = metrics.interval_late_finishes;
            timing.scheduling_interval_failures = metrics.interval_failures;
            timing.scheduling_interval_auto_finishes = metrics.interval_auto_finishes;
            timing.scheduling_interval_error = metrics.interval_error;
            timing.scheduling_interval_last_requested_us = metrics.interval_last_requested_us;
            timing.scheduling_interval_last_deadline_us = metrics.interval_last_deadline_us;
            timing.scheduling_interval_last_reanchor_us = metrics.interval_last_reanchor_us;
            timing.scheduling_interval_last_anchor_tick = metrics.interval_last_anchor_tick;
            timing.scheduling_interval_last_start_tick = metrics.interval_last_start_tick;
            timing.scheduling_interval_last_deadline_tick = metrics.interval_last_deadline_tick;
            timing
        })
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
    pub fn shutdown(&mut self) -> bool {
        let _ = self.tx.send(Control::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(s) = self.stream.take() {
            let _ = s.pause();
        }
        // Diagnostic stderr never holds up a preparation worker. This
        // ordinary caller drains records already queued at shutdown only.
        crate::profile::flush_worker_events()
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.shutdown();
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

/// The two preroll waits keep their wall deadlines and their shared device
/// origin when a control interrupts preparation without starting a new epoch.
struct PrerollHold {
    epoch: u32,
    device_start: u64,
    stretch_deadline: std::time::Instant,
    source_deadline: Option<std::time::Instant>,
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
    rx: Receiver<ControlEnvelope>,
    /// A control that interrupted paused preparation; handled first.
    deferred: Option<ControlEnvelope>,
    batching: bool,
    /// Urgent jobs for the final control position, submitted after the full
    /// control batch has been applied under the producer's audio schedule.
    pending_prepare: bool,
    /// A control or consumed loop head requires another boundary pass.
    boundary_due: bool,
    /// The paused starts of the last complete preparation pass and when it
    /// ran: unchanged and still ready, they are re-checked only every
    /// [`PAUSED_REFRESH`] rather than every idle turn.
    settled: Option<(Vec<i64>, std::time::Instant)>,
    shared: Arc<Shared>,
    ring: Arc<Ring>,
    transport: Transport,
    /// An undecoded segment is retried before advancing the transport.
    /// Never feed its zero-filled samples into the shuttle or converter.
    pending_segment: Option<crate::transport::Segment>,
    /// End of the last committed segment, to distinguish a wrap from
    /// continuous entry into the work area.
    last_segment_end: Option<i64>,
    /// The playhead (seconds) at which today's `audioLayers` index was last
    /// built, where it reads a precomposition's animated `trim`; `None`
    /// once an edit invalidated it ([`Self::rebuild_today_index`]).
    today_index: Option<f64>,
    late_start: Option<i64>,
    /// When the producer began holding a segment for a running history.
    history_wait: Option<std::time::Instant>,
    /// Each voice owns its deadline and notification until it recovers.
    stalls: HashMap<String, VoiceStall>,
    /// The device frame a play or seek began pre-rolling, until its first
    /// device output chunk is queued. Until then, holding for a
    /// cold voice is start latency, not a gap: every missing voice gets the
    /// whole A3 hold from here.
    startup: Option<u64>,
    preroll_hold: Option<PrerollHold>,
    /// Output formed in the current producer-managed headless interval.
    interval_output_frames: u64,
    interval_start_epoch: Option<u32>,
    interval_failed_epoch: Option<u32>,
    /// The whole-render failure of the latest render, if it failed.
    render_failure: Option<String>,
    mixer: Mixer,
    preparation: crate::preparation::Preparation,
    schedule: crate::realtime::AudioSchedule,
    config: PlaybackConfig,
    solo: Option<HashSet<String>>,
    converter: Option<Converter>,
    /// The 2× J/K/L stretch, warmed before accepting project controls.
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
        let _idle_priority = StartPriority::new();
        self.warm_shuttle();
        crate::profile::init_control_events();
        let mut realtime: Option<crate::realtime::AudioScheduleGuard> = None;
        let mut need_preroll = false;
        let mut last_prefetch: Option<i64> = None;
        let mut lookahead_pending = false;
        let mut paused_urgent_until: Option<std::time::Instant> = None;
        loop {
            #[cfg(test)]
            self.shared.render_turns.fetch_add(1, Ordering::Release);
            // Controls.
            let wait = if self.transport.is_running() || self.settled.is_none() {
                Duration::from_millis(2)
            } else {
                Duration::from_millis(20)
            };
            let next = match self.deferred.take() {
                Some(c) => Ok(c),
                None => self.rx.recv_timeout(wait),
            };
            match next {
                Ok(c) => {
                    if matches!(&c.control, Control::Shutdown) {
                        return;
                    }
                    self.schedule_for_control(&c.control, &mut realtime);
                    if self.control(c.into_dispatch()) {
                        need_preroll = true;
                    }
                    while let Ok(c) = self.rx.try_recv() {
                        if matches!(&c.control, Control::Shutdown) {
                            return;
                        }
                        self.schedule_for_control(&c.control, &mut realtime);
                        if self.control(c.into_dispatch()) {
                            need_preroll = true;
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            if self.pending_prepare {
                // A paused edit gets a short producer priority window from
                // this dispatch, even if its decode or vocoder jobs stall.
                let dispatched_at = std::time::Instant::now();
                paused_urgent_until = (!self.transport.is_running())
                    .then(|| dispatched_at + Duration::from_millis(20));
                self.ensure_schedule(&mut realtime);
                // An edit starts preparation before Play. A restart's cycle
                // keeps the original request deadline, including delivery.
                let requested = if self.transport.is_running() && need_preroll {
                    self.shared.start_timing.requested.load(Ordering::Relaxed)
                } else {
                    host_now_us()
                };
                self.begin_audio_interval(requested);
                crate::realtime::checkpoint();
                {
                    let _trace = crate::profile::control_timer("urgent-dispatch");
                    self.preparation
                        .schedule(&mut self.mixer, &[self.transport.position()], true);
                }
                self.pending_prepare = false;
            }
            let watch_elapsed = self.last_watch.elapsed();
            // Let the first 512 frames finish before a due device probe can
            // demote this producer; a stalled start defers it at most 20 ms.
            let first_buffer_pending = self.transport.is_running()
                && self
                    .shared
                    .start_timing
                    .published_frames
                    .load(Ordering::Relaxed)
                    < 512
                && host_now_us()
                    .saturating_sub(self.shared.start_timing.requested.load(Ordering::Relaxed))
                    < 20_000;
            if watch_elapsed >= Duration::from_secs(1)
                && !first_buffer_pending
                && (!self.preparation.pending_start(self.transport.position())
                    || watch_elapsed >= Duration::from_millis(1100))
            {
                drop(realtime.take());
                if self.device_changed() {
                    need_preroll = true;
                }
            }
            if !self.transport.is_running() {
                if self.stop_pending && self.ring.queued() == 0 {
                    self.stop_pending = false;
                    self.publish_stopped();
                }
                if self.stop_pending {
                    paused_urgent_until = None;
                    drop(realtime.take());
                    continue;
                }
                let start = self.transport.position();
                let urgent =
                    paused_urgent_until.is_some_and(|until| std::time::Instant::now() < until);
                if urgent
                    && (self.preparation.pending_start(start) || !self.mixer.stretches_ready(start))
                {
                    self.ensure_schedule(&mut realtime);
                }
                if !urgent {
                    drop(realtime.take());
                }
                if realtime.is_some() {
                    {
                        let _trace = crate::profile::control_timer("paused-urgent-poll");
                        self.preparation.poll(&mut self.mixer);
                    }
                    if !self.mixer.stretches_ready(start) && !self.preparation.pending_start(start)
                    {
                        crate::realtime::checkpoint();
                        let _trace = crate::profile::control_timer("paused-urgent-retry");
                        self.preparation.schedule(&mut self.mixer, &[start], true);
                    }
                    if (self.preparation.pending_start(start) || !self.mixer.stretches_ready(start))
                        && paused_urgent_until
                            .is_some_and(|until| std::time::Instant::now() < until)
                    {
                        // The receiver wait burns no CPU reservation. Keep
                        // the producer eligible during the bounded edit
                        // window, including jobs behind a shared head.
                        continue;
                    }
                    drop(realtime.take());
                }
                self.preparation.poll(&mut self.mixer);
                if !self.preparation.pending_start(start) && self.mixer.stretches_ready(start) {
                    paused_urgent_until = None;
                }
                {
                    let _trace = crate::profile::control_timer("paused-metadata");
                    self.prepare_paused();
                }
                crate::profile::flush_control_events();
                continue;
            }

            paused_urgent_until = None;
            self.ensure_schedule(&mut realtime);
            crate::realtime::checkpoint();
            if need_preroll {
                if !self.preroll() {
                    // The queued control owns its ticket until dispatch.
                    // Do not fill this superseded run before handling it.
                    continue;
                }
                need_preroll = false;
                last_prefetch = None;
            }
            self.fill();
            // A partial first callback retains its original cycle across
            // retries. Once complete, idle/metadata time is outside a cycle.
            if self
                .shared
                .start_timing
                .published_frames
                .load(Ordering::Relaxed)
                >= 512
                || !self.transport.is_running()
            {
                self.finish_audio_interval();
            }

            if self.late_start.is_some() {
                continue;
            }
            if self
                .shared
                .start_timing
                .published_frames
                .load(Ordering::Relaxed)
                < 512
            {
                // The first callback's whole buffer is still being formed.
                // Bounds and speculative work wait until it is published.
                continue;
            }
            let position = self.transport.position();
            let rate = self.transport.rate() as i64;
            let lookahead_due = lookahead_pending
                || last_prefetch.is_none_or(|last| (position - last).abs() > rate / 4);
            let boundaries_due = self.boundary_due && self.pending_segment.is_none();
            if boundaries_due || lookahead_due {
                // Bounds and speculative decode have no bounded audio
                // quantum. Stay scheduled across the next receiver wait
                // when neither maintenance pass is due.
                drop(realtime.take());
            }
            // Replenish loop/work-area voices after a control or wrap.
            if boundaries_due && !self.preempted() {
                let _trace = crate::profile::control_timer("boundary-metadata");
                self.boundary_due = !self.prepare_boundaries();
            }
            if lookahead_due {
                let _trace = crate::profile::control_timer("lookahead-metadata");
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
            if realtime.is_none() {
                crate::profile::flush_control_events();
            }
        }
    }

    fn schedule_for_control(
        &mut self,
        control: &Control,
        realtime: &mut Option<crate::realtime::AudioScheduleGuard>,
    ) {
        // Record the state at receipt, before this control changes policy.
        crate::profile::control_mark(if realtime.is_some() {
            "received-with-rt-guard"
        } else {
            "received-without-rt-guard"
        });
        let _trace = crate::profile::control_timer(match control {
            Control::Project(..) => "schedule-project",
            Control::Commands(..) => "schedule-commands",
            Control::Command(TransportCommand::Play { .. }, ..) => "schedule-play",
            Control::Command(TransportCommand::Seek { .. }, ..) => "schedule-seek",
            Control::Command(TransportCommand::Pause, ..) => "schedule-pause",
            _ => "schedule-other",
        });
        match control {
            Control::Command(..) | Control::Commands(..) => {
                self.ensure_schedule(realtime);
                crate::realtime::checkpoint();
            }
            Control::Project(..) => {
                // Cancel old real-time jobs before giving up the producer's
                // priority to rebuild the document and mixer.
                self.ensure_schedule(realtime);
                crate::realtime::checkpoint();
                self.preparation.invalidate();
                drop(realtime.take());
            }
            _ => {
                // Evaluator, device, and mix metadata can perform unbounded
                // work and must use ordinary producer scheduling.
                drop(realtime.take());
            }
        }
    }

    fn ensure_schedule(&mut self, realtime: &mut Option<crate::realtime::AudioScheduleGuard>) {
        if realtime.is_some() {
            return;
        }
        let guard = self.schedule.enter_producer();
        self.interval_output_frames = 0;
        self.interval_start_epoch = None;
        *self
            .shared
            .start_timing
            .scheduling
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = guard.method().label().to_string();
        let status = guard.status();
        let mut errors = self
            .shared
            .start_timing
            .scheduling_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if errors.0.is_none() {
            errors.0 = status.workgroup_error;
        }
        if errors.1.is_none() {
            errors.1 = status.mach_error;
        }
        if errors.2.is_none() {
            errors.2 = status.hint_error;
        }
        drop(errors);
        *realtime = Some(guard);
    }

    fn begin_audio_interval(&mut self, requested_host_us: u64) {
        if !self.schedule.interval_master() {
            return;
        }
        let first_callback = self.transport.is_running()
            && self
                .shared
                .start_timing
                .published_frames
                .load(Ordering::Relaxed)
                < 512;
        let epoch = self.transport.epoch();
        let requested_host_us = if first_callback {
            if self.interval_failed_epoch == Some(epoch)
                || (self.schedule.interval_active() && self.interval_start_epoch == Some(epoch))
            {
                return;
            }
            // Every control and guard re-entry before first publication uses
            // the original Play/Seek deadline. A paused edit's cycle is
            // replaced once; nonrestart controls cannot renew this bound.
            self.shared.start_timing.requested.load(Ordering::Relaxed)
        } else {
            requested_host_us
        };
        match self.schedule.begin_interval(requested_host_us) {
            Ok(()) => {
                self.interval_output_frames = 0;
                self.interval_start_epoch = first_callback.then_some(epoch);
                if first_callback {
                    let metrics = self.schedule.metrics();
                    self.shared
                        .start_timing
                        .first_buffer_last_deadline_tick
                        .store(metrics.interval_last_deadline_tick, Ordering::Relaxed);
                    if self
                        .shared
                        .start_timing
                        .first_interval_requested
                        .load(Ordering::Relaxed)
                        == 0
                    {
                        self.shared
                            .start_timing
                            .first_interval_anchor_tick
                            .store(metrics.interval_last_anchor_tick, Ordering::Relaxed);
                        self.shared
                            .start_timing
                            .first_interval_start_tick
                            .store(metrics.interval_last_start_tick, Ordering::Relaxed);
                        self.shared
                            .start_timing
                            .first_interval_deadline_tick
                            .store(metrics.interval_last_deadline_tick, Ordering::Relaxed);
                        self.shared
                            .start_timing
                            .first_interval_requested
                            .store(requested_host_us, Ordering::Relaxed);
                        self.shared.start_timing.first_interval_deadline.store(
                            requested_host_us
                                .saturating_add(self.schedule.period_ns().div_ceil(1_000)),
                            Ordering::Relaxed,
                        );
                    } else {
                        self.shared
                            .start_timing
                            .first_buffer_reanchors
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    self.shared
                        .start_timing
                        .first_buffer_last_reanchor
                        .store(host_now_us(), Ordering::Relaxed);
                }
            }
            Err(_) if first_callback => {
                // Keep the diagnostic in the schedule metrics. Repeated
                // attempts with a newer timestamp would hide this miss.
                // A failed finish may still leave the prior cycle active.
                self.interval_failed_epoch = Some(epoch);
            }
            Err(_) => {}
        }
    }

    fn finish_audio_interval(&mut self) {
        if !self.schedule.interval_master() || !self.schedule.interval_active() {
            return;
        }
        if self.schedule.finish_interval().is_ok() {
            self.interval_output_frames = 0;
            self.interval_start_epoch = None;
        }
    }

    fn published_audio(&mut self, frames: u64) {
        self.shared.start_timing.published(frames);
        if !self.schedule.has_custom_interval() || !self.schedule.interval_active() {
            return;
        }
        self.interval_output_frames = self.interval_output_frames.saturating_add(frames);
        // Headless schedules declare the 512-frame callback period. Close
        // each completed callback. The next cycle begins only if fill has
        // another segment to form; no CPU reservation debt is restarted here.
        if self.interval_output_frames >= 512 {
            self.interval_failed_epoch = None;
            self.finish_audio_interval();
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

    /// A new project (an edit) invalidates today's `audioLayers` index, as
    /// its `layers`/`project` events and `retune` do
    /// (`legacy/core/audio.ts`). Today rebuilds it at the next
    /// `audioLayers()` call: while playing, the edit's own `sync(PM.time)`,
    /// at the playhead heard now; paused, the next Play, at its start
    /// (seeks, loop wraps and later Plays keep it).
    fn rebuild_today_index(&mut self) {
        self.today_index = if self.transport.is_running() {
            let at = self.heard_sample().unwrap_or(self.transport.position());
            Some(at as f64 / self.mixer.format().sample_rate as f64)
        } else {
            None
        };
        self.mixer.set_today_playhead(self.today_index);
    }

    /// The media position heard now, from the published clock. Until the
    /// device presents the current epoch (a seek's pre-roll), the listener
    /// is at the epoch's start, not on the old line.
    fn heard_sample(&self) -> Option<i64> {
        let clock = self.shared.clock.latest()?;
        let now_us = host_now_us();
        // The callback publishes a line for a future device presentation.
        // Before the epoch's first frame is audible, extrapolating that line
        // backwards moves Pause before its seek target (or around the loop).
        let before_first_output = self.shared.start_timing.latest().is_some_and(|first| {
            first.epoch == self.transport.epoch()
                && now_us
                    < first
                        .device_host_us
                        .saturating_add(self.shared.latency_us.load(Ordering::Relaxed) as u64)
        });
        if clock.epoch != self.transport.epoch() || before_first_output {
            return self
                .epoch_start
                .filter(|(e, _)| *e == self.transport.epoch())
                .map(|(_, at)| at);
        }
        if clock.state == TransportState::Stopped {
            return None;
        }
        let now = pm_protocol::common::HostMicros::new(now_us).ok()?;
        let ticks = clock.media_time_at(now).get();
        Some(crate::offline::sample_of_ticks(
            ticks,
            self.transport.rate(),
        ))
    }

    /// Applies a control; returns whether playback must pre-roll.
    fn control(&mut self, c: Control) -> bool {
        let _trace = crate::profile::control_timer(control_label(&c, self.batching));
        self.boundary_due = true;
        self.shared
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        match c {
            Control::Commands(commands, reply, requested_us, start) => {
                let mut restart = false;
                self.batching = true;
                for command in commands {
                    crate::realtime::checkpoint();
                    let (tx, rx) = std::sync::mpsc::sync_channel(1);
                    restart |= self.control(Control::Command(command, tx, requested_us, start));
                    if let Ok(Err(error)) = rx.try_recv() {
                        self.batching = false;
                        let _ = reply.send(Err(error));
                        return restart;
                    }
                }
                self.batching = false;
                self.pending_prepare = true;
                let _ = reply.send(Ok(()));
                restart
            }
            Control::Command(cmd, reply, requested_us, start) => {
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
                let before = self.transport.position();
                let result = self.transport.apply(&cmd);
                if self.transport.position() != before {
                    self.preparation.invalidate();
                }
                let restart = match &result {
                    Ok(Effect::Restart) => {
                        *self
                            .shared
                            .start_timing
                            .idle_baseline
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = self.shared.idle_workers.stats();
                        *self
                            .shared
                            .start_timing
                            .scheduling_baseline
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = self.schedule.metrics();
                        self.shared
                            .start_timing
                            .begin(self.transport.epoch(), requested_us);
                        self.epoch_start =
                            Some((self.transport.epoch(), self.transport.position()));
                        let start = match cmd {
                            TransportCommand::Play { .. } => start,
                            _ => Start::Ready,
                        };
                        self.ring.set_epoch_at(self.transport.epoch(), start);
                        self.ring.set_expecting(true);
                        self.stopped_published = false;
                        self.stop_pending = false;
                        true
                    }
                    Ok(Effect::Stopped) => {
                        self.finish_audio_interval();
                        if let Some(start) = self.late_start.take() {
                            self.mixer.discard_prepared(start);
                        }
                        crate::realtime::checkpoint();
                        self.mixer.abandon_prefills();
                        crate::realtime::checkpoint();
                        self.mixer.clear_preparing();
                        // A stop drops the run's starved segment: the next
                        // run renders from its own position.
                        self.pending_segment = None;
                        self.history_wait = None;
                        self.stalls.clear();
                        self.startup = None;
                        self.render_failure = None;
                        self.shared
                            .errors
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .new_run();
                        crate::realtime::checkpoint();
                        self.mixer.abandon_retries();
                        crate::realtime::checkpoint();
                        self.ring.set_epoch(self.transport.epoch());
                        self.stopped_published = false;
                        self.stop_pending = false;
                        self.publish_stopped();
                        false
                    }
                    _ => false,
                };
                // SetLoop and Pause often precede a Seek in the same app
                // task. Do not launch obsolete vocoders for those intermediate
                // positions; Project/Seek queue the final urgent batch after
                // controls have drained.
                if !self.batching
                    && !matches!(
                        cmd,
                        TransportCommand::SetLoop { .. } | TransportCommand::Pause
                    )
                {
                    self.pending_prepare = true;
                }
                let _ = reply.send(result.map(|_| ()));
                restart
            }
            Control::Project(doc, graph, rate, end) => {
                if !self.preparation.is_standard() {
                    self.mixer.invalidate_prepared();
                }
                self.settled = None;
                self.rebuild_today_index();
                // A live edit glides into the new mix (a retune); paused, the
                // next run starts on it.
                if self.transport.is_running() {
                    self.mixer.glide_to(doc, graph, self.solo.clone());
                } else {
                    self.mixer.set_graph(doc, graph);
                }
                self.transport.set_composition(rate, end);
                self.pending_prepare = true;
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
            Control::Evaluator(ev, make, static_content) => {
                self.preparation.evaluators(make, static_content);
                self.mixer.set_evaluator(*ev);
                self.mixer.standard_content(static_content);
                false
            }
            Control::Schedule(schedule) => {
                self.preparation.set_schedule(schedule.clone());
                *self
                    .shared
                    .schedule
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = schedule.clone();
                self.schedule = schedule;
                *self
                    .shared
                    .start_timing
                    .scheduling_errors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = (None, None, None);
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
        if let Some((device, _)) = now {
            let frames =
                crate::device::default_output_info().map_or(512, |info| info.buffer_frames);
            let schedule =
                crate::realtime::AudioSchedule::new(self.device_rate, frames, Some(device));
            self.preparation.set_schedule(schedule.clone());
            *self
                .shared
                .schedule
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = schedule.clone();
            self.schedule = schedule;
            *self
                .shared
                .start_timing
                .scheduling_errors
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = (None, None, None);
        }
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
        self.shared
            .start_timing
            .running
            .store(false, Ordering::Relaxed);
        self.ring.set_expecting(false);
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

    /// Master FFT plans/workspace need no project input. Warm before
    /// receiving controls so an immediate edit/Play cannot wait on them.
    fn warm_shuttle(&mut self) {
        if self.shuttle.is_none() {
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
    }

    /// Idle work is cancellable between turns: decode around the playhead
    /// first, then both work-area edges. Each target has its own vocoder.
    fn prepare_paused(&mut self) {
        if self.stop_pending || self.preempted() {
            return;
        }
        let rate = self.transport.rate() as i64;
        let position = self.transport.position();
        // The app pauses on the displayed composition frame, while the
        // audio line can stop a fraction of a frame beyond it. Warm that
        // exact frame first, then the unsnapped position for raw API seeks.
        let frame_start =
            pm_eval::Comp::resolve(self.mixer.document(), &self.mixer.graph().comp_id)
                .ok()
                .and_then(|comp| comp.rate())
                .and_then(|fps| {
                    let frame = fps.rate.frame_of(self.transport.ticks_of(position));
                    fps.rate.frame_start(frame).ok()
                })
                .map(|ticks| crate::offline::sample_of_ticks(ticks, rate as u32))
                .unwrap_or(position)
                .max(0);
        let mut starts = vec![frame_start];
        if position != frame_start {
            starts.push(position);
        }
        for at in self.boundary_starts() {
            if starts.len() < 3 && !starts.contains(&at) {
                starts.push(at);
            }
        }
        starts.dedup();
        // Once everything is prepared, re-bounding every voice's spans each
        // 20 ms turn cost 1-2% CPU with 16 stretched voices while idle.
        // Recheck occasionally so evicted source blocks are queued again.
        if let Some((last, at)) = &self.settled
            && *last == starts
            && at.elapsed() < PAUSED_REFRESH
            && self.mixer.prepared_ready(&starts)
        {
            return;
        }
        self.settled = None;
        self.mixer.retain_prepared(&starts);
        self.preparation.schedule(&mut self.mixer, &starts, true);
        let ahead = (self.config.lookahead_ms.max(self.config.preroll_ms) as i64 * rate / 1000)
            .max(CHUNK as i64);
        let mut ready = Vec::new();
        let mut forward = true;
        for start in starts.iter().copied() {
            if self.preempted() {
                forward = false;
                break;
            }
            // The first chunk's pre-roll comes before the optional three
            // seconds of read-ahead. Otherwise edits spend hundreds of ms
            // bounding audio that Play does not yet need.
            if !self.mixer.prepared_ready(&[start]) {
                if !self.mixer.stretches_ready(start) {
                    forward = false;
                    break;
                }
                self.mixer.prefetch_within(start, CHUNK, LOOKAHEAD_GROUPS);
                self.mixer.mark_unstretched_ready(start);
                if !self.mixer.prepared_ready(&[start]) {
                    forward = false;
                    break;
                }
            }
            ready.push(start);
            if !self
                .mixer
                .prefetch_within(start, ahead as usize, LOOKAHEAD_GROUPS)
            {
                forward = false;
                break;
            }
        }
        let mut behind = forward;
        for start in starts.iter().copied().filter(|_| forward) {
            if self.preempted() {
                behind = false;
                break;
            }
            behind &= self.mixer.prefetch_within(
                start - 2 * ahead,
                (2 * ahead) as usize,
                LOOKAHEAD_GROUPS,
            );
        }
        if behind && ready.len() == starts.len() {
            self.settled = Some((starts, std::time::Instant::now()));
        }
        *self
            .shared
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = ready;
    }

    /// Work-area and loop boundaries, independent of the moving playhead.
    fn boundary_starts(&self) -> Vec<i64> {
        let rate = self.transport.rate();
        let mut starts = Vec::new();
        if let Some(range) = self.transport.loop_range() {
            starts.push(crate::offline::sample_of_ticks(range.start.get(), rate));
            starts.push(
                crate::offline::sample_of_ticks(range.end.get(), rate)
                    .saturating_sub(CHUNK as i64)
                    .max(0),
            );
        }
        if let Ok(comp) = pm_eval::Comp::resolve(self.mixer.document(), &self.mixer.graph().comp_id)
            && let Some(work) = pm_eval::node::array(comp.member("work"))
        {
            for edge in work.iter().take(2) {
                if let Some(t) = pm_eval::node::number(Some(edge)).filter(|t| t.is_finite()) {
                    let at = ((t * rate as f64).round() as i64).max(0);
                    if starts.len() < 3 && !starts.contains(&at) {
                        starts.push(at);
                    }
                }
            }
        }
        starts
    }

    fn prepare_boundaries(&mut self) -> bool {
        if self.transport.play_rate() < 0 {
            return true;
        }
        let starts = self.boundary_starts();
        self.mixer.retain_prepared(&starts);
        self.preparation.schedule(&mut self.mixer, &starts, false);
        for &start in &starts {
            if self.mixer.stretches_ready(start) && !self.mixer.prepared_ready(&[start]) {
                self.mixer.prefetch_within(start, CHUNK, LOOKAHEAD_GROUPS);
                self.mixer.mark_unstretched_ready(start);
                break;
            }
        }
        self.mixer.prepared_ready(&starts)
    }

    /// A control arrived during idle preparation: keep it for the loop.
    fn preempted(&mut self) -> bool {
        if self.deferred.is_none()
            && let Ok(c) = self.rx.try_recv()
        {
            self.deferred = Some(c);
        }
        self.deferred.is_some()
    }

    /// Decodes what the first moments of playback need before queueing.
    fn preroll(&mut self) -> bool {
        if self.preempted() {
            return false;
        }
        self.pending_segment = None;
        self.history_wait = None;
        self.last_segment_end = None;
        // A superseded late start's entry marks its installed voices served
        // without their histories: never reuse it for a later Play.
        if let Some(start) = self.late_start.take() {
            self.mixer.discard_prepared(start);
        }
        self.mixer.clear_preparing();
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
        self.mixer.abandon_prefills();
        self.mixer.abandon_retries();
        let position = self.transport.position();
        let epoch = self.transport.epoch();
        if self
            .preroll_hold
            .as_ref()
            .is_none_or(|hold| hold.epoch != epoch)
        {
            self.preroll_hold = Some(PrerollHold {
                epoch,
                device_start: self.ring.played_frames(),
                stretch_deadline: std::time::Instant::now()
                    + Duration::from_millis(self.config.preroll_wait_ms as u64),
                source_deadline: None,
            });
        }
        let hold = self.preroll_hold.as_ref().unwrap();
        let device_start = hold.device_start;
        let stretch_deadline = hold.stretch_deadline;
        self.startup = Some(device_start);
        // Detached work was launched by Project/Seek. Join only the first
        // chunk, without re-evaluating every voice's source bounds serially.
        if self.transport.play_rate() == 1 {
            self.preparation
                .schedule(&mut self.mixer, &[position], true);
            let _wait = crate::profile::timer(crate::profile::Kind::PrepareWait);
            while !self.mixer.stretches_ready(position)
                && std::time::Instant::now() < stretch_deadline
                && self.ring.played_frames().saturating_sub(device_start)
                    < self.device_rate as u64 * SOURCE_WAIT_MS / 1000
            {
                crate::realtime::checkpoint();
                self.preparation
                    .wait(&mut self.mixer, Duration::from_micros(250));
                self.preparation
                    .schedule(&mut self.mixer, &[position], true);
                if self.preempted() {
                    return false;
                }
            }
        }
        if self.preempted() {
            return false;
        }
        let prepared = self.transport.play_rate() == 1 && self.mixer.prepared_ready(&[position]);
        if self.transport.play_rate() == 1
            && !self.mixer.stretches_ready(position)
            && self.ring.played_frames().saturating_sub(device_start)
                >= self.device_rate as u64 * SOURCE_WAIT_MS / 1000
            && !self.mixer.is_ready(position, CHUNK)
            && self.mixer.begin_prepared(position)
        {
            self.late_start = Some(position);
            crate::preparation::wake_workers();
        }
        self.shared.start_timing.prefill_ready.store(
            self.mixer.prefill_ready_at(position).unwrap_or(0),
            Ordering::Release,
        );
        self.shared
            .start_timing
            .ready
            .store(host_now_us(), Ordering::Release);
        let speed = self.transport.play_rate();
        // Only the first input chunk gates Play. The ordinary bounded
        // look-ahead fills the rest after output is queued; evaluating a
        // quarter-second of every edited map here delayed the first sample.
        let frames = CHUNK as i64 * speed.abs().max(1);
        let from = if speed < 0 {
            position - frames
        } else {
            position
        };
        let frames = frames as usize;
        if !prepared {
            self.mixer.prefetch(from, frames);
        }
        // The A3 source hold: ordinary starts wait for all voices. Only
        // a decoder missing beyond the device-time bound may drop. Wait for
        // the remaining first-chunk input, for at most
        // `preroll_wait_ms` of wall time and SOURCE_WAIT_MS of device time.
        // Nothing of this run is queued, so no ready voice goes silent while
        // it waits. `fill` keeps holding a still-missing first segment by
        // device time from the same frame.
        let source_deadline = *self
            .preroll_hold
            .as_mut()
            .unwrap()
            .source_deadline
            .get_or_insert_with(|| {
                std::time::Instant::now()
                    + Duration::from_millis(self.config.preroll_wait_ms as u64)
            });
        while !prepared
            && !self.mixer.is_ready(from, frames)
            && std::time::Instant::now() < source_deadline
            && self.ring.played_frames().saturating_sub(device_start)
                < self.device_rate as u64 * SOURCE_WAIT_MS / 1000
        {
            if self.preempted() {
                return false;
            }
            crate::realtime::checkpoint();
            let _timing = crate::profile::timer(crate::profile::Kind::Hold);
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.preempted() {
            return false;
        }
        if speed == 1 && self.late_start.is_none() {
            self.mixer.use_prepared(position);
            crate::preparation::wake_workers();
        }
        if let Some(c) = &mut self.converter {
            c.reset();
        }
        if let Some(s) = &mut self.shuttle {
            s.reset();
        }
        true
    }

    /// Renders until the ring holds its callback-adapted look-ahead target.
    fn fill(&mut self) {
        self.preparation.poll(&mut self.mixer);
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
        self.shared
            .start_timing
            .refill(self.ring.queued() as u64 * CHUNK as u64);
        let channels = self.mixer.format().channels();
        let mut planar = vec![Vec::new(); channels];
        loop {
            let required = if self.late_start.is_some() {
                // Keep two callbacks ahead during the short CPU preparation
                // gap, so a late voice does not also lose the whole 80 ms ring.
                self.ring
                    .refill_reserve()
                    .max(CHUNK as u64)
                    .div_ceil(CHUNK as u64) as usize
            } else {
                target
            };
            if self.ring.queued() >= required {
                break;
            }
            if self.schedule.has_custom_interval() && !self.schedule.interval_active() {
                self.begin_audio_interval(host_now_us());
            }
            crate::realtime::checkpoint();
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
                    // What is queued plays out; nothing more is due.
                    self.ring.set_expecting(false);
                    self.shared
                        .start_timing
                        .running
                        .store(false, Ordering::Relaxed);
                }
                return;
            };
            // The samples the segment plays, mixed forward; reverse runs
            // play them backwards. Reverse renders each chunk on its own,
            // so stretched voices resample there (see RenderOptions).
            let (lo, hi) = seg.media_span();
            if self.mixer.awaiting_history(lo, (hi - lo) as usize) {
                // The bounded serial claim budget is zero. Every independent
                // history stays on its real-time preparation worker. Priming
                // sixteen queued histories here spent 68-111 ms without one
                // ring publication and exhausted the audible reserve.
                self.history_wait
                    .get_or_insert_with(std::time::Instant::now);
                self.pending_segment = Some(seg);
                return;
            }
            if let Some(since) = self.history_wait.take() {
                self.shared
                    .start_timing
                    .history_wait(since.elapsed().as_micros() as u64);
            }
            let track_start = self.startup.is_some() || self.late_start.is_some();
            if let Some(start) = self.late_start {
                self.preparation.poll(&mut self.mixer);
                if self.mixer.join_prepared(start, lo) {
                    self.late_start = None;
                } else {
                    self.preparation
                        .schedule_at(&mut self.mixer, &[start], true, Some(lo));
                }
            }
            let preparing = self.mixer.preparing_ids();
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
            options.drop_voices.extend(preparing.iter().cloned());
            let n = (hi - lo) as usize;
            if !retry
                && seg.rate > 0
                && self.last_segment_end.is_some_and(|end| lo < end)
                && seg.loop_range.is_some_and(|range| {
                    lo == crate::offline::sample_of_ticks(range.start.get(), self.transport.rate())
                })
            {
                self.mixer.use_prepared(lo);
                self.boundary_due = true;
                crate::preparation::wake_workers();
            }
            if seg.rate > 0 && self.last_segment_end != Some(lo) {
                // Today schedules every clip and its gain envelope when
                // playback starts, seeks or wraps (`sync(time, true)`), from
                // the playhead: a run starts today's model there. Its index
                // is rebuilt there only when an edit invalidated it.
                if self.today_index.is_none() {
                    self.today_index = Some(lo as f64 / self.mixer.format().sample_rate as f64);
                    self.mixer.set_today_playhead(self.today_index);
                }
                self.mixer.set_today_playback(lo);
            }
            let rendering = std::time::Instant::now();
            let mut result = {
                let _render = crate::profile::timer(crate::profile::Kind::FirstRender);
                self.mixer.render(lo, n, &mut planar, &options)
            };
            self.shared
                .start_timing
                .rendered(rendering.elapsed().as_micros() as u64);
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
                    {
                        let mut late = self
                            .shared
                            .start_timing
                            .late
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        for (id, frames) in &report.dropped_by_voice {
                            if track_start {
                                *late.entry(id.clone()).or_default() += frames;
                            }
                        }
                    }
                    if track_start && report.waiting_voices.is_empty() {
                        let mut voices = self
                            .shared
                            .start_timing
                            .voices
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        for id in &report.heard_whole {
                            voices.entry(id.clone()).or_insert(lo);
                        }
                        if report.dropped_voice_frames > 0 {
                            self.shared
                                .start_timing
                                .safety_net
                                .store(1, Ordering::Relaxed);
                        }
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
            self.last_segment_end = Some(hi);
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
            self.published_audio(n as u64);
            // Stateful DSP can consume many input segments before emitting
            // a device frame. Keep the cold-start hold until output exists.
            self.preroll_hold = None;
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
            self.published_audio(u64::from(tag.frames));
            self.preroll_hold = None;
            self.startup = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_receipt_times_out_when_empty_but_keeps_a_ready_answer() {
        let (sender, answer) = std::sync::mpsc::sync_channel(1);
        let mut receipt = CommandReceipt {
            answer,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        assert!(receipt.try_result().is_none());
        receipt.deadline = Instant::now() - Duration::from_millis(1);
        let error = receipt.try_result().unwrap().unwrap_err();
        assert!(error.to_string().contains("did not answer"), "{error}");
        let error = receipt.wait().unwrap_err();
        assert!(error.to_string().contains("did not answer"), "{error}");
        drop(sender);

        let (sender, answer) = std::sync::mpsc::sync_channel(1);
        sender
            .send(Err(AudioError::invalid("producer rejected command")))
            .unwrap();
        let receipt = CommandReceipt {
            answer,
            deadline: Instant::now() - Duration::from_millis(1),
        };
        let error = receipt.wait().unwrap_err();
        assert!(
            error.to_string().contains("producer rejected command"),
            "{error}"
        );
    }

    #[test]
    fn queued_transport_receipts_preserve_control_order_and_individual_errors() {
        use pm_protocol::clock::SeekMode;
        use pm_protocol::common::{PlaybackRate, TickRange, Ticks};

        struct ShutdownCache(Arc<SourceCache>);
        impl Drop for ShutdownCache {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }

        let cache = SourceCache::new(1 << 20, 0);
        let _decode_workers = ShutdownCache(cache.clone());
        let (mut playback, mut side) = Playback::start_headless(
            MixFormat::stereo(48_000),
            cache,
            PlaybackConfig::default(),
            48_000,
            2,
            0,
        )
        .unwrap();
        // Start a run so the queued Pause must stop it, rather than being a
        // no-op on an already stopped transport. Epoch 1 becomes 2 here.
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        let pause = playback.enqueue_command(TransportCommand::Pause).unwrap();
        let seek = playback
            .enqueue_command(TransportCommand::Seek {
                time: Ticks::from_whole_seconds(3),
                mode: SeekMode::Exact,
            })
            .unwrap();
        let invalid = playback
            .enqueue_command(TransportCommand::SetLoop {
                range: Some(TickRange {
                    start: Ticks::from_whole_seconds(1),
                    end: Ticks::from_whole_seconds(1),
                }),
            })
            .unwrap();
        let valid = playback
            .enqueue_command(TransportCommand::SetLoop { range: None })
            .unwrap();

        // The composition ends before the queued seek. Play must restart at
        // zero, even though the earlier loop was invalid.
        let doc = Arc::new(Document::new(
            Some(pm_model::json::parse(
                r#"{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":30,"dur":2,"layers":[],"comps":{},"assets":{},"params":{}}"#,
            )
            .unwrap()),
            1,
        ));
        assert!(
            playback
                .set_project(doc, "C", &|_: &str| None)
                .unwrap()
                .is_empty()
        );
        playback
            .commands([TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            }])
            .unwrap();
        assert!(pause.wait().is_ok());
        assert!(seek.wait().is_ok());
        let error = invalid.wait().unwrap_err();
        assert!(error.to_string().contains("loop range"), "{error}");
        assert!(valid.wait().is_ok());
        assert_eq!(
            playback.shared.start_timing.epoch.load(Ordering::Acquire),
            5 // Pause, exact Seek, and the final Play each advance the epoch.
        );

        // Chunks the first run queued before the Pause (an older epoch) can
        // fill the ring; only device callbacks drop them, so drain them as
        // a device would until the final run's frames are queued.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while side.queued_frames() < 512 && std::time::Instant::now() < deadline {
            if side.queued_frames() == 0 && side.queued_chunks() > 0 {
                side.callback(&mut [0.0; 256], 2, host_now_us());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            side.queued_frames() >= 512,
            "queued {} frames of the final run, {} chunks",
            side.queued_frames(),
            side.queued_chunks()
        );
        side.callback(&mut [0.0; 1_024], 2, host_now_us());
        let clock = playback.clock().expect("first queued callback");
        assert_eq!(clock.epoch, 5);
        assert!((0..TICKS_PER_SECOND).contains(&clock.media_time.get()));
        assert!(playback.shutdown());
    }

    #[test]
    fn control_tickets_cover_queue_deferral_and_dispatch() {
        let pending = Arc::new(AtomicU64::new(0));
        let (raw_tx, rx) = channel();
        let sender = ControlSender {
            tx: raw_tx,
            pending: pending.clone(),
        };
        let another_sender = sender.clone();
        assert!(sender.send(Control::Solo(None)).is_ok());
        assert!(another_sender.send(Control::Shutdown).is_ok());
        assert_eq!(pending.load(Ordering::Acquire), 2);

        // Taking a control off the channel, including into deferred, does
        // not free its ticket until the producer begins dispatching it.
        let mut deferred = Some(rx.try_recv().unwrap());
        assert_eq!(pending.load(Ordering::Acquire), 2);
        assert!(matches!(
            deferred.take().unwrap().into_dispatch(),
            Control::Solo(None)
        ));
        assert_eq!(pending.load(Ordering::Acquire), 1);
        assert!(matches!(
            rx.try_recv().unwrap().into_dispatch(),
            Control::Shutdown
        ));
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

    #[test]
    fn control_tickets_release_on_failed_send_and_receiver_drop() {
        let pending = Arc::new(AtomicU64::new(0));
        let (raw_tx, rx) = channel();
        let sender = ControlSender {
            tx: raw_tx,
            pending: pending.clone(),
        };
        drop(rx);
        let failure = sender.send(Control::Shutdown).unwrap_err();
        assert_eq!(pending.load(Ordering::Acquire), 1);
        drop(failure);
        assert_eq!(pending.load(Ordering::Acquire), 0);

        let (raw_tx, rx) = channel();
        let sender = ControlSender {
            tx: raw_tx,
            pending: pending.clone(),
        };
        assert!(sender.send(Control::Shutdown).is_ok());
        assert_eq!(pending.load(Ordering::Acquire), 1);
        drop(rx);
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }

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

    #[cfg(all(target_os = "macos", feature = "test-hooks"))]
    #[test]
    fn blocked_first_buffer_reanchors_original_interval_after_project_control() {
        use crate::offline::{PcmFormat, WavWriter};
        use pm_model::json::parse;
        use pm_protocol::clock::{SeekMode, TransportCommand};
        use pm_protocol::common::{PlaybackRate, TICKS_PER_SECOND, Ticks};

        struct ReleaseDecoder(Arc<SourceCache>, Arc<crate::source::Source>);
        impl Drop for ReleaseDecoder {
            fn drop(&mut self) {
                self.0.set_decode_blocked(&self.1, false);
            }
        }
        struct ShutdownCache(Arc<SourceCache>);
        impl Drop for ShutdownCache {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }
        let dir = tempfile::Builder::new()
            .prefix("pm-audio-first-interval-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("source.wav");
        let frames = 2 * crate::source::BLOCK;
        let wav = std::fs::File::create(&file).unwrap();
        let mut writer =
            WavWriter::new(std::io::BufWriter::new(wav), PcmFormat::F32, 2, 48_000).unwrap();
        writer.write(0, &vec![0.1; frames * 2]).unwrap();
        writer.finish().unwrap();

        let cache = SourceCache::new(64 << 20, 2);
        let _decode_workers = ShutdownCache(cache.clone());
        let source = cache.open(&file, None).unwrap();
        cache.block(&source, 0, true).unwrap();
        cache.set_decode_blocked(&source, true);
        let unblock = ReleaseDecoder(cache.clone(), source);
        let (mut playback, side) = Playback::start_headless(
            MixFormat::stereo(48_000),
            cache.clone(),
            PlaybackConfig {
                lookahead_ms: 0,
                preroll_wait_ms: 400,
                ..PlaybackConfig::default()
            },
            48_000,
            2,
            0,
        )
        .unwrap();
        // Give the regression ample wall time while checking that both
        // actual cycles retain one declared deadline across guard re-entry.
        let schedule = crate::realtime::AudioSchedule::headless(48_000, 1_920);
        playback
            .tx
            .send(Control::Schedule(schedule.clone()))
            .unwrap();
        let duration = frames as f64 / 48_000.0;
        let layer = format!(
            r#"{{"id":"A","type":"audio","name":"A","from":0,"dur":{duration},"on":true,"solo":false,"group":null,"parent":null,"p":{{}},"fx":[],"masks":[],"d":{{"asset":"s","gain":1,"trim":0,"fadeIn":0,"fadeOut":0}}}}"#
        );
        let json = format!(
            r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":30,"dur":{duration},"layers":[{layer}],"comps":{{}},"assets":{{}},"params":{{}}}}"#
        );
        let document = Arc::new(Document::new(Some(parse(&json).unwrap()), 1));
        let resolver = |_: &str| Some(file.clone());
        assert!(
            playback
                .set_project(document.clone(), "C", &resolver)
                .unwrap()
                .is_empty()
        );
        let target = (crate::source::BLOCK + 1_000) as f64 / 48_000.0;
        playback
            .commands([
                TransportCommand::Seek {
                    time: Ticks::new((target * TICKS_PER_SECOND as f64).round() as i64).unwrap(),
                    mode: SeekMode::Exact,
                },
                TransportCommand::Play {
                    rate: PlaybackRate::NORMAL,
                },
            ])
            .unwrap();
        let requested = playback
            .shared
            .start_timing
            .requested
            .load(Ordering::Acquire);
        let wait_until = |condition: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !condition() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_micros(100));
            }
            assert!(condition(), "interval condition did not become true");
        };
        wait_until(&|| {
            cache.blocked_decoders() > 0
                && schedule.metrics().interval_last_requested_us == requested
        });
        assert_eq!(side.queued_frames(), 0);
        let first = schedule.metrics();
        assert_eq!(first.interval_last_deadline_us, requested + 40_000);

        assert!(
            playback
                .set_project(document.clone(), "C", &resolver)
                .unwrap()
                .is_empty()
        );
        wait_until(&|| {
            schedule.metrics().interval_starts > first.interval_starts
                && playback
                    .shared
                    .start_timing
                    .first_buffer_reanchors
                    .load(Ordering::Relaxed)
                    >= 1
        });
        let second = schedule.metrics();
        assert!(second.interval_auto_finishes > first.interval_auto_finishes);
        assert_eq!(second.interval_last_requested_us, requested);
        assert_eq!(
            second.interval_last_deadline_us,
            first.interval_last_deadline_us
        );
        assert!(second.interval_last_reanchor_us >= first.interval_last_reanchor_us);
        assert_eq!(
            playback
                .shared
                .start_timing
                .first_interval_requested
                .load(Ordering::Relaxed),
            requested
        );
        assert_eq!(
            playback
                .shared
                .start_timing
                .first_interval_deadline
                .load(Ordering::Relaxed),
            requested + 40_000
        );
        playback
            .command(TransportCommand::SetLoop { range: None })
            .unwrap();
        let turns = playback.shared.render_turns.load(Ordering::Acquire);
        wait_until(&|| playback.shared.render_turns.load(Ordering::Acquire) >= turns + 3);
        assert_eq!(schedule.metrics().interval_starts, second.interval_starts);

        std::thread::sleep(Duration::from_millis(45));
        assert!(
            playback
                .set_project(document, "C", &resolver)
                .unwrap()
                .is_empty()
        );
        wait_until(&|| schedule.metrics().interval_expired > second.interval_expired);
        let expired = schedule.metrics();
        assert_eq!(expired.interval_last_requested_us, requested);
        assert_eq!(
            expired.interval_last_deadline_us,
            first.interval_last_deadline_us
        );
        playback
            .command(TransportCommand::SetLoop { range: None })
            .unwrap();
        let turns = playback.shared.render_turns.load(Ordering::Acquire);
        wait_until(&|| playback.shared.render_turns.load(Ordering::Acquire) >= turns + 3);
        assert_eq!(
            schedule.metrics().interval_expired,
            expired.interval_expired
        );
        assert_eq!(side.queued_frames(), 0);
        assert!(
            playback.take_errors().is_empty(),
            "interval diagnostics are not user audio errors"
        );
        drop(unblock);
        assert!(playback.shutdown());
        cache.shutdown();
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_dir(dir);
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
