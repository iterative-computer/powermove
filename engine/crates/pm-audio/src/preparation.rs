//! Detached, cancellable vocoder preparation. A process-wide pool bounds
//! CPU use across viewers; render and device threads never execute its jobs.
use crate::mixer::Mixer;
use pm_eval::Evaluator;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

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
                        #[cfg(target_os = "macos")]
                        unsafe {
                            // Utility QoS with reduced priority: background mode
                            // additionally throttles I/O and delays urgent starts.
                            unsafe extern "C" {
                                fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
                            }
                            pthread_set_qos_class_self_np(0x11, -5);
                        }
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
                            // keeps utility QoS and its single-job limit.
                            #[cfg(target_os = "macos")]
                            unsafe {
                                unsafe extern "C" {
                                    fn pthread_set_qos_class_self_np(
                                        class: u32,
                                        priority: i32,
                                    ) -> i32;
                                }
                                if !background {
                                    pthread_set_qos_class_self_np(0x21, 0);
                                }
                            }
                            job();
                            #[cfg(target_os = "macos")]
                            unsafe {
                                unsafe extern "C" {
                                    fn pthread_set_qos_class_self_np(
                                        class: u32,
                                        priority: i32,
                                    ) -> i32;
                                }
                                pthread_set_qos_class_self_np(0x11, -5);
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
impl Drop for Preparation {
    fn drop(&mut self) {
        self.invalidate();
    }
}
