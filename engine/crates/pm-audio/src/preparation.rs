//! Detached, cancellable vocoder preparation. A process-wide pool bounds
//! CPU use across viewers; render and device threads never execute its jobs.
use crate::mixer::Mixer;
use pm_eval::Evaluator;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

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
type Job = (
    usize,
    Arc<AtomicBool>,
    Box<dyn Fn() -> bool + Send>,
    Box<dyn FnOnce() + Send>,
);
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
                ((priority == 0 || self.background == 0) && (j.2)()).then_some((i, priority))
            })
            .min_by_key(|(_, priority)| *priority)
    }
}
fn job_priority(job: &Job) -> usize {
    if job.1.load(Ordering::Acquire) {
        0
    } else {
        job.0
    }
}
pub(crate) fn wake_workers() {
    Executor::shared().wake.notify_all();
}

struct Executor {
    jobs: Mutex<Queue>,
    wake: Condvar,
}
impl Executor {
    fn shared() -> &'static Arc<Self> {
        static POOL: OnceLock<Arc<Executor>> = OnceLock::new();
        POOL.get_or_init(|| {
            let pool = Arc::new(Self {
                jobs: Mutex::new(Queue {
                    jobs: Vec::new(),
                    background: 0,
                }),
                wake: Condvar::new(),
            });
            let count = std::thread::available_parallelism()
                .map_or(1, usize::from)
                .saturating_sub(2)
                .max(1);
            for i in 0..count {
                let p = pool.clone();
                std::thread::Builder::new()
                    .name(format!("pm-audio-prepare-{i}"))
                    .spawn(move || {
                        // Darwin's thread-specific priority leaves the renderer and
                        // real-time callback unchanged. Linux uses the calling TID.
                        //
                        // An idle worker waits at user-interactive QoS: the class a
                        // thread holds when it is woken decides how soon it runs,
                        // and a worker woken at utility QoS was starved by default
                        // class load for longer than a start's shared head lasts.
                        // Actual preparation (including boundary histories) now
                        // joins the real-time audio schedule while it works.
                        #[cfg(target_os = "macos")]
                        set_qos(QOS_USER_INTERACTIVE, 0);
                        #[cfg(target_os = "linux")]
                        unsafe {
                            libc::setpriority(libc::PRIO_PROCESS, 0, 15);
                        }
                        loop {
                            let (background, job) = {
                                let mut queue = p.jobs.lock().unwrap_or_else(|e| e.into_inner());
                                loop {
                                    if let Some((at, priority)) = queue.ready_index() {
                                        let background = priority != 0;
                                        let (_, _, _, job) = queue.jobs.remove(at);
                                        if background {
                                            queue.background += 1;
                                        }
                                        break (background, job);
                                    }
                                    queue = p.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
                                }
                            };
                            // Playhead preparation is latency-sensitive even at
                            // edit time, before Play arrives. Boundary refill
                            // keeps its single-job limit; its closure enters
                            // real-time CPU scheduling and restores the worker's
                            // effective Mach priority before it waits again.
                            job();
                            if background {
                                p.jobs.lock().unwrap_or_else(|e| e.into_inner()).background -= 1;
                            }
                            // A foreground job may have published a shared
                            // immutable head, releasing its dependent jobs.
                            p.wake.notify_all();
                        }
                    })
                    .expect("audio preparation worker");
            }
            pool
        })
    }
    #[cfg(test)]
    fn submit(
        &self,
        priority: usize,
        ready: impl Fn() -> bool + Send + 'static,
        job: impl FnOnce() + Send + 'static,
    ) {
        self.submit_promotable(priority, Arc::new(AtomicBool::new(false)), ready, job);
    }
    fn submit_promotable(
        &self,
        priority: usize,
        critical: Arc<AtomicBool>,
        ready: impl Fn() -> bool + Send + 'static,
        job: impl FnOnce() + Send + 'static,
    ) {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .push((priority, critical, Box::new(ready), Box::new(job)));
        self.wake.notify_one();
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
    /// The job found its history claimed by another thread and did not run.
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
}
impl SchedulingCounters {
    fn record(&self, status: crate::realtime::ScheduleStatus) {
        if let Some(e) = status.workgroup_error {
            self.workgroup_error.store(e, Ordering::Relaxed);
        }
        if let Some(e) = status.mach_error {
            self.mach_error.store(e, Ordering::Relaxed);
        }
        let counter = match status.method.label() {
            "device-workgroup" => &self.workgroup,
            "mach-time-constraint" => &self.mach,
            _ => &self.failed,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) struct Preparation {
    generation: Arc<AtomicU64>,
    schedule: crate::realtime::AudioSchedule,
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
        // Start the threads before a project or Play arrives.
        Executor::shared();
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            schedule: crate::realtime::AudioSchedule::new(48_000, 512, None),
            scheduling: Arc::new(SchedulingCounters::default()),
            make: Arc::new(Evaluator::new),
            static_content: true,
            pending: HashSet::new(),
            tx,
            rx,
        }
    }
    pub(crate) fn set_schedule(&mut self, schedule: crate::realtime::AudioSchedule) {
        self.schedule = schedule;
    }
    pub(crate) fn is_standard(&self) -> bool {
        self.static_content
    }
    pub(crate) fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.pending.clear();
        Executor::shared().wake.notify_all();
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
                let scheduling = self.scheduling.clone();
                let ready_generation = current.clone();
                let critical = work.critical.clone();
                Executor::shared().submit_promotable(
                    priority + usize::from(!urgent),
                    critical,
                    move || {
                        ready_generation.load(Ordering::Relaxed) != generation
                            || gate.as_ref().is_none_or(|g| g.load(Ordering::Acquire))
                    },
                    move || {
                        let cancelled = || current.load(Ordering::Relaxed) != generation;
                        if cancelled() {
                            return;
                        }
                        if !work.claim() {
                            // Paused preparation already owns this history.
                            // Report only so the pending queue slot is freed.
                            let _ = tx.send(Done {
                                generation,
                                start,
                                voice,
                                history,
                                ready: Ok(false),
                                claimed: true,
                                profile: Default::default(),
                            });
                            return;
                        }
                        // Boundary and secondary starts can become audible
                        // before their queued histories finish too. Every job
                        // enters the audio schedule; the queue still limits
                        // background refill to one job at a time.
                        let _realtime = schedule.enter();
                        scheduling.record(_realtime.status());
                        crate::profile::reset();
                        let ready = work.run(&mut make(), static_content, cancelled);
                        let profile = crate::profile::snapshot();
                        drop(_realtime);
                        let _ = tx.send(Done {
                            generation,
                            start,
                            voice,
                            history,
                            ready,
                            claimed: false,
                            profile,
                        });
                    },
                );
            }
        }
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
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

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
                    let schedule = crate::realtime::AudioSchedule::new(48_000, 512, None);
                    {
                        let guard = schedule.enter();
                        assert_eq!(guard.method().label(), "mach-time-constraint");
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
    #[test]
    fn an_audible_history_bypasses_the_serial_boundary_limit() {
        let critical = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(true));
        let gate = ready.clone();
        let queue = Queue {
            jobs: vec![(
                1,
                critical.clone(),
                Box::new(move || gate.load(Ordering::Acquire)),
                Box::new(|| {}),
            )],
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
