//! Detached, cancellable vocoder preparation. A process-wide pool bounds
//! CPU use across viewers; render and device threads never execute its jobs.
use crate::mixer::Mixer;
use pm_eval::Evaluator;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[cfg(target_os = "macos")]
const QOS_USER_INTERACTIVE: u32 = 0x21;
#[cfg(target_os = "macos")]
const QOS_UTILITY: u32 = 0x11;
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
    Box<dyn Fn() -> bool + Send>,
    Box<dyn FnOnce() + Send>,
);
struct Queue {
    jobs: Vec<Job>,
    background: usize,
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
                        // class load for longer than a start's shared head lasts
                        // (the producer then primed every unstarted history itself,
                        // serially, and the ring ran dry). Boundary refill lowers
                        // itself to utility QoS for the length of its job.
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
                                    if let Some(at) = queue
                                        .jobs
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, j)| {
                                            (j.0 == 0 || queue.background == 0) && (j.1)()
                                        })
                                        .min_by_key(|(_, j)| j.0)
                                        .map(|(i, _)| i)
                                    {
                                        let (priority, _, job) = queue.jobs.remove(at);
                                        let background = priority != 0;
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
                            // runs at utility QoS (background mode would also
                            // throttle I/O) and keeps its single-job limit.
                            #[cfg(target_os = "macos")]
                            if background {
                                set_qos(QOS_UTILITY, -5);
                            }
                            job();
                            #[cfg(target_os = "macos")]
                            if background {
                                set_qos(QOS_USER_INTERACTIVE, 0);
                            }
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
    fn submit(
        &self,
        priority: usize,
        ready: impl Fn() -> bool + Send + 'static,
        job: impl FnOnce() + Send + 'static,
    ) {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .push((priority, Box::new(ready), Box::new(job)));
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
pub(crate) struct Preparation {
    generation: Arc<AtomicU64>,
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
            make: Arc::new(Evaluator::new),
            static_content: true,
            pending: HashSet::new(),
            tx,
            rx,
        }
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
                let ready_generation = current.clone();
                Executor::shared().submit(
                    priority + usize::from(!urgent),
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
                            // The producer (or paused preparation) owns this
                            // history now. Report only so the slot is freed.
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
                        crate::profile::reset();
                        let ready = work.run(&mut make(), static_content, cancelled);
                        let _ = tx.send(Done {
                            generation,
                            start,
                            voice,
                            history,
                            ready,
                            claimed: false,
                            profile: crate::profile::snapshot(),
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
    /// What a worker reports after dequeuing `(start, voice)` and finding
    /// its history claimed by another thread (the producer at an exhausted
    /// head, or paused preparation).
    pub(crate) fn deliver_claimed(
        &mut self,
        start: i64,
        voice: usize,
        history: Arc<Mutex<crate::stretch::StretchRun>>,
    ) {
        self.pending.insert((start, voice));
        let _ = self.tx.send(Done {
            generation: self.generation.load(Ordering::Relaxed),
            start,
            voice,
            history,
            ready: Ok(false),
            claimed: true,
            profile: Default::default(),
        });
    }
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

    fn own_qos() -> u32 {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut relative = 0;
        // SAFETY: queries the calling thread into valid out-parameters.
        unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative);
        }
        class as u32
    }

    fn observe(priority: usize) -> u32 {
        let (tx, rx) = channel();
        Executor::shared().submit(
            priority,
            || true,
            move || {
                let _ = tx.send(own_qos());
            },
        );
        rx.recv_timeout(Duration::from_secs(30)).unwrap()
    }

    /// An idle worker waits at user-interactive QoS, so a playhead job is not
    /// starved behind default-class load before it can start (the class a
    /// thread holds when woken decides when it runs). Only boundary refill
    /// runs at utility QoS, and only for its own job.
    #[test]
    fn idle_workers_wait_at_user_interactive_qos() {
        assert_eq!(observe(0), QOS_USER_INTERACTIVE);
        assert_eq!(observe(1), QOS_UTILITY);
        assert_eq!(observe(0), QOS_USER_INTERACTIVE, "restored after refill");
        // The idle class itself, read from outside the worker while it waits.
        // Other tests share the pool: a sample may land on a refill job.
        let (tx, rx) = channel();
        Executor::shared().submit(
            1,
            || true,
            move || {
                let _ = tx.send(unsafe { libc::pthread_self() } as usize);
            },
        );
        let worker = rx.recv_timeout(Duration::from_secs(30)).unwrap() as libc::pthread_t;
        std::thread::sleep(Duration::from_millis(20));
        let idle = (0..20)
            .filter(|_| {
                std::thread::sleep(Duration::from_millis(2));
                let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
                let mut relative = 0;
                // SAFETY: pool workers never exit, so the handle stays valid.
                unsafe { libc::pthread_get_qos_class_np(worker, &mut class, &mut relative) };
                class as u32 == QOS_USER_INTERACTIVE
            })
            .count();
        assert!(
            idle >= 15,
            "idle worker at user-interactive QoS in {idle} of 20 samples"
        );
    }
}
