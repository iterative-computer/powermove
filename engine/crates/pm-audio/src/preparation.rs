//! Detached, cancellable vocoder preparation. A process-wide pool bounds
//! CPU use across viewers; render and device threads never execute its jobs.
use crate::mixer::Mixer;
use pm_eval::Evaluator;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}
fn elapsed_ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

fn control_yield_delta(before: Option<(u64, u64)>) -> (u64, u64) {
    let Some((count, wait_ns)) = before else {
        return (0, 0);
    };
    let (after_count, after_wait_ns) = crate::realtime::thread_control_yield_stats();
    (
        after_count.saturating_sub(count),
        after_wait_ns.saturating_sub(wait_ns) / 1_000,
    )
}

#[cfg(target_os = "macos")]
const QOS_USER_INTERACTIVE: u32 = 0x21;
/// Sets the calling thread's Darwin QoS class and relative priority.
#[cfg(target_os = "macos")]
fn set_qos(class: u32, priority: i32) {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
    }
    // SAFETY: affects only the calling thread; both arguments are valid
    // Darwin QoS values.
    unsafe {
        pthread_set_qos_class_self_np(class, priority);
    }
}

pub(crate) type Evaluators = Arc<dyn Fn() -> Evaluator + Send + Sync>;
struct Job {
    priority: usize,
    critical: Arc<AtomicBool>,
    ready: Box<dyn Fn() -> bool + Send>,
    step: Box<dyn FnMut(Option<ClaimStamp>) -> JobOutcome + Send>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobOutcome {
    /// The claimed voice still owns its history; it stays pending and may
    /// run its next complete DSP call on a different worker.
    PendingCpu,
    /// A completed, missing-input, cancelled, or collided voice has either
    /// sent its one Done record or belongs to an invalidated generation.
    Terminal,
}
#[derive(Clone, Copy)]
struct ClaimStamp {
    host_us: u64,
    at: Instant,
    worker: usize,
    inline: bool,
    wake_drop_us: u64,
    queue_lock_us: u64,
    wake_claim_us: u64,
}
/// Opt-in diagnostic state follows a voice even when its continuation moves
/// to another worker. Ordinary preparation allocates no clocks or log lines.
struct JobTrace {
    queued_at: Option<Instant>,
    enqueue_host_us: u64,
    first_claim_host_us: u64,
    first_worker: usize,
    last_worker: usize,
    worker_handoffs: u64,
    inline_continuations: u64,
    first_queue_us: u64,
    queue_total_us: u64,
    last_slice_end: Option<Instant>,
    wake_stats: (u64, u64, u64),
    enter_us: u64,
    work_us: u64,
    cpu_us: u64,
    throttle_us: u64,
    max_slice_cpu_us: u64,
    control_yields: u64,
    control_wait_us: u64,
    slices: u64,
    /// Number of CPU yields. An inline continuation is not a queue requeue.
    cpu_yields: u64,
    profile: crate::profile::Profile,
}
impl JobTrace {
    fn new() -> Self {
        let enabled = crate::profile::enabled();
        Self {
            queued_at: enabled.then(Instant::now),
            enqueue_host_us: if enabled {
                crate::clock::host_now_us()
            } else {
                0
            },
            first_claim_host_us: 0,
            first_worker: 0,
            last_worker: 0,
            worker_handoffs: 0,
            inline_continuations: 0,
            first_queue_us: 0,
            queue_total_us: 0,
            last_slice_end: None,
            wake_stats: (0, 0, 0),
            enter_us: 0,
            work_us: 0,
            cpu_us: 0,
            throttle_us: 0,
            max_slice_cpu_us: 0,
            control_yields: 0,
            control_wait_us: 0,
            slices: 0,
            cpu_yields: 0,
            profile: Default::default(),
        }
    }
    fn claim(&mut self, stamp: Option<ClaimStamp>) {
        let Some(stamp) = stamp else { return };
        let from = self.last_slice_end.or(self.queued_at).unwrap_or(stamp.at);
        let wait_us = stamp
            .at
            .duration_since(from)
            .as_micros()
            .min(u64::MAX as u128) as u64;
        self.queue_total_us = self.queue_total_us.saturating_add(wait_us);
        if self.first_claim_host_us == 0 {
            self.first_claim_host_us = stamp.host_us;
            self.first_worker = stamp.worker;
            self.first_queue_us = wait_us;
            self.wake_stats = (stamp.wake_drop_us, stamp.queue_lock_us, stamp.wake_claim_us);
        } else if self.last_worker != stamp.worker {
            self.worker_handoffs += 1;
        }
        self.last_worker = stamp.worker;
        self.inline_continuations += u64::from(stamp.inline);
    }
    fn slice(
        &mut self,
        enter_us: u64,
        work_us: u64,
        cpu_us: u64,
        throttle_us: u64,
        control: (u64, u64),
        profile: crate::profile::Profile,
    ) {
        if self.queued_at.is_none() {
            return;
        }
        self.slices += 1;
        self.enter_us = self.enter_us.saturating_add(enter_us);
        self.work_us = self.work_us.saturating_add(work_us);
        self.cpu_us = self.cpu_us.saturating_add(cpu_us);
        self.throttle_us = self.throttle_us.saturating_add(throttle_us);
        self.max_slice_cpu_us = self.max_slice_cpu_us.max(cpu_us);
        self.control_yields = self.control_yields.saturating_add(control.0);
        self.control_wait_us = self.control_wait_us.saturating_add(control.1);
        self.profile.merge(profile);
        self.last_slice_end = Some(Instant::now());
    }
    fn log(&self, generation: u64, start: i64, voice: usize, result: &str) {
        if self.queued_at.is_none() {
            return;
        }
        let host_end_us = crate::clock::host_now_us();
        crate::profile::worker_event(format!(
            "pm-audio-prepare generation={generation} start={start} voice={voice} enqueue_host_us={} host_start_us={} host_claim_us={} host_end_us={host_end_us} queue_us={} queue_total_us={} first_worker={} last_worker={} worker_handoffs={} inline_continuations={} wake_drop_us={} queue_lock_us={} wake_claim_us={} enter_us={} work_us={} cpu_us={} max_slice_cpu_us={} throttle_us={} control_yields={} control_wait_us={} slices={} cpu_yields={} requeues={} ready={} result={result} profile={:?}",
            self.enqueue_host_us,
            self.first_claim_host_us,
            self.first_claim_host_us,
            self.first_queue_us,
            self.queue_total_us,
            self.first_worker,
            self.last_worker,
            self.worker_handoffs,
            self.inline_continuations,
            self.wake_stats.0,
            self.wake_stats.1,
            self.wake_stats.2,
            self.enter_us,
            self.work_us,
            self.cpu_us,
            self.max_slice_cpu_us,
            self.throttle_us,
            self.control_yields,
            self.control_wait_us,
            self.slices,
            self.cpu_yields,
            self.cpu_yields.saturating_sub(self.inline_continuations),
            result == "ready",
            self.profile,
        ));
    }
}
struct Queue {
    jobs: Vec<Job>,
    background: usize,
}
impl Queue {
    fn ready_index(&self) -> Option<(usize, usize)> {
        self.jobs
            .iter()
            .enumerate()
            .filter_map(|(i, j)| {
                let priority = job_priority(j);
                ((priority == 0 || self.background == 0) && (j.ready)()).then_some((i, priority))
            })
            .min_by_key(|(_, priority)| *priority)
    }

    /// A sole runnable foreground voice may keep its exclusive task on the
    /// current worker for one more slice. Every other ready P0 goes first by
    /// FIFO; the caller checks the queue again before each later slice.
    fn may_inline(&self, job: &Job, background: bool) -> bool {
        !background
            && job_priority(job) == 0
            && (job.ready)()
            && !matches!(self.ready_index(), Some((_, 0)))
    }
}
fn job_priority(job: &Job) -> usize {
    if job.critical.load(Ordering::Acquire) {
        0
    } else {
        job.priority
    }
}

pub(crate) fn wake_workers() {
    Executor::shared().unpark_all();
}

fn worker_count(available: usize, performance_cores: Option<usize>) -> usize {
    let total_ceiling = available.saturating_sub(2).max(1);
    performance_cores
        .filter(|&cores| cores > 0 && cores <= available)
        .map_or(total_ceiling, |cores| {
            // Reserve two performance cores for the producer and device
            // callback. Eight workers still prepare sixteen voices in two
            // waves where the total-core cap permits.
            cores.saturating_sub(2).max(8).min(total_ceiling)
        })
}

#[cfg(target_os = "macos")]
fn performance_cores() -> Option<usize> {
    fn name(key: &std::ffi::CStr) -> Option<[u8; 64]> {
        let mut value = [0_u8; 64];
        let mut size = value.len();
        // SAFETY: the key is NUL terminated and the output buffer and size
        // remain valid for the duration of this read-only sysctl call.
        let result = unsafe {
            libc::sysctlbyname(
                key.as_ptr(),
                value.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (result == 0 && size <= value.len()).then_some(value)
    }

    // Only use perflevel0's physical count on a heterogeneous Mac whose
    // reported cluster names identify performance and efficiency cores.
    if !name(c"hw.perflevel0.name")?.starts_with(b"Performance\0")
        || !name(c"hw.perflevel1.name")?.starts_with(b"Efficiency\0")
    {
        return None;
    }
    let mut cores: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&cores);
    // SAFETY: the key is NUL terminated and the integer output has its
    // exact expected size. This reads public hardware topology only.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&mut cores as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && size == std::mem::size_of_val(&cores))
        .then(|| usize::try_from(cores).ok())
        .flatten()
        .filter(|&cores| cores > 0)
}

/// These counters describe the process-wide shared worker pool, not just one
/// viewer. A lease observes concurrent viewers using the same pool.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleStats {
    pub arm_failures: u64,
    pub arm_timeouts: u64,
    /// Transitions into a state with incompatible live callback periods.
    pub conflicts: u64,
    pub wake_count: u64,
    /// Claimed slices, including resumed jobs.
    pub job_claims: u64,
    pub job_requeues: u64,
    pub job_completions: u64,
    pub wake_guard_drop_ns: u64,
    pub queue_lock_wait_ns: u64,
    pub wake_to_claim_ns: u64,
    pub successful_idle_entries: u64,
    pub max_unit_ns: u64,
    pub max_period_cpu_ns: u64,
    pub computation_overruns: u64,
    pub constraint_overruns: u64,
    pub restore_failures: u64,
    pub hint_failures: u64,
    pub constraint_readback_failures: u64,
    pub policy_transition_waits: u64,
    pub policy_transition_wait_ns: u64,
}
impl IdleStats {
    /// Counter changes since `before`; CPU maxima remain process-lifetime
    /// peaks because subtracting maxima would not describe a time window.
    pub fn counter_delta(self, before: Self) -> Self {
        Self {
            arm_failures: self.arm_failures.saturating_sub(before.arm_failures),
            arm_timeouts: self.arm_timeouts.saturating_sub(before.arm_timeouts),
            conflicts: self.conflicts.saturating_sub(before.conflicts),
            wake_count: self.wake_count.saturating_sub(before.wake_count),
            job_claims: self.job_claims.saturating_sub(before.job_claims),
            job_requeues: self.job_requeues.saturating_sub(before.job_requeues),
            job_completions: self.job_completions.saturating_sub(before.job_completions),
            wake_guard_drop_ns: self.wake_guard_drop_ns,
            queue_lock_wait_ns: self.queue_lock_wait_ns,
            wake_to_claim_ns: self.wake_to_claim_ns,
            successful_idle_entries: self
                .successful_idle_entries
                .saturating_sub(before.successful_idle_entries),
            max_unit_ns: self.max_unit_ns,
            max_period_cpu_ns: self.max_period_cpu_ns,
            computation_overruns: self
                .computation_overruns
                .saturating_sub(before.computation_overruns),
            constraint_overruns: self
                .constraint_overruns
                .saturating_sub(before.constraint_overruns),
            restore_failures: self
                .restore_failures
                .saturating_sub(before.restore_failures),
            hint_failures: self.hint_failures.saturating_sub(before.hint_failures),
            constraint_readback_failures: self
                .constraint_readback_failures
                .saturating_sub(before.constraint_readback_failures),
            policy_transition_waits: self
                .policy_transition_waits
                .saturating_sub(before.policy_transition_waits),
            policy_transition_wait_ns: self
                .policy_transition_wait_ns
                .saturating_sub(before.policy_transition_wait_ns),
        }
    }
}

#[derive(Default)]
struct IdleCounters {
    arm_failures: AtomicU64,
    arm_timeouts: AtomicU64,
    conflicts: AtomicU64,
    wake_count: AtomicU64,
    job_claims: AtomicU64,
    job_requeues: AtomicU64,
    job_completions: AtomicU64,
    wake_guard_drop_ns: AtomicU64,
    queue_lock_wait_ns: AtomicU64,
    wake_to_claim_ns: AtomicU64,
    successful_idle_entries: AtomicU64,
    max_unit_ns: AtomicU64,
    max_period_cpu_ns: AtomicU64,
    computation_overruns: AtomicU64,
    constraint_overruns: AtomicU64,
    restore_failures: AtomicU64,
    hint_failures: AtomicU64,
    constraint_readback_failures: AtomicU64,
    policy_transition_waits: AtomicU64,
    policy_transition_wait_ns: AtomicU64,
}
impl IdleCounters {
    fn snapshot(&self) -> IdleStats {
        IdleStats {
            arm_failures: self.arm_failures.load(Ordering::Relaxed),
            arm_timeouts: self.arm_timeouts.load(Ordering::Relaxed),
            conflicts: self.conflicts.load(Ordering::Relaxed),
            wake_count: self.wake_count.load(Ordering::Relaxed),
            job_claims: self.job_claims.load(Ordering::Relaxed),
            job_requeues: self.job_requeues.load(Ordering::Relaxed),
            job_completions: self.job_completions.load(Ordering::Relaxed),
            wake_guard_drop_ns: self.wake_guard_drop_ns.load(Ordering::Relaxed),
            queue_lock_wait_ns: self.queue_lock_wait_ns.load(Ordering::Relaxed),
            wake_to_claim_ns: self.wake_to_claim_ns.load(Ordering::Relaxed),
            successful_idle_entries: self.successful_idle_entries.load(Ordering::Relaxed),
            max_unit_ns: self.max_unit_ns.load(Ordering::Relaxed),
            max_period_cpu_ns: self.max_period_cpu_ns.load(Ordering::Relaxed),
            computation_overruns: self.computation_overruns.load(Ordering::Relaxed),
            constraint_overruns: self.constraint_overruns.load(Ordering::Relaxed),
            restore_failures: self.restore_failures.load(Ordering::Relaxed),
            hint_failures: self.hint_failures.load(Ordering::Relaxed),
            constraint_readback_failures: self.constraint_readback_failures.load(Ordering::Relaxed),
            policy_transition_waits: self.policy_transition_waits.load(Ordering::Relaxed),
            policy_transition_wait_ns: self.policy_transition_wait_ns.load(Ordering::Relaxed),
        }
    }
    #[cfg(target_os = "macos")]
    fn record_metrics(
        &self,
        before: crate::realtime::RealtimeMetrics,
        after: crate::realtime::RealtimeMetrics,
    ) {
        self.max_unit_ns
            .fetch_max(after.max_unit_ns, Ordering::Relaxed);
        self.max_period_cpu_ns
            .fetch_max(after.max_period_cpu_ns, Ordering::Relaxed);
        self.computation_overruns.fetch_add(
            after
                .computation_overruns
                .saturating_sub(before.computation_overruns),
            Ordering::Relaxed,
        );
        self.constraint_overruns.fetch_add(
            after
                .constraint_overruns
                .saturating_sub(before.constraint_overruns),
            Ordering::Relaxed,
        );
        self.restore_failures.fetch_add(
            after
                .restore_failures
                .saturating_sub(before.restore_failures),
            Ordering::Relaxed,
        );
        self.hint_failures.fetch_add(
            after.hint_failures.saturating_sub(before.hint_failures),
            Ordering::Relaxed,
        );
        self.constraint_readback_failures.fetch_add(
            after
                .constraint_readback_failures
                .saturating_sub(before.constraint_readback_failures),
            Ordering::Relaxed,
        );
        self.policy_transition_waits.fetch_add(
            after
                .policy_transition_waits
                .saturating_sub(before.policy_transition_waits),
            Ordering::Relaxed,
        );
        self.policy_transition_wait_ns.fetch_add(
            after
                .policy_transition_wait_ns
                .saturating_sub(before.policy_transition_wait_ns),
            Ordering::Relaxed,
        );
    }
}

struct IdleContext {
    period_ns: u64,
    #[cfg(target_os = "macos")]
    schedule: crate::realtime::AudioSchedule,
}
#[derive(Default)]
struct IdleLeases {
    contexts: HashMap<u64, Arc<IdleContext>>,
    selected: Option<Arc<IdleContext>>,
    conflict: bool,
    epoch: u64,
}
impl IdleLeases {
    /// Returns true only on entry into a conflicting-period state.
    fn recompute(&mut self) -> bool {
        let was_conflicted = self.conflict;
        let mut contexts = self.contexts.values();
        let first = contexts.next().cloned();
        self.conflict = first
            .as_ref()
            .is_some_and(|first| contexts.any(|other| other.period_ns != first.period_ns));
        self.selected = if self.conflict { None } else { first };
        self.epoch = self.epoch.wrapping_add(1).max(1);
        self.conflict && !was_conflicted
    }
}

struct WorkerState {
    waiting: AtomicBool,
    #[cfg(target_os = "macos")]
    ack_epoch: AtomicU64,
    #[cfg(target_os = "macos")]
    arm_error: AtomicI32,
}
struct WorkerRegistration {
    thread: std::thread::Thread,
    state: Arc<WorkerState>,
}
#[cfg(test)]
struct EmptyScanHook {
    reached: Sender<()>,
    release: Receiver<()>,
}

struct Executor {
    jobs: Mutex<Queue>,
    leases: Mutex<IdleLeases>,
    lease_epoch: AtomicU64,
    wake_generation: AtomicU64,
    next_lease: AtomicU64,
    workers: OnceLock<Vec<WorkerRegistration>>,
    idle_stats: IdleCounters,
    #[cfg(test)]
    stop: AtomicBool,
    #[cfg(test)]
    joins: Mutex<Vec<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    empty_scan_hook: Mutex<Option<EmptyScanHook>>,
    #[cfg(test)]
    inline_scan_hook: Mutex<Option<EmptyScanHook>>,
}

#[derive(Clone)]
pub(crate) struct IdleLeaseHandle {
    id: u64,
    executor: Arc<Executor>,
    revoked: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleLeaseState {
    pub active: bool,
    pub conflict: bool,
    pub epoch: u64,
    pub period_ns: Option<u64>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdleArmOutcome {
    Armed {
        workers: usize,
    },
    Conflict,
    Unavailable,
    Failed {
        armed: usize,
        total: usize,
        error: i32,
    },
    TimedOut {
        armed: usize,
        total: usize,
    },
    Changed,
    Revoked,
}

impl IdleLeaseHandle {
    pub(crate) fn set_schedule(&self, schedule: &crate::realtime::AudioSchedule) {
        let period_ns = schedule.period_ns();
        let mut leases = self
            .executor
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.revoked.load(Ordering::Acquire) {
            return;
        }
        if leases
            .contexts
            .get(&self.id)
            .is_some_and(|old| old.period_ns == period_ns)
        {
            return;
        }
        leases.contexts.insert(
            self.id,
            Arc::new(IdleContext {
                period_ns,
                #[cfg(target_os = "macos")]
                schedule: schedule.mach_only(),
            }),
        );
        if leases.recompute() {
            self.executor
                .idle_stats
                .conflicts
                .fetch_add(1, Ordering::Relaxed);
        }
        self.executor
            .lease_epoch
            .store(leases.epoch, Ordering::Release);
        drop(leases);
        self.executor.unpark_all();
    }

    fn revoke(&self) {
        let mut leases = self
            .executor
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.revoked.store(true, Ordering::Release);
        if leases.contexts.remove(&self.id).is_none() {
            return;
        }
        if leases.recompute() {
            self.executor
                .idle_stats
                .conflicts
                .fetch_add(1, Ordering::Relaxed);
        }
        self.executor
            .lease_epoch
            .store(leases.epoch, Ordering::Release);
        drop(leases);
        self.executor.unpark_all();
    }

    pub(crate) fn stats(&self) -> IdleStats {
        self.executor.idle_stats.snapshot()
    }

    pub(crate) fn state(&self) -> IdleLeaseState {
        let leases = self
            .executor
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let period_ns = if self.revoked.load(Ordering::Acquire) {
            None
        } else {
            leases
                .contexts
                .get(&self.id)
                .map(|context| context.period_ns)
        };
        IdleLeaseState {
            active: period_ns.is_some(),
            conflict: period_ns.is_some() && leases.conflict,
            epoch: leases.epoch,
            period_ns,
        }
    }

    /// Waits only on the constructing/calling thread, before the playback is
    /// exposed. A queued Play dispatch never waits for worker acknowledgement.
    pub(crate) fn arm(&self, timeout: Duration) -> IdleArmOutcome {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = timeout;
            return if self.revoked.load(Ordering::Acquire) {
                IdleArmOutcome::Revoked
            } else {
                IdleArmOutcome::Unavailable
            };
        }
        #[cfg(target_os = "macos")]
        {
            let (epoch, total) = {
                let leases = self
                    .executor
                    .leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if self.revoked.load(Ordering::Acquire) || !leases.contexts.contains_key(&self.id) {
                    return IdleArmOutcome::Revoked;
                }
                if leases.conflict {
                    return IdleArmOutcome::Conflict;
                }
                if leases.selected.is_none() {
                    return IdleArmOutcome::Unavailable;
                }
                (
                    leases.epoch,
                    self.executor.workers.get().map_or(0, Vec::len),
                )
            };
            let deadline = Instant::now() + timeout;
            loop {
                if self.executor.lease_epoch.load(Ordering::Acquire) != epoch {
                    return IdleArmOutcome::Changed;
                }
                let mut armed = 0;
                let mut error = 0;
                if let Some(workers) = self.executor.workers.get() {
                    for worker in workers {
                        if worker.state.ack_epoch.load(Ordering::Acquire) == epoch {
                            let result = worker.state.arm_error.load(Ordering::Relaxed);
                            if result == 0 {
                                armed += 1
                            } else {
                                error = result
                            }
                        }
                    }
                }
                if error != 0 {
                    self.executor
                        .idle_stats
                        .arm_failures
                        .fetch_add(1, Ordering::Relaxed);
                    return IdleArmOutcome::Failed {
                        armed,
                        total,
                        error,
                    };
                }
                if armed == total && total != 0 {
                    return IdleArmOutcome::Armed { workers: armed };
                }
                if Instant::now() >= deadline {
                    self.executor
                        .idle_stats
                        .arm_timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    return IdleArmOutcome::TimedOut { armed, total };
                }
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    }
}
impl Executor {
    fn shared() -> &'static Arc<Self> {
        crate::profile::init_worker_events();
        static POOL: OnceLock<Arc<Executor>> = OnceLock::new();
        POOL.get_or_init(|| {
            let available = std::thread::available_parallelism().map_or(1, usize::from);
            #[cfg(target_os = "macos")]
            let performance = performance_cores();
            #[cfg(not(target_os = "macos"))]
            let performance = None;
            let count = worker_count(available, performance);
            if crate::profile::enabled() {
                crate::profile::worker_event(format!(
                    "pm-audio-prepare-pool workers={count} available={available} performance_cores={performance:?}"
                ));
            }
            Self::spawn(count)
        })
    }
    fn spawn(count: usize) -> Arc<Self> {
        let pool = Arc::new(Self {
            jobs: Mutex::new(Queue {
                jobs: Vec::new(),
                background: 0,
            }),
            leases: Mutex::new(IdleLeases::default()),
            lease_epoch: AtomicU64::new(0),
            wake_generation: AtomicU64::new(0),
            next_lease: AtomicU64::new(1),
            workers: OnceLock::new(),
            idle_stats: IdleCounters::default(),
            #[cfg(test)]
            stop: AtomicBool::new(false),
            #[cfg(test)]
            joins: Mutex::new(Vec::with_capacity(count)),
            #[cfg(test)]
            empty_scan_hook: Mutex::new(None),
            #[cfg(test)]
            inline_scan_hook: Mutex::new(None),
        });
        #[cfg(test)]
        let mut handles = Vec::with_capacity(count);
        let mut workers = Vec::with_capacity(count);
        for i in 0..count {
            let p = pool.clone();
            let state = Arc::new(WorkerState {
                waiting: AtomicBool::new(false),
                #[cfg(target_os = "macos")]
                ack_epoch: AtomicU64::new(0),
                #[cfg(target_os = "macos")]
                arm_error: AtomicI32::new(0),
            });
            let worker_state = state.clone();
            let handle = std::thread::Builder::new()
                .name(format!("pm-audio-prepare-{i}"))
                .spawn(move || {
                    // Darwin's thread-specific priority leaves the renderer and
                    // real-time callback unchanged. Linux uses the calling TID.
                    //
                    // The initial QoS covers startup and fallback idle waits.
                    // Once an exact-period lease is armed, the worker parks
                    // under its private Mach time constraint and drops that
                    // guard immediately on waking. The claimed job enters
                    // its own playback's actual audio schedule.
                    #[cfg(target_os = "macos")]
                    set_qos(QOS_USER_INTERACTIVE, 0);
                    #[cfg(target_os = "linux")]
                    unsafe {
                        libc::setpriority(libc::PRIO_PROCESS, 0, 15);
                    }
                    #[cfg(target_os = "macos")]
                    let mut parked_schedule: Option<(
                        u64,
                        crate::realtime::AudioSchedule,
                    )> = None;
                    let mut wake_profile: Option<(Instant, u64)> = None;
                    // At most one native DSP step is retained locally; the
                    // ordinary queue is rechecked before every continuation.
                    let mut local_job: Option<Job> = None;
                    loop {
                        #[cfg(test)]
                        if p.stop.load(Ordering::Acquire) {
                            break;
                        }
                        // Read before the empty queue scan. Any mutation
                        // after that scan changes this value even if it
                        // precedes publication of `waiting`.
                        let observed_wake = p.wake_generation.load(Ordering::SeqCst);
                        let lock_at = crate::profile::enabled().then(Instant::now);
                        let mut queue = p.jobs.lock().unwrap_or_else(|e| e.into_inner());
                        let lock_wait_ns = lock_at.map_or(0, elapsed_ns);
                        let mut selected = None;
                        if let Some(job) = local_job.take() {
                            if queue.may_inline(&job, false) {
                                selected = Some((job, false, true));
                            } else {
                                // A newly arrived equal P0 precedes the
                                // retained voice. Put it at the FIFO tail.
                                queue.jobs.push(job);
                                p.idle_stats.job_requeues.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        if selected.is_none()
                            && let Some((at, priority)) = queue.ready_index()
                        {
                            let background = priority != 0;
                            let job = queue.jobs.remove(at);
                            if background {
                                queue.background += 1;
                            }
                            selected = Some((job, background, false));
                        }
                        if let Some((mut job, background, inline)) = selected {
                            let claimed_at = crate::profile::enabled().then(|| {
                                let claimed = Instant::now();
                                let (wake_claim_us, wake_drop_us) = if inline {
                                    (0, 0)
                                } else {
                                    wake_profile.take().map_or((0, 0), |(woken, drop_ns)| {
                                        (
                                            claimed
                                                .duration_since(woken)
                                                .as_micros()
                                                .min(u64::MAX as u128)
                                                as u64,
                                            drop_ns / 1_000,
                                        )
                                    })
                                };
                                let wake_claim_ns = wake_claim_us.saturating_mul(1_000);
                                p.idle_stats
                                    .wake_to_claim_ns
                                    .fetch_max(wake_claim_ns, Ordering::Relaxed);
                                ClaimStamp {
                                    host_us: crate::clock::host_now_us(),
                                    at: claimed,
                                    worker: i,
                                    inline,
                                    wake_drop_us,
                                    queue_lock_us: lock_wait_ns / 1_000,
                                    wake_claim_us,
                                }
                            });
                            p.idle_stats
                                .queue_lock_wait_ns
                                .fetch_max(lock_wait_ns, Ordering::Relaxed);
                            drop(queue);
                            #[cfg(test)]
                            if inline
                                && let Some(hook) = p
                                    .inline_scan_hook
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .take()
                            {
                                let _ = hook.reached.send(());
                                let _ = hook.release.recv_timeout(Duration::from_secs(5));
                            }
                            p.idle_stats.job_claims.fetch_add(1, Ordering::Relaxed);
                            // Playhead preparation is latency-sensitive even at
                            // edit time, before Play arrives. Boundary refill
                            // keeps its single-job limit; its closure enters
                            // real-time CPU scheduling and restores the worker's
                            // effective Mach priority before it waits again.
                            let outcome = (job.step)(claimed_at);
                            // The closure released its real-time guard before
                            // returning PendingCpu. Keep a sole ready P0 on
                            // this worker for one more slice; otherwise put
                            // it at the FIFO tail under the ordinary lock.
                            // The thread's physical CPU debt is retained.
                            let mut queue = p.jobs.lock().unwrap_or_else(|e| e.into_inner());
                            if background {
                                queue.background -= 1;
                            }
                            let keep_local = outcome == JobOutcome::PendingCpu
                                && queue.may_inline(&job, background);
                            if keep_local {
                                local_job = Some(job);
                            } else if outcome == JobOutcome::PendingCpu {
                                queue.jobs.push(job);
                            }
                            // A finished representative may have made its
                            // dependent voices ready. Inspect that published
                            // state before waking parked workers; an empty
                            // queue needs no all-worker wake storm.
                            let has_ready = queue.ready_index().is_some();
                            drop(queue);
                            if outcome == JobOutcome::PendingCpu {
                                if !keep_local {
                                    p.idle_stats.job_requeues.fetch_add(1, Ordering::Relaxed);
                                }
                                if has_ready {
                                    p.unpark_one();
                                }
                            } else {
                                p.idle_stats.job_completions.fetch_add(1, Ordering::Relaxed);
                                if has_ready {
                                    // A terminal foreground job may have
                                    // published a shared head, releasing
                                    // several gated dependents at once.
                                    p.unpark_all();
                                }
                            }
                            continue;
                        }
                        drop(queue);
                        #[cfg(test)]
                        if let Some(hook) = p
                            .empty_scan_hook
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .take()
                        {
                            let _ = hook.reached.send(());
                            let _ = hook.release.recv_timeout(Duration::from_secs(5));
                        }
                        let (context, epoch) = {
                            let leases = p.leases.lock().unwrap_or_else(|e| e.into_inner());
                            (leases.selected.clone(), leases.epoch)
                        };
                        #[cfg(target_os = "macos")]
                        let attempted = context.is_some();
                        #[cfg(target_os = "macos")]
                        if let Some(context) = context {
                            if parked_schedule
                                .as_ref()
                                .is_none_or(|(period, _)| *period != context.period_ns)
                            {
                                // A private Mach-only context gives each worker
                                // disjoint metrics and a stable per-thread CPU
                                // ledger across repeated park/wake cycles.
                                parked_schedule =
                                    Some((context.period_ns, context.schedule.mach_only()));
                            }
                            let schedule = &parked_schedule.as_ref().expect("idle policy").1;
                            let metrics_before = schedule.metrics();
                            let guard = schedule.enter();
                            let status = guard.status();
                            let metrics_entered = schedule.metrics();
                            // Publish entry/readback errors before the arm
                            // acknowledgement can be observed by startup.
                            p.idle_stats.record_metrics(metrics_before, metrics_entered);
                            let error = if status.method.label() == "mach-time-constraint" {
                                0
                            } else {
                                status.mach_error.unwrap_or(libc::EIO)
                            };
                            worker_state.arm_error.store(error, Ordering::Relaxed);
                            // This acknowledges a successful entry for the
                            // entire lease epoch. Normal job wakes must not
                            // erase it and turn a caller-side arm barrier
                            // into a false timeout under queue churn.
                            worker_state.ack_epoch.store(epoch, Ordering::Release);
                            if error == 0 {
                                p.idle_stats
                                    .successful_idle_entries
                                    .fetch_add(1, Ordering::Relaxed);
                                // No queue mutex or user closure runs with this
                                // idle constraint. The generation recheck
                                // closes the scan-to-park handoff.
                                worker_state.waiting.store(true, Ordering::SeqCst);
                                if p.wake_generation.load(Ordering::SeqCst) != observed_wake {
                                    worker_state.waiting.store(false, Ordering::SeqCst);
                                    drop(guard);
                                    p.idle_stats
                                        .record_metrics(metrics_entered, schedule.metrics());
                                    wake_profile = None;
                                    continue;
                                }
                                std::thread::park();
                                worker_state.waiting.store(false, Ordering::SeqCst);
                                let woken = crate::profile::enabled().then(Instant::now);
                                drop(guard);
                                p.idle_stats
                                    .record_metrics(metrics_entered, schedule.metrics());
                                p.idle_stats.wake_count.fetch_add(1, Ordering::Relaxed);
                                wake_profile = woken.map(|at| {
                                    let drop_ns = elapsed_ns(at);
                                    p.idle_stats
                                        .wake_guard_drop_ns
                                        .fetch_max(drop_ns, Ordering::Relaxed);
                                    (at, drop_ns)
                                });
                                continue;
                            }
                            drop(guard);
                            p.idle_stats
                                .record_metrics(metrics_entered, schedule.metrics());
                        }
                        #[cfg(not(target_os = "macos"))]
                        let _ = (context, epoch);
                        #[cfg(target_os = "macos")]
                        if !attempted {
                            worker_state.ack_epoch.store(0, Ordering::Release);
                        }
                        worker_state.waiting.store(true, Ordering::SeqCst);
                        if p.wake_generation.load(Ordering::SeqCst) != observed_wake {
                            worker_state.waiting.store(false, Ordering::SeqCst);
                            wake_profile = None;
                            continue;
                        }
                        wake_profile = None;
                        std::thread::park();
                        worker_state.waiting.store(false, Ordering::SeqCst);
                    }
                })
                .expect("audio preparation worker");
            workers.push(WorkerRegistration {
                thread: handle.thread().clone(),
                state,
            });
            #[cfg(test)]
            handles.push(handle);
        }
        pool.workers
            .set(workers)
            .unwrap_or_else(|_| panic!("workers registered twice"));
        #[cfg(test)]
        pool.joins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(handles);
        pool
    }
    #[cfg(test)]
    fn stop_and_join(&self) {
        self.stop.store(true, Ordering::Release);
        self.unpark_all();
        let handles = std::mem::take(&mut *self.joins.lock().unwrap_or_else(|e| e.into_inner()));
        for handle in handles {
            let _ = handle.join();
        }
    }
    fn unpark_all(&self) {
        // The generation closes the empty-scan/publication race. A worker
        // that was busy or not yet waiting will observe the change before
        // parking, so it needs no kernel wake or accumulated park token.
        // SeqCst pairs the generation and waiting flag across two atomics:
        // either the publisher sees waiting=true or the worker sees the new
        // generation. Acquire/Release alone permits both loads to be stale.
        self.wake_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(workers) = self.workers.get() {
            for worker in workers {
                if worker.state.waiting.load(Ordering::SeqCst) {
                    worker.thread.unpark();
                }
            }
        }
    }
    fn unpark_one(&self) {
        // As with unpark_all, publish the queue change before inspecting
        // waiting flags. A worker between its empty scan and park sees the
        // new generation; one already parked receives a token.
        self.wake_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(workers) = self.workers.get()
            && let Some(worker) = workers
                .iter()
                .find(|worker| worker.state.waiting.load(Ordering::SeqCst))
        {
            worker.thread.unpark();
        }
    }
    fn lease(self: &Arc<Self>) -> IdleLeaseHandle {
        IdleLeaseHandle {
            id: self.next_lease.fetch_add(1, Ordering::Relaxed),
            executor: self.clone(),
            revoked: Arc::new(AtomicBool::new(false)),
        }
    }
    #[cfg(test)]
    fn submit(
        &self,
        priority: usize,
        ready: impl Fn() -> bool + Send + 'static,
        job: impl FnOnce() + Send + 'static,
    ) {
        let mut job = Some(job);
        self.submit_batch(vec![Job {
            priority,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(ready),
            step: Box::new(move |_| {
                job.take().expect("one-shot test job")();
                JobOutcome::Terminal
            }),
        }]);
    }
    fn submit_batch(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let mut queue = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        queue.jobs.extend(jobs);
        drop(queue);
        self.unpark_all();
    }
}
struct Done {
    generation: u64,
    start: i64,
    voice: usize,
    // Pin the allocation until acceptance, preventing pointer reuse after
    // a start is evicted and recreated while its result is in transit.
    history: Arc<Mutex<crate::stretch::StretchRun>>,
    ready: crate::Result<bool>,
    /// The history was claimed elsewhere or retired by the producer while
    /// this task was yielded. Clear pending without updating the mix slot.
    claimed: bool,
    profile: crate::profile::Profile,
}
#[derive(Default)]
pub(crate) struct SchedulingCounters {
    pub(crate) workgroup: AtomicU64,
    pub(crate) mach: AtomicU64,
    pub(crate) failed: AtomicU64,
    pub(crate) workgroup_error: AtomicI32,
    pub(crate) mach_error: AtomicI32,
    pub(crate) hint_error: AtomicI32,
    #[cfg(test)]
    pub(crate) jobs_started: AtomicU64,
    #[cfg(test)]
    pub(crate) slice_entries: AtomicU64,
}
impl SchedulingCounters {
    fn record(&self, status: crate::realtime::ScheduleStatus) {
        if let Some(e) = status.workgroup_error {
            self.workgroup_error.store(e, Ordering::Relaxed);
        }
        if let Some(e) = status.mach_error {
            self.mach_error.store(e, Ordering::Relaxed);
        }
        if let Some(e) = status.hint_error {
            self.hint_error.store(e, Ordering::Relaxed);
        }
        let counter = match status.method.label() {
            "device-workgroup" | "custom-audio-interval" => &self.workgroup,
            "mach-time-constraint" => &self.mach,
            _ => &self.failed,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) struct Preparation {
    generation: Arc<AtomicU64>,
    schedule: crate::realtime::AudioSchedule,
    idle: IdleLeaseHandle,
    pending_controls: Option<Arc<AtomicU64>>,
    pub(crate) scheduling: Arc<SchedulingCounters>,
    make: Evaluators,
    static_content: bool,
    pending: HashSet<(i64, usize)>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}
impl Preparation {
    pub(crate) fn new() -> Self {
        let (tx, rx) = channel();
        // The ordinary diagnostic writer must exist before a worker can
        // enqueue a profile event, and never inherit its audio policy.
        crate::profile::init_worker_events();
        // Start the threads before a project or Play arrives.
        let idle = Executor::shared().lease();
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            schedule: crate::realtime::AudioSchedule::new(48_000, 512, None),
            idle,
            pending_controls: None,
            scheduling: Arc::new(SchedulingCounters::default()),
            make: Arc::new(Evaluator::new),
            static_content: true,
            pending: HashSet::new(),
            tx,
            rx,
        }
    }
    pub(crate) fn set_schedule(&mut self, schedule: crate::realtime::AudioSchedule) {
        self.idle.set_schedule(&schedule);
        self.schedule = schedule;
    }
    pub(crate) fn idle_handle(&self) -> IdleLeaseHandle {
        self.idle.clone()
    }
    pub(crate) fn arm_idle_workers(&self, timeout: Duration) -> IdleArmOutcome {
        self.idle.arm(timeout)
    }
    pub(crate) fn set_pending_controls(&mut self, pending: Arc<AtomicU64>) {
        self.pending_controls = Some(pending);
    }
    pub(crate) fn is_standard(&self) -> bool {
        self.static_content
    }
    pub(crate) fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.pending.clear();
        Executor::shared().unpark_all();
    }
    pub(crate) fn evaluators(&mut self, make: Evaluators, static_content: bool) {
        self.invalidate();
        self.make = make;
        self.static_content = static_content;
    }
    pub(crate) fn schedule(&mut self, mixer: &mut Mixer, starts: &[i64], urgent: bool) {
        self.schedule_at(mixer, starts, urgent, None);
    }
    pub(crate) fn schedule_at(
        &mut self,
        mixer: &mut Mixer,
        starts: &[i64],
        urgent: bool,
        target: Option<i64>,
    ) {
        self.poll(mixer);
        let mut batch: Vec<Job> = Vec::new();
        for (priority, &start) in starts.iter().enumerate() {
            let pending = &self.pending;
            let jobs = mixer.preparation_work_except(start, target.unwrap_or(start), |vi| {
                pending.contains(&(start, vi))
            });
            for work in jobs {
                if !self.pending.insert((start, work.vi)) {
                    continue;
                }
                let voice = work.vi;
                let history = work.history();
                let generation = self.generation.load(Ordering::Relaxed);
                let current = self.generation.clone();
                let make = self.make.clone();
                let static_content = self.static_content;
                let tx = self.tx.clone();
                let gate = work.gate.clone();
                let schedule = self.schedule.clone();
                let pending_controls = self.pending_controls.clone();
                let scheduling = self.scheduling.clone();
                let ready_generation = current.clone();
                let critical = work.critical.clone();
                let mut trace = JobTrace::new();
                let mut work = Some(work);
                let mut task: Option<crate::mixer::PrepareTask> = None;
                batch.push(Job {
                    priority: priority + usize::from(!urgent),
                    critical,
                    ready: Box::new(move || {
                        ready_generation.load(Ordering::Relaxed) != generation
                            || gate.as_ref().is_none_or(|g| g.load(Ordering::Acquire))
                    }),
                    step: Box::new(move |claimed_at: Option<ClaimStamp>| {
                        trace.claim(claimed_at);
                        let cancelled = || current.load(Ordering::Relaxed) != generation;
                        if cancelled() {
                            // A continuation has already claimed its history;
                            // dropping the owned task finalizes its flags and
                            // releases a dependent head even though this
                            // generation's Done would be discarded.
                            drop(task.take());
                            trace.log(generation, start, voice, "cancelled");
                            return JobOutcome::Terminal;
                        }
                        if task.is_none() && !work.as_ref().expect("first claim").claim() {
                            let _ = tx.send(Done {
                                generation,
                                start,
                                voice,
                                history: history.clone(),
                                ready: Ok(false),
                                claimed: true,
                                profile: Default::default(),
                            });
                            trace.log(generation, start, voice, "claimed-elsewhere");
                            return JobOutcome::Terminal;
                        }
                        // This flag belongs only to the preparation worker.
                        // The producer must never yield to its own controls.
                        let control_before = trace
                            .queued_at
                            .map(|_| crate::realtime::thread_control_yield_stats());
                        let _control_gate = pending_controls.as_ref().map(|pending| {
                            crate::realtime::worker_control_gate(pending.clone(), &schedule)
                        });
                        let enter_at = trace.queued_at.map(|_| Instant::now());
                        let throttle_before = trace
                            .queued_at
                            .map(|_| crate::realtime::thread_throttle_sleep_ns());
                        let realtime = schedule.enter();
                        #[cfg(test)]
                        scheduling.slice_entries.fetch_add(1, Ordering::Relaxed);
                        let enter_us = enter_at.map(elapsed_us).unwrap_or(0);
                        scheduling.record(realtime.status());
                        crate::profile::reset();
                        crate::realtime::checkpoint_phase("prepare-job");
                        let work_at = trace.queued_at.map(|_| Instant::now());
                        let cpu_before = trace.queued_at.map(|_| crate::realtime::thread_cpu_ns());
                        if task.is_none() {
                            task = Some(
                                work.take()
                                    .expect("claimed voice")
                                    .into_task(make(), static_content),
                            );
                            #[cfg(test)]
                            scheduling.jobs_started.fetch_add(1, Ordering::Relaxed);
                        }
                        let outcome = task.as_mut().expect("owned voice").step(cancelled);
                        let work_us = work_at.map(elapsed_us).unwrap_or(0);
                        let cpu_us = cpu_before.map_or(0, |start| {
                            crate::realtime::thread_cpu_ns().saturating_sub(start) / 1_000
                        });
                        let throttle_us = throttle_before.map_or(0, |start| {
                            crate::realtime::thread_throttle_sleep_ns().saturating_sub(start)
                                / 1_000
                        });
                        // No queue lock, Done send, or logging while joined to
                        // the playback's workgroup or Mach time constraint.
                        drop(realtime);
                        let profile = crate::profile::snapshot();
                        trace.slice(
                            enter_us,
                            work_us,
                            cpu_us,
                            throttle_us,
                            control_yield_delta(control_before),
                            profile,
                        );
                        match outcome {
                            Ok(crate::mixer::PrepareStep::Yielded) if !cancelled() => {
                                trace.cpu_yields += 1;
                                JobOutcome::PendingCpu
                            }
                            Ok(crate::mixer::PrepareStep::Yielded) => {
                                drop(task.take());
                                trace.log(generation, start, voice, "cancelled");
                                JobOutcome::Terminal
                            }
                            Ok(crate::mixer::PrepareStep::Retired) => {
                                drop(task.take());
                                let _ = tx.send(Done {
                                    generation,
                                    start,
                                    voice,
                                    history: history.clone(),
                                    ready: Ok(false),
                                    claimed: true,
                                    profile: trace.profile,
                                });
                                trace.log(generation, start, voice, "retired");
                                JobOutcome::Terminal
                            }
                            Ok(crate::mixer::PrepareStep::Complete(ready)) => {
                                drop(task.take());
                                let _ = tx.send(Done {
                                    generation,
                                    start,
                                    voice,
                                    history: history.clone(),
                                    ready: Ok(ready),
                                    claimed: false,
                                    profile: trace.profile,
                                });
                                trace.log(
                                    generation,
                                    start,
                                    voice,
                                    if ready { "ready" } else { "missing-input" },
                                );
                                JobOutcome::Terminal
                            }
                            Err(error) => {
                                drop(task.take());
                                if !cancelled() {
                                    let _ = tx.send(Done {
                                        generation,
                                        start,
                                        voice,
                                        history: history.clone(),
                                        ready: Err(error),
                                        claimed: false,
                                        profile: trace.profile,
                                    });
                                }
                                trace.log(generation, start, voice, "error-or-cancelled");
                                JobOutcome::Terminal
                            }
                        }
                    }),
                });
            }
        }
        // A worker can preempt the caller as soon as it wakes, so expose the
        // entire voice set under one queue lock before signaling the pool.
        Executor::shared().submit_batch(batch);
    }
    fn accept(&mut self, mixer: &mut Mixer, done: Done) {
        if done.generation == self.generation.load(Ordering::Relaxed) {
            self.pending.remove(&(done.start, done.voice));
            if done.claimed {
                return;
            }
            if !mixer.preparation_matches(
                done.start,
                done.voice,
                Arc::as_ptr(&done.history) as usize,
            ) {
                return;
            }
            match done.ready {
                Ok(ready) => mixer.prepared_voice(done.start, done.voice, ready),
                Err(_) => mixer.preparation_failed(done.start, done.voice),
            }
            crate::profile::add(done.profile);
        }
    }
    pub(crate) fn poll(&mut self, mixer: &mut Mixer) {
        while let Ok(done) = self.rx.try_recv() {
            self.accept(mixer, done);
        }
    }
    pub(crate) fn pending_start(&self, start: i64) -> bool {
        self.pending.iter().any(|(at, _)| *at == start)
    }
    pub(crate) fn wait(&mut self, mixer: &mut Mixer, timeout: Duration) {
        if let Ok(done) = self.rx.recv_timeout(timeout) {
            self.accept(mixer, done);
        }
        self.poll(mixer);
    }
}
#[cfg(test)]
impl Preparation {
    /// Jobs submitted and not yet accepted.
    pub(crate) fn pending(&self) -> &HashSet<(i64, usize)> {
        &self.pending
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        self.invalidate();
        self.idle.revoke();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::thread::JoinHandleExt;

    struct PrivatePool(Arc<Executor>);
    impl Drop for PrivatePool {
        fn drop(&mut self) {
            self.0.stop_and_join();
        }
    }

    fn wait_for_policy(
        thread: libc::pthread_t,
        schedule: &crate::realtime::AudioSchedule,
        realtime: bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let priority = crate::realtime::thread_priority(thread);
            let actual = if realtime {
                crate::realtime::thread_has_constraint(thread, schedule)
            } else {
                crate::realtime::thread_is_timeshare(thread)
                    && priority.is_some_and(|base| (47..80).contains(&base))
            };
            if actual {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worker policy: realtime={realtime}, priority={priority:?}, timeshare={}",
                crate::realtime::thread_is_timeshare(thread)
            );
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    #[test]
    fn armed_idle_workers_leave_mach_policy_before_queue_readiness_and_job() {
        let private = PrivatePool(Executor::spawn(1));
        let pool = &private.0;
        let thread = pool.joins.lock().unwrap()[0].as_pthread_t();
        assert_ne!(thread, 0, "owned worker has a native pthread");
        let schedule = crate::realtime::AudioSchedule::new(48_000, 512, None);
        let idle = pool.lease();
        idle.set_schedule(&schedule);
        let armed = idle.arm(Duration::from_secs(2));
        assert!(matches!(armed, IdleArmOutcome::Armed { .. }), "{armed:?}");
        assert!(idle.state().active);
        assert!(!idle.state().conflict);
        wait_for_policy(thread, &schedule, true);
        let before = idle.stats();
        let (done_tx, done_rx) = channel();
        pool.submit(
            0,
            || {
                assert!(
                    !crate::realtime::has_active_guard(),
                    "ready closure is ordinary"
                );
                assert!(crate::realtime::thread_is_timeshare(unsafe {
                    libc::pthread_self()
                }));
                true
            },
            move || {
                assert!(
                    !crate::realtime::has_active_guard(),
                    "job claim follows idle guard drop"
                );
                assert!(crate::realtime::thread_is_timeshare(unsafe {
                    libc::pthread_self()
                }));
                done_tx
                    .send(crate::realtime::current_thread_priority())
                    .unwrap();
            },
        );
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_some_and(|priority| priority >= 47)
        );
        assert!(idle.stats().wake_count > before.wake_count);
        let armed_again = idle.arm(Duration::from_secs(2));
        assert!(
            matches!(armed_again, IdleArmOutcome::Armed { .. }),
            "{armed_again:?}"
        );
        wait_for_policy(thread, &schedule, true);
        let other = pool.lease();
        other.set_schedule(&crate::realtime::AudioSchedule::new(48_000, 256, None));
        assert_eq!(
            idle.arm(Duration::from_millis(10)),
            IdleArmOutcome::Conflict
        );
        assert!(idle.state().conflict);
        wait_for_policy(thread, &schedule, false);
        other.revoke();
        let rearmed = idle.arm(Duration::from_secs(2));
        assert!(
            matches!(rearmed, IdleArmOutcome::Armed { .. }),
            "{rearmed:?}"
        );
        wait_for_policy(thread, &schedule, true);
        idle.revoke();
        wait_for_policy(thread, &schedule, false);
        assert!(!idle.state().active);
        idle.set_schedule(&schedule);
        assert!(
            !idle.state().active,
            "stale lease cannot rearm after revocation"
        );
        assert_eq!(idle.arm(Duration::from_millis(10)), IdleArmOutcome::Revoked);
    }

    /// Mach scheduling permanently opts these owned audio threads out of
    /// pthread QoS. Their *kernel* idle priority must still stay interactive.
    #[test]
    fn idle_workers_restore_interactive_kernel_priority() {
        for priority in [0, 1, 0] {
            let (tx, rx) = channel();
            let (release, wait) = channel();
            Executor::shared().submit(
                priority,
                || true,
                move || {
                    let before = crate::realtime::current_thread_priority();
                    let schedule = crate::realtime::AudioSchedule::headless(48_000, 512);
                    {
                        let guard = schedule.enter();
                        assert_eq!(guard.method().label(), "custom-audio-interval");
                    }
                    let after = crate::realtime::current_thread_priority();
                    let failures = schedule.metrics().restore_failures;
                    let _ = tx.send((
                        unsafe { libc::pthread_self() } as usize,
                        before,
                        after,
                        failures,
                    ));
                    // A leased idle worker cannot be mistaken for a thread
                    // another concurrent test has already put back into DSP.
                    let _ = wait.recv_timeout(Duration::from_secs(30));
                },
            );
            let (worker, before, after, failures) =
                rx.recv_timeout(Duration::from_secs(30)).unwrap();
            let target = Some(
                before
                    .expect("kernel priority before real-time work")
                    .max(47),
            );
            assert_eq!(after, target, "owned audio thread idle kernel priority");
            assert_eq!(failures, 0);
            for _ in 0..20 {
                assert_eq!(
                    crate::realtime::thread_priority(worker as libc::pthread_t),
                    target
                );
            }
            let _ = release.send(());
        }
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn local_pool() -> Arc<Executor> {
        Arc::new(Executor {
            jobs: Mutex::new(Queue {
                jobs: Vec::new(),
                background: 0,
            }),
            leases: Mutex::new(IdleLeases::default()),
            lease_epoch: AtomicU64::new(0),
            wake_generation: AtomicU64::new(0),
            next_lease: AtomicU64::new(1),
            workers: OnceLock::new(),
            idle_stats: IdleCounters::default(),
            stop: AtomicBool::new(false),
            joins: Mutex::new(Vec::new()),
            empty_scan_hook: Mutex::new(None),
            inline_scan_hook: Mutex::new(None),
        })
    }

    #[test]
    fn idle_leases_revoke_and_recover_from_period_conflict() {
        let first = crate::realtime::AudioSchedule::new(48_000, 512, None);
        let second = crate::realtime::AudioSchedule::new(48_000, 256, None);
        let pool = local_pool();
        let one = pool.lease();
        let two = pool.lease();
        one.set_schedule(&first);
        assert!(one.state().active);
        let first_epoch = one.state().epoch;
        two.set_schedule(&second);
        // period_ns() is zero off macOS, where the idle Mach policy is
        // unavailable. Force distinct test metadata to exercise the lease
        // state machine portably.
        #[cfg(not(target_os = "macos"))]
        {
            let mut leases = pool.leases.lock().unwrap();
            leases
                .contexts
                .insert(two.id, Arc::new(IdleContext { period_ns: 1 }));
            leases.recompute();
            pool.lease_epoch.store(leases.epoch, Ordering::Release);
        }
        assert!(one.state().conflict);
        assert_ne!(one.state().epoch, first_epoch);
        let stale = two.clone();
        two.revoke();
        assert!(!one.state().conflict);
        assert!(!two.state().active);
        stale.set_schedule(&first);
        assert!(
            !stale.state().active,
            "a clone cannot resurrect a revoked lease"
        );
        one.revoke();
        assert!(!one.state().active);
        assert!(pool.leases.lock().unwrap().selected.is_none());
    }

    #[test]
    fn queued_job_between_empty_scan_and_wait_publication_is_not_lost() {
        // Pause the *production worker loop* immediately after its real
        // empty queue scan. Publishing a job before it marks itself waiting
        // must be caught by the wake generation recheck.
        let pool = Executor::spawn(1);
        let (scanned_tx, scanned_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (done_tx, done_rx) = channel();
        *pool.empty_scan_hook.lock().unwrap() = Some(EmptyScanHook {
            reached: scanned_tx,
            release: release_rx,
        });
        pool.unpark_all();
        scanned_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        pool.submit(
            0,
            || true,
            move || {
                done_tx.send(()).unwrap();
            },
        );
        release_tx.send(()).unwrap();
        let completed = done_rx.recv_timeout(Duration::from_secs(1));
        pool.stop_and_join();
        completed.unwrap();
    }

    #[test]
    fn yielded_job_moves_to_fifo_tail_and_completes_once() {
        let pool = Executor::spawn(1);
        let (events_tx, events_rx) = channel();
        let mut slices = 0;
        let first_tx = events_tx.clone();
        let first = Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                slices += 1;
                first_tx.send((0, slices)).unwrap();
                if slices < 3 {
                    JobOutcome::PendingCpu
                } else {
                    JobOutcome::Terminal
                }
            }),
        };
        let second = Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                events_tx.send((1, 1)).unwrap();
                JobOutcome::Terminal
            }),
        };
        pool.submit_batch(vec![first, second]);
        let events: Vec<_> = (0..4)
            .map(|_| events_rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        pool.stop_and_join();
        assert_eq!(events, [(0, 1), (1, 1), (0, 2), (0, 3)]);
        let stats = pool.idle_stats.snapshot();
        assert_eq!(stats.job_claims, 4);
        assert_eq!(stats.job_requeues, 1, "only the contested slice requeues");
        assert_eq!(stats.job_completions, 2);
    }

    #[test]
    fn a_sole_ready_head_continues_on_one_worker_without_requeue() {
        let pool = Executor::spawn(2);
        let (tx, rx) = channel();
        let mut slices = 0;
        pool.submit_batch(vec![Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                slices += 1;
                tx.send((std::thread::current().id(), slices)).unwrap();
                if slices == 5 {
                    JobOutcome::Terminal
                } else {
                    JobOutcome::PendingCpu
                }
            }),
        }]);
        let events: Vec<_> = (0..5)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        pool.stop_and_join();
        assert_eq!(
            events.iter().map(|(_, slice)| *slice).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert!(events.iter().all(|(worker, _)| *worker == events[0].0));
        let stats = pool.idle_stats.snapshot();
        assert_eq!(stats.job_claims, 5);
        assert_eq!(stats.job_requeues, 0);
        assert_eq!(stats.job_completions, 1);
    }

    #[test]
    fn a_new_equal_priority_job_waits_at_most_one_inline_slice() {
        let pool = Executor::spawn(1);
        let (at_inline_tx, at_inline_rx) = channel();
        let (release_tx, release_rx) = channel();
        *pool.inline_scan_hook.lock().unwrap() = Some(EmptyScanHook {
            reached: at_inline_tx,
            release: release_rx,
        });
        let (events_tx, events_rx) = channel();
        let first_tx = events_tx.clone();
        let mut slices = 0;
        pool.submit_batch(vec![Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                slices += 1;
                first_tx.send((0, slices)).unwrap();
                if slices == 3 {
                    JobOutcome::Terminal
                } else {
                    JobOutcome::PendingCpu
                }
            }),
        }]);
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (0, 1)
        );
        at_inline_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        pool.submit_batch(vec![Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                events_tx.send((1, 1)).unwrap();
                JobOutcome::Terminal
            }),
        }]);
        release_tx.send(()).unwrap();
        let rest: Vec<_> = (0..3)
            .map(|_| events_rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        pool.stop_and_join();
        assert_eq!(rest, [(0, 2), (1, 1), (0, 3)]);
        assert_eq!(pool.idle_stats.snapshot().job_requeues, 1);
    }

    #[test]
    fn a_retired_local_continuation_stops_after_its_next_check() {
        let pool = Executor::spawn(1);
        let (at_inline_tx, at_inline_rx) = channel();
        let (release_tx, release_rx) = channel();
        *pool.inline_scan_hook.lock().unwrap() = Some(EmptyScanHook {
            reached: at_inline_tx,
            release: release_rx,
        });
        let retired = Arc::new(AtomicBool::new(false));
        let worker_retired = retired.clone();
        let (tx, rx) = channel();
        pool.submit_batch(vec![Job {
            priority: 0,
            critical: Arc::new(AtomicBool::new(false)),
            ready: Box::new(|| true),
            step: Box::new(move |_| {
                if worker_retired.load(Ordering::Acquire) {
                    tx.send("retired").unwrap();
                    JobOutcome::Terminal
                } else {
                    tx.send("yielded").unwrap();
                    JobOutcome::PendingCpu
                }
            }),
        }]);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "yielded");
        at_inline_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        retired.store(true, Ordering::Release);
        release_tx.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "retired");
        pool.stop_and_join();
        assert_eq!(pool.idle_stats.snapshot().job_completions, 1);
    }

    #[test]
    fn pool_count_keeps_two_total_cores_and_two_waves_when_possible() {
        assert_eq!(worker_count(16, Some(12)), 10);
        assert_eq!(worker_count(16, None), 14);
        assert_eq!(worker_count(16, Some(24)), 14);
        assert_eq!(worker_count(12, Some(8)), 8);
        assert_eq!(worker_count(10, Some(8)), 8);
        assert_eq!(worker_count(10, Some(6)), 8);
        assert_eq!(worker_count(8, Some(4)), 6);
        assert_eq!(worker_count(2, Some(1)), 1);
    }

    #[test]
    fn an_audible_history_bypasses_the_serial_boundary_limit() {
        let critical = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(true));
        let gate = ready.clone();
        let queue = Queue {
            jobs: vec![Job {
                priority: 1,
                critical: critical.clone(),
                ready: Box::new(move || gate.load(Ordering::Acquire)),
                step: Box::new(|_| JobOutcome::Terminal),
            }],
            background: 1,
        };
        assert_eq!(
            queue.ready_index(),
            None,
            "ordinary boundary work stays serial"
        );
        critical.store(true, Ordering::Release);
        assert_eq!(
            queue.ready_index(),
            Some((0, 0)),
            "audible histories become parallel foreground work"
        );
        ready.store(false, Ordering::Release);
        assert_eq!(
            queue.ready_index(),
            None,
            "promotion never bypasses the representative head gate"
        );
    }
}
