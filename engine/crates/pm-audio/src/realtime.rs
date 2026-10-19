//! Scheduling shared by a playback's producer and start-critical preparation.
//!
//! A CoreAudio device owns an audio workgroup. Its property returns a retained
//! object; the schedule keeps it alive until every per-thread join guard exits.
//! Headless playback and devices without that property use a Mach time
//! constraint matched to the callback period. Call `checkpoint` between bounded
//! units of DSP when using the latter: the constraint cannot make an arbitrary
//! uninterrupted job meet the callback deadline.

use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Observed work between cooperative checkpoints, across all threads in one
/// playback schedule. CPU maxima use the calling thread's CPU clock; the
/// constraint count records CPU service beyond the full callback period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RealtimeMetrics {
    pub max_unit_ns: u64,
    pub max_period_cpu_ns: u64,
    pub computation_overruns: u64,
    pub constraint_overruns: u64,
    pub throttle_sleeps: u64,
    pub total_sleep_ns: u64,
    pub restore_failures: u64,
}

#[derive(Default)]
struct MetricCounters {
    max_unit_ns: AtomicU64,
    max_period_cpu_ns: AtomicU64,
    computation_overruns: AtomicU64,
    constraint_overruns: AtomicU64,
    throttle_sleeps: AtomicU64,
    total_sleep_ns: AtomicU64,
    restore_failures: AtomicU64,
}

impl MetricCounters {
    fn snapshot(&self) -> RealtimeMetrics {
        RealtimeMetrics {
            max_unit_ns: self.max_unit_ns.load(Ordering::Relaxed),
            max_period_cpu_ns: self.max_period_cpu_ns.load(Ordering::Relaxed),
            computation_overruns: self.computation_overruns.load(Ordering::Relaxed),
            constraint_overruns: self.constraint_overruns.load(Ordering::Relaxed),
            throttle_sleeps: self.throttle_sleeps.load(Ordering::Relaxed),
            total_sleep_ns: self.total_sleep_ns.load(Ordering::Relaxed),
            restore_failures: self.restore_failures.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleMethod {
    DeviceWorkgroup,
    MachTimeConstraint,
    Unavailable,
}
impl ScheduleMethod {
    pub const fn label(self) -> &'static str {
        match self {
            Self::DeviceWorkgroup => "device-workgroup",
            Self::MachTimeConstraint => "mach-time-constraint",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The actual result of entering the schedule on the calling thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleStatus {
    pub method: ScheduleMethod,
    /// CoreAudio property or workgroup-join error, if there was one.
    pub workgroup_error: Option<i32>,
    /// Mach policy error when neither mechanism succeeded.
    pub mach_error: Option<i32>,
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
    workgroup_error: Option<i32>,
    #[cfg(target_os = "macos")]
    period: u32,
    #[cfg(target_os = "macos")]
    computation: u32,
    #[cfg(target_os = "macos")]
    period_ns: u64,
    #[cfg(target_os = "macos")]
    computation_ns: u64,
    #[cfg(target_os = "macos")]
    id: u64,
}

/// A playback's scheduling context. `device_id = Some(id)` attempts the
/// CoreAudio output-device workgroup; `None` is for headless playback. Supply
/// the real callback size, including the 512-frame headless callback size.
impl AudioSchedule {
    pub fn new(sample_rate: u32, callback_frames: u32, device_id: Option<u32>) -> Self {
        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT_ID: AtomicU64 = AtomicU64::new(1);
            let period_ns = (if callback_frames == 0 {
                512
            } else {
                callback_frames
            } as u64)
                .saturating_mul(1_000_000_000)
                / sample_rate.max(1) as u64;
            // Keep a small margin for scheduling and the callback itself.
            let computation_ns = period_ns.saturating_mul(4) / 5;
            let (period, computation) = mac::constraint_ticks(period_ns, computation_ns);
            let (workgroup, workgroup_error) = match device_id {
                Some(id) => match mac::get_workgroup(id) {
                    Ok(wg) => (Some(wg), None),
                    Err(e) => (None, Some(e)),
                },
                None => (None, None),
            };
            Self {
                inner: Arc::new(Inner {
                    metrics: Arc::new(MetricCounters::default()),
                    workgroup,
                    workgroup_error,
                    period,
                    computation,
                    period_ns,
                    computation_ns,
                    id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                }),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (sample_rate, callback_frames, device_id);
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

    /// Install the callback-matched Mach policy before joining the device's
    /// audio workgroup (which only accepts real-time threads). The guard must
    /// outlive the work and be
    /// dropped on the same thread. Failure is observable in its status.
    pub fn enter(&self) -> AudioScheduleGuard {
        #[cfg(target_os = "macos")]
        {
            let mut workgroup_error = self.inner.workgroup_error;
            match mac::enter_constraint(self.inner.period, self.inner.computation) {
                Ok(previous) => {
                    mac::set_budget(
                        self.inner.id,
                        self.inner.period_ns,
                        self.inner.computation_ns,
                        self.inner.metrics.clone(),
                    );
                    if let Some(wg) = &self.inner.workgroup {
                        let mut token = mac::JoinToken::default();
                        // SAFETY: this is now a real-time thread; the retained
                        // object outlives the guard and its token stays here.
                        let result = unsafe { mac::os_workgroup_join(wg.0, &mut token) };
                        if result == 0 {
                            return AudioScheduleGuard {
                                inner: self.inner.clone(),
                                state: GuardState::Workgroup { token, previous },
                                status: ScheduleStatus {
                                    method: ScheduleMethod::DeviceWorkgroup,
                                    workgroup_error: None,
                                    mach_error: None,
                                },
                                _thread: PhantomData,
                            };
                        }
                        workgroup_error = Some(result);
                    }
                    AudioScheduleGuard {
                        inner: self.inner.clone(),
                        state: GuardState::Constraint(previous),
                        status: ScheduleStatus {
                            method: ScheduleMethod::MachTimeConstraint,
                            workgroup_error,
                            mach_error: None,
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
                    },
                    _thread: PhantomData,
                },
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            AudioScheduleGuard {
                inner: self.inner.clone(),
                status: ScheduleStatus {
                    method: ScheduleMethod::Unavailable,
                    workgroup_error: None,
                    mach_error: None,
                },
                _thread: PhantomData,
            }
        }
    }
}

/// Check the current thread's Mach budget without requiring its schedule at
/// nested source/DSP call sites. No-op outside a Mach schedule guard.
#[inline]
pub fn checkpoint() {
    #[cfg(target_os = "macos")]
    mac::checkpoint_current();
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
    },
    Constraint(mac::PreviousPolicy),
    Unavailable,
}

impl Drop for AudioScheduleGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        match &mut self.state {
            GuardState::Workgroup { token, previous } => {
                let wg = self
                    .inner
                    .workgroup
                    .as_ref()
                    .expect("joined workgroup retained");
                // SAFETY: guard is !Send; this is the same thread and token
                // used at join, and its retained object is still alive.
                unsafe { mac::os_workgroup_leave(wg.0, token) };
                mac::clear_budget(self.inner.id);
                mac::restore_constraint(*previous, &self.inner.metrics);
            }
            GuardState::Constraint(previous) => {
                mac::clear_budget(self.inner.id);
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
        // SAFETY: CoreAudio gives the caller one retained os_workgroup_t.
        unsafe { mac::os_release(self.0) };
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{MetricCounters, Workgroup};
    use std::{
        cell::RefCell,
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
    const SCHED_TIMESHARE_INFO: i32 = 10;
    const EXTENDED_INFO: i32 = 5;
    const KERN_SUCCESS: i32 = 0;

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
    unsafe extern "C" {
        pub(super) fn os_workgroup_join(wg: *mut c_void, token: *mut JoinToken) -> i32;
        pub(super) fn os_workgroup_leave(wg: *mut c_void, token: *mut JoinToken);
        pub(super) fn os_release(object: *mut c_void);
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

    pub(super) fn constraint_ticks(period_ns: u64, computation_ns: u64) -> (u32, u32) {
        let mut base = Timebase { numer: 0, denom: 0 };
        // SAFETY: valid writable timebase information.
        let ok = unsafe { mach_timebase_info(&mut base) } == KERN_SUCCESS && base.numer != 0;
        if !ok {
            return (0, 0);
        }
        let ticks = |ns: u64| {
            (ns as u128 * base.denom as u128 / base.numer as u128).clamp(1, u32::MAX as u128) as u32
        };
        let period = ticks(period_ns);
        let computation = ticks(computation_ns).min(period);
        (period, computation)
    }

    pub(super) fn enter_constraint(period: u32, computation: u32) -> Result<PreviousPolicy, i32> {
        if period == 0 || computation == 0 {
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
            constraint: period,
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
        exit_cpu_ns: u64,
        period_ns: u64,
        computation_ns: u64,
        computation_reported: bool,
        constraint_reported: bool,
        metrics: Arc<MetricCounters>,
    }
    thread_local! { static BUDGET: RefCell<Option<Budget>> = const { RefCell::new(None) }; }

    fn thread_cpu_ns() -> u64 {
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

    pub(super) fn set_budget(
        id: u64,
        period_ns: u64,
        computation_ns: u64,
        metrics: Arc<MetricCounters>,
    ) {
        BUDGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(previous) = slot.as_mut().filter(|b| b.id == id || b.depth == 0) {
                resume_budget(
                    previous,
                    id,
                    period_ns,
                    computation_ns,
                    metrics,
                    thread_cpu_ns(),
                );
                if previous.wall_start.elapsed() >= Duration::from_nanos(previous.period_ns) {
                    reset_period(previous);
                }
            } else {
                let cpu = thread_cpu_ns();
                *slot = Some(Budget {
                    id,
                    depth: 1,
                    wall_start: Instant::now(),
                    cpu_start_ns: cpu,
                    last_checkpoint_cpu_ns: cpu,
                    exit_cpu_ns: cpu,
                    period_ns,
                    computation_ns,
                    computation_reported: false,
                    constraint_reported: false,
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
        metrics: Arc<MetricCounters>,
        cpu_now: u64,
    ) {
        // Different viewers sharing a pool thread cannot erase guarded CPU
        // already spent in this reservation window.
        let changed_context = b.id != id;
        if b.depth == 0 {
            // Exclude pool-control or unrelated CPU between guards.
            b.cpu_start_ns = b
                .cpu_start_ns
                .saturating_add(cpu_now.saturating_sub(b.exit_cpu_ns));
            b.last_checkpoint_cpu_ns = cpu_now;
        }
        b.id = id;
        b.period_ns = period_ns;
        b.computation_ns = computation_ns;
        b.metrics = metrics;
        if changed_context {
            b.computation_reported = false;
            b.constraint_reported = false;
        }
        b.depth = b.depth.saturating_add(1);
    }
    pub(super) fn clear_budget(id: u64) {
        BUDGET.with(|slot| {
            if let Some(budget) = slot.borrow_mut().as_mut().filter(|x| x.id == id) {
                // Observe the last work unit even when no source/DSP callback
                // made another checkpoint before this guard exited.
                let cpu = thread_cpu_ns();
                sample(budget, Instant::now(), cpu);
                budget.depth = budget.depth.saturating_sub(1);
                if budget.depth == 0 {
                    budget.exit_cpu_ns = cpu;
                }
            }
        });
    }

    fn sample(b: &mut Budget, _now: Instant, cpu: u64) {
        let unit = cpu.saturating_sub(b.last_checkpoint_cpu_ns);
        let period_cpu = cpu.saturating_sub(b.cpu_start_ns);
        b.metrics.max_unit_ns.fetch_max(unit, Ordering::Relaxed);
        b.metrics
            .max_period_cpu_ns
            .fetch_max(period_cpu, Ordering::Relaxed);
        if period_cpu > b.computation_ns && !b.computation_reported {
            b.metrics
                .computation_overruns
                .fetch_add(1, Ordering::Relaxed);
            b.computation_reported = true;
        }
        // Waiting for controls or the next I/O activation is not DSP work.
        // CPU service beyond the full callback period violates even the
        // constraint, independently of normal blocked/idle wall time.
        if period_cpu > b.period_ns && !b.constraint_reported {
            b.metrics
                .constraint_overruns
                .fetch_add(1, Ordering::Relaxed);
            b.constraint_reported = true;
        }
        b.last_checkpoint_cpu_ns = cpu;
    }

    fn reset_period(b: &mut Budget) {
        let cpu = thread_cpu_ns();
        b.wall_start = Instant::now();
        b.cpu_start_ns = cpu;
        b.last_checkpoint_cpu_ns = cpu;
        b.computation_reported = false;
        b.constraint_reported = false;
    }

    pub(super) fn checkpoint(id: u64) {
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
            // Leave one bounded DSP unit of headroom before the nominal Mach
            // computation limit; checkpoints run *before* that unit.
            let reserve = (b.computation_ns / 2).min(500_000);
            if period_cpu >= b.computation_ns.saturating_sub(reserve) {
                let period = Duration::from_nanos(b.period_ns);
                if elapsed < period {
                    let sleep_start = Instant::now();
                    std::thread::sleep(period - elapsed);
                    b.metrics.throttle_sleeps.fetch_add(1, Ordering::Relaxed);
                    b.metrics.total_sleep_ns.fetch_add(
                        sleep_start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        Ordering::Relaxed,
                    );
                }
                reset_period(b);
            } else if elapsed >= Duration::from_nanos(b.period_ns) {
                reset_period(b);
            }
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::realtime::{AudioSchedule, ScheduleMethod};

        #[test]
        fn unknown_callback_size_uses_headless_512_frame_period() {
            let unknown = AudioSchedule::new(48_000, 0, None);
            let headless = AudioSchedule::new(48_000, 512, None);
            let actual_small = AudioSchedule::new(48_000, 64, None);
            assert_eq!(unknown.inner.period_ns, headless.inner.period_ns);
            assert_eq!(unknown.inner.computation_ns, headless.inner.computation_ns);
            assert_eq!(unknown.inner.period_ns, 10_666_666);
            assert_eq!(unknown.inner.computation_ns, 8_533_332);
            assert_eq!(actual_small.inner.period_ns, 1_333_333);
        }

        #[test]
        fn headless_guard_installs_a_real_mach_constraint() {
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
                let guard = schedule.enter();
                assert_eq!(
                    guard.method(),
                    ScheduleMethod::MachTimeConstraint,
                    "{:?}",
                    guard.status()
                );
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
                    exit_cpu_ns: cpu,
                    period_ns: 50_000_000,
                    computation_ns: 1, // Already consumed on this thread.
                    computation_reported: false,
                    constraint_reported: false,
                    metrics: metrics.clone(),
                })
            });
            set_budget(ID, 50_000_000, 1, metrics);
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
                    exit_cpu_ns: cpu,
                    period_ns: 50_000_000,
                    computation_ns: 1,
                    computation_reported: false,
                    constraint_reported: false,
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
        fn reentry_excludes_unguarded_cpu_and_records_carried_debt_for_new_context() {
            const OLD: u64 = u64::MAX - 3;
            const NEW: u64 = u64::MAX - 2;
            let old_metrics = Arc::new(MetricCounters::default());
            let new_metrics = Arc::new(MetricCounters::default());
            let mut b = Budget {
                id: OLD,
                depth: 0,
                wall_start: Instant::now(),
                // 2 ms guarded work, then 5 ms outside the guard.
                cpu_start_ns: 93_000_000,
                last_checkpoint_cpu_ns: 95_000_000,
                exit_cpu_ns: 95_000_000,
                period_ns: 50_000_000,
                computation_ns: 4_000_000,
                computation_reported: true,
                constraint_reported: true,
                metrics: old_metrics,
            };
            resume_budget(
                &mut b,
                NEW,
                50_000_000,
                4_000_000,
                new_metrics.clone(),
                100_000_000,
            );
            assert_eq!(100_000_000 - b.cpu_start_ns, 2_000_000);
            assert!(!b.computation_reported);
            assert!(!b.constraint_reported);
            sample(&mut b, Instant::now(), 100_000_000);
            let observed = new_metrics.snapshot();
            assert_eq!(observed.max_period_cpu_ns, 2_000_000);
            assert_eq!(observed.computation_overruns, 0);
            assert_eq!(observed.constraint_overruns, 0);
            assert_eq!(observed.throttle_sleeps, 0);

            // A third viewer with a tighter computation budget must report
            // the inherited 2 ms debt, even though the previous viewer had
            // already marked its own period as reported.
            b.depth = 0;
            b.exit_cpu_ns = 100_000_000;
            b.computation_reported = true;
            b.constraint_reported = true;
            let third = Arc::new(MetricCounters::default());
            resume_budget(
                &mut b,
                NEW - 1,
                50_000_000,
                1_000_000,
                third.clone(),
                101_000_000,
            );
            sample(&mut b, Instant::now(), 101_000_000);
            assert_eq!(third.snapshot().computation_overruns, 1);
        }
    }
}
