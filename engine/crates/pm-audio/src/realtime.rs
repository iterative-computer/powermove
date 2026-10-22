//! Scheduling shared by a playback's producer and start-critical preparation.
//!
//! A CoreAudio device owns an audio workgroup. Its property returns a retained
//! object; the schedule keeps it alive until every per-thread join guard exits.
//! Headless playback uses its own audio work interval; a missing workgroup
//! falls back to a Mach time constraint matched to the callback period.
//! Public Mach latency and
//! throughput tier-0 hints supplement that policy on our owned audio threads;
//! they do not grant core affinity. Call `checkpoint` between bounded units of
//! DSP: a constraint cannot make arbitrary uninterrupted work meet a deadline.

#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicU8, AtomicU64, Ordering},
    },
};

struct WorkerControlState {
    pending: Arc<AtomicU64>,
    metrics: Arc<MetricCounters>,
}

thread_local! {
    static WORKER_PENDING_CONTROLS: RefCell<Option<WorkerControlState>> = const { RefCell::new(None) };
    static CONTROL_YIELD_COUNT: Cell<u64> = const { Cell::new(0) };
    static CONTROL_WAIT_NS: Cell<u64> = const { Cell::new(0) };
}

/// Installed only by a detached preparation worker. Nested scopes restore
/// the previous viewer's counter, and ownership keeps the TLS reference live.
pub(crate) struct WorkerControlGate {
    previous: Option<WorkerControlState>,
    _thread: PhantomData<Rc<()>>,
}

pub(crate) fn worker_control_gate(
    pending: Arc<AtomicU64>,
    schedule: &AudioSchedule,
) -> WorkerControlGate {
    let state = WorkerControlState {
        pending,
        metrics: schedule.inner.metrics.clone(),
    };
    let previous = WORKER_PENDING_CONTROLS.with(|slot| slot.replace(Some(state)));
    let guard = WorkerControlGate {
        previous,
        _thread: PhantomData,
    };
    // Cooperate even before Work::claim or real-time policy entry. Every
    // later forced DSP checkpoint performs the same one-shot check.
    yield_to_pending_control();
    guard
}

impl Drop for WorkerControlGate {
    fn drop(&mut self) {
        let previous = self.previous.take();
        WORKER_PENDING_CONTROLS.with(|slot| {
            drop(slot.replace(previous));
        });
    }
}

fn yield_to_pending_control() -> bool {
    let metrics = WORKER_PENDING_CONTROLS.with(|slot| {
        let slot = slot.borrow();
        slot.as_ref()
            .filter(|state| state.pending.load(Ordering::Acquire) > 0)
            .map(|state| state.metrics.clone())
    });
    let Some(metrics) = metrics else {
        return false;
    };
    // Sleep once, rather than spinning until the queue empties: shutdown
    // and cancellation must still make progress. This changes neither
    // DSP arithmetic nor the calling thread's CPU reservation debt.
    let started = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_micros(50));
    let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    CONTROL_YIELD_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    CONTROL_WAIT_NS.with(|total| total.set(total.get().saturating_add(elapsed_ns)));
    metrics.control_yields.fetch_add(1, Ordering::Relaxed);
    metrics
        .control_wait_ns
        .fetch_add(elapsed_ns, Ordering::Relaxed);
    true
}

/// Thread-local evidence for opt-in worker profiling; excludes Mach budget
/// throttling and ordinary OS scheduling delays.
pub(crate) fn thread_control_yield_stats() -> (u64, u64) {
    (CONTROL_YIELD_COUNT.get(), CONTROL_WAIT_NS.get())
}

#[cfg(test)]
mod worker_control_tests {
    use super::*;

    #[test]
    fn worker_gate_yields_only_for_its_current_pending_controls() {
        std::thread::spawn(|| {
            let outer = Arc::new(AtomicU64::new(1));
            let schedule = AudioSchedule::new(48_000, 512, None);
            let before = thread_control_yield_stats();
            // The producer has no worker gate, even while this queue is busy.
            assert!(!yield_to_pending_control());
            assert_eq!(thread_control_yield_stats(), before);

            let gate = worker_control_gate(outer.clone(), &schedule);
            let after_outer = thread_control_yield_stats();
            assert_eq!(after_outer.0, before.0 + 1);
            assert!(after_outer.1 > before.1);

            let inner = Arc::new(AtomicU64::new(0));
            let nested = worker_control_gate(inner.clone(), &schedule);
            assert!(!yield_to_pending_control());
            assert_eq!(thread_control_yield_stats(), after_outer);

            inner.store(1, Ordering::Release);
            assert!(yield_to_pending_control());
            drop(nested);
            assert!(yield_to_pending_control());
            drop(gate);
            assert!(!yield_to_pending_control());
            assert_eq!(thread_control_yield_stats().0, before.0 + 3);
            assert_eq!(schedule.metrics().control_yields, 3);
            assert!(schedule.metrics().control_wait_ns > 0);
        })
        .join()
        .unwrap();
    }
}

/// The first observed computation-budget overrun in one playback schedule.
/// All CPU values come from the offending thread's CPU clock. The preceding
/// phase names the indivisible work unit completed at the checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputationOverrun {
    pub host_us: u64,
    pub role: &'static str,
    pub phase: &'static str,
    pub unit_cpu_ns: u64,
    pub prior_period_cpu_ns: u64,
    pub period_cpu_ns: u64,
    pub wall_elapsed_ns: u64,
    pub period_ns: u64,
    pub computation_ns: u64,
    pub constraint_ns: u64,
}

// Stable IDs avoid storing pointers or formatting phase names on an audio
// thread. Unknown call sites remain visible rather than borrowing a stale
// label from an unrelated task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum RealtimePhase {
    Unknown,
    PrepareJob,
    PrepareResolve,
    PrepareMap,
    PreparePrefetch,
    PrepareScratch,
    StretchFresh,
    StretchMap,
    StretchInit,
    StretchRead,
    StretchOutputSeek,
    StretchProcess,
    ShuttleMap,
    ShuttleRead,
    ShuttleReset,
    ShuttleOutputSeek,
    ShuttleProcess,
    ReadLinearGrid,
    ReadLinearFree,
}

impl RealtimePhase {
    #[cfg(any(target_os = "macos", test))]
    fn from_label(label: &'static str) -> Self {
        match label {
            "prepare-job" => Self::PrepareJob,
            "prepare-resolve" => Self::PrepareResolve,
            "prepare-map" => Self::PrepareMap,
            "prepare-prefetch" => Self::PreparePrefetch,
            "prepare-scratch" => Self::PrepareScratch,
            "stretch-fresh" => Self::StretchFresh,
            "stretch-map" => Self::StretchMap,
            "stretch-init" => Self::StretchInit,
            "stretch-read" => Self::StretchRead,
            "stretch-output-seek" => Self::StretchOutputSeek,
            "stretch-process" => Self::StretchProcess,
            "shuttle-map" => Self::ShuttleMap,
            "shuttle-read" => Self::ShuttleRead,
            "shuttle-reset" => Self::ShuttleReset,
            "shuttle-output-seek" => Self::ShuttleOutputSeek,
            "shuttle-process" => Self::ShuttleProcess,
            "read-linear-grid" => Self::ReadLinearGrid,
            "read-linear-free" => Self::ReadLinearFree,
            _ => Self::Unknown,
        }
    }

    fn from_id(id: u8) -> Self {
        match id {
            1 => Self::PrepareJob,
            2 => Self::PrepareResolve,
            3 => Self::PrepareMap,
            4 => Self::PreparePrefetch,
            5 => Self::PrepareScratch,
            6 => Self::StretchFresh,
            7 => Self::StretchMap,
            8 => Self::StretchInit,
            9 => Self::StretchRead,
            10 => Self::StretchOutputSeek,
            11 => Self::StretchProcess,
            12 => Self::ShuttleMap,
            13 => Self::ShuttleRead,
            14 => Self::ShuttleReset,
            15 => Self::ShuttleOutputSeek,
            16 => Self::ShuttleProcess,
            17 => Self::ReadLinearGrid,
            18 => Self::ReadLinearFree,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::PrepareJob => "prepare-job",
            Self::PrepareResolve => "prepare-resolve",
            Self::PrepareMap => "prepare-map",
            Self::PreparePrefetch => "prepare-prefetch",
            Self::PrepareScratch => "prepare-scratch",
            Self::StretchFresh => "stretch-fresh",
            Self::StretchMap => "stretch-map",
            Self::StretchInit => "stretch-init",
            Self::StretchRead => "stretch-read",
            Self::StretchOutputSeek => "stretch-output-seek",
            Self::StretchProcess => "stretch-process",
            Self::ShuttleMap => "shuttle-map",
            Self::ShuttleRead => "shuttle-read",
            Self::ShuttleReset => "shuttle-reset",
            Self::ShuttleOutputSeek => "shuttle-output-seek",
            Self::ShuttleProcess => "shuttle-process",
            Self::ReadLinearGrid => "read-linear-grid",
            Self::ReadLinearFree => "read-linear-free",
        }
    }
}

/// A single immutable record, published only after every field has been
/// written. Its snapshot is coherent even while another thread is writing.
#[derive(Default)]
struct FirstComputationOverrun {
    state: AtomicU8, // 0 empty, 1 writing, 2 published
    host_us: AtomicU64,
    role: AtomicU8,
    phase: AtomicU8,
    unit_cpu_ns: AtomicU64,
    prior_period_cpu_ns: AtomicU64,
    period_cpu_ns: AtomicU64,
    wall_elapsed_ns: AtomicU64,
    period_ns: AtomicU64,
    computation_ns: AtomicU64,
    constraint_ns: AtomicU64,
}

impl FirstComputationOverrun {
    #[cfg(any(target_os = "macos", test))]
    fn record(&self, value: ComputationOverrun) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        self.host_us.store(value.host_us, Ordering::Relaxed);
        self.role
            .store(u8::from(value.role == "producer"), Ordering::Relaxed);
        self.phase.store(
            RealtimePhase::from_label(value.phase) as u8,
            Ordering::Relaxed,
        );
        self.unit_cpu_ns.store(value.unit_cpu_ns, Ordering::Relaxed);
        self.prior_period_cpu_ns
            .store(value.prior_period_cpu_ns, Ordering::Relaxed);
        self.period_cpu_ns
            .store(value.period_cpu_ns, Ordering::Relaxed);
        self.wall_elapsed_ns
            .store(value.wall_elapsed_ns, Ordering::Relaxed);
        self.period_ns.store(value.period_ns, Ordering::Relaxed);
        self.computation_ns
            .store(value.computation_ns, Ordering::Relaxed);
        self.constraint_ns
            .store(value.constraint_ns, Ordering::Relaxed);
        self.state.store(2, Ordering::Release);
    }

    fn snapshot(&self) -> Option<ComputationOverrun> {
        (self.state.load(Ordering::Acquire) == 2).then(|| ComputationOverrun {
            host_us: self.host_us.load(Ordering::Relaxed),
            role: if self.role.load(Ordering::Relaxed) == 1 {
                "producer"
            } else {
                "worker"
            },
            phase: RealtimePhase::from_id(self.phase.load(Ordering::Relaxed)).label(),
            unit_cpu_ns: self.unit_cpu_ns.load(Ordering::Relaxed),
            prior_period_cpu_ns: self.prior_period_cpu_ns.load(Ordering::Relaxed),
            period_cpu_ns: self.period_cpu_ns.load(Ordering::Relaxed),
            wall_elapsed_ns: self.wall_elapsed_ns.load(Ordering::Relaxed),
            period_ns: self.period_ns.load(Ordering::Relaxed),
            computation_ns: self.computation_ns.load(Ordering::Relaxed),
            constraint_ns: self.constraint_ns.load(Ordering::Relaxed),
        })
    }
}

#[cfg(test)]
mod first_overrun_tests {
    use super::*;

    #[test]
    fn concurrent_first_writer_publishes_one_coherent_record() {
        let pending = FirstComputationOverrun::default();
        pending.state.store(1, Ordering::Release);
        pending.host_us.store(42, Ordering::Relaxed);
        assert!(
            pending.snapshot().is_none(),
            "a partial record must stay hidden"
        );

        let first = Arc::new(FirstComputationOverrun::default());
        std::thread::scope(|scope| {
            for seed in 1..=8 {
                let first = first.clone();
                scope.spawn(move || {
                    first.record(ComputationOverrun {
                        host_us: seed,
                        role: if seed.is_multiple_of(2) {
                            "producer"
                        } else {
                            "worker"
                        },
                        phase: if seed.is_multiple_of(2) {
                            "stretch-fresh"
                        } else {
                            "read-linear-grid"
                        },
                        unit_cpu_ns: seed * 10,
                        prior_period_cpu_ns: seed * 20,
                        period_cpu_ns: seed * 30,
                        wall_elapsed_ns: seed * 40,
                        period_ns: seed * 50,
                        computation_ns: seed * 60,
                        constraint_ns: seed * 70,
                    });
                });
            }
        });
        let record = first.snapshot().unwrap();
        let seed = record.host_us;
        assert!((1..=8).contains(&seed));
        assert_eq!(
            record.role,
            if seed.is_multiple_of(2) {
                "producer"
            } else {
                "worker"
            }
        );
        assert_eq!(
            record.phase,
            if seed.is_multiple_of(2) {
                "stretch-fresh"
            } else {
                "read-linear-grid"
            }
        );
        assert_eq!(record.unit_cpu_ns, seed * 10);
        assert_eq!(record.prior_period_cpu_ns, seed * 20);
        assert_eq!(record.period_cpu_ns, seed * 30);
        assert_eq!(record.wall_elapsed_ns, seed * 40);
        assert_eq!(record.period_ns, seed * 50);
        assert_eq!(record.computation_ns, seed * 60);
        assert_eq!(record.constraint_ns, seed * 70);
        first.record(ComputationOverrun {
            host_us: 99,
            ..record
        });
        assert_eq!(first.snapshot(), Some(record));
    }
}

/// Observed work between cooperative checkpoints, across all threads in one
/// playback schedule. CPU maxima use the calling thread's CPU clock; the
/// constraint count records CPU service beyond the calling role's actual
/// Mach constraint (the full callback period for workers, 85% for producer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RealtimeMetrics {
    pub max_unit_ns: u64,
    /// Largest thread-CPU span following an admitted DSP-step checkpoint.
    pub max_dsp_unit_ns: u64,
    /// Largest DSP span as parts per million of its actual computation budget.
    pub max_dsp_unit_fraction_ppm: u64,
    /// Number of sampled spans following admitted DSP-step checkpoints.
    pub dsp_unit_observations: u64,
    pub max_period_cpu_ns: u64,
    pub computation_overruns: u64,
    pub first_computation_overrun: Option<ComputationOverrun>,
    pub constraint_overruns: u64,
    pub throttle_sleeps: u64,
    pub total_sleep_ns: u64,
    pub restore_failures: u64,
    pub hint_failures: u64,
    pub constraint_readback_failures: u64,
    /// Current context totals, including preparation before Play.
    pub control_yields: u64,
    pub control_wait_ns: u64,
    /// Waiting for an old callback policy's real wall window before a
    /// different Mach reservation can be installed on the same thread.
    pub policy_transition_waits: u64,
    pub policy_transition_wait_ns: u64,
    /// Successful starts and finishes of producer-managed custom cycles.
    pub interval_starts: u64,
    pub interval_finishes: u64,
    /// A requested callback deadline had already elapsed before start.
    pub interval_expired: u64,
    /// Completed cycles whose actual finish passed their declared deadline.
    pub interval_late_finishes: u64,
    /// Failed create, join, start, finish, or master-lifetime operations.
    pub interval_failures: u64,
    /// Active cycles closed by producer guard teardown.
    pub interval_auto_finishes: u64,
    /// Last errno or Mach-style error from an interval operation.
    pub interval_error: i32,
    /// The most recent attempted cycle's original request and declared
    /// deadline, in the same host-microsecond domain as playback timing.
    pub interval_last_requested_us: u64,
    pub interval_last_deadline_us: u64,
    /// Host time when that request was reanchored into Mach time.
    pub interval_last_reanchor_us: u64,
    /// Exact Mach-absolute arguments passed to the most recent SDK start
    /// call, and the tick sampled while converting the host request.
    pub interval_last_anchor_tick: u64,
    pub interval_last_start_tick: u64,
    pub interval_last_deadline_tick: u64,
}

#[derive(Default)]
struct MetricCounters {
    max_unit_ns: AtomicU64,
    max_dsp_unit_ns: AtomicU64,
    max_dsp_unit_fraction_ppm: AtomicU64,
    dsp_unit_observations: AtomicU64,
    max_period_cpu_ns: AtomicU64,
    computation_overruns: AtomicU64,
    first_computation_overrun: FirstComputationOverrun,
    constraint_overruns: AtomicU64,
    throttle_sleeps: AtomicU64,
    total_sleep_ns: AtomicU64,
    restore_failures: AtomicU64,
    hint_failures: AtomicU64,
    constraint_readback_failures: AtomicU64,
    control_yields: AtomicU64,
    control_wait_ns: AtomicU64,
    policy_transition_waits: AtomicU64,
    policy_transition_wait_ns: AtomicU64,
    interval_starts: AtomicU64,
    interval_finishes: AtomicU64,
    interval_expired: AtomicU64,
    interval_late_finishes: AtomicU64,
    interval_failures: AtomicU64,
    interval_auto_finishes: AtomicU64,
    interval_error: AtomicI32,
    interval_last_requested_us: AtomicU64,
    interval_last_deadline_us: AtomicU64,
    interval_last_reanchor_us: AtomicU64,
    interval_last_anchor_tick: AtomicU64,
    interval_last_start_tick: AtomicU64,
    interval_last_deadline_tick: AtomicU64,
}

impl MetricCounters {
    fn snapshot(&self) -> RealtimeMetrics {
        RealtimeMetrics {
            max_unit_ns: self.max_unit_ns.load(Ordering::Relaxed),
            max_dsp_unit_ns: self.max_dsp_unit_ns.load(Ordering::Relaxed),
            max_dsp_unit_fraction_ppm: self.max_dsp_unit_fraction_ppm.load(Ordering::Relaxed),
            dsp_unit_observations: self.dsp_unit_observations.load(Ordering::Relaxed),
            max_period_cpu_ns: self.max_period_cpu_ns.load(Ordering::Relaxed),
            computation_overruns: self.computation_overruns.load(Ordering::Relaxed),
            first_computation_overrun: self.first_computation_overrun.snapshot(),
            constraint_overruns: self.constraint_overruns.load(Ordering::Relaxed),
            throttle_sleeps: self.throttle_sleeps.load(Ordering::Relaxed),
            total_sleep_ns: self.total_sleep_ns.load(Ordering::Relaxed),
            restore_failures: self.restore_failures.load(Ordering::Relaxed),
            hint_failures: self.hint_failures.load(Ordering::Relaxed),
            constraint_readback_failures: self.constraint_readback_failures.load(Ordering::Relaxed),
            control_yields: self.control_yields.load(Ordering::Relaxed),
            control_wait_ns: self.control_wait_ns.load(Ordering::Relaxed),
            policy_transition_waits: self.policy_transition_waits.load(Ordering::Relaxed),
            policy_transition_wait_ns: self.policy_transition_wait_ns.load(Ordering::Relaxed),
            interval_starts: self.interval_starts.load(Ordering::Relaxed),
            interval_finishes: self.interval_finishes.load(Ordering::Relaxed),
            interval_expired: self.interval_expired.load(Ordering::Relaxed),
            interval_late_finishes: self.interval_late_finishes.load(Ordering::Relaxed),
            interval_failures: self.interval_failures.load(Ordering::Relaxed),
            interval_auto_finishes: self.interval_auto_finishes.load(Ordering::Relaxed),
            interval_error: self.interval_error.load(Ordering::Relaxed),
            interval_last_requested_us: self.interval_last_requested_us.load(Ordering::Relaxed),
            interval_last_deadline_us: self.interval_last_deadline_us.load(Ordering::Relaxed),
            interval_last_reanchor_us: self.interval_last_reanchor_us.load(Ordering::Relaxed),
            interval_last_anchor_tick: self.interval_last_anchor_tick.load(Ordering::Relaxed),
            interval_last_start_tick: self.interval_last_start_tick.load(Ordering::Relaxed),
            interval_last_deadline_tick: self.interval_last_deadline_tick.load(Ordering::Relaxed),
        }
    }

    #[cfg(target_os = "macos")]
    fn interval_failure(&self, error: i32) {
        self.interval_error.store(error, Ordering::Relaxed);
        self.interval_failures.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleMethod {
    DeviceWorkgroup,
    CustomAudioInterval,
    MachTimeConstraint,
    Unavailable,
}
impl ScheduleMethod {
    pub const fn label(self) -> &'static str {
        match self {
            Self::DeviceWorkgroup => "device-workgroup",
            Self::CustomAudioInterval => "custom-audio-interval",
            Self::MachTimeConstraint => "mach-time-constraint",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The actual result of entering the schedule on the calling thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleStatus {
    pub method: ScheduleMethod,
    /// CoreAudio property, custom interval creation, or workgroup-join error.
    pub workgroup_error: Option<i32>,
    /// Mach policy set or effective-mode verification error.
    pub mach_error: Option<i32>,
    /// Error setting or reading public latency/throughput hints. The actual
    /// Mach time constraint is checked separately and may still be active.
    pub hint_error: Option<i32>,
}

#[derive(Clone)]
pub struct AudioSchedule {
    inner: Arc<Inner>,
}

struct Inner {
    metrics: Arc<MetricCounters>,
    #[cfg(target_os = "macos")]
    workgroup: Option<Workgroup>,
    #[cfg(target_os = "macos")]
    custom_interval: Option<Workgroup>,
    #[cfg(target_os = "macos")]
    custom_requested: bool,
    #[cfg(target_os = "macos")]
    max_parallel_threads: u32,
    #[cfg(target_os = "macos")]
    master_claimed: AtomicBool,
    #[cfg(target_os = "macos")]
    workgroup_error: Option<i32>,
    #[cfg(target_os = "macos")]
    period: u32,
    #[cfg(target_os = "macos")]
    computation: u32,
    #[cfg(target_os = "macos")]
    producer_computation: u32,
    #[cfg(target_os = "macos")]
    producer_constraint: u32,
    #[cfg(target_os = "macos")]
    period_ns: u64,
    #[cfg(target_os = "macos")]
    computation_ns: u64,
    #[cfg(target_os = "macos")]
    producer_computation_ns: u64,
    #[cfg(target_os = "macos")]
    producer_constraint_ns: u64,
    #[cfg(target_os = "macos")]
    id: u64,
}

#[cfg(target_os = "macos")]
static NEXT_SCHEDULE_ID: AtomicU64 = AtomicU64::new(1);

/// A playback's scheduling context. `device_id = Some(id)` attempts the
/// CoreAudio output-device workgroup; `None` uses a callback-matched Mach
/// constraint without claiming a device workgroup. Use [`Self::headless`] for
/// a producer-owned custom audio interval. Supply the real callback size.
impl AudioSchedule {
    pub fn new(sample_rate: u32, callback_frames: u32, device_id: Option<u32>) -> Self {
        Self::with_source(sample_rate, callback_frames, device_id, false)
    }

    /// A headless playback owns one custom audio interval coordinated by its
    /// producer. The 512-frame callback size should be passed explicitly.
    pub fn headless(sample_rate: u32, callback_frames: u32) -> Self {
        Self::with_source(sample_rate, callback_frames, None, true)
    }

    fn with_source(
        sample_rate: u32,
        callback_frames: u32,
        device_id: Option<u32>,
        custom_requested: bool,
    ) -> Self {
        #[cfg(target_os = "macos")]
        {
            let period_ns = (if callback_frames == 0 {
                512
            } else {
                callback_frames
            } as u64)
                .saturating_mul(1_000_000_000)
                / sample_rate.max(1) as u64;
            // Keep a small margin for scheduling and the callback itself.
            let computation_ns = period_ns.saturating_mul(19) / 20;
            let producer_computation_ns = period_ns.saturating_mul(4) / 5;
            let producer_constraint_ns = period_ns.saturating_mul(17) / 20;
            let (period, computation, _) =
                mac::constraint_ticks(period_ns, computation_ns, period_ns);
            let (_, producer_computation, producer_constraint) =
                mac::constraint_ticks(period_ns, producer_computation_ns, producer_constraint_ns);
            let metrics = Arc::new(MetricCounters::default());
            let (workgroup, custom_interval, workgroup_error) = match device_id {
                Some(id) => match mac::get_workgroup(id) {
                    Ok(wg) => (Some(wg), None, None),
                    Err(e) => (None, None, Some(e)),
                },
                None if custom_requested => match mac::create_audio_interval() {
                    Ok(wg) => (None, Some(wg), None),
                    Err(e) => {
                        metrics.interval_failure(e);
                        (None, None, Some(e))
                    }
                },
                None => (None, None, None),
            };
            let max_parallel_threads = workgroup
                .as_ref()
                .or(custom_interval.as_ref())
                .map(mac::max_parallel_threads)
                .unwrap_or(0);
            Self {
                inner: Arc::new(Inner {
                    metrics,
                    workgroup,
                    custom_interval,
                    custom_requested,
                    max_parallel_threads,
                    master_claimed: AtomicBool::new(false),
                    workgroup_error,
                    period,
                    computation,
                    producer_computation,
                    producer_constraint,
                    period_ns,
                    computation_ns,
                    producer_computation_ns,
                    producer_constraint_ns,
                    id: NEXT_SCHEDULE_ID.fetch_add(1, Ordering::Relaxed),
                }),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (sample_rate, callback_frames, device_id, custom_requested);
            Self {
                inner: Arc::new(Inner {
                    metrics: Arc::new(MetricCounters::default()),
                }),
            }
        }
    }

    pub fn metrics(&self) -> RealtimeMetrics {
        self.inner.metrics.snapshot()
    }

    /// A fresh Mach-only context with this schedule's exact callback policy.
    /// The idle worker context cannot own or retain the playback workgroup.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn mach_only(&self) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                inner: Arc::new(Inner {
                    metrics: Arc::new(MetricCounters::default()),
                    workgroup: None,
                    custom_interval: None,
                    custom_requested: false,
                    max_parallel_threads: 0,
                    master_claimed: AtomicBool::new(false),
                    workgroup_error: None,
                    period: self.inner.period,
                    computation: self.inner.computation,
                    producer_computation: self.inner.producer_computation,
                    producer_constraint: self.inner.producer_constraint,
                    period_ns: self.inner.period_ns,
                    computation_ns: self.inner.computation_ns,
                    producer_computation_ns: self.inner.producer_computation_ns,
                    producer_constraint_ns: self.inner.producer_constraint_ns,
                    id: NEXT_SCHEDULE_ID.fetch_add(1, Ordering::Relaxed),
                }),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self {
                inner: Arc::new(Inner {
                    metrics: Arc::new(MetricCounters::default()),
                }),
            }
        }
    }

    /// The exact callback period used by both the active and idle contexts.
    pub(crate) fn period_ns(&self) -> u64 {
        #[cfg(target_os = "macos")]
        {
            self.inner.period_ns
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    /// The SDK's advisory workgroup parallelism recommendation, queried once
    /// at creation. Zero means no group or no recommendation; it is neither
    /// a core-affinity guarantee nor a dynamic measurement of system load.
    pub fn max_parallel_threads(&self) -> u32 {
        #[cfg(target_os = "macos")]
        {
            self.inner.max_parallel_threads
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    /// Whether this headless schedule actually owns a custom audio interval.
    pub(crate) fn has_custom_interval(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.inner.custom_interval.is_some()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// True only for the current producer thread while its cycle is active.
    pub(crate) fn interval_active(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            mac::interval_active(&self.inner)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Whether this thread owns this schedule's producer master, even before
    /// the first cycle begins. A joined worker or Mach-only fallback is false.
    pub(crate) fn interval_master(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            mac::interval_master(&self.inner)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Start one producer-owned headless work cycle. `requested_host_us` is
    /// in the same CLOCK_UPTIME_RAW domain as `crate::clock::host_now_us`.
    /// The deadline remains anchored to that request, even if dispatch is
    /// late. Device and Mach-only schedules have no producer-owned interval,
    /// so this method is intentionally a no-op for them.
    pub fn begin_interval(&self, requested_host_us: u64) -> Result<(), i32> {
        #[cfg(target_os = "macos")]
        {
            if !self.inner.custom_requested {
                return Ok(());
            }
            if self.inner.custom_interval.is_none() {
                let error = self.inner.workgroup_error.unwrap_or(libc::EINVAL);
                self.inner.metrics.interval_failure(error);
                return Err(error);
            }
            mac::begin_interval(&self.inner, requested_host_us)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = requested_host_us;
            Ok(())
        }
    }

    /// Finish an active headless cycle. Repeated finishes are harmless; a
    /// producer guard also finishes any still-active cycle before it leaves.
    pub fn finish_interval(&self) -> Result<(), i32> {
        #[cfg(target_os = "macos")]
        {
            if !self.inner.custom_requested || self.inner.custom_interval.is_none() {
                return Ok(());
            }
            mac::finish_interval(&self.inner)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    }

    /// Install the callback-matched Mach policy before joining the device's
    /// audio workgroup (which only accepts real-time threads). The guard must
    /// outlive the work and be
    /// dropped on the same thread. Failure is observable in its status.
    pub fn enter(&self) -> AudioScheduleGuard {
        #[cfg(target_os = "macos")]
        {
            self.enter_with_limits(
                self.inner.computation,
                self.inner.period,
                self.inner.computation_ns,
                self.inner.period_ns,
                false,
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            AudioScheduleGuard {
                inner: self.inner.clone(),
                status: ScheduleStatus {
                    method: ScheduleMethod::Unavailable,
                    workgroup_error: None,
                    mach_error: None,
                    hint_error: None,
                },
                _thread: PhantomData,
            }
        }
    }

    /// Enter with the producer's earlier deadline while retaining the real
    /// device callback period and the same retained workgroup. Producer CPU
    /// debt remains in this schedule's thread-local period across guards.
    pub fn enter_producer(&self) -> AudioScheduleGuard {
        #[cfg(target_os = "macos")]
        {
            if self.inner.custom_requested && mac::master_present() {
                self.inner.metrics.interval_failure(libc::EALREADY);
                return AudioScheduleGuard {
                    inner: self.inner.clone(),
                    state: GuardState::Unavailable,
                    status: ScheduleStatus {
                        method: ScheduleMethod::Unavailable,
                        workgroup_error: Some(libc::EALREADY),
                        mach_error: None,
                        hint_error: None,
                    },
                    _thread: PhantomData,
                };
            }
            self.enter_with_limits(
                self.inner.producer_computation,
                self.inner.producer_constraint,
                self.inner.producer_computation_ns,
                self.inner.producer_constraint_ns,
                true,
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.enter()
        }
    }

    #[cfg(target_os = "macos")]
    fn enter_with_limits(
        &self,
        computation: u32,
        constraint: u32,
        computation_ns: u64,
        constraint_ns: u64,
        producer: bool,
    ) -> AudioScheduleGuard {
        let mut workgroup_error = self.inner.workgroup_error;
        // A previous guard may have left a reservation window on this owned
        // thread. Different callback limits can begin only after that real
        // wall window ends; identical limits retain their CPU debt.
        if let Err(error) = mac::wait_for_policy_transition(
            self.inner.id,
            self.inner.period_ns,
            computation_ns,
            constraint_ns,
            &self.inner.metrics,
        ) {
            return AudioScheduleGuard {
                inner: self.inner.clone(),
                state: GuardState::Unavailable,
                status: ScheduleStatus {
                    method: ScheduleMethod::Unavailable,
                    workgroup_error,
                    mach_error: Some(error),
                    hint_error: None,
                },
                _thread: PhantomData,
            };
        }
        match mac::enter_constraint(self.inner.period, computation, constraint) {
            Ok(previous) => {
                let (hints, hint_error) = match mac::enter_tier_hints(&self.inner.metrics) {
                    Ok(hints) => (Some(hints), None),
                    Err(error) => {
                        self.inner
                            .metrics
                            .hint_failures
                            .fetch_add(1, Ordering::Relaxed);
                        (None, Some(error))
                    }
                };
                // A hint failure is optional, but a demoted timeshare
                // mode is not. Require matching policy parameters and
                // an effective non-timeshare kernel mode.
                if let Err(error) =
                    mac::verify_constraint(self.inner.period, computation, constraint)
                {
                    self.inner
                        .metrics
                        .constraint_readback_failures
                        .fetch_add(1, Ordering::Relaxed);
                    if let Some(hints) = hints {
                        mac::restore_tier_hints(hints, &self.inner.metrics);
                    }
                    mac::restore_constraint(previous, &self.inner.metrics);
                    return AudioScheduleGuard {
                        inner: self.inner.clone(),
                        state: GuardState::Unavailable,
                        status: ScheduleStatus {
                            method: ScheduleMethod::Unavailable,
                            workgroup_error,
                            mach_error: Some(error),
                            hint_error,
                        },
                        _thread: PhantomData,
                    };
                }
                mac::set_budget(
                    self.inner.id,
                    self.inner.period_ns,
                    computation_ns,
                    constraint_ns,
                    self.inner.metrics.clone(),
                );
                let selected = self
                    .inner
                    .workgroup
                    .as_ref()
                    .map(|wg| (wg, false))
                    .or_else(|| self.inner.custom_interval.as_ref().map(|wg| (wg, true)));
                if let Some((wg, custom)) = selected {
                    let mut token = mac::JoinToken::default();
                    // SAFETY: this is now a real-time thread; the retained
                    // object outlives the guard and its token stays here.
                    let result = unsafe { mac::os_workgroup_join(wg.0, &mut token) };
                    if result == 0 {
                        if custom
                            && producer
                            && let Err(error) = mac::install_master(self.inner.clone())
                        {
                            // No master slot means an interval could not
                            // safely be started on this producer thread.
                            unsafe { mac::os_workgroup_leave(wg.0, &mut token) };
                            mac::clear_budget(self.inner.id);
                            if let Some(hints) = hints {
                                mac::restore_tier_hints(hints, &self.inner.metrics);
                            }
                            mac::restore_constraint(previous, &self.inner.metrics);
                            self.inner.metrics.interval_failure(error);
                            return AudioScheduleGuard {
                                inner: self.inner.clone(),
                                state: GuardState::Unavailable,
                                status: ScheduleStatus {
                                    method: ScheduleMethod::Unavailable,
                                    workgroup_error: Some(error),
                                    mach_error: None,
                                    hint_error,
                                },
                                _thread: PhantomData,
                            };
                        }
                        return AudioScheduleGuard {
                            inner: self.inner.clone(),
                            state: GuardState::Workgroup {
                                token,
                                previous,
                                hints,
                                custom,
                                producer,
                            },
                            status: ScheduleStatus {
                                method: if custom {
                                    ScheduleMethod::CustomAudioInterval
                                } else {
                                    ScheduleMethod::DeviceWorkgroup
                                },
                                workgroup_error: None,
                                mach_error: None,
                                hint_error,
                            },
                            _thread: PhantomData,
                        };
                    }
                    workgroup_error = Some(result);
                    if custom {
                        self.inner.metrics.interval_failure(result);
                    }
                }
                AudioScheduleGuard {
                    inner: self.inner.clone(),
                    state: GuardState::Constraint { previous, hints },
                    status: ScheduleStatus {
                        method: ScheduleMethod::MachTimeConstraint,
                        workgroup_error,
                        mach_error: None,
                        hint_error,
                    },
                    _thread: PhantomData,
                }
            }
            Err(error) => AudioScheduleGuard {
                inner: self.inner.clone(),
                state: GuardState::Unavailable,
                status: ScheduleStatus {
                    method: ScheduleMethod::Unavailable,
                    workgroup_error,
                    mach_error: Some(error),
                    hint_error: None,
                },
                _thread: PhantomData,
            },
        }
    }
}

/// Check the current thread's Mach budget without requiring its schedule at
/// nested source/DSP call sites. No-op outside a Mach schedule guard.
#[inline]
pub fn checkpoint() {
    #[cfg(target_os = "macos")]
    mac::checkpoint_current();
    #[cfg(not(target_os = "macos"))]
    {
        yield_to_pending_control();
        if crate::profile::enabled() {
            crate::profile::phase("");
        }
    }
}

/// Attribute CPU before this boundary to the previous opt-in profile phase,
/// then label the work that follows it. The checkpoint remains active in
/// normal playback; the small static phase ID is retained even when detailed
/// profiling is disabled, so a rare overrun still identifies its work unit.
#[inline]
pub(crate) fn checkpoint_phase(label: &'static str) {
    #[cfg(target_os = "macos")]
    mac::checkpoint_current_phase(RealtimePhase::from_label(label));
    #[cfg(not(target_os = "macos"))]
    checkpoint();
    if crate::profile::enabled() {
        crate::profile::phase(label);
    }
}

/// Attribute the preceding work unit, then admit and label a larger known
/// unit before it starts. Profiling stays opt-in; admission is always active.
#[inline]
pub(crate) fn checkpoint_phase_reserve(label: &'static str, unit_ns: u64) {
    #[cfg(target_os = "macos")]
    mac::checkpoint_current_phase_reserve(RealtimePhase::from_label(label), unit_ns);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = unit_ns;
        yield_to_pending_control();
    }
    if crate::profile::enabled() {
        crate::profile::phase(label);
    }
}

/// Gate short DSP-step callbacks with a cheap wall clock. The full CPU-clock
/// checkpoint runs after at most 100 µs, or 1/16 of a shorter callback period.
/// Use `checkpoint` for larger initialization and processing boundaries.
#[inline]
pub(crate) fn checkpoint_dsp_step() {
    #[cfg(target_os = "macos")]
    mac::checkpoint_dsp_step();
}

/// True only while this thread holds at least one successful audio schedule
/// guard. A retained budget between consecutive jobs is not an active guard.
pub(crate) fn has_active_guard() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::has_active_guard()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Opt-in worker profiling reads this only while PM_AUDIO_PROFILE is set.
pub(crate) fn thread_cpu_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        mac::thread_cpu_ns()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// Cumulative checkpoint sleep on this worker, independent of other workers
/// sharing the playback schedule's aggregate counters.
pub(crate) fn thread_throttle_sleep_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        mac::thread_throttle_sleep_ns()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// The kernel's effective timeshare base priority for a pthread. Audio
/// workers use this to verify their wake priority after an RT guard exits.
#[cfg(all(test, target_os = "macos"))]
pub(crate) fn thread_priority(thread: libc::pthread_t) -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the pthread belongs to this process; the returned Mach
        // thread port is borrowed from pthread and must not be deallocated.
        mac::timeshare_base(unsafe { libc::pthread_mach_thread_np(thread) })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = thread;
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn current_thread_priority() -> Option<i32> {
    // SAFETY: pthread_self is always a valid calling-thread identifier.
    thread_priority(unsafe { libc::pthread_self() })
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn thread_has_constraint(thread: libc::pthread_t, schedule: &AudioSchedule) -> bool {
    // SAFETY: this is a live pthread owned by the private test executor.
    let port = unsafe { libc::pthread_mach_thread_np(thread) };
    mac::verify_constraint_on(
        port,
        schedule.inner.period,
        schedule.inner.computation,
        schedule.inner.period,
    )
    .is_ok()
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn thread_is_timeshare(thread: libc::pthread_t) -> bool {
    // SAFETY: this is a live pthread owned by the private test executor.
    mac::is_timeshare(unsafe { libc::pthread_mach_thread_np(thread) })
}

pub struct AudioScheduleGuard {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    inner: Arc<Inner>,
    #[cfg(target_os = "macos")]
    state: GuardState,
    status: ScheduleStatus,
    // Both the Mach policy and the workgroup join are thread-local.
    _thread: PhantomData<Rc<()>>,
}

impl AudioScheduleGuard {
    pub fn status(&self) -> ScheduleStatus {
        self.status
    }
    pub fn method(&self) -> ScheduleMethod {
        self.status.method
    }
}

#[cfg(target_os = "macos")]
enum GuardState {
    Workgroup {
        token: mac::JoinToken,
        previous: mac::PreviousPolicy,
        hints: Option<mac::PreviousTiers>,
        custom: bool,
        producer: bool,
    },
    Constraint {
        previous: mac::PreviousPolicy,
        hints: Option<mac::PreviousTiers>,
    },
    Unavailable,
}

impl Drop for AudioScheduleGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        match &mut self.state {
            GuardState::Workgroup {
                token,
                previous,
                hints,
                custom,
                producer,
            } => {
                if *custom && *producer {
                    mac::remove_master(&self.inner);
                }
                let wg = if *custom {
                    self.inner.custom_interval.as_ref()
                } else {
                    self.inner.workgroup.as_ref()
                }
                .expect("joined workgroup retained");
                // SAFETY: guard is !Send; this is the same thread and token
                // used at join, and its retained object is still alive.
                unsafe { mac::os_workgroup_leave(wg.0, token) };
                mac::clear_budget(self.inner.id);
                if let Some(hints) = hints {
                    mac::restore_tier_hints(*hints, &self.inner.metrics);
                }
                mac::restore_constraint(*previous, &self.inner.metrics);
            }
            GuardState::Constraint { previous, hints } => {
                mac::clear_budget(self.inner.id);
                if let Some(hints) = hints {
                    mac::restore_tier_hints(*hints, &self.inner.metrics);
                }
                mac::restore_constraint(*previous, &self.inner.metrics);
            }
            GuardState::Unavailable => {}
        }
    }
}

#[cfg(target_os = "macos")]
struct Workgroup(*mut std::ffi::c_void);
#[cfg(target_os = "macos")]
unsafe impl Send for Workgroup {}
#[cfg(target_os = "macos")]
unsafe impl Sync for Workgroup {}
#[cfg(target_os = "macos")]
impl Drop for Workgroup {
    fn drop(&mut self) {
        // SAFETY: CoreAudio's device property and AudioWorkIntervalCreate
        // each give the caller one retained workgroup object.
        unsafe { mac::os_release(self.0) };
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Inner, MetricCounters, Workgroup};
    use std::{
        cell::{Cell, RefCell},
        ffi::c_void,
        mem, ptr,
        sync::{Arc, atomic::Ordering},
        time::{Duration, Instant},
    };

    const fn fourcc(s: &[u8; 4]) -> u32 {
        ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
    }
    const GLOBAL: u32 = fourcc(b"glob");
    const IO_WORKGROUP: u32 = fourcc(b"oswg");
    const TIME_CONSTRAINT: i32 = 2;
    const STANDARD: i32 = 1;
    const PRECEDENCE: i32 = 3;
    // mach/thread_policy.h and mach/task_policy.h. These are public Mach
    // policy hints, not CPU affinity or a replacement for time constraints.
    const LATENCY_QOS: i32 = 7;
    const THROUGHPUT_QOS: i32 = 8;
    const LATENCY_TIER_0: i32 = 0x00ff_0001;
    const THROUGHPUT_TIER_0: i32 = 0x00fe_0001;
    const SCHED_TIMESHARE_INFO: i32 = 10;
    const EXTENDED_INFO: i32 = 5;
    const KERN_SUCCESS: i32 = 0;
    const KERN_INVALID_POLICY: i32 = 28;
    // os/clock.h: interval timestamps use mach_absolute_time ticks.
    const MACH_ABSOLUTE_CLOCK: u32 = 32;

    struct MasterInterval {
        // TLS owns the context; neither a worker nor guard teardown can leave
        // a dangling workgroup pointer here.
        inner: Arc<Inner>,
        deadline: Option<u64>,
    }
    thread_local! {
        static MASTER_INTERVAL: RefCell<Option<MasterInterval>> = const { RefCell::new(None) };
        #[cfg(test)]
        static FORCE_CREATE_FAILURE: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn master_present() -> bool {
        MASTER_INTERVAL.with(|slot| slot.borrow().is_some())
    }

    pub(super) fn interval_active(inner: &Arc<Inner>) -> bool {
        MASTER_INTERVAL.with(|slot| {
            slot.borrow().as_ref().is_some_and(|master| {
                Arc::ptr_eq(&master.inner, inner) && master.deadline.is_some()
            })
        })
    }

    pub(super) fn interval_master(inner: &Arc<Inner>) -> bool {
        MASTER_INTERVAL.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|master| Arc::ptr_eq(&master.inner, inner))
        })
    }

    pub(super) fn install_master(inner: Arc<Inner>) -> Result<(), i32> {
        if inner
            .master_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(libc::EALREADY);
        }
        MASTER_INTERVAL.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                inner.master_claimed.store(false, Ordering::Release);
                return Err(libc::EALREADY);
            }
            *slot = Some(MasterInterval {
                inner,
                deadline: None,
            });
            Ok(())
        })
    }

    fn finish_started(master: &mut MasterInterval) -> Result<(), i32> {
        let Some(deadline) = master.deadline else {
            return Ok(());
        };
        let wg = master
            .inner
            .custom_interval
            .as_ref()
            .expect("master retains its custom workgroup");
        // SAFETY: the producer joined this retained workgroup, started one
        // cycle, and is finishing it on the same thread before leaving.
        let result = unsafe { os_workgroup_interval_finish(wg.0, ptr::null_mut()) };
        if result != 0 {
            master.inner.metrics.interval_failure(result);
            return Err(result);
        }
        let finished_at = unsafe { mach_absolute_time() };
        master.deadline = None;
        master
            .inner
            .metrics
            .interval_finishes
            .fetch_add(1, Ordering::Relaxed);
        if finished_at > deadline {
            master
                .inner
                .metrics
                .interval_late_finishes
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(super) fn remove_master(inner: &Arc<Inner>) {
        MASTER_INTERVAL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(mut master) = slot.take() else {
                inner.metrics.interval_failure(libc::EPERM);
                inner.master_claimed.store(false, Ordering::Release);
                return;
            };
            if !Arc::ptr_eq(&master.inner, inner) {
                *slot = Some(master);
                inner.metrics.interval_failure(libc::EPERM);
                return;
            }
            if master.deadline.is_some() {
                inner
                    .metrics
                    .interval_auto_finishes
                    .fetch_add(1, Ordering::Relaxed);
                let _ = finish_started(&mut master);
            }
            inner.master_claimed.store(false, Ordering::Release);
        });
    }

    pub(super) fn finish_interval(inner: &Arc<Inner>) -> Result<(), i32> {
        MASTER_INTERVAL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(master) = slot
                .as_mut()
                .filter(|master| Arc::ptr_eq(&master.inner, inner))
            else {
                inner.metrics.interval_failure(libc::EPERM);
                return Err(libc::EPERM);
            };
            finish_started(master)
        })
    }

    pub(super) fn begin_interval(inner: &Arc<Inner>, requested_host_us: u64) -> Result<(), i32> {
        MASTER_INTERVAL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(master) = slot
                .as_mut()
                .filter(|master| Arc::ptr_eq(&master.inner, inner))
            else {
                inner.metrics.interval_failure(libc::EPERM);
                return Err(libc::EPERM);
            };
            // `start` twice without `finish` is invalid. A superseding Play
            // closes the old generation's cycle exactly once first.
            finish_started(master)?;
            let start = unsafe { mach_absolute_time() };
            // Both values must describe the same sampled uptime instant.
            // A second clock call could be preempted and shift the declared
            // SDK deadline earlier than the recorded anchor.
            let now_us = host_us(start).inspect_err(|&error| {
                inner.metrics.interval_failure(error);
            })?;
            inner
                .metrics
                .interval_last_requested_us
                .store(requested_host_us, Ordering::Relaxed);
            inner.metrics.interval_last_deadline_us.store(
                requested_host_us.saturating_add(inner.period_ns.div_ceil(1_000)),
                Ordering::Relaxed,
            );
            inner
                .metrics
                .interval_last_reanchor_us
                .store(now_us, Ordering::Relaxed);
            // Clear prior SDK-call arguments if this attempt expires or
            // fails conversion before reaching os_workgroup_interval_start.
            inner
                .metrics
                .interval_last_anchor_tick
                .store(0, Ordering::Relaxed);
            inner
                .metrics
                .interval_last_start_tick
                .store(0, Ordering::Relaxed);
            inner
                .metrics
                .interval_last_deadline_tick
                .store(0, Ordering::Relaxed);
            let elapsed_ns = now_us
                .saturating_sub(requested_host_us)
                .saturating_mul(1_000);
            if elapsed_ns >= inner.period_ns {
                inner
                    .metrics
                    .interval_expired
                    .fetch_add(1, Ordering::Relaxed);
                inner.metrics.interval_failure(libc::ETIMEDOUT);
                return Err(libc::ETIMEDOUT);
            }
            let remaining_ns = inner.period_ns - elapsed_ns;
            let remaining_ticks = match interval_ticks(remaining_ns) {
                Ok(ticks) => ticks,
                Err(error) => {
                    inner.metrics.interval_failure(error);
                    return Err(error);
                }
            };
            let Some(deadline) = start.checked_add(remaining_ticks) else {
                inner.metrics.interval_failure(libc::EOVERFLOW);
                return Err(libc::EOVERFLOW);
            };
            // `start` describes when this callback's work began, which may
            // precede the producer's dispatch. Keep it in the same Mach clock
            // as the deadline without assuming host and Mach clock epochs
            // coincide.
            let start_time = match interval_ticks(elapsed_ns) {
                Ok(elapsed_ticks) => start.saturating_sub(elapsed_ticks),
                Err(error) => {
                    inner.metrics.interval_failure(error);
                    return Err(error);
                }
            };
            let wg = inner
                .custom_interval
                .as_ref()
                .expect("master retains its custom workgroup");
            inner
                .metrics
                .interval_last_anchor_tick
                .store(start, Ordering::Relaxed);
            inner
                .metrics
                .interval_last_start_tick
                .store(start_time, Ordering::Relaxed);
            inner
                .metrics
                .interval_last_deadline_tick
                .store(deadline, Ordering::Relaxed);
            // SAFETY: this joined producer is the sole master; the requested
            // start precedes its deadline in the group's mach-absolute clock.
            let result =
                unsafe { os_workgroup_interval_start(wg.0, start_time, deadline, ptr::null_mut()) };
            if result != 0 {
                inner.metrics.interval_failure(result);
                return Err(result);
            }
            master.deadline = Some(deadline);
            inner
                .metrics
                .interval_starts
                .fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    }

    pub(super) fn timeshare_base(thread: u32) -> Option<i32> {
        if thread == 0 {
            return None;
        }
        // mach/policy.h policy_timeshare_info: five integer_t fields.
        let mut info = [0i32; 5];
        let mut count = info.len() as u32;
        let result =
            unsafe { thread_info(thread, SCHED_TIMESHARE_INFO, info.as_mut_ptr(), &mut count) };
        if result == KERN_SUCCESS && count >= 2 && info[1] > 0 {
            return Some(info[1]);
        }
        // Flavor 10 is obsolete in current SDKs. The supported extended
        // flavor exposes pth_priority at integer index 10 (thread_info.h).
        let mut extended = [0i32; 28];
        let mut count = extended.len() as u32;
        let result =
            unsafe { thread_info(thread, EXTENDED_INFO, extended.as_mut_ptr(), &mut count) };
        (result == KERN_SUCCESS && count > 10 && extended[10] > 0).then_some(extended[10])
    }

    fn precedence(thread: u32) -> Option<i32> {
        let mut importance = 0i32;
        let mut count = 1u32;
        let mut get_default = 0i32;
        let result = unsafe {
            thread_policy_get(
                thread,
                PRECEDENCE,
                &mut importance,
                &mut count,
                &mut get_default,
            )
        };
        (result == KERN_SUCCESS && count == 1).then_some(importance)
    }

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    // os/workgroup_base.h: 4-byte signature plus 36 opaque bytes on LP64.
    #[repr(C)]
    pub(super) struct JoinToken {
        sig: u32,
        opaque: [u8; 36],
    }
    impl Default for JoinToken {
        fn default() -> Self {
            Self {
                sig: 0,
                opaque: [0; 36],
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct Constraint {
        period: u32,
        computation: u32,
        constraint: u32,
        preemptible: i32,
    }

    #[derive(Clone, Copy)]
    pub(super) enum PreviousPolicy {
        Constraint(Constraint),
        Standard { base_priority: i32 },
    }

    #[derive(Clone, Copy)]
    pub(super) struct PreviousTiers {
        latency: i32,
        throughput: i32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }
    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioWorkIntervalCreate(
            name: *const libc::c_char,
            clock: u32,
            attr: *const c_void,
        ) -> *mut c_void;
    }
    unsafe extern "C" {
        pub(super) fn os_workgroup_join(wg: *mut c_void, token: *mut JoinToken) -> i32;
        pub(super) fn os_workgroup_leave(wg: *mut c_void, token: *mut JoinToken);
        fn os_workgroup_max_parallel_threads(wg: *mut c_void, attr: *const c_void) -> i32;
        #[cfg(test)]
        fn os_workgroup_cancel(wg: *mut c_void);
        fn os_workgroup_interval_start(
            wg: *mut c_void,
            start: u64,
            deadline: u64,
            data: *mut c_void,
        ) -> i32;
        fn os_workgroup_interval_finish(wg: *mut c_void, data: *mut c_void) -> i32;
        pub(super) fn os_release(object: *mut c_void);
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
        fn mach_thread_self() -> u32;
        static mach_task_self_: u32;
        fn mach_port_deallocate(task: u32, name: u32) -> i32;
        fn thread_policy_get(
            thread: u32,
            flavor: i32,
            info: *mut i32,
            count: *mut u32,
            get_default: *mut i32,
        ) -> i32;
        fn thread_policy_set(thread: u32, flavor: i32, info: *const i32, count: u32) -> i32;
        fn thread_info(thread: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }

    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    fn task_self() -> u32 {
        // SAFETY: Mach publishes this process's task port as a read-only
        // process-global value (`mach_task_self()` is a C macro for it).
        unsafe { mach_task_self_ }
    }

    pub(super) fn get_workgroup(device: u32) -> Result<Workgroup, i32> {
        let address = Address {
            selector: IO_WORKGROUP,
            scope: GLOBAL,
            element: 0,
        };
        let mut object: *mut c_void = ptr::null_mut();
        let mut size = mem::size_of_val(&object) as u32;
        // SAFETY: HAL writes one retained pointer into `object`.
        let error = unsafe {
            AudioObjectGetPropertyData(
                device,
                &address,
                0,
                ptr::null(),
                &mut size,
                &mut object as *mut _ as *mut c_void,
            )
        };
        if error != 0 {
            return Err(error);
        }
        if size as usize != mem::size_of_val(&object) || object.is_null() {
            if !object.is_null() {
                unsafe { os_release(object) };
            }
            return Err(libc::EINVAL);
        }
        Ok(Workgroup(object))
    }

    pub(super) fn create_audio_interval() -> Result<Workgroup, i32> {
        #[cfg(test)]
        if FORCE_CREATE_FAILURE.get() {
            return Err(libc::ENOTSUP);
        }
        // AudioWorkInterval.h: creates an owned, initially empty interval;
        // OS_CLOCK_MACH_ABSOLUTE_TIME and NULL attr are the documented audio
        // configuration. os_release balances its retained return value.
        let object = unsafe {
            AudioWorkIntervalCreate(
                c"pm-audio-headless".as_ptr(),
                MACH_ABSOLUTE_CLOCK,
                ptr::null(),
            )
        };
        if object.is_null() {
            // AudioWorkIntervalCreate reports failure only by returning NULL;
            // its declaration does not promise errno. Avoid leaking a stale
            // errno from unrelated earlier work as a false API diagnosis.
            Err(libc::ENOTSUP)
        } else {
            Ok(Workgroup(object))
        }
    }

    pub(super) fn max_parallel_threads(group: &Workgroup) -> u32 {
        // os/workgroup_object.h: attr is currently unused and must be NULL.
        // This is an advisory capacity for a workload, not a runtime cap.
        unsafe { os_workgroup_max_parallel_threads(group.0, ptr::null()) }.max(0) as u32
    }

    fn interval_ticks(ns: u64) -> Result<u64, i32> {
        let mut base = Timebase { numer: 0, denom: 0 };
        let result = unsafe { mach_timebase_info(&mut base) };
        if result != KERN_SUCCESS {
            return Err(result);
        }
        if base.numer == 0 || base.denom == 0 {
            return Err(libc::EINVAL);
        }
        let ticks = ns as u128 * base.denom as u128 / base.numer as u128;
        u64::try_from(ticks.max(1)).map_err(|_| libc::EOVERFLOW)
    }

    fn host_us(ticks: u64) -> Result<u64, i32> {
        let mut base = Timebase { numer: 0, denom: 0 };
        // SAFETY: valid writable timebase information, no other preconditions.
        let result = unsafe { mach_timebase_info(&mut base) };
        if result != KERN_SUCCESS {
            return Err(result);
        }
        if base.numer == 0 || base.denom == 0 {
            return Err(libc::EINVAL);
        }
        u64::try_from(ticks as u128 * base.numer as u128 / base.denom as u128 / 1_000)
            .map_err(|_| libc::EOVERFLOW)
    }

    pub(super) fn constraint_ticks(
        period_ns: u64,
        computation_ns: u64,
        constraint_ns: u64,
    ) -> (u32, u32, u32) {
        let mut base = Timebase { numer: 0, denom: 0 };
        // SAFETY: valid writable timebase information.
        let ok = unsafe { mach_timebase_info(&mut base) } == KERN_SUCCESS && base.numer != 0;
        if !ok {
            return (0, 0, 0);
        }
        let ticks = |ns: u64| {
            (ns as u128 * base.denom as u128 / base.numer as u128).clamp(1, u32::MAX as u128) as u32
        };
        let period = ticks(period_ns);
        let constraint = ticks(constraint_ns).min(period);
        let computation = ticks(computation_ns).min(constraint);
        (period, computation, constraint)
    }

    pub(super) fn enter_constraint(
        period: u32,
        computation: u32,
        constraint: u32,
    ) -> Result<PreviousPolicy, i32> {
        if period == 0 || computation == 0 || computation > constraint || constraint > period {
            return Err(libc::EINVAL);
        }
        // mach_thread_self creates a send right; release it after both calls.
        let thread = unsafe { mach_thread_self() };
        let mut old = Constraint::default();
        let mut count = 4;
        let mut get_default = 0;
        let got = unsafe {
            thread_policy_get(
                thread,
                TIME_CONSTRAINT,
                &mut old as *mut _ as *mut i32,
                &mut count,
                &mut get_default,
            )
        };
        let previous = if got == KERN_SUCCESS && count == 4 && get_default == 0 && old.period != 0 {
            PreviousPolicy::Constraint(old)
        } else if let Some(base_priority) = timeshare_base(thread) {
            // These are private audio threads. A QoS USER_INTERACTIVE request
            // can still report Mach timeshare base 31, and legacy Mach policy
            // permanently opts out of pthread QoS. Give the idle worker an
            // explicit interactive Mach base (47), preserving any higher one.
            PreviousPolicy::Standard {
                base_priority: base_priority.max(47),
            }
        } else {
            unsafe { mach_port_deallocate(task_self(), thread) };
            return Err(libc::EINVAL);
        };
        let new = Constraint {
            period,
            computation,
            constraint,
            preemptible: 1,
        };
        let result = unsafe {
            thread_policy_set(thread, TIME_CONSTRAINT, &new as *const _ as *const i32, 4)
        };
        unsafe { mach_port_deallocate(task_self(), thread) };
        if result == KERN_SUCCESS {
            Ok(previous)
        } else {
            Err(result)
        }
    }

    fn tier(thread: u32, flavor: i32) -> Result<i32, i32> {
        let mut value = 0i32;
        let mut count = 1u32;
        let mut get_default = 0i32;
        let result =
            unsafe { thread_policy_get(thread, flavor, &mut value, &mut count, &mut get_default) };
        if result == KERN_SUCCESS && count == 1 {
            Ok(value)
        } else if result != KERN_SUCCESS {
            Err(result)
        } else {
            Err(libc::EINVAL)
        }
    }

    pub(super) fn verify_constraint_on(
        thread: u32,
        period: u32,
        computation: u32,
        constraint: u32,
    ) -> Result<(), i32> {
        let mut actual = Constraint::default();
        let mut count = 4u32;
        let mut get_default = 0i32;
        let result = unsafe {
            thread_policy_get(
                thread,
                TIME_CONSTRAINT,
                &mut actual as *mut _ as *mut i32,
                &mut count,
                &mut get_default,
            )
        };
        if result != KERN_SUCCESS {
            return Err(result);
        }
        if count != 4
            || get_default != 0
            || actual.period != period
            || actual.computation != computation
            || actual.constraint != constraint
            || actual.preemptible != 1
        {
            return Err(libc::EINVAL);
        }
        // XNU retains time-constraint parameters after demotion. Its
        // timeshare-info flavor inspects the effective sched_mode and returns
        // KERN_INVALID_POLICY for non-timeshare modes. Together with the
        // matching constraint parameters, this rejects timeshare demotion;
        // this public query alone does not distinguish RT from fixed mode.
        let mut timeshare = [0i32; 5];
        let mut info_count = timeshare.len() as u32;
        let mode = unsafe {
            thread_info(
                thread,
                SCHED_TIMESHARE_INFO,
                timeshare.as_mut_ptr(),
                &mut info_count,
            )
        };
        match mode {
            KERN_INVALID_POLICY => Ok(()),
            KERN_SUCCESS => Err(libc::EINVAL),
            error => Err(error),
        }
    }

    #[cfg(test)]
    pub(super) fn is_timeshare(thread: u32) -> bool {
        let mut info = [0i32; 5];
        let mut count = info.len() as u32;
        // SAFETY: `thread` is a borrowed, live Mach thread port and the
        // output storage has exactly the requested integer count.
        (unsafe { thread_info(thread, SCHED_TIMESHARE_INFO, info.as_mut_ptr(), &mut count) })
            == KERN_SUCCESS
            && count >= 2
    }

    pub(super) fn verify_constraint(
        period: u32,
        computation: u32,
        constraint: u32,
    ) -> Result<(), i32> {
        let thread = unsafe { mach_thread_self() };
        let active = verify_constraint_on(thread, period, computation, constraint);
        unsafe { mach_port_deallocate(task_self(), thread) };
        active
    }

    pub(super) fn enter_tier_hints(metrics: &MetricCounters) -> Result<PreviousTiers, i32> {
        let thread = unsafe { mach_thread_self() };
        let result = (|| {
            let previous = PreviousTiers {
                latency: tier(thread, LATENCY_QOS)?,
                throughput: tier(thread, THROUGHPUT_QOS)?,
            };
            let requested = PreviousTiers {
                latency: LATENCY_TIER_0,
                throughput: THROUGHPUT_TIER_0,
            };
            let latency_result =
                unsafe { thread_policy_set(thread, LATENCY_QOS, &requested.latency, 1) };
            if latency_result != KERN_SUCCESS {
                return Err(latency_result);
            }
            let throughput_result =
                unsafe { thread_policy_set(thread, THROUGHPUT_QOS, &requested.throughput, 1) };
            if throughput_result != KERN_SUCCESS {
                restore_tier_hints_on(thread, previous, metrics);
                return Err(throughput_result);
            }
            if tier(thread, LATENCY_QOS) != Ok(requested.latency)
                || tier(thread, THROUGHPUT_QOS) != Ok(requested.throughput)
            {
                restore_tier_hints_on(thread, previous, metrics);
                return Err(libc::EINVAL);
            }
            Ok(previous)
        })();
        unsafe { mach_port_deallocate(task_self(), thread) };
        result
    }

    fn restore_tier_hints_on(thread: u32, previous: PreviousTiers, metrics: &MetricCounters) {
        for (flavor, value) in [
            (LATENCY_QOS, previous.latency),
            (THROUGHPUT_QOS, previous.throughput),
        ] {
            let result = unsafe { thread_policy_set(thread, flavor, &value, 1) };
            if result != KERN_SUCCESS || tier(thread, flavor) != Ok(value) {
                metrics.restore_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn restore_tier_hints(previous: PreviousTiers, metrics: &MetricCounters) {
        let thread = unsafe { mach_thread_self() };
        restore_tier_hints_on(thread, previous, metrics);
        unsafe { mach_port_deallocate(task_self(), thread) };
    }

    pub(super) fn restore_constraint(previous: PreviousPolicy, metrics: &MetricCounters) {
        let thread = unsafe { mach_thread_self() };
        match previous {
            PreviousPolicy::Constraint(old) => {
                let result = unsafe {
                    thread_policy_set(thread, TIME_CONSTRAINT, &old as *const _ as *const i32, 4)
                };
                if result != KERN_SUCCESS {
                    metrics.restore_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
            PreviousPolicy::Standard { base_priority } => {
                let result = unsafe { thread_policy_set(thread, STANDARD, ptr::null(), 0) };
                if result != KERN_SUCCESS {
                    metrics.restore_failures.fetch_add(1, Ordering::Relaxed);
                }
                // pthread_set_qos_class_self_np cannot undo the legacy Mach
                // API's permanent QoS opt-out. Adjust ordinary precedence
                // against the current base, which can reflect task niceness.
                if let (Some(mut current), Some(mut importance)) =
                    (timeshare_base(thread), precedence(thread))
                {
                    // A nice-adjusted task can report base 31 even while its
                    // hidden task offset makes importance +16 yield base 32.
                    // Read back after each adjustment instead of assuming a
                    // task base. Four steps cover the supported nice range.
                    let mut failed = false;
                    for _ in 0..4 {
                        if current == base_priority {
                            break;
                        }
                        importance =
                            importance.saturating_add(base_priority.saturating_sub(current));
                        let result =
                            unsafe { thread_policy_set(thread, PRECEDENCE, &importance, 1) };
                        if result != KERN_SUCCESS {
                            failed = true;
                            break;
                        }
                        let Some(actual) = timeshare_base(thread) else {
                            failed = true;
                            break;
                        };
                        current = actual;
                    }
                    if failed || current != base_priority {
                        metrics.restore_failures.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    metrics.restore_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        unsafe { mach_port_deallocate(task_self(), thread) };
    }

    struct Budget {
        id: u64,
        depth: u32,
        wall_start: Instant,
        cpu_start_ns: u64,
        last_checkpoint_cpu_ns: u64,
        last_checkpoint_wall: Instant,
        exit_cpu_ns: u64,
        period_ns: u64,
        computation_ns: u64,
        constraint_ns: u64,
        computation_reported: bool,
        constraint_reported: bool,
        dsp_span_active: bool,
        phase: super::RealtimePhase,
        metrics: Arc<MetricCounters>,
    }
    thread_local! { static BUDGET: RefCell<Option<Budget>> = const { RefCell::new(None) }; }
    thread_local! { static THROTTLE_SLEEP_NS: Cell<u64> = const { Cell::new(0) }; }

    pub(super) fn has_active_guard() -> bool {
        BUDGET.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|budget| budget.depth > 0)
        })
    }

    const STRETCH_PROCESS_RESERVE_NS: u64 = 250_000;

    fn phase_reserve(phase: super::RealtimePhase) -> Option<u64> {
        // The short, wall-gated stretch processing steps need less headroom
        // than an unknown unit. Setup, seek, and shuttle retain their larger
        // existing reservations.
        (phase == super::RealtimePhase::StretchProcess).then_some(STRETCH_PROCESS_RESERVE_NS)
    }

    pub(super) fn checkpoint_dsp_step() {
        // Wall elapsed is an upper bound on accrued thread CPU. Do not read
        // CLOCK_THREAD_CPUTIME_ID for each short vendor DSP step.
        let checkpoint = BUDGET.with(|slot| {
            let slot = slot.borrow();
            let budget = slot.as_ref().filter(|budget| budget.depth > 0)?;
            let interval_ns = (budget.period_ns / 16).clamp(1, 100_000);
            (budget.last_checkpoint_wall.elapsed().as_nanos() >= interval_ns as u128)
                .then_some((budget.id, phase_reserve(budget.phase)))
        });
        if let Some((id, reserve)) = checkpoint {
            checkpoint_with_reserve(id, reserve, None, true);
        }
    }

    pub(super) fn thread_cpu_ns() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0;
        if ok {
            ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
        } else {
            0
        }
    }

    pub(super) fn thread_throttle_sleep_ns() -> u64 {
        THROTTLE_SLEEP_NS.get()
    }

    fn same_policy(b: &Budget, period_ns: u64, computation_ns: u64, constraint_ns: u64) -> bool {
        b.period_ns == period_ns
            && b.computation_ns == computation_ns
            && b.constraint_ns == constraint_ns
    }

    pub(super) fn wait_for_policy_transition(
        id: u64,
        period_ns: u64,
        computation_ns: u64,
        constraint_ns: u64,
        metrics: &MetricCounters,
    ) -> Result<(), i32> {
        let old_window = BUDGET.with(|slot| {
            let slot = slot.borrow();
            let Some(b) = slot.as_ref() else {
                return Ok(None);
            };
            if b.depth > 0 {
                // An active nested guard cannot be replaced by another
                // context or role. Reject before changing Mach policy.
                return if b.id == id && same_policy(b, period_ns, computation_ns, constraint_ns) {
                    Ok(None)
                } else {
                    Err(libc::EBUSY)
                };
            }
            if same_policy(b, period_ns, computation_ns, constraint_ns) {
                Ok(None)
            } else {
                Ok(Some((b.wall_start, b.period_ns)))
            }
        })?;
        let Some((wall_start, old_period_ns)) = old_window else {
            return Ok(());
        };
        let old_period = Duration::from_nanos(old_period_ns);
        let wait_start = Instant::now();
        while let Some(remaining) = old_period.checked_sub(wall_start.elapsed()) {
            if remaining.is_zero() {
                break;
            }
            // No TLS borrow, Mach reservation, or queue lock is held here.
            std::thread::sleep(remaining);
        }
        let waited_ns = wait_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        if waited_ns > 0 && wait_start.duration_since(wall_start) < old_period {
            metrics
                .policy_transition_waits
                .fetch_add(1, Ordering::Relaxed);
            metrics
                .policy_transition_wait_ns
                .fetch_add(waited_ns, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(super) fn set_budget(
        id: u64,
        period_ns: u64,
        computation_ns: u64,
        constraint_ns: u64,
        metrics: Arc<MetricCounters>,
    ) {
        BUDGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(previous) = slot
                .as_mut()
                .filter(|b| same_policy(b, period_ns, computation_ns, constraint_ns))
            {
                debug_assert!(previous.depth == 0 || previous.id == id);
                let now = Instant::now();
                let cpu = thread_cpu_ns();
                resume_budget(
                    previous,
                    id,
                    period_ns,
                    computation_ns,
                    constraint_ns,
                    metrics,
                    (cpu, now),
                );
                if previous.wall_start.elapsed() >= Duration::from_nanos(previous.period_ns) {
                    reset_period(previous);
                }
            } else {
                // A changed policy is a fresh reservation only after the
                // previous policy's genuine wall window has elapsed. This
                // never attributes prior CPU to new limits or forgives it
                // merely because the new callback period is shorter.
                debug_assert!(slot.as_ref().is_none_or(|b| {
                    b.depth == 0 && b.wall_start.elapsed() >= Duration::from_nanos(b.period_ns)
                }));
                let now = Instant::now();
                let cpu = thread_cpu_ns();
                *slot = Some(Budget {
                    id,
                    depth: 1,
                    wall_start: now,
                    cpu_start_ns: cpu,
                    last_checkpoint_cpu_ns: cpu,
                    last_checkpoint_wall: now,
                    exit_cpu_ns: cpu,
                    period_ns,
                    computation_ns,
                    constraint_ns,
                    computation_reported: false,
                    constraint_reported: false,
                    dsp_span_active: false,
                    phase: super::RealtimePhase::Unknown,
                    metrics,
                });
            }
        });
        // A new job on this worker must honor compute already spent by an
        // earlier job in the same period before it starts more DSP.
        checkpoint(id);
    }

    fn resume_budget(
        b: &mut Budget,
        id: u64,
        period_ns: u64,
        computation_ns: u64,
        constraint_ns: u64,
        metrics: Arc<MetricCounters>,
        (cpu_now, wall_now): (u64, Instant),
    ) {
        // Different viewers sharing a pool thread cannot erase guarded CPU
        // already spent in this reservation window.
        if b.depth == 0 {
            // Exclude pool-control or unrelated CPU between guards.
            b.cpu_start_ns = b
                .cpu_start_ns
                .saturating_add(cpu_now.saturating_sub(b.exit_cpu_ns));
            b.last_checkpoint_cpu_ns = cpu_now;
            b.last_checkpoint_wall = wall_now;
        }
        b.id = id;
        b.period_ns = period_ns;
        b.computation_ns = computation_ns;
        b.constraint_ns = constraint_ns;
        b.metrics = metrics;
        b.phase = super::RealtimePhase::Unknown;
        b.dsp_span_active = false;
        // The reservation window owns these flags, not its diagnostic
        // context. Re-entering the same policy must not count an earlier
        // job's overrun again when an idle guard inherits its CPU debt.
        b.depth = b.depth.saturating_add(1);
    }
    pub(super) fn clear_budget(id: u64) {
        BUDGET.with(|slot| {
            if let Some(budget) = slot.borrow_mut().as_mut().filter(|x| x.id == id) {
                // Observe the last work unit even when no source/DSP callback
                // made another checkpoint before this guard exited.
                let now = Instant::now();
                let cpu = thread_cpu_ns();
                sample(budget, now, cpu);
                budget.depth = budget.depth.saturating_sub(1);
                if budget.depth == 0 {
                    budget.exit_cpu_ns = cpu;
                    budget.dsp_span_active = false;
                }
            }
        });
    }

    fn sample(b: &mut Budget, now: Instant, cpu: u64) {
        let unit = cpu.saturating_sub(b.last_checkpoint_cpu_ns);
        if b.dsp_span_active {
            b.metrics
                .dsp_unit_observations
                .fetch_add(1, Ordering::Relaxed);
            b.metrics.max_dsp_unit_ns.fetch_max(unit, Ordering::Relaxed);
            b.metrics.max_dsp_unit_fraction_ppm.fetch_max(
                dsp_unit_fraction_ppm(unit, b.computation_ns),
                Ordering::Relaxed,
            );
        }
        if crate::profile::enabled() {
            crate::profile::record_unit(unit);
        }
        let period_cpu = cpu.saturating_sub(b.cpu_start_ns);
        b.metrics.max_unit_ns.fetch_max(unit, Ordering::Relaxed);
        b.metrics
            .max_period_cpu_ns
            .fetch_max(period_cpu, Ordering::Relaxed);
        if period_cpu > b.computation_ns && !b.computation_reported {
            // This branch runs once per offending reservation. The winner
            // writes a fixed atomic record; there is no RT allocation, lock,
            // log, or string formatting on this path.
            b.metrics
                .first_computation_overrun
                .record(super::ComputationOverrun {
                    host_us: crate::clock::host_now_us(),
                    role: if b.constraint_ns < b.period_ns {
                        "producer"
                    } else {
                        "worker"
                    },
                    phase: b.phase.label(),
                    unit_cpu_ns: unit,
                    prior_period_cpu_ns: b.last_checkpoint_cpu_ns.saturating_sub(b.cpu_start_ns),
                    period_cpu_ns: period_cpu,
                    wall_elapsed_ns: now
                        .duration_since(b.wall_start)
                        .as_nanos()
                        .min(u64::MAX as u128) as u64,
                    period_ns: b.period_ns,
                    computation_ns: b.computation_ns,
                    constraint_ns: b.constraint_ns,
                });
            b.metrics
                .computation_overruns
                .fetch_add(1, Ordering::Relaxed);
            b.computation_reported = true;
        }
        // Waiting for controls or the next I/O activation is not DSP work.
        // CPU service beyond this role's actual Mach constraint violates it,
        // independently of normal blocked/idle wall time.
        if period_cpu > b.constraint_ns && !b.constraint_reported {
            b.metrics
                .constraint_overruns
                .fetch_add(1, Ordering::Relaxed);
            b.constraint_reported = true;
        }
        b.last_checkpoint_cpu_ns = cpu;
        b.last_checkpoint_wall = now;
    }

    fn dsp_unit_fraction_ppm(unit_ns: u64, computation_ns: u64) -> u64 {
        if computation_ns == 0 {
            return if unit_ns == 0 { 0 } else { u64::MAX };
        }
        // Multiplication and ceiling division stay in u128 even for the
        // largest possible u64 unit and budget values.
        (u128::from(unit_ns) * 1_000_000)
            .div_ceil(u128::from(computation_ns))
            .min(u128::from(u64::MAX)) as u64
    }

    fn reset_period(b: &mut Budget) {
        let now = Instant::now();
        let cpu = thread_cpu_ns();
        b.wall_start = now;
        b.cpu_start_ns = cpu;
        b.last_checkpoint_cpu_ns = cpu;
        b.last_checkpoint_wall = b.wall_start;
        b.computation_reported = false;
        b.constraint_reported = false;
    }

    fn reservation_requires_next_period(
        period_cpu_ns: u64,
        computation_ns: u64,
        requested_reserve_ns: Option<u64>,
    ) -> bool {
        // Ordinary checkpoints retain their historical 500 µs reserve.
        // A caller that knows the next indivisible unit can request more
        // headroom, up to the callback's computation allowance.
        let reserve = requested_reserve_ns
            .map(|unit| unit.min(computation_ns))
            .unwrap_or_else(|| (computation_ns / 2).min(500_000));
        period_cpu_ns >= computation_ns.saturating_sub(reserve)
    }

    pub(super) fn checkpoint(id: u64) {
        checkpoint_with_reserve(id, None, Some(super::RealtimePhase::Unknown), false);
        // The sampled work belongs to the preceding profile phase. Clear it
        // only after sampling, so following unlabelled work cannot inherit it.
        if crate::profile::enabled() {
            crate::profile::phase("");
        }
    }

    fn checkpoint_with_reserve(
        id: u64,
        requested_reserve_ns: Option<u64>,
        next_phase: Option<super::RealtimePhase>,
        next_dsp_span: bool,
    ) {
        // Never hold the thread-local budget borrow while yielding to a
        // pending producer control. DSP wall-gated hooks reach this only
        // after their existing short-step interval.
        super::yield_to_pending_control();
        BUDGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(b) = slot.as_mut().filter(|b| b.id == id && b.depth > 0) else {
                return;
            };
            let now = Instant::now();
            let cpu_now = thread_cpu_ns();
            sample(b, now, cpu_now);
            let elapsed = now.duration_since(b.wall_start);
            let period_cpu = cpu_now.saturating_sub(b.cpu_start_ns);
            if reservation_requires_next_period(period_cpu, b.computation_ns, requested_reserve_ns)
            {
                let period = Duration::from_nanos(b.period_ns);
                if elapsed < period {
                    let sleep_start = Instant::now();
                    std::thread::sleep(period - elapsed);
                    b.metrics.throttle_sleeps.fetch_add(1, Ordering::Relaxed);
                    let slept_ns = sleep_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                    b.metrics
                        .total_sleep_ns
                        .fetch_add(slept_ns, Ordering::Relaxed);
                    THROTTLE_SLEEP_NS.with(|total| total.set(total.get().saturating_add(slept_ns)));
                }
                reset_period(b);
            } else if elapsed >= Duration::from_nanos(b.period_ns) {
                reset_period(b);
            }
            if let Some(phase) = next_phase {
                b.phase = phase;
            }
            // `sample` above attributed the preceding span. A wall-gated
            // DSP hook that did not reach this function retains its marker.
            b.dsp_span_active = next_dsp_span;
        });
    }
    pub(super) fn checkpoint_current() {
        BUDGET.with(|slot| {
            let id = slot.borrow().as_ref().map(|b| b.id);
            if let Some(id) = id {
                checkpoint(id);
            }
        });
    }

    pub(super) fn checkpoint_current_phase(phase: super::RealtimePhase) {
        BUDGET.with(|slot| {
            let id = slot.borrow().as_ref().map(|b| b.id);
            if let Some(id) = id {
                checkpoint_with_reserve(id, phase_reserve(phase), Some(phase), false);
            }
        });
    }

    pub(super) fn checkpoint_current_phase_reserve(phase: super::RealtimePhase, unit_ns: u64) {
        BUDGET.with(|slot| {
            let id = slot.borrow().as_ref().map(|b| b.id);
            if let Some(id) = id {
                checkpoint_with_reserve(id, Some(unit_ns), Some(phase), false);
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::realtime::{AudioSchedule, ScheduleMethod};

        #[test]
        fn sampled_first_overrun_reports_actual_role_and_preceding_phase() {
            let metrics = Arc::new(MetricCounters::default());
            let now = Instant::now();
            let mut budget = Budget {
                id: 1,
                depth: 1,
                wall_start: now,
                cpu_start_ns: 1_000_000,
                last_checkpoint_cpu_ns: 1_500_000,
                last_checkpoint_wall: now,
                exit_cpu_ns: 1_500_000,
                period_ns: 1_333_333,
                computation_ns: 1_066_666,
                constraint_ns: 1_133_333,
                computation_reported: false,
                constraint_reported: false,
                dsp_span_active: false,
                phase: super::super::RealtimePhase::StretchFresh,
                metrics: metrics.clone(),
            };
            sample(&mut budget, now, 2_100_000);
            let record = metrics.snapshot().first_computation_overrun.unwrap();
            assert_eq!(record.role, "producer");
            assert_eq!(record.phase, "stretch-fresh");
            assert_eq!(record.unit_cpu_ns, 600_000);
            assert_eq!(record.prior_period_cpu_ns, 500_000);
            assert_eq!(record.period_cpu_ns, 1_100_000);
            assert_eq!(record.constraint_ns, 1_133_333);
            assert_eq!(metrics.snapshot().computation_overruns, 1);
            assert_eq!(metrics.snapshot().constraint_overruns, 0);
        }

        #[test]
        fn labelled_and_plain_checkpoints_attribute_then_clear_the_preceding_phase() {
            std::thread::spawn(|| {
                const ID: u64 = u64::MAX - 90;
                let first = Arc::new(MetricCounters::default());
                let now = Instant::now();
                let cpu = thread_cpu_ns();
                BUDGET.with(|slot| {
                    *slot.borrow_mut() = Some(Budget {
                        id: ID,
                        depth: 1,
                        wall_start: now - Duration::from_millis(60),
                        cpu_start_ns: cpu.saturating_sub(10_000),
                        last_checkpoint_cpu_ns: cpu,
                        last_checkpoint_wall: now - Duration::from_millis(1),
                        exit_cpu_ns: cpu,
                        period_ns: 50_000_000,
                        computation_ns: 1,
                        constraint_ns: 50_000_000,
                        computation_reported: false,
                        constraint_reported: false,
                        dsp_span_active: false,
                        phase: crate::realtime::RealtimePhase::StretchMap,
                        metrics: first.clone(),
                    });
                });

                // A label takes effect after its checkpoint samples the
                // previous unit; the expired synthetic wall window avoids
                // sleeping or changing any real Mach policy in this test.
                crate::realtime::checkpoint_phase("stretch-read");
                assert_eq!(
                    first.snapshot().first_computation_overrun.unwrap().phase,
                    "stretch-map"
                );
                assert_eq!(
                    BUDGET.with(|slot| slot.borrow().as_ref().unwrap().phase),
                    crate::realtime::RealtimePhase::StretchRead
                );

                let rearm = |metrics: Arc<MetricCounters>| {
                    BUDGET.with(|slot| {
                        let mut slot = slot.borrow_mut();
                        let b = slot.as_mut().unwrap();
                        let cpu = thread_cpu_ns();
                        b.wall_start = Instant::now() - Duration::from_millis(60);
                        b.cpu_start_ns = cpu.saturating_sub(10_000);
                        b.last_checkpoint_cpu_ns = cpu;
                        b.last_checkpoint_wall = Instant::now() - Duration::from_millis(1);
                        b.computation_reported = false;
                        b.constraint_reported = false;
                        b.metrics = metrics;
                    });
                };

                let dsp = Arc::new(MetricCounters::default());
                rearm(dsp.clone());
                crate::realtime::checkpoint_dsp_step();
                assert_eq!(
                    dsp.snapshot().first_computation_overrun.unwrap().phase,
                    "stretch-read"
                );
                assert_eq!(
                    BUDGET.with(|slot| slot.borrow().as_ref().unwrap().phase),
                    crate::realtime::RealtimePhase::StretchRead
                );

                let plain = Arc::new(MetricCounters::default());
                rearm(plain.clone());
                crate::realtime::checkpoint();
                assert_eq!(
                    plain.snapshot().first_computation_overrun.unwrap().phase,
                    "stretch-read"
                );
                assert_eq!(
                    BUDGET.with(|slot| slot.borrow().as_ref().unwrap().phase),
                    crate::realtime::RealtimePhase::Unknown
                );

                let next = Arc::new(MetricCounters::default());
                rearm(next.clone());
                crate::realtime::checkpoint();
                assert_eq!(
                    next.snapshot().first_computation_overrun.unwrap().phase,
                    "unknown"
                );
                BUDGET.with(|slot| *slot.borrow_mut() = None);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn dsp_spans_are_counted_only_after_admitted_hooks() {
            std::thread::spawn(|| {
                const ID: u64 = u64::MAX - 91;
                let metrics = Arc::new(MetricCounters::default());
                let now = Instant::now();
                let cpu = thread_cpu_ns();
                BUDGET.with(|slot| {
                    *slot.borrow_mut() = Some(Budget {
                        id: ID,
                        depth: 1,
                        // The first checkpoint starts a fresh synthetic
                        // period without sleeping or touching Mach policy.
                        wall_start: now - Duration::from_millis(60),
                        cpu_start_ns: cpu,
                        last_checkpoint_cpu_ns: cpu,
                        last_checkpoint_wall: now - Duration::from_millis(1),
                        exit_cpu_ns: cpu,
                        period_ns: 50_000_000,
                        computation_ns: 40_000_000,
                        constraint_ns: 50_000_000,
                        computation_reported: false,
                        constraint_reported: false,
                        dsp_span_active: false,
                        phase: crate::realtime::RealtimePhase::Unknown,
                        metrics: metrics.clone(),
                    });
                });

                crate::realtime::checkpoint_dsp_step();
                assert_eq!(metrics.snapshot().dsp_unit_observations, 0);
                assert!(BUDGET.with(|slot| slot.borrow().as_ref().unwrap().dsp_span_active));

                // Both labeled and reserve-bearing checkpoints sample the
                // preceding DSP span, then begin ordinary work.
                crate::realtime::checkpoint_phase("stretch-read");
                assert_eq!(metrics.snapshot().dsp_unit_observations, 1);
                assert!(!BUDGET.with(|slot| slot.borrow().as_ref().unwrap().dsp_span_active));
                crate::realtime::checkpoint_phase_reserve("stretch-fresh", 1_000_000);
                assert_eq!(metrics.snapshot().dsp_unit_observations, 1);

                BUDGET.with(|slot| {
                    slot.borrow_mut().as_mut().unwrap().last_checkpoint_wall =
                        Instant::now() - Duration::from_millis(1);
                });
                crate::realtime::checkpoint_dsp_step();
                assert_eq!(metrics.snapshot().dsp_unit_observations, 1);
                BUDGET.with(|slot| {
                    slot.borrow_mut().as_mut().unwrap().last_checkpoint_wall =
                        Instant::now() + Duration::from_secs(1);
                });
                // A gated-out hook leaves the admitted span in force. The
                // synthetic future timestamp makes the skip deterministic.
                crate::realtime::checkpoint_dsp_step();
                assert!(BUDGET.with(|slot| slot.borrow().as_ref().unwrap().dsp_span_active));
                assert_eq!(metrics.snapshot().dsp_unit_observations, 1);
                crate::realtime::checkpoint();
                assert_eq!(metrics.snapshot().dsp_unit_observations, 2);
                crate::realtime::checkpoint();
                assert_eq!(metrics.snapshot().dsp_unit_observations, 2);

                BUDGET.with(|slot| {
                    slot.borrow_mut().as_mut().unwrap().last_checkpoint_wall =
                        Instant::now() - Duration::from_millis(1);
                });
                crate::realtime::checkpoint_dsp_step();
                clear_budget(ID);
                let snapshot = metrics.snapshot();
                assert_eq!(snapshot.dsp_unit_observations, 3);
                assert!(snapshot.max_dsp_unit_ns > 0);
                assert!(snapshot.max_dsp_unit_fraction_ppm > 0);
                assert!(!BUDGET.with(|slot| slot.borrow().as_ref().unwrap().dsp_span_active));
                BUDGET.with(|slot| *slot.borrow_mut() = None);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn dsp_fraction_rounds_up_without_u64_overflow() {
            assert_eq!(dsp_unit_fraction_ppm(1, 3), 333_334);
            assert_eq!(dsp_unit_fraction_ppm(1, 1_000_000), 1);
            assert_eq!(dsp_unit_fraction_ppm(0, 3), 0);
            assert_eq!(dsp_unit_fraction_ppm(u64::MAX, u64::MAX), 1_000_000);
            assert_eq!(dsp_unit_fraction_ppm(u64::MAX, 1), u64::MAX);
        }

        #[test]
        fn unknown_callback_size_uses_headless_512_frame_period() {
            let unknown = AudioSchedule::new(48_000, 0, None);
            let headless = AudioSchedule::headless(48_000, 512);
            let actual_small = AudioSchedule::new(48_000, 64, None);
            assert_eq!(unknown.inner.period_ns, headless.inner.period_ns);
            assert_eq!(unknown.inner.computation_ns, headless.inner.computation_ns);
            assert!(unknown.inner.custom_interval.is_none());
            assert_eq!(unknown.max_parallel_threads(), 0);
            assert!(headless.has_custom_interval());
            assert_eq!(unknown.inner.period_ns, 10_666_666);
            assert_eq!(unknown.inner.computation_ns, 10_133_332);
            assert_eq!(actual_small.inner.period_ns, 1_333_333);
        }

        #[test]
        fn mach_only_guard_installs_a_real_mach_constraint() {
            // Legacy Mach policy permanently opts its pthread out of QoS.
            // A dedicated thread contains that effect to an owned worker.
            std::thread::spawn(|| {
                assert_eq!(
                    unsafe {
                        libc::pthread_set_qos_class_self_np(
                            libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
                            0,
                        )
                    },
                    0
                );
                let before_base = crate::realtime::current_thread_priority().unwrap();
                let idle_target = before_base.max(47);
                let before_default = policy_default_flag();
                let schedule = AudioSchedule::new(48_000, 512, None);
                assert!(!crate::realtime::has_active_guard());
                let guard = schedule.enter();
                assert!(crate::realtime::has_active_guard());
                assert_eq!(
                    guard.method(),
                    ScheduleMethod::MachTimeConstraint,
                    "{:?}",
                    guard.status()
                );
                assert!(!schedule.interval_master());
                let thread = unsafe { mach_thread_self() };
                let mut current = Constraint::default();
                let mut count = 4;
                let mut get_default = 0;
                let result = unsafe {
                    thread_policy_get(
                        thread,
                        TIME_CONSTRAINT,
                        &mut current as *mut _ as *mut i32,
                        &mut count,
                        &mut get_default,
                    )
                };
                unsafe { mach_port_deallocate(task_self(), thread) };
                assert_eq!(result, KERN_SUCCESS);
                assert_eq!(count, 4);
                assert_eq!(get_default, 0);
                assert_eq!(current.period, schedule.inner.period);
                assert_eq!(current.computation, schedule.inner.computation);
                drop(guard);
                assert!(!crate::realtime::has_active_guard());
                assert_eq!(
                    crate::realtime::current_thread_priority(),
                    Some(idle_target)
                );
                assert_eq!(policy_default_flag(), before_default);
                assert_eq!(schedule.metrics().restore_failures, 0);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn producer_deadline_is_exact_and_survives_a_blocked_wake() {
            let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
            let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(0);
            let worker = std::thread::spawn(move || {
                let schedule = AudioSchedule::headless(48_000, 512);
                let p = schedule.inner.period_ns;
                assert_eq!(schedule.inner.producer_computation_ns, p * 4 / 5);
                assert_eq!(schedule.inner.producer_constraint_ns, p * 17 / 20);
                assert!(schedule.inner.producer_computation > 0);
                assert!(schedule.inner.producer_computation < schedule.inner.producer_constraint);
                assert!(schedule.inner.producer_constraint < schedule.inner.period);
                assert!(schedule.inner.computation > schedule.inner.producer_constraint);

                let before_default = policy_default_flag();
                let idle_target = crate::realtime::current_thread_priority().unwrap().max(47);
                let guard = schedule.enter_producer();
                assert_eq!(
                    guard.method(),
                    ScheduleMethod::CustomAudioInterval,
                    "{:?}",
                    guard.status(),
                );
                let verify = || {
                    verify_constraint(
                        schedule.inner.period,
                        schedule.inner.producer_computation,
                        schedule.inner.producer_constraint,
                    )
                };
                assert_eq!(verify(), Ok(()));
                ready_tx.send(()).unwrap();
                wake_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(verify(), Ok(()), "producer lost RT mode on wake");

                drop(guard);
                assert_eq!(policy_default_flag(), before_default);
                assert_eq!(
                    crate::realtime::current_thread_priority(),
                    Some(idle_target)
                );
                assert_eq!(schedule.metrics().restore_failures, 0);
                assert_eq!(schedule.metrics().constraint_readback_failures, 0);
            });
            ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            // Ensure the worker has entered its receiver wait, so the second
            // readback checks the policy after a real blocked wake.
            std::thread::sleep(Duration::from_millis(3));
            wake_tx.send(()).unwrap();
            worker.join().unwrap();
        }

        #[test]
        fn custom_interval_has_one_master_and_closes_each_cycle() {
            std::thread::spawn(|| {
                let schedule = AudioSchedule::headless(48_000, 512);
                assert!(schedule.has_custom_interval());
                let producer = schedule.enter_producer();
                assert_eq!(producer.method(), ScheduleMethod::CustomAudioInterval);
                assert!(schedule.interval_master());
                assert!(!schedule.interval_active());
                let second = schedule.enter_producer();
                assert_eq!(second.method(), ScheduleMethod::Unavailable);
                assert_eq!(second.status().workgroup_error, Some(libc::EALREADY));
                drop(second);

                let worker_schedule = schedule.clone();
                std::thread::spawn(move || {
                    let worker = worker_schedule.enter();
                    assert_eq!(worker.method(), ScheduleMethod::CustomAudioInterval);
                    assert!(!worker_schedule.interval_master());
                    // Membership alone does not grant interval-master rights.
                    assert_eq!(
                        worker_schedule.begin_interval(crate::clock::host_now_us()),
                        Err(libc::EPERM)
                    );
                })
                .join()
                .unwrap();

                assert_eq!(schedule.begin_interval(crate::clock::host_now_us()), Ok(()));
                assert!(schedule.interval_active());
                assert_eq!(schedule.metrics().interval_starts, 1);
                // A superseding cycle finishes the old one exactly once.
                assert_eq!(schedule.begin_interval(crate::clock::host_now_us()), Ok(()));
                assert_eq!(schedule.metrics().interval_starts, 2);
                assert_eq!(schedule.metrics().interval_finishes, 1);
                assert_eq!(schedule.finish_interval(), Ok(()));
                assert_eq!(schedule.finish_interval(), Ok(()));
                assert!(!schedule.interval_active());
                assert!(schedule.interval_master());
                assert_eq!(schedule.metrics().interval_finishes, 2);

                let expired = crate::clock::host_now_us()
                    .saturating_sub(schedule.inner.period_ns.div_ceil(1_000) + 1);
                assert_eq!(schedule.begin_interval(expired), Err(libc::ETIMEDOUT));
                assert_eq!(schedule.metrics().interval_expired, 1);
                assert_eq!(schedule.begin_interval(crate::clock::host_now_us()), Ok(()));
                drop(producer);
                assert!(!schedule.interval_master());
                let metrics = schedule.metrics();
                assert_eq!(metrics.interval_starts, 3);
                assert_eq!(metrics.interval_finishes, 3);
                assert_eq!(metrics.interval_auto_finishes, 1);
                assert_eq!(metrics.restore_failures, 0);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn producer_reentry_reanchors_the_same_request_without_renewing_its_deadline() {
            std::thread::spawn(|| {
                // A longer test period tolerates scheduler pauses while still
                // checking the same request across actual guard teardown.
                let schedule = AudioSchedule::headless(48_000, 1_920);
                let request = crate::clock::host_now_us();
                let guard = schedule.enter_producer();
                assert_eq!(guard.method(), ScheduleMethod::CustomAudioInterval);
                assert_eq!(schedule.begin_interval(request), Ok(()));
                let first = schedule.metrics();
                assert_eq!(first.interval_last_requested_us, request);
                assert_eq!(first.interval_last_deadline_us, request + 40_000);
                assert!(first.interval_last_start_tick > 0);
                assert!(first.interval_last_deadline_tick > first.interval_last_start_tick);
                drop(guard); // The active cycle is auto-finished once.
                assert_eq!(schedule.metrics().interval_auto_finishes, 1);

                std::thread::sleep(Duration::from_millis(1));
                let guard = schedule.enter_producer();
                assert_eq!(guard.method(), ScheduleMethod::CustomAudioInterval);
                assert_eq!(schedule.begin_interval(request), Ok(()));
                let second = schedule.metrics();
                assert_eq!(
                    second.interval_last_requested_us,
                    first.interval_last_requested_us
                );
                assert_eq!(
                    second.interval_last_deadline_us,
                    first.interval_last_deadline_us
                );
                assert_eq!(second.interval_last_requested_us, request);
                assert!(second.interval_last_start_tick > 0);
                assert!(second.interval_last_deadline_tick > second.interval_last_start_tick);
                let period_ticks = interval_ticks(schedule.inner.period_ns).unwrap();
                for interval in [first, second] {
                    assert!(
                        interval.interval_last_anchor_tick >= interval.interval_last_start_tick
                    );
                    assert!(
                        interval.interval_last_deadline_tick > interval.interval_last_anchor_tick
                    );
                    let submitted_period =
                        interval.interval_last_deadline_tick - interval.interval_last_start_tick;
                    // Two individually truncated conversions can lose one tick.
                    assert!(submitted_period >= period_ticks.saturating_sub(1));
                    assert!(submitted_period <= period_ticks);
                }
                // Host time is quantized to microseconds. A second SDK call
                // may round by that quantum, but cannot renew a whole period.
                assert!(
                    second.interval_last_deadline_tick
                        <= first.interval_last_deadline_tick + interval_ticks(2_000).unwrap()
                );
                assert!(second.interval_last_reanchor_us >= first.interval_last_reanchor_us);
                assert_eq!(second.interval_starts, 2);
                assert_eq!(second.interval_finishes, 1);
                assert_eq!(schedule.finish_interval(), Ok(()));
                drop(guard);

                let mach_only = AudioSchedule::new(48_000, 512, None);
                assert_eq!(mach_only.begin_interval(request), Ok(()));
                assert_eq!(mach_only.metrics().interval_starts, 0);
                assert_eq!(mach_only.metrics().interval_last_deadline_us, 0);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn idle_mach_context_preserves_callback_policy_without_retaining_group() {
            let active = AudioSchedule::headless(48_000, 512);
            let idle = active.mach_only();
            assert_eq!(idle.period_ns(), active.period_ns());
            assert_eq!(idle.inner.period, active.inner.period);
            assert_eq!(idle.inner.computation, active.inner.computation);
            assert_eq!(
                idle.inner.producer_computation,
                active.inner.producer_computation
            );
            assert_eq!(
                idle.inner.producer_constraint,
                active.inner.producer_constraint
            );
            assert!(idle.inner.workgroup.is_none());
            assert!(idle.inner.custom_interval.is_none());
            assert!(!idle.inner.custom_requested);
            assert_ne!(idle.inner.id, active.inner.id);
            assert_eq!(idle.metrics().interval_starts, 0);
        }

        #[test]
        fn failed_custom_creation_and_join_keep_mach_policy_and_restore_idle_priority() {
            // Confining legacy Mach policy to this owned thread also lets
            // each fallback verify that no producer master was installed.
            std::thread::spawn(|| {
                let idle_target = crate::realtime::current_thread_priority().unwrap().max(47);
                let original_default = policy_default_flag();
                FORCE_CREATE_FAILURE.set(true);
                let creation_failed = AudioSchedule::headless(48_000, 512);
                FORCE_CREATE_FAILURE.set(false);
                assert!(!creation_failed.has_custom_interval());
                assert_eq!(creation_failed.max_parallel_threads(), 0);
                assert_eq!(creation_failed.metrics().interval_failures, 1);
                let guard = creation_failed.enter_producer();
                assert_eq!(guard.method(), ScheduleMethod::MachTimeConstraint);
                assert_eq!(guard.status().workgroup_error, Some(libc::ENOTSUP));
                assert!(!creation_failed.interval_master());
                assert_eq!(
                    verify_constraint(
                        creation_failed.inner.period,
                        creation_failed.inner.producer_computation,
                        creation_failed.inner.producer_constraint,
                    ),
                    Ok(())
                );
                drop(guard);
                assert_eq!(policy_default_flag(), original_default);
                assert_eq!(
                    crate::realtime::current_thread_priority(),
                    Some(idle_target)
                );
                assert_eq!(creation_failed.metrics().restore_failures, 0);

                let join_failed = AudioSchedule::headless(48_000, 512);
                let group = join_failed.inner.custom_interval.as_ref().unwrap();
                // The public SDK specifies EINVAL for a cancelled workgroup
                // join. Cancellation here is test-only, before any member
                // joins or interval cycle starts.
                unsafe { os_workgroup_cancel(group.0) };
                let guard = join_failed.enter_producer();
                assert_eq!(guard.method(), ScheduleMethod::MachTimeConstraint);
                assert_eq!(guard.status().workgroup_error, Some(libc::EINVAL));
                assert!(!join_failed.interval_master());
                assert_eq!(
                    verify_constraint(
                        join_failed.inner.period,
                        join_failed.inner.producer_computation,
                        join_failed.inner.producer_constraint,
                    ),
                    Ok(())
                );
                drop(guard);
                assert_eq!(policy_default_flag(), original_default);
                assert_eq!(
                    crate::realtime::current_thread_priority(),
                    Some(idle_target)
                );
                assert_eq!(join_failed.metrics().interval_failures, 1);
                assert_eq!(join_failed.metrics().restore_failures, 0);
            })
            .join()
            .unwrap();
        }

        #[test]
        fn tier_hints_preserve_repeated_time_constraint_entries() {
            // A dedicated thread contains the Mach policy's QoS opt-out.
            std::thread::spawn(|| {
                let schedule = AudioSchedule::headless(48_000, 512);
                for _ in 0..3 {
                    let thread = unsafe { mach_thread_self() };
                    let old_latency = tier(thread, LATENCY_QOS).unwrap();
                    let old_throughput = tier(thread, THROUGHPUT_QOS).unwrap();
                    unsafe { mach_port_deallocate(task_self(), thread) };

                    let guard = schedule.enter();
                    assert_eq!(guard.method(), ScheduleMethod::CustomAudioInterval);
                    assert_eq!(guard.status().hint_error, None);
                    assert_eq!(
                        verify_constraint(
                            schedule.inner.period,
                            schedule.inner.computation,
                            schedule.inner.period,
                        ),
                        Ok(())
                    );
                    let thread = unsafe { mach_thread_self() };
                    assert_eq!(tier(thread, LATENCY_QOS), Ok(LATENCY_TIER_0));
                    assert_eq!(tier(thread, THROUGHPUT_QOS), Ok(THROUGHPUT_TIER_0));
                    unsafe { mach_port_deallocate(task_self(), thread) };

                    drop(guard);
                    assert_eq!(policy_default_flag(), 1);
                    let thread = unsafe { mach_thread_self() };
                    let mut timeshare = [0i32; 5];
                    let mut count = timeshare.len() as u32;
                    assert_eq!(
                        unsafe {
                            thread_info(
                                thread,
                                SCHED_TIMESHARE_INFO,
                                timeshare.as_mut_ptr(),
                                &mut count,
                            )
                        },
                        KERN_SUCCESS
                    );
                    assert_eq!(tier(thread, LATENCY_QOS), Ok(old_latency));
                    assert_eq!(tier(thread, THROUGHPUT_QOS), Ok(old_throughput));
                    unsafe { mach_port_deallocate(task_self(), thread) };
                }
                assert_eq!(schedule.metrics().hint_failures, 0);
                assert_eq!(schedule.metrics().restore_failures, 0);
            })
            .join()
            .unwrap();
        }

        fn policy_default_flag() -> i32 {
            let thread = unsafe { mach_thread_self() };
            let mut current = Constraint::default();
            let mut count = 4;
            let mut get_default = 0;
            let result = unsafe {
                thread_policy_get(
                    thread,
                    TIME_CONSTRAINT,
                    &mut current as *mut _ as *mut i32,
                    &mut count,
                    &mut get_default,
                )
            };
            unsafe { mach_port_deallocate(task_self(), thread) };
            assert_eq!(result, KERN_SUCCESS);
            get_default
        }

        #[test]
        fn consecutive_jobs_cannot_reset_the_threads_period_budget() {
            const ID: u64 = u64::MAX - 1;
            let now = Instant::now();
            let cpu = thread_cpu_ns();
            let metrics = Arc::new(MetricCounters::default());
            BUDGET.with(|slot| {
                *slot.borrow_mut() = Some(Budget {
                    id: ID,
                    depth: 0, // Previous job's guard has exited.
                    wall_start: now,
                    cpu_start_ns: cpu,
                    last_checkpoint_cpu_ns: cpu,
                    last_checkpoint_wall: now,
                    exit_cpu_ns: cpu,
                    period_ns: 50_000_000,
                    computation_ns: 1, // Already consumed on this thread.
                    constraint_ns: 50_000_000,
                    computation_reported: false,
                    constraint_reported: false,
                    dsp_span_active: false,
                    phase: crate::realtime::RealtimePhase::Unknown,
                    metrics: metrics.clone(),
                })
            });
            set_budget(ID, 50_000_000, 1, 50_000_000, metrics);
            assert!(now.elapsed() >= Duration::from_millis(25));
            clear_budget(ID);
        }

        #[test]
        fn mach_budget_checkpoint_waits_for_the_next_period() {
            const ID: u64 = u64::MAX;
            let now = Instant::now();
            let cpu = thread_cpu_ns();
            let metrics = Arc::new(MetricCounters::default());
            BUDGET.with(|slot| {
                *slot.borrow_mut() = Some(Budget {
                    id: ID,
                    depth: 1,
                    wall_start: now,
                    cpu_start_ns: cpu,
                    last_checkpoint_cpu_ns: cpu,
                    last_checkpoint_wall: now,
                    exit_cpu_ns: cpu,
                    period_ns: 50_000_000,
                    computation_ns: 1,
                    constraint_ns: 50_000_000,
                    computation_reported: false,
                    constraint_reported: false,
                    dsp_span_active: false,
                    phase: crate::realtime::RealtimePhase::Unknown,
                    metrics: metrics.clone(),
                })
            });
            checkpoint(ID);
            clear_budget(ID);
            assert!(now.elapsed() >= Duration::from_millis(25));
            assert!(metrics.snapshot().max_period_cpu_ns > 0);
            assert_eq!(metrics.snapshot().throttle_sleeps, 1);
            assert!(metrics.snapshot().total_sleep_ns >= 25_000_000);
        }

        #[test]
        fn reentry_keeps_debt_without_recounting_an_earlier_contexts_overrun() {
            const OLD: u64 = u64::MAX - 3;
            const NEW: u64 = u64::MAX - 2;
            let old_metrics = Arc::new(MetricCounters::default());
            let new_metrics = Arc::new(MetricCounters::default());
            old_metrics.computation_overruns.store(1, Ordering::Relaxed);
            old_metrics.constraint_overruns.store(1, Ordering::Relaxed);
            let mut b = Budget {
                id: OLD,
                depth: 0,
                wall_start: Instant::now(),
                // 6 ms guarded work already exceeded the old 4/5 ms limits,
                // followed by 5 ms outside the guard.
                cpu_start_ns: 89_000_000,
                last_checkpoint_cpu_ns: 95_000_000,
                last_checkpoint_wall: Instant::now(),
                exit_cpu_ns: 95_000_000,
                period_ns: 50_000_000,
                computation_ns: 4_000_000,
                constraint_ns: 5_000_000,
                computation_reported: true,
                constraint_reported: true,
                dsp_span_active: true,
                phase: crate::realtime::RealtimePhase::Unknown,
                metrics: old_metrics.clone(),
            };
            resume_budget(
                &mut b,
                NEW,
                50_000_000,
                4_000_000,
                5_000_000,
                new_metrics.clone(),
                (100_000_000, Instant::now()),
            );
            assert_eq!(100_000_000 - b.cpu_start_ns, 6_000_000);
            assert!(!b.dsp_span_active);
            assert!(b.computation_reported);
            assert!(b.constraint_reported);
            sample(&mut b, Instant::now(), 100_000_000);
            let observed = new_metrics.snapshot();
            assert_eq!(observed.max_period_cpu_ns, 6_000_000);
            assert_eq!(observed.dsp_unit_observations, 0);
            assert_eq!(observed.computation_overruns, 0);
            assert_eq!(observed.constraint_overruns, 0);
            assert_eq!(observed.throttle_sleeps, 0);
            assert_eq!(old_metrics.snapshot().computation_overruns, 1);
            assert_eq!(old_metrics.snapshot().constraint_overruns, 1);

            // A third viewer with the same policy inherits guarded CPU debt
            // without inheriting ordinary CPU between the two guards.
            b.depth = 0;
            b.exit_cpu_ns = 100_000_000;
            b.computation_reported = true;
            b.constraint_reported = true;
            let third = Arc::new(MetricCounters::default());
            resume_budget(
                &mut b,
                NEW - 1,
                50_000_000,
                4_000_000,
                5_000_000,
                third.clone(),
                (101_000_000, Instant::now()),
            );
            sample(&mut b, Instant::now(), 101_000_000);
            assert_eq!(third.snapshot().max_period_cpu_ns, 6_000_000);
            assert_eq!(third.snapshot().computation_overruns, 0);
        }

        #[test]
        fn shorter_policy_waits_for_the_old_wall_window_before_starting_fresh() {
            const OLD: u64 = u64::MAX - 10;
            const NEW: u64 = u64::MAX - 11;
            let old_start = Instant::now();
            let cpu = thread_cpu_ns();
            let old_metrics = Arc::new(MetricCounters::default());
            let new_metrics = Arc::new(MetricCounters::default());
            BUDGET.with(|slot| {
                *slot.borrow_mut() = Some(Budget {
                    id: OLD,
                    depth: 0,
                    wall_start: old_start,
                    cpu_start_ns: cpu.saturating_sub(100_000),
                    last_checkpoint_cpu_ns: cpu,
                    last_checkpoint_wall: old_start,
                    exit_cpu_ns: cpu,
                    period_ns: 12_000_000,
                    computation_ns: 8_000_000,
                    constraint_ns: 12_000_000,
                    computation_reported: false,
                    constraint_reported: false,
                    dsp_span_active: false,
                    phase: crate::realtime::RealtimePhase::Unknown,
                    metrics: old_metrics,
                });
            });
            wait_for_policy_transition(NEW, 2_000_000, 1_000_000, 2_000_000, &new_metrics).unwrap();
            assert!(old_start.elapsed() >= Duration::from_millis(12));
            assert_eq!(new_metrics.snapshot().policy_transition_waits, 1);
            set_budget(NEW, 2_000_000, 1_000_000, 2_000_000, new_metrics.clone());
            BUDGET.with(|slot| {
                let slot = slot.borrow();
                let fresh = slot.as_ref().unwrap();
                assert_eq!(fresh.id, NEW);
                assert!(fresh.wall_start.duration_since(old_start) >= Duration::from_millis(12));
                assert!(fresh.cpu_start_ns >= cpu);
                assert_eq!(fresh.depth, 1);
            });
            clear_budget(NEW);
            BUDGET.with(|slot| *slot.borrow_mut() = None);
        }

        #[test]
        fn longer_policy_does_not_charge_expired_work_to_its_new_window() {
            const OLD: u64 = u64::MAX - 12;
            const NEW: u64 = u64::MAX - 13;
            let old_start = Instant::now() - Duration::from_millis(15);
            let cpu = thread_cpu_ns();
            let new_metrics = Arc::new(MetricCounters::default());
            BUDGET.with(|slot| {
                *slot.borrow_mut() = Some(Budget {
                    id: OLD,
                    depth: 0,
                    wall_start: old_start,
                    cpu_start_ns: cpu.saturating_sub(100_000),
                    last_checkpoint_cpu_ns: cpu,
                    last_checkpoint_wall: old_start,
                    exit_cpu_ns: cpu,
                    period_ns: 5_000_000,
                    computation_ns: 3_000_000,
                    constraint_ns: 5_000_000,
                    computation_reported: false,
                    constraint_reported: false,
                    dsp_span_active: false,
                    phase: crate::realtime::RealtimePhase::Unknown,
                    metrics: Arc::new(MetricCounters::default()),
                });
            });
            let entered_at = Instant::now();
            wait_for_policy_transition(NEW, 50_000_000, 30_000_000, 50_000_000, &new_metrics)
                .unwrap();
            assert_eq!(new_metrics.snapshot().policy_transition_waits, 0);
            set_budget(NEW, 50_000_000, 30_000_000, 50_000_000, new_metrics.clone());
            BUDGET.with(|slot| {
                let slot = slot.borrow();
                let fresh = slot.as_ref().unwrap();
                assert_eq!(fresh.id, NEW);
                assert!(fresh.wall_start >= entered_at);
                assert!(fresh.cpu_start_ns >= cpu);
                assert_eq!(fresh.depth, 1);
            });
            clear_budget(NEW);
            BUDGET.with(|slot| *slot.borrow_mut() = None);
        }

        #[test]
        fn nested_policy_mismatch_fails_before_changing_mach_constraint() {
            let old = AudioSchedule::new(48_000, 512, None);
            let changed = AudioSchedule::new(48_000, 64, None);
            let guard = old.enter();
            assert_eq!(guard.method(), ScheduleMethod::MachTimeConstraint);
            let nested = changed.enter();
            assert_eq!(nested.method(), ScheduleMethod::Unavailable);
            assert_eq!(nested.status().mach_error, Some(libc::EBUSY));
            assert_eq!(
                verify_constraint(old.inner.period, old.inner.computation, old.inner.period),
                Ok(())
            );
            drop(nested);
            drop(guard);
        }

        #[test]
        fn known_large_unit_requires_next_period_with_carried_cpu_debt() {
            let metrics = Arc::new(MetricCounters::default());
            let now = Instant::now();
            let mut budget = Budget {
                id: 42,
                depth: 0,
                wall_start: now,
                // Prior guarded work used 9.2 ms; subsequent 5 ms of pool
                // control work happened outside the schedule guard.
                cpu_start_ns: 90_800_000,
                last_checkpoint_cpu_ns: 100_000_000,
                last_checkpoint_wall: now,
                exit_cpu_ns: 100_000_000,
                period_ns: 20_000_000,
                computation_ns: 10_000_000,
                constraint_ns: 20_000_000,
                computation_reported: false,
                constraint_reported: false,
                dsp_span_active: false,
                phase: crate::realtime::RealtimePhase::Unknown,
                metrics: metrics.clone(),
            };
            resume_budget(
                &mut budget,
                43,
                20_000_000,
                10_000_000,
                20_000_000,
                metrics,
                (105_000_000, now),
            );
            let carried_cpu_ns = 105_000_000 - budget.cpu_start_ns;
            assert_eq!(carried_cpu_ns, 9_200_000);
            assert!(!reservation_requires_next_period(
                carried_cpu_ns,
                budget.computation_ns,
                None,
            ));
            assert!(reservation_requires_next_period(
                carried_cpu_ns,
                budget.computation_ns,
                Some(1_000_000),
            ));
        }

        #[test]
        fn short_stretch_process_reserve_does_not_change_other_phases() {
            use crate::realtime::RealtimePhase;

            assert_eq!(
                phase_reserve(RealtimePhase::StretchProcess),
                Some(STRETCH_PROCESS_RESERVE_NS)
            );
            // The phase ID table includes setup, seek, shuttle, and unknown
            // work; none of them may inherit the shorter process admission.
            for id in 0..=u8::MAX {
                let phase = RealtimePhase::from_id(id);
                if phase != RealtimePhase::StretchProcess {
                    assert_eq!(phase_reserve(phase), None, "{phase:?}");
                }
            }

            const COMPUTATION_NS: u64 = 1_266_666; // 64 frames at 48 kHz, 95%.
            assert!(!reservation_requires_next_period(
                766_665,
                COMPUTATION_NS,
                phase_reserve(RealtimePhase::Unknown),
            ));
            assert!(reservation_requires_next_period(
                766_666,
                COMPUTATION_NS,
                phase_reserve(RealtimePhase::Unknown),
            ));
            assert!(!reservation_requires_next_period(
                1_016_665,
                COMPUTATION_NS,
                phase_reserve(RealtimePhase::StretchProcess),
            ));
            assert!(reservation_requires_next_period(
                1_016_666,
                COMPUTATION_NS,
                phase_reserve(RealtimePhase::StretchProcess),
            ));
            // Constructor/reset admission still uses its explicit 1 ms.
            assert!(reservation_requires_next_period(
                266_666,
                COMPUTATION_NS,
                Some(1_000_000),
            ));
        }
    }
}
